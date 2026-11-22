//! A networked match in a browser: one socket, one seat, and the presentation the hot seat
//! game already draws with.
//!
//! Composition root. Realizes /implementation/multiplayer-client.md. The seat is
//! `crates/seat`, the state machine behind it is `crates/coordinator`, the panels are
//! `crates/render-minimal`, and what lives here is the socket, the screens, and the
//! person's half of a `Decides`.
//!
//! Two things in this file are not obvious and are the reason it exists.
//!
//! A browser socket is callback driven and a frame loop is not, so they meet through
//! [`Arrivals`] — a queue the callback pushes to and [`Page::update`] drains. Draining in
//! arrival order is draining in log order, which is the only order anything may be applied
//! in.
//!
//! And a networked client may not pause the match. The hot seat game stops at a resolution
//! and waits to be acknowledged, which it can do because it owns time; this does not own
//! time. The next round opens the moment every seat acknowledges the last one, and whoever
//! opens it may claim immediately, so a client that held its game back to show an outcome
//! would apply that claim to the round it was still showing. The outcome is therefore drawn
//! from [`seat::Showing`], a copy, over a match that has already moved on.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use eframe::egui;
use protocol::Who;
use render_minimal::palette::{PALETTE, index_of};
use render_minimal::{
    Seating, calls, dialler, die, history, odds, opening_claim, palette_choice, ranking, standing,
    table, tap, verdict,
};
use rules::{Call, Claim, View};
use seat::{Ending, Listed, MatchId, Player, Seated, Stopped, dashboard};

/// How long another seat may go quiet before the screen says so.
///
/// A client cannot tell an absent player from a slow one: presence is not in the log, by
/// design, and nothing else is coming. So this is a threshold on what has been *heard*,
/// never a claim about who is there, and crossing it changes no rule and sends no message.
const QUIET: f64 = 20.0;

/// How long a socket may carry nothing before this page sends something to keep it.
///
/// API Gateway closes a WebSocket after ten minutes with no traffic, and that quota cannot
/// be raised. A turn-based game is quiet for long stretches — somebody thinking, everybody
/// talking elsewhere — so without this a lull of ten minutes drops every player in the
/// match. The relay was written for this: a frame with no body is "not a message. Nothing
/// to record and nothing to answer, and closing the socket over it would punish a
/// keepalive."
///
/// Four minutes, so two of them can be lost before the timeout is reached.
const KEEPALIVE: f64 = 240.0;

/// What to say when a ticket cannot be read at all.
///
/// Not reachable while this page only ever builds a guest, and here rather than as an
/// `expect` so that the Discord path has to answer for it rather than panicking in front
/// of somebody.
const UNREADABLE_TICKET: &str = "Your sign-in has gone stale. Reload the page and sign in again.";

/// The longest a handle may be, and what may be in one.
///
/// Refused rather than repaired. A handle that is quietly cleaned up is a person quietly
/// becoming a different player, because the cleaned handle keys a different identity than
/// the one they typed.
#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(
        dead_code,
        reason = "read from the address bar, which only a browser has"
    )
)]
const HANDLE_LIMIT: usize = 32;

/// The handle a fragment names, if it names a readable one.
///
/// `#as=bob`. A fragment rather than a query or a path because a fragment is never sent:
/// `A handle is never part of a request`.
#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(
        dead_code,
        reason = "read from the address bar, which only a browser has"
    )
)]
fn handle_in(fragment: &str) -> Option<String> {
    let handle = pairs_of(fragment).find_map(|(key, value)| (key == "as").then_some(value))?;
    clean_handle(&handle)
}

/// A handle, or nothing if what was offered is not one.
#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(
        dead_code,
        reason = "read from the address bar, which only a browser has"
    )
)]
fn clean_handle(raw: &str) -> Option<String> {
    let handle = raw.trim();
    let usable = !handle.is_empty()
        && handle.len() <= HANDLE_LIMIT
        && handle
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    usable.then(|| handle.to_string())
}

/// The same fragment with any `as` taken out, and everything else left alone.
///
/// Per key rather than a blanket, because the two things a fragment might name have
/// opposite sharing rules: a link naming a match is meant to be passed around, and a link
/// naming a player is not.
#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(
        dead_code,
        reason = "read from the address bar, which only a browser has"
    )
)]
fn fragment_without_handle(fragment: &str) -> String {
    pairs_of(fragment)
        .filter(|(key, _)| key != "as")
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// The `key=value` pairs in a fragment, with the leading `#` if there is one ignored.
#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(
        dead_code,
        reason = "read from the address bar, which only a browser has"
    )
)]
fn pairs_of(fragment: &str) -> impl Iterator<Item = (String, String)> + '_ {
    fragment
        .trim_start_matches('#')
        .split('&')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
}

/// Where in storage a handle's belongings go.
///
/// With no handle this is exactly the key used before handles existed, which is
/// `A browser holds one player until another is chosen` — anything else would hand every
/// existing player a new identity on their next reload, and their seat with it.
#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(
        dead_code,
        reason = "read from the address bar, which only a browser has"
    )
)]
fn storage_key(base: &str, handle: Option<&str>) -> String {
    match handle {
        Some(handle) => format!("{base}:{handle}"),
        None => base.to_string(),
    }
}

/// How often the page redraws while a match is live.
///
/// Arrivals wake the frame loop themselves, so this exists only to notice silence: without
/// it the screen could not say that nothing has happened, because nothing happening is not
/// an event.
const TICK: Duration = Duration::from_millis(500);

/// The swatch a colour nobody recognises is drawn in. Unreachable through the picker.
const UNKNOWN_COLOUR: usize = 19;

/// The frames that have arrived, oldest first.
///
/// What to poke when something arrives while nobody is looking.
///
/// `An observation that arrives unasked wakes the program`. egui runs a frame when there is
/// input or when something asked for one, and an answer landing in a cell is neither, so
/// without this a page nobody is touching never looks in the cell.
///
/// A function rather than the `egui::Context` itself, so a test can compose the page over a
/// waker that counts instead of one that paints — `Every program can be composed over a
/// simulated outside world`. There is no default: a waker that might not wake is one
/// nothing can rely on, so every mailbox is handed a real one at construction.
#[derive(Clone)]
struct Waker(Rc<dyn Fn()>);

impl Waker {
    /// The real one. Built by a composition root, which is the only place the context is.
    fn over(context: egui::Context) -> Self {
        Self(Rc::new(move || context.request_repaint()))
    }

    fn wake(&self) {
        (self.0)();
    }
}

/// `Rc<RefCell<_>>` rather than `Arc<Mutex<_>>` because a tab is one thread and a lock
/// nobody contends is a lie about the code.
#[derive(Clone)]
struct Arrivals {
    queue: Rc<RefCell<VecDeque<String>>>,
    waker: Waker,
}

impl Arrivals {
    fn new(waker: Waker) -> Self {
        Self {
            queue: Rc::default(),
            waker,
        }
    }

    /// Pushed to from the socket's message callback, which only a browser has.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(
            dead_code,
            reason = "the only caller outside a test is the browser socket"
        )
    )]
    fn push(&self, frame: String) {
        self.queue.borrow_mut().push_back(frame);
        self.waker.wake();
    }

    fn take(&self) -> Option<String> {
        self.queue.borrow_mut().pop_front()
    }
}

/// What the transport has to say, which is never a relay frame.
///
/// A socket that will not open, or that closes, is not something the protocol has a word
/// for, so it is kept apart from [`Arrivals`] rather than disguised as a frame that failed
/// to parse.
#[derive(Clone)]
struct Mishap {
    trouble: Rc<RefCell<Option<String>>>,
    waker: Waker,
}

impl Mishap {
    fn new(waker: Waker) -> Self {
        Self {
            trouble: Rc::default(),
            waker,
        }
    }

    /// Said by the socket, which only a browser has.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(dead_code, reason = "the only caller is the browser socket")
    )]
    fn say(&self, trouble: impl Into<String>) {
        *self.trouble.borrow_mut() = Some(trouble.into());
        self.waker.wake();
    }

    fn take(&self) -> Option<String> {
        self.trouble.borrow_mut().take()
    }
}

/// A seat a person is playing.
///
/// The screen puts a call in and the seat takes it out, so a person reaches the relay
/// through the same `Decides` trait an AI does and nothing between them branches on which
/// it is.
#[derive(Clone, Default)]
struct Hand(Rc<Cell<Option<Call>>>);

impl coordinator::Decides for Hand {
    fn choose(&mut self, _view: &View) -> Option<Call> {
        self.0.take()
    }
}

/// Somewhere to put a frame. A browser socket is one, and a test double is the other.
trait Wire {
    fn send(&self, frame: &str) -> Result<(), String>;
}

/// What the seat says about itself, which is all the choice of screen depends on.
///
/// Pulled out so [`screen_for`] is a function of four facts rather than of a page: every
/// combination of them can then be checked without reaching the state that produces it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Stance {
    over: bool,
    started: bool,
    my_turn: bool,
    /// Whether this seat still holds dice.
    playing: bool,
}

/// Which screen is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen {
    /// A name, a colour, and a relay. Nothing has been connected to yet.
    SignIn,
    /// Connected and not at a match: what is open, and the two ways to be at one.
    Dashboard,
    /// In a match that has not started.
    Lobby,
    /// This seat's own view, with the dialler.
    Playing,
    /// This seat's own view, with nothing to press.
    Watching,
    /// Another seat has the turn and has gone quiet.
    Halted,
    /// The ranking, and the choice to play again or go.
    Over,
}

/// Which screen to show. The whole of the decision.
///
/// Never stored, because a stored screen is a second place that knows whether this seat
/// holds dice, and the two would disagree.
fn screen_for(connected: bool, stance: Option<Stance>, quiet: bool) -> Screen {
    let Some(stance) = stance else {
        // A socket and no seat is the dashboard. The two are separate because the dashboard is
        // what a client does *before* it knows whether it will host, join, or just look,
        // and that question is asked over the socket it will go on to play through.
        return if connected {
            Screen::Dashboard
        } else {
            Screen::SignIn
        };
    };
    if stance.over {
        return Screen::Over;
    }
    if !stance.started {
        return Screen::Lobby;
    }
    // A seat is never waiting on itself, however long it has sat there: the match is
    // stopped at this screen because nobody has pressed anything on it.
    if !stance.my_turn && quiet {
        return Screen::Halted;
    }
    if stance.playing {
        Screen::Playing
    } else {
        Screen::Watching
    }
}

/// What the lobby's colour picker shows: this seat's colour, and the ones it may not have.
///
/// A free function of the roster rather than four lines inside the frame loop, for the
/// reason `screen_for` is one: a decision inside `ui` can only be tested by running a
/// frame loop, and this one has an off-by-one in it waiting to happen — a seat must not
/// count its own colour as taken, or nobody could ever keep the colour they have.
fn choosable(roster: &[Player], mine: Option<usize>) -> (Option<usize>, Vec<usize>) {
    let chosen = mine
        .and_then(|at| roster.get(at))
        .and_then(|player| index_of(&player.colour));
    let taken = roster
        .iter()
        .enumerate()
        .filter(|(at, _)| Some(*at) != mine)
        .filter_map(|(_, player)| index_of(&player.colour))
        .collect();
    (chosen, taken)
}

