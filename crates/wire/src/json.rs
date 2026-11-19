//! A [`Shape`] as JSON, which is what crosses the socket.
//!
//! `{"kind": "claim", "seat": 3, "quantity": 2, "face": 6, "odds": 38.207}` — the kind
//! names the variant and every other key is one of its fields. Flat, except for the
//! event a claim carries.
//!
//! JSON because every tool on the path already reads it: API Gateway, CloudWatch, and
//! whatever eventually connects that is not this Rust client. `serde_json` parses the
//! text, because escapes and unicode in a hand-rolled parser are a bug waiting for a
//! player with an interesting name. Nothing in `protocol` or `relay` derives anything:
//! the mapping is [`Shape`], and those crates keep their empty dependency lists.
//!
//! Keys are written in alphabetical order, which `serde_json`'s map does by default and
//! which is worth keeping: a frame's bytes are then a function of what the shape holds,
//! and not of the order somebody happened to declare the fields in. The alternative buys
//! `kind` appearing first in a log line, for a dependency and a weaker property.

use serde_json::{Map, Number, Value as Json};

use crate::{KIND, Malformed, Shape, Value};

pub fn write(shape: &Shape) -> String {
    object(shape).to_string()
}

fn object(shape: &Shape) -> Json {
    let mut map = Map::new();
    map.insert(KIND.to_string(), Json::String(shape.kind.clone()));
    for (name, value) in &shape.fields {
        map.insert(name.clone(), rendered(value));
    }
    Json::Object(map)
}

fn rendered(value: &Value) -> Json {
    match value {
        Value::Text(text) => Json::String(text.clone()),
        Value::Count(count) => Json::Number(Number::from(*count)),
        // Full precision. `serde_json` writes the shortest decimal that reads back as the
        // same bits, so this is exact rather than nearly exact. The archive is where odds
        // are rounded, because that is where a person reads them.
        Value::Chance(odds) => Number::from_f64(*odds)
            .map(Json::Number)
            // Not reachable from any odds the game computes, and a null is a readable
            // failure where a silent zero would be a plausible one.
            .unwrap_or(Json::Null),
        Value::Flag(flag) => Json::Bool(*flag),
        Value::Counts(counts) => Json::Array(
            counts
                .iter()
                .map(|count| Json::Number(Number::from(*count)))
                .collect(),
        ),
        Value::Nested(shape) => object(shape),
    }
}

pub fn read(text: &str) -> Result<Shape, Malformed> {
    let json: Json =
        serde_json::from_str(text).map_err(|why| Malformed::NotAShape(why.to_string()))?;
    shape(&json)
}

fn shape(json: &Json) -> Result<Shape, Malformed> {
    let map = json
        .as_object()
        .ok_or_else(|| Malformed::NotAShape("not an object".to_string()))?;
    let kind = map
        .get(KIND)
        .and_then(Json::as_str)
        .ok_or_else(|| Malformed::NotAShape("no kind".to_string()))?;
    let mut fields = Vec::new();
    for (name, value) in map {
        if name == KIND {
            continue;
        }
        fields.push((name.clone(), parsed(value, kind, name)?));
    }
    Ok(Shape {
        kind: kind.to_string(),
        fields,
    })
}

/// A JSON number carries no type, so the value decides: a whole number that fits is a
/// count, and anything else is odds.
///
/// Reading `38.0` as a count is harmless because [`Shape::chance`] accepts one; reading
/// `38.207` as a count would lose the point of storing it, so it does not.
fn parsed(json: &Json, kind: &str, field: &str) -> Result<Value, Malformed> {
    let wrong = || Malformed::Field {
        kind: kind.to_string(),
        field: field.to_string(),
    };
    match json {
        Json::String(text) => Ok(Value::Text(text.clone())),
        Json::Bool(flag) => Ok(Value::Flag(*flag)),
        Json::Number(number) => match number.as_u64() {
            Some(count) => Ok(Value::Count(count)),
            None => number.as_f64().map(Value::Chance).ok_or_else(wrong),
        },
        Json::Array(values) => values
            .iter()
            .map(|value| value.as_u64().ok_or_else(wrong))
            .collect::<Result<Vec<u64>, Malformed>>()
            .map(Value::Counts),
        Json::Object(_) => shape(json).map(|nested| Value::Nested(Box::new(nested))),
        Json::Null => Err(wrong()),
    }
}
