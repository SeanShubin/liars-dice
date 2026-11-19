//! The relay service: the composition root that will wire `crates/relay` to API Gateway
//! WebSocket, DynamoDB for the log and the connection table, and SSM for the secret.
//!
//! The storage is written and tested; the three handlers are not. See
//! /implementation/relay-service.md, and `relay::memory` for the arrangement that
//! already plays a whole match in one process with none of this.
//!
//! Composition root. No behavior.

fn main() {}
