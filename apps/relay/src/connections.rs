//! [`Connections`] on DynamoDB: which socket is which seat.
//!
//! Realizes the connection table in /implementation/relay-service.md. The one piece of
//! state that is neither in the log nor derived from it, because a socket belongs to the
//! transport and outlives no invocation.

use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::types::AttributeValue;
use protocol::Seat;
use relay::{Connected, ConnectionId, Connections, MatchId, Role};
use tokio::runtime::Handle;

/// The partition key: which match the socket is attached to.
pub const MATCH: &str = "match";
/// The sort key: the connection id, as the gateway assigned it.
pub const CONNECTION: &str = "connection";
/// Which seat the socket is playing.
pub const SEAT: &str = "seat";
/// Why it is here. Stored rather than derived, so admitting spectators changes no table.
pub const ROLE: &str = "role";
/// Reverses the key, for the one caller that knows only the connection: `$disconnect`.
pub const BY_CONNECTION: &str = "by-connection";

/// The connection table.
///
/// Named for where a connection id comes from — the API Gateway — rather than for where
/// the row is kept.
pub struct GatewayConnections {
    client: Client,
    table: String,
    handle: Handle,
}

impl GatewayConnections {
    pub fn new(client: Client, table: impl Into<String>, handle: Handle) -> Self {
        Self {
            client,
            table: table.into(),
            handle,
        }
    }

    /// Which matches a connection is attached to, through the reverse index.
    ///
    /// A list rather than an option because the index can only promise what it holds:
    /// one row is the invariant [`Connections::attach`] maintains, not something the
    /// query can assume.
    fn whereabouts(&self, connection: &ConnectionId) -> Vec<MatchId> {
        let query = self
            .client
            .query()
            .table_name(&self.table)
            .index_name(BY_CONNECTION)
            .key_condition_expression("#connection = :connection")
            .expression_attribute_names("#connection", CONNECTION)
            .expression_attribute_values(":connection", AttributeValue::S(connection.0.clone()));
        let answer = self
            .handle
            .block_on(query.send())
            .unwrap_or_else(|error| panic!("finding connection {}: {error}", connection.0));
        answer
            .items()
            .iter()
            .filter_map(|item| item.get(MATCH).and_then(|value| value.as_s().ok()))
            .map(|id| MatchId(id.clone()))
            .collect()
    }
}

impl Connections for GatewayConnections {
    fn of(&self, match_id: &MatchId) -> Vec<Connected> {
        let query = self
            .client
            .query()
            .table_name(&self.table)
            .key_condition_expression("#match = :match")
            .expression_attribute_names("#match", MATCH)
            .expression_attribute_values(":match", AttributeValue::S(match_id.0.clone()));
        let answer = self
            .handle
            .block_on(query.send())
            .unwrap_or_else(|error| panic!("reading connections of {}: {error}", match_id.0));
        answer
            .items()
            .iter()
            .filter_map(|item| {
                let connection = item.get(CONNECTION)?.as_s().ok()?;
                let seat = item.get(SEAT)?.as_n().ok()?.parse().ok()?;
                let role = match item.get(ROLE)?.as_s().ok()?.as_str() {
                    "player" => Role::Player,
                    // A role this build does not know means a newer writer put it there.
                    // Dropping the row is right where panicking is not: a spectator it
                    // cannot represent is a socket it must not send a seat's dice to.
                    _ => return None,
                };
                Some(Connected {
                    connection: ConnectionId(connection.clone()),
                    seat: Seat(seat),
                    role,
                })
            })
            .collect()
    }

    /// Puts this connection on this seat and clears both ways out of it.
    ///
    /// The socket leaves wherever else it was, and whoever else held this seat leaves it.
    /// The second eviction is the one that matters: the relay sends a seat's deal to
    /// every connection attached to that seat, so two rows on one seat is one player
    /// reading another's dice.
    fn attach(&mut self, match_id: &MatchId, connection: ConnectionId, seat: Seat, role: Role) {
        self.detach(&connection);
        for held in self.of(match_id) {
            if held.seat == seat && held.connection != connection {
                self.detach(&held.connection);
            }
        }
        let role = match role {
            Role::Player => "player",
        };
        let put = self
            .client
            .put_item()
            .table_name(&self.table)
            .item(MATCH, AttributeValue::S(match_id.0.clone()))
            .item(CONNECTION, AttributeValue::S(connection.0.clone()))
            .item(SEAT, AttributeValue::N(seat.0.to_string()))
            .item(ROLE, AttributeValue::S(role.to_string()));
        self.handle.block_on(put.send()).unwrap_or_else(|error| {
            panic!(
                "attaching {} to {} seat {seat}: {error}",
                connection.0, match_id.0
            )
        });
    }

    /// `$disconnect` knows the connection and nothing else, which is why the index exists.
    fn detach(&mut self, connection: &ConnectionId) {
        for match_id in self.whereabouts(connection) {
            let delete = self
                .client
                .delete_item()
                .table_name(&self.table)
                .key(MATCH, AttributeValue::S(match_id.0.clone()))
                .key(CONNECTION, AttributeValue::S(connection.0.clone()));
            self.handle.block_on(delete.send()).unwrap_or_else(|error| {
                panic!("detaching {} from {}: {error}", connection.0, match_id.0)
            });
        }
    }
}
