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
use protocol::Account;
use render_minimal::palette::{PALETTE, index_of};
use render_minimal::{
    Seating, calls, dialler, die, history, odds, opening_claim, palette_picker, ranking, standing,
    table, tap, verdict,
};
use rules::{Call, Claim, View};
use seat::{Ending, Listed, MatchId, Seated, Stopped, lobby};

/// How long another seat may go quiet before the screen says so.
///
/// A client cannot tell an absent player from a slow one: presence is not in the log, by
/// design, and nothing else is coming. So this is a threshold on what has been *heard*,
/// never a claim about who is there, and crossing it changes no rule and sends no message.
const QUIET: f64 = 20.0;

/// How often the page redraws while a match is live.
///
/// Arrivals wake the frame loop themselves, so this exists only to notice silence: without
/// it the screen could not say that nothing has happened, because nothing happening is not
/// an event.
const TICK: Duration = Duration::from_millis(500);

/// The swatch a colour nobody recognises is drawn in. Unreachable through the picker.
const UNKNOWN_COLOUR: usize = 19;

/// As many as a match holds. The relay refuses the eleventh, and the lobby says so before
/// anybody tries.
const MAX_SEATS: usize = 10;

/// The frames that have arrived, oldest first.
///
/// `Rc<RefCell<_>>` rather than `Arc<Mutex<_>>` because a tab is one thread and a lock
/// nobody contends is a lie about the code.
#[derive(Clone, Default)]
struct Arrivals(Rc<RefCell<VecDeque<String>>>);

impl Arrivals {
    /// Pushed to from the socket's message callback, which only a browser has.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(
            dead_code,
            reason = "the only caller outside a test is the browser socket"
        )
    )]
    fn push(&self, frame: String) {
        self.0.borrow_mut().push_back(frame);
    }

    fn take(&self) -> Option<String> {
        self.0.borrow_mut().pop_front()
    }
}

/// What the transport has to say, which is never a relay frame.
///
/// A socket that will not open, or that closes, is not something the protocol has a word
/// for, so it is kept apart from [`Arrivals`] rather than disguised as a frame that failed
/// to parse.
#[derive(Clone, Default)]
struct Mishap(Rc<RefCell<Option<String>>>);

