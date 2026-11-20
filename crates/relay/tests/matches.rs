//! A whole match, two relays, a disconnect and a return — in one process.
//!
//! These are the tests /implementation/relay.md asks for, and they are the reason the
//! relay is a library with two traits rather than a Lambda handler: multiplayer can be
//! proved before any infrastructure exists.

use integrity::Secret;
use protocol::{Account, Event, PROTOCOL_VERSION, Seat};
use relay::memory::{MemoryConnections, MemoryLog};
use relay::{
    Connected, ConnectionId, Connections, Log, MatchId, Message, Refused, Relay, Role, Taken,
};

fn secret() -> Secret {
    Secret::from_bytes([3u8; 32])
}

fn relay() -> Relay<MemoryLog, MemoryConnections> {
    Relay::new(MemoryLog::new(), MemoryConnections::new(), secret())
}

fn discord(id: &str) -> Account {
    Account::Discord(id.to_string())
}

fn connection(name: &str) -> ConnectionId {
    ConnectionId(name.to_string())
}

fn host<L: Log, C: Connections>(relay: &mut Relay<L, C>, who: &ConnectionId) -> MatchId {
    let outcome = relay.receive(
        who,
        Message::Host {
            account: discord("1"),
            name: "Alice".to_string(),
            colour: "red".to_string(),
            version: PROTOCOL_VERSION,
            nonce: "alice-hosts".to_string(),
        },
    );
    assert_eq!(outcome.refused, None);
    outcome.created.expect("a match was created")
}

fn join<L: Log, C: Connections>(
    relay: &mut Relay<L, C>,
    who: &ConnectionId,
    match_id: &MatchId,
    account: &str,
    name: &str,
    colour: &str,
) -> Option<Refused> {
    relay
        .receive(
            who,
            Message::Join {
                match_id: match_id.clone(),
                account: discord(account),
                name: name.to_string(),
                colour: colour.to_string(),
                version: PROTOCOL_VERSION,
            },
        )
        .refused
}

/// Host, join, start: the lobby as a match that has not begun.
fn two_player_match() -> (Relay<MemoryLog, MemoryConnections>, MatchId) {
    let mut relay = relay();
    let alice = connection("alice");
    let bob = connection("bob");
    let match_id = host(&mut relay, &alice);
    assert_eq!(join(&mut relay, &bob, &match_id, "2", "Bob", "blue"), None);
    let started = relay.receive(
        &alice,
        Message::Start {
            match_id: match_id.clone(),
        },
    );
    assert_eq!(started.refused, None);
    (relay, match_id)
}

#[test]
fn a_match_that_has_not_started_is_listed_and_one_that_has_is_not() {
    let mut relay = relay();
    let alice = connection("alice");
    let match_id = host(&mut relay, &alice);

    let listed = relay.open_matches(None);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].host, "Alice");
    assert_eq!(listed[0].players, 1);

    relay.receive(
        &alice,
        Message::Start {
            match_id: match_id.clone(),
        },
    );
    assert!(
        relay.open_matches(None).is_empty(),
        "a started match cannot be joined, so it is not listed"
    );
}

/// `Every match that can be joined is listed`, asked for over the wire rather than read
/// off the relay.
///
/// `open_matches` was right for a long time and reachable by nothing, which is a feature
/// that does not exist. This is the message that makes it one.
#[test]
fn asking_what_is_open_lists_the_matches_that_are() {
    let mut relay = relay();
    let alice = connection("alice");
    let looking = connection("looking");

    let empty = relay.receive(
        &looking,
        Message::List {
            account: discord("9"),
        },
    );
    assert_eq!(
        empty.listed,
        Some(Vec::new()),
        "nothing open is an answer, and not the absence of one"
    );

    let match_id = host(&mut relay, &alice);
    let listed = relay
        .receive(
            &looking,
            Message::List {
                account: discord("9"),
            },
        )
        .listed
        .expect("a listing");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].match_id, match_id);
    assert_eq!(listed[0].host, "Alice", "a listed match shows its host");
    assert_eq!(listed[0].players, 1, "and how many have joined");

    // Asking changes nothing: the log is as long as hosting left it.
    assert_eq!(relay.log().read(&match_id).len(), 3);

    relay.receive(&alice, Message::Start { match_id });
    let after = relay
        .receive(
            &looking,
            Message::List {
                account: discord("9"),
            },
        )
        .listed
        .expect("a listing");
    assert!(
        after.is_empty(),
        "a started match cannot be joined, so it is not listed: {after:?}"
    );
}

