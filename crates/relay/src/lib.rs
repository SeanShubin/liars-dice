//! The relay: orders messages, deals rounds, and fans them out.
//!
//! Realizes /implementation/relay.md.
//!
//! Knows no game rules, and the build says so: this crate does not depend on `rules` and
//! must not. It never decides whether a claim was true, who loses a die, who is
//! eliminated, or whose turn it is. If a change here needs `rules`, the design has gone
//! wrong — stop and re-read the document.
//!
//! Transport-agnostic. [`Log`] and [`Connections`] are the whole seam: wire them to
//! DynamoDB for production, or to [`memory`] to play a whole match in one process.

use integrity::Secret;
use protocol::{Account, Event, PROTOCOL_VERSION, Seat};

pub mod memory;
mod view;

pub use view::MatchView;

/// A match's identity. Also the key its log is stored under.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MatchId(pub String);

/// One client's transport connection. Assigned by the transport, never by the log.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ConnectionId(pub String);

/// What a connection is attached to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Connected {
    pub connection: ConnectionId,
    pub seat: Seat,
    pub role: Role,
}

/// Why a connection is here. Spectators are deferred, and this is the row that will carry
/// them when they arrive, so adding them changes no storage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Player,
}

/// Something was already written at that position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Taken;

/// The append-only event log. The only state.
pub trait Log {
    /// Append every event at consecutive positions from `from`, or append none of them.
    ///
    /// The conditional write is what gives the log a total order without a lock, and the
    /// position is a version number: `from` is the length of the log the caller decided
    /// against, so `Taken` means the log moved and the decision is stale. The caller
    /// re-reads and redoes the whole message rather than moving its events along, which
    /// would commit a decision that was never true.
    ///
    /// All or nothing, because a message is several events — hosting is three and
    /// starting is a dozen — and `A message either happens entirely or not at all`.
    fn append(&mut self, match_id: &MatchId, from: u64, events: &[Event]) -> Result<(), Taken>;

    fn read(&self, match_id: &MatchId) -> Vec<Event>;

    /// Every match the log holds, for the lobby list.
    fn matches(&self) -> Vec<MatchId>;
}

/// Which socket is which seat. The one thing that is neither derived nor in the log.
pub trait Connections {
    fn of(&self, match_id: &MatchId) -> Vec<Connected>;
    fn attach(&mut self, match_id: &MatchId, connection: ConnectionId, seat: Seat, role: Role);
    fn detach(&mut self, connection: &ConnectionId);
}

/// Storage outlives any one relay, so a borrow of it is storage too. This is what lets a
/// replacement relay be constructed over the same log and connection table, which is how
/// "a relay may be stopped and replaced between any two messages" is tested rather than
/// asserted.
impl<T: Log + ?Sized> Log for &mut T {
    fn append(&mut self, match_id: &MatchId, from: u64, events: &[Event]) -> Result<(), Taken> {
        (**self).append(match_id, from, events)
    }

    fn read(&self, match_id: &MatchId) -> Vec<Event> {
        (**self).read(match_id)
    }

    fn matches(&self) -> Vec<MatchId> {
        (**self).matches()
    }
}

impl<T: Connections + ?Sized> Connections for &mut T {
    fn of(&self, match_id: &MatchId) -> Vec<Connected> {
        (**self).of(match_id)
    }

    fn attach(&mut self, match_id: &MatchId, connection: ConnectionId, seat: Seat, role: Role) {
        (**self).attach(match_id, connection, seat, role)
    }

    fn detach(&mut self, connection: &ConnectionId) {
        (**self).detach(connection)
    }
}

