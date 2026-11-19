//! Reading an API Gateway WebSocket invocation. Needs nothing running.

use relay::ConnectionId;
use relay_service::incoming::{self, NotAnInvocation, Route};
use serde_json::json;

/// The envelope as the gateway sends it, trimmed to the fields this reads plus a few it
/// must ignore. Keeping the extras in is the point: the service reads five fields out of
/// a large event and must not care what else arrives.
fn event(route: &str, body: Option<&str>) -> serde_json::Value {
    let mut event = json!({
        "requestContext": {
            "routeKey": route,
            "connectionId": "Lq1dPcOgPHcCIbw=",
            "domainName": "abc123.execute-api.us-east-1.amazonaws.com",
            "stage": "live",
            "eventType": "MESSAGE",
            "messageDirection": "IN",
            "requestId": "Lq1dPhxZPHcFsOg=",
            "apiId": "abc123",
        },
        "isBase64Encoded": false,
    });
    if let Some(body) = body {
        event["body"] = json!(body);
    }
    event
}

#[test]
fn a_message_carries_its_route_its_connection_and_where_to_answer() {
    let read = incoming::read(&event("$default", Some(r#"{"kind":"start","match":"m"}"#)))
        .expect("an invocation");
    assert_eq!(read.route, Route::Default);
    assert_eq!(
        read.connection,
        ConnectionId("Lq1dPcOgPHcCIbw=".to_string())
    );
    // Built from the request context rather than configured: this is where the gateway
    // serving this particular connection lives.
    assert_eq!(
        read.endpoint,
        "https://abc123.execute-api.us-east-1.amazonaws.com/live"
    );
    assert_eq!(
        read.body.as_deref(),
        Some(r#"{"kind":"start","match":"m"}"#)
    );
}

#[test]
fn connect_and_disconnect_carry_no_body() {
    for (route, expected) in [
        ("$connect", Route::Connect),
        ("$disconnect", Route::Disconnect),
    ] {
        let read = incoming::read(&event(route, None)).expect("an invocation");
        assert_eq!(read.route, expected);
        assert_eq!(read.body, None);
    }
}

/// A route the gateway was configured with and this build was not. Louder than guessing
/// `$default`, which would hand an unparseable body to the relay.
#[test]
fn an_unknown_route_is_refused_rather_than_treated_as_a_message() {
    assert_eq!(
        incoming::read(&event("$sendmessage", Some("{}"))),
        Err(NotAnInvocation::UnknownRoute("$sendmessage".to_string()))
    );
}

#[test]
fn an_event_that_is_not_an_invocation_names_what_is_missing() {
    assert_eq!(
        incoming::read(&json!({})),
        Err(NotAnInvocation::Missing("requestContext"))
    );
    assert_eq!(
        incoming::read(&json!({"requestContext": {}})),
        Err(NotAnInvocation::Missing("routeKey"))
    );
    assert_eq!(
        incoming::read(&json!({"requestContext": {"routeKey": "$default"}})),
        Err(NotAnInvocation::Missing("connectionId"))
    );
}

/// The body is a string the client chose. Reading it is `wire`'s job and failing to read
/// it is not this layer's failure, so an unparseable body still reads as an invocation.
#[test]
fn a_body_is_carried_without_being_understood() {
    let read = incoming::read(&event("$default", Some("not json at all"))).expect("an invocation");
    assert_eq!(read.body.as_deref(), Some("not json at all"));
}