/// `A match is not listed to a player barred from it`, which is why the ask carries an
/// account at all.
#[test]
fn a_listing_is_answered_for_whoever_asked() {
    let mut relay = relay();
    let alice = connection("alice");
    let bob = connection("bob");
    let match_id = host(&mut relay, &alice);
    assert_eq!(join(&mut relay, &bob, &match_id, "2", "Bob", "blue"), None);
    relay.receive(
        &alice,
        Message::Bar {
            match_id: match_id.clone(),
            seat: Seat(1),
        },
    );

    let to_bob = relay
        .receive(
            &bob,
            Message::List {
                account: discord("2"),
            },
        )
        .listed
        .expect("a listing");
    assert!(to_bob.is_empty(), "a barred player is not shown the match");

    let to_anybody = relay
        .receive(
            &connection("carol"),
            Message::List {
                account: discord("3"),
            },
        )
        .listed
        .expect("a listing");
    assert_eq!(to_anybody.len(), 1, "and everybody else still sees it");
}

#[test]
fn a_barred_player_is_not_shown_the_match_and_cannot_join() {
    let mut relay = relay();
    let alice = connection("alice");
    let bob = connection("bob");
    let match_id = host(&mut relay, &alice);
    assert_eq!(join(&mut relay, &bob, &match_id, "2", "Bob", "blue"), None);

    relay.receive(
        &alice,
        Message::Bar {
            match_id: match_id.clone(),
            seat: Seat(1),
        },
    );
    assert!(relay.open_matches(Some(&discord("2"))).is_empty());
    assert_eq!(relay.open_matches(Some(&discord("3"))).len(), 1);

    relay.receive(
        &alice,
        Message::Lift {
            match_id: match_id.clone(),
            seat: Seat(1),
        },
    );
    assert_eq!(
        relay.open_matches(Some(&discord("2"))).len(),
        1,
        "lifting a bar shows the match again"
    );
}

#[test]
fn only_the_host_may_bar_or_start() {
    let mut relay = relay();
    let alice = connection("alice");
    let bob = connection("bob");
    let match_id = host(&mut relay, &alice);
    join(&mut relay, &bob, &match_id, "2", "Bob", "blue");

    assert_eq!(
        relay
            .receive(
                &bob,
                Message::Bar {
                    match_id: match_id.clone(),
                    seat: Seat(0)
                }
            )
            .refused,
        Some(Refused::NotTheHost)
    );
    assert_eq!(
        relay
            .receive(
                &bob,
                Message::Start {
                    match_id: match_id.clone()
                }
            )
            .refused,
        Some(Refused::NotTheHost)
    );
}

#[test]
fn a_join_is_refused_for_a_mismatched_version_a_taken_colour_or_a_started_match() {
    let mut relay = relay();
    let alice = connection("alice");
    let match_id = host(&mut relay, &alice);

    let mismatched = relay.receive(
        &connection("old"),
        Message::Join {
            match_id: match_id.clone(),
            account: discord("9"),
            name: "Old".to_string(),
            colour: "green".to_string(),
            version: PROTOCOL_VERSION - 1,
        },
    );
    assert_eq!(
        mismatched.refused,
        Some(Refused::VersionMismatch {
            match_version: PROTOCOL_VERSION,
            yours: PROTOCOL_VERSION - 1,
        })
    );

    assert_eq!(
        join(&mut relay, &connection("bob"), &match_id, "2", "Bob", "red"),
        Some(Refused::ColourTaken)
    );

    relay.receive(
        &alice,
        Message::Start {
            match_id: match_id.clone(),
        },
    );
    assert_eq!(
        join(
            &mut relay,
            &connection("late"),
            &match_id,
            "3",
            "Late",
            "blue"
        ),
        Some(Refused::AlreadyStarted)
    );
}

/// The secrecy property, which is the one that would be worst to get wrong: during a
/// round a client is sent its own dice and nobody else's.
#[test]
fn a_seat_is_sent_its_own_dice_and_no_others_until_the_reveal() {
    let mut relay = relay();
    let alice = connection("alice");
    let bob = connection("bob");
    let match_id = host(&mut relay, &alice);
    join(&mut relay, &bob, &match_id, "2", "Bob", "blue");

    let started = relay.receive(
        &alice,
        Message::Start {
            match_id: match_id.clone(),
        },
    );

    let dealt_to = |who: &ConnectionId| -> Vec<Seat> {
        started
            .sends
            .iter()
            .filter(|(to, event)| to == who && matches!(event, Event::Deal { .. }))
            .map(|(_, event)| match event {
                Event::Deal { seat, .. } => *seat,
                _ => unreachable!(),
            })
            .collect()
    };
    assert_eq!(dealt_to(&alice), vec![Seat(0)]);
    assert_eq!(dealt_to(&bob), vec![Seat(1)]);

    // The log holds both, because the log is the record of what the relay did. Secrecy
    // is in the fan-out, not in what is written down.
    let deals = relay
        .log()
        .read(&match_id)
        .into_iter()
        .filter(|event| matches!(event, Event::Deal { .. }))
        .count();
    assert_eq!(deals, 2);
}