/// What a client sends.
#[derive(Clone, PartialEq, Debug)]
pub enum Message {
    Host {
        account: Account,
        name: String,
        colour: String,
        version: u32,
    },
    Join {
        match_id: MatchId,
        account: Account,
        name: String,
        colour: String,
        version: u32,
    },
    Bar {
        match_id: MatchId,
        seat: Seat,
    },
    Lift {
        match_id: MatchId,
        seat: Seat,
    },
    Start {
        match_id: MatchId,
    },
    /// Any claim: a raise, a liar or a spot on. The relay records it and forwards it; it
    /// does not judge it.
    Claim {
        match_id: MatchId,
        event: Event,
    },
    /// This seat has seen the round end.
    RoundOver {
        match_id: MatchId,
    },
    /// Reattach to a seat already in the match.
    Resume {
        match_id: MatchId,
        account: Account,
    },
}

/// Why a message was not accepted. None of these are game rules.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refused {
    NoSuchMatch,
    VersionMismatch { match_version: u32, yours: u32 },
    AlreadyStarted,
    Barred,
    ColourTaken,
    MatchIsFull,
    NotTheHost,
    NotInThisMatch,
    SeatIsAbsent,
    NotYourSeat,
}

/// What the relay did, and what to send where.
#[derive(Clone, PartialEq, Debug)]
pub struct Outcome {
    pub refused: Option<Refused>,
    pub sends: Vec<(ConnectionId, Event)>,
    /// Set when a message created a match.
    pub created: Option<MatchId>,
}

impl Outcome {
    fn refused(why: Refused) -> Self {
        Self {
            refused: Some(why),
            sends: Vec::new(),
            created: None,
        }
    }
}

/// A match in the lobby list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Listed {
    pub match_id: MatchId,
    pub host: String,
    pub players: usize,
}

/// A match has at most ten players.
pub const MAX_PLAYERS: usize = 10;

pub struct Relay<L: Log, C: Connections> {
    log: L,
    connections: C,
    secret: Secret,
}

impl<L: Log, C: Connections> Relay<L, C> {
    pub fn new(log: L, connections: C, secret: Secret) -> Self {
        Self {
            log,
            connections,
            secret,
        }
    }

    pub fn log(&self) -> &L {
        &self.log
    }

    pub fn connections(&self) -> &C {
        &self.connections
    }

    /// Every match a given account may join: hosted, not started, not barring them.
    ///
    /// Actorless in the specification, so `None` lists everything joinable. An account
    /// that has been barred is not shown the match at all.
    pub fn open_matches(&self, asking: Option<&Account>) -> Vec<Listed> {
        let mut listed = Vec::new();
        for match_id in self.log.matches() {
            let view = MatchView::fold(&self.log.read(&match_id));
            if view.started || view.host().is_none() {
                continue;
            }
            if let Some(account) = asking
                && view.is_barred(account)
            {
                continue;
            }
            listed.push(Listed {
                match_id,
                host: view
                    .host()
                    .map(|seat| view.name_of(seat).to_string())
                    .unwrap_or_default(),
                players: view.seats.len(),
            });
        }
        listed.sort_by(|left, right| left.match_id.cmp(&right.match_id));
        listed
    }

    pub fn view(&self, match_id: &MatchId) -> MatchView {
        MatchView::fold(&self.log.read(match_id))
    }

    pub fn receive(&mut self, from: &ConnectionId, message: Message) -> Outcome {
        match message {
            Message::Host {
                account,
                name,
                colour,
                version,
            } => self.host(from, account, name, colour, version),
            Message::Join {
                match_id,
                account,
                name,
                colour,
                version,
            } => self.join(from, match_id, account, name, colour, version),
            Message::Bar { match_id, seat } => self.bar(from, match_id, seat, true),
            Message::Lift { match_id, seat } => self.bar(from, match_id, seat, false),
            Message::Start { match_id } => self.start(from, match_id),
            Message::Claim { match_id, event } => self.claim(from, match_id, event),
            Message::RoundOver { match_id } => self.round_over(from, match_id),
            Message::Resume { match_id, account } => self.resume(from, match_id, account),
        }
    }

