//! [`Log`] on DynamoDB. The append is a conditional write; the order is the database's.
//!
//! Realizes the log in /implementation/relay-service.md.

use std::time::Instant;

use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::types::{AttributeValue, ReturnConsumedCapacity, Select};
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

impl Log for DynamoLog {
    /// `attribute_not_exists(position)` is the whole concurrency design.
    ///
    /// Two invocations racing for the same position cannot both land; the loser gets
    /// `Taken`, re-reads, and tries again at the end it now knows about. The total order
    /// the protocol depends on comes from the database rather than from a lock, which is
    /// what lets any number of Lambdas serve one match.
    fn append(&mut self, match_id: &MatchId, position: u64, event: &Event) -> Result<(), Taken> {
        let put = self
            .client
            .put_item()
            .table_name(&self.table)
            .set_item(Some(codec::item(&match_id.0, position, event)))
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
                // `Log` has no error channel, by design: the in-memory implementation
                // cannot fail, and widening the trait would spread `Result` through a
                // library whose point is that it runs with no storage at all. There is
                // also exactly one thing to do about a storage failure here — fail the
                // invocation, so the client retries against a log that is append-only and
                // safe to retry against — and a panic already does that.
                panic!("appending {event} to {} at {position}: {error}", match_id.0);
            }
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
                let event = codec::from_item(item).unwrap_or_else(|why| {
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
