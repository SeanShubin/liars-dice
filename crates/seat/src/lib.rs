//! One seat at a match, played over a wire. Frames in, frames out, no I/O.
//!
//! Realizes the client half of /implementation/relay-service.md's end-to-end test. The
//! transport is the caller's: `apps/probe` gives this a socket from a terminal and
//! `apps/web` gives it one from a browser, and neither has any logic to get wrong because
//! there is none here to duplicate.
//!
//! `crates/table` is the other arrangement of the same pieces — it owns the relay and
//! every player in one process, which is what lets a whole match run in a unit test. This
//! owns one player and no relay, which is what a real client is.

use ai::Seen;
use coordinator::{Client, Decides, Intent};
use protocol::{Account, Event, PROTOCOL_VERSION, Seat};
use relay::Message;
use rules::{Call, Claim, Resolution, View};
use wire::answer::Answer;
use wire::{Malformed, json, message};

/// A match's name, and why a message was not accepted.
///
/// Re-exported so a client depends on this crate alone. A client does not share a library
/// with the service that routes it — the relay reaches it as frames and nothing else — but
/// it has to be able to name the match it is joining and to say why it was turned away,
/// and both of those are the protocol rather than the service.
pub use relay::{Ending, Listed, MatchId, Refused};

/// Why a seat stopped.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Stopped {
    /// The relay refused something. None of these are game rules.
    Refused(Refused),
    /// A frame arrived that this build cannot read.
    Unreadable(String),
    /// The match ended.
    Over,
}

impl std::fmt::Display for Stopped {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stopped::Refused(why) => write!(out, "the relay refused the message: {why:?}"),
            Stopped::Unreadable(why) => write!(out, "unreadable frame: {why}"),
            Stopped::Over => write!(out, "the match is over"),
        }
    }
}

impl std::error::Error for Stopped {}

/// A round's outcome, kept so it can be drawn after the match has moved past it.
///
/// The four things a verdict is made of, captured at the reveal. Nothing here is read back
/// out of the game later, because the game does not stand still: the next round opens the
/// moment every seat acknowledges this one, and whoever opens it may claim immediately. A
/// screen that held the game back to show an outcome would apply that claim to the round it
/// was still showing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Showing {
    pub resolution: Resolution,
    /// Who ended the round.
    pub by: Seat,
    /// What they called. Never a raise: a raise continues a round.
    pub call: Call,
    /// The claim they disputed.
    pub claim: Claim,
}

/// A player at the match, as the log says. Indexed by seat.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Player {
    pub name: String,
    /// The colour they chose, by name. Distinct within a match, because the relay refuses
    /// one another seat has taken.
    pub colour: String,
    pub left: bool,
}

impl Player {
    /// A seat in the roster that no join has been seen for.
    ///
    /// Reachable only if a later seat's join arrived before an earlier seat's, which the
    /// ordered log rules out. It exists so that a gap cannot be a panic.
    fn nobody() -> Self {
        Self {
            name: "nobody".to_string(),
            colour: String::new(),
            left: false,
        }
    }
}

/// What this seat is here to do.
enum Opening {
    Host { nonce: String },
    Join { match_id: MatchId },
}

/// One player, waiting on frames.
pub struct Seated {
    account: Account,
    name: String,
    colour: String,
    opening: Opening,
    match_id: Option<MatchId>,
    /// The seat this player is on, learned from the `join` that names its own account.
    mine: Option<Seat>,
    client: Option<Client>,
    /// Events that arrived before the seat was known.
    ///
    /// `Client::new` takes a seat, and a joiner does not learn its own until the relay
    /// assigns one and says so in a `join`. Everything before that is kept here and
    /// replayed in order once the client exists, which is exact because the log is
    /// ordered — that being the whole point of the conditional write.
    waiting: Vec<Event>,
    driver: Box<dyn Decides>,
    /// A claim this seat sent that has not come back yet.
    ///
    /// `Client::act` deliberately does not apply the call locally — every client applies
    /// the same ordered stream, so a claim takes effect when the relay sends it back.
    /// Which means that between sending one and seeing it, the game still says it is this
    /// seat's turn. `crates/table` never notices, because it delivers a claim
    /// synchronously in the same loop that made it; over a socket there is a real gap,
    /// and a seat that keeps answering during it proposes the same call again and again.
    ///
    /// If a claim is genuinely lost the seat waits rather than guessing, which is what
    /// `Resume` is for.
    awaiting: bool,
    /// The last round's outcome, for drawing. See [`Showing`].
    showing: Option<Showing>,
    over: bool,
    /// How the match ended, once the relay has said so.
    ///
    /// Separate from `over`, which this seat sets for itself the moment the rules say the
    /// match is finished. This is the relay's word, and it is the only thing that can say
    /// the match was *cancelled* rather than won — a screen that read `over` alone would
    /// present a winner that was never recorded.
    ending: Option<Ending>,
    /// Seconds left before the host may cancel, from the last refusal that said so.
    ///
    /// Not a `Stopped`. A refusal of the Cancel button is an answer to the button, not a
    /// failure of the seat, and treating it as one would drop the host out of the match
    /// they were trying to end.
    still_active: Option<u32>,
}

