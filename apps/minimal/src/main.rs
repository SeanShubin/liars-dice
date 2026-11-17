//! The minimal game. Mechanics only: two dimensional, one human against AI opponents.
//!
//! Composition root. No behavior: the rules come from `rules`, the probabilities from
//! `odds`, the opponents from `ai`, the palette from `render-minimal`. What lives here is
//! the window, the screens, and the one place non-determinism enters.
//!
//! Realizes /specification/game-minimal.md.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

use std::io::Write;

use ai::View;
use protocol::Event;
use render_minimal::palette::{PALETTE, assign, ink};
use rules::{Call, Claim, Entry, Match, Outcome, Rolls, Variants};

const DICE_EACH: u32 = 5;
const MAX_PLAYERS: usize = 10;
/// Every die is drawn in a box this size, shown or not, so the dice line up in a grid.
const DIE: f32 = 32.0;
/// Where the audit logs go.
const LOGS: &str = "logs";

/// The match's audit log: one file per match, one event per line.
///
/// Writing is an effect, so it lives in the composition root. What an event is and how it
/// reads belongs to `protocol`.
struct Audit {
    file: Option<std::fs::File>,
}

impl Audit {
    fn open(directory: &std::path::Path, stamp: u64) -> Self {
        let opened = std::fs::create_dir_all(directory).and_then(|()| {
            std::fs::File::create(directory.join(format!("match-{stamp:016x}.log")))
        });
        match opened {
            Ok(file) => Self { file: Some(file) },
            Err(reason) => {
                warn!("no audit log: {reason}");
                Self { file: None }
            }
        }
    }

    fn record(&mut self, event: Event) {
        if let Some(file) = self.file.as_mut() {
            if let Err(reason) = writeln!(file, "{event}") {
                warn!("audit log write failed: {reason}");
                self.file = None;
            }
        }
    }

    /// All players roll their dice, so all of them are recorded.
    fn record_roll(&mut self, game: &Match, players: usize) {
        for player in 0..players {
            let dice = game.dice_of(player).to_vec();
            if !dice.is_empty() {
                self.record(Event::Rolled { player, dice });
            }
        }
    }
}

/// The only non-determinism in the game, and it lives in the composition root.
///
/// Rejection sampling rather than a modulo, so every face is equally likely.
struct Dice {
    state: u64,
}

impl Dice {
    fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

impl Rolls for Dice {
    fn face(&mut self) -> u8 {
        let limit = u64::MAX - (u64::MAX % 6) - 1;
        loop {
            let draw = self.next();
            if draw <= limit {
                return (draw % 6) as u8 + 1;
            }
        }
    }
}

/// A match in progress, with everything the screens need.
struct Playing {
    game: Match,
    colours: Vec<usize>,
    dice: Dice,
    /// The claim the human is dialling in. Always legal.
    pending: Claim,
    /// Dice as they were when the round ended, for the reveal.
    revealed: Vec<Vec<u8>>,
    verdict: Option<String>,
    audit: Audit,
}

/// The human is always player zero. Turn order is a shuffled permutation, so where they
/// sit varies.
const HUMAN: usize = 0;

#[derive(Resource, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Setup { colour: usize, opponents: usize },
    Play,
    Reveal,
    Over,
}

/// The match in progress. Separate from [`Screen`] so changing screens never moves it.
#[derive(Resource, Default)]
struct Table(Option<Playing>);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Liars Dice".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(Screen::Setup {
            colour: 1,
            opponents: 3,
        })
        .insert_resource(Table::default())
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Camera2d);
        })
        .add_systems(EguiPrimaryContextPass, screen)
        .run();
}

fn colour_of(index: usize) -> egui::Color32 {
    let [r, g, b] = PALETTE[index].rgb;
    egui::Color32::from_rgb(r, g, b)
}

/// A tap target big enough for a finger.
fn tap(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(label).size(20.0)).min_size([64.0, 44.0].into()),
    )
    .clicked()
}

/// Reading the clock is the one piece of non-determinism outside the dice, and it happens
/// here so that each match differs from the last and names its own log.
fn stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0x2545_F491_4F6C_DD1D)
}

