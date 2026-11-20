//! A hosted match with AI players in it, played out in one process.
//!
//! `The host can add an AI player to a hosted match` and `An AI player's turn is taken by
//! the client that added it`. Realizes *AI players take guest accounts* in
//! /implementation/identity.md.
//!
//! The property worth proving is a negative one: nothing in the relay, the protocol, the
//! log or another client branches on a seat being a bot. There is no `if virtual` anywhere
//! to drift out of fidelity, which is the only basis on which a bot playing says anything
//! about a person playing. So this test does not check that bots work — it checks that a
//! match with bots in it is the same match, built out of the same pieces, as one without.
//!
//! **One socket per player, bot or not.** The document said the host's client would run one
//! `Seated` per bot *over the same socket*, and that is wrong: `Connections::attach` evicts
//! a connection from wherever else it was, because a socket is in one place — so the second
//! seat on a socket takes the first's connection away and the match halts at the host's own
//! turn. A socket each keeps the invariant that decides who is sent whose dice, and costs
//! nothing but sockets.

use integrity::Secret;
use protocol::{Moment, Who};
use relay::memory::{MemoryConnections, MemoryLog};
use relay::{ConnectionId, Relay};
use seat::{Ai, Seated};
use wire::answer::Answer;
use wire::{answer, json, message};

/// One fixed reading for every message, so nothing expires mid-match.
fn moment() -> Moment {
    Moment::from_unix_nanos(1_700_000_000_000_000_000)
}

/// The relay, and the frames it answers with. One invocation's worth of work per call.
struct Wire {
    relay: Relay<MemoryLog, MemoryConnections>,
}

impl Wire {
    fn new() -> Self {
        Self {
            relay: Relay::new(
                MemoryLog::new(),
                MemoryConnections::new(),
                Secret::from_bytes([23; 32]),
            ),
        }
    }

    fn send(&mut self, from: &ConnectionId, frame: &str) -> Vec<(ConnectionId, String)> {
        let shape = json::read(frame).expect("a readable frame");
        let message = message::from_shape(&shape).expect("a known message");
        let outcome = self.relay.receive(from, message, moment());
        let mut out = Vec::new();
        for (to, event) in outcome.sends {
            out.push((to, frame_of(&Answer::Event(event))));
        }
        if let Some(created) = outcome.created {
            out.push((from.clone(), frame_of(&Answer::Created(created))));
        }
        assert_eq!(outcome.refused, None, "the relay refused a message");
        out
    }
}

fn frame_of(answer: &Answer) -> String {
    json::write(&answer::shape_of(answer))
}

/// A guest, which is what every player in a hosted match is until they authenticate — and
/// what every AI player is forever, because it never authenticates.
fn guest(id: &str, name: &str) -> Who {
    Who::Guest {
        id: id.to_string(),
        name: name.to_string(),
    }
}

/// One page's worth of state: a person's seat, and a seat per bot it added.
struct Page {
    seats: Vec<(ConnectionId, Seated)>,
}

#[test]
fn a_host_plays_a_whole_match_against_two_bots_it_added() {
    let mut wire = Wire::new();

    // The person, and two bots the host added. Each gets its own connection, which is the
    // correction this test exists for: a socket holds one seat.
    let host_socket = ConnectionId("page-host".to_string());
    let host = Seated::hosting(guest("host-id", "Sean"), "red", "bots", Box::new(Ai))
        .expect("a guest is always a seat");

    let mut page = Page {
        seats: vec![(host_socket.clone(), host)],
    };
    let mut pending: Vec<(ConnectionId, String)> =
        vec![(host_socket.clone(), page.seats[0].1.opening())];
    let mut added = false;
    let mut started = false;
    let mut guard = 0;

    while page.seats.iter().any(|(_, seat)| !seat.over()) || !pending.is_empty() {
        guard += 1;
        assert!(guard < 40_000, "the match would not settle");

        let mut answers: Vec<(ConnectionId, String)> = Vec::new();
        for (from, frame) in pending.drain(..) {
            answers.extend(wire.send(&from, &frame));
        }

        // The bots join as soon as the host knows which match they are joining. A bot is
        // added by the host's client, so this is the only moment anything about it is
        // special — and what it does is build an ordinary joiner.
        if !added && let Some(match_id) = page.seats[0].1.match_id().cloned() {
            for (at, colour) in ["blue", "green"].iter().enumerate() {
                let socket = ConnectionId(format!("page-bot-{at}"));
                let bot = Seated::joining(
                    // A guest id the host's client generated, and a name it chose. The
                    // relay cannot tell this from a person, which is the point.
                    guest(&format!("bot-{at}"), &format!("Ada {at}")),
                    *colour,
                    match_id.clone(),
                    Box::new(Ai),
                )
                .expect("a guest is always a seat");
                pending.push((socket.clone(), bot.opening()));
                page.seats.push((socket, bot));
            }
            added = true;
        }

        for (to, frame) in answers {
            let Some((_, seat)) = page.seats.iter_mut().find(|(at, _)| at == &to) else {
                panic!("a frame for {to:?} with nobody there");
            };
            let answered = seat.receive(&frame).expect("a seat kept playing");
            pending.extend(answered.into_iter().map(|frame| (to.clone(), frame)));
        }

        // Three seats taken, so the host starts it.
        if !started
            && page.seats[0].1.seated() >= 3
            && let Some(start) = page.seats[0].1.start()
        {
            pending.push((host_socket.clone(), start));
            started = true;
        }

        // Every seat is offered its turn each pass, which is what a frame loop does. The
        // host's page drives all three the same way; `Ai` answers at once and a person
        // would not, and nothing else differs.
        if pending.is_empty() && started {
            for (socket, seat) in page.seats.iter_mut() {
                let asked = seat.offer().expect("a seat kept playing");
                pending.extend(asked.into_iter().map(|frame| (socket.clone(), frame)));
            }
            if pending.is_empty() {
                break;
            }
        }
    }

    assert!(started, "the match never started");
    assert_eq!(page.seats.len(), 3, "a host and two bots");
    for (socket, seat) in &page.seats {
        assert!(seat.over(), "{socket:?} never saw the end");
        assert_eq!(seat.complaints(), &[], "{socket:?} saw something wrong");
    }

    let match_id = page.seats[0].1.match_id().expect("a match").clone();
    let view = wire.relay.view(&match_id);

    // The log says nothing about any of them being a bot. There is no field for it,
    // which is what stops anything reading one.
    assert_eq!(view.seats.len(), 3);
    assert!(
        view.seats
            .iter()
            .all(|seat| matches!(seat.account, protocol::Account::Guest(_))),
        "a bot is a guest like anybody who has not authenticated: {:?}",
        view.seats
    );

    // And a bot never makes a match ranked, because it never authenticates. Nothing
    // enforces that; it follows from a bot holding a guest account.
    assert!(!view.ranked(), "a match with bots in it is not ranked");

    // The whole match is in the log, reached by three clients that between them sent every
    // claim. A seat per socket is what made that possible.
    assert_eq!(
        view.ending,
        Some(relay::Ending::Complete),
        "every present seat said the match was over"
    );
}