impl Seated {
    pub fn hosting(
        account: Account,
        name: impl Into<String>,
        colour: impl Into<String>,
        nonce: impl Into<String>,
        driver: Box<dyn Decides>,
    ) -> Self {
        Self::new(
            account,
            name,
            colour,
            Opening::Host {
                nonce: nonce.into(),
            },
            driver,
        )
    }

    pub fn joining(
        account: Account,
        name: impl Into<String>,
        colour: impl Into<String>,
        match_id: MatchId,
        driver: Box<dyn Decides>,
    ) -> Self {
        Self::new(account, name, colour, Opening::Join { match_id }, driver)
    }

    fn new(
        account: Account,
        name: impl Into<String>,
        colour: impl Into<String>,
        opening: Opening,
        driver: Box<dyn Decides>,
    ) -> Self {
        let match_id = match &opening {
            Opening::Host { .. } => None,
            Opening::Join { match_id } => Some(match_id.clone()),
        };
        Self {
            account,
            name: name.into(),
            colour: colour.into(),
            opening,
            match_id,
            mine: None,
            client: None,
            waiting: Vec::new(),
            driver,
            awaiting: false,
            showing: None,
            over: false,
            ending: None,
            still_active: None,
        }
    }

    /// The first frame to put on the socket.
    pub fn opening(&self) -> String {
        let message = match &self.opening {
            Opening::Host { nonce } => Message::Host {
                account: self.account.clone(),
                name: self.name.clone(),
                colour: self.colour.clone(),
                version: PROTOCOL_VERSION,
                nonce: nonce.clone(),
            },
            Opening::Join { match_id } => Message::Join {
                match_id: match_id.clone(),
                account: self.account.clone(),
                name: self.name.clone(),
                colour: self.colour.clone(),
                version: PROTOCOL_VERSION,
            },
        };
        json::write(&message::shape_of(&message))
    }

    /// The match this seat is at, once it is known. A host learns it from `created`.
    pub fn match_id(&self) -> Option<&MatchId> {
        self.match_id.as_ref()
    }

    pub fn seat(&self) -> Option<Seat> {
        self.mine
    }

    pub fn over(&self) -> bool {
        self.over
    }

    /// How the match ended, if the relay has said.
    pub fn ending(&self) -> Option<Ending> {
        self.ending
    }

    /// How many seconds remain before the host may cancel, if a refusal said so.
    ///
    /// Taken rather than read, because it answers one press of one button. A caller that
    /// wants to keep counting down holds the number itself and runs its own clock over it,
    /// which is display and decides nothing.
    pub fn take_still_active(&mut self) -> Option<u32> {
        self.still_active.take()
    }

    /// Ask the relay to end the match.
    ///
    /// `Only the host can cancel a match`, and the relay says so if this is not the host.
    /// A started match that has been active within the minute is refused with the time
    /// remaining, which arrives at [`Seated::take_still_active`].
    pub fn cancel(&self) -> Option<String> {
        let match_id = self.match_id.clone()?;
        Some(json::write(&message::shape_of(&Message::Cancel {
            match_id,
        })))
    }

    /// What arrived that should not have.
    ///
    /// The client checks every round's reveal against its commitment and every deal
    /// against the seed it was supposedly dealt from, so an empty list is the integrity
    /// scheme verified from the event stream alone — no secret and no storage needed,
    /// which is what `Verification requires nothing beyond the event log` means.
    pub fn complaints(&self) -> &[coordinator::Wrong] {
        match &self.client {
            Some(client) => client.complaints(),
            None => &[],
        }
    }

    /// Whether this seat hosted the match, and so may start it.
    ///
    /// Read from the log rather than from how this seat was opened. The two agree today
    /// and would not after a resume, where a client reattaches to a seat it did not open
    /// in this session and the log is the only thing that still remembers.
    pub fn host(&self) -> bool {
        self.stream()
            .iter()
            .any(|event| matches!(event, Event::Host { seat } if Some(*seat) == self.mine))
    }

