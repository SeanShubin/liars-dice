//! The three handlers. A Lambda invocation is the whole lifetime of a relay.
//!
//! Realizes the routes in /implementation/relay-service.md. Everything that could be a
//! judgement call — whether a join is legal, which seats see which deal, what the log
//! says — happened inside `Relay::receive` and was decided by code that already has
//! tests. What is here is the wiring, and it is deliberately dull.

use std::time::Instant;

use aws_sdk_apigatewaymanagement::primitives::Blob;
use integrity::Secret;
use protocol::Moment;
use relay::{ConnectionId, Connections, MatchId, Outcome, Relay, Secrets};
use wire::answer::Answer;
use wire::{json, message};

use crate::incoming::{Incoming, Route};
use crate::{DynamoLog, GatewayConnections, Meter, NAMESPACE};

/// Everything a cold start can prepare. Built once, used by every invocation.
///
/// The secret is read here and cached for the life of the execution environment, which is
/// right because it is the same for every invocation serving any match, and because a
/// parameter read per message would be a charge per message for a value that cannot change.
pub struct Service {
    pub dynamodb: aws_sdk_dynamodb::Client,
    pub config: aws_config::SdkConfig,
    pub log_table: String,
    pub place_table: String,
    secret: Secret,
    /// The key tickets are verified with, if there is one.
    ///
    /// Optional where the dice secret is mandatory, and the difference is the point. An
    /// invented dice secret would make every commitment in every match unverifiable with
    /// nothing saying so; a missing ticket secret means nobody can sign in, which is a
    /// state a deployment is allowed to be in — `deployment.md` requires that the stack
    /// deploy before the secrets are set, and guests are unaffected.
    ticket_secret: Option<Secret>,
}

/// The service could not be built, so no invocation can be served.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NotConfigured {
    MissingVariable(&'static str),
    UnreadableParameter {
        name: String,
        why: String,
    },
    /// The parameter exists and does not hold 64 hexadecimal digits.
    ///
    /// Fatal rather than a fallback. A secret that was invented because the real one
    /// could not be read would make every commitment in every match unverifiable, and
    /// nothing anywhere would say so.
    NotASecret {
        name: String,
    },
}

impl std::fmt::Display for NotConfigured {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotConfigured::MissingVariable(name) => write!(out, "{name} is not set"),
            NotConfigured::UnreadableParameter { name, why } => {
                write!(out, "cannot read the parameter {name}: {why}")
            }
            NotConfigured::NotASecret { name } => write!(
                out,
                "the parameter {name} does not hold 64 hexadecimal digits, and a secret \
                 will not be invented in its place"
            ),
        }
    }
}

impl std::error::Error for NotConfigured {}

/// The system clock, read as a [`Moment`].
///
/// The only observation of time anywhere in the deployed relay, and it lives in the
/// composition root on purpose: `crates/relay` holds no clock, so `The relay holds no game
/// state between messages except what the log holds` stays literally true, and a test
/// decides what time it is by passing a number.
///
/// Before the epoch is not a time this program can be at, so a clock that claims it reads
/// as the epoch. Nothing is derived from it that reaches a log — see `The time the relay
/// received an event decides nothing but whether a match is inactive`.
fn now() -> Moment {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or_default();
    Moment::from_unix_nanos(nanos)
}

/// Where the log is, where the places are, and where the secret is.
pub const LOG_TABLE: &str = "LOG_TABLE";
pub const PLACE_TABLE: &str = "PLACE_TABLE";
pub const SECRET_PARAMETER: &str = "SECRET_PARAMETER";
pub const TICKET_SECRET_PARAMETER: &str = "TICKET_SECRET_PARAMETER";

