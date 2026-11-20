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

/// A match, folded from its log.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MatchView {
    pub version: u32,
    pub seats: Vec<SeatInfo>,
    pub started: bool,
    /// Which round the match is in, counting from zero. A reveal ends one.
    pub round: u32,
    host: Option<Seat>,
    barred: Vec<Seat>,
    acknowledged: Vec<Seat>,
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
            host: None,
            barred: Vec::new(),
            acknowledged: Vec::new(),
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
                Event::Reveal { .. } => {
                    view.round += 1;
                    view.acknowledged.clear();
                }
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