fn start(colour: usize, opponents: usize) -> Playing {
    let players = opponents + 1;
    let colours = assign(colour, players);

    let seed = stamp();
    let mut dice = Dice::new(seed);

    // Turn order is chosen at random when the match starts.
    let mut order: Vec<usize> = (0..players).collect();
    for position in (1..players).rev() {
        let swap = (dice.next() % (position as u64 + 1)) as usize;
        order.swap(position, swap);
    }

    let variants = Variants::default();
    let game = Match::start(order.clone(), DICE_EACH, variants, &mut dice);
    let pending = first_legal(&game);

    let mut audit = Audit::open(std::path::Path::new(LOGS), seed);
    audit.record(Event::MatchStarted {
        players,
        order,
        colours: colours
            .iter()
            .map(|&index| PALETTE[index].name.to_string())
            .collect(),
        dice_each: DICE_EACH,
        ones_wild: variants.ones_wild,
    });
    audit.record_roll(&game, players);

    Playing {
        game,
        colours,
        dice,
        pending,
        revealed: Vec::new(),
        verdict: None,
        audit,
    }
}

fn first_legal(game: &Match) -> Claim {
    game.legal_raises().first().copied().unwrap_or(Claim {
        quantity: 1,
        face: 1,
    })
}

fn name_of(player: usize) -> String {
    if player == HUMAN {
        "you".to_string()
    } else {
        format!("opponent {player}")
    }
}

fn screen(
    mut contexts: EguiContexts,
    mut screen: ResMut<Screen>,
    mut table: ResMut<Table>,
) -> Result {
    let context = contexts.ctx_mut()?.clone();
    let mut next: Option<Screen> = None;

    egui::Window::new("Liars Dice")
        .default_width(560.0)
        .default_pos([20.0, 20.0])
        .show(&context, |ui| {
            let mut current = *screen;
            match &mut current {
                Screen::Setup { colour, opponents } => {
                    if let Some(started) = setup(ui, colour, opponents) {
                        table.0 = Some(started);
                        next = Some(Screen::Play);
                    } else {
                        next = Some(Screen::Setup {
                            colour: *colour,
                            opponents: *opponents,
                        });
                    }
                }
                Screen::Play => {
                    if let Some(playing) = table.0.as_mut() {
                        next = play(ui, playing);
                    }
                }
                Screen::Reveal => {
                    if let Some(playing) = table.0.as_mut() {
                        next = reveal(ui, playing);
                    }
                }
                Screen::Over => {
                    if let Some(playing) = table.0.as_mut() {
                        next = over(ui, playing);
                    }
                }
            }
        });

    if let Some(screen_next) = next {
        *screen = screen_next;
    }
    Ok(())
}

fn setup(ui: &mut egui::Ui, colour: &mut usize, opponents: &mut usize) -> Option<Playing> {
    ui.heading("Your dice");
    ui.label(PALETTE[*colour].name);

    egui::Grid::new("palette")
        .spacing([6.0, 6.0])
        .show(ui, |ui| {
            for (index, swatch) in PALETTE.iter().enumerate() {
                let selected = index == *colour;
                let size = if selected { 52.0 } else { 44.0 };
                if ui
                    .add(
                        egui::Button::new("")
                            .fill(colour_of(index))
                            .min_size([size, size].into()),
                    )
                    .on_hover_text(swatch.name)
                    .clicked()
                {
                    *colour = index;
                }
                if index % 5 == 4 {
                    ui.end_row();
                }
            }
        });

    ui.separator();
    ui.heading("Opponents");
    ui.horizontal(|ui| {
        if tap(ui, "-", *opponents > 1) {
            *opponents -= 1;
        }
        ui.label(egui::RichText::new(format!("{opponents}")).size(28.0));
        if tap(ui, "+", *opponents + 1 < MAX_PLAYERS) {
            *opponents += 1;
        }
    });
    ui.label(format!("{} players, {MAX_PLAYERS} at most", *opponents + 1));

    ui.separator();
    if tap(ui, "Start", true) {
        return Some(start(*colour, *opponents));
    }
    None
}

fn ink_of(index: usize) -> egui::Color32 {
    let [r, g, b] = ink(PALETTE[index]);
    egui::Color32::from_rgb(r, g, b)
}