impl Service {
    pub async fn new(config: aws_config::SdkConfig) -> Result<Self, NotConfigured> {
        let variable = |name: &'static str| {
            std::env::var(name).map_err(|_| NotConfigured::MissingVariable(name))
        };
        let parameter = variable(SECRET_PARAMETER)?;
        let secret = read_secret(&config, &parameter).await?;
        // Read if it is there and not missed if it is not, which is why this is not
        // `read_secret` with a `?`. A failure here disables signing in; it must not take
        // the service down with it.
        let ticket_secret = match variable(TICKET_SECRET_PARAMETER) {
            Ok(name) => match read_secret(&config, &name).await {
                Ok(secret) => Some(secret),
                Err(why) => {
                    eprintln!("signing in is off: {why}");
                    None
                }
            },
            Err(why) => {
                eprintln!("signing in is off: {why}");
                None
            }
        };
        Ok(Self {
            dynamodb: aws_sdk_dynamodb::Client::new(&config),
            log_table: variable(LOG_TABLE)?,
            place_table: variable(PLACE_TABLE)?,
            config,
            secret,
            ticket_secret,
        })
    }

    fn log(&self, meter: Meter) -> DynamoLog {
        DynamoLog::new(
            self.dynamodb.clone(),
            &self.log_table,
            tokio::runtime::Handle::current(),
            meter,
        )
    }

    fn connections(&self) -> GatewayConnections {
        GatewayConnections::new(
            self.dynamodb.clone(),
            &self.place_table,
            tokio::runtime::Handle::current(),
        )
    }

    /// Serve one invocation. Returns the status code the gateway wants.
    ///
    /// The clock is read once, here, and nothing downstream reads another. That reading
    /// serves the message — where it decides whether a match is inactive, and stamps
    /// every row the message writes — and the metric line's own timestamp. One
    /// observation of the outside world per invocation, made by the composition root,
    /// is what lets every layer below this be handed a value instead of reaching for
    /// one.
    pub async fn serve(&self, incoming: Incoming) -> Result<u16, lambda_runtime::Error> {
        let began = Instant::now();
        let now = now();
        let meter = Meter::new();
        let route = match incoming.route {
            // Nothing but accept. A connection belongs to no match until a message says
            // so, and `Connections::attach` is what says so.
            Route::Connect => "$connect",
            Route::Disconnect => {
                let mut connections = self.connections();
                let connection = incoming.connection.clone();
                tokio::task::spawn_blocking(move || connections.detach(&connection)).await?;
                "$disconnect"
            }
            Route::Default => {
                self.message(&incoming, meter.clone(), now).await?;
                "$default"
            }
        };
        println!("{}", meter.emf(NAMESPACE, route, began.elapsed(), now));
        Ok(200)
    }

    async fn message(
        &self,
        incoming: &Incoming,
        meter: Meter,
        now: Moment,
    ) -> Result<(), lambda_runtime::Error> {
        let Some(body) = &incoming.body else {
            // `$default` with no body is not a message. Nothing to record and nothing to
            // answer, and closing the socket over it would punish a keepalive.
            return Ok(());
        };
        let message = match json::read(body).and_then(|shape| message::from_shape(&shape)) {
            Ok(message) => message,
            Err(why) => {
                // The sender is told, and the socket stays open. A frame this build cannot
                // read is a client bug or a version skew, and both are worth saying out
                // loud rather than answering with silence.
                eprintln!("unreadable frame from {}: {why}", incoming.connection.0);
                return Ok(());
            }
        };

        let log = self.log(meter);
        let connections = self.connections();
        let secrets = Secrets {
            dice: self.secret,
            ticket: self.ticket_secret,
        };
        let from = incoming.connection.clone();
        // `receive` is synchronous and the storage blocks on the SDK from inside it, which
        // is legal here and nowhere else: `spawn_blocking` hands the closure to a thread
        // that is not a runtime worker. See the async seam in the document.
        let outcome = tokio::task::spawn_blocking(move || {
            let mut relay = Relay::new(log, connections, secrets);
            relay.receive(&from, message, now)
        })
        .await?;

        self.answer(incoming, outcome).await
    }

    /// Post what the relay returned, then tell the sender what became of their message.
    async fn answer(
        &self,
        incoming: &Incoming,
        outcome: Outcome,
    ) -> Result<(), lambda_runtime::Error> {
        let gateway = aws_sdk_apigatewaymanagement::Client::from_conf(
            aws_sdk_apigatewaymanagement::config::Builder::from(&self.config)
                .endpoint_url(&incoming.endpoint)
                .build(),
        );
        let mut gone = Vec::new();
        for (to, event) in &outcome.sends {
            self.post(&gateway, to, &Answer::Event(event.clone()), &mut gone)
                .await;
        }
        // Only the host is told the id, and this is how they learn it.
        if let Some(created) = &outcome.created {
            let answer = Answer::Created(MatchId(created.0.clone()));
            self.post(&gateway, &incoming.connection, &answer, &mut gone)
                .await;
        }
        // Only the asker is told what is open, because only the asker asked.
        if let Some(listed) = outcome.listed {
            let answer = Answer::Listing(listed);
            self.post(&gateway, &incoming.connection, &answer, &mut gone)
                .await;
        }
        if let Some(refused) = outcome.refused {
            let answer = Answer::Refused(refused);
            self.post(&gateway, &incoming.connection, &answer, &mut gone)
                .await;
        }
        if !gone.is_empty() {
            // Sockets that died between the query and the post. Not an error: tidy the
            // rows and carry on, which is what `$disconnect` would have done had it fired.
            let mut connections = self.connections();
            tokio::task::spawn_blocking(move || {
                for connection in &gone {
                    connections.detach(connection);
                }
            })
            .await?;
        }
        Ok(())
    }

    async fn post(
        &self,
        gateway: &aws_sdk_apigatewaymanagement::Client,
        to: &ConnectionId,
        answer: &Answer,
        gone: &mut Vec<ConnectionId>,
    ) {
        let frame = json::write(&wire::answer::shape_of(answer));
        let sent = gateway
            .post_to_connection()
            .connection_id(&to.0)
            .data(Blob::new(frame.into_bytes()))
            .send()
            .await;
        if let Err(error) = sent {
            if error
                .as_service_error()
                .is_some_and(|service| service.is_gone_exception())
            {
                gone.push(to.clone());
                return;
            }
            // A send that was lost while the log already holds the event is the one state
            // a client cannot detect by itself. It recovers by re-reading, which `Resume`
            // already does, so this is logged and not retried.
            //
            // `DisplayErrorContext` rather than the error itself: an `SdkError`'s own
            // `Display` is the words "service error" and nothing more, which is exactly
            // what the first line this ever logged in production said. The context walks
            // the source chain and names the thing that actually went wrong.
            eprintln!(
                "lost a frame to {}: {}",
                to.0,
                aws_sdk_apigatewaymanagement::error::DisplayErrorContext(&error)
            );
        }
    }
}

async fn read_secret(config: &aws_config::SdkConfig, name: &str) -> Result<Secret, NotConfigured> {
    let answer = aws_sdk_ssm::Client::new(config)
        .get_parameter()
        .name(name)
        .with_decryption(true)
        .send()
        .await
        .map_err(|error| NotConfigured::UnreadableParameter {
            name: name.to_string(),
            why: error.to_string(),
        })?;
    answer
        .parameter()
        .and_then(|parameter| parameter.value())
        .and_then(Secret::from_hex)
        .ok_or_else(|| NotConfigured::NotASecret {
            name: name.to_string(),
        })
}
