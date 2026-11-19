//! A hotseat match: every seat of one match in one browser, with the screen passed from
//! hand to hand.
//!
//! Composition root. Realizes /implementation/hotseat.md. The rules come from `rules`, the
//! match from `table`, the presentation from `render-minimal`, and what lives here is the
//! lobby, the handover, and the one driver a person sits behind.
//!
//! The whole arrangement rests on one thing: a screen is drawn from a [`rules::View`], and
//! a view returns `None` for another seat's dice until the round is revealed. So the panels
//! cannot show what the seat holding the screen may not see, and the only decision this
//! file makes about secrecy is *which* view to hand them. That decision is [`Match::step`],
//! and it is nine lines.

use std::cell::Cell;
use std::rc::Rc;

use eframe::egui;
use integrity::Secret;
use render_minimal::palette::{PALETTE, assign};
use render_minimal::{
    Seating, calls, colour_of, dialler, die, history, odds, opening_claim, ranking, standing,
    table, tap, verdict,
};
use rules::{Call, Claim, Resolution, View};
use table::{Ai, Decides, Table, Waiting};

/// As many as the palette keeps visibly distinct.
const MAX_SEATS: usize = 10;
/// Virtual players get names rather than numbers, so the screen reads as a sentence.
const VIRTUAL_NAMES: [&str; 9] = [
    "Alice", "Bob", "Carol", "Dave", "Erin", "Frank", "Grace", "Heidi", "Ivan",
];

/// A seat a person is playing.
///
/// The screen puts a call in and [`Table::advance`] takes it out, so a person reaches the
/// table through the same `Decides` trait an AI does and nothing between them branches on
/// which it is. `Rc<Cell<_>>` because the table owns the driver and the screen does not,
/// and a browser tab is one thread, so an `Arc<Mutex<_>>` would be a lock nobody contends.
#[derive(Clone, Default)]
struct Hand(Rc<Cell<Option<Call>>>);

impl Decides for Hand {
    fn choose(&mut self, _view: &View) -> Option<Call> {
        self.0.take()
    }
}

/// A seat in the lobby, before the match is launched.
struct Chair {
    name: String,
    /// Whether a person plays it. A virtual player is a driver and never an account, so
    /// this is the only place in the whole system that knows the difference.
    human: bool,
}

/// Which screen is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen {
    /// Seats are being filled and the match has not started.
    Lobby,
    /// The screen is being handed over, and shows no part of the match.
    Handover,
    /// The seat holding the screen is looking at its own view, and it is its turn.
    Playing,
    /// A round ended. A reveal is public, so whoever is holding the screen may see it.
    Outcome,
    /// The match is over. Also public.
    Over,
}

/// A match in progress, and who is looking at it.
struct Match {
    table: Table,
    seating: Seating,
    /// One per seat: the hand a person plays it with, or `None` for a virtual player.
    hands: Vec<Option<Hand>>,
    /// Whose chair the screen is in. The one piece of state that decides what is visible.
    holding: usize,
    /// The claim the seat holding the screen is dialling. Always legal.
    pending: Claim,
    /// The round's resolution, while it is being shown.
    outcome: Option<Resolution>,
}

impl Match {
    /// Seat everyone and start the match, through the relay's own messages.
    ///
    /// The same hosting and joining a networked lobby does — the only difference is that
    /// the host fills the seats instead of waiting for people to arrive, so this path is
    /// exercised by hotseat rather than bypassed by it.
    fn launch(lobby: &[Chair], secret: u8) -> Self {
        let colours = assign(1, lobby.len());
        let mut hands = Vec::new();
        let drivers = lobby
            .iter()
            .map(|chair| {
                let driver: Box<dyn Decides> = if chair.human {
                    let hand = Hand::default();
                    hands.push(Some(hand.clone()));
                    Box::new(hand)
                } else {
                    hands.push(None);
                    Box::new(Ai)
                };
                (chair.name.clone(), driver)
            })
            .collect();

        let table = Table::seat(Secret::from_bytes([secret; 32]), drivers);
        let seating = Seating::new(
            lobby.iter().map(|chair| chair.name.clone()).collect(),
            colours,
        );
        let pending = Claim {
            quantity: 1,
            face: 1,
        };
        Self {
            table,
            seating,
            hands,
            // The host is holding the screen when the match launches, and the host is the
            // first seat.
            holding: 0,
            pending,
            outcome: None,
        }
    }

