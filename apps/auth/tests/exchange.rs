//! Every answer the exchange can give, against a stubbed Discord.
//!
//! The whole handler is reachable without a network, which is what the [`auth::Discord`]
//! seam is for: the exchange decides when to call Discord and with what, so it is a trait
//! and not a value, and a trait is a thing a test can supply.
//!
//! The one thing not tested here is the Discord redirect itself. It needs a browser, a
//! real application registration and a person to click *authorize*, so it is checked by
//! hand once and then by the fact that a ranked match is possible at all.

use std::cell::RefCell;
use std::rc::Rc;

use auth::{Discord, Exchange, Identified, Login, Request, Trouble};
use integrity::{Secret, TICKET_SECONDS};
use protocol::Moment;

fn secret() -> Secret {
    Secret::from_bytes([41; 32])
}

fn login() -> Login {
    Login {
        client_id: "1234567890".to_string(),
        client_secret: "not the real one".to_string(),
        redirect_uri: "https://seanshubin.com/liars-dice/multiplayer/".to_string(),
    }
}

fn moment() -> Moment {
    Moment::from_unix_nanos(1_700_000_000_000_000_000)
}

fn seconds() -> u64 {
    moment().to_unix_nanos() / 1_000_000_000
}

/// What a stub was asked, shared with the test that built it.
///
/// Shared rather than reachable through the exchange: the exchange owns its `Discord` and
/// should not grow an accessor for a test's benefit. The stub and the test hold the same
/// record, which is what lets one check what the other sent.
type Asked = Rc<RefCell<Vec<(Login, String)>>>;

/// A Discord that answers whatever a test told it to, and records what it was asked.
struct Stub {
    answer: Result<Identified, Trouble>,
    asked: Asked,
}

impl Stub {
    fn saying(answer: Result<Identified, Trouble>) -> Self {
        Self {
            answer,
            asked: Asked::default(),
        }
    }

    fn identifying(id: &str, name: &str) -> Self {
        Self::saying(Ok(Identified {
            id: id.to_string(),
            name: name.to_string(),
            // Deliberately not the display name. A test that used one string for both
            // would pass against an exchange that minted the wrong one into the ticket.
            username: format!("{}_handle", name.to_lowercase()),
        }))
    }
}

impl Discord for Stub {
    fn identify(&self, login: &Login, code: &str) -> Result<Identified, Trouble> {
        self.asked
            .borrow_mut()
            .push((login.clone(), code.to_string()));
        self.answer.clone()
    }
}

/// A Discord that must not be called at all.
struct Never;

impl Discord for Never {
    fn identify(&self, _login: &Login, _code: &str) -> Result<Identified, Trouble> {
        panic!("Discord was called when it should not have been");
    }
}

fn body(response: &auth::Response) -> serde_json::Value {
    serde_json::from_str(&response.body).expect("a json body")
}

/// A code becomes a ticket the relay will verify, for the account Discord named.
#[test]
fn a_code_becomes_a_ticket_for_the_account_discord_named() {
    let discord = Stub::identifying("450853396908343307", "Ada Lovelace");
    let exchange = Exchange::new(Some(login()), Some(secret()), discord);

    let response = exchange.serve(
        Request::Trade {
            code: "a-real-code".to_string(),
        },
        moment(),
    );
    assert_eq!(response.status, 200);
    let answer = body(&response);

    // The ticket is what the relay will read, so it is checked the way the relay checks
    // it: with `check_ticket`, under the same secret, at the same moment.
    let ticket = answer["ticket"].as_str().expect("a ticket");
    let proved = integrity::check_ticket(&secret(), ticket, seconds()).expect("it verifies");
    assert_eq!(proved.account, "discord:450853396908343307");
    assert_eq!(proved.name, "Ada Lovelace");

    // And the two fields repeated outside it, for a page that cannot read the ticket.
    assert_eq!(answer["account"], "discord:450853396908343307");
    assert_eq!(answer["name"], "Ada Lovelace");
    assert_eq!(answer["expiresAt"], seconds() + TICKET_SECONDS);
}

/// The ticket expires a day on, and not before.
#[test]
fn a_minted_ticket_lasts_exactly_as_long_as_it_says() {
    let exchange = Exchange::new(Some(login()), Some(secret()), Stub::identifying("1", "Ada"));
    let response = exchange.serve(
        Request::Trade {
            code: "code".to_string(),
        },
        moment(),
    );
    let answer = body(&response);
    let ticket = answer["ticket"].as_str().expect("a ticket").to_string();
    let expires_at = answer["expiresAt"].as_u64().expect("an expiry");

    assert!(
        integrity::check_ticket(&secret(), &ticket, expires_at - 1).is_some(),
        "good a second before it expires"
    );
    assert!(
        integrity::check_ticket(&secret(), &ticket, expires_at).is_none(),
        "and not at the instant it does"
    );
}