    /// How many players have taken a seat, which is how a host knows when to start.
    pub fn seated(&self) -> usize {
        self.stream()
            .iter()
            .filter(|event| matches!(event, Event::Join { .. }))
            .count()
    }

    /// This seat's own view of the match, which is what a screen draws from. `None` until
    /// the match has started.
    ///
    /// A view returns `None` for another seat's dice until the round is revealed, so a
    /// panel handed one cannot draw what this seat may not see. That is a type fact rather
    /// than a convention, and it is why a screen is given this and never a `Game`.
    pub fn view(&self) -> Option<View<'_>> {
        self.client.as_ref()?.view()
    }

    /// The last round's outcome, kept for drawing. `None` until a round has ended.
    pub fn showing(&self) -> Option<&Showing> {
        self.showing.as_ref()
    }

    /// Who is at the match, in seat order.
    ///
    /// Folded from the stream rather than remembered, because the log already says it and
    /// a second copy is a second thing to keep in step. A `Join` is the only event that
    /// carries a name, so this is the one place a screen can learn them.
    pub fn roster(&self) -> Vec<Player> {
        let mut seats: Vec<Player> = Vec::new();
        for event in self.stream() {
            match event {
                Event::Join {
                    seat, name, colour, ..
                } => {
                    let index = seat.0 as usize;
                    if seats.len() <= index {
                        seats.resize_with(index + 1, Player::nobody);
                    }
                    seats[index] = Player {
                        name: name.clone(),
                        colour: colour.clone(),
                        left: false,
                    };
                }
                Event::Left { seat } => {
                    if let Some(player) = seats.get_mut(seat.0 as usize) {
                        player.left = true;
                    }
                }
                _ => {}
            }
        }
        seats
    }

    /// Every event this seat has been told about, for checking a run against the log.
    pub fn stream(&self) -> &[Event] {
        match &self.client {
            Some(client) => client.stream(),
            None => &[],
        }
    }

    /// Leave the match for good.
    ///
    /// Available whenever this seat is in a match, which includes after elimination —
    /// that being the case it exists for. The rules read the departure as an elimination,
    /// so leaving while still holding dice forfeits them.
    pub fn leave(&self) -> Option<String> {
        let match_id = self.match_id.clone()?;
        Some(json::write(&message::shape_of(&Message::Leave {
            match_id,
        })))
    }

    /// Whether this seat still holds dice, which is what decides whether it is playing or
    /// watching. `None` until the match has started.
    pub fn playing(&self) -> Option<bool> {
        let client = self.client.as_ref()?;
        let public = client.public()?;
        Some(!public.eliminated(client.seat().0 as usize))
    }

    /// Start the match. Only the host may, and the relay says so if it is not.
    pub fn start(&self) -> Option<String> {
        let match_id = self.match_id.clone()?;
        Some(json::write(&message::shape_of(&Message::Start {
            match_id,
        })))
    }

    /// Ask the driver again, without a frame having arrived.
    ///
    /// A seat played by an AI answers inside [`Seated::receive`], because its answer is
    /// ready the moment the event asking for it is. A person is not: they look at the
    /// screen and decide in their own time, by which point the frame that made it their
    /// turn is long gone. So a frame loop calls this every frame, and the turn taking is
    /// the same code either way — there is no second path for a human seat to drift out of
    /// step with.
    ///
    /// Cheap when there is nothing to do: `act` returns before touching the driver unless
    /// it really is this seat's turn, and a driver a person has not touched answers `None`.
    pub fn offer(&mut self) -> Result<Vec<String>, Stopped> {
        if self.over {
            return Ok(Vec::new());
        }
        Ok(self.act()?.into_iter().collect())
    }

    /// Take one frame from the socket; give back the frames to send in answer.
    pub fn receive(&mut self, frame: &str) -> Result<Vec<String>, Stopped> {
        let shape = json::read(frame).map_err(unreadable)?;
        let answer = wire::answer::from_shape(&shape).map_err(unreadable)?;
        match answer {
            // The one refusal that is not a stop: it answers the Cancel button, and the
            // seat carries on exactly as it was.
            Answer::Refused(Refused::StillActive { seconds }) => {
                self.still_active = Some(seconds);
                Ok(Vec::new())
            }
            Answer::Refused(why) => Err(Stopped::Refused(why)),
            Answer::Created(match_id) => {
                self.match_id = Some(match_id);
                Ok(Vec::new())
            }
            // A listing is the lobby's answer and says nothing about this seat. One can
            // still arrive here, because the socket that asked what was open is the socket
            // that went on to sit down, and a frame in flight does not know that happened.
            // `lobby::listing` is where a caller reads it.
            Answer::Listing(_) => Ok(Vec::new()),
            Answer::Event(event) => self.apply(event),
        }
    }

    fn apply(&mut self, event: Event) -> Result<Vec<String>, Stopped> {
        // The three endings, ahead of everything else and ahead of the `over` guard in
        // `feed`. They are the relay's word that the match has stopped, and they have to
        // land even on a seat that already worked the result out for itself — otherwise
        // the screen cannot tell *the host ended this* from *somebody won*.
        if let Some(ending) = ending_of(&event) {
            self.over = true;
            self.ending = Some(ending);
            return Ok(Vec::new());
        }
        // A join naming this account is how a seat finds out which seat it is.
        if self.mine.is_none()
            && let Event::Join { seat, account, .. } = &event
            && account == &self.account
        {
            self.mine = Some(*seat);
            self.client = Some(Client::new(*seat));
            let waiting = std::mem::take(&mut self.waiting);
            let mut frames = Vec::new();
            for earlier in waiting {
                frames.extend(self.feed(earlier)?);
            }
            frames.extend(self.feed(event)?);
            return Ok(frames);
        }
        if self.client.is_none() {
            self.waiting.push(event);
            return Ok(Vec::new());
        }
        self.feed(event)
    }

    /// Hand one event to the client, then let it act if it is its turn.
    fn feed(&mut self, event: Event) -> Result<Vec<String>, Stopped> {
        // The relay does not know a match has ended — deciding that needs the rules,
        // which it does not have — so it opens the next round regardless and goes quiet
        // only when nobody answers. Being that silence is this seat's job. Without this
        // the seat plays a round that no longer exists, because the commitment and the
        // deals for it arrive in the same message as the reveal that ended the match.
        if self.over {
            return Ok(Vec::new());
        }
        let mut frames = Vec::new();
        // This seat's own claim, come back from the relay. Whatever it was waiting for
        // has arrived, so it may think again.
        if claimed_by(&event) == self.mine {
            self.awaiting = false;
        }
        let intents = match &mut self.client {
            Some(client) => client.receive(&event),
            None => Vec::new(),
        };
        for intent in intents {
            frames.push(self.frame_for(intent)?);
        }

        // A round is settled by its reveal, and only by its reveal.
        //
        // Asking after every event looks harmless and is not. The reveal's own deals
        // arrive just before the `Reveal` that carries the seed, so on the last of them
        // the round's dice are all present and the round resolves one event early — and
        // from then on the client's `revealed` flag belongs to the round after the one
        // the seed is for, so every later deal is queued for a round that has already
        // begun instead of being installed in it. The match then resolves rounds against
        // dice from the round before, which is how one ended twice.
        //
        // `crates/table` avoids this by delivering a whole batch before settling
        // anything; a seat on a socket has no batch to end, so it waits for the reveal.
        if matches!(event, Event::Reveal { .. })
            && let Some(client) = &mut self.client
            && let Some(resolution) = client.settle()
        {
            // Copied out before the round is left behind. `ended_by` is cleared by
            // `begin_next_round`, and the claim that was disputed is the last entry of a
            // history the next round forgets, so after that call neither is readable
            // anywhere.
            self.showing = client.ended_by().and_then(|(by, call)| {
                Some(Showing {
                    resolution,
                    by,
                    call,
                    claim: client.public()?.history().last()?.claim,
                })
            });
            if resolution.over {
                self.over = true;
                // And the relay is told. It writes no resolution event and may not derive
                // one, so `A match is complete when every seat present in it says so` is
                // the only way it can learn that this match is finished — and until it
                // does, the match sits in the lobby in everybody's way.
                if let Some(match_id) = self.match_id.clone() {
                    frames.push(json::write(&message::shape_of(&Message::Over { match_id })));
                }
                return Ok(frames);
            }
            client.begin_next_round(resolution.opens);
        }

        if let Some(frame) = self.act()? {
            frames.push(frame);
        }
        Ok(frames)
    }

    /// Ask the driver for a call, if it is this seat's turn and it has one.
    fn act(&mut self) -> Result<Option<String>, Stopped> {
        let Some(client) = &mut self.client else {
            return Ok(None);
        };
        // A round that has ended is not a round to act in, even though the game may still
        // say it is this seat's turn: the dice that settle it have not arrived, so nothing
        // has moved on yet. `ended_by` is set when the round ends and cleared by
        // `begin_next_round`, which makes it exactly the window to sit out.
        if client.ended_by().is_some() {
            return Ok(None);
        }
        if !client.is_my_turn() {
            return Ok(None);
        }
        // And a claim already sent is a turn already taken, whatever the game says until
        // it comes back.
        if self.awaiting {
            return Ok(None);
        }
        // The view borrows the client and the driver is a sibling field, so the choice is
        // made in its own scope and the view is gone before the client is asked to act.
        let Some((call, odds)) = ({
            match client.view() {
                Some(view) => self
                    .driver
                    .choose(&view)
                    .map(|call| (call, odds_of(&view, call))),
                None => None,
            }
        }) else {
            return Ok(None);
        };
        let Some(client) = &mut self.client else {
            return Ok(None);
        };
        // A rejected call is the client's own rules refusing it, which means the driver
        // asked for something illegal. Nothing to send and nothing to say to the relay.
        let Ok(intent) = client.act(call, odds) else {
            return Ok(None);
        };
        if matches!(intent, Intent::Claim(_)) {
            self.awaiting = true;
        }
        self.frame_for(intent).map(Some)
    }

    fn frame_for(&self, intent: Intent) -> Result<String, Stopped> {
        let match_id = self.match_id.clone().ok_or(Stopped::Over)?;
        let message = match intent {
            Intent::Claim(event) => Message::Claim { match_id, event },
            Intent::RoundOver => Message::RoundOver { match_id },
        };
        Ok(json::write(&message::shape_of(&message)))
    }
}

