//! What the log says, folded.
//!
//! The relay caches nothing between messages: to answer anything it reads the log back
//! and folds it here. A match is a few hundred events, and this is what
//! /specification/architecture.md's "a relay may be stopped and replaced between any two
//! messages" actually requires.
//!
//! Nothing in this file knows a game rule. It counts seats, joins, bars, starts and
//! round-over acknowledgements, and it can see where a round boundary is because a reveal
//! marks one.

use protocol::{Account, Event, Seat};

/// One seat, as the join events describe it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SeatInfo {
    pub seat: Seat,
    pub name: String,
    pub colour: String,
    pub account: Account,
}

/// How a match stopped.
///
/// One field rather than three booleans, because the three are exclusive and a reader
/// should not be able to ask a question that has two answers. `A log says whether its
/// match is complete`, `A log says whether its match was cancelled` and `A log says
/// whether a cancelled match was cancelled by its host` are all one `match` on this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    /// Every seat present said so, and the relay recorded that they agreed.
    Complete,
    /// The host ended it, by the button or by leaving before the start.
    Cancelled { by: Seat },
    /// Nothing was recorded in it for an hour.
    Abandoned,
}

/// A match, folded from its log.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MatchView {
    pub version: u32,
    pub seats: Vec<SeatInfo>,
    pub started: bool,
    /// Which round the match is in, counting from zero. A reveal ends one.
    pub round: u32,
    /// How it stopped, or `None` while it is still going.
    pub ending: Option<Ending>,
    host: Option<Seat>,
    barred: Vec<Seat>,
    acknowledged: Vec<Seat>,
    /// Seats that have said the match is over. The relay counts these; it does not check
    /// them, because checking one would mean knowing a rule.
    finished: Vec<Seat>,
    /// Seats that have left for good. Membership, so it comes from the log; presence does
    /// not, and lives in the connection table.
    left: Vec<Seat>,
}

impl MatchView {
    pub fn fold(events: &[Event]) -> Self {
        let mut view = Self {
            version: 0,
            seats: Vec::new(),
            started: false,
            round: 0,
            ending: None,
            host: None,
            barred: Vec::new(),
            acknowledged: Vec::new(),
            finished: Vec::new(),
            left: Vec::new(),
        };
        for event in events {
            match event {
                Event::Version { protocol } => view.version = *protocol,
                Event::Join {
                    seat,
                    name,
                    colour,
                    account,
                } => view.seats.push(SeatInfo {
                    seat: *seat,
                    name: name.clone(),
                    colour: colour.clone(),
                    account: account.clone(),
                }),
                Event::Host { seat } => view.host = Some(*seat),
                Event::Bar { seat } => {
                    if !view.barred.contains(seat) {
                        view.barred.push(*seat);
                    }
                }
                Event::Lift { seat } => view.barred.retain(|barred| barred != seat),
                Event::Left { seat } => {
                    if !view.left.contains(seat) {
                        view.left.push(*seat);
                    }
                }
                Event::Start => view.started = true,
                Event::RoundOver { seat } => {
                    if !view.acknowledged.contains(seat) {
                        view.acknowledged.push(*seat);
                    }
                }
                Event::MatchOver { seat } => {
                    if !view.finished.contains(seat) {
                        view.finished.push(*seat);
                    }
                }
                Event::Reveal { .. } => {
                    view.round += 1;
                    view.acknowledged.clear();
                }
                // The three endings. First one wins, which is not a tie-break anybody has
                // to think about: `No event is recorded in a match that is over` means a
                // second one is never appended.
                Event::Complete => view.ending = view.ending.or(Some(Ending::Complete)),
                Event::Cancel { seat } => {
                    view.ending = view.ending.or(Some(Ending::Cancelled { by: *seat }));
                }
                Event::Abandon => view.ending = view.ending.or(Some(Ending::Abandoned)),
                _ => {}
            }
        }
        view
    }

    pub fn host(&self) -> Option<Seat> {
        self.host
    }

    /// How many seats have said the round is over. The relay reveals when this reaches
    /// the number of seats, which is a count of messages rather than a judgement about
    /// the round.
    pub fn round_over_count(&self) -> usize {
        self.acknowledged.len()
    }

    /// `A match is ranked when every seat in it is held by an authenticated player`.
    ///
    /// Derived, never recorded. Recording it would be a second copy of something the log
    /// already says, and it would have to be trusted; derived, anybody replaying the log
    /// reaches the same answer without trusting the relay at all — the same argument the
    /// commitment scheme rests on.
    ///
    /// `Whether a match is ranked is settled when it starts` falls out rather than being
    /// enforced: `A player cannot join a match that has started` fixes the seats at the
    /// start, so nothing afterwards can change the answer. Leaving does not, because a
    /// seat that left is still a seat in the log.
    ///
    /// An empty match is not ranked. A match with nobody in it is not a match anybody
    /// played, and `all` over nothing would say otherwise.
    pub fn ranked(&self) -> bool {
        !self.seats.is_empty() && self.seats.iter().all(|seat| seat.account.is_proven())
    }

    /// Whether the match has stopped, however it stopped.
    pub fn over(&self) -> bool {
        self.ending.is_some()
    }

    /// Whether this seat has said the match is over.
    pub fn has_finished(&self, seat: Seat) -> bool {
        self.finished.contains(&seat)
    }

    /// Whether this seat has left the match for good.
    pub fn has_left(&self, seat: Seat) -> bool {
        self.left.contains(&seat)
    }

    /// The seats taking part: joined, and not departed.
    ///
    /// Nothing to do with the rules and nothing to do with dice — a seat with no dice is
    /// still taking part until it says otherwise. This is the coordination layer's answer
    /// to who is in the match, which is the only answer the relay is entitled to have.
    pub fn participants(&self) -> Vec<Seat> {
        self.seats
            .iter()
            .map(|info| info.seat)
            .filter(|seat| !self.has_left(*seat))
            .collect()
    }

    pub fn is_barred(&self, account: &Account) -> bool {
        match self.seat_of_account(account) {
            Some(seat) => self.barred.contains(&seat),
            None => false,
        }
    }

    pub fn seat_of_account(&self, account: &Account) -> Option<Seat> {
        self.seats
            .iter()
            .find(|seat| &seat.account == account)
            .map(|seat| seat.seat)
    }

    pub fn name_of(&self, seat: Seat) -> &str {
        self.seats
            .iter()
            .find(|info| info.seat == seat)
            .map(|info| info.name.as_str())
            .unwrap_or("")
    }
}

/// Which rounds have been revealed, by position in the log.
///
/// A reveal ends the round it closes, so everything dealt up to it becomes public. This
/// is how a replay knows which deal lines a reconnecting client may be shown.
pub fn revealed_rounds(events: &[Event]) -> Vec<u32> {
    let mut revealed = Vec::new();
    let mut round = 0;
    for event in events {
        if matches!(event, Event::Reveal { .. }) {
            revealed.push(round);
            round += 1;
        }
    }
    revealed
}
