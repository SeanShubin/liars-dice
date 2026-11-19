//! The two implementations against a real DynamoDB, which here is DynamoDB Local.
//!
//! What is tested is that they honour the traits' contracts, which is the only thing that
//! can differ from the in-memory ones. The library's own suite is the behaviour suite and
//! is not repeated here.
//!
//! Every test is `#[ignore]`, and the gate runs
//! `cargo test --workspace -- --include-ignored` with the container up. So `cargo test`
//! on a machine with no Docker stays green and stays honest: the runner prints what it
//! skipped, which a test that quietly passes when it cannot reach an endpoint does not.
//!
//! To run them here:
//!
//! ```text
//! docker run -d -p 8000:8000 amazon/dynamodb-local
//! cargo test -p relay-service --test storage -- --include-ignored
//! ```

use std::sync::atomic::{AtomicU64, Ordering};

use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_dynamodb::types::{
    AttributeDefinition, BillingMode, GlobalSecondaryIndex, KeySchemaElement, KeyType, Projection,
    ProjectionType, ScalarAttributeType,
};
use protocol::{Account, Event, Seat};
use relay::{ConnectionId, Connections, Log, MatchId, Role, Taken};
use relay_service::connections::{BY_CONNECTION, CONNECTION};
use relay_service::{DynamoLog, GatewayConnections, Meter, codec};
use tokio::runtime::Runtime;

/// Where DynamoDB Local is. The SDK's own variable, so nothing bespoke has to be set.
fn endpoint() -> String {
    std::env::var("AWS_ENDPOINT_URL_DYNAMODB").unwrap_or_else(|_| "http://localhost:8000".into())
}

/// Credentials DynamoDB Local does not check. Static, so no provider chain runs and no
/// real profile on the machine can be picked up by accident.
fn client() -> Client {
    let config = aws_sdk_dynamodb::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .endpoint_url(endpoint())
        .credentials_provider(Credentials::new(
            "local",
            "local",
            None,
            None,
            "dynamodb-local",
        ))
        .build();
    Client::from_conf(config)
}

