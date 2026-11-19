//! An [`Event`] as a DynamoDB item, and back.
//!
//! Realizes the item format in /implementation/relay-service.md: one attribute for the
//! kind, then one per field of that variant. Numbers stay numbers so the database can
//! compare them, and nothing is rounded or collapsed on the way in.
//!
//! The archive's line format is not used here. `Display` is a rendering: it cuts odds to
//! two places and turns the spaces in a name into hyphens, which is correct for a line
//! somebody reads and wrong for the only copy of what happened.
//!
//! [`to_item`] matches on `Event` with no wildcard arm, so a new variant in `protocol`
//! stops this compiling. That forces it to be *handled*; only the round-trip list in
//! tests/codec.rs forces it to be handled correctly, so add it there in the same change.

use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue;
use protocol::{Account, Event, Moment, Seat};

/// The partition key: which match this event belongs to.
pub const MATCH: &str = "match";
/// The sort key: where in the log it sits. The conditional write is on this.
pub const POSITION: &str = "position";
/// Which variant of [`Event`] the rest of the attributes describe.
pub const KIND: &str = "kind";

/// An item that is not an event this version can read.
///
/// Carries what was wrong rather than a unit, because the only time anyone reads one of
/// these is while looking at a log that will not load, and the attribute name is the
/// first thing they need.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Malformed {
    /// No `kind` attribute at all. Not an event item.
    NoKind,
    /// A `kind` this version does not know. Deliberately an error and not a skip: an
    /// event nobody can read is still an event that happened, and replaying the log
    /// without it would silently produce a different match.
    UnknownKind(String),
    /// The kind was known but a field it needs is missing or the wrong type.
    Field { kind: String, attribute: String },
    /// A variant name the relay does not write. See [`relay::VARIANTS`].
    UnknownVariant(String),
    /// An account that is neither a Discord id, an AI, nor the local player.
    UnknownAccount(String),
}

impl std::fmt::Display for Malformed {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Malformed::NoKind => write!(out, "no kind attribute: not an event item"),
            Malformed::UnknownKind(kind) => write!(out, "unknown event kind {kind}"),
            Malformed::Field { kind, attribute } => {
                write!(out, "event {kind} is missing or misreads {attribute}")
            }
            Malformed::UnknownVariant(name) => write!(out, "unknown variant name {name}"),
            Malformed::UnknownAccount(account) => write!(out, "unknown account {account}"),
        }
    }
}

impl std::error::Error for Malformed {}

/// The key of one event, for a Query or a conditional Put.
pub fn key(match_id: &str, position: u64) -> HashMap<String, AttributeValue> {
    HashMap::from([
        (MATCH.to_string(), text(match_id)),
        (POSITION.to_string(), number(position)),
    ])
}

/// The whole item: the key, then the event.
pub fn item(match_id: &str, position: u64, event: &Event) -> HashMap<String, AttributeValue> {
    let mut item = key(match_id, position);
    item.extend(to_item(event));
    item
}

