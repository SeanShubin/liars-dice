//! Every event survives the trip to storage and back.
//!
//! This is the test that matters most in this crate and the only one that needs nothing
//! running, so it is an ordinary unit test. `to_item` matches on `Event` with no wildcard
//! arm, so a new variant cannot compile until it is handled — but only [`every_event`]
//! below makes it handled *correctly*. Adding an event to `protocol` means adding it
//! here, in the same change.

use std::collections::HashMap;

use protocol::{Account, Event, Moment, Seat};
use relay_service::codec::{self, KIND, MATCH, Malformed, POSITION};

/// One of every variant, with values chosen to be awkward rather than representative.
fn every_event() -> Vec<Event> {
    let seat = Seat(3);
    vec![
        Event::Version { protocol: 3 },
        Event::Started {
            at: Moment::from_unix_nanos(1_709_210_096_123_000_000),
        },
        Event::Host { seat },
        Event::Bar { seat },
        Event::Lift { seat },
        Event::Start,
        Event::RoundOver { seat },
        Event::Join {
            seat,
            // A space and a colon: the archive's line format would hyphenate the one and
            // a naive account encoding would split on the other.
            name: "Mary Jane".to_string(),
            colour: "dark red".to_string(),
            account: Account::Discord("450853396908343307".to_string()),
        },
        Event::Join {
            seat: Seat(0),
            name: "Ada".to_string(),
            colour: "blue".to_string(),
            account: Account::Ai,
        },
        Event::Join {
            seat: Seat(1),
            name: "the keyboard".to_string(),
            colour: "green".to_string(),
            account: Account::Local,
        },
        Event::PlayerOrder(vec![Seat(2), Seat(0), seat]),
        Event::Variant {
            name: "ones-wild",
            on: true,
        },
        Event::Commit {
            digest: "d1e5f0".repeat(10),
        },
        Event::Reveal {
            seed: "00ff".repeat(16),
        },
        Event::Deal {
            seat,
            // Repeats and order both matter, which is why dice are a list and not a set.
            dice: vec![6, 1, 1, 3, 1],
        },
        Event::Claim {
            seat,
            quantity: 3,
            face: 6,
            // Three decimal places, to catch anything that rounds to the archive's two.
            odds: 38.207,
        },
        Event::Liar { seat, odds: 61.793 },
        Event::SpotOn { seat, odds: 0.004 },
        Event::Lucky { seat, odds: 1.2 },
        Event::Unlucky { seat, odds: 3.704 },
        Event::LoseDie { seat },
        Event::GainDie { seat },
        Event::GainShield { seat },
        Event::SpendShield { seat },
        Event::Eliminate { seat },
        Event::NextTurn { seat },
        Event::Winner { seat },
    ]
}

#[test]
fn every_event_survives_being_stored_and_read_back() {
    for event in every_event() {
        let item = codec::to_item(&event);
        let read = codec::from_item(&item)
            .unwrap_or_else(|why| panic!("{event} did not read back: {why}"));
        assert_eq!(read, event, "{event} changed on the way through storage");
    }
}

/// The failure the archive's format would have shipped silently.
#[test]
fn a_name_keeps_its_spaces_and_odds_keep_their_precision() {
    let event = Event::Claim {
        seat: Seat(1),
        quantity: 2,
        face: 5,
        odds: 38.207,
    };
    assert_eq!(event.to_string(), "claim #1 2 5 38.21%");
    let Ok(Event::Claim { odds, .. }) = codec::from_item(&codec::to_item(&event)) else {
        panic!("a claim did not read back as a claim");
    };
    assert_eq!(
        odds, 38.207,
        "storage rounded what only the archive may round"
    );

    let joined = Event::Join {
        seat: Seat(1),
        name: "Mary Jane".to_string(),
        colour: "red".to_string(),
        account: Account::Discord("1".to_string()),
    };
    assert_eq!(joined.to_string(), "join #1 Mary-Jane red discord:1");
    let Ok(Event::Join { name, .. }) = codec::from_item(&codec::to_item(&joined)) else {
        panic!("a join did not read back as a join");
    };
    assert_eq!(name, "Mary Jane", "storage renamed a player");
}

#[test]
fn the_item_carries_the_key_and_the_event() {
    let item = codec::item("match-7", 12, &Event::Start);
    assert_eq!(
        item.get(MATCH).and_then(|value| value.as_s().ok()).unwrap(),
        "match-7"
    );
    assert_eq!(
        item.get(POSITION)
            .and_then(|value| value.as_n().ok())
            .unwrap(),
        "12"
    );
    assert_eq!(
        item.get(KIND).and_then(|value| value.as_s().ok()).unwrap(),
        "start"
    );
}

/// Every kind is distinct, so no two events can be stored as each other.
///
/// The archive deliberately writes a claim, a liar and a spot on with the same command
/// word and a discriminating token. Storage cannot: the kind is all the decoder has.
#[test]
fn no_two_events_store_under_the_same_kind() {
    let mut seen: HashMap<String, Event> = HashMap::new();
    for event in every_event() {
        let kind = codec::to_item(&event)
            .get(KIND)
            .and_then(|value| value.as_s().ok())
            .expect("every event stores a kind")
            .clone();
        if let Some(first) = seen.get(&kind) {
            // Join appears three times on purpose, for its three kinds of account.
            assert!(
                matches!((first, &event), (Event::Join { .. }, Event::Join { .. })),
                "{first} and {event} both store as {kind}"
            );
            continue;
        }
        seen.insert(kind, event);
    }
}

#[test]
fn an_item_that_is_not_an_event_says_so() {
    assert_eq!(codec::from_item(&HashMap::new()), Err(Malformed::NoKind));
}

/// A log written by a newer build fails loudly rather than replaying as a shorter match.
#[test]
fn an_unknown_kind_is_refused_rather_than_skipped() {
    let item = HashMap::from([(
        KIND.to_string(),
        aws_sdk_dynamodb::types::AttributeValue::S("teleport".to_string()),
    )]);
    assert_eq!(
        codec::from_item(&item),
        Err(Malformed::UnknownKind("teleport".to_string()))
    );
}

#[test]
fn a_known_kind_missing_a_field_names_both() {
    let item = HashMap::from([(
        KIND.to_string(),
        aws_sdk_dynamodb::types::AttributeValue::S("claim".to_string()),
    )]);
    assert_eq!(
        codec::from_item(&item),
        Err(Malformed::Field {
            kind: "claim".to_string(),
            attribute: "seat".to_string()
        })
    );
}

/// A variant nobody here writes cannot be guessed at: it would mean replaying a match
/// under rules this build does not know were in force.
#[test]
fn an_unknown_variant_name_is_refused() {
    let item = HashMap::from([
        (
            KIND.to_string(),
            aws_sdk_dynamodb::types::AttributeValue::S("variant".to_string()),
        ),
        (
            "name".to_string(),
            aws_sdk_dynamodb::types::AttributeValue::S("sixes-wild".to_string()),
        ),
        (
            "on".to_string(),
            aws_sdk_dynamodb::types::AttributeValue::Bool(true),
        ),
    ]);
    assert_eq!(
        codec::from_item(&item),
        Err(Malformed::UnknownVariant("sixes-wild".to_string()))
    );
}

/// Every name the relay writes can be read back, which is the other half of refusing the
/// ones it does not.
#[test]
fn every_variant_the_relay_writes_reads_back() {
    for (name, on) in relay::VARIANTS {
        let event = Event::Variant { name, on };
        assert_eq!(codec::from_item(&codec::to_item(&event)), Ok(event));
    }
}