/// Every seat says the round is over before the seed is revealed. One client cannot force
/// a reveal, which is what keeps a griefer from desynchronising the match.
#[test]
fn a_round_is_revealed_only_when_every_seat_has_said_it_is_over() {
    let (mut relay, match_id) = two_player_match();
    let alice = connection("alice");
    let bob = connection("bob");

    let first = relay.receive(
        &alice,
        Message::RoundOver {
            match_id: match_id.clone(),
        },
    );
    assert_eq!(first.refused, None);
    assert!(
        !first
            .sends
            .iter()
            .any(|(_, event)| matches!(event, Event::Reveal { .. })),
        "one seat must not be able to force the reveal"
    );

    let second = relay.receive(
        &bob,
        Message::RoundOver {
            match_id: match_id.clone(),
        },
    );
    assert!(
        second
            .sends
            .iter()
            .any(|(_, event)| matches!(event, Event::Reveal { .. })),
        "the last acknowledgement reveals the round"
    );
    assert_eq!(
        relay.view(&match_id).round,
        1,
        "and the next round has begun"
    );
}

/// The reveal waits for the seats that are there, not for every seat that ever joined.
///
/// Without this an absent player halts the match at the reveal rather than at their turn,
/// and `A match halts at the turn of an absent player` would be false. They catch up
/// through `Resume`, which replays everything their seat may see.
#[test]
fn an_absent_seat_does_not_hold_up_the_reveal() {
    let (mut relay, match_id) = two_player_match();
    relay.disconnect(&connection("bob"));

    let only = relay.receive(
        &connection("alice"),
        Message::RoundOver {
            match_id: match_id.clone(),
        },
    );
    assert!(
        only.sends
            .iter()
            .any(|(_, event)| matches!(event, Event::Reveal { .. })),
        "the one seat present must be enough to reveal the round"
    );
    assert_eq!(relay.view(&match_id).round, 1);
}

/// A connection table that reports one seat twice.
///
/// `attach` cannot produce this once the seat is in the key — see `relay-service.md`, the
/// place table — and it could, through a race, for as long as the eviction was a procedure
/// rather than a key. The reveal's own arithmetic should not depend on which of those is
/// true, so it is asserted against the state rather than against the storage that used to
/// allow it.
struct SeatHeldTwice(MemoryConnections);

impl Connections for SeatHeldTwice {
    fn of(&self, match_id: &MatchId) -> Vec<Connected> {
        let mut rows = self.0.of(match_id);
        if let Some(first) = rows.first().cloned() {
            rows.push(Connected {
                connection: ConnectionId(format!("{}-ghost", first.connection.0)),
                ..first
            });
        }
        rows
    }

    fn attach(&mut self, match_id: &MatchId, connection: ConnectionId, seat: Seat, role: Role) {
        self.0.attach(match_id, connection, seat, role);
    }

    fn detach(&mut self, connection: &ConnectionId) {
        self.0.detach(connection);
    }
}

/// A seat holding two sockets does not hold the round still.
///
/// The deadlock, and the reason it was one: `round_over_count` counts distinct seats and
/// can never exceed the number of them, so counting *rows* against it put the threshold
/// one above a number it could not reach. The round never revealed, and no timeout or
/// reconnection could free it — a tab that was gone held the whole match still for good.
#[test]
fn a_seat_holding_two_sockets_does_not_hold_the_round_still() {
    let mut relay = Relay::new(
        MemoryLog::new(),
        SeatHeldTwice(MemoryConnections::new()),
        secret(),
    );
    let alice = connection("alice");
    let bob = connection("bob");
    let match_id = host(&mut relay, &alice);
    assert_eq!(join(&mut relay, &bob, &match_id, "2", "Bob", "blue"), None);
    relay.receive(
        &alice,
        Message::Start {
            match_id: match_id.clone(),
        },
    );

    relay.receive(
        &alice,
        Message::RoundOver {
            match_id: match_id.clone(),
        },
    );
    let last = relay.receive(
        &bob,
        Message::RoundOver {
            match_id: match_id.clone(),
        },
    );

    assert!(
        last.sends
            .iter()
            .any(|(_, event)| matches!(event, Event::Reveal { .. })),
        "both seats acknowledged, so the round must reveal however many sockets they hold"
    );
    assert_eq!(relay.view(&match_id).round, 1);
}

/// `A match continues while an eliminated player is absent`, by way of their departure.
#[test]
fn a_departed_seat_does_not_hold_up_the_reveal() {
    let (mut relay, match_id) = two_player_match();
    relay.receive(
        &connection("bob"),
        Message::Leave {
            match_id: match_id.clone(),
        },
    );

    let only = relay.receive(
        &connection("alice"),
        Message::RoundOver {
            match_id: match_id.clone(),
        },
    );
    assert!(
        only.sends
            .iter()
            .any(|(_, event)| matches!(event, Event::Reveal { .. })),
        "a seat that has left must not be waited on"
    );
}

