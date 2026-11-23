//! A [`Shape`] as a DynamoDB item, and back.
//!
//! Realizes the item format in /implementation/relay-service.md. One attribute for the
//! kind, then one per field. Numbers stay numbers so the database can compare them, and
//! nothing is rounded or collapsed on the way in.
//!
//! This is a rendering of `wire::Shape` and not a second decomposition of an `Event`.
//! There are three places a value has to survive — this item, a frame to a client, and a
//! frame from one — and three independent mappings would be three places to drift. The
//! decomposition lives in `crates/wire` next to the JSON rendering; what lives here is
//! only the part the browser must never compile, because `AttributeValue` is an AWS type.
//!
//! The archive's line format is still not one of these. It rounds odds to two places and
//! collapses the spaces in a name, which is right for a line a person reads and wrong for
//! the only copy of what happened.

use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue;
use protocol::{Event, Moment};
use wire::{KIND, Malformed, Shape, Value, event};

/// The partition key: which match this event belongs to.
pub const MATCH: &str = "match";
/// The sort key: where in the log it sits. The conditional write is on this.
pub const POSITION: &str = "position";
/// The account a `join` row names. The hash key of `by-account`.
///
/// Only a `join` carries one, which is what makes the index sparse: it holds one row per
/// seat ever taken rather than one per event.
pub const ACCOUNT: &str = "account";
/// The index that answers which matches an account has joined.
pub const BY_ACCOUNT: &str = "by-account";
/// When the relay recorded this row, in unix nanoseconds.
///
/// `The relay records when it received each event`, and deliberately beside the event
/// rather than inside it: `event_from_item` excludes this attribute along with the key, so
/// an `Event` never carries it and no fold can see it. A time in an event would reach the
/// archive's line format, every golden-file test and every client's view of the match.
pub const RECORDED: &str = "recorded";

/// The key of one event, for a Query or a conditional Put.
pub fn key(match_id: &str, position: u64) -> HashMap<String, AttributeValue> {
    HashMap::from([
        (MATCH.to_string(), AttributeValue::S(match_id.to_string())),
        (
            POSITION.to_string(),
            AttributeValue::N(position.to_string()),
        ),
    ])
}

/// The whole item: the key, when it was recorded, then the event.
pub fn item(
    match_id: &str,
    position: u64,
    event: &Event,
    now: Moment,
) -> HashMap<String, AttributeValue> {
    let mut item = key(match_id, position);
    item.insert(
        RECORDED.to_string(),
        AttributeValue::N(now.to_unix_nanos().to_string()),
    );
    item.extend(written(&event::shape_of(event)));
    item
}

/// The event an item describes, or why it is not one.
pub fn event_from_item(item: &HashMap<String, AttributeValue>) -> Result<Event, Malformed> {
    event::from_shape(&shape_from_item(item, &[MATCH, POSITION, RECORDED])?)
}

/// When an item was recorded, or `None` for one written before times were recorded.
pub fn recorded_at(item: &HashMap<String, AttributeValue>) -> Option<Moment> {
    let nanos = item.get(RECORDED)?.as_n().ok()?.parse().ok()?;
    Some(Moment::from_unix_nanos(nanos))
}

/// A shape's attributes, without the key.
pub fn written(shape: &Shape) -> HashMap<String, AttributeValue> {
    let mut item = HashMap::from([(KIND.to_string(), AttributeValue::S(shape.kind.clone()))]);
    for (name, value) in &shape.fields {
        item.insert(name.clone(), rendered(value));
    }
    item
}