/// A table name no other test is using, so the tests can run in parallel.
fn unique(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{nanos}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn key(name: &str, kind: KeyType) -> KeySchemaElement {
    KeySchemaElement::builder()
        .attribute_name(name)
        .key_type(kind)
        .build()
        .expect("a key schema element")
}

fn attribute(name: &str, kind: ScalarAttributeType) -> AttributeDefinition {
    AttributeDefinition::builder()
        .attribute_name(name)
        .attribute_type(kind)
        .build()
        .expect("an attribute definition")
}

/// The log table: partitioned by match, ordered by position.
fn log_table(runtime: &Runtime, client: &Client) -> String {
    let table = unique("log");
    runtime
        .block_on(
            client
                .create_table()
                .table_name(&table)
                .billing_mode(BillingMode::PayPerRequest)
                .key_schema(key(codec::MATCH, KeyType::Hash))
                .key_schema(key(codec::POSITION, KeyType::Range))
                .attribute_definitions(attribute(codec::MATCH, ScalarAttributeType::S))
                .attribute_definitions(attribute(codec::POSITION, ScalarAttributeType::N))
                .send(),
        )
        .expect("the table should be created; if nothing answers at all, see this file's header");
    table
}

/// The connection table, and the reverse index `$disconnect` needs.
///
/// `KEYS_ONLY` because the index is asked one question — which match is this socket in —
/// and the base table's keys are the answer.
fn connection_table(runtime: &Runtime, client: &Client) -> String {
    let table = unique("connections");
    let index = GlobalSecondaryIndex::builder()
        .index_name(BY_CONNECTION)
        .key_schema(key(CONNECTION, KeyType::Hash))
        .projection(
            Projection::builder()
                .projection_type(ProjectionType::KeysOnly)
                .build(),
        )
        .build()
        .expect("a global secondary index");
    runtime
        .block_on(
            client
                .create_table()
                .table_name(&table)
                .billing_mode(BillingMode::PayPerRequest)
                .key_schema(key(codec::MATCH, KeyType::Hash))
                .key_schema(key(CONNECTION, KeyType::Range))
                .attribute_definitions(attribute(codec::MATCH, ScalarAttributeType::S))
                .attribute_definitions(attribute(CONNECTION, ScalarAttributeType::S))
                .global_secondary_indexes(index)
                .send(),
        )
        .expect("the table should be created; if nothing answers at all, see this file's header");
    table
}

fn claim(seat: u8, quantity: u32) -> Event {
    Event::Claim {
        seat: Seat(seat),
        quantity,
        face: 6,
        odds: 38.207,
    }
}

/// The library calls `append` and `read` from a thread that is not a runtime worker,
/// because the handler hands `receive` to `spawn_blocking`. A test thread is also not a
/// worker, so blocking inside the trait methods is exercised here exactly as deployed.
fn arrange() -> (Runtime, Client) {
    let runtime = Runtime::new().expect("a tokio runtime");
    let client = client();
    (runtime, client)
}

#[test]
#[ignore = "needs DynamoDB Local; the gate runs it with --include-ignored"]
fn a_log_reads_back_in_position_order() {
    let (runtime, client) = arrange();
    let table = log_table(&runtime, &client);
    let mut log = DynamoLog::new(client, &table, runtime.handle().clone(), Meter::new());
    let match_id = MatchId("match-0".into());

    let written = vec![
        Event::Version { protocol: 3 },
        Event::Join {
            seat: Seat(0),
            name: "Mary Jane".into(),
            colour: "red".into(),
            account: Account::Discord("450853396908343307".into()),
        },
        claim(0, 2),
        claim(1, 3),
    ];
    for (position, event) in written.iter().enumerate() {
        log.append(&match_id, position as u64, std::slice::from_ref(event))
            .expect("an empty position");
    }

    // Written in order, and read back in it. The sort key is the position, so this holds
    // whatever order the writes happened to land in.
    assert_eq!(log.read(&match_id), written);
}

#[test]
#[ignore = "needs DynamoDB Local; the gate runs it with --include-ignored"]
fn an_append_at_a_taken_position_is_refused_and_changes_nothing() {
    let (runtime, client) = arrange();
    let table = log_table(&runtime, &client);
    let mut log = DynamoLog::new(client, &table, runtime.handle().clone(), Meter::new());
    let match_id = MatchId("match-0".into());

    log.append(&match_id, 0, &[claim(0, 2)])
        .expect("position 0");
    assert_eq!(
        log.append(&match_id, 0, &[claim(1, 9)]),
        Err(Taken),
        "the condition expression is the whole concurrency design"
    );
    assert_eq!(
        log.read(&match_id),
        vec![claim(0, 2)],
        "a refused append must not overwrite: the loser re-reads, and would re-read a lie"
    );
}

/// A message of several events lands whole or not at all.
///
/// This is the `TransactWriteItems` path, which only a message of more than one event
/// takes — hosting is three and starting is a dozen. `A message either happens entirely
/// or not at all`, so the half-committed case is the one worth proving does not exist.
#[test]
#[ignore = "needs DynamoDB Local; the gate runs it with --include-ignored"]
fn a_message_of_several_events_lands_whole_or_not_at_all() {
    let (runtime, client) = arrange();
    let table = log_table(&runtime, &client);
    let mut log = DynamoLog::new(client, &table, runtime.handle().clone(), Meter::new());
    let match_id = MatchId("match-0".into());

    let message = vec![
        Event::Version { protocol: 3 },
        Event::Join {
            seat: Seat(0),
            name: "Mary Jane".into(),
            colour: "red".into(),
            account: Account::Discord("450853396908343307".into()),
        },
        Event::Host { seat: Seat(0) },
    ];
    log.append(&match_id, 0, &message).expect("an empty log");
    assert_eq!(log.read(&match_id), message);

    // Somebody else is already at position 0, so the whole message is refused — and in
    // particular its second and third events do not land at 3 and 4.
    assert_eq!(log.append(&match_id, 0, &message), Err(Taken));
    assert_eq!(
        log.read(&match_id),
        message,
        "a refused message must leave nothing of itself behind"
    );

    // The same message does land at the position the log is actually at.
    log.append(&match_id, 3, &message)
        .expect("the end of the log");
    assert_eq!(log.read(&match_id).len(), 6);
}

/// A message whose first position is free but whose later ones are not is still refused.
///
/// The condition is on every item, not only the first, so this cannot be committed by a
/// transaction that happened to start in a gap.
#[test]
#[ignore = "needs DynamoDB Local; the gate runs it with --include-ignored"]
fn a_message_overlapping_an_existing_event_is_refused() {
    let (runtime, client) = arrange();
    let table = log_table(&runtime, &client);
    let mut log = DynamoLog::new(client, &table, runtime.handle().clone(), Meter::new());
    let match_id = MatchId("match-0".into());

    // Position 1 is taken and position 0 is not, which a one-item condition would miss.
    log.append(&match_id, 1, &[claim(0, 9)])
        .expect("position 1");
    assert_eq!(
        log.append(&match_id, 0, &[claim(1, 1), claim(1, 2), claim(1, 3)]),
        Err(Taken)
    );
    assert_eq!(log.read(&match_id), vec![claim(0, 9)]);
}

/// What the conditional write is for. Many threads race for one position; exactly one
/// lands, and every loser is told so rather than told nothing.
#[test]
#[ignore = "needs DynamoDB Local; the gate runs it with --include-ignored"]
fn concurrent_appends_at_one_position_produce_one_winner() {
    let (runtime, client) = arrange();
    let table = log_table(&runtime, &client);
    let match_id = MatchId("match-0".into());
    let racers = 8;

    let outcomes: Vec<Result<(), Taken>> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..racers)
            .map(|seat| {
                let client = client.clone();
                let table = table.clone();
                let handle = runtime.handle().clone();
                let match_id = match_id.clone();
                scope.spawn(move || {
                    let mut log = DynamoLog::new(client, table, handle, Meter::new());
                    log.append(&match_id, 0, &[claim(seat, 2)])
                    // Every racer claims a different quantity of the same face, so the
                    // winner is identifiable from the log alone.
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().expect("a racer"))
            .collect()
    });

    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
        1,
        "exactly one append may land at a position: {outcomes:?}"
    );
    let log = DynamoLog::new(client, &table, runtime.handle().clone(), Meter::new());
    assert_eq!(log.read(&match_id).len(), 1);
}