    /// Whether anybody is playing by hand.
    fn anyone_human(&self) -> bool {
        self.hands.iter().any(Option::is_some)
    }

    /// The match as the seat holding the screen sees it.
    ///
    /// The only call in this app that chooses a view, which is why it takes `holding` and
    /// not a parameter: there is nowhere to pass the wrong seat from.
    fn view(&self) -> Option<View<'_>> {
        self.table.players.get(self.holding)?.client.view()
    }

    /// Advance the match and say what the screen should show.
    ///
    /// Every seat whose driver answers is played inside `advance`, so a virtual player
    /// takes its turn without the local player acting and without the screen changing
    /// hands. The screen is handed over when, and only when, the seat that has to be asked
    /// is not the seat already holding it.
    fn step(&mut self) -> Screen {
        match self.table.advance() {
            Waiting::Over => Screen::Over,
            Waiting::Outcome(resolution) => {
                self.outcome = Some(resolution);
                Screen::Outcome
            }
            Waiting::Call(seat) => {
                if seat == self.holding {
                    self.take_the_screen();
                    Screen::Playing
                } else {
                    self.holding = seat;
                    Screen::Handover
                }
            }
        }
    }

    /// Begin a turn for whoever is now holding the screen.
    fn take_the_screen(&mut self) {
        if let Some(view) = self.view() {
            self.pending = opening_claim(&view);
        }
    }

    /// Play a call for the seat holding the screen.
    fn play(&mut self, call: Call) -> Screen {
        if let Some(Some(hand)) = self.hands.get(self.holding) {
            hand.0.set(Some(call));
        }
        self.step()
    }

    /// What ended the round being shown, as a sentence.
    ///
    /// Read from the client holding the screen rather than from the relay's log, because
    /// that is what a real client has: the seat and the call come from its own stream, and
    /// the claim they disputed is the last one of the round, which is still there because
    /// the round has not been left behind yet.
    fn said(&self) -> Option<String> {
        let client = &self.table.players.get(self.holding)?.client;
        let (seat, call) = client.ended_by()?;
        let claim = client.public()?.history().last()?.claim;
        let resolution = self.outcome.as_ref()?;
        Some(verdict(
            &self.seating,
            seat.0 as usize,
            call,
            claim,
            resolution,
        ))
    }
}

/// The page.
struct Hotseat {
    screen: Screen,
    lobby: Vec<Chair>,
    playing: Option<Match>,
    /// Changed for each match so a new one deals differently.
    secret: u8,
    log_shown: usize,
}

impl Default for Hotseat {
    fn default() -> Self {
        Self {
            screen: Screen::Lobby,
            // One person and two virtual players: the smallest match that is worth
            // playing, and the one a single tester can play alone.
            lobby: vec![
                Chair {
                    name: "Player 1".to_string(),
                    human: true,
                },
                Chair {
                    name: VIRTUAL_NAMES[0].to_string(),
                    human: false,
                },
                Chair {
                    name: VIRTUAL_NAMES[1].to_string(),
                    human: false,
                },
            ],
            playing: None,
            secret: 11,
            log_shown: 10,
        }
    }
}

impl eframe::App for Hotseat {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            // Do not shrink to the content: a grid inside a shrinking scroll area is
            // given the narrowest width it can be drawn in, which squeezes every field in
            // it down to nothing.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| self.draw(ui));
        });
    }
}

impl Hotseat {
    fn draw(&mut self, ui: &mut egui::Ui) {
        match self.screen {
            Screen::Lobby => self.lobby(ui),
            Screen::Handover => self.handover(ui),
            Screen::Playing => self.playing(ui),
            Screen::Outcome => self.outcome(ui),
            Screen::Over => self.over(ui),
        }
    }

