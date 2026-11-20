//! A whole match, two relays, a disconnect and a return — in one process.
//!
//! These are the tests /implementation/relay.md asks for, and they are the reason the
//! relay is a library with two traits rather than a Lambda handler: multiplayer can be
//! proved before any infrastructure exists.

use integrity::Secret;
use protocol::{Account, Event, Moment, PROTOCOL_VERSION, Seat, Who};
use relay::memory::{MemoryConnections, MemoryLog};
use relay::{
    ABANDON_AFTER_SECONDS, CANCEL_AFTER_SECONDS, Connected, ConnectionId, Connections, Ending, Log,
    MatchId, Message, Recording, Refused, Relay, Role, Taken,
};

fn secret() -> Secret {
    Secret::from_bytes([3u8; 32])
}

/// The moment every message in these tests arrives at, unless a test says otherwise.
///
/// Arbitrary, round, and in the past. With every message at the same moment no match is
/// ever inactive, so nothing expires and no inactivity gate opens by accident: a test that
/// wants time to have passed says so with [`later`].
fn moment() -> Moment {
    Moment::from_unix_nanos(1_700_000_000_000_000_000)
}

/// [`moment`], some seconds on. The clock is an argument, so this is the whole of how
/// these tests make time pass — nothing sleeps and nothing is flaky.
fn later(seconds: u64) -> Moment {
    Moment::from_unix_nanos(1_700_000_000_000_000_000 + seconds * 1_000_000_000)
}

fn relay() -> Relay<MemoryLog, MemoryConnections> {
    Relay::new(MemoryLog::new(), MemoryConnections::new(), secret())
}

/// An unauthenticated account, which is what a client can actually produce.
///
/// These tests used to say `discord`, and could, because a client sent whatever account it
/// liked. Now a `Discord` account exists only behind a ticket the relay verifies, so a
/// test that wants one mints it — see the ranked tests.
fn guest(id: &str) -> Account {
    Account::Guest(id.to_string())
}

/// A proven account, which only a ticket can produce.
fn discord_account(id: &str) -> Account {
    Account::Discord(id.to_string())
}

/// What a client sends to say who it is.
fn guest_who(id: &str, name: &str) -> Who {
    Who::Guest {
        id: id.to_string(),
        name: name.to_string(),
    }
}

fn connection(name: &str) -> ConnectionId {
    ConnectionId(name.to_string())
}

fn host<L: Log, C: Connections>(relay: &mut Relay<L, C>, who: &ConnectionId) -> MatchId {
    let outcome = relay.receive(
        who,
        Message::Host {
            who: guest_who("1", "Alice"),
            colour: "red".to_string(),
            version: PROTOCOL_VERSION,
            nonce: "alice-hosts".to_string(),
        },
        moment(),
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
                who: guest_who(account, name),
                colour: colour.to_string(),
                version: PROTOCOL_VERSION,
            },
            moment(),
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
        moment(),
    );
    assert_eq!(started.refused, None);
    (relay, match_id)
}

