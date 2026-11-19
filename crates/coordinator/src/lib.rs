//! A client: everything between the wire and a decision.
//!
//! Realizes /implementation/coordinator.md.
//!
//! Events in, intents out, and a [`rules::View`] in between. This owns the only path from
//! a message to a client's state — nothing else may build or change a client's game — so
//! whatever is driving the seat sees exactly what a person at a keyboard would see.
//!
//! That is the whole point of the arrangement. An **account** says who a seat belongs to
//! and is in the log for everyone to read; a **driver** chooses the next call and is
//! visible to nobody. A virtual player is a driver, never an account, so nothing in the
//! relay, the protocol, the log or another client branches on it. There is no `if virtual`
//! to drift out of fidelity, which is the only basis on which a virtual player working
//! says anything about a real one.
//!
//! Depends on `rules`, `protocol` and `integrity`, and not on `relay`: a client speaks the
//! protocol, it does not share a library with the service that routes it.

use integrity::{Commitment, Seed};
use protocol::{Event, Seat};
use rules::{Call, Claimed, Game, Public, Resolution, SEAT, Variants, View};

/// What a client wants sent. The composition root addresses it; this crate does not know
/// what a connection is.
#[derive(Clone, PartialEq, Debug)]
pub enum Intent {
    /// A call to be ordered and fanned out. Applied to this client only when it comes
    /// back, so every client applies the same ordered stream.
    Claim(Event),
    /// This seat has seen the round end.
    RoundOver,
}

/// Something arrived that should not have.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Wrong {
    /// A revealed seed is not the one the round committed to.
    SeedDoesNotMatchCommitment,
    /// A deal disagrees with the seed it was supposedly dealt from.
    DiceDisagreeWithTheSeed { seat: Seat },
}

/// What chooses the next call.
///
/// The one thing that differs between a person, an AI opponent and a test fixture. It is
/// handed a [`View`] and nothing else, so it cannot reach another seat's dice even when
/// the harness around it knows them.
pub trait Decides {
    /// `None` when no choice has been made yet, which is a human's usual answer.
    fn choose(&mut self, view: &View) -> Option<Call>;
}

/// One client of one match.
pub struct Client {
    seat: Seat,
    order: Vec<usize>,
    variants: Variants,
    game: Option<Game>,
    commitment: Option<Commitment>,
    /// Every event this client was sent, in order.
    ///
    /// What this seat would have seen and nothing else, which is a weaker thing than the
    /// relay's log: the relay holds a `Deal` for every seat, and a client is only sent its
    /// own until the round is revealed. A screen that lists events must read this one, or
    /// it shows the holder of the screen every other seat's dice.
    received: Vec<Event>,
    /// What ended the round being shown: the seat that called, and what it called.
    ///
    /// `Public` records raises, because a raise is a claim, but not the liar or spot on
    /// that disputes one. This is how a screen says who ended the round and how, without
    /// going back to a log to re-derive something the client already knew. Cleared when
    /// the next round begins.
    ended_by: Option<(Seat, Call)>,
    /// Whether this round's seed has arrived.
    ///
    /// Not the same as `Game::revealed`, which only turns true when the round is resolved
    /// — and resolving happens after the whole delivery has finished, by which time the
    /// next round's deals have already come and gone. This turns true the moment the seed
    /// lands, which is the point after which a deal belongs to the next round.
    revealed: bool,
    /// Dice dealt for the next round, waiting for this one to be left behind.
    ///
    /// The relay reveals a round and opens the next one in the same breath
    /// (`crates/relay/src/lib.rs`, `round_over`), so the next round's commitment and deals
    /// arrive while this client is still showing the round that just ended. Installing one
    /// then is refused, because that seat is already dealt for the round in progress, and
    /// `Game::next_round` forgets every die — so a deal that arrived early would be lost
    /// between the two, and the new round would begin with nobody holding anything.
    next_deal: Vec<(Seat, [u8; SEAT])>,
    /// Problems found while applying events, kept rather than thrown away so a caller can
    /// report that the log disagrees with itself.
    complaints: Vec<Wrong>,
}

impl Client {
    pub fn new(seat: Seat) -> Self {
        Self {
            seat,
            order: Vec::new(),
            variants: Variants::default(),
            game: None,
            commitment: None,
            received: Vec::new(),
            ended_by: None,
            revealed: false,
            next_deal: Vec::new(),
            complaints: Vec::new(),
        }
    }

    pub fn seat(&self) -> Seat {
        self.seat
    }

    /// The match, once it has started.
    pub fn game(&self) -> Option<&Game> {
        self.game.as_ref()
    }

    pub fn public(&self) -> Option<&Public> {
        self.game.as_ref().map(Game::public)
    }