    /// The lobby: fill the seats, then launch.
    fn lobby(&mut self, ui: &mut egui::Ui) {
        ui.heading("Liars Dice");
        ui.label(
            "One screen, passed around. Everybody plays from their own seat and sees only \
             what that seat may see.",
        );
        ui.separator();

        let mut remove = None;
        // The colours a match of this size is actually dealt, so the lobby shows the dice
        // each seat will play with rather than a guess at them.
        let colours = assign(1, self.lobby.len());
        let can_remove = self.lobby.len() > 2;
        egui::Grid::new("lobby")
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                for (seat, chair) in self.lobby.iter_mut().enumerate() {
                    // A die rather than the colour's name: a name has to be drawn in some
                    // colour, and a dark one is invisible against a dark page.
                    die(ui, colours[seat], None);
                    ui.label(egui::RichText::new(PALETTE[colours[seat]].name).size(13.0));
                    // Forced rather than desired: inside a grid a desired width loses to
                    // the column, and the field collapses to a few characters.
                    ui.add_sized(
                        [160.0, 30.0],
                        egui::TextEdit::singleline(&mut chair.name).hint_text("name"),
                    );
                    // A person or a virtual player. Nothing but the driver differs.
                    if tap(ui, if chair.human { "person" } else { "virtual" }, true) {
                        chair.human = !chair.human;
                    }
                    if tap(ui, "remove", can_remove) {
                        remove = Some(seat);
                    }
                    ui.end_row();
                }
            });
        if let Some(seat) = remove {
            self.lobby.remove(seat);
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if tap(ui, "Add a seat", self.lobby.len() < MAX_SEATS) {
                let seat = self.lobby.len();
                self.lobby.push(Chair {
                    name: VIRTUAL_NAMES
                        .get(seat.saturating_sub(1))
                        .map_or_else(|| format!("Seat {seat}"), |name| (*name).to_string()),
                    human: false,
                });
            }
            ui.label(format!(
                "{} seats, {MAX_SEATS} at most, 2 at least",
                self.lobby.len()
            ));
        });

        let people = self.lobby.iter().filter(|chair| chair.human).count();
        ui.label(match people {
            0 => "No people: the match will play itself.".to_string(),
            1 => "One person. The screen never has to change hands.".to_string(),
            more => format!("{more} people. The screen is handed over between them."),
        });

        ui.add_space(6.0);
        ui.separator();
        if tap(ui, "Start the match", self.lobby.len() >= 2) {
            let mut playing = Match::launch(&self.lobby, self.secret);
            self.screen = playing.step();
            self.playing = Some(playing);
        }
    }

    /// The handover. Shows the name of the seat taking the screen and nothing else.
    ///
    /// `A handover screen shows no part of the match`, so there is no table, no claim, no
    /// log and no count here. Nothing stops the previous player from tapping the button
    /// and looking; that is inherent to one device, and every hotseat board game lives
    /// with it.
    fn handover(&mut self, ui: &mut egui::Ui) {
        let Some(playing) = self.playing.as_mut() else {
            self.screen = Screen::Lobby;
            return;
        };
        let name = playing.seating.name(playing.holding).to_string();
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("Pass the screen to").size(20.0));
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(&name)
                    .size(44.0)
                    .strong()
                    .color(colour_of(playing.seating.colour(playing.holding))),
            );
            ui.add_space(24.0);
            if tap(ui, &format!("I am {name}"), true) {
                playing.take_the_screen();
                self.screen = Screen::Playing;
            }
        });
    }

    /// One seat's turn, drawn from that seat's view.
    fn playing(&mut self, ui: &mut egui::Ui) {
        let Some(playing) = self.playing.as_mut() else {
            self.screen = Screen::Lobby;
            return;
        };

        // With nobody playing by hand there is nothing to wait for, so the match plays
        // itself a step at a time and is watched from the first seat.
        if !playing.anyone_human() {
            self.screen = playing.step();
            ui.ctx().request_repaint();
        }

        let mut pending = playing.pending;
        let call = {
            let Some(view) = playing.view() else {
                ui.label("the match has not started");
                return;
            };
            ui.horizontal(|ui| {
                ui.heading(playing.seating.name(playing.holding));
                ui.label(
                    egui::RichText::new(format!("round {}", view.public().round() + 1)).size(15.0),
                );
            });
            ui.separator();
            table(ui, &view, &playing.seating);
            ui.separator();
            standing(ui, &view, &playing.seating);
            history(ui, &view, &playing.seating);
            ui.separator();

            ui.label(egui::RichText::new("Your claim").strong());
            dialler(ui, &view, &mut pending);
            odds(ui, &view, pending);
            ui.separator();
            calls(ui, &view, pending)
        };
        playing.pending = pending;

        if let Some(call) = call {
            self.screen = playing.play(call);
            return;
        }

        ui.separator();
        self.events(ui);
    }

    /// A round's outcome, which is public once the dice are revealed.
    fn outcome(&mut self, ui: &mut egui::Ui) {
        let Some(playing) = self.playing.as_mut() else {
            self.screen = Screen::Lobby;
            return;
        };

        ui.heading("Round over");
        if let Some(said) = playing.said() {
            ui.label(egui::RichText::new(said).size(17.0));
        }
        ui.separator();
        {
            let Some(view) = playing.view() else {
                return;
            };
            table(ui, &view, &playing.seating);
            ui.separator();
            history(ui, &view, &playing.seating);
        }
        ui.separator();

        // With nobody playing by hand, the local player does not have to act for the
        // match to continue.
        if !playing.anyone_human() {
            playing.table.resume();
            playing.outcome = None;
            self.screen = playing.step();
            ui.ctx().request_repaint();
            return;
        }

        if tap(ui, "Next round", true) {
            playing.table.resume();
            playing.outcome = None;
            self.screen = playing.step();
            return;
        }

        ui.separator();
        self.events(ui);
    }

    fn over(&mut self, ui: &mut egui::Ui) {
        let Some(playing) = self.playing.as_mut() else {
            self.screen = Screen::Lobby;
            return;
        };

        ui.heading("Match over");
        if let Some(said) = playing.said() {
            ui.label(egui::RichText::new(said).size(17.0));
        }
        ui.separator();
        {
            let Some(view) = playing.view() else {
                return;
            };
            table(ui, &view, &playing.seating);
            ui.separator();
            ranking(ui, &view, &playing.seating);
        }
        ui.separator();
        if tap(ui, "New match", true) {
            self.secret = self.secret.wrapping_add(1);
            self.playing = None;
            self.screen = Screen::Lobby;
            return;
        }

        ui.separator();
        self.events(ui);
    }

    /// What this seat has been sent, which is not what the relay holds.
    ///
    /// The relay's log carries a `Deal` for every seat; a client is only sent its own
    /// until the round is revealed. Listing the relay's log here would show the holder of
    /// the screen every other seat's dice — a worse leak than the table above it, and one
    /// no amount of care in the table would catch.
    fn events(&mut self, ui: &mut egui::Ui) {
        let Some(playing) = self.playing.as_ref() else {
            return;
        };
        let Some(player) = playing.table.players.get(playing.holding) else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("What this seat has seen").strong());
            ui.add(egui::Slider::new(&mut self.log_shown, 4..=60).text("lines"));
        });
        let stream = player.client.stream();
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

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    eframe::run_native(
        "Liars Dice hotseat",
        eframe::NativeOptions::default(),
        Box::new(|_| Ok(Box::<Hotseat>::default())),
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
                Box::new(|_| Ok(Box::<Hotseat>::default())),
            )
            .await
            .expect("the hotseat match started");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chairs(humans: &[bool]) -> Vec<Chair> {
        humans
            .iter()
            .enumerate()
            .map(|(seat, &human)| Chair {
                name: format!("Seat{seat}"),
                human,
            })
            .collect()
    }

    /// Drive a match exactly the way the screens do, and check the invariant the whole app
    /// rests on at every single state it passes through.
    ///
    /// Returns the finished match and every seat the screen was handed to, in order.
    fn play_it(lobby: &[Chair], secret: u8) -> (Match, Vec<usize>) {
        let mut playing = Match::launch(lobby, secret);
        let mut handovers = Vec::new();
        let mut screen = playing.step();
        let mut guard = 0;

        loop {
            guard += 1;
            assert!(guard < 10_000, "the match would not finish");

            if screen == Screen::Handover {
                handovers.push(playing.holding);
            }

            // Whatever is showing is drawn from the seat holding the screen and from no
            // other, and while a round is live that seat can reach nobody else's dice.
            if let Some(view) = playing.view() {
                assert_eq!(
                    view.seat(),
                    playing.holding,
                    "the screen was drawn from the wrong seat"
                );
                if matches!(screen, Screen::Playing | Screen::Handover) {
                    for seat in 0..lobby.len() {
                        if seat != playing.holding {
                            assert_eq!(
                                view.dice_of(seat),
                                None,
                                "seat {} could see seat {seat} mid-round",
                                playing.holding
                            );
                        }
                    }
                }
            }

            screen = match screen {
                Screen::Over => break,
                Screen::Lobby => panic!("a launched match went back to the lobby"),
                Screen::Handover => {
                    playing.take_the_screen();
                    Screen::Playing
                }
                Screen::Outcome => {
                    playing.table.resume();
                    playing.outcome = None;
                    playing.step()
                }
                Screen::Playing => {
                    let call = {
                        let view = playing.view().expect("a seat being played has a view");
                        match view.legal_raises().first() {
                            Some(&claim) => Call::Raise(claim),
                            None => Call::Liar,
                        }
                    };
                    playing.play(call)
                }
            };
        }

        (playing, handovers)
    }

    /// The case the specification exists for, and the one a single tester can play alone.
    #[test]
    fn one_person_against_virtual_players_reaches_a_winner() {
        let lobby = chairs(&[true, false, false]);
        let (playing, handovers) = play_it(&lobby, 11);
        assert!(playing.table.over(), "the match should have ended");
        assert!(
            handovers.is_empty(),
            "one person should never be handed the screen, but it went to {handovers:?}"
        );
    }

    /// `The screen is handed over before a human seat other than the one shown acts`, and
    /// never for any other reason: not to a virtual player, and not to whoever already has
    /// it.
    #[test]
    fn the_screen_is_handed_over_only_to_another_person() {
        let lobby = chairs(&[true, false, true]);
        let (playing, handovers) = play_it(&lobby, 23);
        assert!(playing.table.over(), "the match should have ended");
        assert!(
            !handovers.is_empty(),
            "two people should have passed the screen between them"
        );
        for &seat in &handovers {
            assert!(
                lobby[seat].human,
                "the screen was handed to seat {seat}, which no person is playing"
            );
        }
        for pair in handovers.windows(2) {
            assert_ne!(
                pair[0], pair[1],
                "the screen was handed to the person already holding it"
            );
        }
    }

    /// `When every seat is a virtual player the match plays without the local player
    /// acting`, and is watched from the first seat.
    #[test]
    fn a_table_of_virtual_players_needs_nobody() {
        let lobby = chairs(&[false, false, false]);
        let (playing, handovers) = play_it(&lobby, 7);
        assert!(playing.table.over(), "the match should have ended");
        assert!(!playing.anyone_human());
        assert!(
            handovers.is_empty(),
            "nobody was there to be handed the screen"
        );
        assert_eq!(playing.holding, 0, "watched from the first seat");
    }

    /// Five people, so the screen goes round and round. The same assertions, under the
    /// most handovers a hotseat match is likely to see.
    #[test]
    fn five_people_pass_one_screen_around() {
        let lobby = chairs(&[true, true, true, true, true]);
        let (playing, handovers) = play_it(&lobby, 5);
        assert!(playing.table.over(), "the match should have ended");
        assert!(
            handovers.len() > 20,
            "only {} handovers, too few to be evidence",
            handovers.len()
        );
    }

    /// Every seat is told who ended the round and how, so the sentence on the outcome
    /// screen is the client's own knowledge rather than something re-derived from a log.
    #[test]
    fn the_seat_holding_the_screen_can_say_how_the_round_ended() {
        let lobby = chairs(&[false, false, false]);
        let mut playing = Match::launch(&lobby, 3);
        let screen = playing.step();
        assert_eq!(screen, Screen::Outcome, "a table of ai ends a round first");
        let said = playing
            .said()
            .expect("the round ended, so it can be described");
        assert!(
            said.contains("called liar") || said.contains("called spot on"),
            "a round ends with a dispute, but the verdict read: {said}"
        );
        assert!(
            said.contains("There were"),
            "the verdict should say what was actually under the cups: {said}"
        );
    }
}