/// A claim is recorded while another seat is absent, because absence is not the relay's
/// to enforce.
///
/// This test used to assert the opposite: the relay refused every claim while any seat
/// lacked a connection. That halted a match the moment anybody's tab closed, and it
/// halted one forever if an eliminated player left. `A match halts at the turn of an
/// absent player` puts the halt where it belongs, and nothing has to enforce it — when it
/// is the absent seat's turn no claim arrives, because no other client will send one.
#[test]
fn a_claim_is_recorded_while_another_seat_is_absent() {
    let (mut relay, match_id) = two_player_match();
    let alice = connection("alice");
    let bob = connection("bob");

    relay.disconnect(&bob);
    let claim = Event::Claim {
        seat: Seat(0),
        quantity: 2,
        face: 5,
        odds: 50.0,
    };
    let outcome = relay.receive(
        &alice,
        Message::Claim {
            match_id: match_id.clone(),
            event: claim.clone(),
        },
    );
    assert_eq!(outcome.refused, None, "absence is not the relay's business");
    assert!(
        relay.log().read(&match_id).contains(&claim),
        "the claim must be recorded, since the relay records every message it receives"
    );
    // And nothing was sent to the seat that is not there.
    assert!(
        outcome.sends.iter().all(|(to, _)| to == &alice),
        "a frame was addressed to an absent seat: {:?}",
        outcome.sends
    );
}

/// `A player holds one seat in a match`.
///
/// The defect this was written for came out of the first match two people played in a
/// browser: one account appears on seat 1 and on seat 2, because somebody reloaded and
/// joined again. The second seat is worse than untidy — `attach` moves a socket to the
/// newest seat, so the first is left with no connection and halts the match at its turn
/// for good, and it is in the log where no reconnection can clear it.
#[test]
fn joining_twice_returns_the_seat_already_held() {
    let mut relay = relay();
    let alice = connection("alice");
    let bob = connection("bob");
    let match_id = host(&mut relay, &alice);

    assert_eq!(join(&mut relay, &bob, &match_id, "2", "Bob", "blue"), None);
    fn seats<L: Log, C: Connections>(relay: &Relay<L, C>, match_id: &MatchId) -> usize {
        relay
            .log()
            .read(match_id)
            .into_iter()
            .filter(|event| matches!(event, Event::Join { .. }))
            .count()
    }
    assert_eq!(seats(&relay, &match_id), 2);

    // The same account again, from a new socket and under a different name and colour.
    let again = connection("bob-again");
    assert_eq!(
        join(&mut relay, &again, &match_id, "2", "Robert", "green"),
        None,
        "coming back is not an error"
    );
    assert_eq!(seats(&relay, &match_id), 2, "and it is not a second seat");

    let view = relay.view(&match_id);
    assert_eq!(view.seat_of_account(&discord("2")), Some(Seat(1)));
    assert_eq!(
        view.name_of(Seat(1)),
        "Bob",
        "the name of the first join is the one that stands"
    );

    // The returning socket holds the seat, and the one it replaced does not.
    let held = relay.connections().of(&match_id);
    let on_seat_one: Vec<_> = held.iter().filter(|row| row.seat == Seat(1)).collect();
    assert_eq!(on_seat_one.len(), 1, "one socket on the seat: {held:?}");
    assert_eq!(on_seat_one[0].connection, again);
}

/// A colour is not taken from yourself, and a full match is not full of you.
///
/// The guards that refuse a join are all about taking a seat, so none of them may fire
/// for somebody who already has one. Before the fix a return was refused `ColourTaken`
/// by its own colour, which is the kind of answer that sends somebody looking for a bug
/// in the colour picker.
#[test]
fn coming_back_is_not_refused_by_your_own_seat() {
    let mut relay = relay();
    let alice = connection("alice");
    let match_id = host(&mut relay, &alice);

    // Alice hosted, so she is on seat zero in red. Joining her own match again with the
    // same colour is a return.
    let outcome = relay.receive(
        &connection("alice-again"),
        Message::Join {
            match_id: match_id.clone(),
            account: discord("1"),
            name: "Alice".to_string(),
            colour: "red".to_string(),
            version: PROTOCOL_VERSION,
        },
    );
    assert_eq!(outcome.refused, None, "your own colour is not taken");

    // And after the start, when joining is closed but returning is not.
    relay.receive(
        &alice,
        Message::Start {
            match_id: match_id.clone(),
        },
    );
    let after = relay.receive(
        &connection("alice-third"),
        Message::Join {
            match_id: match_id.clone(),
            account: discord("1"),
            name: "Alice".to_string(),
            colour: "red".to_string(),
            version: PROTOCOL_VERSION,
        },
    );
    assert_eq!(
        after.refused, None,
        "a match that has started is one she is already in"
    );
}