/// Whether the call buttons are offered.
///
/// Not the same question as whether a call would be refused, and the difference is the
/// whole reason this exists: a call made out of turn is not refused, it is **kept**.
/// `Seated` asks the driver only when it is this seat's turn, so a call put in the hand
/// early sits there and fires the instant the turn comes round — a mis-click while waiting
/// becoming a committed call a minute later, with nothing on the screen having said so.
///
/// The dialler stays, because dialling commits nothing and thinking ahead is the point of
/// having one.
/// Seconds still to wait, counted down from what the relay last said.
///
/// A free function for the same reason `screen_for` is one: the arithmetic is the part
/// that can be wrong, and it is worth checking at the boundaries rather than reasoning
/// about. Rounds up, so the number here never contradicts the relay's own — which also
/// rounds up — and reaches `None` rather than zero, because zero seconds to go is
/// nothing left to say.
fn remaining(countdown: Option<(u32, f64)>, now: f64) -> Option<u32> {
    let (seconds, at) = countdown?;
    let left = f64::from(seconds) - (now - at);
    if left <= 0.0 {
        None
    } else {
        Some(left.ceil() as u32)
    }
}

fn players(count: usize) -> String {
    match count {
        1 => "1 player".to_string(),
        many => format!("{many} players"),
    }
}

/// Said only when it is true, because *unranked* is the ordinary case and labelling it
/// would make the ordinary case look like a warning.
fn ranked_note(ranked: bool) -> String {
    if ranked {
        "ranked".to_string()
    } else {
        String::new()
    }
}

fn may_call(stance: Stance) -> bool {
    stance.started && !stance.over && stance.playing && stance.my_turn
}

/// The page.
/// A ticket, and what it says, for a page that cannot read one.
///
/// The exchange repeats the name and the expiry outside the ticket because the ticket is
/// opaque here. Not a second source of truth: the relay reads the ticket and neither of
/// these.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Held {
    ticket: String,
    name: String,
    expires_at: u64,
}

/// What the page knows about signing in.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Signin {
    /// The exchange has not answered yet.
    Asking,
    /// Login is off.
    ///
    /// Either the exchange said so, or it could not be reached, or the page carries no
    /// exchange address at all. One state for all three, because the page can do nothing
    /// different about any of them: it offers guest play and says why.
    Off,
    /// Where to send somebody to authorize.
    Offered {
        client_id: String,
        redirect_uri: String,
    },
    /// Signed in, with a ticket to present.
    Held(Held),
}

/// What the exchange said, once a fetch that started earlier has finished.
///
/// The same seam as [`Arrivals`]: a browser callback pushes and the frame loop drains.
/// A fetch answers on its own schedule and a frame loop does not wait, so they meet in a
/// cell rather than by one blocking the other.
#[derive(Clone)]
struct FromExchange {
    answer: Rc<RefCell<Option<Answered>>>,
    waker: Waker,
}

/// One answer from the exchange.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Answered {
    /// The answer to *where do I send somebody*. `None` means login is off.
    Where(Option<(String, String)>),
    /// The answer to a code: a ticket, or why not.
    Traded(Result<Held, String>),
}

impl FromExchange {
    fn new(waker: Waker) -> Self {
        Self {
            answer: Rc::default(),
            waker,
        }
    }

    fn put(&self, answered: Answered) {
        *self.answer.borrow_mut() = Some(answered);
        self.waker.wake();
    }

    fn take(&self) -> Option<Answered> {
        self.answer.borrow_mut().take()
    }
}

struct Page {
    /// Where the relay is. Read from the page, and editable so a local build can reach a
    /// deployed relay.
    relay: String,
    /// What this person is called. Theirs to choose, and recorded at the join like any
    /// other name: `An unauthenticated player chooses their own name`.
    name: String,
    /// This browser's guest id, from local storage.
    ///
    /// Sixteen random bytes in hexadecimal, generated once and kept. It replaces deriving
    /// the account from the typed name, which the relay's log has already demonstrated the
    /// cost of: two people who both typed the same name were one account, and the second
    /// would land on the first's seat.
    ///
    /// Local storage rather than session storage, because the point of a stable id is that
    /// closing a tab does not lose a seat. The cost is that a shared browser shares an
    /// identity, which on the unranked side of the line is not worth paying anything to
    /// avoid.
    guest: String,
    /// The name the host gives the match, which is what its identity is derived from. Host
    /// twice under one name and the relay answers with the match that already exists, which
    /// is `A repeated request to host creates no second match` made useful rather than
    /// merely true.
    match_name: String,
    /// The id pasted in to join somebody else's match.
    paste: String,
    /// What to tell the person on the sign-in screen.
    trouble: Option<String>,

    /// Which player this tab is, if it is not the browser's usual one.
    ///
    /// Read once, at startup, and never changed: switching player mid-page would mean
    /// deciding what happens to a match already in progress, which no statement answers.
    handle: Option<String>,
    /// Handed to every mailbox, and kept because a new socket means new mailboxes.
    waker: Waker,
    seated: Option<Seated>,
    /// What is open to join, as the relay last said. Stale by the time it is drawn, which
    /// is why joining one can still be refused.
    open: Vec<Listed>,
    /// Whether the relay has answered the dashboard yet, so an empty list can be told from a
    /// question that has not come back.
    answered: bool,
    wire: Option<Box<dyn Wire>>,
    arrivals: Arrivals,
    mishap: Mishap,
    hand: Hand,
    /// The claim being dialled. Always legal.
    pending: Claim,
    /// When a frame last arrived, on the frame loop's clock.
    heard: f64,
    /// Where the exchange is, from the page. Empty means login is off.
    exchange: String,
    /// Whether the first frame has asked the exchange anything yet.
    asked_the_exchange: bool,
    /// When this page last sent a keepalive, on the frame loop's clock.
    spoke: f64,
    /// Whether this page has already acted on a listing that named a match of its own.
    ///
    /// Once per socket. Without it, every listing that arrives while the page is sitting
    /// in the dashboard would resume again, and a listing arrives whenever anybody asks.
    returned: bool,
    /// What the page knows about signing in.
    signin: Signin,
    /// Where the exchange's answers land.
    from_exchange: FromExchange,
    /// How long the host must still wait to cancel, and when the relay said so.
    ///
    /// Display only. The relay decides whether a cancel may happen — it is the only thing
    /// that knows when the match last moved — and this exists so the number counts down
    /// while somebody watches it instead of sitting still. Nothing here is a decision.
    countdown: Option<(u32, f64)>,
    log_shown: usize,
}

impl Page {
    fn new(waker: Waker) -> Self {
        let handle = socket::handle();
        Self {
            relay: socket::relay_from_page(),
            name: String::new(),
            guest: socket::guest_id(handle.as_deref()),
            exchange: socket::auth_from_page(),
            asked_the_exchange: false,
            spoke: 0.0,
            returned: false,
            signin: Signin::Asking,
            from_exchange: FromExchange::new(waker.clone()),
            match_name: String::new(),
            paste: String::new(),
            trouble: None,
            seated: None,
            open: Vec::new(),
            answered: false,
            wire: None,
            arrivals: Arrivals::new(waker.clone()),
            mishap: Mishap::new(waker.clone()),
            handle,
            waker,
            hand: Hand::default(),
            pending: Claim {
                quantity: 1,
                face: 1,
            },
            heard: 0.0,
            countdown: None,
            log_shown: 10,
        }
    }

    /// The account this client asserts.
    ///
    /// What this page says it is.
    ///
    /// A guest: the id this browser kept, and the name typed on the sign-in screen. Not an
    /// `Account` — a client does not send one. It says who it is and the relay decides what
    /// that means, which is why a page cannot claim a Discord account without a ticket.
    fn who(&self) -> Who {
        match &self.signin {
            // Signed in, so the ticket is the whole answer: the account and the name both
            // come out of it, and this page could not name them if it wanted to.
            Signin::Held(held) => Who::Authenticated {
                ticket: held.ticket.clone(),
            },
            _ => Who::Guest {
                id: self.guest.clone(),
                name: self.name.trim().to_string(),
            },
        }
    }

    /// What to call this person on screen.
    ///
    /// The Discord display name when there is one, because `An authenticated player's name
    /// is their Discord display name` and the typed field is not theirs to set.
    fn shown_name(&self) -> &str {
        match &self.signin {
            Signin::Held(held) => &held.name,
            _ => self.name.trim(),
        }
    }

    /// Ask the exchange what it can do, or trade a code it sent us back with.
    ///
    /// Called once, from the first frame, because `Page::new` runs before there is a
    /// runtime to spawn a fetch on.
    fn ask_the_exchange(&mut self) {
        if self.exchange.is_empty() {
            self.signin = Signin::Off;
            return;
        }
        // A ticket already in storage outlives a reload, which is the point of keeping
        // one. Expiry is checked here rather than trusted: a stale ticket would be
        // refused by the relay and look like a bug.
        if let Some(held) = socket::ticket_kept(self.handle.as_deref()) {
            self.signin = Signin::Held(held);
            return;
        }
        match socket::code_from_url() {
            // Back from Discord. The code is single-use and a URL holding one is a URL
            // somebody will paste, so it is taken out of the address bar at once.
            Some(code) => {
                socket::forget_code();
                socket::trade(&self.exchange, &code, &self.from_exchange);
            }
            None => socket::ask_where(&self.exchange, &self.from_exchange),
        }
    }

    /// Take whatever the exchange has said since the last frame.
    fn hear_the_exchange(&mut self) {
        let Some(answered) = self.from_exchange.take() else {
            return;
        };
        match answered {
            Answered::Where(Some((client_id, redirect_uri))) => {
                self.signin = Signin::Offered {
                    client_id,
                    redirect_uri,
                };
            }
            Answered::Where(None) => self.signin = Signin::Off,
            Answered::Traded(Ok(held)) => {
                socket::keep_ticket(self.handle.as_deref(), &held);
                self.signin = Signin::Held(held);
            }
            Answered::Traded(Err(why)) => {
                // Login is not off — the exchange is there and answered — so the page asks
                // again where to send somebody, and says what went wrong meanwhile.
                self.trouble = Some(why);
                socket::ask_where(&self.exchange, &self.from_exchange);
                self.signin = Signin::Asking;
            }
        }
    }

    /// Open a socket and ask what is open.
    ///
    /// The dashboard's question is the opening frame, so a client is connected before it has
    /// decided anything. Hosting and joining both happen later on this same socket.
    fn connect(&mut self, now: f64) {
        self.trouble = None;
        // A new socket, so the question of where this player belongs is open again.
        self.returned = false;
        self.spoke = now;
        self.arrivals = Arrivals::new(self.waker.clone());
        self.mishap = Mishap::new(self.waker.clone());
        self.hand = Hand::default();
        self.open = Vec::new();
        self.answered = false;
        self.heard = now;
        let asking = dashboard::ask(&self.who());
        match socket::open(&self.relay, &asking, &self.arrivals, &self.mishap) {
            Ok(wire) => self.wire = Some(wire),
            Err(why) => {
                self.wire = None;
                self.seated = None;
                self.trouble = Some(why);
            }
        }
    }

    /// Take a seat on the socket that is already open.
    fn sit(&mut self, seated: Seated) {
        self.trouble = None;
        self.hand = Hand::default();
        let opening = seated.opening();
        self.seated = Some(seated);
        self.say(vec![opening]);
    }

