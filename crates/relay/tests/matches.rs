//! A whole match, two relays, a disconnect and a return — in one process.
//!
//! These are the tests /implementation/relay.md asks for, and they are the reason the
//! relay is a library with two traits rather than a Lambda handler: multiplayer can be
//! proved before any infrastructure exists.

use integrity::Secret;
use protocol::{Account, Event, PROTOCOL_VERSION, Seat};
use relay::memory::{MemoryConnections, MemoryLog};
use relay::{ConnectionId, Connections, Log, MatchId, Message, Refused, Relay};

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

#[test]
fn a_claim_is_refused_while_a_seat_is_absent_and_accepted_once_it_returns() {
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
    assert_eq!(
        relay
            .receive(
                &alice,
                Message::Claim {
                    match_id: match_id.clone(),
                    event: claim.clone(),
                }
            )
            .refused,
        Some(Refused::SeatIsAbsent)
    );

    let back = connection("bob-again");
    relay.receive(
        &back,
        Message::Resume {
            match_id: match_id.clone(),
            account: discord("2"),
        },
    );
    assert_eq!(
        relay
            .receive(
                &alice,
                Message::Claim {
                    match_id: match_id.clone(),
                    event: claim,
                }
            )
            .refused,
        None
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
