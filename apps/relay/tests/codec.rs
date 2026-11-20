//! Every event survives the trip to a DynamoDB item and back.
//!
//! The other rendering of the same decomposition is JSON, tested in
//! `crates/wire/tests/round_trip.rs`. Both are held to `wire::examples::every_event`, so
//! a variant added to that one list is covered here without this file being touched —
//! which is the point of there being one list.
//!
//! Needs nothing running, which is why it is an ordinary unit test and the tests in
//! storage.rs are not.

use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue;
use protocol::{Account, Event, Moment, Seat};
use relay_service::codec;
use wire::{KIND, Malformed, examples, message};

/// When the rows these tests build were recorded. Any moment; nothing reads it back as
/// part of an event, which is the property the round trip is checking.
fn recorded() -> Moment {
    Moment::from_unix_nanos(1_700_000_000_000_000_000)
}

#[test]
fn every_event_survives_being_stored_and_read_back() {
    for original in examples::every_event() {
        let item = codec::item("match-0", 0, &original, recorded());
        let read = codec::event_from_item(&item)
            .unwrap_or_else(|why| panic!("{original} did not read back: {why}"));
        assert_eq!(
            read, original,
            "{original} changed on the way through storage"
        );
    }
}

/// The losses that reusing the archive's line format would have shipped silently.
#[test]
fn a_name_keeps_its_spaces_and_odds_keep_their_precision() {
    let claim = Event::Claim {
        seat: Seat(1),
        quantity: 2,
        face: 5,
        odds: 38.207,
    };
    assert_eq!(claim.to_string(), "claim #1 2 5 38.21%");
    let Ok(Event::Claim { odds, .. }) =
        codec::event_from_item(&codec::item("m", 0, &claim, recorded()))
    else {
        panic!("a claim did not read back as a claim");
    };
    assert_eq!(
        odds, 38.207,
        "storage rounded what only the archive may round"
    );

    let joined = Event::Join {
        seat: Seat(1),
        name: examples::awkward_name(),
        colour: "red".to_string(),
        account: Account::Discord("1".to_string()),
    };
    assert_eq!(joined.to_string(), "join #1 Mary-Jane red discord:1");
    let Ok(Event::Join { name, .. }) =
        codec::event_from_item(&codec::item("m", 0, &joined, recorded()))
    else {
        panic!("a join did not read back as a join");
    };
    assert_eq!(name, examples::awkward_name(), "storage renamed a player");
}

#[test]
fn the_item_carries_the_key_and_the_event() {
    let item = codec::item("match-7", 12, &Event::Start, recorded());
    let text = |field: &str| item.get(field).and_then(|value| value.as_s().ok()).cloned();
    let number = |field: &str| item.get(field).and_then(|value| value.as_n().ok()).cloned();
    assert_eq!(text(codec::MATCH).as_deref(), Some("match-7"));
    assert_eq!(number(codec::POSITION).as_deref(), Some("12"));
    assert_eq!(text(KIND).as_deref(), Some("start"));
    assert_eq!(
        number(codec::RECORDED).as_deref(),
        Some("1700000000000000000"),
        "`The relay records when it received each event`"
    );
    assert_eq!(codec::recorded_at(&item), Some(recorded()));
}

/// A row with no recorded time is one written before this build, and says so.
///
/// `relay::idle_seconds` reads `None` as inactive since forever, which is what abandons
/// the matches that were in the table before times were kept. Treating a missing time as
/// *unknown, so not expired* would leave them in the lobby permanently, which is the
/// problem the hour rule exists to solve.
#[test]
fn an_item_from_before_recorded_times_has_none() {
    let mut item = codec::item("match-7", 12, &Event::Start, recorded());
    item.remove(codec::RECORDED);
    assert_eq!(codec::recorded_at(&item), None);
    assert_eq!(
        codec::event_from_item(&item).expect("still an event"),
        Event::Start,
        "an old row is still readable; only its time is missing"
    );
}

/// The key says where the item is and the recorded time says when it arrived; neither is
/// part of what it holds, so reading a shape back leaves both out. An event with a field
/// called `match`, `position` or `recorded` would collide, and none has one.
///
/// This is the test that keeps the time out of the fold. `MatchView::fold` takes
/// `&[Event]`, so an `Event` that carried its arrival time would put a clock reading into
/// every client's view of the match — and into the archive, and into every golden file.
#[test]
fn neither_the_key_nor_the_recorded_time_is_mistaken_for_a_field() {
    let item = codec::item("match-7", 12, &Event::LoseDie { seat: Seat(2) }, recorded());
    let shape = codec::shape_from_item(&item, &[codec::MATCH, codec::POSITION, codec::RECORDED])
        .expect("a shape");
    assert_eq!(shape.kind, "lose-die");
    assert_eq!(
        shape.fields.len(),
        1,
        "{shape:?} kept part of the key or the recorded time"
    );
    assert!(shape.get("seat").is_some());
}

/// A message is never stored, but the rendering is general and a nested shape is the one
/// case a flat mapping would have got wrong. Checking it here costs a test and proves the
/// rendering handles every `Value`.
#[test]
fn a_nested_shape_renders_and_reads_back() {
    for original in examples::every_message() {
        let item = codec::written(&message::shape_of(&original));
        // Nothing ignored: a message carries no key, and its `match` field is a field.
        let shape = codec::shape_from_item(&item, &[]).expect("a shape");
        let read = message::from_shape(&shape)
            .unwrap_or_else(|why| panic!("{original:?} did not read back: {why}"));
        assert_eq!(read, original);
    }
}

#[test]
fn an_item_that_is_not_an_event_says_so() {
    assert!(matches!(
        codec::event_from_item(&HashMap::new()),
        Err(Malformed::NotAShape(_))
    ));
}

/// A log written by a newer build fails loudly rather than replaying as a shorter match.
#[test]
fn an_unknown_kind_is_refused_rather_than_skipped() {
    let item = HashMap::from([(KIND.to_string(), AttributeValue::S("teleport".to_string()))]);
    assert_eq!(
        codec::event_from_item(&item),
        Err(Malformed::UnknownKind("teleport".to_string()))
    );
}

#[test]
fn a_known_kind_missing_a_field_names_both() {
    let item = HashMap::from([(KIND.to_string(), AttributeValue::S("claim".to_string()))]);
    assert_eq!(
        codec::event_from_item(&item),
        Err(Malformed::Field {
            kind: "claim".to_string(),
            field: "seat".to_string()
        })
    );
}
