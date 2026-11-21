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

use protocol::{Account, Event, Moment, Seat, Who};
use relay::{Listed, MatchId, Message, Refused};

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
        Event::Left { seat },
        Event::Start,
        Event::RoundOver { seat },
        Event::MatchOver { seat },
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
        // The three endings. Two of them carry nothing, which is the case a rendering
        // that assumes every event has fields gets wrong.
        Event::Complete,
        Event::Cancel { seat },
        Event::Abandon,
    ]
}

pub fn every_message() -> Vec<Message> {
    let match_id = MatchId("match-7".to_string());
    vec![
        // A ticket, which is opaque text and may hold a dot and hexadecimal either side
        // of it — the one shape a format that split on punctuation would mangle.
        Message::Host {
            who: Who::Authenticated {
                ticket: "0011aabb.ffee0000".to_string(),
            },
            colour: "dark red".to_string(),
            version: protocol::PROTOCOL_VERSION,
            nonce: "a nonce with a space".to_string(),
        },
        // And a guest, whose name is theirs to choose and is awkward on purpose.
        Message::Host {
            who: Who::Guest {
                id: "0123456789abcdef0123456789abcdef".to_string(),
                name: awkward_name(),
            },
            colour: "dark red".to_string(),
            version: protocol::PROTOCOL_VERSION,
            nonce: "a nonce with a space".to_string(),
        },
        Message::Join {
            match_id: match_id.clone(),
            who: Who::Guest {
                id: "fedcba9876543210fedcba9876543210".to_string(),
                name: "Ada".to_string(),
            },
            colour: "blue".to_string(),
            version: protocol::PROTOCOL_VERSION,
        },
        Message::List {
            who: Who::Authenticated {
                ticket: "0011aabb.ffee0000".to_string(),
            },
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
        Message::Leave {
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
        Message::Over {
            match_id: match_id.clone(),
        },
        Message::Cancel {
            match_id: match_id.clone(),
        },
        Message::Resume {
            match_id,
            who: Who::Guest {
                id: "0123456789abcdef0123456789abcdef".to_string(),
                name: "Ada".to_string(),
            },
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
        Answer::Refused(Refused::MatchIsOver),
        Answer::Refused(Refused::BadTicket),
        Answer::Refused(Refused::AlreadyInAMatch {
            match_id: MatchId("match-8".to_string()),
        }),
        // Zero and not zero. The relay never sends zero — a refusal means at least one
        // second remains — but zero is the number a rendering is most likely to omit or
        // read back as absent, so the format is held to it anyway.
        Answer::Refused(Refused::StillActive { seconds: 0 }),
        Answer::Refused(Refused::StillActive { seconds: 47 }),
        Answer::Refused(Refused::NotYourSeat),
        Answer::Refused(Refused::MatchIdTaken),
        // Empty and not empty, because an empty list is the case the two renderings
        // cannot tell apart on the way back — see `Value::Many`.
        Answer::Listing(Vec::new()),
        Answer::Listing(vec![
            Listed {
                match_id: MatchId("match-7".to_string()),
                host: awkward_name(),
                players: 1,
                ranked: false,
                joinable: true,
                mine: false,
            },
            // Both values of the flag, because a rendering that dropped it would come
            // back as one of them and look right half the time.
            Listed {
                match_id: MatchId("match-8".to_string()),
                host: "Ada".to_string(),
                players: 9,
                ranked: true,
                // Every flag both ways across the two rows, because a rendering that
                // dropped one would come back as a default and look right half the time.
                joinable: false,
                mine: true,
            },
        ]),
    ];
    answers.extend(every_event().into_iter().map(Answer::Event));
    answers
}
