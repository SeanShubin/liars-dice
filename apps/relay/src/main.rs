//! The relay service: `crates/relay` wired to API Gateway WebSocket, DynamoDB and SSM.
//!
//! Composition root. No behavior: the state machine is `crates/relay`, the storage is
//! `event_log` and `connections`, and the three routes are `service`.
//!
//! A cold start builds the clients and reads the secret; every invocation after that
//! constructs a relay over them, hands it one message, sends what it returns, and drops
//! it. That is not a compromise — `A relay may be stopped and replaced between any two
//! messages` is already a tested property of the library, and this is that test running
//! in production on every message.

use lambda_runtime::{Error, LambdaEvent, service_fn};
use relay_service::{Service, incoming};
use serde_json::{Value, json};

#[tokio::main]
async fn main() -> Result<(), Error> {
    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    // Failing here fails the cold start, which is right: a service that cannot read its
    // secret must not serve a single message rather than serve them unverifiably.
    let service = Service::new(config).await?;

    lambda_runtime::run(service_fn(|event: LambdaEvent<Value>| {
        let service = &service;
        async move {
            let incoming = incoming::read(&event.payload)?;
            let status = service.serve(incoming).await?;
            Ok::<Value, Error>(json!({ "statusCode": status }))
        }
    }))
    .await
}
