//! Everything survives the trip to JSON and back, and the archive's losses are named.

use protocol::{Account, Event, Seat};
use relay::{MatchId, Message};
use wire::{Malformed, Shape, Value, event, examples, json, message};

#[test]
fn every_event_survives_the_wire() {
    for original in examples::every_event() {
        let text = json::write(&event::shape_of(&original));
        let shape = json::read(&text).unwrap_or_else(|why| panic!("{text} did not parse: {why}"));
        let read = event::from_shape(&shape)
            .unwrap_or_else(|why| panic!("{original} did not read back from {text}: {why}"));
        assert_eq!(read, original, "{original} changed on the wire as {text}");
    }
}

#[test]
fn every_message_survives_the_wire() {
    for original in examples::every_message() {
        let text = json::write(&message::shape_of(&original));
        let shape = json::read(&text).unwrap_or_else(|why| panic!("{text} did not parse: {why}"));
        let read = message::from_shape(&shape)
            .unwrap_or_else(|why| panic!("{original:?} did not read back from {text}: {why}"));
        assert_eq!(read, original, "{original:?} changed on the wire as {text}");
    }
}

/// The two losses that reusing the archive's line format would have shipped silently.
#[test]
fn a_name_keeps_its_spaces_and_odds_keep_their_precision() {
    let claim = Event::Claim {
        seat: Seat(1),
        quantity: 2,
        face: 5,
        odds: 38.207,
    };
    assert_eq!(claim.to_string(), "claim #1 2 5 38.21%");
    let text = json::write(&event::shape_of(&claim));
    assert!(text.contains("38.207"), "the wire rounded the odds: {text}");

    let joined = Event::Join {
        seat: Seat(1),
        name: examples::awkward_name(),
        colour: "red".to_string(),
        account: Account::Discord("1".to_string()),
    };
    assert_eq!(joined.to_string(), "join #1 Mary-Jane red discord:1");
    let text = json::write(&event::shape_of(&joined));
    assert!(
        text.contains("Mary Jane"),
        "the wire renamed a player: {text}"
    );
}

/// What the format looks like, pinned so a change to it is a change to this test.
///
/// Keys come out in alphabetical order, which is why `kind` is not always first: the
/// frame's bytes are then a function of what the shape holds and not of the order the
/// fields were declared in.
#[test]
fn the_rendering_is_the_kind_and_its_fields() {
    assert_eq!(
        json::write(&event::shape_of(&Event::Start)),
        r#"{"kind":"start"}"#
    );
    assert_eq!(
        json::write(&event::shape_of(&Event::LoseDie { seat: Seat(2) })),
        r#"{"kind":"lose-die","seat":2}"#
    );
    assert_eq!(
        json::write(&message::shape_of(&Message::Claim {
            match_id: MatchId("m".to_string()),
            event: Event::Liar {
                seat: Seat(1),
                odds: 61.793
            },
        })),
        r#"{"event":{"kind":"liar","odds":61.793,"seat":1},"kind":"claim","match":"m"}"#
    );
}

/// Every kind is distinct, so no two events can be sent as each other.
///
/// The archive deliberately writes a claim, a liar and a spot on with the same command
/// word and a discriminating token. A shape cannot: the kind is all the reader has.
#[test]
fn no_two_events_share_a_kind() {
    let mut seen: Vec<(String, Event)> = Vec::new();
    for original in examples::every_event() {
        let kind = event::shape_of(&original).kind;
        if let Some((_, first)) = seen.iter().find(|(seen, _)| *seen == kind) {
            // Join appears three times on purpose, for its three kinds of account.
            assert!(
                matches!((first, &original), (Event::Join { .. }, Event::Join { .. })),
                "{first} and {original} share the kind {kind}"
            );
            continue;
        }
        seen.push((kind, original));
    }
}

#[test]
fn text_that_is_not_a_shape_says_so() {
    assert!(matches!(
        json::read("not json"),
        Err(Malformed::NotAShape(_))
    ));
    assert!(matches!(json::read("[]"), Err(Malformed::NotAShape(_))));
    assert!(matches!(json::read("{}"), Err(Malformed::NotAShape(_))));
}

/// A frame from a newer build fails loudly rather than being treated as a shorter match.
#[test]
fn an_unknown_kind_is_refused_rather_than_skipped() {
    let shape = json::read(r#"{"kind":"teleport","seat":1}"#).expect("a shape");
    assert_eq!(
        event::from_shape(&shape),
        Err(Malformed::UnknownKind("teleport".to_string()))
    );
    assert_eq!(
        message::from_shape(&shape),
        Err(Malformed::UnknownKind("teleport".to_string()))
    );
}

#[test]
fn a_known_kind_missing_a_field_names_both() {
    let shape = json::read(r#"{"kind":"claim"}"#).expect("a shape");
    assert_eq!(
        event::from_shape(&shape),
        Err(Malformed::Field {
            kind: "claim".to_string(),
            field: "seat".to_string()
        })
    );
}

/// A variant nobody writes cannot be guessed at: it would mean replaying a match under
/// rules this build does not know were in force.
#[test]
fn an_unknown_variant_name_is_refused() {
    let shape = json::read(r#"{"kind":"variant","name":"sixes-wild","on":true}"#).expect("a shape");
    assert_eq!(
        event::from_shape(&shape),
        Err(Malformed::UnknownVariant("sixes-wild".to_string()))
    );
}

#[test]
fn every_variant_the_relay_writes_reads_back() {
    for (name, on) in relay::VARIANTS {
        let original = Event::Variant { name, on };
        let shape = json::read(&json::write(&event::shape_of(&original))).expect("a shape");
        assert_eq!(event::from_shape(&shape), Ok(original));
    }
}

#[test]
fn an_account_that_is_none_of_the_three_is_refused() {
    let shape = Shape::of(
        "resume",
        vec![
            ("match", Value::Text("m".to_string())),
            ("account", Value::Text("steam:7".to_string())),
        ],
    );
    assert_eq!(
        message::from_shape(&shape),
        Err(Malformed::UnknownAccount("steam:7".to_string()))
    );
}

/// A colon inside an id must not change who the account is.
#[test]
fn a_discord_id_keeps_everything_after_the_first_colon() {
    let shape = Shape::of(
        "resume",
        vec![
            ("match", Value::Text("m".to_string())),
            ("account", Value::Text("discord:a:b".to_string())),
        ],
    );
    assert_eq!(
        message::from_shape(&shape),
        Ok(Message::Resume {
            match_id: MatchId("m".to_string()),
            account: Account::Discord("a:b".to_string())
        })
    );
}