    /// Ask again what is open. A listing is only true of the moment it was read.
    fn look_again(&mut self) {
        self.answered = false;
        let asking = dashboard::ask(&self.who());
        self.say(vec![asking]);
    }

    /// Leave the match but not the relay, which is where somebody goes to play another.
    fn leave_seat(&mut self, trouble: Option<String>) {
        self.seated = None;
        self.trouble = trouble;
        // Somebody who left must not be put straight back. `returned` is cleared only by
        // a new socket, so the listing that arrives next does not undo a deliberate leave
        // — and the relay agrees, because a seat that left is not a seat held.
        self.returned = true;
        // Belongs to the match being left. Carrying it into the next one would show a
        // stale countdown over a match nothing has been asked about.
        self.countdown = None;
        self.look_again();
    }

    /// Drop the socket. Only for a transport that has gone, never for a refusal.
    fn disconnect(&mut self, trouble: Option<String>) {
        self.wire = None;
        self.seated = None;
        self.open = Vec::new();
        self.answered = false;
        self.countdown = None;
        self.trouble = trouble;
    }

    fn say(&mut self, frames: Vec<String>) {
        for frame in frames {
            let Some(wire) = &self.wire else {
                return;
            };
            if let Err(why) = wire.send(&frame) {
                self.disconnect(Some(why));
                return;
            }
        }
    }

    /// Keep the socket from being closed for being quiet.
    ///
    /// An empty frame, which the relay reads as no message at all. See [`KEEPALIVE`] for
    /// why this exists rather than the socket being left alone.
    ///
    /// On its own timer rather than only when the socket has been idle. Real traffic
    /// resets API Gateway's clock too, so a keepalive during an active match is redundant
    /// — and tracking that would mean every `say` knowing what time it is, for a saving
    /// of one empty frame every four minutes. Unconditional has no case to get wrong.
    fn keep_the_socket(&mut self, now: f64) {
        if self.wire.is_none() || now - self.spoke < KEEPALIVE {
            return;
        }
        self.spoke = now;
        self.say(vec![String::new()]);
    }

    /// When a frame is owed with nothing arriving, if one is.
    ///
    /// A socket and not a seat. The two things that need a frame when nothing is happening
    /// are the keepalive and noticing that the relay has gone [`QUIET`], and both of those
    /// last exactly as long as the socket does — a page sitting on the dashboard holds an open
    /// socket and must go on saying so, or the ten minute idle timeout takes it.
    ///
    /// A page with no socket owes nothing on a clock and sleeps until something wakes it,
    /// which is why the sign-in screen does not spin while it waits for the exchange.
    fn frame_owed(&self) -> Option<Duration> {
        self.wire.is_some().then_some(TICK)
    }

    /// Apply everything that has arrived, then let the seat answer.
    ///
    /// The one ordering that matters is inside this function: every frame is applied before
    /// the driver is asked, so a call is chosen against the whole of what is known rather
    /// than against a prefix of it.
    fn pump(&mut self, now: f64) {
        if let Some(trouble) = self.mishap.take() {
            self.disconnect(Some(trouble));
            return;
        }
        while let Some(frame) = self.arrivals.take() {
            self.heard = now;
            // The dashboard's answer is not about any seat, and can arrive after one has been
            // taken, because the socket that asked is the socket that sat down.
            if let Some(open) = dashboard::listing(&frame) {
                self.open = open;
                self.answered = true;
                self.return_to_my_match();
                continue;
            }
            let Some(seated) = &mut self.seated else {
                continue;
            };
            match seated.receive(&frame) {
                Ok(answers) => self.say(answers),
                Err(Stopped::Over) => return,
                Err(why) => {
                    // A refusal is the relay declining a message, not the socket dying.
                    // Going back to the dashboard keeps the connection and says what happened.
                    self.leave_seat(Some(why.to_string()));
                    return;
                }
            }
        }
        // The one refusal that does not end the seat. `Seated::receive` keeps it rather
        // than returning `Stopped`, because a refused Cancel is an answer to the button and
        // not the relay declining to have this client in the match.
        if let Some(seconds) = self.seated.as_mut().and_then(Seated::take_still_active) {
            self.countdown = Some((seconds, now));
        }
        let Some(seated) = &mut self.seated else {
            return;
        };
        match seated.offer() {
            Ok(answers) => self.say(answers),
            Err(why) => {
                self.leave_seat(Some(why.to_string()));
                return;
            }
        }
        // A seat that has left is no longer in the match, and the relay has already let go
        // of its socket, so nothing more is coming.
        if let Some(seated) = &self.seated
            && let Some(mine) = seated.seat()
            && seated
                .roster()
                .get(mine.0 as usize)
                .is_some_and(|player| player.left)
        {
            self.leave_seat(Some("You left the match.".to_string()));
        }
    }

    /// Go back to the match this account already holds a seat in.
    ///
    /// `A client returns its player to the match they hold a seat in, without asking`. The
    /// listing is what knows which one: it is the pass that already read every match, so
    /// no second question is needed and there is nothing to keep in step.
    ///
    /// This is what makes a drop survivable. API Gateway closes every connection at two
    /// hours whatever happens, and a dropped socket used to leave somebody at the entry
    /// screen needing the match id from the host. Now the page finds its own way back.
    fn return_to_my_match(&mut self) {
        if self.returned || self.seated.is_some() {
            return;
        }
        let Some(mine) = self.open.iter().find(|open| open.mine) else {
            return;
        };
        // Once per socket, whatever listings arrive later.
        self.returned = true;
        let seated = Seated::resuming(
            self.who(),
            mine.match_id.clone(),
            Box::new(self.hand.clone()),
        );
        match seated {
            Some(seated) => self.sit(seated),
            None => self.trouble = Some(UNREADABLE_TICKET.to_string()),
        }
    }

    /// What the seat says about itself.
    fn stance(&self) -> Option<Stance> {
        let seated = self.seated.as_ref()?;
        let Some(view) = seated.view() else {
            return Some(Stance {
                over: seated.over(),
                started: false,
                my_turn: false,
                playing: false,
            });
        };
        Some(Stance {
            over: seated.over(),
            started: true,
            my_turn: view.is_my_turn(),
            playing: seated.playing().unwrap_or(false),
        })
    }

    /// Who is at the match, as the panels want it.
    ///
    /// Rebuilt every frame from the log rather than kept, because the log is where the
    /// names are and a copy is a thing to keep in step. A departed seat is not renamed: it
    /// holds no dice, which is what the table already shows and what a player needs.
    fn seating(seated: &Seated) -> Seating {
        let roster = seated.roster();
        Seating::new(
            roster.iter().map(|player| player.name.clone()).collect(),
            roster
                .iter()
                .map(|player| index_of(&player.colour).unwrap_or(UNKNOWN_COLOUR))
                .collect(),
        )
    }
}

impl eframe::App for Page {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = ui.input(|input| input.time);
        // Once, on the first frame. `Page::new` runs before there is a runtime to spawn a
        // fetch on, so the question cannot be asked from there.
        if !self.asked_the_exchange {
            self.asked_the_exchange = true;
            self.ask_the_exchange();
        }
        self.hear_the_exchange();
        self.pump(now);
        self.keep_the_socket(now);
        if let Some(after) = self.frame_owed() {
            ui.ctx().request_repaint_after(after);
        }
        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            // Do not shrink to the content: a grid inside a shrinking scroll area is given
            // the narrowest width it can be drawn in, which squeezes every field in it down
            // to nothing.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| self.draw(ui, now));
        });
    }
}

impl Page {
    fn draw(&mut self, ui: &mut egui::Ui, now: f64) {
        let quiet = now - self.heard > QUIET;
        let stance = self.stance();
        match screen_for(self.wire.is_some(), stance, quiet) {
            Screen::SignIn => self.sign_in(ui, now),
            Screen::Dashboard => self.dashboard(ui),
            Screen::Lobby => self.lobby(ui, now),
            // The three table screens differ in what they draw and not in what they are,
            // so they are one function given the same facts the choice was made from.
            Screen::Playing | Screen::Watching => {
                if let Some(stance) = stance {
                    self.at_the_table(ui, stance, None, now);
                }
            }
            Screen::Halted => {
                let waiting_on = self.waiting_on();
                if let Some(stance) = stance {
                    self.at_the_table(ui, stance, waiting_on, now);
                }
            }
            Screen::Over => self.over(ui),
        }
    }

    /// The name of the seat whose turn it is, when it is not this one's.
    fn waiting_on(&self) -> Option<String> {
        let seated = self.seated.as_ref()?;
        let view = seated.view()?;
        let current = view.public().current();
        Some(Self::seating(seated).name(current).to_string())
    }

    /// Whether to offer signing in, and what to say about it.
    ///
    /// Three answers and no fourth. The page asks the exchange before drawing anything
    /// here, so a sign-in button is never shown by a page that cannot complete the flow —
    /// `A missing Discord config disables login rather than failing`, from the one side of
    /// it a person can see.
    fn with_discord(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Signing in").strong());
        match self.signin.clone() {
            Signin::Asking => {
                ui.label(egui::RichText::new("Asking whether Discord is available…").size(12.0));
            }
            Signin::Off => {
                ui.label(
                    egui::RichText::new(
                        "Discord sign-in is not switched on here, so every match is \
                         unranked. Everything else works.",
                    )
                    .size(12.0),
                );
            }
            Signin::Offered {
                client_id,
                redirect_uri,
            } => {
                if tap(ui, "Sign in with Discord", true) {
                    socket::authorize(&client_id, &redirect_uri);
                }
                ui.label(
                    egui::RichText::new(
                        "Only a match where every seat has signed in counts as ranked. \
                         Playing without is the same game.",
                    )
                    .size(12.0),
                );
            }
            Signin::Held(held) => {
                ui.label(egui::RichText::new(format!("Signed in as {}", held.name)).size(15.0));
                if tap(ui, "Sign out", true) {
                    // The ticket goes, and the guest id stays — so signing out is going
                    // back to being the guest this browser already was, not becoming
                    // somebody new.
                    socket::forget_ticket(self.handle.as_deref());
                    self.signin = Signin::Asking;
                    socket::ask_where(&self.exchange, &self.from_exchange);
                }
            }
        }
    }

    /// A name, a colour, and either host or join.
    fn sign_in(&mut self, ui: &mut egui::Ui, now: f64) {
        ui.heading("Liars Dice");
        ui.label(
            "A match over the network. One person per seat, and the dice are dealt by a \
             relay that cannot see them either.",
        );
        ui.separator();

        if let Some(trouble) = &self.trouble {
            ui.label(egui::RichText::new(trouble).strong());
            ui.add_space(6.0);
        }

        self.with_discord(ui);

        ui.add_space(8.0);
        ui.separator();
        match &self.signin {
            // `An authenticated player's name is their Discord display name`, so there is
            // nothing to type and no field to type it in.
            Signin::Held(held) => {
                ui.label(egui::RichText::new("Your name").strong());
                ui.label(egui::RichText::new(&held.name).size(17.0));
                ui.label(
                    egui::RichText::new("From Discord, which is what makes a match ranked.")
                        .size(12.0),
                );
            }
            _ => {
                ui.label(egui::RichText::new("Your name").strong());
                ui.add_sized(
                    [240.0, 30.0],
                    egui::TextEdit::singleline(&mut self.name).hint_text("name"),
                );
                ui.label(
                    egui::RichText::new(
                        "Yours to choose, and nothing checks it. Who you are is an id this \
                         browser keeps, so two people called the same thing are still two \
                         people.",
                    )
                    .size(12.0),
                );
            }
        }

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("The relay").strong());
        ui.add_sized(
            [460.0, 30.0],
            egui::TextEdit::singleline(&mut self.relay).hint_text("wss://host/stage"),
        );

