//! A [`Message`] as a [`Shape`], and back. What a client puts on the socket.
//!
//! The kinds deliberately read as the archive's command words where there is one, so a
//! frame in a CloudWatch log and a line in an archive say the same thing in the same
//! vocabulary.
//!
//! As with events, the match on `Message` has no wildcard arm, so a new message stops
//! this compiling rather than arriving as nothing.

use protocol::Seat;
use relay::{MatchId, Message};

use crate::{Malformed, Shape, Value, account_of, account_text, event};

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn of(match_id: &MatchId) -> (&'static str, Value) {
    ("match", text(&match_id.0))
}

pub fn shape_of(message: &Message) -> Shape {
    match message {
        Message::Host {
            account,
            name,
            colour,
            version,
            nonce,
        } => Shape::of(
            "host",
            vec![
                ("account", text(&account_text(account))),
                ("name", text(name)),
                ("colour", text(colour)),
                ("version", Value::Count(u64::from(*version))),
                ("nonce", text(nonce)),
            ],
        ),
        Message::Join {
            match_id,
            account,
            name,
            colour,
            version,
        } => Shape::of(
            "join",
            vec![
                of(match_id),
                ("account", text(&account_text(account))),
                ("name", text(name)),
                ("colour", text(colour)),
                ("version", Value::Count(u64::from(*version))),
            ],
        ),
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
        Message::Resume { match_id, account } => Shape::of(
            "resume",
            vec![of(match_id), ("account", text(&account_text(account)))],
        ),
    }
}

pub fn from_shape(shape: &Shape) -> Result<Message, Malformed> {
    let match_id =
        || -> Result<MatchId, Malformed> { Ok(MatchId(shape.text("match")?.to_string())) };
    let seat = || -> Result<Seat, Malformed> { Ok(Seat(shape.count("seat")?)) };
    match shape.kind.as_str() {
        "host" => Ok(Message::Host {
            account: account_of(shape.text("account")?)?,
            name: shape.text("name")?.to_string(),
            colour: shape.text("colour")?.to_string(),
            version: shape.count("version")?,
            nonce: shape.text("nonce")?.to_string(),
        }),
        "join" => Ok(Message::Join {
            match_id: match_id()?,
            account: account_of(shape.text("account")?)?,
            name: shape.text("name")?.to_string(),
            colour: shape.text("colour")?.to_string(),
            version: shape.count("version")?,
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
        "resume" => Ok(Message::Resume {
            match_id: match_id()?,
            account: account_of(shape.text("account")?)?,
        }),
        other => Err(Malformed::UnknownKind(other.to_string())),
    }
}
