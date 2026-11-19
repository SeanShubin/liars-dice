//! What an API Gateway WebSocket invocation says, and nothing more.
//!
//! Read out of the event by hand rather than through a typed events crate. Five fields
//! are wanted out of a large envelope, the envelope is documented and stable, and a
//! dependency that models all of it would be a dependency that has to agree with AWS
//! about the parts this never reads.

use relay::ConnectionId;
use serde_json::Value;

/// Which route the gateway matched. `$default` carries everything a client said.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Route {
    Connect,
    Disconnect,
    Default,
}

/// One invocation, reduced to what the service acts on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Incoming {
    pub route: Route,
    pub connection: ConnectionId,
    /// Where to send answers: `https://<domain>/<stage>`, built from the request context
    /// rather than configured, because that is where this connection's gateway is.
    pub endpoint: String,
    /// Absent on `$connect` and `$disconnect`, which carry no body.
    pub body: Option<String>,
}

/// The event was not a WebSocket invocation this service can serve.
///
/// Names the field, because the only time anyone reads one of these is while looking at
/// an invocation that should have worked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NotAnInvocation {
    Missing(&'static str),
    UnknownRoute(String),
}

impl std::fmt::Display for NotAnInvocation {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotAnInvocation::Missing(field) => write!(out, "the event has no {field}"),
            NotAnInvocation::UnknownRoute(route) => write!(out, "unknown route {route}"),
        }
    }
}

impl std::error::Error for NotAnInvocation {}

pub fn read(event: &Value) -> Result<Incoming, NotAnInvocation> {
    let context = event
        .get("requestContext")
        .ok_or(NotAnInvocation::Missing("requestContext"))?;
    let text = |field: &'static str| -> Result<&str, NotAnInvocation> {
        context
            .get(field)
            .and_then(Value::as_str)
            .ok_or(NotAnInvocation::Missing(field))
    };
    let route = match text("routeKey")? {
        "$connect" => Route::Connect,
        "$disconnect" => Route::Disconnect,
        "$default" => Route::Default,
        // A route the gateway was configured with and this was not. Louder than guessing
        // `$default`, which would hand an unparseable body to the relay.
        other => return Err(NotAnInvocation::UnknownRoute(other.to_string())),
    };
    Ok(Incoming {
        route,
        connection: ConnectionId(text("connectionId")?.to_string()),
        endpoint: format!("https://{}/{}", text("domainName")?, text("stage")?),
        body: event
            .get("body")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}