        // A signed-in player is already named, so there is nothing to wait for.
        let named = !self.shown_name().is_empty();
        let reachable = named && self.relay.contains("://");

        ui.add_space(8.0);
        ui.separator();
        if tap(ui, "Look for matches", reachable) {
            self.connect(now);
        }
        if !reachable {
            // A button that does nothing and says nothing is the worst of both. Say which
            // of the two things is missing, since one of them arrives with the page and a
            // person has no reason to expect either.
            ui.label(
                egui::RichText::new(if named {
                    "A relay address is needed, and this page did not come with one."
                } else {
                    "A name is needed first."
                })
                .size(12.0),
            );
        }
    }

    /// Connected, and not at a match: what is open, and the two ways to be at one.
    fn dashboard(&mut self, ui: &mut egui::Ui) {
        ui.heading("Matches");
        if let Some(trouble) = &self.trouble {
            ui.label(egui::RichText::new(trouble).strong());
        }
        ui.label(
            egui::RichText::new(format!(
                "You are {}. Your dice get their colour in the match.",
                self.shown_name()
            ))
            .size(12.0),
        );
        ui.separator();

        let mut joining = None;
        if !self.answered {
            ui.label("asking the relay…");
        } else if self.open.is_empty() {
            ui.label("Nothing is going on. Host a match, and somebody can join it.");
        } else {
            // Two groups, because `A listed match shows whether it can be joined` and the
            // honest way to show that is to stop offering a button for the ones that
            // cannot. The relay decides which is which: `joinable` is the refusal `join`
            // would give, so a greyed row and a refused click can never disagree.
            let (open, going_on): (Vec<&Listed>, Vec<&Listed>) =
                self.open.iter().partition(|open| open.joinable);

            if open.is_empty() {
                ui.label("Nothing is open to join just now.");
            } else {
                ui.label(egui::RichText::new("Open to join").strong());
                egui::Grid::new("open").spacing([12.0, 8.0]).show(ui, |ui| {
                    for listed in &open {
                        ui.label(egui::RichText::new(&listed.host).strong());
                        ui.label(players(listed.players));
                        ui.label(ranked_note(listed.ranked));
                        if tap(ui, if listed.mine { "Rejoin" } else { "Join" }, true) {
                            joining = Some(listed.match_id.clone());
                        }
                        ui.end_row();
                    }
                });
            }

            if !going_on.is_empty() {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("In progress").strong());
                egui::Grid::new("going-on")
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        for listed in &going_on {
                            ui.label(egui::RichText::new(&listed.host).strong());
                            ui.label(players(listed.players));
                            ui.label(ranked_note(listed.ranked));
                            // No button. Why there is none is the useful part, and the
                            // relay already said it.
                            ui.label(
                                egui::RichText::new(if listed.mine {
                                    "yours — reconnecting"
                                } else {
                                    "under way"
                                })
                                .size(12.0),
                            );
                            ui.end_row();
                        }
                    });
            }
        }

        ui.add_space(6.0);
        if tap(ui, "Look again", true) {
            self.look_again();
        }

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Host a match").strong());
        let mut hosting = false;
        ui.horizontal(|ui| {
            ui.add_sized(
                [240.0, 30.0],
                egui::TextEdit::singleline(&mut self.match_name).hint_text("what to call it"),
            );
            hosting = tap(ui, "Host", !self.match_name.trim().is_empty());
        });
        ui.label(
            egui::RichText::new(
                "The name decides which match this is, so hosting the same name twice \
                 takes you back to the same match rather than making a second one.",
            )
            .size(12.0),
        );

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Join by id").strong());
        ui.horizontal(|ui| {
            ui.add_sized(
                [340.0, 30.0],
                egui::TextEdit::singleline(&mut self.paste).hint_text("the id the host gives you"),
            );
            if tap(ui, "Join", !self.paste.trim().is_empty()) {
                joining = Some(MatchId(self.paste.trim().to_string()));
            }
        });
        ui.label(
            egui::RichText::new(
                "Still here because a listing shows only what is open to everyone, and an \
                 id reaches a match whoever has it.",
            )
            .size(12.0),
        );

        ui.add_space(8.0);
        ui.separator();
        if tap(ui, "Disconnect", true) {
            self.disconnect(None);
            return;
        }

        // `Seated::hosting` and `joining` answer `None` only for an `Authenticated` whose
        // ticket cannot be read, and this page builds a guest, so the seat always exists.
        // Said with a `let else` rather than an `expect` so that adding the Discord path
        // later has to answer the question here instead of panicking in front of somebody.
        if hosting {
            let seated = Seated::hosting(
                self.who(),
                self.match_name.trim(),
                Box::new(self.hand.clone()),
            );
            match seated {
                Some(seated) => self.sit(seated),
                None => self.trouble = Some(UNREADABLE_TICKET.to_string()),
            }
        } else if let Some(match_id) = joining {
            let seated = Seated::joining(self.who(), match_id, Box::new(self.hand.clone()));
            match seated {
                Some(seated) => self.sit(seated),
                None => self.trouble = Some(UNREADABLE_TICKET.to_string()),
            }
        }
    }

    /// In a match that has not started.
    fn lobby(&mut self, ui: &mut egui::Ui, now: f64) {
        let Some(seated) = &self.seated else {
            return;
        };
        // Read once, into owned values, before anything is sent. This screen both reads
        // the seat and speaks on it, and a borrow cannot be held across the sending.
        let host = seated.host();
        let roster = seated.roster();
        let mine = seated.seat().map(|seat| seat.0 as usize);
        let named = seated.match_id();
        let enough = roster.iter().filter(|player| !player.left).count() >= 2;

        ui.heading(if host {
            "Your match"
        } else {
            "Waiting for the host"
        });

        if let Some(match_id) = named {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("The id to pass to somebody").strong());
            // A field rather than a label: a label cannot be selected, and this has to be
            // copied out of the page by hand until the dashboard exists.
            let mut shown = match_id.0.clone();
            ui.add_sized(
                [340.0, 30.0],
                egui::TextEdit::singleline(&mut shown).font(egui::TextStyle::Monospace),
            );
        } else {
            ui.label("asking the relay to open it…");
        }

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Who is here").strong());
        egui::Grid::new("roster")
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                for player in &roster {
                    die(ui, index_of(&player.colour).unwrap_or(UNKNOWN_COLOUR), None);
                    ui.label(&player.name);
                    if player.left {
                        ui.label(egui::RichText::new("left").size(12.0));
                    }
                    ui.end_row();
                }
            });

        // `A lobby is the only screen that offers to change a colour`, and the only one
        // that can: what is free is a property of the match, which nothing before this
        // screen knows.
        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Your dice").strong());
        ui.label(
            egui::RichText::new(
                "Yours until the match starts. The colours somebody else has are not                  offered.",
            )
            .size(12.0),
        );
        let (chosen, taken) = choosable(&roster, mine);
        let recoloured = palette_choice(ui, chosen, &taken).and_then(|index| {
            self.seated
                .as_ref()
                .and_then(|seated| seated.recolour(PALETTE[index].name))
        });
        if let Some(frame) = recoloured {
            self.say(vec![frame]);
        }

        ui.add_space(8.0);
        ui.separator();
        let start = if host {
            if tap(ui, "Start the match", enough) {
                self.seated.as_ref().and_then(Seated::start)
            } else {
                None
            }
        } else {
            ui.label("The host starts it.");
            None
        };
        if let Some(frame) = start {
            self.say(vec![frame]);
        }

        ui.add_space(6.0);
        let leave = if tap(ui, "Leave", true) {
            self.seated.as_ref().and_then(Seated::leave)
        } else {
            None
        };
        if let Some(frame) = leave {
            self.say(vec![frame]);
        }
        self.cancelling(ui, now);
    }

    /// The host's Cancel button, and why it did not work if it did not.
    ///
    /// Always enabled. The relay decides whether a cancel may happen, and a button greyed
    /// out by this page's own clock would be a second opinion about the time — one that
    /// could disagree with the only one that counts. So it asks, and shows the answer.
    /// Pressing it again simply refreshes the number.
    fn cancelling(&mut self, ui: &mut egui::Ui, now: f64) {
        let Some(seated) = &self.seated else {
            return;
        };
        if !seated.host() {
            return;
        }
        ui.add_space(6.0);
        let asked = if tap(ui, "Cancel the match", true) {
            seated.cancel()
        } else {
            None
        };
        if let Some(remaining) = remaining(self.countdown, now) {
            ui.label(
                egui::RichText::new(format!(
                    "Not yet. Somebody has moved in the last minute — {remaining}s to go."
                ))
                .size(13.0),
            );
        }
        if let Some(frame) = asked {
            // Cleared on the way out, so the old number is not shown beside a fresh
            // answer that has not come back yet.
            self.countdown = None;
            self.say(vec![frame]);
        }
    }

    /// This seat's own view: the table, the history, and the dialler if it has one.
    ///
    /// Playing, Watching and Halted are one screen because that is what the difference
    /// amounts to. `Seated::playing` decides whether the dialler is drawn, and `waiting_on`
    /// whether a line is added saying who has gone quiet. The `View` already returns nothing
    /// this seat may not see, so an eliminated seat watching reveals nothing a playing seat
    /// would not have seen.
    fn at_the_table(
        &mut self,
        ui: &mut egui::Ui,
        stance: Stance,
        waiting_on: Option<String>,
        now: f64,
    ) {
        let Some(seated) = &self.seated else {
            return;
        };
        let playing = stance.playing;
        let seating = Self::seating(seated);
        let mine = seated.seat().map_or(0, |seat| seat.0 as usize);

        ui.horizontal(|ui| {
            ui.heading(seating.name(mine));
            if let Some(view) = seated.view() {
                ui.label(
                    egui::RichText::new(format!("round {}", view.public().round() + 1)).size(15.0),
                );
            }
            if !playing && waiting_on.is_none() {
                ui.label(egui::RichText::new("watching").size(15.0));
            }
        });

        if let Some(name) = &waiting_on {
            ui.separator();
            ui.label(
                egui::RichText::new(format!("Waiting on {name}."))
                    .size(17.0)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(
                    "Nothing has arrived for a while. They may be thinking, or they may \
                     have closed their tab — this page cannot tell the two apart. The match \
                     waits for them however long it takes, and goes on the moment they \
                     claim. Leaving it costs nobody else their match, and the host can end \
                     it once a minute has passed with nothing happening.",
                )
                .size(12.0),
            );
        }

        // The last round's outcome, over a match that has already moved on.
        if let Some(showing) = seated.showing() {
            ui.separator();
            ui.label(egui::RichText::new("Last round").size(12.0));
            ui.label(
                egui::RichText::new(verdict(
                    &seating,
                    showing.by.0 as usize,
                    showing.call,
                    showing.claim,
                    &showing.resolution,
                ))
                .size(15.0),
            );
        }

        ui.separator();
        let mut pending = self.pending;
        // Chosen inside its own scope: the view borrows the seat, and putting a call in the
        // hand needs it back.
        let call = {
            let Some(view) = seated.view() else {
                return;
            };
            table(ui, &view, &seating);
            ui.separator();
            standing(ui, &view, &seating);
            history(ui, &view, &seating);
            if playing {
                ui.separator();
                // A dial left where an earlier round left it can be a dead end: every step
                // away from an illegal claim is itself illegal, so `dialler` offers nothing
                // and `calls` refuses to raise, and the person is stuck with liar or spot
                // on. Pulling it up to the lowest legal claim costs nothing and is checked
                // rather than reset on a turn boundary — there is no event for "it is your
                // turn now", only a view that says it is, and the frame that delivers the
                // claim making it this seat's turn is the same frame that would have to
                // notice.
                if view.public().check_raise(pending).is_err() {
                    pending = opening_claim(&view);
                }
                ui.label(egui::RichText::new("Your claim").strong());
                dialler(ui, &view, &mut pending);
                odds(ui, &view, pending);
                ui.separator();
                if may_call(stance) {
                    calls(ui, &view, pending)
                } else {
                    ui.label(
                        egui::RichText::new("Not your turn. Dial ahead if you like.").size(13.0),
                    );
                    None
                }
            } else {
                None
            }
        };
        self.pending = pending;
        if let Some(call) = call {
            self.hand.0.set(Some(call));
        }

        ui.separator();
        let leave = if tap(ui, "Leave the match", true) {
            seated.leave()
        } else {
            None
        };
        if let Some(frame) = leave {
            self.say(vec![frame]);
        }
        // Offered whenever the host is at the table, not only when the screen has noticed
        // the match going quiet. A host whose opponent is thinking for a long time and a
        // host whose opponent has gone are the same picture from here, and the relay is
        // the one that can tell them apart — so the button is always there and the answer
        // says whether it is too soon.
        self.cancelling(ui, now);

        ui.separator();
        self.events(ui);
    }

    /// The ranking, and the choice to play again or go.
    fn over(&mut self, ui: &mut egui::Ui) {
        let Some(seated) = &self.seated else {
            return;
        };
        let seating = Self::seating(seated);

        // What the heading says depends on the relay's word, not on this seat having
        // worked out that the match was finished. A cancelled match has no winner, and
        // presenting one would be inventing it.
        match seated.ending() {
            Some(Ending::Cancelled { by }) => {
                ui.heading("Match cancelled");
                ui.label(
                    egui::RichText::new(format!(
                        "{} ended it. Nobody won.",
                        seating.name(by.0 as usize)
                    ))
                    .size(15.0),
                );
            }
            Some(Ending::Abandoned) => {
                ui.heading("Match abandoned");
                ui.label(
                    egui::RichText::new(
                        "Nothing happened in it for an hour, so it was ended. Nobody won.",
                    )
                    .size(15.0),
                );
            }
            // , or an ending this seat derived before the relay confirmed it —
            // which is the ordinary case, since a seat knows the match is over the moment
            // it folds the last reveal.
            _ => {
                ui.heading("Match over");
            }
        }
        if let Some(showing) = seated.showing() {
            ui.label(
                egui::RichText::new(verdict(
                    &seating,
                    showing.by.0 as usize,
                    showing.call,
                    showing.claim,
                    &showing.resolution,
                ))
                .size(17.0),
            );
        }
        ui.separator();
        if let Some(view) = seated.view() {
            table(ui, &view, &seating);
            ui.separator();
            ranking(ui, &view, &seating);
        }
        ui.separator();

        let mut done = false;
        ui.horizontal(|ui| {
            if tap(ui, "Play again", true) {
                done = true;
            }
            if tap(ui, "Leave", true) {
                done = true;
            }
        });
        if done {
            // Both buttons go back to the dashboard, and say so differently on purpose: one
            // person is going to host or join another and the other is done. Neither needs
            // the match they have just finished, and neither needs the socket closed —
            // leaving the relay is its own button, one screen on.
            self.match_name.clear();
            self.paste.clear();
            self.leave_seat(None);
        }

        ui.separator();
        self.events(ui);
    }

    /// What this seat has been sent, which is not what the relay holds.
    ///
    /// The relay's log carries a `Deal` for every seat; a client is only sent its own until
    /// the round is revealed. This is the client's own stream, so there is nothing in it
    /// this seat was not told.
    fn events(&mut self, ui: &mut egui::Ui) {
        let Some(seated) = &self.seated else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("What this seat has seen").strong());
            ui.add(egui::Slider::new(&mut self.log_shown, 4..=60).text("lines"));
        });
        let stream = seated.stream();
        let from = stream.len().saturating_sub(self.log_shown);
        egui::ScrollArea::vertical()
            .id_salt("events")
            .max_height(220.0)
            .show(ui, |ui| {
                for event in &stream[from..] {
                    ui.label(egui::RichText::new(event.to_string()).monospace());
                }
            });
    }
}