/// Which ending an event is, if it is one.
///
/// The three terminal events and nothing else. A `match` rather than a chain of `if`s so
/// that `Ending` gaining a case is a compile error here.
fn ending_of(event: &Event) -> Option<Ending> {
    match event {
        Event::Complete => Some(Ending::Complete),
        Event::Cancel { seat } => Some(Ending::Cancelled { by: *seat }),
        Event::Abandon => Some(Ending::Abandoned),
        _ => None,
    }
}

/// Which seat made a call, for any of the three kinds of call.
///
/// The relay has the same function, because it too has to know whose claim a message is
/// without knowing what the claim says.
fn claimed_by(event: &Event) -> Option<Seat> {
    match event {
        Event::Claim { seat, .. } | Event::Liar { seat, .. } | Event::SpotOn { seat, .. } => {
            Some(*seat)
        }
        _ => None,
    }
}

fn unreadable(why: Malformed) -> Stopped {
    Stopped::Unreadable(why.to_string())
}

/// The percentage the log records for a call.
///
/// `ai::Seen::chance_of_call` decides which chance a call is asking about; this only
/// rounds it, which is why the two live apart — `crates/ai` may name no floating point
/// type, because what it decides becomes an event.
fn odds_of(view: &rules::View<'_>, call: rules::Call) -> f64 {
    view.chance_of_call(call)
        .map(|chance| readout::percent(&chance))
        .unwrap_or_default()
}

