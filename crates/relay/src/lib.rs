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
use protocol::{Account, Event, Moment, PROTOCOL_VERSION, Seat, Who};

pub mod memory;
mod view;

pub use view::{Ending, MatchView};

/// How long a started match must be inactive before its host may cancel it.
pub const CANCEL_AFTER_SECONDS: u64 = 60;

/// How long any match must be inactive before it is abandoned.
pub const ABANDON_AFTER_SECONDS: u64 = 60 * 60;

/// How long ago the relay recorded the last event in a match, in whole seconds.
///
/// No recorded time means inactive since forever. A match always holds events, because
/// hosting writes three, so a log with no recorded time is one written before the relay
/// recorded them — provably older than the deploy, and therefore older than any threshold
/// here. One meaning, not two.
///
/// Saturating, so a clock that went backwards reads as *just now* rather than as an
/// enormous idle time. The safe direction: it delays a cancellation instead of causing one.
fn idle_seconds(last: Option<Moment>, now: Moment) -> u64 {
    match last {
        Some(last) => now.to_unix_nanos().saturating_sub(last.to_unix_nanos()) / 1_000_000_000,
        None => u64::MAX,
    }
}

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

/// Who is sitting down, and what they are called.
///
/// One struct rather than four parameters repeated on two handlers. Hosting and joining
/// ask the same four questions, and a parameter list long enough for a lint to complain
/// about is a parameter list long enough to pass two strings in the wrong order.
struct Sitting {
    account: Account,
    name: String,
    version: u32,
}

/// The colours the seats of a match hold.
fn taken_colours(view: &MatchView) -> Vec<&str> {
    view.seats.iter().map(|seat| seat.colour.as_str()).collect()
}

/// The colour a seat is given when it is taken.
///
/// `A seat is given a colour when it is taken`, from `protocol::ASSIGNMENT_ORDER`, which
/// is a declared constant precisely so that no floating point decides the content of an
/// event. Falls back to the first palette entry if every colour is somehow taken, which
/// needs twenty seats and so cannot happen in a match: a fallback rather than a panic,
/// because a seat with a repeated colour is a worse match and a panicking relay is a
/// worse service.
fn given_colour(taken: &[&str]) -> String {
    protocol::free_colour(taken)
        .unwrap_or(protocol::COLOURS[0])
        .to_string()
}

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
    /// When the relay recorded the last event it read. `None` if there were none, or if
    /// the row predates recorded times.
    last: Option<Moment>,
    /// The one clock reading this message gets.
    ///
    /// Held here rather than threaded separately so that the decision and the stamp on
    /// the rows it writes come from the same reading, and cannot disagree about when the
    /// message happened.
    now: Moment,
}

