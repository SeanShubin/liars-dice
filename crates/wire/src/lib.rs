//! One decomposition of an event or a message, and the renderings of it.
//!
//! `implementation/protocol.md` records a property worth keeping: *the format remains the
//! only representation, there is no second serialization to keep in step*. When it was
//! written nothing read a log back. The relay now reads its log back on every message and
//! sends events over a socket, so there are three places a value has to survive: a
//! DynamoDB item, a frame to a client, and a frame from one.
//!
//! Three mappings would be three places to drift. So there is one decomposition — a
//! [`Shape`]: a kind, and named [`Value`]s — and the renderings are mechanical. JSON lives
//! here, because a browser client and the service both speak it. The DynamoDB rendering
//! lives in `apps/relay`, because `AttributeValue` is an AWS type and the browser must
//! never compile it.
//!
//! The archive's line format is still not one of these. It is a rendering for a person:
//! it rounds odds to two places and collapses the spaces in a name, which is right for a
//! line somebody reads and wrong for the only copy of what happened.

use protocol::Account;

pub mod answer;
pub mod event;
pub mod examples;
pub mod json;
pub mod message;

/// The key naming the variant, in every rendering. Not a field, so nothing may have one
/// called this.
pub const KIND: &str = "kind";

/// One field of a shape.
///
/// Narrow on purpose: these are every kind of value the log and the message format
/// actually hold, so a rendering has a finite set of cases to answer for and a new one
/// cannot be added without every rendering being made to handle it.
#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    Text(String),
    /// An unsigned integer: a seat, a quantity, a face, a version, a position.
    Count(u64),
    /// The one float in the format. Written at full precision everywhere but the archive.
    Chance(f64),
    Flag(bool),
    /// Order and repeats both matter, so this is a list and never a set.
    Counts(Vec<u64>),
    /// Several strings as one field. Only the names in a listed match need it.
    ///
    /// Order matters: `A listed match shows its host` is the first element. Repeats
    /// are possible, because two players in one match may share a display name when
    /// neither of them chose it.
    ///
    /// Encoding these as one separated string was the alternative, and it is not
    /// available: a display name may hold any character, so there is no separator that
    /// cannot appear inside a name. That is the same reason a ticket's payload is
    /// length-prefixed rather than delimited.
    Texts(Vec<String>),
    /// A whole shape as a field. Only `Message::Claim` needs it, which carries an event.
    Nested(Box<Shape>),
    /// Several shapes as one field. Only the dashboard's listing needs it.
    ///
    /// An empty one renders as an empty array, which reads back as an empty [`Counts`]
    /// because a JSON array carries no element type and there is nothing in it to tell
    /// them apart. [`Shape::many`] accepts that, which is where the asymmetry is paid;
    /// [`Shape::counts`] is left exact, because an empty list of dice is not a thing the
    /// game produces and a reader should say so if it ever sees one.
    ///
    /// [`Counts`]: Value::Counts
    Many(Vec<Shape>),
}

/// What something is, and what it holds.
///
/// The kind is a `String` because a shape comes from outside as often as it is built
/// here, and the whole point of reading one is to find out whether it names something
/// this build knows.
#[derive(Clone, PartialEq, Debug)]
pub struct Shape {
    pub kind: String,
    pub fields: Vec<(String, Value)>,
}

/// Something that is not an event or a message this build can read.
///
/// Carries what was wrong rather than a unit, because the only time anyone reads one is
/// while looking at something that will not load, and the field name is the first thing
/// they need.
#[derive(Clone, PartialEq, Debug)]
pub enum Malformed {
    /// Not a shape at all: no kind, or the text was not the rendering it claimed to be.
    NotAShape(String),
    /// A kind this build does not know. Deliberately an error and not a skip: an event
    /// nobody can read is still an event that happened, and replaying a log without it
    /// would quietly produce a different match.
    UnknownKind(String),
    /// The kind was known but a field it needs is absent or the wrong type.
    Field { kind: String, field: String },
    /// A variant name the relay does not write. See `relay::VARIANTS`.
    UnknownVariant(String),
    /// An account that is neither a Discord id, an AI, nor the local player.
    UnknownAccount(String),
}

impl std::fmt::Display for Malformed {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Malformed::NotAShape(why) => write!(out, "not a shape: {why}"),
            Malformed::UnknownKind(kind) => write!(out, "unknown kind {kind}"),
            Malformed::Field { kind, field } => {
                write!(out, "{kind} is missing or misreads {field}")
            }
            Malformed::UnknownVariant(name) => write!(out, "unknown variant name {name}"),
            Malformed::UnknownAccount(account) => write!(out, "unknown account {account}"),
        }
    }
}

impl std::error::Error for Malformed {}

