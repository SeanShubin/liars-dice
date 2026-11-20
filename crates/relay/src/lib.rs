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

/// The log moved while a message was being decided, so the decision is thrown away.
///
/// Never returned to a caller: [`Relay::receive`] reads again and decides again, and the
/// client is told nothing because nothing happened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Stale;

/// How many times one message may be decided before the relay gives up.
///
/// A turn-based game has no contention to speak of — two players acting at the same
/// instant is the whole of it — so reaching this means something is wrong that retrying
/// will not fix. Failing the invocation is the right answer: nothing has been written, and
/// the client is free to send the message again.
const MAX_ATTEMPTS: usize = 16;

/// One match's log as a message is being decided against it.
///
/// Holds what was read and what is about to be appended, which is what makes a decision
/// and the write that follows it one unit. Every guard in every handler reads this rather
/// than the log, so a handler sees its own writes — `round_over` counts the
/// acknowledgement it just added — without any of them reaching storage until the whole
/// message does.
struct Working {
    match_id: MatchId,
    /// The length of the log this was read at. The version the decision is based on.
    at: u64,
    /// The events read, then the ones this message adds.
    events: Vec<Event>,
    /// Where the added ones begin.
    added: usize,
}

impl Working {
    /// A match that must not already exist.
    ///
    /// `at` is zero rather than the end of whatever is there, so creating a match is a
    /// claim on position zero. Without that, a host that lost the race for an id would
    /// append its events to the end of the winner's match and both would be told they
    /// created it.
    fn fresh(match_id: MatchId) -> Self {
        Self {
            match_id,
            at: 0,
            events: Vec::new(),
            added: 0,
        }
    }

    fn view(&self) -> MatchView {
        MatchView::fold(&self.events)
    }

    fn push(&mut self, event: Event) {
        self.events.push(event);
    }