#[test]
#[ignore = "needs DynamoDB Local; the gate runs it with --include-ignored"]
fn matches_lists_every_match_once_however_many_events_it_holds() {
    let (runtime, client) = arrange();
    let table = log_table(&runtime, &client);
    let mut log = DynamoLog::new(client, &table, runtime.handle().clone(), Meter::new());

    for (index, name) in ["match-0", "match-1", "match-2"].iter().enumerate() {
        let match_id = MatchId((*name).to_string());
        for position in 0..=index as u64 {
            log.append(&match_id, position, &[claim(0, position as u32 + 1)])
                .expect("an empty position");
        }
    }

    assert_eq!(
        log.matches(),
        vec![
            MatchId("match-0".into()),
            MatchId("match-1".into()),
            MatchId("match-2".into())
        ],
        "one entry per match, not one per event"
    );
}

/// The counters count. The timings cannot be asserted on, which is why the counts are:
/// a measurement nobody checked is a decoration, and these two are the numbers that
/// trigger the lobby projection.
#[test]
#[ignore = "needs DynamoDB Local; the gate runs it with --include-ignored"]
fn the_meter_counts_what_a_lobby_view_cost() {
    let (runtime, client) = arrange();
    let table = log_table(&runtime, &client);
    let meter = Meter::new();
    let mut log = DynamoLog::new(client, &table, runtime.handle().clone(), meter.clone());

    // Three matches holding one, two and three events: six events in all.
    for (index, name) in ["match-0", "match-1", "match-2"].iter().enumerate() {
        let match_id = MatchId((*name).to_string());
        for position in 0..=index as u64 {
            log.append(&match_id, position, &[claim(0, position as u32 + 1)])
                .expect("an empty position");
        }
    }

    // What a lobby view does: find the matches, then fold each one.
    let found = log.matches();
    for match_id in &found {
        log.read(match_id);
    }

    assert_eq!(meter.scans(), 1, "one scan per lobby view");
    assert_eq!(meter.reads(), 3, "one read per match folded");
    assert_eq!(meter.events(), 6, "every event of every match");

    let line = meter.emf(
        "LiarsDice/Relay",
        "$default",
        std::time::Duration::from_millis(7),
    );
    assert!(line.contains("\"events_read\":6"), "{line}");
    assert!(line.contains("\"log_reads\":3"), "{line}");
    assert!(line.contains("\"route\":\"$default\""), "{line}");
}

#[test]
#[ignore = "needs DynamoDB Local; the gate runs it with --include-ignored"]
fn a_connection_is_found_by_match_and_detached_by_itself() {
    let (runtime, client) = arrange();
    let table = connection_table(&runtime, &client);
    let mut connections = GatewayConnections::new(client, &table, runtime.handle().clone());
    let match_id = MatchId("match-0".into());
    let socket = ConnectionId("abc=".into());

    connections.attach(&match_id, socket.clone(), Seat(2), Role::Player);
    assert_eq!(
        connections.of(&match_id),
        vec![relay::Connected {
            connection: socket.clone(),
            seat: Seat(2),
            role: Role::Player
        }]
    );

    // `$disconnect` knows only the connection id, which is what the index is for.
    connections.detach(&socket);
    assert_eq!(
        connections.of(&match_id),
        vec![],
        "a detached socket must leave no row: the seat becomes absent, and the library \
         turns that into whatever the match requires"
    );
}

/// A socket is in one place. Attaching it somewhere else moves it rather than cloning it.
#[test]
#[ignore = "needs DynamoDB Local; the gate runs it with --include-ignored"]
fn attaching_a_connection_twice_leaves_it_in_one_match() {
    let (runtime, client) = arrange();
    let table = connection_table(&runtime, &client);
    let mut connections = GatewayConnections::new(client, &table, runtime.handle().clone());
    let first = MatchId("match-0".into());
    let second = MatchId("match-1".into());
    let socket = ConnectionId("abc=".into());

    connections.attach(&first, socket.clone(), Seat(0), Role::Player);
    connections.attach(&second, socket.clone(), Seat(1), Role::Player);

    assert_eq!(connections.of(&first), vec![]);
    assert_eq!(
        connections.of(&second),
        vec![relay::Connected {
            connection: socket,
            seat: Seat(1),
            role: Role::Player
        }]
    );
}