/// `A match whose host has left cannot be joined`, and is not listed either.
///
/// Host powers are the seat in the log and nothing transfers them, so a match whose host
/// has gone is finished without having started. It used to sit in the lobby forever,
/// offering a start that could never come.
#[test]
fn a_match_whose_host_has_left_is_neither_listed_nor_joinable() {
    let mut relay = relay();
    let alice = connection("alice");
    let match_id = host(&mut relay, &alice);
    assert_eq!(
        relay.open_matches(None).len(),
        1,
        "listed while the host is there"
    );

    relay.receive(
        &alice,
        Message::Leave {
            match_id: match_id.clone(),
        },
    );

    assert!(
        relay.open_matches(None).is_empty(),
        "a match that cannot begin is not on offer"
    );
    assert_eq!(
        join(
            &mut relay,
            &connection("bob"),
            &match_id,
            "2",
            "Bob",
            "blue"
        ),
        Some(Refused::HostHasLeft),
        "and saying why beats letting somebody wait in it"
    );
}

/// `A player can leave a match`, and the record says so.
#[test]
fn a_seat_that_leaves_is_recorded_as_having_left() {
    let (mut relay, match_id) = two_player_match();
    let bob = connection("bob");

    let outcome = relay.receive(
        &bob,
        Message::Leave {
            match_id: match_id.clone(),
        },
    );
    assert_eq!(outcome.refused, None);
    assert!(
        relay
            .log()
            .read(&match_id)
            .contains(&Event::Left { seat: Seat(1) }),
        "leaving is membership, so it belongs in the log beside the join"
    );
    // The socket goes with them: a seat that has left holds no connection, so nothing is
    // sent to it and nothing waits for it.
    assert!(
        relay
            .connections()
            .of(&match_id)
            .iter()
            .all(|row| row.seat != Seat(1)),
        "a departed seat must hold no connection"
    );

    // Leaving twice is not an error and not a second event.
    let again = relay.receive(
        &bob,
        Message::Leave {
            match_id: match_id.clone(),
        },
    );
    assert_eq!(again.refused, Some(Refused::NotInThisMatch));
    let departures = relay
        .log()
        .read(&match_id)
        .into_iter()
        .filter(|event| matches!(event, Event::Left { .. }))
        .count();
    assert_eq!(departures, 1, "one departure, however often it is sent");
}

/// Somebody who is not in the match cannot leave it.
#[test]
fn a_stranger_cannot_leave_a_match() {
    let (mut relay, match_id) = two_player_match();
    assert_eq!(
        relay
            .receive(&connection("stranger"), Message::Leave { match_id })
            .refused,
        Some(Refused::NotInThisMatch)
    );
}

/// A player who reconnects resumes their seat, and is replayed through the same filter
/// as if they had never left.
#[test]
fn a_returning_player_resumes_their_seat_and_learns_nothing_extra() {
    let (mut relay, match_id) = two_player_match();
    let bob = connection("bob");
    relay.disconnect(&bob);

    let back = connection("bob-again");
    let resumed = relay.receive(
        &back,
        Message::Resume {
            match_id: match_id.clone(),
            account: discord("2"),
        },
    );
    assert_eq!(resumed.refused, None);
    assert_eq!(
        relay
            .connections()
            .of(&match_id)
            .iter()
            .find(|row| row.connection == back)
            .map(|row| row.seat),
        Some(Seat(1)),
        "the same seat, not a new one"
    );

    let seats_seen: Vec<Seat> = resumed
        .sends
        .iter()
        .filter_map(|(_, event)| match event {
            Event::Deal { seat, .. } => Some(*seat),
            _ => None,
        })
        .collect();
    assert_eq!(
        seats_seen,
        vec![Seat(1)],
        "the replay must not hand over the other seat's dice"
    );
}

#[test]
fn a_stranger_cannot_resume_a_seat() {
    let (mut relay, match_id) = two_player_match();
    assert_eq!(
        relay
            .receive(
                &connection("nobody"),
                Message::Resume {
                    match_id,
                    account: discord("404"),
                }
            )
            .refused,
        Some(Refused::NotInThisMatch)
    );
}

#[test]
fn a_seat_cannot_claim_as_another_seat() {
    let (mut relay, match_id) = two_player_match();
    assert_eq!(
        relay
            .receive(
                &connection("alice"),
                Message::Claim {
                    match_id,
                    event: Event::Liar {
                        seat: Seat(1),
                        odds: 10.0
                    },
                }
            )
            .refused,
        Some(Refused::NotYourSeat)
    );
}