/// One die: a fixed size box in its owner's colour, with a border and a value in whichever
/// ink contrasts with that colour.
///
/// The border is what makes a black die visible against a dark background and a white one
/// visible against a light one, whatever the theme does. `face` is `None` for a die whose
/// value is not revealed; the box is the same size either way, so the dice line up in a
/// grid whether they are shown or not.
fn die(ui: &mut egui::Ui, swatch: usize, face: Option<u8>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(DIE, DIE), egui::Sense::hover());
    let ink = ink_of(swatch);
    let painter = ui.painter();
    painter.rect_filled(rect, 5.0, colour_of(swatch));
    painter.rect_stroke(
        rect,
        5.0,
        egui::Stroke::new(1.5, ink),
        egui::StrokeKind::Inside,
    );
    match face {
        Some(value) => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                value,
                egui::FontId::proportional(17.0),
                ink,
            );
        }
        // An unrevealed die still shows whose it is and that it is there.
        None => {
            painter.circle_filled(rect.center(), 3.5, ink);
        }
    }
}

/// Every player and their dice, in a grid so the columns line up across players.
///
/// Only the human's values are shown while a round is live; everyone else's dice occupy
/// their slots unrevealed.
fn table(ui: &mut egui::Ui, playing: &Playing, revealed: bool) {
    let game = &playing.game;
    egui::Grid::new("table").spacing([5.0, 5.0]).show(ui, |ui| {
        for player in 0..playing.colours.len() {
            let out = game.eliminated(player);
            let mut name = egui::RichText::new(name_of(player)).size(15.0);
            if out {
                name = name.strikethrough();
            }
            ui.label(name);

            let dice: Vec<u8> = if revealed {
                playing.revealed.get(player).cloned().unwrap_or_default()
            } else {
                game.dice_of(player).to_vec()
            };
            let show = revealed || player == HUMAN;
            if dice.is_empty() && out {
                ui.label(egui::RichText::new("out").size(14.0).weak());
            }
            for value in dice {
                die(ui, playing.colours[player], show.then_some(value));
            }
            ui.end_row();
        }
    });
}

/// This round's claims: the claim, then the claimer's dice.
///
/// The dice carry both who claimed and how many they held, so neither has to be written
/// out.
fn history(ui: &mut egui::Ui, playing: &Playing) {
    ui.label(egui::RichText::new("This round").strong());
    if playing.game.history().is_empty() {
        ui.label("no claims yet");
        return;
    }
    egui::Grid::new("history")
        .spacing([5.0, 5.0])
        .show(ui, |ui| {
            for Entry {
                player,
                dice_held,
                claim,
            } in playing.game.history()
            {
                ui.label(
                    egui::RichText::new(format!("at least {} × {}", claim.quantity, claim.face))
                        .size(15.0),
                );
                for _ in 0..*dice_held {
                    die(ui, playing.colours[*player], None);
                }
                ui.end_row();
            }
        });
}

/// Advance every opponent's turn until it is the human's, or the round ends.
fn run_opponents(playing: &mut Playing) -> Option<Screen> {
    while !playing.game.over() && playing.game.current() != HUMAN {
        let player = playing.game.current();
        let legal = playing.game.legal_raises();
        let own = playing.game.dice_of(player).to_vec();
        let unknown = playing.game.dice_in_play() - playing.game.count(player);
        let call = ai::choose(&View {
            own_dice: &own,
            unknown,
            legal_raises: &legal,
            previous: playing.game.previous_claim(),
            variants: playing.game.variants(),
        });
        if let Some(screen) = apply(playing, call) {
            return Some(screen);
        }
    }
    playing.pending = first_legal(&playing.game);
    None
}