    fn added(&self) -> &[Event] {
        &self.events[self.added..]
    }
}

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

    /// Put this connection on this seat, and nothing else anywhere near either.
    ///
    /// Two evictions, not one. The connection leaves wherever else it was, because a
    /// socket is in one place. And whatever else held this seat leaves it, because a seat
    /// is held by one socket — which is the invariant that actually decides who is sent
    /// whose dice. `Two players never share a seat` is a statement about the log; this is
    /// the same statement about the transport, and it is the half that can leak.
    ///
    /// So a second tab on one account takes the seat and the first goes quiet. That is
    /// the right trade: the alternative is a seat whose deal is sent to two sockets, and
    /// before identity exists there is nothing to distinguish a second tab from somebody
    /// else claiming the account.
    ///
    /// **An implementation may not do this by reading and then writing.** Two calls for
    /// one seat would both read, both find nobody to evict, and both write — which is what
    /// the deployed one did, and no caller could have noticed. The eviction has to fall out
    /// of how a place is stored rather than out of steps taken in order, so the seat
    /// belongs in the key. `relay-service.md` has the table; `memory` gets it for free by
    /// being one thread.
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
        /// Which request this is, so the match it makes has a name.
        ///
        /// Need not be secret or unpredictable, only distinct per request. Sending the
        /// same one twice names the same match, which is what makes hosting safe to
        /// repeat when an answer went missing.
        nonce: String,
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
    /// This seat is done with the match.
    ///
    /// Coordination, not a rule: who is taking part is the relay's business, the same as
    /// who hosted and who is barred. What leaving *means* belongs to the rules, which
    /// read it as an elimination.
    Leave {
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
    /// What is open to join.
    ///
    /// The one message that names no match, because asking which there are is the thing a
    /// client does before it has one. The account is carried so a barred player is not
    /// shown the match they are barred from, which is the only reason the relay needs to
    /// know who is asking.
    List {
        account: Account,
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
    VersionMismatch {
        match_version: u32,
        yours: u32,
    },
    AlreadyStarted,
    Barred,
    ColourTaken,
    MatchIsFull,
    NotTheHost,
    NotInThisMatch,
    SeatIsAbsent,
    NotYourSeat,
    /// The derived match id names a match this account did not host.
    ///
    /// Reachable only by a collision in sixteen bytes of HMAC, which is not a thing that
    /// happens — but refusing is still better than the alternatives. Retrying cannot help,
    /// because the name is a function of the request and would be derived identically, and
    /// appending to the match would hand it to somebody who is not its host.
    MatchIdTaken,
}

/// What the relay did, and what to send where.
#[derive(Clone, PartialEq, Debug)]
pub struct Outcome {
    pub refused: Option<Refused>,
    pub sends: Vec<(ConnectionId, Event)>,
    /// Set when a message created a match.
    pub created: Option<MatchId>,
    /// Set when a message asked what is open. Empty is an answer; absent is not an answer.
    pub listed: Option<Vec<Listed>>,
}

impl Outcome {
    fn refused(why: Refused) -> Self {
        Self {
            refused: Some(why),
            sends: Vec::new(),
            created: None,
            listed: None,
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

    /// Serve one message: decide it against the log, and commit it to the log it was
    /// decided against.
    ///
    /// The loop is the concurrency design. Every handler reads the log, decides from what
    /// it read, and appends at the length it saw; if something else appended first the
    /// append is refused, the decision is discarded, and the whole message is decided
    /// again against what is now there. See `The position is the version` in
    /// /implementation/relay.md.
    pub fn receive(&mut self, from: &ConnectionId, message: Message) -> Outcome {
        for _ in 0..MAX_ATTEMPTS {
            match self.attempt(from, message.clone()) {
                Ok(outcome) => return outcome,
                Err(Stale) => continue,
            }
        }
        panic!("gave up deciding a message after {MAX_ATTEMPTS} attempts; nothing was written");
    }

    /// Read the log, decide, and commit — once.
    fn attempt(&mut self, from: &ConnectionId, message: Message) -> Result<Outcome, Stale> {
        match message {
            Message::Host {
                account,
                name,
                colour,
                version,
                nonce,
            } => self.host(from, account, name, colour, version, nonce),
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
            Message::Leave { match_id } => self.leave(from, match_id),
            Message::Claim { match_id, event } => self.claim(from, match_id, event),
            Message::RoundOver { match_id } => self.round_over(from, match_id),
            Message::List { account } => self.list(&account),
            Message::Resume { match_id, account } => self.resume(from, match_id, account),
        }
    }

    /// The match's log as it is now, ready to be decided against.
    fn open(&self, match_id: &MatchId) -> Working {
        let events = self.log.read(match_id);
        let at = events.len() as u64;
        Working {
            match_id: match_id.clone(),
            at,
            events,
            added: at as usize,
        }
    }

    /// Append what the message added, at the position it was decided against.
    fn commit(&mut self, work: &Working) -> Result<(), Stale> {
        if work.added().is_empty() {
            return Ok(());
        }
        self.log
            .append(&work.match_id, work.at, work.added())
            .map_err(|Taken| Stale)
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
        nonce: String,
    ) -> Result<Outcome, Stale> {
        if version != PROTOCOL_VERSION {
            return Ok(Outcome::refused(Refused::VersionMismatch {
                match_version: PROTOCOL_VERSION,
                yours: version,
            }));
        }
        // Derived, not counted. Nothing is read to name a match, so hosting scans no
        // table and has no window between choosing a name and claiming it.
        let match_id = MatchId(integrity::name_for(
            &self.secret,
            &protocol::account_text(&account),
            &nonce,
        ));
        let seat = Seat(0);
        // `fresh` rather than `open`: a new match is a claim on position zero, so a name
        // that is already taken refuses the whole message instead of appending these
        // three events to the end of whatever is there.
        let mut work = Working::fresh(match_id.clone());
        work.push(Event::Version { protocol: version });
        work.push(Event::Join {
            seat,
            name,
            colour,
            account: account.clone(),
        });
        work.push(Event::Host { seat });

        if self.commit(&work).is_err() {
            // The name is taken, and the name is a function of this request, so the
            // likeliest reason by far is that this request already succeeded and its
            // answer went missing. Answering with the match the sender already has is
            // what `A repeated request to host creates no second match` asks for.
            //
            // Not a retry: deriving the name again would derive the same name.
            if self.open(&match_id).view().seat_of_account(&account) != Some(seat) {
                return Ok(Outcome::refused(Refused::MatchIdTaken));
            }
        }
        // After the commit, because a connection attached to a match that was never
        // created is a row pointing at nothing. Also on the repeated path: the resend
        // may well come from a new socket, and that socket is this seat's.
        self.connections
            .attach(&match_id, from.clone(), seat, Role::Player);

        // The host is sent its own join, exactly as a joiner is. A client learns which
        // seat it is on from the join that names its own account and from nothing else,
        // so a host that was told only the match id could never build a client at all.
        // Read back from the log rather than taken from `work`, so a repeated request is
        // answered with the join that is really recorded rather than with the one it just
        // asked for — the name and colour of the first request are the ones that stand.
        let joined = self
            .open(&match_id)
            .events
            .into_iter()
            .find(|event| matches!(event, Event::Join { .. }));
        let sends = match joined {
            Some(event) => self.to_everyone(&match_id, event),
            None => Vec::new(),
        };
        Ok(Outcome {
            refused: None,
            sends,
            created: Some(match_id),
            listed: None,
        })
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
    ) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id);
        let view = work.view();
        if view.host().is_none() {
            return Ok(Outcome::refused(Refused::NoSuchMatch));
        }
        if version != view.version {
            return Ok(Outcome::refused(Refused::VersionMismatch {
                match_version: view.version,
                yours: version,
            }));
        }
        if view.started {
            return Ok(Outcome::refused(Refused::AlreadyStarted));
        }
        if view.is_barred(&account) {
            return Ok(Outcome::refused(Refused::Barred));
        }
        if view.seats.len() >= MAX_PLAYERS {
            return Ok(Outcome::refused(Refused::MatchIsFull));
        }
        if view.seats.iter().any(|seat| seat.colour == colour) {
            return Ok(Outcome::refused(Refused::ColourTaken));
        }
        // Every guard above read the same snapshot this event is appended against, so the
        // seat, the colour, the player count and the started flag are all decided by the
        // commit rather than merely consulted before it.
        let seat = Seat(view.seats.len() as u8);
        let event = Event::Join {
            seat,
            name,
            colour,
            account,
        };
        work.push(event.clone());
        self.commit(&work)?;
        self.connections
            .attach(&match_id, from.clone(), seat, Role::Player);
        Ok(Outcome {
            refused: None,
            sends: self.to_everyone(&match_id, event),
            created: None,
            listed: None,
        })
    }

    fn bar(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        seat: Seat,
        on: bool,
    ) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id);
        let view = work.view();
        match self.seat_of(&match_id, from) {
            Some(mine) if Some(mine) == view.host() => {}
            Some(_) => return Ok(Outcome::refused(Refused::NotTheHost)),
            None => return Ok(Outcome::refused(Refused::NotInThisMatch)),
        }
        let event = if on {
            Event::Bar { seat }
        } else {
            Event::Lift { seat }
        };
        work.push(event.clone());
        self.commit(&work)?;
        Ok(Outcome {
            refused: None,
            sends: self.to_everyone(&match_id, event),
            created: None,
            listed: None,
        })
    }

    fn start(&mut self, from: &ConnectionId, match_id: MatchId) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id);
        let view = work.view();
        match self.seat_of(&match_id, from) {
            Some(mine) if Some(mine) == view.host() => {}
            Some(_) => return Ok(Outcome::refused(Refused::NotTheHost)),
            None => return Ok(Outcome::refused(Refused::NotInThisMatch)),
        }
        if view.started {
            return Ok(Outcome::refused(Refused::AlreadyStarted));
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
        work.push(order.clone());
        sends.extend(self.to_everyone(&match_id, order));

        // One per variant, so a replay knows which rules were in force. The relay does
        // not know what a variant means; it records the settings it was given.
        for (name, on) in VARIANTS {
            let event = Event::Variant { name, on };
            work.push(event.clone());
            sends.extend(self.to_everyone(&match_id, event));
        }

        // The setup is declared before the match starts, so a client has the order and
        // the variants in hand when it builds its game.
        work.push(Event::Start);
        sends.extend(self.to_everyone(&match_id, Event::Start));

        sends.extend(self.open_round(&mut work, 0));
        // Every event of the start — the order, the variants, the start, the commitment
        // and one deal per seat — lands together or not at all. A match that had been
        // dealt but not started, or started but not dealt, is not a state any client
        // knows how to be in.
        self.commit(&work)?;
        Ok(Outcome {
            refused: None,
            sends,
            created: None,
            listed: None,
        })
    }

    fn claim(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        event: Event,
    ) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id);
        let Some(seat) = self.seat_of(&match_id, from) else {
            return Ok(Outcome::refused(Refused::NotInThisMatch));
        };
        if seat_named(&event) != Some(seat) {
            return Ok(Outcome::refused(Refused::NotYourSeat));
        }
        // Recorded and forwarded without being judged. Whether it was legal is for the
        // receiving clients to decide; the relay only guarantees everyone sees the same
        // messages in the same order.
        work.push(event.clone());
        self.commit(&work)?;
        Ok(Outcome {
            refused: None,
            sends: self.to_everyone(&match_id, event),
            created: None,
            listed: None,
        })
    }

    /// A seat says the round ended. When every connected seat has said so, the seed is
    /// revealed and the next round is dealt.
    fn round_over(&mut self, from: &ConnectionId, match_id: MatchId) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id);
        let Some(seat) = self.seat_of(&match_id, from) else {
            return Ok(Outcome::refused(Refused::NotInThisMatch));
        };
        let event = Event::RoundOver { seat };
        work.push(event.clone());
        let mut sends = self.to_everyone(&match_id, event);

        // Counted from the working copy, so this seat's own acknowledgement — pushed a
        // line ago and not yet in storage — is included. Reading the log here would
        // count every seat but the one that just spoke.
        //
        // Against the seats that are present and have not left, which is what
        // `relay.md` always said and what the code used to get wrong by counting every
        // seat that ever joined. A seat that is absent would otherwise hold the reveal,
        // and `A match halts at the turn of an absent player` says the halt comes at
        // their turn and not before it.
        let view = work.view();
        // Seats, not rows. `round_over_count` counts distinct seats and can never exceed
        // the number of them, so counting rows against it compares two different kinds of
        // thing: one duplicated row would lift this above a number it cannot reach and the
        // round would never reveal. One row per seat makes the two counts equal today —
        // see `relay-service.md`, the place table — and naming the right one keeps this
        // correct when a place can be a watcher, since a watcher is sent the round and
        // never acknowledges it.
        let mut present: Vec<Seat> = Vec::new();
        for row in self.connections.of(&match_id) {
            let Role::Player = row.role;
            if !view.has_left(row.seat) && !present.contains(&row.seat) {
                present.push(row.seat);
            }
        }
        let waiting_on = present.len();
        if view.round_over_count() < waiting_on {
            self.commit(&work)?;
            return Ok(Outcome {
                refused: None,
                sends,
                created: None,
                listed: None,
            });
        }

        let round = view.round;
        sends.extend(self.reveal(&mut work, round));
        sends.extend(self.open_round(&mut work, round + 1));
        // The last acknowledgement, the reveal and the next round's deal are one message.
        // Revealing a seed without dealing the round that follows it would leave every
        // client waiting on a round that never opens.
        self.commit(&work)?;
        Ok(Outcome {
            refused: None,
            sends,
            created: None,
            listed: None,
        })
    }

    /// A seat leaves the match for good.
    ///
    /// Recorded rather than inferred. A lost connection is presence and says nothing
    /// permanent; this is membership, and it is what lets the relay stop waiting on
    /// somebody without knowing a single rule about why they are gone.
    fn leave(&mut self, from: &ConnectionId, match_id: MatchId) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id);
        let Some(seat) = self.seat_of(&match_id, from) else {
            return Ok(Outcome::refused(Refused::NotInThisMatch));
        };
        // Leaving twice is not an error and not an event. The second one would be a
        // second elimination of a player who is already gone.
        if work.view().has_left(seat) {
            return Ok(Outcome {
                refused: None,
                sends: Vec::new(),
                created: None,
                listed: None,
            });
        }
        let event = Event::Left { seat };
        work.push(event.clone());
        self.commit(&work)?;
        // The socket goes too. A seat that has left holds no connection, so nothing is
        // sent to it and nothing waits for it.
        let sends = self.to_everyone(&match_id, event);
        self.connections.detach(from);
        Ok(Outcome {
            refused: None,
            sends,
            created: None,
            listed: None,
        })
    }

    /// What is open to join.
    ///
    /// Writes nothing and reads everything, which is the opposite of every other handler
    /// and the reason the lobby has a cost worth measuring — see `Measuring what the lobby
    /// costs` in `relay-service.md`. There is no decision to invalidate, so it cannot be
    /// stale: a listing is true of the moment it was read and a match may fill before
    /// anybody acts on it, which is what the refusals on `join` are for.
    fn list(&self, asking: &Account) -> Result<Outcome, Stale> {
        Ok(Outcome {
            refused: None,
            sends: Vec::new(),
            created: None,
            listed: Some(self.open_matches(Some(asking))),
        })
    }

    /// Reattach to a seat. The one message that writes nothing.
    fn resume(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        account: Account,
    ) -> Result<Outcome, Stale> {
        let work = self.open(&match_id);
        let view = work.view();
        let Some(seat) = view.seat_of_account(&account) else {
            return Ok(Outcome::refused(Refused::NotInThisMatch));
        };
        self.connections
            .attach(&match_id, from.clone(), seat, Role::Player);
        // Replayed through the same filter as if they had never left, so returning
        // reveals nothing that staying would not have.
        let sends = self
            .visible_to(&work, seat)
            .into_iter()
            .map(|event| (from.clone(), event))
            .collect();
        // No commit: this message appends nothing, so there is no decision for the log to
        // invalidate. A seat that was resumed against a log that has since grown is still
        // resumed, and the events it missed reach it the usual way.
        Ok(Outcome {
            refused: None,
            sends,
            created: None,
            listed: None,
        })
    }

    /// Commit to the round, deal every seat, and send each seat only its own dice.
    fn open_round(&mut self, work: &mut Working, round: u32) -> Vec<(ConnectionId, Event)> {
        let match_id = work.match_id.clone();
        let seed = integrity::seed_for(&self.secret, &match_id.0, round);
        let commit = Event::Commit {
            digest: integrity::commit(&seed).to_hex(),
        };
        work.push(commit.clone());
        let mut sends = self.to_everyone(&match_id, commit);

        let seats = work.view().seats.len();
        for index in 0..seats {
            let seat = Seat(index as u8);
            let dice = integrity::deal(&seed, seat.0, DICE_PER_SEAT);
            let event = Event::Deal { seat, dice };
            work.push(event.clone());
            // Only this seat's own connection is told. Every other seat learns it at the
            // reveal.
            for connected in self.connections.of(&match_id) {
                if connected.seat == seat {
                    sends.push((connected.connection.clone(), event.clone()));
                }
            }
        }
        sends
    }

    /// The round is over: every seat's dice, then the seed they came from.
    fn reveal(&mut self, work: &mut Working, round: u32) -> Vec<(ConnectionId, Event)> {
        let match_id = work.match_id.clone();
        let seed = integrity::seed_for(&self.secret, &match_id.0, round);
        let mut sends = Vec::new();
        let seats = work.view().seats.len();
        for index in 0..seats {
            let seat = Seat(index as u8);
            let event = Event::Deal {
                seat,
                dice: integrity::deal(&seed, seat.0, DICE_PER_SEAT),
            };
            for connected in self.connections.of(&match_id) {
                if connected.seat != seat {
                    sends.push((connected.connection.clone(), event.clone()));
                }
            }
        }
        let event = Event::Reveal {
            seed: seed.to_hex(),
        };
        work.push(event.clone());
        sends.extend(self.to_everyone(&match_id, event));
        sends
    }

    /// Everything this seat would have seen, in order.
    fn visible_to(&self, work: &Working, seat: Seat) -> Vec<Event> {
        let events = work.events.clone();
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