impl Working {
    /// A match that must not already exist.
    ///
    /// `at` is zero rather than the end of whatever is there, so creating a match is a
    /// claim on position zero. Without that, a host that lost the race for an id would
    /// append its events to the end of the winner's match and both would be told they
    /// created it.
    fn fresh(match_id: MatchId, now: Moment) -> Self {
        Self {
            match_id,
            at: 0,
            events: Vec::new(),
            added: 0,
            last: None,
            now,
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

/// What a read of one match's log returned.
///
/// The events, and when the relay recorded the last of them. Both from one read, because
/// a second method would be a second query per match — which is what the dashboard pass can
/// least afford — and a second reader of the same rows, which is a second thing to get
/// wrong.
///
/// The time is deliberately *beside* the events rather than inside one. An event field
/// would reach `wire`, the archive's line format, every golden-file test and every
/// client's fold, and `Every client derives the same match state from the same events`
/// would start depending on when they arrived. Here, `MatchView::fold` takes `&[Event]`
/// and so structurally cannot see it.
pub struct Recording {
    pub events: Vec<Event>,
    /// `None` if the log holds no events, or if the last row was written before the relay
    /// recorded times. See [`idle_seconds`] for why those are one case and not two.
    pub last: Option<Moment>,
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
    ///
    /// `now` is stamped on every row written, which is `The relay records when it
    /// received each event`. It is passed in rather than read here so that an
    /// implementation of this trait observes nothing: one reading per message serves the
    /// decision and the stamp, so they cannot disagree about when the message happened.
    fn append(
        &mut self,
        match_id: &MatchId,
        from: u64,
        events: &[Event],
        now: Moment,
    ) -> Result<(), Taken>;

    fn read(&self, match_id: &MatchId) -> Recording;

    /// Every match the log holds, for the dashboard list.
    fn matches(&self) -> Vec<MatchId>;

    /// Every match this account has ever joined, for finding the one it is in.
    ///
    /// `account` is as `protocol::account_text` writes it. The default answers *all of
    /// them*, which is correct and is what an in-memory log can afford; `DynamoLog`
    /// overrides it with an index, because asking about every match on the path of every
    /// host and every join would put the hot path on the dashboard's cost.
    ///
    /// An override may only ever narrow this, and cannot change the answer: the index is
    /// derived by the database from the log's own rows, so it holds exactly the matches an
    /// account appears in. The caller folds each candidate anyway, because *has ever
    /// joined* is not *is in*.
    fn matches_of(&self, account: &str) -> Vec<MatchId> {
        let _ = account;
        self.matches()
    }
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
    fn append(
        &mut self,
        match_id: &MatchId,
        from: u64,
        events: &[Event],
        now: Moment,
    ) -> Result<(), Taken> {
        (**self).append(match_id, from, events, now)
    }

    fn read(&self, match_id: &MatchId) -> Recording {
        (**self).read(match_id)
    }

    fn matches(&self) -> Vec<MatchId> {
        (**self).matches()
    }

    fn matches_of(&self, account: &str) -> Vec<MatchId> {
        (**self).matches_of(account)
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
        who: Who,
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
        who: Who,
        version: u32,
    },
    /// A seat chooses the colour of its dice, which it may do until the match starts.
    Colour {
        match_id: MatchId,
        colour: String,
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
    /// This seat has derived that the match is over.
    ///
    /// The relay writes no resolution event, so it cannot tell a finished match from one
    /// in progress and may not work it out — that would be a game rule. So it counts these
    /// the way it counts [`Message::RoundOver`], and records `Complete` when every seat
    /// present has said it.
    Over {
        match_id: MatchId,
    },
    /// The host ends the match.
    Cancel {
        match_id: MatchId,
    },
    /// What is open to join.
    ///
    /// The one message that names no match, because asking which there are is the thing a
    /// client does before it has one. Who is asking is carried so a barred player is not
    /// shown the match they are barred from.
    ///
    /// A `Who` like every other message that names a player, rather than an `Account`: a
    /// client never sends an account. It says who it is, and the relay decides what that
    /// means — which is the whole of why a client cannot claim a Discord account it does
    /// not hold.
    List {
        who: Who,
    },
    /// Reattach to a seat already in the match.
    Resume {
        match_id: MatchId,
        who: Who,
    },
}

impl Message {
    /// The match this message is about, if it names one.
    fn about(&self) -> Option<&MatchId> {
        match self {
            // Hosting names the match it is about to make, which does not exist yet.
            Message::Host { .. } | Message::List { .. } => None,
            Message::Join { match_id, .. }
            | Message::Colour { match_id, .. }
            | Message::Bar { match_id, .. }
            | Message::Lift { match_id, .. }
            | Message::Start { match_id }
            | Message::Leave { match_id }
            | Message::Claim { match_id, .. }
            | Message::RoundOver { match_id }
            | Message::Over { match_id }
            | Message::Cancel { match_id }
            | Message::Resume { match_id, .. } => Some(match_id),
        }
    }

    /// Whether this message would append to the match it names.
    ///
    /// `No event is recorded in a match that is over` is one guard in [`Relay::attempt`]
    /// rather than one per handler, and this is what it asks. Two messages are let
    /// through: `List`, which is about every match rather than one, and `Resume`, because
    /// somebody reconnecting to a finished match wants to see how it ended.
    ///
    /// A `match` with no wildcard, so a message added later cannot be forgotten here.
    fn writes(&self) -> bool {
        match self {
            Message::Host { .. }
            | Message::Join { .. }
            | Message::Colour { .. }
            | Message::Bar { .. }
            | Message::Lift { .. }
            | Message::Start { .. }
            | Message::Leave { .. }
            | Message::Claim { .. }
            | Message::RoundOver { .. }
            | Message::Over { .. }
            | Message::Cancel { .. } => true,
            Message::List { .. } | Message::Resume { .. } => false,
        }
    }
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
    /// The match has ended, so nothing more is recorded in it.
    ///
    /// One refusal for all three endings, because `No event is recorded in a match that is
    /// over` is one rule. It is what a join into a cancelled match gets, and what a claim
    /// into a finished one gets, and it replaced `HostHasLeft` — a host who leaves a match
    /// that cannot begin is cancelling it, and two refusals for *this match is finished*
    /// was one too many. How it ended is in the log, for anybody the log is sent to.
    MatchIsOver,
    /// This account already holds a seat in another match.
    ///
    /// `A player cannot join a match while holding a seat in another match`, and the id is
    /// carried so a client can offer to take somebody there rather than leaving them to
    /// find it.
    ///
    /// Refused rather than resolved by leaving the other match for them: leaving is an
    /// elimination in the rules and goes in the log permanently, so a misclick must not
    /// cause one.
    AlreadyInAMatch {
        match_id: MatchId,
    },
    /// The ticket does not verify, has expired, or does not name a Discord account.
    ///
    /// One refusal for all of those, because `check_ticket` gives one answer for all of
    /// them: a relay that could tell a forged ticket from an expired one is a relay that
    /// can be asked which it was. The client's answer either way is to authenticate
    /// again, and it already knows its own ticket's expiry because the exchange told it.
    BadTicket,
    /// The host asked to cancel a started match too soon. `seconds` remain.
    ///
    /// Carries a number because there is something to say beyond the name: a button that
    /// does nothing is a bug report, and a button that says *forty-seven seconds* is a
    /// feature. Whole seconds, rounded up, so a host told three and asking again on the
    /// third second is not told zero.
    StillActive {
        seconds: u32,
    },
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

/// A match in the dashboard list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Listed {
    pub match_id: MatchId,
    pub host: String,
    pub players: usize,
    /// Whether the asking account could join it right now.
    ///
    /// The refusal `join` would give, inverted — not a second opinion about it. Two copies
    /// of *can this be joined* would eventually disagree, and then the dashboard offers a match
    /// that refuses you with the bug in neither place. See `why_not_joinable`.
    pub joinable: bool,
    /// Whether the asking account already holds a seat in it.
    ///
    /// At most one listed match can say yes, because `A player cannot join a match while
    /// holding a seat in another match`. It is how a client knows which match to return to
    /// without being asked.
    pub mine: bool,
    /// Whether every seat taken so far is held by an authenticated player.
    ///
    /// Derived in the same fold that produces the host and the player count, so the dashboard
    /// can say which matches are which before anybody joins one. True of a match that is
    /// still filling says only that it is ranked *so far*: `Whether a match is ranked is
    /// settled when it starts`, and one guest joining before then settles it the other
    /// way.
    pub ranked: bool,
}

/// A match has at most ten players.
pub const MAX_PLAYERS: usize = 10;

/// The secrets a relay holds. Two, and kept apart on purpose.
///
/// The exchange that mints tickets has to hold the key they are verified with, and it is
/// the one piece of this system that talks to the public internet. Sharing the dice secret
/// with it would mean that compromising the exchange let somebody predict every roll, and
/// `Clients must not be able to choose their roll beforehand or alter it after the fact` is
/// the property this whole repository is arranged around. So they are two keys, and only
/// the relay ever holds the one that matters most.
///
/// The alternative was one secret with the ticket tagged apart from the seeds, which
/// `integrity` already does and which this still relies on. That stops one message space
/// being mistaken for another; it does not stop a second service from deriving seeds. The
/// tag is about confusion and this is about reach.
pub struct Secrets {
    /// Round seeds and match names derive from this. Nothing else ever holds it.
    pub dice: Secret,
    /// Tickets are verified with this. The exchange holds it too, because it mints them.
    ///
    /// `None` means nobody can authenticate — which is what a deployment looks like before
    /// the parameter is set, and `deployment.md` requires that the stack deploy before the
    /// secrets are. Guests are unaffected, so every unranked thing still works.
    pub ticket: Option<Secret>,
}

pub struct Relay<L: Log, C: Connections> {
    log: L,
    connections: C,
    secrets: Secrets,
}

impl<L: Log, C: Connections> Relay<L, C> {
    pub fn new(log: L, connections: C, secrets: Secrets) -> Self {
        Self {
            log,
            connections,
            secrets,
        }
    }

    pub fn log(&self) -> &L {
        &self.log
    }

    pub fn connections(&self) -> &C {
        &self.connections
    }

    /// Every match worth showing, and what the asking account may do with each.
    ///
    /// `A started match is listed` and `A match that is over is not listed`, so this is
    /// what is going on rather than only what is on offer. Each row carries whether the
    /// asker can join it and whether they are already in it.
    ///
    /// Actorless in the specification, so `None` lists everything joinable. An account
    /// that has been barred is not shown the match at all.
    ///
    /// Takes `&mut self` because this is also where matches expire. There is no monitor,
    /// so the hour is noticed by whatever next reads the match, and this pass reads all of
    /// them already.
    pub fn open_matches(&mut self, asking: Option<&Account>, now: Moment) -> Vec<Listed> {
        let mut listed = Vec::new();
        for match_id in self.log.matches() {
            let recording = self.log.read(&match_id);
            let view = MatchView::fold(&recording.events);
            // `A match that is over is not listed`, and `No event is recorded in a match
            // that is over` — one test doing both jobs, and ahead of the sweep so that
            // abandoning is idempotent for the obvious reason: an abandoned match is
            // over, so the next pass stops here.
            //
            // The guard in `attempt` cannot cover this. That one asks whether a *message*
            // writes, and the sweep is not a message — nobody sent it and nobody is
            // answered.
            if view.over() {
                continue;
            }
            // `A match in which no event has been recorded for an hour is cancelled`, and
            // nothing is scheduled to notice it: the check runs where the matches are
            // already being read. One conditional put per match crossing the hour, which
            // is normally none. This is why listing is a message that writes.
            if idle_seconds(recording.last, now) >= ABANDON_AFTER_SECONDS {
                let at = recording.events.len() as u64;
                // A `Taken` means something appended first — a real event, or this same
                // abandonment from another connection listing at the same moment. Either
                // way this pass does not list the match, and nothing needs to be retried:
                // whoever won wrote the truth.
                let _ = self.log.append(&match_id, at, &[Event::Abandon], now);
                continue;
            }
            // A match with no host is one whose log is not a match at all. A started
            // one is listed now — `A started match is listed` — because the dashboard shows
            // what is going on and not only what is on offer.
            if view.host().is_none() {
                continue;
            }
            // `A match is not listed to a player barred from it`: not greyed out, not
            // shown as full. Not shown.
            if let Some(account) = asking
                && view.is_barred(account)
            {
                continue;
            }
            let mine = asking.is_some_and(|account| {
                view.seat_of_account(account)
                    .is_some_and(|seat| !view.has_left(seat))
            });
            listed.push(Listed {
                host: view
                    .host()
                    .map(|seat| view.name_of(seat).to_string())
                    .unwrap_or_default(),
                players: view.seats.len(),
                ranked: view.ranked(),
                joinable: self.why_not_joinable(&match_id, &view, asking).is_none(),
                mine,
                match_id,
            });
        }
        listed.sort_by(|left, right| left.match_id.cmp(&right.match_id));
        listed
    }

    /// The match this account holds a seat in, if any.
    ///
    /// *Holds* rather than *has ever joined*: a seat that left is not held, and a match
    /// that is over holds nobody — otherwise finishing a game would lock somebody out of
    /// the next one until they remembered to leave the old one, which is a worse trap than
    /// the one this closes.
    ///
    /// The log is the only thing that can answer this. The connection table knows who is
    /// connected, and a seat is held whether or not anybody is attached to it.
    pub fn match_of(&self, account: &Account) -> Option<MatchId> {
        let text = protocol::account_text(account);
        for match_id in self.log.matches_of(&text) {
            let view = MatchView::fold(&self.log.read(&match_id).events);
            if view.over() {
                continue;
            }
            if let Some(seat) = view.seat_of_account(account)
                && !view.has_left(seat)
            {
                return Some(match_id);
            }
        }
        None
    }

    /// Why this account cannot join this match, or `None` if it can.
    ///
    /// The one answer to that question. `join` returns what this returns and the listing
    /// sets `joinable` from whether it returned anything, so the flag in the dashboard *is*
    /// the refusal rather than a second opinion about it — which is the only version of
    /// this that cannot drift.
    ///
    /// The colour guard is deliberately not here. A colour is part of a request and not a
    /// property of a match, so there is nothing for a listing to say about it: a listing
    /// promises the match will take you, not that it will take you in green.
    fn why_not_joinable(
        &self,
        match_id: &MatchId,
        view: &MatchView,
        asking: Option<&Account>,
    ) -> Option<Refused> {
        if view.over() {
            return Some(Refused::MatchIsOver);
        }
        if view.host().is_none() {
            return Some(Refused::NoSuchMatch);
        }
        if let Some(account) = asking {
            // Already here, so this is a return rather than a join and none of the guards
            // below apply: `A player holds one seat in a match`, and the one they hold is
            // this one.
            if view
                .seat_of_account(account)
                .is_some_and(|seat| !view.has_left(seat))
            {
                return None;
            }
            if view.is_barred(account) {
                return Some(Refused::Barred);
            }
            if let Some(elsewhere) = self.match_of(account)
                && &elsewhere != match_id
            {
                return Some(Refused::AlreadyInAMatch {
                    match_id: elsewhere,
                });
            }
        }
        if view.started {
            return Some(Refused::AlreadyStarted);
        }
        if view.seats.len() >= MAX_PLAYERS {
            return Some(Refused::MatchIsFull);
        }
        None
    }

    pub fn view(&self, match_id: &MatchId) -> MatchView {
        MatchView::fold(&self.log.read(match_id).events)
    }

    /// Serve one message: decide it against the log, and commit it to the log it was
    /// decided against.
    ///
    /// The loop is the concurrency design. Every handler reads the log, decides from what
    /// it read, and appends at the length it saw; if something else appended first the
    /// append is refused, the decision is discarded, and the whole message is decided
    /// again against what is now there. See `The position is the version` in
    /// /implementation/relay.md.
    /// `now` is the one clock reading this message gets, read by the composition root and
    /// passed in. The relay holds no clock: `The relay holds no game state between
    /// messages except what the log holds` stays literally true, and a test controls time
    /// by passing a number instead of sleeping.
    pub fn receive(&mut self, from: &ConnectionId, message: Message, now: Moment) -> Outcome {
        for _ in 0..MAX_ATTEMPTS {
            match self.attempt(from, message.clone(), now) {
                Ok(outcome) => return outcome,
                Err(Stale) => continue,
            }
        }
        panic!("gave up deciding a message after {MAX_ATTEMPTS} attempts; nothing was written");
    }

    /// Read the log, decide, and commit — once.
    fn attempt(
        &mut self,
        from: &ConnectionId,
        message: Message,
        now: Moment,
    ) -> Result<Outcome, Stale> {
        // `No event is recorded in a match that is over`, in one place rather than in
        // every handler. It refuses a start, a claim, a join and a leave into a dead
        // match with one line; it stops the expiry sweep and the Cancel button from
        // overwriting an ending that is already recorded; and it is what makes the sweep
        // idempotent by construction instead of by a guard somebody has to remember.
        if message.writes()
            && let Some(match_id) = message.about()
            && self.view(match_id).over()
        {
            return Ok(Outcome::refused(Refused::MatchIsOver));
        }
        match message {
            Message::Host {
                who,
                version,
                nonce,
            } => match self.identify(who, now) {
                Ok((account, name)) => self.host(
                    from,
                    Sitting {
                        account,
                        name,
                        version,
                    },
                    nonce,
                    now,
                ),
                Err(why) => Ok(Outcome::refused(why)),
            },
            Message::Join {
                match_id,
                who,
                version,
            } => match self.identify(who, now) {
                Ok((account, name)) => self.join(
                    from,
                    match_id,
                    Sitting {
                        account,
                        name,
                        version,
                    },
                    now,
                ),
                Err(why) => Ok(Outcome::refused(why)),
            },
            Message::Colour { match_id, colour } => self.recolour(from, match_id, colour, now),
            Message::Bar { match_id, seat } => self.bar(from, match_id, seat, true, now),
            Message::Lift { match_id, seat } => self.bar(from, match_id, seat, false, now),
            Message::Start { match_id } => self.start(from, match_id, now),
            Message::Leave { match_id } => self.leave(from, match_id, now),
            Message::Claim { match_id, event } => self.claim(from, match_id, event, now),
            Message::RoundOver { match_id } => self.round_over(from, match_id, now),
            Message::Over { match_id } => self.match_over(from, match_id, now),
            Message::Cancel { match_id } => self.cancel(from, match_id, now),
            Message::List { who } => match self.identify(who, now) {
                Ok((account, _)) => self.list(&account, now),
                Err(why) => Ok(Outcome::refused(why)),
            },
            Message::Resume { match_id, who } => match self.identify(who, now) {
                Ok((account, _)) => self.resume(from, match_id, account, now),
                Err(why) => Ok(Outcome::refused(why)),
            },
        }
    }

    /// Turn a claim about who somebody is into an account and a name.
    ///
    /// The one place a ticket is checked, and the only way a `Discord` account can come
    /// into existence from a message — so `A match is ranked when every seat in it is
    /// held by an authenticated player` rests on a check that happens here and nowhere
    /// else.
    ///
    /// A guest id is taken as given and nothing looks at it. That is the decision rather
    /// than an oversight: anybody who learns somebody else's guest id can take their seat
    /// in an unranked match, and the whole of the security budget is spent on the one
    /// question a ranked match turns on.
    fn identify(&self, who: Who, now: Moment) -> Result<(Account, String), Refused> {
        match who {
            Who::Guest { id, name } => Ok((Account::Guest(id), name)),
            Who::Authenticated { ticket } => {
                // No ticket secret, so nothing can be proved and nothing is guessed at.
                // A deployment in this state serves guests and refuses sign-ins, which is
                // the same answer the exchange gives when it cannot mint.
                let secret = self.secrets.ticket.as_ref().ok_or(Refused::BadTicket)?;
                // Seconds, from the one reading this message was given. The relay never
                // calls Discord: an outbound call on the hot path would turn a Discord
                // outage into a relay outage, and there is nothing it could learn that
                // the ticket does not already carry.
                let seconds = now.to_unix_nanos() / 1_000_000_000;
                let proved =
                    integrity::check_ticket(secret, &ticket, seconds).ok_or(Refused::BadTicket)?;
                let account =
                    protocol::account_from_text(&proved.account).ok_or(Refused::BadTicket)?;
                // A ticket may only ever prove an account that counts as proven. Nothing
                // mints one for a guest, so this is unreachable — and it is what makes
                // `Who::Authenticated` yield a proven account by construction rather than
                // by the exchange having been careful.
                if !account.is_proven() {
                    return Err(Refused::BadTicket);
                }
                Ok((account, proved.name))
            }
        }
    }

    /// The match's log as it is now, ready to be decided against.
    fn open(&self, match_id: &MatchId, now: Moment) -> Working {
        let Recording { events, last } = self.log.read(match_id);
        let at = events.len() as u64;
        Working {
            match_id: match_id.clone(),
            at,
            events,
            added: at as usize,
            last,
            now,
        }
    }

    /// Append what the message added, at the position it was decided against.
    fn commit(&mut self, work: &Working) -> Result<(), Stale> {
        if work.added().is_empty() {
            return Ok(());
        }
        self.log
            .append(&work.match_id, work.at, work.added(), work.now)
            .map_err(|Taken| Stale)
    }

    /// A connection went away. Its seat is absent until it returns.
    pub fn disconnect(&mut self, connection: &ConnectionId) {
        self.connections.detach(connection);
    }

    fn host(
        &mut self,
        from: &ConnectionId,
        sitting: Sitting,
        nonce: String,
        now: Moment,
    ) -> Result<Outcome, Stale> {
        let Sitting {
            account,
            name,
            version,
        } = sitting;
        if version != PROTOCOL_VERSION {
            return Ok(Outcome::refused(Refused::VersionMismatch {
                match_version: PROTOCOL_VERSION,
                yours: version,
            }));
        }
        // Derived, not counted. Nothing is read to name a match, so hosting scans no
        // table and has no window between choosing a name and claiming it.
        let match_id = MatchId(integrity::name_for(
            &self.secrets.dice,
            &protocol::account_text(&account),
            &nonce,
        ));
        // `A player cannot host a match while holding a seat in another match`. Checked
        // after the id is derived rather than before, so that `A repeated request to host
        // creates no second match` still holds: a host sending the same request twice is
        // in a match, and the match they are in is the one they are asking for.
        if let Some(elsewhere) = self.match_of(&account)
            && elsewhere != match_id
        {
            return Ok(Outcome::refused(Refused::AlreadyInAMatch {
                match_id: elsewhere,
            }));
        }
        let seat = Seat(0);
        // `fresh` rather than `open`: a new match is a claim on position zero, so a name
        // that is already taken refuses the whole message instead of appending these
        // three events to the end of whatever is there.
        let mut work = Working::fresh(match_id.clone(), now);
        work.push(Event::Version { protocol: version });
        work.push(Event::Join {
            seat,
            name,
            // The first seat of a new match, so nothing is taken and this is the first
            // colour in the assignment order. Expected to be changed in the lobby, which
            // is the only screen that can show what is free.
            colour: given_colour(&[]),
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
            if self.open(&match_id, now).view().seat_of_account(&account) != Some(seat) {
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
            .log
            .read(&match_id)
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

    fn join(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        sitting: Sitting,
        now: Moment,
    ) -> Result<Outcome, Stale> {
        let Sitting {
            account,
            name,
            version,
        } = sitting;
        let mut work = self.open(&match_id, now);
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
        // Every guard, from the same function the listing asks. The version is above
        // because it is a property of the client rather than of the match, and a listing
        // produced by this relay always matches it.
        if let Some(why) = self.why_not_joinable(&match_id, &view, Some(&account)) {
            return Ok(Outcome::refused(why));
        }
        // Already here, so this is a return and not a join. `A player holds one seat in
        // a match`: answering with the seat they have is the same answer `host` gives to a
        // repeated host, and it is what `Resume` does, so a client that reloaded is caught
        // up rather than seated twice.
        //
        // Before the guards below on purpose. Every one of them is about taking a seat:
        // a match that has started is one they are already in, and the match being full
        // counts them.
        if view.seat_of_account(&account).is_some() {
            return self.resume(from, match_id, account, now);
        }
        // Every guard above read the same snapshot this event is appended against, so the
        // seat, the colour, the player count and the started flag are all decided by the
        // commit rather than merely consulted before it.
        let seat = Seat(view.seats.len() as u8);
        let event = Event::Join {
            seat,
            name,
            colour: given_colour(&taken_colours(&view)),
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

    /// `A player can change their seat's colour until the match starts`.
    fn recolour(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        colour: String,
        now: Moment,
    ) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id, now);
        let view = work.view();
        let Some(mine) = self.seat_of(&match_id, from) else {
            return Ok(Outcome::refused(Refused::NotInThisMatch));
        };
        if view.started {
            return Ok(Outcome::refused(Refused::AlreadyStarted));
        }
        // *Another* seat. Re-choosing the colour already held is not a collision with
        // oneself, and answering a repeat with success is what `host` and `join` do.
        if view
            .seats
            .iter()
            .any(|seat| seat.seat != mine && seat.colour == colour)
        {
            return Ok(Outcome::refused(Refused::ColourTaken));
        }
        let event = Event::Colour { seat: mine, colour };
        work.push(event.clone());
        self.commit(&work)?;
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
        now: Moment,
    ) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id, now);
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

    fn start(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        now: Moment,
    ) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id, now);
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
            &integrity::seed_for(&self.secrets.dice, &match_id.0, ORDER_ROUND),
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
        now: Moment,
    ) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id, now);
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
    fn round_over(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        now: Moment,
    ) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id, now);
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
        let waiting_on = self.present_seats(&match_id, &view).len();
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
    fn leave(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        now: Moment,
    ) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id, now);
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
        // `A host who leaves a match that has not started cancels it`. Host powers are the
        // seat in the log and nothing transfers them, so there is no start left to wait
        // for — which is the same fact the Cancel button records, and so the same event.
        // It used to be its own refusal, `HostHasLeft`, which was a second name for *this
        // match is finished*.
        //
        // Only before the start. A host who leaves a match in progress is one player
        // leaving; the others play on, and the rules read the departure as an elimination.
        let view = work.view();
        let cancelled = !view.started && view.host() == Some(seat);
        if cancelled {
            work.push(Event::Cancel { seat });
        }
        self.commit(&work)?;
        // The socket goes too. A seat that has left holds no connection, so nothing is
        // sent to it and nothing waits for it.
        let mut sends = self.to_everyone(&match_id, event);
        if cancelled {
            // Before the detach, so whoever was waiting in the match is told it is over
            // rather than left watching a dashboard entry that has already gone.
            sends.extend(self.to_everyone(&match_id, Event::Cancel { seat }));
        }
        self.connections.detach(from);
        Ok(Outcome {
            refused: None,
            sends,
            created: None,
            listed: None,
        })
    }

    /// Which distinct seats are present and have not left.
    ///
    /// Seats, not rows. The counts it is compared against — `round_over_count`, and the
    /// seats that have said the match is over — count distinct seats and can never exceed
    /// the number of them, so counting rows against either compares two different kinds of
    /// thing: one duplicated row would lift this above a number it cannot reach and the
    /// round would never reveal. One row per seat makes the two counts equal today — see
    /// `relay-service.md`, the place table — and naming the right one keeps this correct
    /// when a place can be a watcher, since a watcher is sent the round and never
    /// acknowledges it.
    ///
    /// One function because two callers now need it, and two copies of *who are we waiting
    /// for* would be two answers to one question.
    fn present_seats(&self, match_id: &MatchId, view: &MatchView) -> Vec<Seat> {
        let mut present: Vec<Seat> = Vec::new();
        for row in self.connections.of(match_id) {
            let Role::Player = row.role;
            if !view.has_left(row.seat) && !present.contains(&row.seat) {
                present.push(row.seat);
            }
        }
        present
    }

    /// A seat says the match is over. When every present seat has, it is recorded.
    ///
    /// `A match is complete when every seat present in it says so`. The relay writes no
    /// resolution event — every `Winner` in a log comes from a desktop game — so it cannot
    /// derive the ending and must not try, because that would mean knowing what
    /// elimination means. Counted, exactly as `round_over` counts: `a count of messages,
    /// not a judgement`.
    ///
    /// Counting rather than taking the first client's word, because the rules are
    /// deterministic and every client derives the same ending from the same events, so a
    /// seat that disagrees is lying or broken. One word would let a losing player end a
    /// match; counting costs only a delay when somebody is absent, and the hour collects
    /// those.
    fn match_over(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        now: Moment,
    ) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id, now);
        let Some(seat) = self.seat_of(&match_id, from) else {
            return Ok(Outcome::refused(Refused::NotInThisMatch));
        };
        let event = Event::MatchOver { seat };
        work.push(event.clone());
        let mut sends = self.to_everyone(&match_id, event);

