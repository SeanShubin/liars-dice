//! A player whose socket died finds its own way back, with no id passed around.
//!
//! `A client returns its player to the match they hold a seat in, without asking`.
//! Realizes *Coming back to the match you are in* in /implementation/dashboard.md.
//!
//! This is the test that matters for a real session. API Gateway closes every WebSocket
//! after ten minutes of quiet and every one after two hours whatever happens, and neither
//! quota can be raised — so a drop is not an edge case, it is a certainty. Before this,
//! recovery meant the host re-sharing the match id, because a started match was not listed
//! and the page remembered nothing.

use coordinator::Decides;
use integrity::Secret;
use protocol::{Moment, Who};
use relay::memory::{MemoryConnections, MemoryLog};
use relay::{ConnectionId, Relay, Secrets};
use rules::{Call, View};
use seat::{Seated, dashboard};
use wire::answer::Answer;
use wire::{answer, json, message};

/// A driver that never acts.
///
/// `seat::Ai` answers the instant it is asked, so a table of them plays the whole match
/// out before anything else can happen — which is how the first version of this test came
/// to assert against a match that had already completed, eighty-four events deep. A real
/// match spends almost all of its life waiting for somebody, and waiting is the state a
/// drop has to be survivable from.
struct Waiting;

impl Decides for Waiting {
    fn choose(&mut self, _view: &View) -> Option<Call> {
        None
    }
}

fn moment() -> Moment {
    Moment::from_unix_nanos(1_700_000_000_000_000_000)
}

fn guest(id: &str) -> Who {
    Who::Guest { id: id.to_string() }
}

struct Wire {
    relay: Relay<MemoryLog, MemoryConnections>,
}

impl Wire {
    fn new() -> Self {
        Self {
            relay: Relay::new(
                MemoryLog::new(),
                MemoryConnections::new(),
                Secrets {
                    dice: Secret::from_bytes([29; 32]),
                    ticket: Some(Secret::from_bytes([29; 32])),
                },
            ),
        }
    }

    /// One frame in, every frame out — including the listing, which is an answer and not
    /// an event.
    fn send(&mut self, from: &ConnectionId, frame: &str) -> Vec<(ConnectionId, String)> {
        let shape = json::read(frame).expect("a readable frame");
        let message = message::from_shape(&shape).expect("a known message");
        let outcome = self.relay.receive(from, message, moment());
        assert_eq!(outcome.refused, None, "the relay refused {frame}");
        let mut out = Vec::new();
        for (to, event) in outcome.sends {
            out.push((to, json::write(&answer::shape_of(&Answer::Event(event)))));
        }
        if let Some(created) = outcome.created {
            out.push((
                from.clone(),
                json::write(&answer::shape_of(&Answer::Created(created))),
            ));
        }
        if let Some(listed) = outcome.listed {
            out.push((
                from.clone(),
                json::write(&answer::shape_of(&Answer::Listing(listed))),
            ));
        }
        out
    }
}

/// Deliver everything, letting each seat answer, until nothing is left to deliver.
fn settle(
    wire: &mut Wire,
    seats: &mut [(ConnectionId, Seated)],
    mut pending: Vec<(ConnectionId, String)>,
) {
    let mut guard = 0;
    while !pending.is_empty() {
        guard += 1;
        assert!(guard < 10_000, "the table would not settle");
        let mut answers = Vec::new();
        for (from, frame) in pending.drain(..) {
            answers.extend(wire.send(&from, &frame));
        }
        for (to, frame) in answers {
            if let Some((_, seat)) = seats.iter_mut().find(|(at, _)| at == &to) {
                let said = seat.receive(&frame).expect("a seat kept playing");
                pending.extend(said.into_iter().map(|frame| (to.clone(), frame)));
            }
        }
    }
}