impl Mishap {
    /// Said by the socket, which only a browser has.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(dead_code, reason = "the only caller is the browser socket")
    )]
    fn say(&self, trouble: impl Into<String>) {
        *self.0.borrow_mut() = Some(trouble.into());
    }

    fn take(&self) -> Option<String> {
        self.0.borrow_mut().take()
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
    Entry,
    /// Connected and not at a match: what is open, and the two ways to be at one.
    Lobby,
    /// In a match that has not started.
    Waiting,
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
        // A socket and no seat is the lobby. The two are separate because the lobby is
        // what a client does *before* it knows whether it will host, join, or just look,
        // and that question is asked over the socket it will go on to play through.
        return if connected {
            Screen::Lobby
        } else {
            Screen::Entry
        };
    };
    if stance.over {
        return Screen::Over;
    }
    if !stance.started {
        return Screen::Waiting;
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

fn may_call(stance: Stance) -> bool {
    stance.started && !stance.over && stance.playing && stance.my_turn
}

/// The page.
struct Page {
    /// Where the relay is. Read from the page, and editable so a local build can reach a
    /// deployed relay.
    relay: String,
    /// What this person is called. Also what the relay is told their account is — asserted,
    /// never proven, which is the hole the identity service closes.
    name: String,
    colour: usize,
    /// The name the host gives the match, which is what its identity is derived from. Host
    /// twice under one name and the relay answers with the match that already exists, which
    /// is `A repeated request to host creates no second match` made useful rather than
    /// merely true.
    match_name: String,
    /// The id pasted in to join somebody else's match.
    paste: String,
    /// What to tell the person on the entry screen.
    trouble: Option<String>,

    seated: Option<Seated>,
    /// What is open to join, as the relay last said. Stale by the time it is drawn, which
    /// is why joining one can still be refused.
    open: Vec<Listed>,
    /// Whether the relay has answered the lobby yet, so an empty list can be told from a
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
    /// How long the host must still wait to cancel, and when the relay said so.
    ///
    /// Display only. The relay decides whether a cancel may happen — it is the only thing
    /// that knows when the match last moved — and this exists so the number counts down
    /// while somebody watches it instead of sitting still. Nothing here is a decision.
    countdown: Option<(u32, f64)>,
    log_shown: usize,
}

impl Page {
    fn new() -> Self {
        Self {
            relay: socket::relay_from_page(),
            name: String::new(),
            colour: 1,
            match_name: String::new(),
            paste: String::new(),
            trouble: None,
            seated: None,
            open: Vec::new(),
            answered: false,
            wire: None,
            arrivals: Arrivals::default(),
            mishap: Mishap::default(),
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
    /// Derived from the typed name so that nothing taken from a clock reaches the log, and
    /// so that hosting the same match name twice really is the same request. Two people
    /// who type the same name share an account, which is exactly the hole recorded under
    /// Decisions in the document: the relay is given an account it cannot check.
    fn account(&self) -> Account {
        Account::Discord(format!("web-{}", self.name.trim()))
    }

    /// Open a socket and ask what is open.
    ///
    /// The lobby's question is the opening frame, so a client is connected before it has
    /// decided anything. Hosting and joining both happen later on this same socket.
    fn connect(&mut self, now: f64) {
        self.trouble = None;
        self.arrivals = Arrivals::default();
        self.mishap = Mishap::default();
        self.hand = Hand::default();
        self.open = Vec::new();
        self.answered = false;
        self.heard = now;
        let asking = lobby::ask(&self.account());
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
        let asking = lobby::ask(&self.account());
        self.say(vec![asking]);
    }

    /// Leave the match but not the relay, which is where somebody goes to play another.
    fn leave_seat(&mut self, trouble: Option<String>) {
        self.seated = None;
        self.trouble = trouble;
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
            // The lobby's answer is not about any seat, and can arrive after one has been
            // taken, because the socket that asked is the socket that sat down.
            if let Some(open) = lobby::listing(&frame) {
                self.open = open;
                self.answered = true;
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
                    // Going back to the lobby keeps the connection and says what happened.
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
        self.pump(now);
        if self.seated.is_some() {
            ui.ctx().request_repaint_after(TICK);
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
            Screen::Entry => self.entry(ui, now),
            Screen::Lobby => self.lobby(ui),
            Screen::Waiting => self.waiting(ui, now),
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

    /// A name, a colour, and either host or join.
    fn entry(&mut self, ui: &mut egui::Ui, now: f64) {
        ui.heading("Liars Dice");
        ui.label(
            "A match over the network. One seat per tab, one person per seat, and the \
             dice are dealt by a relay that cannot see them either.",
        );
        ui.separator();

        if let Some(trouble) = &self.trouble {
            ui.label(egui::RichText::new(trouble).strong());
            ui.add_space(6.0);
        }

        ui.label(egui::RichText::new("Your name").strong());
        ui.add_sized(
            [240.0, 30.0],
            egui::TextEdit::singleline(&mut self.name).hint_text("name"),
        );
        ui.label(
            egui::RichText::new(
                "Nothing checks this yet, so it is also who the relay thinks you are.",
            )
            .size(12.0),
        );

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Your dice").strong());
        palette_picker(ui, &mut self.colour);
        ui.horizontal(|ui| {
            die(ui, self.colour, Some(5));
            ui.label(PALETTE[self.colour].name);
        });

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("The relay").strong());
        ui.add_sized(
            [460.0, 30.0],
            egui::TextEdit::singleline(&mut self.relay).hint_text("wss://host/stage"),
        );

        let named = !self.name.trim().is_empty();
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
    fn lobby(&mut self, ui: &mut egui::Ui) {
        ui.heading("Matches");
        if let Some(trouble) = &self.trouble {
            ui.label(egui::RichText::new(trouble).strong());
        }
        ui.label(
            egui::RichText::new(format!(
                "You are {}, playing {}.",
                self.name.trim(),
                PALETTE[self.colour].name
            ))
            .size(12.0),
        );
        ui.separator();

        let mut joining = None;
        if !self.answered {
            ui.label("asking the relay…");
        } else if self.open.is_empty() {
            ui.label("Nothing is open. Host one, and somebody can join it.");
        } else {
            egui::Grid::new("open").spacing([12.0, 8.0]).show(ui, |ui| {
                for open in &self.open {
                    ui.label(egui::RichText::new(&open.host).strong());
                    ui.label(match open.players {
                        1 => "1 player".to_string(),
                        many => format!("{many} players"),
                    });
                    if tap(ui, "Join", open.players < MAX_SEATS) {
                        joining = Some(open.match_id.clone());
                    }
                    ui.end_row();
                }
            });
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

        if hosting {
            let seated = Seated::hosting(
                self.account(),
                self.name.trim(),
                PALETTE[self.colour].name,
                self.match_name.trim(),
                Box::new(self.hand.clone()),
            );
            self.sit(seated);
        } else if let Some(match_id) = joining {
            let seated = Seated::joining(
                self.account(),
                self.name.trim(),
                PALETTE[self.colour].name,
                match_id,
                Box::new(self.hand.clone()),
            );
            self.sit(seated);
        }
    }

    /// In a match that has not started.
    fn waiting(&mut self, ui: &mut egui::Ui, now: f64) {
        let Some(seated) = &self.seated else {
            return;
        };
        let host = seated.host();
        let roster = seated.roster();
        let enough = roster.iter().filter(|player| !player.left).count() >= 2;

        ui.heading(if host {
            "Your match"
        } else {
            "Waiting for the host"
        });

        if let Some(match_id) = seated.match_id() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("The id to pass to somebody").strong());
            // A field rather than a label: a label cannot be selected, and this has to be
            // copied out of the page by hand until the lobby exists.
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

        ui.add_space(8.0);
        ui.separator();
        let start = if host {
            if tap(ui, "Start the match", enough) {
                seated.start()
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
            // Both buttons go back to the lobby, and say so differently on purpose: one
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

    /// Where the relay is, according to the page.
    pub fn relay_from_page() -> String {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return String::new();
        };
        let Ok(Some(meta)) = document.query_selector("meta[name=relay]") else {
            return String::new();
        };
        match meta.get_attribute("content") {
            Some(url) if url != PLACEHOLDER => url,
            _ => String::new(),
        }
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
        Box::new(|_| Ok(Box::new(Page::new()))),
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
                Box::new(|_| Ok(Box::new(Page::new()))),
            )
            .await
            .expect("the match started");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Frames are applied in the order they arrived, which is the order the log is in,
    /// which is the only order anything may be applied in.
    #[test]
    fn the_frame_queue_keeps_its_order() {
        let arrivals = Arrivals::default();
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
        assert_eq!(screen_for(false, None, false), Screen::Entry);
        assert_eq!(screen_for(false, None, true), Screen::Entry);
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

    /// Before the start there is no view to draw from, so nothing but Waiting can be shown.
    #[test]
    fn a_match_that_has_not_started_is_waited_for() {
        for stance in every_stance()
            .into_iter()
            .filter(|stance| !stance.over && !stance.started)
        {
            for quiet in [false, true] {
                assert_eq!(
                    screen_for(true, Some(stance), quiet),
                    Screen::Waiting,
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

    /// An account is derived from the name and from nothing else, so hosting the same match
    /// name twice really is the same request — `A repeated request to host creates no second
    /// match` — and nothing taken from a clock reaches the log.
    #[test]
    fn the_account_is_the_typed_name_and_nothing_else() {
        let mut page = Page::new();
        page.name = "  Sean ".to_string();
        let once = page.account();
        assert_eq!(once, page.account(), "asking twice gives the same account");
        assert_eq!(
            once,
            Account::Discord("web-Sean".to_string()),
            "the name is trimmed, so a stray space is not a different person"
        );
    }

    /// Outside a browser there is no socket, and the entry screen is told so rather than
    /// the page pretending it reached the relay.
    #[test]
    fn without_a_browser_nothing_is_connected() {
        let mut page = Page::new();
        page.name = "Sean".to_string();
        page.relay = "wss://nowhere/live".to_string();
        page.connect(0.0);
        assert!(page.wire.is_none(), "there is no socket outside a browser");
        assert!(page.seated.is_none(), "and so no seat either");
        assert!(page.trouble.is_some(), "and the reason is on the screen");
        assert_eq!(
            screen_for(page.wire.is_some(), page.stance(), false),
            Screen::Entry
        );
    }

    /// A socket and no seat is the lobby, whatever else is true, and no seat and no socket
    /// is the entry. Nothing else distinguishes the two screens, which is why connecting
    /// is a step of its own rather than part of hosting.
    #[test]
    fn a_socket_without_a_seat_is_the_lobby() {
        for quiet in [false, true] {
            assert_eq!(screen_for(true, None, quiet), Screen::Lobby);
            assert_eq!(screen_for(false, None, quiet), Screen::Entry);
        }
    }

    /// Being at a match beats being connected: a seat is always drawn as the seat it is,
    /// never as the lobby it was reached from.
    #[test]
    fn a_seat_is_never_drawn_as_the_lobby() {
        for stance in every_stance() {
            for quiet in [false, true] {
                assert_ne!(
                    screen_for(true, Some(stance), quiet),
                    Screen::Lobby,
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
