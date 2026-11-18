//! The relay service: the composition root that will wire `crates/relay` to API Gateway
//! WebSocket, DynamoDB for the log and the connection table, and SSM for the secret.
//!
//! The behaviour lives in `crates/relay`, which knows nothing about any of those. Until
//! this is wired, a whole match already runs in one process over `relay::memory`; see
//! crates/relay/tests/matches.rs.
//!
//! Composition root. No behavior.

fn main() {}
