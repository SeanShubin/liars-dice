//! The two calls to Discord, behind [`Discord`].
//!
//! The only part of this repository that talks to anything outside AWS. Blocking, inside
//! `spawn_blocking`, which is the same seam `apps/relay` uses for the same reason: the
//! trait is synchronous and worth keeping so, because an async trait would spread `async`
//! through the handler and every test of it.

use std::time::Duration;

use auth::{Discord, Identified, Login, Trouble};

/// Discord's OAuth token endpoint.
const TOKEN: &str = "https://discord.com/api/oauth2/token";

/// Discord's own account endpoint. An `identify` token reads exactly one account: its own.
const ME: &str = "https://discord.com/api/v10/users/@me";

/// Long enough for a round trip, short enough that the page is not left hanging. The
/// Lambda's own timeout is above this, so a slow Discord fails here with something to say
/// rather than being killed with nothing.
const PATIENCE: Duration = Duration::from_secs(8);

/// The real Discord.
pub struct Live {
    client: reqwest::blocking::Client,
}

impl Live {
    pub fn new() -> Result<Self, String> {
        // A generic client name and no contact details. Discord asks automated clients to
        // identify themselves; it does not get told whose deployment this is.
        let client = reqwest::blocking::Client::builder()
            .timeout(PATIENCE)
            .user_agent("liars-dice (+https://seanshubin.com/liars-dice)")
            .build()
            .map_err(|error| format!("building an http client: {error}"))?;
        Ok(Self { client })
    }
}

impl Discord for Live {
    fn identify(&self, login: &Login, code: &str) -> Result<Identified, Trouble> {
        let token = self.token(login, code)?;
        self.me(&token)
    }
}

impl Live {
    /// Trade the code for an access token.
    ///
    /// The token is used once, here, and never leaves this function. Handing one to the
    /// browser would be handing out a credential this system has no reason to hold and the
    /// browser no reason to have.
    fn token(&self, login: &Login, code: &str) -> Result<String, Trouble> {
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code),
            // Sent again and compared by Discord against the one the authorize URL
            // carried, which is why all three places have to agree on it exactly.
            ("redirect_uri", login.redirect_uri.as_str()),
        ];
        let answer = self
            .client
            .post(TOKEN)
            // In the header rather than the form. Both are allowed; this keeps the secret
            // out of a body that anything logging a request would write down.
            .basic_auth(&login.client_id, Some(&login.client_secret))
            .form(&form)
            .send()
            .map_err(|error| Trouble::Unreachable(format!("asking for a token: {error}")))?;
        let status = answer.status();
        let body: serde_json::Value = answer
            .json()
            .map_err(|error| Trouble::Unreachable(format!("reading a token: {error}")))?;
        if !status.is_success() {
            // Discord's own wording, which is specific enough to act on — an expired code
            // reads differently from a redirect URI that does not match the registration,
            // and the second is the one a deployment gets wrong.
            let why = body
                .get("error_description")
                .or_else(|| body.get("error"))
                .and_then(|value| value.as_str())
                .unwrap_or("no reason given")
                .to_string();
            return Err(Trouble::Refused(why));
        }
        body.get("access_token")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| Trouble::Unreachable("a token answer with no token in it".to_string()))
    }

    /// Read the one account the token is for.
    fn me(&self, token: &str) -> Result<Identified, Trouble> {
        let answer = self
            .client
            .get(ME)
            .bearer_auth(token)
            .send()
            .map_err(|error| Trouble::Unreachable(format!("asking who they are: {error}")))?;
        if !answer.status().is_success() {
            return Err(Trouble::Unreachable(format!(
                "reading an account: {}",
                answer.status()
            )));
        }
        let body: serde_json::Value = answer
            .json()
            .map_err(|error| Trouble::Unreachable(format!("reading an account: {error}")))?;
        let id = body
            .get("id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| Trouble::Unreachable("an account with no id".to_string()))?;
        Ok(Identified {
            id: id.to_string(),
            name: display_name(&body, id),
            username: username(&body, id),
        })
    }
}

/// The username, which Discord keeps unique.
///
/// The same JSON this already reads the display name out of, so keeping it costs no
/// second call. Unlike `display_name` this does not fall back to `global_name`: a
/// display name is not unique, and a username that was really a display name would be
/// a worse answer than none. The id is still the last resort, so a reader always has
/// something that differs between two accounts.
fn username(body: &serde_json::Value, id: &str) -> String {
    body.get("username")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|username| !username.is_empty())
        .unwrap_or(id)
        .to_string()
}

/// The display name, which is not one field.
///
/// Discord has moved from `username#discriminator` to a global display name, so `username`
/// is now the handle and `global_name` is what a person is called. Preferring the second
/// is what `An authenticated player's name is their Discord display name` asks for, and
/// falling back to the handle is for accounts that never set one. The id is the last
/// resort, so a seat is never nameless.
fn display_name(body: &serde_json::Value, id: &str) -> String {
    for field in ["global_name", "username"] {
        if let Some(name) = body.get(field).and_then(|value| value.as_str())
            && !name.trim().is_empty()
        {
            return name.to_string();
        }
    }
    id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The TLS stack works, which no other test here can tell us.
    ///
    /// Both `ring` and `aws-lc-rs` reach this binary — the AWS SDK brings one and reqwest
    /// brings the other — and when two providers are enabled rustls can refuse to pick a
    /// default and panic at the first handshake. `apps/probe` installs one explicitly for
    /// exactly that reason.
    ///
    /// That would be a runtime failure in the one code path nothing else exercises. The
    /// `GET` the page makes at startup needs no TLS, so a deployment could answer
    /// `configured: true` and still be unable to reach Discord at all — green gate, green
    /// deploy, broken on the one action only a person performs.
    ///
    /// So this makes a real request, with credentials that are obviously not credentials.
    /// Being *refused* is the answer we want: a refusal means the handshake happened.
    #[test]
    #[ignore = "reaches discord.com; the gate runs it with --include-ignored"]
    fn a_handshake_with_discord_succeeds_even_though_the_code_does_not() {
        let live = Live::new().expect("a client");
        let nobody = Login {
            client_id: "0".to_string(),
            client_secret: "not a secret".to_string(),
            redirect_uri: "https://example.invalid/".to_string(),
        };
        match live.identify(&nobody, "not a code") {
            Err(Trouble::Refused(_)) => {}
            // Two causes, and this cannot tell them apart, so it names both rather than
            // asserting the one it was written for. A red gate during a Discord outage
            // should not read as a claim about this build.
            Err(Trouble::Unreachable(why)) => panic!(
                "Discord was not reached, so the code was never refused. Either this                  build cannot do TLS — two rustls providers and no default chosen, which                  is what this test exists for — or Discord is unreachable right now: {why}"
            ),
            Ok(identified) => panic!("discord identified nobody as {identified:?}"),
        }
    }

    /// The display name is the global one where there is one, and the handle where there
    /// is not.
    #[test]
    fn a_name_is_the_global_one_then_the_handle_then_the_id() {
        let cases = [
            (
                json!({"global_name": "Ada Lovelace", "username": "ada"}),
                "Ada Lovelace",
            ),
            (json!({"global_name": null, "username": "ada"}), "ada"),
            (json!({"global_name": "", "username": "ada"}), "ada"),
            (json!({"global_name": "   ", "username": "ada"}), "ada"),
            (json!({"username": "ada"}), "ada"),
            (json!({}), "450853396908343307"),
        ];
        for (body, expected) in cases {
            assert_eq!(
                display_name(&body, "450853396908343307"),
                expected,
                "{body}"
            );
        }
    }
}