/// `A repeated request to host creates no second match`.
///
/// The id is a function of the request, so the resend derives the same name, finds it
/// taken, finds the match is its own, and is answered with it. Nothing is written twice.
#[test]
fn hosting_the_same_request_twice_creates_one_match() {
    let mut relay = Relay::new(
        MemoryLog::new(),
        MemoryConnections::new(),
        Secret::from_bytes(MEDDLED_SECRET),
    );
    let request = Message::Host {
        account: meddled_host(),
        name: "Host".to_string(),
        colour: "red".to_string(),
        version: PROTOCOL_VERSION,
        nonce: HOST_NONCE.to_string(),
    };

    let first = relay.receive(&ConnectionId("socket-a".to_string()), request.clone());
    // A different socket, because the usual reason to resend is that the answer never
    // arrived — which often means the connection it would have arrived on is gone.
    let again = relay.receive(&ConnectionId("socket-b".to_string()), request);

    assert_eq!(again.refused, None, "a resend is not an error");
    assert_eq!(first.created, again.created, "two names for one match");
    let match_id = first.created.expect("a match");
    assert_eq!(relay.log().matches(), vec![match_id.clone()], "one match");
    assert_eq!(
        relay.log().read(&match_id).len(),
        3,
        "the resend must not append a second version, join and host"
    );
    // The resend's socket is the one now holding the seat, which is the point of
    // answering it rather than ignoring it.
    assert_eq!(
        relay.connections().of(&match_id),
        vec![relay::Connected {
            connection: ConnectionId("socket-b".to_string()),
            seat: Seat(0),
            role: relay::Role::Player
        }]
    );
}

/// A different request from the same account is a different match.
#[test]
fn hosting_twice_on_purpose_creates_two_matches() {
    let mut relay = Relay::new(
        MemoryLog::new(),
        MemoryConnections::new(),
        Secret::from_bytes(MEDDLED_SECRET),
    );
    let request = |nonce: &str| Message::Host {
        account: meddled_host(),
        name: "Host".to_string(),
        colour: "red".to_string(),
        version: PROTOCOL_VERSION,
        nonce: nonce.to_string(),
    };
    let one = relay.receive(&ConnectionId("socket-a".to_string()), request("one"));
    let two = relay.receive(&ConnectionId("socket-a".to_string()), request("two"));
    assert_ne!(one.created, two.created);
    assert_eq!(relay.log().matches().len(), 2);
}

/// The secret these tests host under, so the match id is computable in advance.
const MEDDLED_SECRET: [u8; 32] = [3; 32];
const HOST_NONCE: &str = "host-once";

fn meddled_host() -> Account {
    Account::Discord("account-host".to_string())
}

/// The match `seat_a_host` will create.
///
/// Derived rather than written down, because the id is now a function of the secret, the
/// account and the nonce. A test that guessed it would interfere with a match nobody
/// creates and would pass while proving nothing — which is what happened to the
/// exhaustive test below the moment ids stopped being `match-0`.
fn meddled_match() -> MatchId {
    MatchId(integrity::name_for(
        &Secret::from_bytes(MEDDLED_SECRET),
        &protocol::account_text(&meddled_host()),
        HOST_NONCE,
    ))
}

/// A log that lets somebody else in once, between a decision and the commit of it.
///
/// A thread race can only *probably* collide, so it can pass by luck and prove nothing.
/// This makes the window certain: the first append a relay attempts is preceded by
/// somebody else's, which makes the relay's position stale and forces it to decide again
/// against a log it has not seen before.
struct Meddling {
    inner: MemoryLog,
    /// How many appends to let through first. The host's message is one of them.
    after: usize,
    /// Appended just before the chosen commit attempt, then forgotten.
    interference: Option<(MatchId, Vec<Event>)>,
}

impl Meddling {
    /// Interfere once, just before the append that follows `after` of them.
    fn interfering_after(after: usize, match_id: &MatchId, events: Vec<Event>) -> Self {
        Self {
            inner: MemoryLog::new(),
            after,
            interference: Some((match_id.clone(), events)),
        }
    }
}

impl Log for Meddling {
    fn append(&mut self, match_id: &MatchId, from: u64, events: &[Event]) -> Result<(), Taken> {
        if self.after > 0 {
            self.after -= 1;
        } else if let Some((elsewhere, interfering)) = self.interference.take() {
            let at = self.inner.read(&elsewhere).len() as u64;
            self.inner
                .append(&elsewhere, at, &interfering)
                .expect("the interloper lands at the end");
        }
        self.inner.append(match_id, from, events)
    }

    fn read(&self, match_id: &MatchId) -> Vec<Event> {
        self.inner.read(match_id)
    }

    fn matches(&self) -> Vec<MatchId> {
        self.inner.matches()
    }
}

impl Meddling {
    /// Whether the interference actually fired.
    ///
    /// A test that asserts an invariant after an interference that never happened is
    /// green and empty. This is how each of them says it was really interfered with.
    fn spent(&self) -> bool {
        self.interference.is_none()
    }
}