fn rendered(value: &Value) -> AttributeValue {
    match value {
        Value::Text(text) => AttributeValue::S(text.clone()),
        Value::Count(count) => AttributeValue::N(count.to_string()),
        // Full precision. `{}` on an `f64` prints the shortest decimal that reads back as
        // the same bits, so this is exact rather than nearly exact.
        Value::Chance(odds) => AttributeValue::N(format!("{odds}")),
        Value::Flag(flag) => AttributeValue::Bool(*flag),
        Value::Counts(counts) => AttributeValue::L(
            counts
                .iter()
                .map(|count| AttributeValue::N(count.to_string()))
                .collect(),
        ),
        Value::Texts(texts) => AttributeValue::L(
            texts
                .iter()
                .map(|text| AttributeValue::S(text.clone()))
                .collect(),
        ),
        Value::Nested(shape) => AttributeValue::M(written(shape)),
        // No event holds one, so nothing with this in it is ever stored — but the
        // rendering is total on purpose: a `Value` the codec cannot answer for would be a
        // kind of frame that silently could not be archived.
        Value::Many(shapes) => AttributeValue::L(
            shapes
                .iter()
                .map(|shape| AttributeValue::M(written(shape)))
                .collect(),
        ),
    }
}

/// What an item says it is, before anything has agreed it is an event.
///
/// `ignoring` names the attributes that say where the item is rather than what it holds,
/// which for a log item is the key. It is a parameter rather than a constant because this
/// reader must not guess: `Message::Join` has a field genuinely called `match`, and a
/// reader that assumed the word always meant the partition key dropped it silently. That
/// shape is never stored, so nothing in production would have found it.
pub fn shape_from_item(
    item: &HashMap<String, AttributeValue>,
    ignoring: &[&str],
) -> Result<Shape, Malformed> {
    let kind = item
        .get(KIND)
        .and_then(|value| value.as_s().ok())
        .ok_or_else(|| Malformed::NotAShape("no kind attribute".to_string()))?;
    let mut fields = Vec::new();
    for (name, value) in item {
        if name == KIND || ignoring.contains(&name.as_str()) {
            continue;
        }
        fields.push((name.clone(), parsed(value, kind, name)?));
    }
    Ok(Shape {
        kind: kind.clone(),
        fields,
    })
}

/// A DynamoDB number carries no type, so the value decides: a whole number that fits is a
/// count, and anything else is odds. [`wire::Shape::chance`] accepts either, so reading
/// odds that happen to be whole as a count costs nothing.
fn parsed(value: &AttributeValue, kind: &str, field: &str) -> Result<Value, Malformed> {
    let wrong = || Malformed::Field {
        kind: kind.to_string(),
        field: field.to_string(),
    };
    match value {
        AttributeValue::S(text) => Ok(Value::Text(text.clone())),
        AttributeValue::Bool(flag) => Ok(Value::Flag(*flag)),
        AttributeValue::N(number) => match number.parse::<u64>() {
            Ok(count) => Ok(Value::Count(count)),
            Err(_) => number
                .parse::<f64>()
                .map(Value::Chance)
                .map_err(|_| wrong()),
        },
        // Shapes before numbers: a list of maps is a list of shapes, and the guard is
        // what makes the arm below unreachable for them rather than wrong about them.
        AttributeValue::L(items)
            if !items.is_empty() && items.iter().all(|item| item.as_m().is_ok()) =>
        {
            items
                .iter()
                .map(|item| {
                    let map = item.as_m().map_err(|_| wrong())?;
                    shape_from_item(map, &[])
                })
                .collect::<Result<Vec<Shape>, Malformed>>()
                .map(Value::Many)
        }
        // Strings before numbers, for the same reason shapes come before both: a list
        // of strings read as a list of counts would fail on every element rather than
        // being read as what it is.
        AttributeValue::L(items)
            if !items.is_empty() && items.iter().all(|item| item.as_s().is_ok()) =>
        {
            items
                .iter()
                .map(|item| item.as_s().cloned().map_err(|_| wrong()))
                .collect::<Result<Vec<String>, Malformed>>()
                .map(Value::Texts)
        }
        AttributeValue::L(values) => values
            .iter()
            .map(|value| {
                value
                    .as_n()
                    .ok()
                    .and_then(|number| number.parse::<u64>().ok())
                    .ok_or_else(wrong)
            })
            .collect::<Result<Vec<u64>, Malformed>>()
            .map(Value::Counts),
        AttributeValue::M(nested) => {
            shape_from_item(nested, &[]).map(|shape| Value::Nested(Box::new(shape)))
        }
        _ => Err(wrong()),
    }
}