#[test]
fn a_player_whose_socket_died_is_returned_to_their_own_match() {
    let mut wire = Wire::new();
    let host_socket = ConnectionId("host".to_string());
    let guest_socket = ConnectionId("guest".to_string());

    let host = Seated::hosting(guest("host-id"), "returning", "Sean", Box::new(Waiting))
        .expect("a guest is always a seat");
    let mut seats = vec![(host_socket.clone(), host)];
    let pending = vec![(host_socket.clone(), seats[0].1.opening())];
    settle(&mut wire, &mut seats, pending);
    let match_id = seats[0].1.match_id().expect("a match").clone();

    let joiner = Seated::joining(
        guest("guest-id"),
        match_id.clone(),
        "Ada",
        Box::new(Waiting),
    )
    .expect("a guest is always a seat");
    seats.push((guest_socket.clone(), joiner));
    let pending = vec![(guest_socket.clone(), seats[1].1.opening())];
    settle(&mut wire, &mut seats, pending);

    let start = seats[0].1.start().expect("the host can start");
    settle(&mut wire, &mut seats, vec![(host_socket.clone(), start)]);
    assert!(
        seats[1].1.view().is_some(),
        "the joiner was dealt in before anything went wrong"
    );

    // The socket dies. Not a leave — nothing is recorded, the seat is simply absent, which
    // is what a ten-minute lull or a two-hour cap does to every player in the match.
    wire.relay.disconnect(&guest_socket);
    seats.pop();

    // A new page, a new socket, the same guest id out of local storage. All it does is ask
    // what is going on, which is what the real page does as its opening frame.
    let returning = ConnectionId("guest-again".to_string());
    let listing = wire.send(&returning, &dashboard::ask(&guest("guest-id")));
    assert_eq!(listing.len(), 1, "one answer, the listing");
    let listed = dashboard::listing(&listing[0].1).expect("a listing");

    assert_eq!(
        listed.len(),
        1,
        "the match is listed, started and all: {listed:?}"
    );
    let mine: Vec<_> = listed.iter().filter(|open| open.mine).collect();
    assert_eq!(
        mine.len(),
        1,
        "exactly one match is this player's: {listed:?}"
    );
    assert_eq!(mine[0].match_id, match_id);
    assert!(
        mine[0].joinable,
        "a seat of your own is reachable, even in a match a stranger could not join"
    );

    // So the page resumes it, having asked nobody for the id.
    let mut back = Seated::resuming(
        // No name: coming back is not taking a seat, and the seat this account
        // holds already has the name its join recorded.
        guest("guest-id"),
        mine[0].match_id.clone(),
        Box::new(Waiting),
    )
    .expect("a guest is always a seat");
    let opening = back.opening();
    let replayed = wire.send(&returning, &opening);
    for (_, frame) in replayed {
        back.receive(&frame).expect("the replay was readable");
    }

    // And it is the same seat, with the same dice, in the same match.
    assert_eq!(back.seat(), Some(protocol::Seat(1)), "seat one, as before");
    let view = back.view().expect("dealt back in");
    assert!(
        view.own_dice().is_some_and(|dice| !dice.is_empty()),
        "a returning seat is dealt its own dice again, which is the whole of coming back"
    );
    assert_eq!(
        wire.relay
            .match_of(&protocol::Account::Guest("guest-id".to_string())),
        Some(match_id),
        "and the relay still says this is where they belong"
    );
}

/// Leaving on purpose is not undone by the next listing.
///
/// The page clears its own *came back* flag only when a socket is new, so a deliberate
/// leave is not followed by being put straight back into the match somebody just left.
/// Tested here from the relay's side: a seat that left is not marked as theirs.
#[test]
fn a_player_who_left_is_not_marked_as_belonging_anywhere() {
    let mut wire = Wire::new();
    let host_socket = ConnectionId("host".to_string());
    let host = Seated::hosting(guest("host-id"), "leaving", "Sean", Box::new(Waiting))
        .expect("a guest is always a seat");
    let mut seats = vec![(host_socket.clone(), host)];
    let pending = vec![(host_socket.clone(), seats[0].1.opening())];
    settle(&mut wire, &mut seats, pending);

    let leave = seats[0].1.leave().expect("a seat can leave");
    let pending = vec![(host_socket.clone(), leave)];
    settle(&mut wire, &mut seats, pending);

    let asking = ConnectionId("host-again".to_string());
    let listing = wire.send(&asking, &dashboard::ask(&guest("host-id")));
    let listed = dashboard::listing(&listing[0].1).expect("a listing");
    assert!(
        listed.iter().all(|open| !open.mine),
        "nothing belongs to somebody who left: {listed:?}"
    );
}

