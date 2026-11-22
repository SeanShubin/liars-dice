//! A [`Message`] as a [`Shape`], and back. What a client puts on the socket.
//!
//! The kinds deliberately read as the archive's command words where there is one, so a
//! frame in a CloudWatch log and a line in an archive say the same thing in the same
//! vocabulary.
//!
//! As with events, the match on `Message` has no wildcard arm, so a new message stops
//! this compiling rather than arriving as nothing.

use protocol::{Seat, Who};
use relay::{MatchId, Message};

use crate::{Malformed, Shape, Value, event};

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn of(match_id: &MatchId) -> (&'static str, Value) {
    ("match", text(&match_id.0))
}

/// A `Who` as fields, flat, the way a refusal names its own variant.
///
/// `who` says which it is and the remaining keys are that variant's. Flat rather than
/// nested because the format is flat except for the event a claim carries, and
/// self-describing because a client that could send both an id and a ticket could send two
/// different answers to one question.
fn who_of(who: &Who) -> Vec<(&'static str, Value)> {
    match who {
        Who::Guest { id, name } => vec![
            ("who", text("guest")),
            ("id", text(id)),
            ("name", text(name)),
        ],
        Who::Authenticated { ticket } => {
            vec![("who", text("authenticated")), ("ticket", text(ticket))]
        }
    }
}

fn who_from(shape: &Shape) -> Result<Who, Malformed> {
    match shape.text("who")? {
        "guest" => Ok(Who::Guest {
            id: shape.text("id")?.to_string(),
            name: shape.text("name")?.to_string(),
        }),
        "authenticated" => Ok(Who::Authenticated {
            ticket: shape.text("ticket")?.to_string(),
        }),
        other => Err(Malformed::UnknownKind(format!("who:{other}"))),
    }
}

pub fn shape_of(message: &Message) -> Shape {
    match message {
        Message::Host {
            who,
            version,
            nonce,
        } => {
            let mut fields = who_of(who);
            fields.extend([
                ("version", Value::Count(u64::from(*version))),
                ("nonce", text(nonce)),
            ]);
            Shape::of("host", fields)
        }
        Message::Join {
            match_id,
            who,
            version,
        } => {
            let mut fields = vec![of(match_id)];
            fields.extend(who_of(who));
            fields.extend([("version", Value::Count(u64::from(*version)))]);
            Shape::of("join", fields)
        }
        Message::Colour { match_id, colour } => {
            Shape::of("colour", vec![of(match_id), ("colour", text(colour))])
        }
        Message::Bar { match_id, seat } => Shape::of(
            "bar",
            vec![of(match_id), ("seat", Value::Count(u64::from(seat.0)))],
        ),
        Message::Lift { match_id, seat } => Shape::of(
            "lift",
            vec![of(match_id), ("seat", Value::Count(u64::from(seat.0)))],
        ),
        Message::Start { match_id } => Shape::of("start", vec![of(match_id)]),
        Message::Leave { match_id } => Shape::of("leave", vec![of(match_id)]),
        // The one nested shape in the format. A claim carries the event it is claiming,
        // because the relay records what it received and judges none of it.
        Message::Claim { match_id, event } => Shape::of(
            "claim",
            vec![
                of(match_id),
                ("event", Value::Nested(Box::new(event::shape_of(event)))),
            ],
        ),
        Message::RoundOver { match_id } => Shape::of("round-over", vec![of(match_id)]),
        Message::Over { match_id } => Shape::of("over", vec![of(match_id)]),
        Message::Cancel { match_id } => Shape::of("cancel", vec![of(match_id)]),
        Message::List { who } => Shape::of("list", who_of(who)),
        Message::Resume { match_id, who } => {
            let mut fields = vec![of(match_id)];
            fields.extend(who_of(who));
            Shape::of("resume", fields)
        }
    }
}

pub fn from_shape(shape: &Shape) -> Result<Message, Malformed> {
    let match_id =
        || -> Result<MatchId, Malformed> { Ok(MatchId(shape.text("match")?.to_string())) };
    let seat = || -> Result<Seat, Malformed> { Ok(Seat(shape.count("seat")?)) };
    match shape.kind.as_str() {
        "host" => Ok(Message::Host {
            who: who_from(shape)?,
            version: shape.count("version")?,
            nonce: shape.text("nonce")?.to_string(),
        }),
        "join" => Ok(Message::Join {
            match_id: match_id()?,
            who: who_from(shape)?,
            version: shape.count("version")?,
        }),
        "colour" => Ok(Message::Colour {
            match_id: match_id()?,
            colour: shape.text("colour")?.to_string(),
        }),
        "bar" => Ok(Message::Bar {
            match_id: match_id()?,
            seat: seat()?,
        }),
        "lift" => Ok(Message::Lift {
            match_id: match_id()?,
            seat: seat()?,
        }),
        "start" => Ok(Message::Start {
            match_id: match_id()?,
        }),
        "leave" => Ok(Message::Leave {
            match_id: match_id()?,
        }),
        "claim" => Ok(Message::Claim {
            match_id: match_id()?,
            event: event::from_shape(shape.nested("event")?)?,
        }),
        "round-over" => Ok(Message::RoundOver {
            match_id: match_id()?,
        }),
        "over" => Ok(Message::Over {
            match_id: match_id()?,
        }),
        "cancel" => Ok(Message::Cancel {
            match_id: match_id()?,
        }),
        "list" => Ok(Message::List {
            who: who_from(shape)?,
        }),
        "resume" => Ok(Message::Resume {
            match_id: match_id()?,
            who: who_from(shape)?,
        }),
        other => Err(Malformed::UnknownKind(other.to_string())),
    }
}