/// The event's own attributes, without the key.
pub fn to_item(event: &Event) -> HashMap<String, AttributeValue> {
    // One arm per variant and no wildcard: this is where `protocol` gaining an event
    // becomes a compile error rather than an event that stores as nothing.
    let fields: Vec<(&str, AttributeValue)> = match event {
        Event::Version { protocol } => {
            vec![(KIND, text("version")), ("protocol", number(*protocol))]
        }
        Event::Started { at } => vec![(KIND, text("started")), ("at", number(at.to_unix_nanos()))],
        Event::Host { seat } => vec![(KIND, text("host")), ("seat", seat_of(*seat))],
        Event::Bar { seat } => vec![(KIND, text("bar")), ("seat", seat_of(*seat))],
        Event::Lift { seat } => vec![(KIND, text("lift")), ("seat", seat_of(*seat))],
        Event::Start => vec![(KIND, text("start"))],
        Event::RoundOver { seat } => vec![(KIND, text("round-over")), ("seat", seat_of(*seat))],
        Event::Join {
            seat,
            name,
            colour,
            account,
        } => vec![
            (KIND, text("join")),
            ("seat", seat_of(*seat)),
            ("name", text(name)),
            ("colour", text(colour)),
            ("account", text(&account_text(account))),
        ],
        Event::PlayerOrder(order) => vec![
            (KIND, text("player-order")),
            (
                "order",
                AttributeValue::L(order.iter().map(|seat| seat_of(*seat)).collect()),
            ),
        ],
        Event::Variant { name, on } => vec![
            (KIND, text("variant")),
            ("name", text(name)),
            ("on", AttributeValue::Bool(*on)),
        ],
        Event::Commit { digest } => vec![(KIND, text("commit")), ("digest", text(digest))],
        Event::Reveal { seed } => vec![(KIND, text("reveal")), ("seed", text(seed))],
        Event::Deal { seat, dice } => vec![
            (KIND, text("deal")),
            ("seat", seat_of(*seat)),
            (
                "dice",
                AttributeValue::L(dice.iter().map(|face| number(*face)).collect()),
            ),
        ],
        Event::Claim {
            seat,
            quantity,
            face,
            odds,
        } => vec![
            (KIND, text("claim")),
            ("seat", seat_of(*seat)),
            ("quantity", number(*quantity)),
            ("face", number(*face)),
            ("odds", chance(*odds)),
        ],
        Event::Liar { seat, odds } => vec![
            (KIND, text("liar")),
            ("seat", seat_of(*seat)),
            ("odds", chance(*odds)),
        ],
        Event::SpotOn { seat, odds } => vec![
            (KIND, text("spot-on")),
            ("seat", seat_of(*seat)),
            ("odds", chance(*odds)),
        ],
        Event::Lucky { seat, odds } => vec![
            (KIND, text("lucky")),
            ("seat", seat_of(*seat)),
            ("odds", chance(*odds)),
        ],
        Event::Unlucky { seat, odds } => vec![
            (KIND, text("unlucky")),
            ("seat", seat_of(*seat)),
            ("odds", chance(*odds)),
        ],
        Event::LoseDie { seat } => vec![(KIND, text("lose-die")), ("seat", seat_of(*seat))],
        Event::GainDie { seat } => vec![(KIND, text("gain-die")), ("seat", seat_of(*seat))],
        Event::GainShield { seat } => vec![(KIND, text("gain-shield")), ("seat", seat_of(*seat))],
        Event::SpendShield { seat } => vec![(KIND, text("spend-shield")), ("seat", seat_of(*seat))],
        Event::Eliminate { seat } => vec![(KIND, text("eliminate")), ("seat", seat_of(*seat))],
        Event::NextTurn { seat } => vec![(KIND, text("next-turn")), ("seat", seat_of(*seat))],
        Event::Winner { seat } => vec![(KIND, text("winner")), ("seat", seat_of(*seat))],
    };
    fields
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect()
}

/// The event an item describes, or why it is not one.
pub fn from_item(item: &HashMap<String, AttributeValue>) -> Result<Event, Malformed> {
    let kind = item
        .get(KIND)
        .and_then(|value| value.as_s().ok())
        .ok_or(Malformed::NoKind)?
        .as_str();
    let read = Reader { kind, item };
    match kind {
        "version" => Ok(Event::Version {
            protocol: read.number("protocol")?,
        }),
        "started" => Ok(Event::Started {
            at: Moment::from_unix_nanos(read.number("at")?),
        }),
        "host" => Ok(Event::Host { seat: read.seat()? }),
        "bar" => Ok(Event::Bar { seat: read.seat()? }),
        "lift" => Ok(Event::Lift { seat: read.seat()? }),
        "start" => Ok(Event::Start),
        "round-over" => Ok(Event::RoundOver { seat: read.seat()? }),
        "join" => Ok(Event::Join {
            seat: read.seat()?,
            name: read.text("name")?.to_string(),
            colour: read.text("colour")?.to_string(),
            account: account_of(read.text("account")?)?,
        }),
        "player-order" => Ok(Event::PlayerOrder(
            read.list("order")?
                .iter()
                .map(|value| Ok(Seat(parse(value).ok_or_else(|| read.wrong("order"))?)))
                .collect::<Result<Vec<_>, Malformed>>()?,
        )),
        "variant" => Ok(Event::Variant {
            name: variant_named(read.text("name")?)?,
            on: *item
                .get("on")
                .and_then(|value| value.as_bool().ok())
                .ok_or_else(|| read.wrong("on"))?,
        }),
        "commit" => Ok(Event::Commit {
            digest: read.text("digest")?.to_string(),
        }),
        "reveal" => Ok(Event::Reveal {
            seed: read.text("seed")?.to_string(),
        }),
        "deal" => Ok(Event::Deal {
            seat: read.seat()?,
            dice: read
                .list("dice")?
                .iter()
                .map(|value| parse(value).ok_or_else(|| read.wrong("dice")))
                .collect::<Result<Vec<_>, Malformed>>()?,
        }),
        "claim" => Ok(Event::Claim {
            seat: read.seat()?,
            quantity: read.number("quantity")?,
            face: read.number("face")?,
            odds: read.odds()?,
        }),
        "liar" => Ok(Event::Liar {
            seat: read.seat()?,
            odds: read.odds()?,
        }),
        "spot-on" => Ok(Event::SpotOn {
            seat: read.seat()?,
            odds: read.odds()?,
        }),
        "lucky" => Ok(Event::Lucky {
            seat: read.seat()?,
            odds: read.odds()?,
        }),
        "unlucky" => Ok(Event::Unlucky {
            seat: read.seat()?,
            odds: read.odds()?,
        }),
        "lose-die" => Ok(Event::LoseDie { seat: read.seat()? }),
        "gain-die" => Ok(Event::GainDie { seat: read.seat()? }),
        "gain-shield" => Ok(Event::GainShield { seat: read.seat()? }),
        "spend-shield" => Ok(Event::SpendShield { seat: read.seat()? }),
        "eliminate" => Ok(Event::Eliminate { seat: read.seat()? }),
        "next-turn" => Ok(Event::NextTurn { seat: read.seat()? }),
        "winner" => Ok(Event::Winner { seat: read.seat()? }),
        other => Err(Malformed::UnknownKind(other.to_string())),
    }
}

