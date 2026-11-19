//! [`Log`] on DynamoDB. The append is a conditional write; the order is the database's.
//!
//! Realizes the log in /implementation/relay-service.md.

use std::time::Instant;

use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::error::DisplayErrorContext;
use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
use aws_sdk_dynamodb::types::{
    AttributeValue, Put, ReturnConsumedCapacity, Select, TransactWriteItem,
};
use protocol::Event;
use relay::{Log, MatchId, Taken};
use tokio::runtime::Handle;

use crate::codec::{self, MATCH, POSITION};
use crate::meter::Meter;

/// The log, over one DynamoDB table.
///
/// Holds a [`Handle`] and blocks on it, because [`Log`] is synchronous and worth keeping
/// so: an async trait would spread `async` through the library, its in-memory
/// implementations and every test that plays a whole match in one process. The handler
/// calls `Relay::receive` inside `spawn_blocking`, where blocking is legal. See the async
/// seam in /implementation/relay-service.md.
pub struct DynamoLog {
    client: Client,
    table: String,
    handle: Handle,
    meter: Meter,
}

impl DynamoLog {
    pub fn new(client: Client, table: impl Into<String>, handle: Handle, meter: Meter) -> Self {
        Self {
            client,
            table: table.into(),
            handle,
            meter,
        }
    }

    pub fn meter(&self) -> &Meter {
        &self.meter
    }
}

impl DynamoLog {
    fn put_one(&self, match_id: &MatchId, at: u64, event: &Event) -> Result<(), Taken> {
        let put = self
            .client
            .put_item()
            .table_name(&self.table)
            .set_item(Some(codec::item(&match_id.0, at, event)))
            .condition_expression("attribute_not_exists(#position)")
            .expression_attribute_names("#position", POSITION);
        match self.handle.block_on(put.send()) {
            Ok(_) => Ok(()),
            Err(error) => {
                // The one expected failure, and the only one that is not a failure.
                if error
                    .as_service_error()
                    .is_some_and(|service| service.is_conditional_check_failed_exception())
                {
                    return Err(Taken);
                }
                panic!(
                    "appending {event} to {} at {at}: {}",
                    match_id.0,
                    DisplayErrorContext(&error)
                );
            }
        }
    }

    fn put_all(&self, match_id: &MatchId, from: u64, events: &[Event]) -> Result<(), Taken> {
        // A hundred is DynamoDB's limit on one transaction. No message comes near it —
        // the largest starts a ten player match — so this is an assertion rather than a
        // case to handle, and it fails loudly if a future message is written that does.
        assert!(
            events.len() <= 100,
            "a message of {} events cannot be committed atomically",
            events.len()
        );
        let mut transaction = self.client.transact_write_items();
        for (offset, event) in events.iter().enumerate() {
            let put = Put::builder()
                .table_name(&self.table)
                .set_item(Some(codec::item(&match_id.0, from + offset as u64, event)))
                .condition_expression("attribute_not_exists(#position)")
                .expression_attribute_names("#position", POSITION)
                .build()
                .expect("a put with a table name and an item");
            transaction = transaction.transact_items(TransactWriteItem::builder().put(put).build());
        }
        match self.handle.block_on(transaction.send()) {
            Ok(_) => Ok(()),
            Err(error) => {
                // A cancelled transaction is how a conditional check fails inside one.
                // Every other cancellation reason — throttling, a conflict with another
                // transaction on the same item — is a real failure, and reading the
                // reasons is how they are told apart.
                if let Some(TransactWriteItemsError::TransactionCanceledException(cancelled)) =
                    error.as_service_error()
                    && cancelled
                        .cancellation_reasons()
                        .iter()
                        .any(|reason| reason.code() == Some("ConditionalCheckFailed"))
                {
                    return Err(Taken);
                }
                panic!(
                    "appending {} events to {} from {from}: {}",
                    events.len(),
                    match_id.0,
                    DisplayErrorContext(&error)
                );
            }
        }
    }
}

impl Log for DynamoLog {
    /// `attribute_not_exists(position)` is the whole concurrency design.
    ///
    /// `from` is the length of the log the caller decided against, so the condition on the
    /// first position is also a check that the decision is still true. Two invocations
    /// racing cannot both land; the loser gets `Taken`, re-reads, and decides again
    /// against what it now sees.
    ///
    /// One event is a `PutItem` and several are a `TransactWriteItems`, because a
    /// transaction costs twice the capacity of a plain write and most messages are one
    /// event. The transaction is what makes a message all or nothing, which matters for
    /// the ones that are not: hosting writes three events and starting writes a dozen,
    /// and half a message in the log is a match nobody can replay.
    fn append(&mut self, match_id: &MatchId, from: u64, events: &[Event]) -> Result<(), Taken> {
        match events {
            [] => Ok(()),
            [event] => self.put_one(match_id, from, event),
            many => self.put_all(match_id, from, many),
        }
    }

    /// Every event of one match, in position order.
    ///
    /// Events are appended once and never changed, so this does not ask for a strongly
    /// consistent read: there is no version of an item to be behind.
    fn read(&self, match_id: &MatchId) -> Vec<Event> {
        let mut events = Vec::new();
        let mut pages = self
            .client
            .query()
            .table_name(&self.table)
            .key_condition_expression("#match = :match")
            .expression_attribute_names("#match", MATCH)
            .expression_attribute_values(":match", AttributeValue::S(match_id.0.clone()))
            .scan_index_forward(true)
            .return_consumed_capacity(ReturnConsumedCapacity::Total)
            .into_paginator()
            .send();
        while let Some(page) = self.handle.block_on(pages.next()) {
            let page = page.unwrap_or_else(|error| panic!("reading {}: {error}", match_id.0));
            if let Some(capacity) = page
                .consumed_capacity()
                .and_then(|used| used.capacity_units())
            {
                self.meter.capacity(capacity);
            }
            for item in page.items() {
                let event = codec::event_from_item(item).unwrap_or_else(|why| {
                    // Refusing to continue is the point. A log that reads as all but one
                    // of its events replays as a different match, and does it quietly.
                    panic!("{} holds an item this build cannot read: {why}", match_id.0)
                });
                events.push(event);
            }
        }
        self.meter.read(events.len());
        events
    }

    /// Every match the log holds, for the lobby list.
    ///
    /// A Scan projecting only the partition key, which grows with every event ever
    /// written rather than with the matches still open. It is timed separately for
    /// exactly that reason: this is the cost no projection of open matches would reduce,
    /// and the Open Item in /implementation/relay-service.md is about replacing it.
    fn matches(&self) -> Vec<MatchId> {
        let began = Instant::now();
        let mut found = std::collections::BTreeSet::new();
        let mut pages = self
            .client
            .scan()
            .table_name(&self.table)
            .projection_expression("#match")
            .expression_attribute_names("#match", MATCH)
            .select(Select::SpecificAttributes)
            .return_consumed_capacity(ReturnConsumedCapacity::Total)
            .into_paginator()
            .send();
        while let Some(page) = self.handle.block_on(pages.next()) {
            let page = page.unwrap_or_else(|error| panic!("listing matches: {error}"));
            if let Some(capacity) = page
                .consumed_capacity()
                .and_then(|used| used.capacity_units())
            {
                self.meter.capacity(capacity);
            }
            for item in page.items() {
                if let Some(id) = item.get(MATCH).and_then(|value| value.as_s().ok()) {
                    found.insert(MatchId(id.clone()));
                }
            }
        }
        self.meter.scan(began.elapsed());
        found.into_iter().collect()
    }
}
