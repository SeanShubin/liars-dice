//! Two seats play a whole match through the wire format, with no socket and no AWS.
//!
//! The relay is in this process, but nothing reaches it except JSON frames — so every
//! part the deployed arrangement uses is exercised here except the socket itself:
//! `message::shape_of`, `json::write`, `json::read`, `answer::from_shape`, the seat
//! finding out which seat it is, the turn taking, and the round settling.
//!
//! `apps/probe` is the same two seats with a real WebSocket between them. What it adds
//! is the transport; what it cannot add is certainty, because a socket introduces timing.
//! This is where the logic is proven.

use integrity::Secret;
use protocol::{Account, Event};
use relay::memory::{MemoryConnections, MemoryLog};
use relay::{ConnectionId, Relay};
use seat::{Ai, Seated};
use wire::answer::Answer;
use wire::{answer, json, message};

/// The relay, reached only through frames.
struct Wire {
    relay: Relay<MemoryLog, MemoryConnections>,
}

impl Wire {
    fn new() -> Self {
        Self {
            relay: Relay::new(
                MemoryLog::new(),
                MemoryConnections::new(),
                Secret::from_bytes([19; 32]),
            ),
        }
    }

    /// One frame in, every frame the relay wants sent out. This is what `apps/relay`'s
    /// `$default` handler does, minus the posting.
    fn send(&mut self, from: &ConnectionId, frame: &str) -> Vec<(ConnectionId, String)> {
        let shape = json::read(frame).expect("a client sent a readable frame");
        let message = message::from_shape(&shape).expect("a client sent a known message");
        let outcome = self.relay.receive(from, message);
        let mut out = Vec::new();
        for (to, event) in outcome.sends {
            out.push((to, frame_of(&Answer::Event(event))));
        }
        if let Some(created) = outcome.created {
            out.push((from.clone(), frame_of(&Answer::Created(created))));
        }
        if let Some(refused) = outcome.refused {
            out.push((from.clone(), frame_of(&Answer::Refused(refused))));
        }
        out
    }
}

fn frame_of(answer: &Answer) -> String {
    json::write(&answer::shape_of(answer))
}

#[test]
fn two_seats_play_a_whole_match_through_the_wire() {
    let mut wire = Wire::new();
    let host_socket = ConnectionId("socket-host".to_string());
    let guest_socket = ConnectionId("socket-guest".to_string());

    let mut host = Seated::hosting(
        Account::Discord("host".to_string()),
        "Host",
        "red",
        "one",
        Box::new(Ai),
    );
    let mut guest_pending: Option<Seated> = None;

    // (which socket sent it, the frame)
    let mut pending: Vec<(ConnectionId, String)> = vec![(host_socket.clone(), host.opening())];
    let mut started = false;
    let mut guard = 0;

    while !host.over() {
        guard += 1;
        assert!(guard < 20_000, "the match would not settle");

        // Everything the clients have to say, handed to the relay.
        let mut answers: Vec<(ConnectionId, String)> = Vec::new();
        for (from, frame) in pending.drain(..) {
            answers.extend(wire.send(&from, &frame));
        }

        // The guest joins as soon as the host knows which match to join.
        if guest_pending.is_none()
            && let Some(match_id) = host.match_id()
        {
            let guest = Seated::joining(
                Account::Discord("guest".to_string()),
                "Guest",
                "blue",
                match_id.clone(),
                Box::new(Ai),
            );
            pending.push((guest_socket.clone(), guest.opening()));
            guest_pending = Some(guest);
        }

        // Everything the relay said, handed to whichever client it was addressed to.
        for (to, frame) in answers {
            let answered = if to == host_socket {
                host.receive(&frame).expect("the host kept playing")
            } else if let Some(guest) = guest_pending.as_mut() {
                guest.receive(&frame).expect("the guest kept playing")
            } else {
                panic!("a frame for {to:?} with nobody there")
            };
            let sender = to.clone();
            pending.extend(answered.into_iter().map(|frame| (sender.clone(), frame)));
        }

        // The host starts the match once both seats are taken.
        if !started
            && host.seated() >= 2
            && let Some(start) = host.start()
        {
            pending.push((host_socket.clone(), start));
            started = true;
        }

        if pending.is_empty() && started {
            break;
        }
    }

    let guest = guest_pending.expect("the guest joined");
    assert!(started, "the match never started");
    assert!(host.over(), "the match never ended for the host");
    assert!(guest.over(), "the match never ended for the guest");

    // The integrity scheme, verified from the event stream alone: every round's reveal
    // against its commitment, and every deal against the seed it came from. No secret
    // and no storage needed, which is what the specification means by verification
    // requiring nothing beyond the log.
    assert_eq!(host.complaints(), &[], "the host saw something wrong");
    assert_eq!(guest.complaints(), &[], "the guest saw something wrong");

    // Each seat learned which seat it was from the join naming its own account.
    assert_eq!(host.seat(), Some(protocol::Seat(0)));
    assert_eq!(guest.seat(), Some(protocol::Seat(1)));

    // A real match, not two events and a stall.
    assert!(
        host.stream().len() > 40,
        "only {} events: that is not a whole match",
        host.stream().len()
    );

    // Each round ends exactly once.
    //
    // This is the assertion that was missing, and its absence let a real defect through:
    // a seat kept re-proposing the call that ended the round every time the relay echoed
    // it back, and a log with two hundred spurious liars in it satisfies "more than forty
    // events" perfectly well. Counting the endings against the reveals pins the shape of
    // the match rather than its size.
    println!("host over: {} guest over: {}", host.over(), guest.over());
    let total = host.stream().len();
    for (at, event) in host
        .stream()
        .iter()
        .enumerate()
        .skip(total.saturating_sub(14))
    {
        println!("{at}: {event}");
    }
    println!("guest stream: {}", guest.stream().len());
    let endings = host
        .stream()
        .iter()
        .filter(|event| matches!(event, Event::Liar { .. } | Event::SpotOn { .. }))
        .count();
    let reveals = host
        .stream()
        .iter()
        .filter(|event| matches!(event, Event::Reveal { .. }))
        .count();
    assert_eq!(
        endings, reveals,
        "{endings} round endings against {reveals} reveals: a round ended more than once"
    );
    assert!(reveals > 0, "no round was ever revealed");
}
