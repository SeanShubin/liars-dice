//! One seat at a match, played over a wire. Frames in, frames out, no I/O.
//!
//! Realizes the client half of /implementation/relay-service.md's end-to-end test. The
//! transport is the caller's: `apps/probe` gives this a socket from a terminal and
//! `apps/web` will give it one from a browser, and neither has any logic to get wrong
//! because there is none here to duplicate.
//!
//! `crates/table` is the other arrangement of the same pieces — it owns the relay and
//! every player in one process, which is what lets a whole match run in a unit test. This
//! owns one player and no relay, which is what a real client is.

use ai::Seen;
use coordinator::{Client, Decides, Intent};
use protocol::{Account, Event, PROTOCOL_VERSION, Seat};
use relay::{MatchId, Message, Refused};
use wire::answer::Answer;
use wire::{Malformed, json, message};

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
    over: bool,
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
            over: false,
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

    /// How many players have taken a seat, which is how a host knows when to start.
    pub fn seated(&self) -> usize {
        self.stream()
            .iter()
            .filter(|event| matches!(event, Event::Join { .. }))
            .count()
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

    /// Take one frame from the socket; give back the frames to send in answer.
    pub fn receive(&mut self, frame: &str) -> Result<Vec<String>, Stopped> {
        let shape = json::read(frame).map_err(unreadable)?;
        let answer = wire::answer::from_shape(&shape).map_err(unreadable)?;
        match answer {
            Answer::Refused(why) => Err(Stopped::Refused(why)),
            Answer::Created(match_id) => {
                self.match_id = Some(match_id);
                Ok(Vec::new())
            }
            Answer::Event(event) => self.apply(event),
        }
    }

    fn apply(&mut self, event: Event) -> Result<Vec<String>, Stopped> {
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
            if resolution.over {
                self.over = true;
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

/// A seat played by the computer.
pub struct Ai;

impl Decides for Ai {
    fn choose(&mut self, view: &rules::View) -> Option<rules::Call> {
        Some(ai::choose(view))
    }
}
