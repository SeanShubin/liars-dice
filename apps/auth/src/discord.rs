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
        })
    }
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