#[test]
fn a_match_that_has_not_started_is_listed_and_one_that_has_is_not() {
    let mut relay = relay();
    let alice = connection("alice");
    let match_id = host(&mut relay, &alice);

    let listed = relay.open_matches(None, moment());
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].host, "Alice");
    assert_eq!(listed[0].players, 1);

    relay.receive(
        &alice,
        Message::Start {
            match_id: match_id.clone(),
        },
        moment(),
    );
    assert!(
        relay.open_matches(None, moment()).is_empty(),
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
            who: guest_who("9", "asking"),
        },
        moment(),
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
                who: guest_who("9", "asking"),
            },
            moment(),
        )
        .listed
        .expect("a listing");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].match_id, match_id);
    assert_eq!(listed[0].host, "Alice", "a listed match shows its host");
    assert_eq!(listed[0].players, 1, "and how many have joined");

    // Asking changes nothing: the log is as long as hosting left it.
    assert_eq!(relay.log().read(&match_id).events.len(), 3);

    relay.receive(&alice, Message::Start { match_id }, moment());
    let after = relay
        .receive(
            &looking,
            Message::List {
                who: guest_who("9", "asking"),
            },
            moment(),
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
        moment(),
    );

    let to_bob = relay
        .receive(
            &bob,
            Message::List {
                who: guest_who("2", "asking"),
            },
            moment(),
        )
        .listed
        .expect("a listing");
    assert!(to_bob.is_empty(), "a barred player is not shown the match");

    let to_anybody = relay
        .receive(
            &connection("carol"),
            Message::List {
                who: guest_who("3", "asking"),
            },
            moment(),
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
        moment(),
    );
    assert!(relay.open_matches(Some(&guest("2")), moment()).is_empty());
    assert_eq!(relay.open_matches(Some(&guest("3")), moment()).len(), 1);

    relay.receive(
        &alice,
        Message::Lift {
            match_id: match_id.clone(),
            seat: Seat(1),
        },
        moment(),
    );
    assert_eq!(
        relay.open_matches(Some(&guest("2")), moment()).len(),
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
                },
                moment()
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
                },
                moment()
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
            who: guest_who("9", "Old"),
            colour: "green".to_string(),
            version: PROTOCOL_VERSION - 1,
        },
        moment(),
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
        moment(),
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
        moment(),
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
        .events
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
        moment(),
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
        moment(),
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
        moment(),
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
        moment(),
    );

    relay.receive(
        &alice,
        Message::RoundOver {
            match_id: match_id.clone(),
        },
        moment(),
    );
    let last = relay.receive(
        &bob,
        Message::RoundOver {
            match_id: match_id.clone(),
        },
        moment(),
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
        moment(),
    );

    let only = relay.receive(
        &connection("alice"),
        Message::RoundOver {
            match_id: match_id.clone(),
        },
        moment(),
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
        moment(),
    );
    assert_eq!(outcome.refused, None, "absence is not the relay's business");
    assert!(
        relay.log().read(&match_id).events.contains(&claim),
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
            .events
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
    assert_eq!(view.seat_of_account(&guest("2")), Some(Seat(1)));
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
            who: guest_who("1", "Alice"),
            colour: "red".to_string(),
            version: PROTOCOL_VERSION,
        },
        moment(),
    );
    assert_eq!(outcome.refused, None, "your own colour is not taken");

    // And after the start, when joining is closed but returning is not.
    relay.receive(
        &alice,
        Message::Start {
            match_id: match_id.clone(),
        },
        moment(),
    );
    let after = relay.receive(
        &connection("alice-third"),
        Message::Join {
            match_id: match_id.clone(),
            who: guest_who("1", "Alice"),
            colour: "red".to_string(),
            version: PROTOCOL_VERSION,
        },
        moment(),
    );
    assert_eq!(
        after.refused, None,
        "a match that has started is one she is already in"
    );
}

/// `A host who leaves a match that has not started cancels it`.
///
/// Host powers are the seat in the log and nothing transfers them, so a match whose host
/// has gone is finished without having started. It used to sit in the lobby forever,
/// offering a start that could never come; then it had a refusal of its own,
/// `HostHasLeft`, which was a second name for *this match is finished*. Now it is a
/// cancellation, recorded as the same event the Cancel button records.
#[test]
fn a_match_whose_host_has_left_is_neither_listed_nor_joinable() {
    let mut relay = relay();
    let alice = connection("alice");
    let match_id = host(&mut relay, &alice);
    assert_eq!(
        relay.open_matches(None, moment()).len(),
        1,
        "listed while the host is there"
    );

    relay.receive(
        &alice,
        Message::Leave {
            match_id: match_id.clone(),
        },
        moment(),
    );

    assert!(
        relay.open_matches(None, moment()).is_empty(),
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
        Some(Refused::MatchIsOver),
        "and saying why beats letting somebody wait in it"
    );
    assert_eq!(
        relay.view(&match_id).ending,
        Some(Ending::Cancelled { by: Seat(0) }),
        "the log says the host cancelled it, not merely that they left"
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
        moment(),
    );
    assert_eq!(outcome.refused, None);
    assert!(
        relay
            .log()
            .read(&match_id)
            .events
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
        moment(),
    );
    assert_eq!(again.refused, Some(Refused::NotInThisMatch));
    let departures = relay
        .log()
        .read(&match_id)
        .events
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
            .receive(
                &connection("stranger"),
                Message::Leave { match_id },
                moment()
            )
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
            who: guest_who("2", "asking"),
        },
        moment(),
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
                    who: guest_who("404", "asking"),
                },
                moment()
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
                },
                moment()
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
        who: meddled_host_who(),
        colour: "red".to_string(),
        version: PROTOCOL_VERSION,
        nonce: HOST_NONCE.to_string(),
    };

    let first = relay.receive(
        &ConnectionId("socket-a".to_string()),
        request.clone(),
        moment(),
    );
    // A different socket, because the usual reason to resend is that the answer never
    // arrived — which often means the connection it would have arrived on is gone.
    let again = relay.receive(&ConnectionId("socket-b".to_string()), request, moment());

    assert_eq!(again.refused, None, "a resend is not an error");
    assert_eq!(first.created, again.created, "two names for one match");
    let match_id = first.created.expect("a match");
    assert_eq!(relay.log().matches(), vec![match_id.clone()], "one match");
    assert_eq!(
        relay.log().read(&match_id).events.len(),
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
        who: meddled_host_who(),
        colour: "red".to_string(),
        version: PROTOCOL_VERSION,
        nonce: nonce.to_string(),
    };
    let one = relay.receive(
        &ConnectionId("socket-a".to_string()),
        request("one"),
        moment(),
    );
    let two = relay.receive(
        &ConnectionId("socket-a".to_string()),
        request("two"),
        moment(),
    );
    assert_ne!(one.created, two.created);
    assert_eq!(relay.log().matches().len(), 2);
}