impl Shape {
    pub fn of(kind: &str, fields: Vec<(&str, Value)>) -> Self {
        Self {
            kind: kind.to_string(),
            fields: fields
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
        }
    }

    fn wrong(&self, field: &str) -> Malformed {
        Malformed::Field {
            kind: self.kind.clone(),
            field: field.to_string(),
        }
    }

    /// The shapes a field holds, or an empty slice when the field is an empty list.
    ///
    /// See [`Value::Many`] for why an empty one arrives as the other kind of list.
    pub fn many(&self, field: &str) -> Result<&[Shape], Malformed> {
        match self.get(field) {
            Some(Value::Many(shapes)) => Ok(shapes),
            Some(Value::Counts(counts)) if counts.is_empty() => Ok(&[]),
            _ => Err(self.wrong(field)),
        }
    }

    pub fn get(&self, field: &str) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, value)| value)
    }

    /// The text a field holds, or `None` when the shape does not carry the field.
    ///
    /// For a field a shape may legitimately omit, where [`Shape::text`] would refuse
    /// it. A field that is present and holds something other than text is still
    /// malformed: absent and wrong are different answers, and reading a wrong one as
    /// missing would let a mistyped field pass as an omitted one.
    pub fn maybe_text(&self, field: &str) -> Result<Option<&str>, Malformed> {
        match self.get(field) {
            None => Ok(None),
            Some(Value::Text(text)) => Ok(Some(text)),
            Some(_) => Err(self.wrong(field)),
        }
    }

    pub fn text(&self, field: &str) -> Result<&str, Malformed> {
        match self.get(field) {
            Some(Value::Text(text)) => Ok(text),
            _ => Err(self.wrong(field)),
        }
    }

    pub fn count<T: TryFrom<u64>>(&self, field: &str) -> Result<T, Malformed> {
        match self.get(field) {
            Some(Value::Count(count)) => T::try_from(*count).map_err(|_| self.wrong(field)),
            _ => Err(self.wrong(field)),
        }
    }

    /// Odds that happen to be whole may arrive as a count, which must not be refused.
    pub fn chance(&self, field: &str) -> Result<f64, Malformed> {
        match self.get(field) {
            Some(Value::Chance(odds)) => Ok(*odds),
            Some(Value::Count(whole)) => Ok(*whole as f64),
            _ => Err(self.wrong(field)),
        }
    }

    pub fn flag(&self, field: &str) -> Result<bool, Malformed> {
        match self.get(field) {
            Some(Value::Flag(flag)) => Ok(*flag),
            _ => Err(self.wrong(field)),
        }
    }

    /// The strings a field holds, or an empty slice when the field is an empty list.
    ///
    /// An empty one arrives as the other kind of list for the reason [`Value::Many`]
    /// gives: a JSON array carries no element type and an empty one says nothing about
    /// what was meant to be in it. A listed match always has at least its host's name,
    /// so this is tolerance rather than a case the relay produces.
    pub fn texts(&self, field: &str) -> Result<Vec<String>, Malformed> {
        match self.get(field) {
            Some(Value::Texts(texts)) => Ok(texts.clone()),
            Some(Value::Counts(counts)) if counts.is_empty() => Ok(Vec::new()),
            _ => Err(self.wrong(field)),
        }
    }

    pub fn counts<T: TryFrom<u64>>(&self, field: &str) -> Result<Vec<T>, Malformed> {
        match self.get(field) {
            Some(Value::Counts(counts)) => counts
                .iter()
                .map(|count| T::try_from(*count).map_err(|_| self.wrong(field)))
                .collect(),
            _ => Err(self.wrong(field)),
        }
    }

    pub fn nested(&self, field: &str) -> Result<&Shape, Malformed> {
        match self.get(field) {
            Some(Value::Nested(shape)) => Ok(shape),
            _ => Err(self.wrong(field)),
        }
    }
}

/// The canonical text of an account, which `protocol` owns.
///
/// Re-exported rather than written again: `relay` derives a match id from the same text
/// and cannot depend on this crate, so the form lives where the format lives.
pub use protocol::account_text;

/// [`account_text`] in reverse, with this crate's error.
pub fn account_of(text: &str) -> Result<Account, Malformed> {
    protocol::account_from_text(text).ok_or_else(|| Malformed::UnknownAccount(text.to_string()))
}

/// The `&'static str` a stored or received variant name refers to.
///
/// `Event::Variant` holds a static name, so a name from outside has to be matched against
/// the ones the relay writes rather than leaked into one. An unknown name is an error: it
/// means something knew a variant this build does not, and guessing which rules were in
/// force is the one thing a replay must never do.
pub fn variant_named(name: &str) -> Result<&'static str, Malformed> {
    relay::VARIANTS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(known, _)| *known)
        .ok_or_else(|| Malformed::UnknownVariant(name.to_string()))
}