    /// A connection went away. Its seat is absent until it returns.
    pub fn disconnect(&mut self, connection: &ConnectionId) {
        self.connections.detach(connection);
    }

    fn host(
        &mut self,
        from: &ConnectionId,
        account: Account,
        name: String,
        colour: String,
        version: u32,
    ) -> Outcome {
        if version != PROTOCOL_VERSION {
            return Outcome::refused(Refused::VersionMismatch {
                match_version: PROTOCOL_VERSION,
                yours: version,
            });
        }
        let match_id = MatchId(format!("match-{}", self.log.matches().len()));
        let seat = Seat(0);
        self.write(&match_id, Event::Version { protocol: version });
        self.write(
            &match_id,
            Event::Join {
                seat,
                name,
                colour,
                account,
            },
        );
        self.write(&match_id, Event::Host { seat });
        self.connections
            .attach(&match_id, from.clone(), seat, Role::Player);
        Outcome {
            refused: None,
            sends: Vec::new(),
            created: Some(match_id),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn join(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        account: Account,
        name: String,
        colour: String,
        version: u32,
    ) -> Outcome {
        let view = self.view(&match_id);
        if view.host().is_none() {
            return Outcome::refused(Refused::NoSuchMatch);
        }
        if version != view.version {
            return Outcome::refused(Refused::VersionMismatch {
                match_version: view.version,
                yours: version,
            });
        }
        if view.started {
            return Outcome::refused(Refused::AlreadyStarted);
        }
        if view.is_barred(&account) {
            return Outcome::refused(Refused::Barred);
        }
        if view.seats.len() >= MAX_PLAYERS {
            return Outcome::refused(Refused::MatchIsFull);
        }
        if view.seats.iter().any(|seat| seat.colour == colour) {
            return Outcome::refused(Refused::ColourTaken);
        }
        let seat = Seat(view.seats.len() as u8);
        let event = Event::Join {
            seat,
            name,
            colour,
            account,
        };
        self.write(&match_id, event.clone());
        self.connections
            .attach(&match_id, from.clone(), seat, Role::Player);
        Outcome {
            refused: None,
            sends: self.to_everyone(&match_id, event),
            created: None,
        }
    }

    fn bar(&mut self, from: &ConnectionId, match_id: MatchId, seat: Seat, on: bool) -> Outcome {
        let view = self.view(&match_id);
        match self.seat_of(&match_id, from) {
            Some(mine) if Some(mine) == view.host() => {}
            Some(_) => return Outcome::refused(Refused::NotTheHost),
            None => return Outcome::refused(Refused::NotInThisMatch),
        }
        let event = if on {
            Event::Bar { seat }
        } else {
            Event::Lift { seat }
        };
        self.write(&match_id, event.clone());
        Outcome {
            refused: None,
            sends: self.to_everyone(&match_id, event),
            created: None,
        }
    }

    fn start(&mut self, from: &ConnectionId, match_id: MatchId) -> Outcome {
        let view = self.view(&match_id);
        match self.seat_of(&match_id, from) {
            Some(mine) if Some(mine) == view.host() => {}
            Some(_) => return Outcome::refused(Refused::NotTheHost),
            None => return Outcome::refused(Refused::NotInThisMatch),
        }
        if view.started {
            return Outcome::refused(Refused::AlreadyStarted);
        }
        let mut sends = Vec::new();

        // Turn order is chosen at random when the match starts. Derived from the match's
        // own secret rather than stored, so anyone holding the log can recompute it, and
        // keyed apart from any round so it says nothing about anyone's dice.
        let ordering = integrity::shuffle(
            &integrity::seed_for(&self.secret, &match_id.0, ORDER_ROUND),
            view.seats.len(),
        );
        let order = Event::PlayerOrder(ordering.iter().map(|seat| Seat(*seat as u8)).collect());
        self.write(&match_id, order.clone());
        sends.extend(self.to_everyone(&match_id, order));

        // One per variant, so a replay knows which rules were in force. The relay does
        // not know what a variant means; it records the settings it was given.
        for (name, on) in VARIANTS {
            let event = Event::Variant { name, on };
            self.write(&match_id, event.clone());
            sends.extend(self.to_everyone(&match_id, event));
        }

        // The setup is declared before the match starts, so a client has the order and
        // the variants in hand when it builds its game.
        self.write(&match_id, Event::Start);
        sends.extend(self.to_everyone(&match_id, Event::Start));

        sends.extend(self.open_round(&match_id, 0));
        Outcome {
            refused: None,
            sends,
            created: None,
        }
    }

    fn claim(&mut self, from: &ConnectionId, match_id: MatchId, event: Event) -> Outcome {
        let Some(seat) = self.seat_of(&match_id, from) else {
            return Outcome::refused(Refused::NotInThisMatch);
        };
        if seat_named(&event) != Some(seat) {
            return Outcome::refused(Refused::NotYourSeat);
        }
        if self.absent(&match_id) {
            return Outcome::refused(Refused::SeatIsAbsent);
        }
        // Recorded and forwarded without being judged. Whether it was legal is for the
        // receiving clients to decide; the relay only guarantees everyone sees the same
        // messages in the same order.
        self.write(&match_id, event.clone());
        Outcome {
            refused: None,
            sends: self.to_everyone(&match_id, event),
            created: None,
        }
    }

    /// A seat says the round ended. When every connected seat has said so, the seed is
    /// revealed and the next round is dealt.
    fn round_over(&mut self, from: &ConnectionId, match_id: MatchId) -> Outcome {
        let Some(seat) = self.seat_of(&match_id, from) else {
            return Outcome::refused(Refused::NotInThisMatch);
        };
        if self.absent(&match_id) {
            return Outcome::refused(Refused::SeatIsAbsent);
        }
        let event = Event::RoundOver { seat };
        self.write(&match_id, event.clone());
        let mut sends = self.to_everyone(&match_id, event);

        let view = self.view(&match_id);
        if view.round_over_count() < view.seats.len() {
            return Outcome {
                refused: None,
                sends,
                created: None,
            };
        }

        let round = view.round;
        sends.extend(self.reveal(&match_id, round));
        sends.extend(self.open_round(&match_id, round + 1));
        Outcome {
            refused: None,
            sends,
            created: None,
        }
    }

    fn resume(&mut self, from: &ConnectionId, match_id: MatchId, account: Account) -> Outcome {
        let view = self.view(&match_id);
        let Some(seat) = view.seat_of_account(&account) else {
            return Outcome::refused(Refused::NotInThisMatch);
        };
        self.connections
            .attach(&match_id, from.clone(), seat, Role::Player);
        // Replayed through the same filter as if they had never left, so returning
        // reveals nothing that staying would not have.
        let sends = self
            .visible_to(&match_id, seat)
            .into_iter()
            .map(|event| (from.clone(), event))
            .collect();
        Outcome {
            refused: None,
            sends,
            created: None,
        }
    }

    /// Commit to the round, deal every seat, and send each seat only its own dice.
    fn open_round(&mut self, match_id: &MatchId, round: u32) -> Vec<(ConnectionId, Event)> {
        let seed = integrity::seed_for(&self.secret, &match_id.0, round);
        let commit = Event::Commit {
            digest: integrity::commit(&seed).to_hex(),
        };
        self.write(match_id, commit.clone());
        let mut sends = self.to_everyone(match_id, commit);

        let seats = self.view(match_id).seats.len();
        for index in 0..seats {
            let seat = Seat(index as u8);
            let dice = integrity::deal(&seed, seat.0, DICE_PER_SEAT);
            let event = Event::Deal { seat, dice };
            self.write(match_id, event.clone());
            // Only this seat's own connection is told. Every other seat learns it at the
            // reveal.
            for connected in self.connections.of(match_id) {
                if connected.seat == seat {
                    sends.push((connected.connection.clone(), event.clone()));
                }
            }
        }
        sends
    }

    /// The round is over: every seat's dice, then the seed they came from.
    fn reveal(&mut self, match_id: &MatchId, round: u32) -> Vec<(ConnectionId, Event)> {
        let seed = integrity::seed_for(&self.secret, &match_id.0, round);
        let mut sends = Vec::new();
        let seats = self.view(match_id).seats.len();
        for index in 0..seats {
            let seat = Seat(index as u8);
            let event = Event::Deal {
                seat,
                dice: integrity::deal(&seed, seat.0, DICE_PER_SEAT),
            };
            for connected in self.connections.of(match_id) {
                if connected.seat != seat {
                    sends.push((connected.connection.clone(), event.clone()));
                }
            }
        }
        let event = Event::Reveal {
            seed: seed.to_hex(),
        };
        self.write(match_id, event.clone());
        sends.extend(self.to_everyone(match_id, event));
        sends
    }

    /// Everything this seat would have seen, in order.
    fn visible_to(&self, match_id: &MatchId, seat: Seat) -> Vec<Event> {
        let events = self.log.read(match_id);
        let revealed = view::revealed_rounds(&events);
        let mut round = 0;
        let mut visible = Vec::new();
        for event in events {
            match &event {
                Event::Deal { seat: dealt, .. } => {
                    if *dealt == seat || revealed.contains(&round) {
                        visible.push(event);
                    }
                }
                Event::Reveal { .. } => {
                    visible.push(event);
                    round += 1;
                }
                _ => visible.push(event),
            }
        }
        visible
    }

    /// Whether any seat has nobody on it.
    fn absent(&self, match_id: &MatchId) -> bool {
        let view = self.view(match_id);
        if !view.started {
            return false;
        }
        let connected = self.connections.of(match_id);
        (0..view.seats.len())
            .any(|index| !connected.iter().any(|row| row.seat == Seat(index as u8)))
    }

    fn seat_of(&self, match_id: &MatchId, connection: &ConnectionId) -> Option<Seat> {
        self.connections
            .of(match_id)
            .into_iter()
            .find(|row| &row.connection == connection)
            .map(|row| row.seat)
    }

    fn to_everyone(&self, match_id: &MatchId, event: Event) -> Vec<(ConnectionId, Event)> {
        self.connections
            .of(match_id)
            .into_iter()
            .map(|row| (row.connection, event.clone()))
            .collect()
    }

    /// Append, retrying at the next position if another message got there first.
    fn write(&mut self, match_id: &MatchId, event: Event) {
        let mut position = self.log.read(match_id).len() as u64;
        while self
            .log
            .append(match_id, position, std::slice::from_ref(&event))
            .is_err()
        {
            position += 1;
        }
    }
}

/// The round number the turn ordering is derived under. Never a real round, so an
/// ordering and a deal can never share a seed.
const ORDER_ROUND: u32 = u32::MAX;

/// The variants a hosted match is played under. Every one off, which is the default rules.
/// The relay records them; it does not know what any of them does.
/// Every variant a match declares, and whether it is on. Public because these are the
/// only names an `Event::Variant` in this log can carry, so anything reading the log back
/// out of storage needs the list to recover the `&'static str` the event holds.
pub const VARIANTS: [(&str, bool); 1] = [("ones-wild", false)];

/// Every seat is dealt the dice limit. The relay does not know what a dice limit means,
/// only how many faces to deal, which is why this is here and not in `rules`.
pub const DICE_PER_SEAT: usize = 5;

/// Which seat a claim came from, without knowing what the claim says.
fn seat_named(event: &Event) -> Option<Seat> {
    match event {
        Event::Claim { seat, .. } | Event::Liar { seat, .. } | Event::SpotOn { seat, .. } => {
            Some(*seat)
        }
        _ => None,
    }
}