/// Apply a call and move to the reveal if the round ended.
fn apply(playing: &mut Playing, call: Call) -> Option<Screen> {
    let disputed = playing.game.previous_claim();
    let actor = playing.game.current();
    let snapshot: Vec<Vec<u8>> = (0..playing.colours.len())
        .map(|player| playing.game.dice_of(player).to_vec())
        .collect();

    let held = playing.game.count(actor);
    let outcome = playing
        .game
        .apply(call, &mut playing.dice)
        .expect("only legal calls are offered");

    match outcome {
        Outcome::Continues => {
            if let Call::Raise(claim) = call {
                playing.audit.record(Event::Claimed {
                    player: actor,
                    dice_held: held,
                    quantity: claim.quantity,
                    face: claim.face,
                });
            }
            None
        }
        Outcome::Resolved {
            lost,
            gained,
            eliminated,
            truth,
            over,
            ..
        } => {
            let claim = disputed.expect("a resolution always disputes a claim");
            let mut said = match call {
                Call::Liar => format!("{} called liar", name_of(actor)),
                Call::SpotOn => format!("{} called spot on", name_of(actor)),
                Call::Raise(_) => unreachable!("a raise does not resolve a round"),
            };
            said.push_str(&format!(
                " on at least {} × {}. There were {truth}.",
                claim.quantity, claim.face
            ));
            if let Some(player) = gained {
                said.push_str(&format!(" {} gained a die.", name_of(player)));
            }
            if let Some(player) = lost {
                said.push_str(&format!(" {} lost a die.", name_of(player)));
            }
            if let Some(player) = eliminated {
                said.push_str(&format!(" {} is out.", name_of(player)));
            }

            playing.audit.record(Event::Called {
                player: actor,
                call: match call {
                    Call::Liar => "liar",
                    Call::SpotOn => "spot on",
                    Call::Raise(_) => unreachable!("a raise does not resolve a round"),
                },
                quantity: claim.quantity,
                face: claim.face,
                truth,
            });
            if let Some(player) = gained {
                playing.audit.record(Event::GainedDie {
                    player,
                    remaining: playing.game.count(player),
                });
            }
            if let Some(player) = lost {
                playing.audit.record(Event::LostDie {
                    player,
                    remaining: playing.game.count(player),
                });
            }
            if let Some(player) = eliminated {
                playing.audit.record(Event::Eliminated { player });
            }

            playing.revealed = snapshot;
            playing.verdict = Some(said);
            if over {
                playing.audit.record(Event::MatchEnded {
                    ranking: playing.game.ranking(),
                });
                Some(Screen::Over)
            } else {
                // A resolution rolls again, so the new round's dice are recorded here.
                let players = playing.colours.len();
                playing.audit.record_roll(&playing.game, players);
                Some(Screen::Reveal)
            }
        }
    }
}

fn play(ui: &mut egui::Ui, playing: &mut Playing) -> Option<Screen> {
    // Opponents act before the human on the first visit to this screen.
    if playing.game.current() != HUMAN {
        if let Some(screen) = run_opponents(playing) {
            return Some(screen);
        }
    }

    table(ui, playing, false);
    ui.separator();
    history(ui, playing);
    ui.separator();

    let game = &playing.game;
    let unknown = game.dice_in_play() - game.count(HUMAN);
    let own = game.dice_of(HUMAN).to_vec();
    let legal = game.legal_raises();
    let view = View {
        own_dice: &own,
        unknown,
        legal_raises: &legal,
        previous: game.previous_claim(),
        variants: game.variants(),
    };

    ui.label(egui::RichText::new("Your claim").strong());
    let pending = playing.pending;
    let step = |claim: Claim| game.check_raise(claim).is_ok();

    let mut dialled = pending;
    ui.horizontal(|ui| {
        ui.label("quantity");
        if tap(
            ui,
            "-",
            pending.quantity > 0
                && step(Claim {
                    quantity: pending.quantity - 1,
                    ..pending
                }),
        ) {
            dialled.quantity -= 1;
        }
        ui.label(egui::RichText::new(format!("{}", pending.quantity)).size(26.0));
        if tap(
            ui,
            "+",
            step(Claim {
                quantity: pending.quantity + 1,
                ..pending
            }),
        ) {
            dialled.quantity += 1;
        }
    });
    ui.horizontal(|ui| {
        ui.label("face    ");
        if tap(
            ui,
            "-",
            pending.face > 1
                && step(Claim {
                    face: pending.face - 1,
                    ..pending
                }),
        ) {
            dialled.face -= 1;
        }
        ui.label(egui::RichText::new(format!("{}", pending.face)).size(26.0));
        if tap(
            ui,
            "+",
            pending.face < 6
                && step(Claim {
                    face: pending.face + 1,
                    ..pending
                }),
        ) {
            dialled.face += 1;
        }
    });
    playing.pending = dialled;

    let chance = readout::percent(&view.chance_of(playing.pending));
    ui.label(
        egui::RichText::new(format!(
            "at least {} × {} is {:.1}% likely",
            playing.pending.quantity, playing.pending.face, chance
        ))
        .size(17.0),
    );

    ui.separator();
    let has_previous = game.previous_claim().is_some();
    let mut call = None;
    ui.horizontal(|ui| {
        if tap(ui, "Raise", step(playing.pending)) {
            call = Some(Call::Raise(playing.pending));
        }
        if tap(ui, "Liar", has_previous) {
            call = Some(Call::Liar);
        }
        if tap(ui, "Spot On", has_previous) {
            call = Some(Call::SpotOn);
        }
    });

    if let Some(call) = call {
        if let Some(screen) = apply(playing, call) {
            return Some(screen);
        }
        return run_opponents(playing);
    }
    None
}