/// The transport. The only part of this app a browser is required for.
#[cfg(target_arch = "wasm32")]
mod socket {
    use eframe::wasm_bindgen::JsCast;
    use eframe::wasm_bindgen::closure::Closure;

    use super::{Arrivals, Mishap, Wire};

    /// The placeholder the published page carries until the deploy writes the real address
    /// over it. The deploy is the only thing that knows the relay's address: the bundle is
    /// built in the gate, which has no AWS credentials and must not have any.
    const PLACEHOLDER: &str = "RELAY_SOCKET_URL";

    /// Where this browser keeps its guest id.
    const GUEST_KEY: &str = "liars-dice-guest";
    const HANDLE_KEY: &str = "liars-dice-handle";

    /// Which player this tab is, if it is not the browser's usual one.
    ///
    /// The fragment sets it; the tab keeps it. `sessionStorage` is per browsing context in
    /// exactly the way `localStorage` is not, which is the whole of why two tabs can be two
    /// people, and it survives a reload, which is `A tab keeps its player across a reload`.
    ///
    /// Keeping it here also settles the Discord round trip without a `state` parameter: a
    /// fragment survives a redirect only unreliably, and nothing depends on it surviving,
    /// because the handle is already in the tab before anybody leaves for Discord.
    pub fn handle() -> Option<String> {
        let window = web_sys::window()?;
        let tab = window.session_storage().ok().flatten();
        if let Some(fragment) = window.location().hash().ok()
            && let Some(handle) = super::handle_in(&fragment)
        {
            if let Some(tab) = &tab {
                let _ = tab.set_item(HANDLE_KEY, &handle);
            }
            forget_handle(&fragment);
            return Some(handle);
        }
        let kept = tab?.get_item(HANDLE_KEY).ok()??;
        super::clean_handle(&kept)
    }

