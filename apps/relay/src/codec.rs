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
use protocol::Event;
use wire::{KIND, Malformed, Shape, Value, event};

/// The partition key: which match this event belongs to.
pub const MATCH: &str = "match";
/// The sort key: where in the log it sits. The conditional write is on this.
pub const POSITION: &str = "position";

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

/// The whole item: the key, then the event.
pub fn item(match_id: &str, position: u64, event: &Event) -> HashMap<String, AttributeValue> {
    let mut item = key(match_id, position);
    item.extend(written(&event::shape_of(event)));
    item
}

/// The event an item describes, or why it is not one.
pub fn event_from_item(item: &HashMap<String, AttributeValue>) -> Result<Event, Malformed> {
    event::from_shape(&shape_from_item(item, &[MATCH, POSITION])?)
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
        Value::Nested(shape) => AttributeValue::M(written(shape)),
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