fn reveal(ui: &mut egui::Ui, playing: &mut Playing) -> Option<Screen> {
    ui.heading("Round over");
    if let Some(verdict) = &playing.verdict {
        ui.label(egui::RichText::new(verdict).size(17.0));
    }
    ui.separator();
    table(ui, playing, true);
    ui.separator();
    if tap(ui, "Next round", true) {
        playing.revealed.clear();
        playing.verdict = None;
        playing.pending = first_legal(&playing.game);
        return Some(Screen::Play);
    }
    None
}

fn over(ui: &mut egui::Ui, playing: &mut Playing) -> Option<Screen> {
    ui.heading("Match over");
    if let Some(verdict) = &playing.verdict {
        ui.label(egui::RichText::new(verdict).size(17.0));
    }
    ui.separator();
    table(ui, playing, true);
    ui.separator();
    ui.label(egui::RichText::new("Finishing order").strong());
    egui::Grid::new("ranking")
        .spacing([5.0, 5.0])
        .show(ui, |ui| {
            for (place, player) in playing.game.ranking().iter().enumerate() {
                ui.label(
                    egui::RichText::new(format!("{}. {}", place + 1, name_of(*player))).size(16.0),
                );
                die(ui, playing.colours[*player], None);
                ui.end_row();
            }
        });
    ui.separator();
    if tap(ui, "New match", true) {
        return Some(Screen::Setup {
            colour: playing.colours[HUMAN],
            opponents: playing.colours.len() - 1,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every face is equally likely, so no face is favoured by the rejection sampling.
    #[test]
    fn the_dice_are_fair() {
        let mut dice = Dice::new(0x1234_5678_9abc_def0);
        let mut counts = [0u32; 7];
        for _ in 0..600_000 {
            counts[dice.face() as usize] += 1;
        }
        assert_eq!(counts[0], 0, "a face below one was rolled");
        for face in 1..=6 {
            let share = counts[face] as f64 / 600_000.0;
            assert!(
                (share - 1.0 / 6.0).abs() < 0.005,
                "face {face} came up {share} of the time"
            );
        }
    }

    #[test]
    fn a_seed_always_rolls_the_same_dice() {
        let mut first = Dice::new(99);
        let mut second = Dice::new(99);
        let rolled: Vec<u8> = (0..200).map(|_| first.face()).collect();
        let again: Vec<u8> = (0..200).map(|_| second.face()).collect();
        assert_eq!(rolled, again);
    }

    #[test]
    fn an_audit_log_is_one_file_per_match_with_one_event_per_line() {
        let directory = std::env::temp_dir().join("liars-dice-audit-test");
        let _ = std::fs::remove_dir_all(&directory);

        for stamp in [1u64, 2] {
            let mut audit = Audit::open(&directory, stamp);
            audit.record(Event::MatchStarted {
                players: 2,
                order: vec![0, 1],
                colours: vec!["red".into(), "navy".into()],
                dice_each: 5,
                ones_wild: false,
            });
            audit.record(Event::Rolled {
                player: 0,
                dice: vec![1, 2, 3, 4, 5],
            });
            audit.record(Event::MatchEnded {
                ranking: vec![0, 1],
            });
        }

        let mut files: Vec<_> = std::fs::read_dir(&directory)
            .expect("the log directory was created")
            .map(|entry| entry.unwrap().path())
            .collect();
        files.sort();
        assert_eq!(files.len(), 2, "each match should have its own file");

        for file in &files {
            let text = std::fs::read_to_string(file).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines.len(), 3, "one line per event");
            assert!(lines[0].starts_with("match-started "));
            assert!(lines[1].starts_with("rolled "));
            assert!(lines[2].starts_with("match-ended "));
        }

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A directory that cannot be opened must not take the game down with it.
    #[test]
    fn a_log_that_cannot_be_written_is_survivable() {
        let impossible = std::path::Path::new("logs").join(" ");
        let mut audit = Audit::open(&impossible, 7);
        audit.record(Event::Eliminated { player: 1 });
    }
}