/// Interference at every point a message could be interrupted, not one chosen point.
///
/// Behind the `Log` seam the places a schedule can diverge are finite and countable: one
/// per append. So the right test is not a representative interleaving but all of them,
/// which is what makes this certainty rather than a sample.
#[test]
fn a_join_survives_interference_at_every_append() {
    // More appends than any message makes, so the last iterations interfere after the
    // join has already committed and the invariant still has to hold.
    for after in 0..6 {
        let match_id = meddled_match();
        let interloper = Event::Join {
            seat: Seat(1),
            name: "Interloper".to_string(),
            colour: "blue".to_string(),
            account: Account::Discord("account-x".to_string()),
        };
        let mut relay = Relay::new(
            Meddling::interfering_after(after, &match_id, vec![interloper]),
            MemoryConnections::new(),
            Secret::from_bytes(MEDDLED_SECRET),
        );

        // The host's message is itself subject to the interference. At `after == 0` the
        // interloper creates this exact match first, and since the id is a function of
        // the request there is no second name to fall back to — so hosting is refused,
        // which is the only iteration where that is the right answer.
        let outcome = relay.receive(
            &ConnectionId("host".to_string()),
            Message::Host {
                account: meddled_host(),
                name: "Host".to_string(),
                colour: "red".to_string(),
                version: PROTOCOL_VERSION,
                nonce: HOST_NONCE.to_string(),
            },
        );
        if after == 0 {
            assert_eq!(
                outcome.refused,
                Some(Refused::MatchIdTaken),
                "a name taken by another account must not be appended to"
            );
            continue;
        }
        let Some(created) = outcome.created else {
            panic!("interfering after {after} stopped a match being created");
        };
        assert_eq!(created, match_id, "the interference named the wrong match");

        let outcome = relay.receive(
            &ConnectionId("late".to_string()),
            Message::Join {
                match_id: created.clone(),
                account: Account::Discord("account-late".to_string()),
                name: "Late".to_string(),
                colour: "green".to_string(),
                version: PROTOCOL_VERSION,
            },
        );
        assert_eq!(
            outcome.refused, None,
            "interfering after {after} refused a join that should have been re-decided"
        );

        // The invariant, whatever the schedule was: no two joins share a seat, and the
        // seats are the ones a match actually dealt out.
        let seats: Vec<u8> = relay
            .log()
            .read(&created)
            .into_iter()
            .filter_map(|event| match event {
                Event::Join { seat, .. } => Some(seat.0),
                _ => None,
            })
            .collect();
        let mut sorted = seats.clone();
        sorted.sort_unstable();
        let mut distinct = sorted.clone();
        distinct.dedup();
        assert_eq!(
            sorted, distinct,
            "interfering after {after} put two players on one seat: {seats:?}"
        );
        assert_eq!(
            sorted,
            (0..sorted.len() as u8).collect::<Vec<u8>>(),
            "interfering after {after} left a gap in the seats: {seats:?}"
        );
    }
}

/// `Two players never share a seat`, with the race made certain rather than likely.
///
/// The seat came from counting the seats in the log, so a join decided against a log that
/// has since gained a seat must be decided again. Before the position was a version, the
/// refused append moved to the next position and the two joins landed on one seat.
#[test]
fn a_join_decided_against_a_stale_log_is_decided_again() {
    let match_id = meddled_match();
    let interloper = Event::Join {
        seat: Seat(1),
        name: "Interloper".to_string(),
        colour: "blue".to_string(),
        account: Account::Discord("account-x".to_string()),
    };
    let mut relay = Relay::new(
        Meddling::interfering_after(1, &match_id, vec![interloper]),
        MemoryConnections::new(),
        Secret::from_bytes(MEDDLED_SECRET),
    );
    seat_a_host(&mut relay, &match_id);

    let outcome = relay.receive(
        &ConnectionId("late".to_string()),
        Message::Join {
            match_id: match_id.clone(),
            account: Account::Discord("account-late".to_string()),
            name: "Late".to_string(),
            colour: "green".to_string(),
            version: PROTOCOL_VERSION,
        },
    );
    assert_eq!(outcome.refused, None);
    assert!(
        relay.log().spent(),
        "the interference never fired, so this proved nothing"
    );

    let seats: Vec<u8> = relay
        .log()
        .read(&match_id)
        .into_iter()
        .filter_map(|event| match event {
            Event::Join { seat, .. } => Some(seat.0),
            _ => None,
        })
        .collect();
    assert_eq!(
        seats,
        vec![0, 1, 2],
        "the late joiner must take the seat after the one that appeared under it"
    );
}

