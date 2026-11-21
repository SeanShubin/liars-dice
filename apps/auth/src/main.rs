//! The exchange, wired to a Lambda function URL and SSM.
//!
//! Composition root. No logic: the handler is `auth::Exchange`, the one outbound call is
//! `discord::Live`, and what lives here is reading the configuration, reading the clock,
//! and turning a function URL event into a [`auth::Request`].
//!
//! A cold start reads four parameters and builds the exchange; every invocation after that
//! hands it one request. Two composition stages, because the second cannot be wired until
//! the first has fetched — see /implementation/composition.md.

mod discord;

use auth::{Exchange, Login, Request, Response};
use integrity::Secret;
use lambda_runtime::{Error, LambdaEvent, service_fn};
use protocol::Moment;
use serde_json::{Value, json};

/// Where the configuration lives. Names rather than values, so the template decides.
const CLIENT_ID: &str = "CLIENT_ID_PARAMETER";
const CLIENT_SECRET: &str = "CLIENT_SECRET_PARAMETER";
const REDIRECT_URI: &str = "REDIRECT_URI_PARAMETER";
const TICKET_SECRET: &str = "TICKET_SECRET_PARAMETER";

#[tokio::main]
async fn main() -> Result<(), Error> {
    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let ssm = aws_sdk_ssm::Client::new(&config);

    // Nothing here fails the cold start. `A missing Discord config disables login rather
    // than failing`, and a service that refused to start without it could not be deployed
    // before the parameters were set — which `deployment.md` requires.
    let login = read_login(&ssm).await;
    let ticket_secret = read_secret(&ssm, TICKET_SECRET).await;
    if login.is_none() || ticket_secret.is_none() {
        // Said once, at the cold start, so a page being told *not configured* can be
        // explained without guessing which parameter is missing.
        eprintln!(
            "login is off: discord config {}, ticket secret {}",
            present(login.is_some()),
            present(ticket_secret.is_some())
        );
    }
    let discord = discord::Live::new().map_err(Error::from)?;
    // An `Arc` so the handler can be moved into `spawn_blocking`, which needs what it is
    // given to outlive the frame that gave it. The exchange holds no mutable state, so
    // sharing it costs nothing.
    let exchange = std::sync::Arc::new(Exchange::new(login, ticket_secret, discord));

    lambda_runtime::run(service_fn(move |event: LambdaEvent<Value>| {
        let exchange = exchange.clone();
        async move {
            let request = request_of(&event.payload);
            let now = now();
            // The handler is synchronous and the call to Discord inside it blocks, which
            // is legal here and nowhere else: `spawn_blocking` hands the closure to a
            // thread that is not a runtime worker. The same seam `apps/relay` uses.
            let response =
                tokio::task::spawn_blocking(move || exchange.serve(request, now)).await?;
            Ok::<Value, Error>(reply(&response))
        }
    }))
    .await
}

fn present(yes: bool) -> &'static str {
    if yes { "present" } else { "missing" }
}

/// The system clock, read once per invocation, in the composition root.
///
/// The only observation of time in this service. It decides when a minted ticket expires
/// and nothing else, and it never reaches a log.
fn now() -> Moment {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or_default();
    Moment::from_unix_nanos(nanos)
}

/// All three Discord parameters, or none of them.
///
/// A client id without a secret cannot complete a flow, so a partial configuration is the
/// same as none — and answering *configured* on one would give the page a sign-in button
/// that could not work, which is the one thing that must never happen.
async fn read_login(ssm: &aws_sdk_ssm::Client) -> Option<Login> {
    Some(Login {
        client_id: read_parameter(ssm, CLIENT_ID).await?,
        client_secret: read_parameter(ssm, CLIENT_SECRET).await?,
        redirect_uri: read_parameter(ssm, REDIRECT_URI).await?,
    })
}

async fn read_secret(ssm: &aws_sdk_ssm::Client, variable: &str) -> Option<Secret> {
    let text = read_parameter(ssm, variable).await?;
    let secret = Secret::from_hex(text.trim());
    if secret.is_none() {
        // Named, not printed. The value is a secret even when it is the wrong shape.
        eprintln!("{variable} does not hold 64 hexadecimal digits");
    }
    secret
}

/// One parameter, by the name an environment variable gives.
///
/// Decrypted where it is encrypted, which is what `with_decryption` asks for; a plain
/// parameter is unaffected by it.
async fn read_parameter(ssm: &aws_sdk_ssm::Client, variable: &str) -> Option<String> {
    let name = std::env::var(variable).ok()?;
    let answer = ssm
        .get_parameter()
        .name(&name)
        .with_decryption(true)
        .send()
        .await;
    match answer {
        Ok(answer) => answer.parameter?.value,
        Err(error) => {
            eprintln!("cannot read {name}: {error}");
            None
        }
    }
}

/// A function URL event as a request.
///
/// The method is all the routing there is: this service has two verbs and no paths. A
/// body that is not JSON, or JSON without a `code`, is an empty code — which `serve`
/// answers with *no code* rather than treating as a Discord failure.
fn request_of(payload: &Value) -> Request {
    let method = payload
        .pointer("/requestContext/http/method")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    match method {
        "GET" => Request::Where,
        "POST" => {
            let code = payload
                .get("body")
                .and_then(|value| value.as_str())
                .and_then(|body| serde_json::from_str::<Value>(body).ok())
                .and_then(|body| {
                    body.get("code")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            Request::Trade { code }
        }
        _ => Request::Unknown,
    }
}

/// What a function URL wants back.
///
/// `Access-Control-Allow-Origin` is set by the function URL's own CORS configuration in
/// the template, not here, so that one place decides who may call this.
fn reply(response: &Response) -> Value {
    json!({
        "statusCode": response.status,
        "headers": { "content-type": "application/json" },
        "body": response.body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A function URL event is read for its method and its code, and nothing else.
    #[test]
    fn a_request_is_its_method_and_its_code() {
        let get = json!({ "requestContext": { "http": { "method": "GET" } } });
        assert_eq!(request_of(&get), Request::Where);

        let post = json!({
            "requestContext": { "http": { "method": "POST" } },
            "body": "{\"code\":\"abc123\"}",
        });
        assert_eq!(
            request_of(&post),
            Request::Trade {
                code: "abc123".to_string()
            }
        );

        // A POST with nothing usable in it is an empty code, not an unknown request: the
        // caller did ask to trade, and `serve` has something specific to say about it.
        for body in [
            json!(null),
            json!("not json"),
            json!("{}"),
            json!("{\"code\":7}"),
        ] {
            let odd = json!({
                "requestContext": { "http": { "method": "POST" } },
                "body": body,
            });
            assert_eq!(
                request_of(&odd),
                Request::Trade {
                    code: String::new()
                },
                "{body}"
            );
        }

        let put = json!({ "requestContext": { "http": { "method": "PUT" } } });
        assert_eq!(request_of(&put), Request::Unknown);
        assert_eq!(request_of(&json!({})), Request::Unknown);
    }
}
