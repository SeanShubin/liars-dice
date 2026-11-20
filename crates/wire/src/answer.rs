//! What the service puts back on the socket.
//!
//! Three things, not one: the events a message produced, the refusal if it produced none,
//! and the match id if it made a match. A client parses one frame type and dispatches on
//! the kind, so there is one place that decides what an inbound frame is.
//!
//! The kinds `refused` and `created` belong to no event, which a test holds to rather
//! than assumes.

use protocol::Event;
use relay::{Listed, MatchId, Refused};

use crate::{Malformed, Shape, Value, event};

/// One frame from the service.
#[derive(Clone, PartialEq, Debug)]
pub enum Answer {
    /// Something that happened. The same shape the log holds.
    Event(Event),
    /// The message was not accepted. None of these are game rules.
    Refused(Refused),
    /// A message made a match. Only the host is told, and it is how they learn the id.
    Created(MatchId),
    /// What is open to join, as it was when it was asked for.
    ///
    /// A listing goes stale the moment it is sent — a match in it can fill or start
    /// before anybody acts on one — which is not a flaw to design out. The refusals on
    /// `join` are what make acting on a stale listing safe, and the alternative is a
    /// subscription that has to be cancelled by every path a client can leave by.
    Listing(Vec<Listed>),
}

/// The kind of a refusal frame.
pub const REFUSED: &str = "refused";
/// The kind of a created-match frame.
pub const CREATED: &str = "created";
/// The kind of a lobby-listing frame.
pub const LISTING: &str = "listing";

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

pub fn shape_of(answer: &Answer) -> Shape {
    match answer {
        Answer::Event(event) => event::shape_of(event),
        Answer::Refused(why) => refusal(why),
        Answer::Created(match_id) => Shape::of(CREATED, vec![("match", text(&match_id.0))]),
        Answer::Listing(open) => Shape::of(
            LISTING,
            vec![(
                "matches",
                Value::Many(open.iter().map(listed_shape).collect()),
            )],
        ),
    }
}

/// One match in a listing: what to join, who is waiting, and how many are there.
///
/// `A listed match shows its host` and `A listed match shows how many players have
/// joined`, which with whether it is ranked is the whole of what a listing may say. Not
/// the seats, not the accounts, not the colours: a lobby is read by people who are not in
/// the match. Ranked is a property of the match rather than of anybody in it, which is
/// why it may be said out loud.
fn listed_shape(open: &Listed) -> Shape {
    Shape::of(
        "open",
        vec![
            ("match", text(&open.match_id.0)),
            ("host", text(&open.host)),
            ("players", Value::Count(open.players as u64)),
            ("ranked", Value::Flag(open.ranked)),
        ],
    )
}

/// A refusal names why in one field, so a client can switch on it, and carries numbers
/// only where there is something to say beyond the name.
fn refusal(why: &Refused) -> Shape {
    let named = |why: &str| Shape::of(REFUSED, vec![("why", text(why))]);
    match why {
        Refused::NoSuchMatch => named("no-such-match"),
        Refused::VersionMismatch {
            match_version,
            yours,
        } => Shape::of(
            REFUSED,
            vec![
                ("why", text("version-mismatch")),
                ("match-version", Value::Count(u64::from(*match_version))),
                ("yours", Value::Count(u64::from(*yours))),
            ],
        ),
        Refused::AlreadyStarted => named("already-started"),
        Refused::Barred => named("barred"),
        Refused::ColourTaken => named("colour-taken"),
        Refused::MatchIsFull => named("match-is-full"),
        Refused::NotTheHost => named("not-the-host"),
        Refused::NotInThisMatch => named("not-in-this-match"),
        Refused::BadTicket => named("bad-ticket"),
        Refused::MatchIsOver => named("match-is-over"),
        Refused::StillActive { seconds } => Shape::of(
            REFUSED,
            vec![
                ("why", text("still-active")),
                ("seconds", Value::Count(u64::from(*seconds))),
            ],
        ),
        Refused::NotYourSeat => named("not-your-seat"),
        Refused::MatchIdTaken => named("match-id-taken"),
    }
}

pub fn from_shape(shape: &Shape) -> Result<Answer, Malformed> {
    match shape.kind.as_str() {
        REFUSED => Ok(Answer::Refused(why(shape)?)),
        CREATED => Ok(Answer::Created(MatchId(shape.text("match")?.to_string()))),
        LISTING => Ok(Answer::Listing(
            shape
                .many("matches")?
                .iter()
                .map(listed_of)
                .collect::<Result<Vec<Listed>, Malformed>>()?,
        )),
        // Anything else is an event, including an unknown kind: `event::from_shape` is
        // the one place that decides whether a kind is known, so there is one answer to
        // that question rather than two that can disagree.
        _ => event::from_shape(shape).map(Answer::Event),
    }
}

fn listed_of(shape: &Shape) -> Result<Listed, Malformed> {
    Ok(Listed {
        match_id: MatchId(shape.text("match")?.to_string()),
        host: shape.text("host")?.to_string(),
        players: shape.count("players")?,
        ranked: shape.flag("ranked")?,
    })
}

fn why(shape: &Shape) -> Result<Refused, Malformed> {
    match shape.text("why")? {
        "no-such-match" => Ok(Refused::NoSuchMatch),
        "version-mismatch" => Ok(Refused::VersionMismatch {
            match_version: shape.count("match-version")?,
            yours: shape.count("yours")?,
        }),
        "already-started" => Ok(Refused::AlreadyStarted),
        "barred" => Ok(Refused::Barred),
        "colour-taken" => Ok(Refused::ColourTaken),
        "match-is-full" => Ok(Refused::MatchIsFull),
        "not-the-host" => Ok(Refused::NotTheHost),
        "not-in-this-match" => Ok(Refused::NotInThisMatch),
        "bad-ticket" => Ok(Refused::BadTicket),
        "match-is-over" => Ok(Refused::MatchIsOver),
        "still-active" => Ok(Refused::StillActive {
            seconds: shape.count("seconds")?,
        }),
        "not-your-seat" => Ok(Refused::NotYourSeat),
        "match-id-taken" => Ok(Refused::MatchIdTaken),
        other => Err(Malformed::UnknownKind(format!("{REFUSED}:{other}"))),
    }
}
