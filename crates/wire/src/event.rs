//! An [`Event`] as a [`Shape`], and back.
//!
//! [`shape_of`] matches on `Event` with no wildcard arm, so a new variant in `protocol`
//! stops this compiling. That forces it to be *handled*; only the round-trip list in
//! tests/round_trip.rs forces it to be handled correctly, so add it there in the same
//! change.

use protocol::{Event, Moment, Seat};

use crate::{Malformed, Shape, Value, account_of, account_text, variant_named};

fn seat(seat: Seat) -> Value {
    Value::Count(u64::from(seat.0))
}

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

pub fn shape_of(event: &Event) -> Shape {
    // One arm per variant and no wildcard: this is where `protocol` gaining an event
    // becomes a compile error rather than an event that renders as nothing.
    match event {
        Event::Version { protocol } => Shape::of(
            "version",
            vec![("protocol", Value::Count(u64::from(*protocol)))],
        ),
        Event::Started { at } => {
            Shape::of("started", vec![("at", Value::Count(at.to_unix_nanos()))])
        }
        Event::Host { seat: at } => Shape::of("host", vec![("seat", seat(*at))]),
        Event::Bar { seat: at } => Shape::of("bar", vec![("seat", seat(*at))]),
        Event::Lift { seat: at } => Shape::of("lift", vec![("seat", seat(*at))]),
        Event::Left { seat: at } => Shape::of("left", vec![("seat", seat(*at))]),
        Event::Start => Shape::of("start", vec![]),
        Event::RoundOver { seat: at } => Shape::of("round-over", vec![("seat", seat(*at))]),
        Event::Join {
            seat: at,
            name,
            colour,
            account,
        } => Shape::of(
            "join",
            vec![
                ("seat", seat(*at)),
                ("name", text(name)),
                ("colour", text(colour)),
                ("account", text(&account_text(account))),
            ],
        ),
        Event::PlayerOrder(order) => Shape::of(
            "player-order",
            vec![(
                "order",
                Value::Counts(order.iter().map(|at| u64::from(at.0)).collect()),
            )],
        ),
        Event::Variant { name, on } => Shape::of(
            "variant",
            vec![("name", text(name)), ("on", Value::Flag(*on))],
        ),
        Event::Commit { digest } => Shape::of("commit", vec![("digest", text(digest))]),
        Event::Reveal { seed } => Shape::of("reveal", vec![("seed", text(seed))]),
        Event::Deal { seat: at, dice } => Shape::of(
            "deal",
            vec![
                ("seat", seat(*at)),
                (
                    "dice",
                    Value::Counts(dice.iter().map(|face| u64::from(*face)).collect()),
                ),
            ],
        ),
        Event::Claim {
            seat: at,
            quantity,
            face,
            odds,
        } => Shape::of(
            "claim",
            vec![
                ("seat", seat(*at)),
                ("quantity", Value::Count(u64::from(*quantity))),
                ("face", Value::Count(u64::from(*face))),
                ("odds", Value::Chance(*odds)),
            ],
        ),
        Event::Liar { seat: at, odds } => Shape::of(
            "liar",
            vec![("seat", seat(*at)), ("odds", Value::Chance(*odds))],
        ),
        Event::SpotOn { seat: at, odds } => Shape::of(
            "spot-on",
            vec![("seat", seat(*at)), ("odds", Value::Chance(*odds))],
        ),
        Event::Lucky { seat: at, odds } => Shape::of(
            "lucky",
            vec![("seat", seat(*at)), ("odds", Value::Chance(*odds))],
        ),
        Event::Unlucky { seat: at, odds } => Shape::of(
            "unlucky",
            vec![("seat", seat(*at)), ("odds", Value::Chance(*odds))],
        ),
        Event::LoseDie { seat: at } => Shape::of("lose-die", vec![("seat", seat(*at))]),
        Event::GainDie { seat: at } => Shape::of("gain-die", vec![("seat", seat(*at))]),
        Event::GainShield { seat: at } => Shape::of("gain-shield", vec![("seat", seat(*at))]),
        Event::SpendShield { seat: at } => Shape::of("spend-shield", vec![("seat", seat(*at))]),
        Event::Eliminate { seat: at } => Shape::of("eliminate", vec![("seat", seat(*at))]),
        Event::NextTurn { seat: at } => Shape::of("next-turn", vec![("seat", seat(*at))]),
        Event::Winner { seat: at } => Shape::of("winner", vec![("seat", seat(*at))]),
    }
}

pub fn from_shape(shape: &Shape) -> Result<Event, Malformed> {
    let at = || -> Result<Seat, Malformed> { Ok(Seat(shape.count("seat")?)) };
    match shape.kind.as_str() {
        "version" => Ok(Event::Version {
            protocol: shape.count("protocol")?,
        }),
        "started" => Ok(Event::Started {
            at: Moment::from_unix_nanos(shape.count("at")?),
        }),
        "host" => Ok(Event::Host { seat: at()? }),
        "bar" => Ok(Event::Bar { seat: at()? }),
        "lift" => Ok(Event::Lift { seat: at()? }),
        "left" => Ok(Event::Left { seat: at()? }),
        "start" => Ok(Event::Start),
        "round-over" => Ok(Event::RoundOver { seat: at()? }),
        "join" => Ok(Event::Join {
            seat: at()?,
            name: shape.text("name")?.to_string(),
            colour: shape.text("colour")?.to_string(),
            account: account_of(shape.text("account")?)?,
        }),
        "player-order" => Ok(Event::PlayerOrder(
            shape.counts("order")?.into_iter().map(Seat).collect(),
        )),
        "variant" => Ok(Event::Variant {
            name: variant_named(shape.text("name")?)?,
            on: shape.flag("on")?,
        }),
        "commit" => Ok(Event::Commit {
            digest: shape.text("digest")?.to_string(),
        }),
        "reveal" => Ok(Event::Reveal {
            seed: shape.text("seed")?.to_string(),
        }),
        "deal" => Ok(Event::Deal {
            seat: at()?,
            dice: shape.counts("dice")?,
        }),
        "claim" => Ok(Event::Claim {
            seat: at()?,
            quantity: shape.count("quantity")?,
            face: shape.count("face")?,
            odds: shape.chance("odds")?,
        }),
        "liar" => Ok(Event::Liar {
            seat: at()?,
            odds: shape.chance("odds")?,
        }),
        "spot-on" => Ok(Event::SpotOn {
            seat: at()?,
            odds: shape.chance("odds")?,
        }),
        "lucky" => Ok(Event::Lucky {
            seat: at()?,
            odds: shape.chance("odds")?,
        }),
        "unlucky" => Ok(Event::Unlucky {
            seat: at()?,
            odds: shape.chance("odds")?,
        }),
        "lose-die" => Ok(Event::LoseDie { seat: at()? }),
        "gain-die" => Ok(Event::GainDie { seat: at()? }),
        "gain-shield" => Ok(Event::GainShield { seat: at()? }),
        "spend-shield" => Ok(Event::SpendShield { seat: at()? }),
        "eliminate" => Ok(Event::Eliminate { seat: at()? }),
        "next-turn" => Ok(Event::NextTurn { seat: at()? }),
        "winner" => Ok(Event::Winner { seat: at()? }),
        other => Err(Malformed::UnknownKind(other.to_string())),
    }
}