    /// Take the handle out of the address bar, leaving the rest of the fragment.
    ///
    /// The same reasoning as [`forget_code`], and the same `replaceState` so the page is
    /// not reloaded: a URL people paste should not carry who they are.
    fn forget_handle(fragment: &str) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(history) = window.history() else {
            return;
        };
        let location = window.location();
        let path = location.pathname().unwrap_or_else(|_| "/".to_string());
        let query = location.search().unwrap_or_default();
        let rest = super::fragment_without_handle(fragment);
        let url = if rest.is_empty() {
            format!("{path}{query}")
        } else {
            format!("{path}{query}#{rest}")
        };
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&url));
    }

    /// This browser's guest id: the one it already had, or a new one it will keep.
    ///
    /// Sixteen bytes from `crypto.getRandomValues`, in hexadecimal. This is the one new
    /// non-determinism the identity unit adds, and it is an *input* rather than a derived
    /// value — an identity is something the world hands the program, like a keystroke.
    /// What the architecture statements forbid is deriving a value from an unrepeatable
    /// source and putting it in the log, and nothing here is derived: it is read once,
    /// stored, and passed inward.
    ///
    /// A reversal worth naming. The page used to derive the account from the typed name
    /// specifically so that nothing from a clock could reach the log, and that was right
    /// while there was nowhere to keep an id. With local storage there is somewhere, and a
    /// stable random id is strictly better: it collides with nobody, where a typed name
    /// collides with everybody who typed it.
    ///
    /// If local storage cannot be reached — a browser with it disabled, or a page opened
    /// from a file — the id is still generated and still works for this page's lifetime.
    /// It is a seat that does not survive a reload, which is worse than the alternative
    /// and better than not playing.
    pub fn guest_id(handle: Option<&str>) -> String {
        let key = super::storage_key(GUEST_KEY, handle);
        let storage = web_sys::window().and_then(|window| window.local_storage().ok().flatten());
        if let Some(storage) = &storage
            && let Ok(Some(kept)) = storage.get_item(&key)
            && kept.len() == 32
            && kept.chars().all(|digit| digit.is_ascii_hexdigit())
        {
            return kept;
        }
        let made = fresh_id();
        if let Some(storage) = &storage {
            // A failure here is a quota or a private-browsing refusal, and there is
            // nothing useful to do about either: the id works, it just will not outlive
            // the tab.
            let _ = storage.set_item(&key, &made);
        }
        made
    }

    /// Sixteen random bytes, in hexadecimal.
    ///
    /// `getRandomValues` is the browser's CSPRNG. If it cannot be reached there is no
    /// second source to fall back to and inventing one would be worse than it looks — a
    /// predictable guest id is somebody else's seat — so this gives up and the page says
    /// so rather than quietly handing out a guessable identity.
    fn fresh_id() -> String {
        let mut bytes = [0u8; 16];
        let drawn = web_sys::window()
            .and_then(|window| window.crypto().ok())
            .is_some_and(|crypto| crypto.get_random_values_with_u8_array(&mut bytes).is_ok());
        if !drawn {
            return String::new();
        }
        let mut id = String::with_capacity(32);
        for byte in bytes {
            id.push(char::from_digit(u32::from(byte >> 4), 16).expect("a nibble"));
            id.push(char::from_digit(u32::from(byte & 0x0f), 16).expect("a nibble"));
        }
        id
    }

    /// Where this browser keeps the ticket it was given.
    const TICKET_KEY: &str = "liars-dice-ticket";

    /// Where the exchange is, according to the page. Empty means login is off.
    pub fn auth_from_page() -> String {
        from_meta("auth", "AUTH_URL")
    }

    /// The `?code=` Discord sent us back with.
    pub fn code_from_url() -> Option<String> {
        let search = web_sys::window()?.location().search().ok()?;
        web_sys::UrlSearchParams::new_with_str(&search)
            .ok()?
            .get("code")
    }

    /// Take the code out of the address bar.
    ///
    /// A code is single-use, and a URL holding one is a URL somebody will paste or
    /// bookmark. `replaceState` rather than a navigation, so the page is not reloaded and
    /// the trade already in flight is not abandoned.
    pub fn forget_code() {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(history) = window.history() else {
            return;
        };
        let bare = window
            .location()
            .pathname()
            .unwrap_or_else(|_| "/".to_string());
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&bare));
    }

    /// The ticket this browser is holding, if it has one that has not expired.
    ///
    /// Expiry is checked here rather than left to the relay. A stale ticket would be
    /// refused with `BadTicket`, which on screen would look like a bug rather than like
    /// *sign in again*.
    pub fn ticket_kept(handle: Option<&str>) -> Option<super::Held> {
        let kept = storage()?
            .get_item(&super::storage_key(TICKET_KEY, handle))
            .ok()??;
        let (expires_at, rest) = kept.split_once(' ')?;
        let expires_at: u64 = expires_at.parse().ok()?;
        let (name_length, rest) = rest.split_once(' ')?;
        // Length-prefixed, because a display name may hold a space and a ticket may not
        // be truncated by one. The same reasoning as the ticket's own payload.
        let name_length: usize = name_length.parse().ok()?;
        let name = rest.get(..name_length)?.to_string();
        let ticket = rest.get(name_length + 1..)?.to_string();
        if expires_at <= seconds_now() {
            forget_ticket(handle);
            return None;
        }
        Some(super::Held {
            ticket,
            name,
            expires_at,
        })
    }

    pub fn keep_ticket(handle: Option<&str>, held: &super::Held) {
        if let Some(storage) = storage() {
            let kept = format!(
                "{} {} {} {}",
                held.expires_at,
                held.name.len(),
                held.name,
                held.ticket
            );
            let _ = storage.set_item(&super::storage_key(TICKET_KEY, handle), &kept);
        }
    }

    pub fn forget_ticket(handle: Option<&str>) {
        if let Some(storage) = storage() {
            let _ = storage.remove_item(&super::storage_key(TICKET_KEY, handle));
        }
    }

    /// Now, in seconds since the epoch.
    ///
    /// The one clock reading in this module, and it decides one thing: whether a ticket
    /// this browser is holding has already expired. Nothing derived from it reaches a log,
    /// and the relay checks the expiry again with its own clock, which is the one that
    /// counts.
    fn seconds_now() -> u64 {
        web_sys::window()
            .map(|window| window.performance().map_or(0.0, |_| js_sys::Date::now()))
            .map(|millis| (millis / 1000.0) as u64)
            .unwrap_or_default()
    }

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    }

    /// Send the browser to Discord to authorize.
    ///
    /// Built here rather than by the exchange, because only the page knows it is about to
    /// navigate. `identify` and nothing else; `prompt=none` is deliberately absent, so a
    /// person is asked rather than silently re-authorized.
    pub fn authorize(client_id: &str, redirect_uri: &str) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let url = format!(
            "https://discord.com/oauth2/authorize?response_type=code&client_id={}&scope=identify&redirect_uri={}",
            encode(client_id),
            encode(redirect_uri),
        );
        let _ = window.location().set_href(&url);
    }

    /// Percent-encode, for the two values that go into a query string.
    ///
    /// `encodeURIComponent`, which is the browser's own and is what Discord expects. A
    /// hand-rolled one here would be a second answer to a question the platform already
    /// answers.
    fn encode(value: &str) -> String {
        js_sys::encode_uri_component(value).into()
    }

    /// Ask the exchange where to send somebody.
    pub fn ask_where(url: &str, answers: &super::FromExchange) {
        let url = url.to_string();
        let answers = answers.clone();
        wasm_bindgen_futures::spawn_local(async move {
            // Anything other than a clear *configured* reads as login being off. The page
            // can do nothing different about an outage than about login being switched
            // off, and offering a button that cannot work is the one thing to avoid.
            let answered = match fetch(&url, None).await {
                Ok(body) => read_where(&body),
                Err(_) => None,
            };
            answers.put(super::Answered::Where(answered));
        });
    }

    /// Trade a code for a ticket.
    pub fn trade(url: &str, code: &str, answers: &super::FromExchange) {
        let url = url.to_string();
        let body = js_sys::JSON::stringify(&{
            let object = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&object, &"code".into(), &code.into());
            object.into()
        })
        .map(String::from)
        .unwrap_or_default();
        let answers = answers.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let answered = match fetch(&url, Some(&body)).await {
                Ok(body) => read_traded(&body),
                Err(why) => Err(why),
            };
            answers.put(super::Answered::Traded(answered));
        });
    }

    fn read_where(body: &str) -> Option<(String, String)> {
        let json = js_sys::JSON::parse(body).ok()?;
        if field(&json, "configured")?.as_bool() != Some(true) {
            return None;
        }
        Some((
            field(&json, "clientId")?.as_string()?,
            field(&json, "redirectUri")?.as_string()?,
        ))
    }

    fn read_traded(body: &str) -> Result<super::Held, String> {
        let Ok(json) = js_sys::JSON::parse(body) else {
            return Err("The sign-in service answered something unreadable.".to_string());
        };
        if let Some(error) = field(&json, "error").and_then(|value| value.as_string()) {
            // Discord's own wording is in `detail` and is for a log rather than a person:
            // `invalid_grant` means nothing to anybody but whoever set the redirect up.
            return Err(match error.as_str() {
                "refused" => "Discord would not accept that sign-in. Try again.".to_string(),
                "unreachable" => "Discord could not be reached. Try again shortly.".to_string(),
                _ => "Signing in is not available right now.".to_string(),
            });
        }
        let read = |name: &str| field(&json, name).and_then(|value| value.as_string());
        let expires_at = field(&json, "expiresAt")
            .and_then(|value| value.as_f64())
            .map(|seconds| seconds as u64);
        match (read("ticket"), read("name"), expires_at) {
            (Some(ticket), Some(name), Some(expires_at)) => Ok(super::Held {
                ticket,
                name,
                expires_at,
            }),
            _ => Err("The sign-in service answered without a ticket.".to_string()),
        }
    }

    fn field(json: &wasm_bindgen::JsValue, name: &str) -> Option<wasm_bindgen::JsValue> {
        js_sys::Reflect::get(json, &name.into()).ok()
    }

    /// One request to the exchange. `None` is a GET; `Some(body)` is a POST of JSON.
    async fn fetch(url: &str, body: Option<&str>) -> Result<String, String> {
        use eframe::wasm_bindgen::JsCast;

        let window = web_sys::window().ok_or("no window")?;
        let options = web_sys::RequestInit::new();
        match body {
            None => options.set_method("GET"),
            Some(body) => {
                options.set_method("POST");
                options.set_body(&body.into());
            }
        }
        let request = web_sys::Request::new_with_str_and_init(url, &options)
            .map_err(|_| "The sign-in service address is not usable.".to_string())?;
        if body.is_some() {
            request
                .headers()
                .set("content-type", "application/json")
                .map_err(|_| "could not set a header".to_string())?;
        }
        let answered = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|_| "The sign-in service could not be reached.".to_string())?;
        let answered: web_sys::Response = answered
            .dyn_into()
            .map_err(|_| "The sign-in service answered oddly.".to_string())?;
        // The body is read whatever the status, because a refusal carries its reason in
        // the body and a page that only read successes could not say why.
        let text = wasm_bindgen_futures::JsFuture::from(
            answered.text().map_err(|_| "no body".to_string())?,
        )
        .await
        .map_err(|_| "The sign-in service answered without a body.".to_string())?;
        text.as_string()
            .ok_or_else(|| "The sign-in service answered without a body.".to_string())
    }

    /// One `<meta>` tag's content, or empty if it is missing or still a placeholder.
    fn from_meta(name: &str, placeholder: &str) -> String {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return String::new();
        };
        let Ok(Some(meta)) = document.query_selector(&format!("meta[name={name}]")) else {
            return String::new();
        };
        match meta.get_attribute("content") {
            Some(value) if value != placeholder => value,
            _ => String::new(),
        }
    }

    /// Where the relay is, according to the page.
    pub fn relay_from_page() -> String {
        from_meta("relay", PLACEHOLDER)
    }

    /// A browser socket, with the closures that feed it.
    ///
    /// They are one struct because a closure handed to JavaScript is dropped when this is,
    /// and JavaScript calling a dropped closure throws. What makes that safe is `Drop`
    /// below, which takes the handlers off before it closes.
    pub struct Socket {
        socket: web_sys::WebSocket,
        _opened: Closure<dyn FnMut()>,
        _message: Closure<dyn FnMut(web_sys::MessageEvent)>,
        _closed: Closure<dyn FnMut()>,
        _failed: Closure<dyn FnMut()>,
    }

    impl Wire for Socket {
        fn send(&self, frame: &str) -> Result<(), String> {
            if self.socket.ready_state() != web_sys::WebSocket::OPEN {
                return Err("The socket is not open.".to_string());
            }
            self.socket
                .send_with_str(frame)
                .map_err(|_| "The socket would not take a frame.".to_string())
        }
    }

    impl Drop for Socket {
        fn drop(&mut self) {
            // Detached before the close, not after, and the order is the whole point:
            // `close` dispatches `onclose` on a later task, by which time these closures
            // have been dropped with the struct — so closing first would hand JavaScript a
            // dead closure to call. Taking the handlers off means the event has nowhere to
            // land. It also means a socket dropped because the page is finishing with it
            // cannot report its own close as a mishap, which would otherwise overwrite the
            // real reason the page let go of it.
            self.socket.set_onopen(None);
            self.socket.set_onmessage(None);
            self.socket.set_onclose(None);
            self.socket.set_onerror(None);
            let _ = self.socket.close();
        }
    }

    /// Open a socket and send the opening frame once it is up.
    ///
    /// The opening frame is sent from `onopen` rather than from here, because a socket
    /// refuses a frame until it is open and this returns long before that.
    pub fn open(
        url: &str,
        opening: &str,
        arrivals: &Arrivals,
        mishap: &Mishap,
    ) -> Result<Box<dyn Wire>, String> {
        let socket = web_sys::WebSocket::new(url)
            .map_err(|_| format!("Nothing could open a socket to {url}."))?;

        let opened = {
            let socket = socket.clone();
            let opening = opening.to_string();
            let mishap = mishap.clone();
            Closure::<dyn FnMut()>::new(move || {
                if socket.send_with_str(&opening).is_err() {
                    mishap.say("The relay would not take the opening frame.");
                }
            })
        };
        socket.set_onopen(Some(opened.as_ref().unchecked_ref()));

        let message = {
            let arrivals = arrivals.clone();
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                if let Some(text) = event.data().as_string() {
                    arrivals.push(text);
                }
            })
        };
        socket.set_onmessage(Some(message.as_ref().unchecked_ref()));

        let closed = {
            let mishap = mishap.clone();
            Closure::<dyn FnMut()>::new(move || mishap.say("The relay closed the socket."))
        };
        socket.set_onclose(Some(closed.as_ref().unchecked_ref()));

        let failed = {
            let mishap = mishap.clone();
            Closure::<dyn FnMut()>::new(move || mishap.say("The socket failed."))
        };
        socket.set_onerror(Some(failed.as_ref().unchecked_ref()));

        Ok(Box::new(Socket {
            socket,
            _opened: opened,
            _message: message,
            _closed: closed,
            _failed: failed,
        }))
    }
}

/// There is no transport outside a browser, and this says so rather than growing a second
/// one.
///
/// A native build exists so that the screens, the queue and the choice of screen are
/// compiled and tested by the same gate as everything else. A second socket implementation
/// here would be a second thing to keep in step with the first, tested by nothing, to no
/// end: `apps/probe` already plays whole matches over a real socket from a terminal.
#[cfg(not(target_arch = "wasm32"))]
mod socket {
    use super::{Arrivals, Mishap, Wire};

    pub fn relay_from_page() -> String {
        String::new()
    }

    /// Fixed, because a desktop build of this page is a compile check and not a client.
    pub fn handle() -> Option<String> {
        None
    }

    pub fn guest_id(_handle: Option<&str>) -> String {
        "0".repeat(32)
    }

    /// Nothing outside a browser, so login is off and every path through the page that
    /// depends on it takes the branch it would take for a deployment with no exchange.
    pub fn auth_from_page() -> String {
        String::new()
    }

