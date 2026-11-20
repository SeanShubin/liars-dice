//! [`Connections`] on DynamoDB: which connection holds which place at a match.
//!
//! Realizes the place table in /implementation/relay-service.md. The one piece of state
//! that is neither in the log nor derived from it, because a socket belongs to the
//! transport and outlives no invocation.
//!
//! **A row is a place, not a socket.** The seat is in the key, so a seat held by two
//! sockets has nowhere to be written — the invariant is a property of the table rather
//! than of a procedure that has to survive being interleaved with itself. It used to be
//! the procedure, and two invocations claiming one seat both read, both found nobody to
//! evict, and both wrote.

use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::types::AttributeValue;
use protocol::Seat;
use relay::{Connected, ConnectionId, Connections, MatchId, Role};
use tokio::runtime::Handle;

/// The partition key: which match the place is at.
pub const MATCH: &str = "match";
/// The sort key: which place at the match this row is.
///
/// `seat#0` today and `watcher#<connection>` once spectators exist, so the prefix says
/// what a row is. That is why no role is stored: a column beside the key could disagree
/// with it, and then two things would claim to say the same thing.
pub const PLACE: &str = "place";
/// Who holds the place. Not part of any key on the table, which is the whole point.
pub const CONNECTION: &str = "connection";
/// Reverses the key, for the one caller that knows only the connection: `$disconnect`.
pub const BY_CONNECTION: &str = "by-connection";

/// The place a seat is stored at.
pub fn place_of(seat: Seat) -> String {
    format!("seat#{}", seat.0)
}

/// The seat a place names, or `None` if it names something that is not a seat.
///
/// A place this build cannot read is a place written by a newer one — a watcher, when
/// they arrive. Dropping the row is right where panicking is not, and it is also why
/// `of` can never hand the relay a spectator to deal to.
fn seat_of_place(place: &str) -> Option<Seat> {
    place.strip_prefix("seat#")?.parse().ok().map(Seat)
}

/// The place table.
///
/// Named for the connection it tracks rather than for the row, because the trait it
/// implements is about attaching and detaching connections. What the storage is keyed by
/// is this module's business and no caller's.
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

    /// Which places a connection holds, through the reverse index.
    ///
    /// A list rather than an option because the index can only promise what it holds: one
    /// place per connection is what [`Connections::attach`] maintains, not something the
    /// query may assume.
    fn whereabouts(&self, connection: &ConnectionId) -> Vec<(String, String)> {
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
            .filter_map(|item| {
                let match_id = item.get(MATCH)?.as_s().ok()?;
                let place = item.get(PLACE)?.as_s().ok()?;
                Some((match_id.clone(), place.clone()))
            })
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
            .unwrap_or_else(|error| panic!("reading places at {}: {error}", match_id.0));
        answer
            .items()
            .iter()
            .filter_map(|item| {
                let connection = item.get(CONNECTION)?.as_s().ok()?;
                let seat = seat_of_place(item.get(PLACE)?.as_s().ok()?)?;
                Some(Connected {
                    connection: ConnectionId(connection.clone()),
                    seat,
                    role: Role::Player,
                })
            })
            .collect()
    }

    /// Puts this connection on this seat, replacing whoever held it.
    ///
    /// No read. The put lands on the place's own key, so a second socket claiming the seat
    /// overwrites the first instead of joining it, however the two invocations interleave.
    /// The detach first is the other half of the contract — a socket is in one place —
    /// and that one is a read, because a connection's whereabouts are not a key here.
    fn attach(&mut self, match_id: &MatchId, connection: ConnectionId, seat: Seat, role: Role) {
        self.detach(&connection);
        // Exhaustive, so a role that is not a player has to decide what it is keyed by
        // rather than silently being filed as a seat.
        let place = match role {
            Role::Player => place_of(seat),
        };
        let put = self
            .client
            .put_item()
            .table_name(&self.table)
            .item(MATCH, AttributeValue::S(match_id.0.clone()))
            .item(PLACE, AttributeValue::S(place))
            .item(CONNECTION, AttributeValue::S(connection.0.clone()));
        self.handle.block_on(put.send()).unwrap_or_else(|error| {
            panic!(
                "attaching {} to {} seat {seat}: {error}",
                connection.0, match_id.0
            )
        });
    }

    /// `$disconnect` knows the connection and nothing else, which is why the index exists.
    ///
    /// Conditional on the row still naming this connection, because a place changes hands.
    /// Deleting by key alone would let a socket moving between places delete the row
    /// somebody else had just taken, and would let a late `$disconnect` evict the
    /// reconnection that had already replaced it. Both of those should do nothing, and a
    /// failed condition is exactly nothing.
    fn detach(&mut self, connection: &ConnectionId) {
        for (match_id, place) in self.whereabouts(connection) {
            let delete = self
                .client
                .delete_item()
                .table_name(&self.table)
                .key(MATCH, AttributeValue::S(match_id.clone()))
                .key(PLACE, AttributeValue::S(place.clone()))
                .condition_expression("#connection = :mine")
                .expression_attribute_names("#connection", CONNECTION)
                .expression_attribute_values(":mine", AttributeValue::S(connection.0.clone()));
            if let Err(error) = self.handle.block_on(delete.send()) {
                let refused = error.into_service_error();
                if !refused.is_conditional_check_failed_exception() {
                    panic!(
                        "detaching {} from {place} at {match_id}: {refused}",
                        connection.0
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A place reads back as the seat it was written for, and nothing else does.
    #[test]
    fn a_seat_survives_being_written_as_a_place() {
        for number in [0u8, 1, 9, 255] {
            assert_eq!(seat_of_place(&place_of(Seat(number))), Some(Seat(number)));
        }
    }

    /// A place this build cannot read is dropped rather than guessed at, which is what
    /// keeps a spectator from being dealt to by a relay that predates them.
    #[test]
    fn a_place_that_is_not_a_seat_is_not_a_seat() {
        for place in [
            "watcher#abc",
            "seat#",
            "seat#x",
            "seat#-1",
            "",
            "0",
            "seat#999",
        ] {
            assert_eq!(seat_of_place(place), None, "{place} read as a seat");
        }
    }
}
