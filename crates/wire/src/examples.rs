//! One of every event and every message, with values chosen to be awkward.
//!
//! Public, and part of the crate rather than of a test, because more than one rendering
//! has to be held to it: the JSON here, and the DynamoDB item in `apps/relay`. Two lists
//! would drift, and the one that drifted would be the one not covering the variant that
//! broke.
//!
//! Adding a variant to `Event` or `Message` is already a compile error in `event::shape_of`
//! and `message::shape_of`. Adding it *here* is what makes it handled correctly, in every
//! rendering at once.

use protocol::{Account, Event, Moment, Seat};
use relay::{MatchId, Message, Refused};

use crate::answer::Answer;

/// A name with a space and an account with a colon: the first is what the archive's line
/// format would hyphenate, the second is what a careless account encoding would split on.
pub fn awkward_name() -> String {
    "Mary Jane".to_string()
}

pub fn every_event() -> Vec<Event> {
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
            name: awkward_name(),
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
        // Whole numbers, which a rendering with no type on a number will read as counts.
        Event::Lucky { seat, odds: 1.0 },
        Event::Unlucky { seat, odds: 0.0 },
        Event::LoseDie { seat },
        Event::GainDie { seat },
        Event::GainShield { seat },
        Event::SpendShield { seat },
        Event::Eliminate { seat },
        Event::NextTurn { seat },
        Event::Winner { seat },
    ]
}

pub fn every_message() -> Vec<Message> {
    let match_id = MatchId("match-7".to_string());
    vec![
        Message::Host {
            account: Account::Discord("450853396908343307".to_string()),
            name: awkward_name(),
            colour: "dark red".to_string(),
            version: protocol::PROTOCOL_VERSION,
            nonce: "a nonce with a space".to_string(),
        },
        Message::Join {
            match_id: match_id.clone(),
            account: Account::Ai,
            name: "Ada".to_string(),
            colour: "blue".to_string(),
            version: protocol::PROTOCOL_VERSION,
        },
        Message::Bar {
            match_id: match_id.clone(),
            seat: Seat(2),
        },
        Message::Lift {
            match_id: match_id.clone(),
            seat: Seat(2),
        },
        Message::Start {
            match_id: match_id.clone(),
        },
        // The nested one. Its event has to survive inside a message as exactly as it does
        // on its own, which is the thing a flat format would have quietly got wrong.
        Message::Claim {
            match_id: match_id.clone(),
            event: Event::Claim {
                seat: Seat(1),
                quantity: 2,
                face: 5,
                odds: 38.207,
            },
        },
        Message::Claim {
            match_id: match_id.clone(),
            event: Event::Liar {
                seat: Seat(1),
                odds: 61.793,
            },
        },
        Message::RoundOver {
            match_id: match_id.clone(),
        },
        Message::Resume {
            match_id,
            account: Account::Local,
        },
    ]
}

/// One of every answer the service can put back on the socket, refusals included.
pub fn every_answer() -> Vec<Answer> {
    let mut answers = vec![
        Answer::Created(MatchId("match-7".to_string())),
        Answer::Refused(Refused::NoSuchMatch),
        Answer::Refused(Refused::VersionMismatch {
            match_version: 3,
            yours: 2,
        }),
        Answer::Refused(Refused::AlreadyStarted),
        Answer::Refused(Refused::Barred),
        Answer::Refused(Refused::ColourTaken),
        Answer::Refused(Refused::MatchIsFull),
        Answer::Refused(Refused::NotTheHost),
        Answer::Refused(Refused::NotInThisMatch),
        Answer::Refused(Refused::SeatIsAbsent),
        Answer::Refused(Refused::NotYourSeat),
        Answer::Refused(Refused::MatchIdTaken),
    ];
    answers.extend(every_event().into_iter().map(Answer::Event));
    answers
}