/// A colour change reaches every seat's own roster, which is what a lobby draws from.
///
/// `A lobby shows each seat's colour`, and a lobby draws from `Seated::roster`, which is
/// a client-side fold with a wildcard arm in it. So a colour change that the relay records
/// and sends to everybody would still show up nowhere until the fold was told to apply it.
/// The relay's tests cannot catch that: they read the log.
#[test]
fn a_colour_change_reaches_every_seats_roster() {
    let mut wire = Wire::new();
    let host_socket = ConnectionId("host".to_string());
    let guest_socket = ConnectionId("guest".to_string());

    let host = Seated::hosting(guest("host-id"), "colours", "Sean", Box::new(Waiting))
        .expect("a guest is always a seat");
    let mut seats = vec![(host_socket.clone(), host)];
    let pending = vec![(host_socket.clone(), seats[0].1.opening())];
    settle(&mut wire, &mut seats, pending);
    let match_id = seats[0].1.match_id().expect("a match").clone();

    let joiner = Seated::joining(guest("guest-id"), match_id, "Ada", Box::new(Waiting))
        .expect("a guest is always a seat");
    seats.push((guest_socket.clone(), joiner));
    let pending = vec![(guest_socket.clone(), seats[1].1.opening())];
    settle(&mut wire, &mut seats, pending);

    // The colours the relay gave, before anybody chose.
    assert_eq!(
        colours(&seats[0].1),
        vec!["white".to_string(), "black".to_string()],
        "the host's roster, as given"
    );

    let asked = seats[0]
        .1
        .recolour("teal")
        .expect("a seat in a match can ask");
    settle(&mut wire, &mut seats, vec![(host_socket.clone(), asked)]);

    // Both rosters, because the one that matters is the *other* seat's: a fold that
    // applied the change only where it was asked for would pass on the asker alone.
    for (at, seat) in seats.iter().enumerate() {
        assert_eq!(
            colours(&seat.1),
            vec!["teal".to_string(), "black".to_string()],
            "seat {at} should see the host in teal"
        );
    }
}

fn colours(seat: &Seated) -> Vec<String> {
    seat.roster()
        .into_iter()
        .map(|player| player.colour)
        .collect()
}

/// A roster carries the account and the username out of the join.
///
/// `A match shows which players have an account` and `An authenticated player's Discord
/// username is available on demand` are both drawn from `Seated::roster`, which is a
/// client-side fold that used to read a join's fields with `..`. A relay recording both
/// perfectly would still show neither until the fold was told to keep them, and the
/// relay's own tests cannot catch that: they read the log.
///
/// `Event::Colour` was dropped by this exact fold once already, which is why this test
/// exists before anybody has complained.
#[test]
fn a_roster_carries_the_account_and_the_username_from_the_join() {
    let mut wire = Wire::new();
    let host_socket = ConnectionId("host".to_string());
    let host = Seated::hosting(guest("host-id"), "accounts", "Sean", Box::new(Waiting))
        .expect("a guest is always a seat");
    let mut seats = vec![(host_socket.clone(), host)];
    let pending = vec![(host_socket.clone(), seats[0].1.opening())];
    settle(&mut wire, &mut seats, pending);

    let roster = seats[0].1.roster();
    assert_eq!(roster.len(), 1, "one seat so far");
    assert_eq!(
        roster[0].account,
        protocol::Account::Guest("host-id".to_string()),
        "the account came through the fold"
    );
    assert!(
        !roster[0].account.is_proven(),
        "a guest is not an authenticated player"
    );
    // A guest has none, which is the case a fold could satisfy by always answering None.
    // The relay tests cover the other side, where a ticket puts one on the join line.
    assert_eq!(roster[0].username, None);
}