/// A guard is decided by the commit, not merely consulted before it.
///
/// The colour was free when the join was decided and taken by the time it landed, so the
/// re-decision refuses it. `A colour another player has chosen cannot be chosen` was
/// already a statement; this is the first test that holds it under a race.
#[test]
fn a_colour_taken_while_a_join_was_decided_is_refused_on_the_retry() {
    let match_id = meddled_match();
    let interloper = Event::Join {
        seat: Seat(1),
        name: "Interloper".to_string(),
        colour: "green".to_string(),
        account: Account::Discord("account-x".to_string()),
    };
    let mut relay = Relay::new(
        Meddling::interfering_after(1, &match_id, vec![interloper]),
        MemoryConnections::new(),
        Secret::from_bytes(MEDDLED_SECRET),
    );
    seat_a_host(&mut relay, &match_id);

    let outcome = relay.receive(
        &ConnectionId("late".to_string()),
        Message::Join {
            match_id: match_id.clone(),
            account: Account::Discord("account-late".to_string()),
            name: "Late".to_string(),
            // Free when this was decided. Taken by the time it tried to land.
            colour: "green".to_string(),
            version: PROTOCOL_VERSION,
        },
    );
    assert_eq!(outcome.refused, Some(Refused::ColourTaken));
    assert!(
        relay.log().spent(),
        "the interference never fired, so this proved nothing"
    );
    // By name, not by count: two joins is also what an interference that never fired
    // would leave, and a test that cannot tell those apart proves nothing.
    let names: Vec<String> = relay
        .log()
        .read(&match_id)
        .into_iter()
        .filter_map(|event| match event {
            Event::Join { name, .. } => Some(name),
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        vec!["Host".to_string(), "Interloper".to_string()],
        "the interloper must be in and the refused join must leave nothing behind"
    );
}

/// Host a match over whatever log the relay was given, and return nothing: the tests
/// above care about what the log holds afterwards rather than about the outcome.
fn seat_a_host<L: Log, C: Connections>(relay: &mut Relay<L, C>, expected: &MatchId) {
    let created = relay
        .receive(
            &ConnectionId("host".to_string()),
            Message::Host {
                account: meddled_host(),
                name: "Host".to_string(),
                colour: "red".to_string(),
                version: PROTOCOL_VERSION,
                nonce: HOST_NONCE.to_string(),
            },
        )
        .created
        .expect("a match");
    assert_eq!(
        &created, expected,
        "the test's interference names this match"
    );
}

/// The conditional append is what gives the log a total order without a lock.
///
/// The position is a version number, so a refused append means the log moved under the
/// caller and whatever it decided from reading that log is stale. This test used to
/// append the loser's event at the next position and assert it landed, which is the
/// defect `relay.md` records: it made the loser's decision come true after the fact.
#[test]
fn an_append_at_a_taken_position_is_refused() {
    let mut log = MemoryLog::new();
    let match_id = MatchId("m".to_string());
    let event = Event::Start;
    assert!(
        log.append(&match_id, 0, std::slice::from_ref(&event))
            .is_ok()
    );
    assert!(
        log.append(&match_id, 0, std::slice::from_ref(&event))
            .is_err(),
        "position zero is taken"
    );
    // The loser's recourse is to read again and decide again, not to move along. Here
    // that means appending at the length it now sees.
    let from = log.read(&match_id).len() as u64;
    assert!(log.append(&match_id, from, &[event]).is_ok());
    assert_eq!(log.read(&match_id).len(), 2);
}

/// `A message either happens entirely or not at all`.
#[test]
fn a_message_that_cannot_land_whole_lands_not_at_all() {
    let mut log = MemoryLog::new();
    let match_id = MatchId("m".to_string());
    let three = [
        Event::Version { protocol: 3 },
        Event::Host { seat: Seat(0) },
        Event::Start,
    ];
    // Somebody else got there first, so the whole message is refused.
    assert!(log.append(&match_id, 0, &[Event::Start]).is_ok());
    assert!(log.append(&match_id, 0, &three).is_err());
    assert_eq!(
        log.read(&match_id),
        vec![Event::Start],
        "a refused message must leave nothing of itself behind"
    );
    // And from the position the log is actually at, all three land together.
    assert!(log.append(&match_id, 1, &three).is_ok());
    assert_eq!(log.read(&match_id).len(), 4);
}

/// A relay may be stopped and replaced between any two messages without affecting a match
/// in progress. Both halves of the state outlive the process, so a second instance picks
/// the match up mid-round.
#[test]
fn a_replacement_relay_continues_a_round_in_progress() {
    let mut log = MemoryLog::new();
    let mut connections = MemoryConnections::new();
    let alice = connection("alice");
    let bob = connection("bob");

    let match_id = {
        let mut first = Relay::new(&mut log, &mut connections, secret());
        let match_id = host(&mut first, &alice);
        join(&mut first, &bob, &match_id, "2", "Bob", "blue");
        first.receive(
            &alice,
            Message::Start {
                match_id: match_id.clone(),
            },
        );
        first.receive(
            &alice,
            Message::RoundOver {
                match_id: match_id.clone(),
            },
        );
        match_id
    };

    // The first relay is gone. Nothing of it survives but the log and the connections.
    let mut second = Relay::new(&mut log, &mut connections, secret());
    let finished = second.receive(
        &bob,
        Message::RoundOver {
            match_id: match_id.clone(),
        },
    );
    assert!(
        finished
            .sends
            .iter()
            .any(|(_, event)| matches!(event, Event::Reveal { .. })),
        "the replacement finished the round the first one started"
    );
    assert_eq!(second.view(&match_id).round, 1);
}
