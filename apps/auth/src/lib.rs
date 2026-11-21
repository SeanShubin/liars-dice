//! The exchange: trades a Discord authorization code for a ticket.
//!
//! Realizes *Proving a Discord account: a ticket, not a token* in
//! /implementation/identity.md. The only thing in the system that holds the Discord client
//! secret, and the only thing that talks to Discord.
//!
//! Two requests and no database:
//!
//! | Method | Answers |
//! |--------|---------|
//! | `GET`  | Where to send somebody to authorize, or that login is not configured. |
//! | `POST` | A code, traded for a ticket. |
//!
//! The relay never calls Discord and never calls this. An outbound call on the hot path
//! would make a Discord outage into a relay outage and add a round trip to every message
//! that names an account, and there is nothing it could learn that the ticket does not
//! already carry: the relay verifies what the browser brings, with the secret alone.
//!
//! [`Exchange::serve`] is pure given a [`Discord`] and a clock, which is what lets the
//! whole of it be tested without a network — including the answer it gives when nothing is
//! configured, which is the case a deployment reaches first.

use integrity::{Secret, TICKET_SECONDS};
use protocol::{Account, Moment, account_text};
use serde_json::{Value, json};

/// What the exchange needs in order to send anybody to Discord.
///
/// All of it or none of it: a client id without a secret cannot complete a flow, and a
/// page that looks like it can authenticate and cannot is the one thing that must never
/// happen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Login {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

/// Who Discord says somebody is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Identified {
    /// The snowflake. What `An authenticated player is identified by their Discord account
    /// id` means.
    pub id: String,
    /// The display name. What `An authenticated player's name is their Discord display
    /// name` means.
    pub name: String,
}

/// Why a code could not be traded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Trouble {
    /// Discord refused the code: it was wrong, used already, or minted for another
    /// application. The ordinary failure, and the one a person causes by reloading a page
    /// with a stale `?code=` in it.
    Refused(String),
    /// Discord could not be reached, or answered something unreadable.
    Unreachable(String),
}

/// The one call this service makes to the outside world.
///
/// A trait because the exchange decides when to call it and with what, which is the test
/// for a seam rather than a passed-in value — see *A value where you can, a trait where
/// you must* in /implementation/composition.md. It is also what lets every answer this
/// service can give be tested without a network.
pub trait Discord {
    /// Trade a code for an account id and a display name.
    ///
    /// Two calls to Discord behind one method, because they are one question: the token is
    /// never used for anything else and never leaves the implementation. An access token
    /// that reached the browser would be a credential this system had no reason to hand
    /// out.
    fn identify(&self, login: &Login, code: &str) -> Result<Identified, Trouble>;
}

/// What arrived.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Request {
    /// `GET`: where do I send somebody to authorize?
    Where,
    /// `POST`: here is a code.
    Trade { code: String },
    /// Anything else.
    Unknown,
}

/// What to answer with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    fn of(status: u16, body: Value) -> Self {
        Self {
            status,
            body: body.to_string(),
        }
    }
}

/// Everything a cold start can prepare.
///
/// `login` is an `Option` and that is the whole of `A missing Discord config disables login
/// rather than failing`: `deployment.md` already requires that the stack deploy before the
/// secrets are set, so a service that refused to start without them would be a service that
/// cannot be deployed. It answers *not configured*, the page offers guest only and says
/// why, and every unranked thing still works.
///
/// The ticket secret is optional for the same reason, and not for the reason the relay's
/// dice secret is mandatory. An invented dice secret would make every commitment in every
/// match unverifiable with nothing saying so; a missing ticket secret just means nobody can
/// log in, which is exactly what *not configured* already means.
pub struct Exchange<D: Discord> {
    login: Option<Login>,
    ticket_secret: Option<Secret>,
    discord: D,
}

impl<D: Discord> Exchange<D> {
    pub fn new(login: Option<Login>, ticket_secret: Option<Secret>, discord: D) -> Self {
        Self {
            login,
            ticket_secret,
            discord,
        }
    }

    /// Whether anybody can log in at all.
    fn ready(&self) -> Option<(&Login, &Secret)> {
        Some((self.login.as_ref()?, self.ticket_secret.as_ref()?))
    }

    /// Serve one request.
    ///
    /// `now` is read by the composition root and passed in, like every other clock reading
    /// in this repository: `One clock reading serves one message`. It decides one thing —
    /// when the minted ticket expires — and it reaches the ticket and never a log.
    pub fn serve(&self, request: Request, now: Moment) -> Response {
        match request {
            Request::Where => self.where_to_go(),
            Request::Trade { code } => self.trade(&code, now),
            // Not an error worth a body. A function URL is reachable by anything that
            // wants to poke at it, and this is a service with two verbs.
            Request::Unknown => Response::of(405, json!({ "error": "unsupported" })),
        }
    }

    /// Where to send somebody to authorize, or that login is not configured.
    ///
    /// The page asks this before it draws a sign-in button, which is why *not configured*
    /// is a successful answer and not an error: the page has to be able to tell *login is
    /// off* from *the exchange is broken*, and offer guest play in the first case.
    ///
    /// The client id and the redirect URI come from here rather than from the bundle, so
    /// the configuration lives in one place — SSM, read by the one thing that already reads
    /// SSM. The pipeline needs no second placeholder to rewrite and the deploy role needs
    /// no new permission.
    fn where_to_go(&self) -> Response {
        let Some((login, _)) = self.ready() else {
            return Response::of(200, json!({ "configured": false }));
        };
        Response::of(
            200,
            json!({
                "configured": true,
                "clientId": login.client_id,
                "redirectUri": login.redirect_uri,
                // `identify` and nothing else. The account id and the display name are the
                // whole of what a seat needs, and a scope that asked for more would be
                // asking for data this system has no use for and no business holding.
                "scope": "identify",
            }),
        )
    }

    /// Trade a code for a ticket.
    fn trade(&self, code: &str, now: Moment) -> Response {
        let Some((login, secret)) = self.ready() else {
            return Response::of(503, json!({ "error": "not configured" }));
        };
        if code.is_empty() {
            return Response::of(400, json!({ "error": "no code" }));
        }
        let identified = match self.discord.identify(login, code) {
            Ok(identified) => identified,
            // The code was wrong or already spent, which is the browser's problem to
            // report and a person's to retry. 400 rather than 401, because nothing here
            // was authenticating: a code is a claim being traded.
            Err(Trouble::Refused(why)) => {
                return Response::of(400, json!({ "error": "refused", "detail": why }));
            }
            // Discord's problem, so this is not the caller's fault and saying so matters:
            // a page told 400 would clear its state and ask the person to start again,
            // which would not help.
            Err(Trouble::Unreachable(why)) => {
                return Response::of(502, json!({ "error": "unreachable", "detail": why }));
            }
        };

        let account = account_text(&Account::Discord(identified.id));
        let expires_at = now.to_unix_nanos() / 1_000_000_000 + TICKET_SECONDS;
        let ticket = integrity::ticket(secret, &account, &identified.name, expires_at);
        Response::of(
            200,
            json!({
                "ticket": ticket,
                // Both repeated outside the ticket, because the ticket is opaque to the
                // page and the page has a name to draw and an expiry to re-authenticate
                // ahead of. Not a second source of truth: the relay reads neither of
                // these, it reads the ticket.
                "account": account,
                "name": identified.name,
                "expiresAt": expires_at,
            }),
        )
    }
}
