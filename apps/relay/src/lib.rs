//! The deployed half of the relay: storage, and what it cost.
//!
//! Realizes /implementation/relay-service.md. `crates/relay` is the state machine and
//! knows nothing about any of this; here are the two trait implementations it is wired
//! to in production, and the meter that will say when the lobby list needs replacing.
//!
//! Knows no game rules, and tests/boundaries.rs holds this crate to that. A composition
//! root is exactly where someone would reach for `rules` to answer a question the relay
//! is not allowed to ask.

pub mod codec;
pub mod connections;
pub mod event_log;
pub mod meter;

pub use connections::GatewayConnections;
pub use event_log::DynamoLog;
pub use meter::Meter;

/// The CloudWatch namespace the metric line reports under.
pub const NAMESPACE: &str = "LiarsDice/Relay";