/// Reads attributes and names the kind in every complaint, so a failure says which event
/// at which position was unreadable rather than only which attribute.
struct Reader<'a> {
    kind: &'a str,
    item: &'a HashMap<String, AttributeValue>,
}

impl Reader<'_> {
    fn wrong(&self, attribute: &str) -> Malformed {
        Malformed::Field {
            kind: self.kind.to_string(),
            attribute: attribute.to_string(),
        }
    }

    fn text(&self, attribute: &str) -> Result<&str, Malformed> {
        self.item
            .get(attribute)
            .and_then(|value| value.as_s().ok())
            .map(String::as_str)
            .ok_or_else(|| self.wrong(attribute))
    }

    fn number<T: std::str::FromStr>(&self, attribute: &str) -> Result<T, Malformed> {
        self.item
            .get(attribute)
            .and_then(parse)
            .ok_or_else(|| self.wrong(attribute))
    }

    fn seat(&self) -> Result<Seat, Malformed> {
        Ok(Seat(self.number("seat")?))
    }

    fn odds(&self) -> Result<f64, Malformed> {
        self.number("odds")
    }

    fn list(&self, attribute: &str) -> Result<&Vec<AttributeValue>, Malformed> {
        self.item
            .get(attribute)
            .and_then(|value| value.as_l().ok())
            .ok_or_else(|| self.wrong(attribute))
    }
}

fn text(value: &str) -> AttributeValue {
    AttributeValue::S(value.to_string())
}

fn number(value: impl std::fmt::Display) -> AttributeValue {
    AttributeValue::N(value.to_string())
}

/// Odds are the one float in the log, written at full precision.
///
/// `{}` on an `f64` prints the shortest decimal that reads back as the same bits, so this
/// is exact rather than nearly exact. The archive is where odds get rounded, because that
/// is where somebody reads them.
fn chance(odds: f64) -> AttributeValue {
    AttributeValue::N(format!("{odds}"))
}

fn parse<T: std::str::FromStr>(value: &AttributeValue) -> Option<T> {
    value.as_n().ok()?.parse().ok()
}

fn seat_of(seat: Seat) -> AttributeValue {
    number(seat.0)
}

/// `discord:<snowflake>`, `ai` or `local`.
///
/// Built here rather than with `Account`'s `Display`, which passes the id through the
/// archive's `token` and would alter anything it considered unsafe for a line.
fn account_text(account: &Account) -> String {
    match account {
        Account::Discord(id) => format!("discord:{id}"),
        Account::Ai => "ai".to_string(),
        Account::Local => "local".to_string(),
    }
}

fn account_of(text: &str) -> Result<Account, Malformed> {
    match text {
        "ai" => Ok(Account::Ai),
        "local" => Ok(Account::Local),
        // Split once, so a colon in an id cannot change who the account is.
        other => match other.split_once(':') {
            Some(("discord", id)) => Ok(Account::Discord(id.to_string())),
            _ => Err(Malformed::UnknownAccount(other.to_string())),
        },
    }
}

/// The `&'static str` a stored variant name refers to.
///
/// `Event::Variant` holds a static name, so a name read out of the database has to be
/// matched against the ones the relay writes rather than leaked into one. An unknown name
/// is an error: it means this log was written by something that knew a variant this build
/// does not, and guessing which rules were in force is the one thing a replay must not do.
fn variant_named(name: &str) -> Result<&'static str, Malformed> {
    relay::VARIANTS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(known, _)| *known)
        .ok_or_else(|| Malformed::UnknownVariant(name.to_string()))
}