    /// This seat's view. `None` before the match starts.
    pub fn view(&self) -> Option<View<'_>> {
        self.game
            .as_ref()
            .map(|game| game.view(self.seat.0 as usize))
    }

    pub fn is_my_turn(&self) -> bool {
        self.view().is_some_and(|view| view.is_my_turn())
    }

    pub fn complaints(&self) -> &[Wrong] {
        &self.complaints
    }

    /// Everything this seat has been sent, in order.
    pub fn stream(&self) -> &[Event] {
        &self.received
    }

    /// Who ended the round being shown, and how. `None` while a round is still running.
    pub fn ended_by(&self) -> Option<(Seat, Call)> {
        self.ended_by
    }

    /// Offer a call. Refused rather than sent if it is not legal, which is
    /// `A client does not send an illegal claim` enforced where a call originates.
    ///
    /// The game is not advanced here. It advances when the call comes back from the
    /// relay, so this client applies the same ordered stream as every other.
    pub fn act(&mut self, call: Call, odds: f64) -> Result<Intent, rules::Rejected> {
        let game = self.game.as_ref().ok_or(rules::Rejected::MatchIsOver)?;
        game.public().check(call)?;
        let seat = self.seat;
        Ok(Intent::Claim(match call {
            Call::Raise(claim) => Event::Claim {
                seat,
                quantity: claim.quantity,
                face: claim.face,
                odds,
            },
            Call::Liar => Event::Liar { seat, odds },
            Call::SpotOn => Event::SpotOn { seat, odds },
        }))
    }

    /// Take one event from the relay.
    pub fn receive(&mut self, event: &Event) -> Vec<Intent> {
        self.received.push(event.clone());
        match event {
            Event::PlayerOrder(order) => {
                self.order = order.iter().map(|seat| seat.0 as usize).collect();
                Vec::new()
            }
            Event::Variant { name, on } => {
                if *name == "ones-wild" {
                    self.variants.ones_wild = *on;
                }
                Vec::new()
            }
            Event::Start => {
                // The order arrives before the start, so a match begins knowing it.
                self.game = Some(Game::start(self.order.clone(), self.variants));
                Vec::new()
            }
            Event::Commit { digest } => {
                self.commitment = Commitment::from_hex(digest);
                Vec::new()
            }
            Event::Deal { seat, dice } => {
                self.take_deal(*seat, dice);
                Vec::new()
            }
            Event::Reveal { seed } => self.take_reveal(seed),
            Event::Claim {
                seat,
                quantity,
                face,
                ..
            } => self.take_call(
                *seat,
                Call::Raise(rules::Claim {
                    quantity: *quantity,
                    face: *face,
                }),
            ),
            Event::Liar { seat, .. } => self.take_call(*seat, Call::Liar),
            Event::SpotOn { seat, .. } => self.take_call(*seat, Call::SpotOn),
            // Joins, bars, acknowledgements and the rest carry nothing this client's
            // state machine turns on.
            _ => Vec::new(),
        }
    }

    fn take_deal(&mut self, seat: Seat, dice: &[u8]) {
        let revealed = self.revealed;
        let Some(game) = self.game.as_mut() else {
            return;
        };
        let Ok(dealt) = <[u8; SEAT]>::try_from(dice) else {
            return;
        };
        // Once this round's seed has arrived the round can take no more dice, so a deal
        // after that belongs to the next round and waits for it. The reveal's own deals
        // arrive before the `Reveal` that carries the seed, so they still land here.
        if revealed {
            self.next_deal.push((seat, dealt));
            return;
        }
        // A seat already dealt is ignored rather than overwritten: a late or duplicated
        // deal must not rewrite a round.
        let _ = game.install(seat.0 as usize, dealt);
    }

    /// The round is over. Check what was dealt against the seed it came from, then settle.
    ///
    /// This is `A player can detect dice that disagree with the event log` happening live
    /// rather than only in an audit afterwards.
    fn take_reveal(&mut self, seed: &str) -> Vec<Intent> {
        let Some(seed) = Seed::from_hex(seed) else {
            return Vec::new();
        };
        self.revealed = true;
        if let Some(commitment) = self.commitment
            && !integrity::verify(&commitment, &seed)
        {
            self.complaints.push(Wrong::SeedDoesNotMatchCommitment);
        }
        let Some(game) = self.game.as_mut() else {
            return Vec::new();
        };
        for seat in 0..game.public().players() {
            let derived = integrity::deal(&seed, seat as u8, SEAT);
            match game.dealt(seat) {
                Some(dealt) if dealt.as_slice() != derived.as_slice() => {
                    self.complaints.push(Wrong::DiceDisagreeWithTheSeed {
                        seat: Seat(seat as u8),
                    });
                }
                Some(_) => {}
                None => {
                    let _ = game.install(
                        seat,
                        <[u8; SEAT]>::try_from(derived.as_slice()).expect("the dice limit"),
                    );
                }
            }
        }
        Vec::new()
    }

    fn take_call(&mut self, seat: Seat, call: Call) -> Vec<Intent> {
        let Some(game) = self.game.as_mut() else {
            return Vec::new();
        };
        // A claim from a seat whose turn it is not is an illegal claim, and a client
        // ignores one it receives. The relay recorded the attempt; nothing else is owed.
        if game.public().current() != seat.0 as usize {
            return Vec::new();
        }
        match game.claim(call) {
            Ok(Claimed::Continues) => Vec::new(),
            // Seen the round end, which is all the acknowledgement means. The dice that
            // settle it have not arrived yet.
            Ok(Claimed::RoundEnded) => {
                self.ended_by = Some((seat, call));
                vec![Intent::RoundOver]
            }
            Err(_) => Vec::new(),
        }
    }

    /// Settle the round if it can be settled, and begin the next.
    ///
    /// Separate from `receive` because a caller usually wants to show the resolution
    /// before the next round replaces it.
    pub fn settle(&mut self) -> Option<Resolution> {
        let game = self.game.as_mut()?;
        let resolution = game.resolve()?;
        Some(resolution)
    }

    /// Begin the next round after a resolution has been seen.
    pub fn begin_next_round(&mut self, opens: usize) {
        if let Some(game) = self.game.as_mut() {
            game.next_round(opens);
            // Whatever was dealt while the last round was still being shown.
            for (seat, dealt) in self.next_deal.drain(..) {
                let _ = game.install(seat.0 as usize, dealt);
            }
        }
        // The commitment is not cleared. The next round's arrives with its deals, before
        // this is called, so clearing it here would throw away the thing the next reveal
        // is checked against — and every reveal after the first would go unverified.
        self.revealed = false;
        self.ended_by = None;
    }
}