/// The secret these tests host under, so the match id is computable in advance.
const MEDDLED_SECRET: [u8; 32] = [3; 32];
const HOST_NONCE: &str = "host-once";

fn meddled_host() -> Account {
    Account::Guest("account-host".to_string())
}

fn meddled_host_who() -> Who {
    Who::Guest {
        id: "account-host".to_string(),
        name: "Host".to_string(),
    }
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
    fn append(
        &mut self,
        match_id: &MatchId,
        from: u64,
        events: &[Event],
        now: Moment,
    ) -> Result<(), Taken> {
        if self.after > 0 {
            self.after -= 1;
        } else if let Some((elsewhere, interfering)) = self.interference.take() {
            let at = self.inner.read(&elsewhere).events.len() as u64;
            self.inner
                .append(&elsewhere, at, &interfering, now)
                .expect("the interloper lands at the end");
        }
        self.inner.append(match_id, from, events, now)
    }

    fn read(&self, match_id: &MatchId) -> Recording {
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
                who: meddled_host_who(),
                colour: "red".to_string(),
                version: PROTOCOL_VERSION,
                nonce: HOST_NONCE.to_string(),
            },
            moment(),
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
                who: guest_who("account-late", "Late"),
                colour: "green".to_string(),
                version: PROTOCOL_VERSION,
            },
            moment(),
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
            .events
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
            who: guest_who("account-late", "Late"),
            colour: "green".to_string(),
            version: PROTOCOL_VERSION,
        },
        moment(),
    );
    assert_eq!(outcome.refused, None);
    assert!(
        relay.log().spent(),
        "the interference never fired, so this proved nothing"
    );

    let seats: Vec<u8> = relay
        .log()
        .read(&match_id)
        .events
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
            who: guest_who("account-late", "Late"),
            // Free when this was decided. Taken by the time it tried to land.
            colour: "green".to_string(),
            version: PROTOCOL_VERSION,
        },
        moment(),
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
        .events
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
                who: meddled_host_who(),
                colour: "red".to_string(),
                version: PROTOCOL_VERSION,
                nonce: HOST_NONCE.to_string(),
            },
            moment(),
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
        log.append(&match_id, 0, std::slice::from_ref(&event), moment())
            .is_ok()
    );
    assert!(
        log.append(&match_id, 0, std::slice::from_ref(&event), moment())
            .is_err(),
        "position zero is taken"
    );
    // The loser's recourse is to read again and decide again, not to move along. Here
    // that means appending at the length it now sees.
    let from = log.read(&match_id).events.len() as u64;
    assert!(log.append(&match_id, from, &[event], moment()).is_ok());
    assert_eq!(log.read(&match_id).events.len(), 2);
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
    assert!(log.append(&match_id, 0, &[Event::Start], moment()).is_ok());
    assert!(log.append(&match_id, 0, &three, moment()).is_err());
    assert_eq!(
        log.read(&match_id).events,
        vec![Event::Start],
        "a refused message must leave nothing of itself behind"
    );
    // And from the position the log is actually at, all three land together.
    assert!(log.append(&match_id, 1, &three, moment()).is_ok());
    assert_eq!(log.read(&match_id).events.len(), 4);
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
            moment(),
        );
        first.receive(
            &alice,
            Message::RoundOver {
                match_id: match_id.clone(),
            },
            moment(),
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
        moment(),
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

// ---------------------------------------------------------------------------------------
// Ending a match: /implementation/ending.md
// ---------------------------------------------------------------------------------------

/// Ask to cancel, at a given number of seconds after the match's last event.
fn cancel<L: Log, C: Connections>(
    relay: &mut Relay<L, C>,
    who: &ConnectionId,
    match_id: &MatchId,
    seconds: u64,
) -> Option<Refused> {
    relay
        .receive(
            who,
            Message::Cancel {
                match_id: match_id.clone(),
            },
            later(seconds),
        )
        .refused
}

/// `The host can cancel a match that has not started`, with no minute to wait.
///
/// There is no turn to wait on before the start, and a minute of the host watching a
/// timer for no reason is a minute of them wondering what is broken.
#[test]
fn the_host_cancels_a_match_that_has_not_started_at_once() {
    let mut relay = relay();
    let alice = connection("alice");
    let match_id = host(&mut relay, &alice);

    assert_eq!(cancel(&mut relay, &alice, &match_id, 0), None);
    assert_eq!(
        relay.view(&match_id).ending,
        Some(Ending::Cancelled { by: Seat(0) })
    );
    assert!(relay.view(&match_id).over());
}

/// `The host cannot cancel a started match in which an event has been recorded within the
/// minute`, and `A cancel the host cannot yet make is refused with the time remaining`.
#[test]
fn cancelling_a_live_match_is_refused_with_the_time_remaining() {
    let (mut relay, match_id) = two_player_match();
    let alice = connection("alice");

    assert_eq!(
        cancel(&mut relay, &alice, &match_id, 0),
        Some(Refused::StillActive { seconds: 60 }),
        "the whole minute remains the instant the match starts"
    );
    assert_eq!(
        cancel(&mut relay, &alice, &match_id, 59),
        Some(Refused::StillActive { seconds: 1 }),
        "one second left, not none"
    );
    assert_eq!(
        cancel(&mut relay, &alice, &match_id, 60),
        None,
        "and on the minute it goes through"
    );
}

/// The remaining seconds round up, so the number a host reads is never a lie.
///
/// Told *one*, a host who asks again on the next second must not be told *zero* and left
/// looking at a button that does nothing.
#[test]
fn the_time_remaining_rounds_up() {
    let (mut relay, match_id) = two_player_match();
    let alice = connection("alice");
    // Fifty-nine and a half seconds after the start: half a second remains, which reads
    // as one.
    let refused = relay
        .receive(
            &alice,
            Message::Cancel {
                match_id: match_id.clone(),
            },
            Moment::from_unix_nanos(moment().to_unix_nanos() + 59_500_000_000),
        )
        .refused;
    assert_eq!(refused, Some(Refused::StillActive { seconds: 1 }));
}

/// A refused cancel is not an event, so it does not reset the minute it is waiting for.
///
/// Otherwise a host who presses the button twice never gets there, which looks exactly
/// like the feature not working.
#[test]
fn a_refused_cancel_does_not_reset_the_clock() {
    let (mut relay, match_id) = two_player_match();
    let alice = connection("alice");
    let before = relay.log().read(&match_id).events.len();

    assert!(cancel(&mut relay, &alice, &match_id, 30).is_some());
    assert!(cancel(&mut relay, &alice, &match_id, 59).is_some());
    assert_eq!(
        relay.log().read(&match_id).events.len(),
        before,
        "two refusals wrote nothing"
    );
    assert_eq!(
        cancel(&mut relay, &alice, &match_id, 60),
        None,
        "so the minute is still measured from the start"
    );
}

/// `Only the host can cancel a match`.
#[test]
fn a_player_who_is_not_the_host_cannot_cancel() {
    let (mut relay, match_id) = two_player_match();
    let bob = connection("bob");
    assert_eq!(
        cancel(&mut relay, &bob, &match_id, CANCEL_AFTER_SECONDS),
        Some(Refused::NotTheHost)
    );
    let stranger = connection("stranger");
    assert_eq!(
        cancel(&mut relay, &stranger, &match_id, CANCEL_AFTER_SECONDS),
        Some(Refused::NotInThisMatch)
    );
    assert!(!relay.view(&match_id).over());
}

/// `A match in which no event has been recorded for an hour is cancelled`, noticed by
/// whatever next reads it.
///
/// There is no monitor. The lobby pass reads every match already, so the hour is noticed
/// there and nowhere else.
#[test]
fn a_match_untouched_for_an_hour_is_abandoned_by_the_next_listing() {
    let mut relay = relay();
    let alice = connection("alice");
    let match_id = host(&mut relay, &alice);

    assert_eq!(
        relay
            .open_matches(None, later(ABANDON_AFTER_SECONDS - 1))
            .len(),
        1,
        "a minute short of the hour it is still on offer"
    );
    assert!(!relay.view(&match_id).over());

    assert!(
        relay
            .open_matches(None, later(ABANDON_AFTER_SECONDS))
            .is_empty(),
        "and on the hour it is gone"
    );
    assert_eq!(relay.view(&match_id).ending, Some(Ending::Abandoned));
}

/// The sweep is idempotent, for the same reason the listing is honest.
///
/// `A match that is over is not listed` is tested before the hour is, so an abandoned
/// match — which is over — is skipped on the next pass. One test, both jobs, and no
/// separate guard against writing a second `Abandon`.
#[test]
fn listing_twice_abandons_a_match_once() {
    let mut relay = relay();
    let alice = connection("alice");
    let match_id = host(&mut relay, &alice);
    for _ in 0..3 {
        relay.open_matches(None, later(ABANDON_AFTER_SECONDS));
    }
    let abandons = relay
        .log()
        .read(&match_id)
        .events
        .into_iter()
        .filter(|event| matches!(event, Event::Abandon))
        .count();
    assert_eq!(abandons, 1);
}

/// A match whose rows carry no recorded time is abandoned on the first listing.
///
/// Those are the matches written before this build. `None` means inactive since forever,
/// not *unknown, so not expired* — which would have left them in the lobby permanently,
/// and that is the problem the hour rule exists to solve.
#[test]
fn a_match_with_no_recorded_time_is_abandoned_at_once() {
    let mut log = MemoryLog::new();
    let match_id = MatchId("from-before".to_string());
    // Written straight to the log, as the old build would have left it.
    log.append(
        &match_id,
        0,
        &[
            Event::Version {
                protocol: PROTOCOL_VERSION,
            },
            Event::Join {
                seat: Seat(0),
                name: "Mary Jane".to_string(),
                colour: "red".to_string(),
                account: guest("1"),
            },
            Event::Host { seat: Seat(0) },
        ],
        moment(),
    )
    .expect("an empty log");
    // And then its recorded time is taken away, which is what an old row looks like.
    let mut relay = Relay::new(Untimed { inner: log }, MemoryConnections::new(), secret());

    assert!(relay.open_matches(None, moment()).is_empty());
    assert_eq!(relay.view(&match_id).ending, Some(Ending::Abandoned));
}

/// A log that reports no recorded time, however recently it was written.
struct Untimed {
    inner: MemoryLog,
}

impl Log for Untimed {
    fn append(
        &mut self,
        match_id: &MatchId,
        from: u64,
        events: &[Event],
        now: Moment,
    ) -> Result<(), Taken> {
        self.inner.append(match_id, from, events, now)
    }

    fn read(&self, match_id: &MatchId) -> Recording {
        Recording {
            events: self.inner.read(match_id).events,
            last: None,
        }
    }

    fn matches(&self) -> Vec<MatchId> {
        self.inner.matches()
    }
}

/// `No event is recorded in a match that is over`, which is one guard and not five.
///
/// Every message that would append is refused, and the two that would not are let
/// through: a listing is about every match, and a resume has to work on a finished one
/// because somebody reconnecting wants to see how it ended.
#[test]
fn nothing_is_recorded_in_a_match_that_is_over() {
    let (mut relay, match_id) = two_player_match();
    let alice = connection("alice");
    let bob = connection("bob");
    assert_eq!(
        cancel(&mut relay, &alice, &match_id, CANCEL_AFTER_SECONDS),
        None
    );
    let after = relay.log().read(&match_id).events.len();

    let refused = |relay: &mut Relay<MemoryLog, MemoryConnections>, message: Message| {
        relay
            .receive(&alice, message, later(CANCEL_AFTER_SECONDS))
            .refused
    };
    for message in [
        Message::Start {
            match_id: match_id.clone(),
        },
        Message::Leave {
            match_id: match_id.clone(),
        },
        Message::RoundOver {
            match_id: match_id.clone(),
        },
        Message::Over {
            match_id: match_id.clone(),
        },
        Message::Cancel {
            match_id: match_id.clone(),
        },
        Message::Bar {
            match_id: match_id.clone(),
            seat: Seat(1),
        },
        Message::Claim {
            match_id: match_id.clone(),
            event: Event::Liar {
                seat: Seat(0),
                odds: 50.0,
            },
        },
    ] {
        assert_eq!(
            refused(&mut relay, message.clone()),
            Some(Refused::MatchIsOver),
            "{message:?} was not refused"
        );
    }
    assert_eq!(
        join(
            &mut relay,
            &connection("carol"),
            &match_id,
            "3",
            "Carol",
            "green"
        ),
        Some(Refused::MatchIsOver),
        "`A cancelled match cannot be joined`"
    );
    assert_eq!(
        relay.log().read(&match_id).events.len(),
        after,
        "and none of them left anything behind"
    );

    // The two that still work.
    let resumed = relay.receive(
        &bob,
        Message::Resume {
            match_id: match_id.clone(),
            who: guest_who("2", "asking"),
        },
        later(CANCEL_AFTER_SECONDS),
    );
    assert_eq!(
        resumed.refused, None,
        "a seat may return to see how it ended"
    );
    assert!(
        resumed
            .sends
            .iter()
            .any(|(_, event)| matches!(event, Event::Cancel { .. })),
        "and is told the ending"
    );
}

/// `A match is complete when every seat present in it says so`.
///
/// Counted, not judged: the relay writes no resolution event and may not derive one. One
/// seat's word is recorded and is not enough; the last present seat's word completes it.
#[test]
fn a_match_is_complete_once_every_present_seat_says_so() {
    let (mut relay, match_id) = two_player_match();
    let alice = connection("alice");
    let bob = connection("bob");

    let said = relay.receive(
        &alice,
        Message::Over {
            match_id: match_id.clone(),
        },
        moment(),
    );
    assert_eq!(said.refused, None);
    assert!(
        relay.view(&match_id).has_finished(Seat(0)),
        "her word is recorded"
    );
    assert_eq!(
        relay.view(&match_id).ending,
        None,
        "and one seat of two is not every seat"
    );

    let last = relay.receive(
        &bob,
        Message::Over {
            match_id: match_id.clone(),
        },
        moment(),
    );
    assert_eq!(last.refused, None);
    assert_eq!(relay.view(&match_id).ending, Some(Ending::Complete));
    assert!(
        last.sends
            .iter()
            .any(|(_, event)| matches!(event, Event::Complete)),
        "and everybody is told"
    );
}

/// A seat that is not there does not hold up the completion of a match it has left.
///
/// Presence, counted the same way the reveal counts it. A seat that has left is not
/// waited for; a seat that is merely absent is, and the hour rule collects those.
#[test]
fn a_seat_that_has_left_is_not_waited_for_at_the_end() {
    let (mut relay, match_id) = two_player_match();
    let alice = connection("alice");
    let bob = connection("bob");

    relay.receive(
        &bob,
        Message::Leave {
            match_id: match_id.clone(),
        },
        moment(),
    );
    relay.receive(
        &alice,
        Message::Over {
            match_id: match_id.clone(),
        },
        moment(),
    );
    assert_eq!(
        relay.view(&match_id).ending,
        Some(Ending::Complete),
        "the only seat still at the table said so, so that is every seat present"
    );
}

/// A match that is already over is not abandoned, and the hour does not overwrite a real
/// ending.
///
/// This is why the rule is `No event is recorded in a match that is over` and not one
/// about cancelled matches: a finished match goes quiet, so a rule that only knew about
/// cancellation would have the hour rule cancel every match anybody ever won.
#[test]
fn an_hour_of_silence_does_not_overwrite_a_finished_match() {
    let (mut relay, match_id) = two_player_match();
    let alice = connection("alice");
    let bob = connection("bob");
    for who in [&alice, &bob] {
        relay.receive(
            who,
            Message::Over {
                match_id: match_id.clone(),
            },
            moment(),
        );
    }
    assert_eq!(relay.view(&match_id).ending, Some(Ending::Complete));

    relay.open_matches(None, later(ABANDON_AFTER_SECONDS * 10));
    assert_eq!(
        relay.view(&match_id).ending,
        Some(Ending::Complete),
        "still complete, and not abandoned on top of it"
    );
}

/// `A match that is over is not listed`, however it ended.
#[test]
fn a_match_that_is_over_is_not_listed() {
    for ending in ["cancelled", "abandoned"] {
        let mut relay = relay();
        let alice = connection("alice");
        let match_id = host(&mut relay, &alice);
        assert_eq!(relay.open_matches(None, moment()).len(), 1);
        match ending {
            "cancelled" => {
                assert_eq!(cancel(&mut relay, &alice, &match_id, 0), None);
            }
            _ => {
                relay.open_matches(None, later(ABANDON_AFTER_SECONDS));
            }
        }
        assert!(
            relay
                .open_matches(None, later(ABANDON_AFTER_SECONDS))
                .is_empty(),
            "a {ending} match is not on offer"
        );
    }
}

/// Inactivity is measured from the last event, so playing keeps a match alive.
#[test]
fn an_event_resets_the_hour() {
    let (mut relay, match_id) = two_player_match();
    let bob = connection("bob");
    // Half an hour in, somebody acknowledges the round. That is an event, so the hour
    // starts again from there.
    relay.receive(
        &bob,
        Message::RoundOver {
            match_id: match_id.clone(),
        },
        later(ABANDON_AFTER_SECONDS / 2),
    );
    assert_eq!(
        relay.open_matches(None, later(ABANDON_AFTER_SECONDS)).len(),
        0,
        "a started match is not listed either way"
    );
    assert!(
        !relay.view(&match_id).over(),
        "but it is not abandoned, because it was touched half an hour ago"
    );
    relay.open_matches(
        None,
        later(ABANDON_AFTER_SECONDS + ABANDON_AFTER_SECONDS / 2),
    );
    assert_eq!(
        relay.view(&match_id).ending,
        Some(Ending::Abandoned),
        "and an hour after that it is"
    );
}

// ---------------------------------------------------------------------------------------
// Identity: /implementation/identity.md
// ---------------------------------------------------------------------------------------

/// A ticket for a Discord account, minted under the secret this relay holds.
fn ticket_for(id: &str, name: &str) -> Who {
    Who::Authenticated {
        ticket: integrity::ticket(
            &secret(),
            &protocol::account_text(&Account::Discord(id.to_string())),
            name,
            // Well after `moment()`, which is what every message in these tests arrives
            // at. Expiry has its own test; this is every other test's ticket.
            u64::MAX,
        ),
    }
}

/// Seat somebody, saying who they are however they say it.
fn seat<L: Log, C: Connections>(
    relay: &mut Relay<L, C>,
    at: &ConnectionId,
    match_id: &MatchId,
    who: Who,
    colour: &str,
) -> Option<Refused> {
    relay
        .receive(
            at,
            Message::Join {
                match_id: match_id.clone(),
                who,
                colour: colour.to_string(),
                version: PROTOCOL_VERSION,
            },
            moment(),
        )
        .refused
}

/// A ticket is the only way to be a Discord account, and the relay reads both out of it.
///
/// `An authenticated player is identified by their Discord account id` and `An
/// authenticated player's name is their Discord display name`: the client sends neither,
/// so neither can be claimed.
#[test]
fn a_ticket_seats_the_account_and_the_name_it_proves() {
    let mut relay = relay();
    let alice = connection("alice");
    let outcome = relay.receive(
        &alice,
        Message::Host {
            who: ticket_for("450853396908343307", "Ada Lovelace"),
            colour: "red".to_string(),
            version: PROTOCOL_VERSION,
            nonce: "once".to_string(),
        },
        moment(),
    );
    assert_eq!(outcome.refused, None);
    let match_id = outcome.created.expect("a match");
    let view = relay.view(&match_id);
    assert_eq!(
        view.seat_of_account(&discord_account("450853396908343307")),
        Some(Seat(0)),
        "the account came out of the ticket"
    );
    assert_eq!(view.name_of(Seat(0)), "Ada Lovelace");
    assert!(view.ranked(), "one authenticated seat and nothing else");
}

/// A guest id is never checked, asserted directly.
///
/// An accidental check would be a feature nobody asked for that fails closed in
/// production, so this is worth a test of its own rather than being left to follow from
/// the absence of code.
#[test]
fn a_guest_id_is_taken_as_given() {
    let mut relay = relay();
    for id in [
        "",
        " ",
        "discord:450853396908343307",
        "../../etc",
        "\u{1f3b2}",
    ] {
        let outcome = relay.receive(
            &connection(id),
            Message::Host {
                who: Who::Guest {
                    id: id.to_string(),
                    name: "Anybody".to_string(),
                },
                colour: "red".to_string(),
                version: PROTOCOL_VERSION,
                nonce: format!("nonce-{id}"),
            },
            moment(),
        );
        assert_eq!(outcome.refused, None, "{id:?} was checked");
        let match_id = outcome.created.expect("a match");
        assert_eq!(
            relay.view(&match_id).seat_of_account(&guest(id)),
            Some(Seat(0)),
            "{id:?} is the account, verbatim"
        );
    }
}

/// A guest id that reads like a Discord account is still a guest.
///
/// The one case where taking a guest id verbatim could have gone wrong: `Account::Guest`
/// and `Account::Discord` are different variants, so a guest saying `discord:1` is
/// `guest:discord:1` and shares nothing with the real account.
#[test]
fn a_guest_cannot_spell_its_way_into_a_discord_account() {
    let mut relay = relay();
    let impostor = relay
        .receive(
            &connection("impostor"),
            Message::Host {
                who: Who::Guest {
                    id: "discord:450853396908343307".to_string(),
                    name: "Ada Lovelace".to_string(),
                },
                colour: "red".to_string(),
                version: PROTOCOL_VERSION,
                nonce: "impostor".to_string(),
            },
            moment(),
        )
        .created
        .expect("a match");
    let view = relay.view(&impostor);
    assert_eq!(
        view.seat_of_account(&discord_account("450853396908343307")),
        None,
        "spelling a prefix is not holding an account"
    );
    assert!(!view.ranked(), "and it certainly is not ranked");
}

/// A ticket that does not verify is refused, and the seat is not taken.
#[test]
fn a_ticket_that_proves_nothing_seats_nobody() {
    let mut relay = relay();
    let alice = connection("alice");
    let match_id = host(&mut relay, &alice);
    let before = relay.view(&match_id).seats.len();

    let forgeries = [
        ("nonsense", "not a ticket".to_string()),
        ("empty", String::new()),
        (
            "another secret",
            integrity::ticket(
                &Secret::from_bytes([99; 32]),
                "discord:450853396908343307",
                "Ada",
                u64::MAX,
            ),
        ),
        (
            "expired",
            integrity::ticket(
                &secret(),
                "discord:450853396908343307",
                "Ada",
                // A second before every message in these tests arrives.
                moment().to_unix_nanos() / 1_000_000_000 - 1,
            ),
        ),
        (
            // Nothing mints one of these, and if anything ever did it must not count as
            // proven — which is what the guard in `identify` is for.
            "a ticket naming a guest",
            integrity::ticket(&secret(), "guest:1", "Ada", u64::MAX),
        ),
    ];
    for (what, ticket) in forgeries {
        assert_eq!(
            seat(
                &mut relay,
                &connection(what),
                &match_id,
                Who::Authenticated { ticket },
                "blue"
            ),
            Some(Refused::BadTicket),
            "{what} was accepted"
        );
    }
    assert_eq!(
        relay.view(&match_id).seats.len(),
        before,
        "and none of them took a seat"
    );
}

/// `A match is ranked when every seat in it is held by an authenticated player`, over
/// every combination of seat kinds up to a small width.
///
/// All authenticated is ranked; one guest anywhere is not; an empty match is not. Over
/// combinations rather than examples, because the property is about *every* seat and a
/// test over two or three cases would pass for an implementation that looked at the first.
#[test]
fn ranked_is_every_seat_and_not_most_of_them() {
    let colours = ["red", "blue", "green", "white"];
    for width in 1..4usize {
        for shape in 0..(1u32 << width) {
            let proven: Vec<bool> = (0..width).map(|at| shape >> at & 1 == 1).collect();
            let mut relay = relay();
            let mut match_id = None;
            for (at, &is_proven) in proven.iter().enumerate() {
                let who = if is_proven {
                    ticket_for(&format!("4508533969083433{at:02}"), &format!("Proven {at}"))
                } else {
                    guest_who(&format!("guest-{at}"), &format!("Guest {at}"))
                };
                let socket = connection(&format!("socket-{at}"));
                match &match_id {
                    None => {
                        let outcome = relay.receive(
                            &socket,
                            Message::Host {
                                who,
                                colour: colours[at].to_string(),
                                version: PROTOCOL_VERSION,
                                nonce: "ranked".to_string(),
                            },
                            moment(),
                        );
                        assert_eq!(outcome.refused, None);
                        match_id = outcome.created;
                    }
                    Some(id) => {
                        assert_eq!(seat(&mut relay, &socket, id, who, colours[at]), None);
                    }
                }
            }
            let match_id = match_id.expect("a match");
            let view = relay.view(&match_id);
            assert_eq!(
                view.ranked(),
                proven.iter().all(|proven| *proven),
                "{proven:?} was judged {}",
                view.ranked()
            );
        }
    }
    // And a log with no seats in it at all.
    let relay = relay();
    assert!(
        !relay.view(&MatchId("nothing".to_string())).ranked(),
        "a match nobody played is not a ranked match"
    );
}

/// `Whether a match is ranked is settled when it starts` — leaving afterwards does not
/// change it.
///
/// Falls out rather than being enforced: `A player cannot join a match that has started`
/// fixes the seats, and a seat that left is still a seat in the log.
#[test]
fn leaving_a_started_match_does_not_change_whether_it_is_ranked() {
    let mut relay = relay();
    let alice = connection("alice");
    let bob = connection("bob");
    let match_id = relay
        .receive(
            &alice,
            Message::Host {
                who: ticket_for("1", "Ada"),
                colour: "red".to_string(),
                version: PROTOCOL_VERSION,
                nonce: "ranked".to_string(),
            },
            moment(),
        )
        .created
        .expect("a match");
    assert_eq!(
        seat(&mut relay, &bob, &match_id, ticket_for("2", "Bob"), "blue"),
        None
    );
    relay.receive(
        &alice,
        Message::Start {
            match_id: match_id.clone(),
        },
        moment(),
    );
    assert!(relay.view(&match_id).ranked());

    relay.receive(
        &bob,
        Message::Leave {
            match_id: match_id.clone(),
        },
        moment(),
    );
    assert!(
        relay.view(&match_id).ranked(),
        "a seat that left is still a seat in the log"
    );
}

/// `Listed` says whether a match is ranked, from the same fold as the host and the count.
#[test]
fn a_listing_says_which_matches_are_ranked() {
    let mut relay = relay();
    let proven = relay
        .receive(
            &connection("ada"),
            Message::Host {
                who: ticket_for("1", "Ada"),
                colour: "red".to_string(),
                version: PROTOCOL_VERSION,
                nonce: "proven".to_string(),
            },
            moment(),
        )
        .created
        .expect("a match");
    let unproven = relay
        .receive(
            &connection("sean"),
            Message::Host {
                who: guest_who("sean", "Sean"),
                colour: "blue".to_string(),
                version: PROTOCOL_VERSION,
                nonce: "unproven".to_string(),
            },
            moment(),
        )
        .created
        .expect("a match");

    let listed = relay.open_matches(None, moment());
    let ranked_of = |id: &MatchId| {
        listed
            .iter()
            .find(|open| &open.match_id == id)
            .map(|open| open.ranked)
    };
    assert_eq!(ranked_of(&proven), Some(true));
    assert_eq!(ranked_of(&unproven), Some(false));
}