    pub fn code_from_url() -> Option<String> {
        None
    }

    pub fn forget_code() {}

    pub fn ticket_kept(_handle: Option<&str>) -> Option<super::Held> {
        None
    }

    pub fn keep_ticket(_handle: Option<&str>, _held: &super::Held) {}

    pub fn forget_ticket(_handle: Option<&str>) {}

    pub fn authorize(_client_id: &str, _redirect_uri: &str) {}

    /// Answers at once, and answers that login is off.
    ///
    /// Not an empty stub: it puts the same answer a deployment with no exchange would, so
    /// a desktop build takes the branch a real page takes rather than a branch of its own.
    pub fn ask_where(_url: &str, answers: &super::FromExchange) {
        answers.put(super::Answered::Where(None));
    }

    pub fn trade(_url: &str, _code: &str, answers: &super::FromExchange) {
        answers.put(super::Answered::Traded(Err(
            "Signing in needs a browser.".to_string()
        )));
    }

    pub fn open(
        _url: &str,
        _opening: &str,
        _arrivals: &Arrivals,
        _mishap: &Mishap,
    ) -> Result<Box<dyn Wire>, String> {
        Err("A networked match needs a browser. Try apps/probe from a terminal.".to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    eframe::run_native(
        "Liars Dice multiplayer",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(Page::new(Waker::over(cc.egui_ctx.clone()))))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast;

    let document = web_sys::window()
        .expect("a window")
        .document()
        .expect("a document");
    let canvas = document
        .get_element_by_id("table")
        .expect("the page has a canvas called table")
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .expect("that element is a canvas");

    wasm_bindgen_futures::spawn_local(async {
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(Page::new(Waker::over(cc.egui_ctx.clone()))))),
            )
            .await
            .expect("the match started");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A waker that counts instead of painting.
    ///
    /// The simulated outside world for the one observation this page cannot make for
    /// itself: whether a frame will happen. Nothing here can run a frame, so what is
    /// asserted is that the page asked for one.
    #[derive(Clone, Default)]
    struct Counted(Rc<Cell<u32>>);

    impl Counted {
        fn waker(&self) -> Waker {
            let count = self.0.clone();
            Waker(Rc::new(move || count.set(count.get() + 1)))
        }

        fn wakes(&self) -> u32 {
            self.0.get()
        }
    }

    /// A socket that keeps what it was told.
    struct Recorder(Rc<RefCell<Vec<String>>>);

    impl Wire for Recorder {
        fn send(&self, frame: &str) -> Result<(), String> {
            self.0.borrow_mut().push(frame.to_string());
            Ok(())
        }
    }

    /// A page holding an open socket, which is the state the dashboard screen is in.
    fn connected(page: &mut Page) -> Rc<RefCell<Vec<String>>> {
        let said = Rc::new(RefCell::new(Vec::new()));
        page.wire = Some(Box::new(Recorder(said.clone())));
        said
    }

    const AT_THE_TABLE: Stance = Stance {
        over: false,
        started: true,
        my_turn: true,
        playing: true,
    };

    /// Every combination of what a seat can say about itself.
    fn every_stance() -> Vec<Stance> {
        let mut all = Vec::new();
        for over in [false, true] {
            for started in [false, true] {
                for my_turn in [false, true] {
                    for playing in [false, true] {
                        all.push(Stance {
                            over,
                            started,
                            my_turn,
                            playing,
                        });
                    }
                }
            }
        }
        all
    }

    /// `A player in a browser is chosen by a handle`, and only by one that is readable.
    #[test]
    fn a_fragment_names_a_handle_or_it_does_not() {
        assert_eq!(handle_in("#as=bob"), Some("bob".to_string()));
        assert_eq!(
            handle_in("as=bob"),
            Some("bob".to_string()),
            "the hash is optional"
        );
        assert_eq!(
            handle_in("#match=abc&as=bob"),
            Some("bob".to_string()),
            "and it need not be first"
        );
        assert_eq!(handle_in("#match=abc"), None, "no handle here");
        assert_eq!(handle_in(""), None);
        assert_eq!(handle_in("#as="), None, "an empty handle is not a handle");
        // Nothing is percent-decoded, and nothing needs to be: a handle is letters,
        // digits, dashes and underscores, none of which are ever encoded. So an encoded
        // value is simply not a handle.
        assert_eq!(
            handle_in("#as=%20%20"),
            None,
            "an encoded value is not a handle"
        );
    }

    /// Refused rather than repaired.
    ///
    /// A handle that is quietly cleaned up keys a different identity than the one that was
    /// typed, so the person silently becomes a different player — which is the exact
    /// failure this whole unit exists to stop.
    #[test]
    fn a_handle_that_is_not_one_is_refused_rather_than_tidied() {
        assert_eq!(clean_handle("bob"), Some("bob".to_string()));
        assert_eq!(
            clean_handle("  bob  "),
            Some("bob".to_string()),
            "surrounding space is not part of it"
        );
        assert_eq!(clean_handle("bob-2_x"), Some("bob-2_x".to_string()));
        assert_eq!(clean_handle(""), None);
        assert_eq!(clean_handle("   "), None);
        assert_eq!(
            clean_handle("bob smith"),
            None,
            "a space inside is refused, not stripped"
        );
        assert_eq!(
            clean_handle("bob/../alice"),
            None,
            "and nothing that could reshape a key"
        );
        assert_eq!(
            clean_handle(&"b".repeat(HANDLE_LIMIT)),
            Some("b".repeat(HANDLE_LIMIT))
        );
        assert_eq!(
            clean_handle(&"b".repeat(HANDLE_LIMIT + 1)),
            None,
            "too long is refused whole"
        );
    }

    /// `A browser holds one player until another is chosen`.
    ///
    /// Asserted against the literal key, because this is the test that fails if anybody
    /// renames it — and renaming it takes the seat of every player who already has an id.
    #[test]
    fn a_browser_with_no_handle_uses_the_key_it_always_used() {
        assert_eq!(storage_key("liars-dice-guest", None), "liars-dice-guest");
        assert_eq!(storage_key("liars-dice-ticket", None), "liars-dice-ticket");
    }

    /// Two handles are two keyrings, and `A handle names a player whether or not they have
    /// authenticated` — so the ticket is filed beside the id, not shared across handles.
    #[test]
    fn each_handle_gets_its_own_keyring() {
        let bob = storage_key("liars-dice-guest", Some("bob"));
        let alice = storage_key("liars-dice-guest", Some("alice"));
        assert_ne!(bob, alice, "two handles are two players");
        assert_ne!(
            bob,
            storage_key("liars-dice-guest", None),
            "and neither is the default one"
        );
        assert_ne!(
            storage_key("liars-dice-ticket", Some("bob")),
            storage_key("liars-dice-guest", Some("bob")),
            "a ticket and an id are different belongings of the same player"
        );
        assert_eq!(
            storage_key("liars-dice-ticket", Some("bob")),
            "liars-dice-ticket:bob"
        );
    }

    /// The scrub is per key, because a link naming a match is meant to be shared and a
    /// link naming a player is not.
    #[test]
    fn taking_the_handle_out_leaves_the_rest_of_the_fragment() {
        assert_eq!(fragment_without_handle("#as=bob"), "");
        assert_eq!(fragment_without_handle("#as=bob&match=abc"), "match=abc");
        assert_eq!(fragment_without_handle("#match=abc&as=bob"), "match=abc");
        assert_eq!(
            fragment_without_handle("#match=abc"),
            "match=abc",
            "nothing to remove, nothing removed"
        );
        assert_eq!(fragment_without_handle(""), "");
    }

    /// `An unattended client stays able to act`.
    ///
    /// A page sitting on the dashboard holds an open socket and no seat, and the keepalive
    /// that stops API Gateway dropping it runs once per frame. Asking for frames only
    /// while seated is how that keepalive came to be unreachable on the one screen people
    /// wait on.
    #[test]
    fn a_page_holding_a_socket_is_owed_frames_whether_or_not_it_has_a_seat() {
        let mut page = Page::new(Counted::default().waker());
        assert_eq!(
            page.frame_owed(),
            None,
            "nothing is open, so nothing is owed"
        );
        connected(&mut page);
        assert!(page.seated.is_none(), "no seat, which is the whole point");
        assert_eq!(
            page.frame_owed(),
            Some(TICK),
            "a socket with nobody sitting at it still has to be kept alive"
        );
    }

    /// The keepalive itself, which was always correct and never reached.
    #[test]
    fn a_silent_page_speaks_once_every_four_minutes() {
        let mut page = Page::new(Counted::default().waker());
        let said = connected(&mut page);
        page.spoke = 0.0;

        page.keep_the_socket(KEEPALIVE - 0.1);
        assert!(
            said.borrow().is_empty(),
            "nothing is owed yet: {:?}",
            said.borrow()
        );

        page.keep_the_socket(KEEPALIVE);
        assert_eq!(said.borrow().len(), 1, "one keepalive, once it is due");
        assert_eq!(
            said.borrow()[0],
            "",
            "an empty frame, which the relay reads as no message at all"
        );

        page.keep_the_socket(KEEPALIVE + 1.0);
        assert_eq!(said.borrow().len(), 1, "and not once per frame after that");
    }

    /// `An observation that arrives unasked wakes the program`, for each of the three
    /// places an observation can arrive: a relay frame, a failed socket, and the
    /// exchange's answer.
    #[test]
    fn everything_that_arrives_unasked_wakes_the_page() {
        let counted = Counted::default();

        let arrivals = Arrivals::new(counted.waker());
        arrivals.push("a frame".to_string());
        assert_eq!(counted.wakes(), 1, "a frame off the socket wakes the page");

        let mishap = Mishap::new(counted.waker());
        mishap.say("the socket failed");
        assert_eq!(counted.wakes(), 2, "so does a socket that failed");

        let from_exchange = FromExchange::new(counted.waker());
        from_exchange.put(Answered::Where(None));
        assert_eq!(counted.wakes(), 3, "so does the exchange answering");
    }

    /// Taking from a mailbox is not an arrival, so draining does not ask for another
    /// frame. Otherwise every frame would ask for the next one and the page would never
    /// be allowed to sleep.
    #[test]
    fn reading_a_mailbox_wakes_nobody() {
        let counted = Counted::default();
        let arrivals = Arrivals::new(counted.waker());
        arrivals.push("a frame".to_string());
        let before = counted.wakes();
        assert_eq!(arrivals.take(), Some("a frame".to_string()));
        assert_eq!(arrivals.take(), None);
        assert_eq!(counted.wakes(), before, "draining is not an arrival");
    }

    /// Frames are applied in the order they arrived, which is the order the log is in,
    /// which is the only order anything may be applied in.
    #[test]
    fn the_frame_queue_keeps_its_order() {
        let arrivals = Arrivals::new(Counted::default().waker());
        // Through a clone, because that is how it is used: the socket callback holds one
        // handle and the frame loop holds another.
        let callback = arrivals.clone();
        for frame in ["first", "second", "third"] {
            callback.push(frame.to_string());
        }
        let mut drained = Vec::new();
        while let Some(frame) = arrivals.take() {
            drained.push(frame);
        }
        assert_eq!(drained, vec!["first", "second", "third"]);
        assert_eq!(arrivals.take(), None, "a drained queue is empty");
    }

    /// A call is taken exactly once, so pressing a button sends one claim and not a claim
    /// per frame until the relay answers.
    #[test]
    fn a_call_put_in_a_hand_is_taken_once() {
        use coordinator::Decides;

        let hand = Hand::default();
        let mut driver = hand.clone();
        let view = rules::Game::start(vec![0, 1], rules::Variants::default());
        let view = view.view(0);

        assert_eq!(driver.choose(&view), None, "an untouched hand has no call");
        hand.0.set(Some(Call::Liar));
        assert_eq!(driver.choose(&view), Some(Call::Liar));
        assert_eq!(driver.choose(&view), None, "and not a second time");
    }

    #[test]
    fn with_no_seat_there_is_nothing_to_show_but_the_entry() {
        assert_eq!(screen_for(false, None, false), Screen::SignIn);
        assert_eq!(screen_for(false, None, true), Screen::SignIn);
    }

    /// A finished match shows its result whatever else is true of it, including when it is
    /// this seat's turn in a round that will never be played.
    #[test]
    fn a_finished_match_shows_its_result() {
        for stance in every_stance().into_iter().filter(|stance| stance.over) {
            for quiet in [false, true] {
                assert_eq!(
                    screen_for(true, Some(stance), quiet),
                    Screen::Over,
                    "{stance:?} quiet: {quiet}"
                );
            }
        }
    }

    /// Before the start there is no view to draw from, so nothing but the lobby can be shown.
    #[test]
    fn a_match_that_has_not_started_is_waited_for() {
        for stance in every_stance()
            .into_iter()
            .filter(|stance| !stance.over && !stance.started)
        {
            for quiet in [false, true] {
                assert_eq!(
                    screen_for(true, Some(stance), quiet),
                    Screen::Lobby,
                    "{stance:?} quiet: {quiet}"
                );
            }
        }
    }

    /// A seat is never waiting on itself, however long it sits there. The match is stopped
    /// because nobody has pressed anything on this screen, and saying otherwise would blame
    /// somebody else for it.
    #[test]
    fn a_seat_is_never_halted_on_its_own_turn() {
        for stance in every_stance().into_iter().filter(|stance| stance.my_turn) {
            for quiet in [false, true] {
                assert_ne!(
                    screen_for(true, Some(stance), quiet),
                    Screen::Halted,
                    "{stance:?} quiet: {quiet}"
                );
            }
        }
    }

    /// Holding no dice is watching, and holding dice is playing. The one difference between
    /// the two screens is the dialler, and this is the fact it is drawn from.
    #[test]
    fn a_seat_with_no_dice_watches_and_a_seat_with_dice_plays() {
        let mine = Stance {
            playing: true,
            ..AT_THE_TABLE
        };
        assert_eq!(screen_for(true, Some(mine), false), Screen::Playing);
        let out = Stance {
            playing: false,
            ..AT_THE_TABLE
        };
        assert_eq!(screen_for(true, Some(out), false), Screen::Watching);
    }

    /// Quiet decides nothing while this seat has the turn, and everything while another
    /// seat has it — including for a seat that is only watching, which is the case the whole
    /// mechanism exists for.
    #[test]
    fn a_quiet_wait_on_another_seat_is_halted_and_a_short_one_is_not() {
        for playing in [false, true] {
            let theirs = Stance {
                my_turn: false,
                playing,
                ..AT_THE_TABLE
            };
            assert_eq!(
                screen_for(true, Some(theirs), false),
                if playing {
                    Screen::Playing
                } else {
                    Screen::Watching
                },
                "a short wait is not a halt"
            );
            assert_eq!(screen_for(true, Some(theirs), true), Screen::Halted);
        }
    }

    /// A call is offered only on this seat's turn, and the reason is not that it would be
    /// refused otherwise: it would be kept, and played later without being asked for again.
    #[test]
    fn a_call_is_offered_only_on_this_seats_own_turn() {
        for stance in every_stance() {
            let offered = may_call(stance);
            if offered {
                assert!(stance.my_turn, "a call was offered out of turn: {stance:?}");
                assert!(
                    stance.started && !stance.over,
                    "a call was offered outside a running match: {stance:?}"
                );
                assert!(
                    stance.playing,
                    "a call was offered to a seat holding no dice: {stance:?}"
                );
            }
        }
        assert!(
            may_call(AT_THE_TABLE),
            "and it is offered when every one of them holds"
        );
    }

    /// Whatever is showing at the table, a call is never offered on a screen that is
    /// waiting on somebody else — which is what makes the dialler safe to leave there.
    #[test]
    fn nothing_can_be_called_from_a_halted_screen() {
        for stance in every_stance() {
            if screen_for(true, Some(stance), true) == Screen::Halted {
                assert!(!may_call(stance), "halted and callable: {stance:?}");
            }
        }
    }

    /// A page says it is a guest, with the id this browser keeps and the name typed on it.
    ///
    /// This test used to assert the opposite — that the account *was* the typed name — and
    /// the relay's log has since shown what that cost: two people who both typed the same
    /// name were one account, and the second would land on the first's seat. The name is
    /// now display and the id is identity.
    #[test]
    fn a_page_says_it_is_a_guest_with_the_id_this_browser_keeps() {
        let mut page = Page::new(Counted::default().waker());
        page.name = "  Sean ".to_string();
        let once = page.who();
        assert_eq!(once, page.who(), "asking twice gives the same answer");
        assert_eq!(
            once,
            Who::Guest {
                id: page.guest.clone(),
                name: "Sean".to_string(),
            },
            "the name is trimmed, so a stray space is not a different person"
        );
    }

    /// Two pages with different ids are different players, whatever they are called.
    ///
    /// The collision this closes was live on the published page: the account was
    /// `web-<name>`, so everybody called Ada was one account.
    #[test]
    fn the_same_name_in_two_browsers_is_two_players() {
        let mut one = Page::new(Counted::default().waker());
        let mut other = Page::new(Counted::default().waker());
        one.name = "Ada".to_string();
        other.name = "Ada".to_string();
        // The desktop build has no local storage, so the ids are equal there and the test
        // sets them apart to say what the browser's `guest_id` guarantees.
        one.guest = "1".repeat(32);
        other.guest = "2".repeat(32);
        assert_ne!(one.who(), other.who());
    }

    /// Outside a browser there is no socket, and the sign-in screen is told so rather than
    /// the page pretending it reached the relay.
    #[test]
    fn without_a_browser_nothing_is_connected() {
        let mut page = Page::new(Counted::default().waker());
        page.name = "Sean".to_string();
        page.relay = "wss://nowhere/live".to_string();
        page.connect(0.0);
        assert!(page.wire.is_none(), "there is no socket outside a browser");
        assert!(page.seated.is_none(), "and so no seat either");
        assert!(page.trouble.is_some(), "and the reason is on the screen");
        assert_eq!(
            screen_for(page.wire.is_some(), page.stance(), false),
            Screen::SignIn
        );
    }

    fn seated_as(colours: &[&str]) -> Vec<Player> {
        colours
            .iter()
            .map(|colour| Player {
                name: "Somebody".to_string(),
                colour: (*colour).to_string(),
                left: false,
            })
            .collect()
    }

    /// A seat's own colour is shown as chosen and never counted as taken.
    ///
    /// The off-by-one this function exists to prevent: if a seat counted itself, the
    /// colour it is wearing would be the one colour it could not keep, and the picker
    /// would offer nineteen colours none of which is the current one.
    #[test]
    fn a_seat_does_not_hold_its_own_colour_against_itself() {
        let roster = seated_as(&["white", "black", "magenta"]);
        let (chosen, taken) = choosable(&roster, Some(1));
        assert_eq!(chosen, index_of("black"));
        assert_eq!(
            taken,
            vec![
                index_of("white").expect("white"),
                index_of("magenta").expect("magenta")
            ]
        );
        assert!(
            !taken.contains(&index_of("black").expect("black")),
            "a seat must be able to keep the colour it has"
        );
    }

    /// Every other seat's colour is taken, including one that has left.
    ///
    /// A seat that left still holds its colour in the log, and `A player who leaves a
    /// match is eliminated` rather than un-joined, so the colour is not free to take.
    #[test]
    fn a_departed_seat_still_holds_its_colour() {
        let mut roster = seated_as(&["white", "black"]);
        roster[1].left = true;
        let (_, taken) = choosable(&roster, Some(0));
        assert_eq!(taken, vec![index_of("black").expect("black")]);
    }

    /// A colour the palette does not know is no reason to offer a swatch, or to hide one.
    ///
    /// Reachable from a log written by a client that sent something else, which the relay
    /// does not police: it refuses a colour another seat holds and says nothing about what
    /// a colour is. So this returns no choice rather than an index into nothing.
    #[test]
    fn a_colour_the_palette_does_not_know_chooses_nothing() {
        let roster = seated_as(&["chartreuse", "black"]);
        let (chosen, taken) = choosable(&roster, Some(0));
        assert_eq!(chosen, None);
        assert_eq!(taken, vec![index_of("black").expect("black")]);
    }

    /// Holding no seat yet, so nothing is chosen and everything anybody holds is taken.
    #[test]
    fn a_page_with_no_seat_has_no_colour_of_its_own() {
        let roster = seated_as(&["white"]);
        let (chosen, taken) = choosable(&roster, None);
        assert_eq!(chosen, None);
        assert_eq!(taken, vec![index_of("white").expect("white")]);
    }

    /// A socket and no seat is the dashboard, whatever else is true, and no seat and no socket
    /// is the entry. Nothing else distinguishes the two screens, which is why connecting
    /// is a step of its own rather than part of hosting.
    #[test]
    fn a_socket_without_a_seat_is_the_dashboard() {
        for quiet in [false, true] {
            assert_eq!(screen_for(true, None, quiet), Screen::Dashboard);
            assert_eq!(screen_for(false, None, quiet), Screen::SignIn);
        }
    }

    /// Being at a match beats being connected: a seat is always drawn as the seat it is,
    /// never as the dashboard it was reached from.
    #[test]
    fn a_seat_is_never_drawn_as_the_dashboard() {
        for stance in every_stance() {
            for quiet in [false, true] {
                assert_ne!(
                    screen_for(true, Some(stance), quiet),
                    Screen::Dashboard,
                    "{stance:?} quiet: {quiet}"
                );
            }
        }
    }

    /// The countdown the host is shown, at the boundaries.
    ///
    /// Rounding up matters in both directions. A number that rounded down would say
    /// *zero* while the relay still refuses, which is a button that looks broken; and it
    /// has to stop at nothing rather than go negative, because a countdown that has run
    /// out has nothing to say.
    #[test]
    fn the_countdown_rounds_up_and_runs_out() {
        assert_eq!(remaining(None, 100.0), None, "nothing was refused");
        // The relay said sixty at frame-clock time ten.
        let said = Some((60u32, 10.0));
        assert_eq!(remaining(said, 10.0), Some(60), "as it was answered");
        assert_eq!(remaining(said, 10.5), Some(60), "59.5 left reads as 60");
        assert_eq!(remaining(said, 11.0), Some(59));
        assert_eq!(remaining(said, 68.8), Some(2), "1.2 left reads as 2");
        assert_eq!(
            remaining(said, 69.5),
            Some(1),
            "half a second is still a second"
        );
        assert_eq!(
            remaining(said, 70.0),
            None,
            "and then there is nothing to say"
        );
        assert_eq!(remaining(said, 600.0), None, "never negative");
    }
}