        // From the working copy, so this seat's own word — pushed a line ago and not yet
        // in storage — is counted. Reading the log here would count every seat but the one
        // that just spoke.
        let view = work.view();
        let present = self.present_seats(&match_id, &view);
        // Never on an empty set. `seat_of` found this connection a seat, so it is present,
        // and `all` over nothing would otherwise complete a match nobody is in.
        if !present.is_empty() && present.iter().all(|seat| view.has_finished(*seat)) {
            work.push(Event::Complete);
            sends.extend(self.to_everyone(&match_id, Event::Complete));
        }
        self.commit(&work)?;
        Ok(Outcome {
            refused: None,
            sends,
            created: None,
            listed: None,
        })
    }

    /// The host ends the match.
    ///
    /// `Only the host can cancel a match`, and the gate is the match's inactivity: the
    /// time since the last event the relay recorded in it. Inactivity is the match's and
    /// not the current player's, which coincide in every case anybody will meet — only the
    /// player whose turn it is can produce an event — except that another seat's
    /// `round-over` or `leave` resets it. The gain is one subtraction on one number, with
    /// no judgement about whose silence it is.
    fn cancel(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        now: Moment,
    ) -> Result<Outcome, Stale> {
        let mut work = self.open(&match_id, now);
        let view = work.view();
        let seat = match self.seat_of(&match_id, from) {
            Some(mine) if Some(mine) == view.host() => mine,
            Some(_) => return Ok(Outcome::refused(Refused::NotTheHost)),
            None => return Ok(Outcome::refused(Refused::NotInThisMatch)),
        };
        // `The host can cancel a match that has not started` — at once. There is no turn
        // to wait on, and a minute of the host watching a timer for no reason is a minute
        // of them wondering what is broken.
        if view.started {
            let idle = idle_seconds(work.last, now);
            if idle < CANCEL_AFTER_SECONDS {
                // `idle` is already whole seconds rounded down, so subtracting it gives
                // the ceiling of what remains: 59.2 seconds elapsed says one, not zero.
                // A refused cancel records nothing, so asking again does not reset the
                // minute it is waiting for — otherwise a host who presses twice never
                // gets there, which looks exactly like the feature not working.
                return Ok(Outcome::refused(Refused::StillActive {
                    seconds: (CANCEL_AFTER_SECONDS - idle) as u32,
                }));
            }
        }
        let event = Event::Cancel { seat };
        work.push(event.clone());
        self.commit(&work)?;
        Ok(Outcome {
            refused: None,
            sends: self.to_everyone(&match_id, event),
            created: None,
            listed: None,
        })
    }

    /// What is open to join.
    ///
    /// Reads everything, which is the reason the dashboard has a cost worth measuring — see
    /// `Measuring what the dashboard costs` in `relay-service.md`. There is no decision to
    /// invalidate, so it cannot be stale: a listing is true of the moment it was read and
    /// a match may fill before anybody acts on it, which is what the refusals on `join`
    /// are for.
    ///
    /// It used to write nothing. It now writes the abandonment of any match that crossed
    /// the hour while nobody was looking, because there is no monitor and this is the pass
    /// that was going to read them anyway.
    fn list(&mut self, asking: &Account, now: Moment) -> Result<Outcome, Stale> {
        Ok(Outcome {
            refused: None,
            sends: Vec::new(),
            created: None,
            listed: Some(self.open_matches(Some(asking), now)),
        })
    }

    /// Reattach to a seat. The one message that writes nothing.
    ///
    /// Takes `now` and decides nothing with it. Every message gets one clock reading, and
    /// a handler that happens not to need it is not a reason to have two shapes of
    /// handler. It works on a match that is over, which is the whole reason the guard in
    /// `attempt` asks whether a message writes rather than refusing everything.
    fn resume(
        &mut self,
        from: &ConnectionId,
        match_id: MatchId,
        account: Account,
        now: Moment,
    ) -> Result<Outcome, Stale> {
        let work = self.open(&match_id, now);
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
        let seed = integrity::seed_for(&self.secrets.dice, &match_id.0, round);
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
        let seed = integrity::seed_for(&self.secrets.dice, &match_id.0, round);
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