/// The code and the redirect URI reach Discord exactly as they were given.
///
/// The redirect URI matters: Discord compares it against the one the authorize URL
/// carried, and a mismatch is the failure a deployment actually hits.
#[test]
fn discord_is_asked_with_the_code_and_the_registered_redirect() {
    let discord = Stub::identifying("1", "Ada");
    let asked = discord.asked.clone();
    let exchange = Exchange::new(Some(login()), Some(secret()), discord);
    exchange.serve(
        Request::Trade {
            code: "  spaces and all  ".to_string(),
        },
        moment(),
    );
    let asked = asked.borrow();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].1, "  spaces and all  ",
        "the code is not trimmed: Discord compares it, so changing it is changing it"
    );
    assert_eq!(asked[0].0, login());
}

/// `A missing Discord config disables login rather than failing`.
///
/// The case a deployment reaches first, because `deployment.md` requires that the stack
/// deploy before the secrets are set. *Not configured* is a successful answer, so the page
/// can tell login being off from the exchange being broken.
#[test]
fn with_nothing_configured_login_is_off_and_nothing_is_broken() {
    for (what, login, secret) in [
        ("neither", None, None),
        ("no discord config", None, Some(secret())),
        ("no ticket secret", Some(login()), None),
    ] {
        let exchange = Exchange::new(login, secret, Never);

        let asked = exchange.serve(Request::Where, moment());
        assert_eq!(asked.status, 200, "{what}: asking is not an error");
        assert_eq!(body(&asked)["configured"], false, "{what}");
        assert_eq!(
            body(&asked).get("clientId"),
            None,
            "{what}: nothing to send anybody to"
        );

        // And a code cannot be traded, which is the service's problem rather than the
        // caller's — so 503 and not 400.
        let traded = exchange.serve(
            Request::Trade {
                code: "code".to_string(),
            },
            moment(),
        );
        assert_eq!(traded.status, 503, "{what}");
    }
}

/// Configured, the answer says where to go and asks for the least it can.
#[test]
fn configured_the_answer_is_where_to_go_and_the_smallest_scope() {
    let exchange = Exchange::new(Some(login()), Some(secret()), Never);
    let answer = body(&exchange.serve(Request::Where, moment()));
    assert_eq!(answer["configured"], true);
    assert_eq!(answer["clientId"], "1234567890");
    assert_eq!(
        answer["redirectUri"],
        "https://seanshubin.com/liars-dice/multiplayer/"
    );
    assert_eq!(
        answer["scope"], "identify",
        "the account id and the display name are the whole of what a seat needs"
    );
    assert_eq!(
        answer.get("clientSecret"),
        None,
        "the secret is the one thing this service exists to keep"
    );
}

/// A code Discord refuses is the caller's to retry; Discord being unreachable is not.
///
/// The distinction is worth a status code each: a page told 400 clears its state and asks
/// the person to start again, which is right for a spent code and useless for an outage.
#[test]
fn a_refused_code_and_an_unreachable_discord_are_different_answers() {
    let refused = Exchange::new(
        Some(login()),
        Some(secret()),
        Stub::saying(Err(Trouble::Refused("invalid_grant".to_string()))),
    );
    let response = refused.serve(
        Request::Trade {
            code: "spent".to_string(),
        },
        moment(),
    );
    assert_eq!(response.status, 400);
    assert_eq!(body(&response)["error"], "refused");
    assert_eq!(
        body(&response)["detail"],
        "invalid_grant",
        "Discord's own wording, which is what tells a deployment its redirect does not match"
    );
    assert_eq!(body(&response).get("ticket"), None, "and no ticket");

    let down = Exchange::new(
        Some(login()),
        Some(secret()),
        Stub::saying(Err(Trouble::Unreachable("timed out".to_string()))),
    );
    let response = down.serve(
        Request::Trade {
            code: "fine".to_string(),
        },
        moment(),
    );
    assert_eq!(response.status, 502);
    assert_eq!(body(&response)["error"], "unreachable");
}

/// An empty code is refused without asking Discord.
#[test]
fn an_empty_code_is_not_sent_to_discord() {
    let exchange = Exchange::new(Some(login()), Some(secret()), Never);
    let response = exchange.serve(
        Request::Trade {
            code: String::new(),
        },
        moment(),
    );
    assert_eq!(response.status, 400);
    assert_eq!(body(&response)["error"], "no code");
}

/// Anything that is not a GET or a POST gets a method error and no body worth reading.
#[test]
fn an_unknown_request_is_answered_and_not_served() {
    let exchange = Exchange::new(Some(login()), Some(secret()), Never);
    let response = exchange.serve(Request::Unknown, moment());
    assert_eq!(response.status, 405);
}