/// The lobby: what is open, before there is a seat to be at.
///
/// Free functions rather than a type, because there is no state here. A lobby is one
/// question and one answer, and the socket it travels on belongs to whoever opened it —
/// which is the same socket a [`Seated`] later speaks on, since a client connects before
/// it knows whether it will host, join, or look first.
pub mod lobby {
    use protocol::Account;
    use relay::{Listed, Message};
    use wire::answer::Answer;
    use wire::{json, message};

    /// The frame that asks what is open.
    pub fn ask(account: &Account) -> String {
        json::write(&message::shape_of(&Message::List {
            account: account.clone(),
        }))
    }

    /// What a frame says about the lobby, or `None` if it is not about the lobby.
    ///
    /// Returns rather than applies, because a listing is true of the moment it was read
    /// and the caller decides what to do about that. An empty listing is `Some(vec![])`
    /// and means nothing is open; `None` means the frame was about something else.
    pub fn listing(frame: &str) -> Option<Vec<Listed>> {
        let shape = json::read(frame).ok()?;
        match wire::answer::from_shape(&shape).ok()? {
            Answer::Listing(open) => Some(open),
            _ => None,
        }
    }
}

/// A seat played by the computer.
pub struct Ai;

impl Decides for Ai {
    fn choose(&mut self, view: &rules::View) -> Option<rules::Call> {
        Some(ai::choose(view))
    }
}
