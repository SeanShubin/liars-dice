//! The minimal game. Mechanics only: two dimensional, one human against AI opponents.
//!
//! Composition root. No behavior: the rules come from `rules`, the probabilities from
//! `odds`, the opponents from `ai`, the palette from `render-minimal`. What lives here is
//! the window, the screens, and the one place non-determinism enters.
//!
//! Realizes /specification/game-minimal.md.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

use ai::View;
use render_minimal::palette::{PALETTE, assign};
use rules::{Call, Claim, Entry, Match, Outcome, Rolls, Variants};

const DICE_EACH: u32 = 5;
const MAX_PLAYERS: usize = 10;

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

#[derive(Resource)]
struct Seed(u64);

fn main() {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0x2545_F491_4F6C_DD1D);

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Liars Dice".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(Seed(seed))
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

fn start(colour: usize, opponents: usize, seed: u64) -> Playing {
    let players = opponents + 1;
    let colours = assign(colour, players);

    let mut dice = Dice::new(seed);

    // Turn order is chosen at random when the match starts.
    let mut order: Vec<usize> = (0..players).collect();
    for position in (1..players).rev() {
        let swap = (dice.next() % (position as u64 + 1)) as usize;
        order.swap(position, swap);
    }

    let game = Match::start(order, DICE_EACH, Variants::default(), &mut dice);
    let pending = first_legal(&game);
    Playing {
        game,
        colours,
        dice,
        pending,
        revealed: Vec::new(),
        verdict: None,
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
    seed: Res<Seed>,
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
                    if let Some(started) = setup(ui, colour, opponents, seed.0) {
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

fn setup(
    ui: &mut egui::Ui,
    colour: &mut usize,
    opponents: &mut usize,
    seed: u64,
) -> Option<Playing> {
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
        return Some(start(*colour, *opponents, seed));
    }
    None
}

fn table(ui: &mut egui::Ui, playing: &Playing) {
    let game = &playing.game;
    ui.horizontal_wrapped(|ui| {
        for player in 0..playing.colours.len() {
            let out = game.eliminated(player);
            let mut text = egui::RichText::new(format!(
                "  {}  {}  ",
                name_of(player),
                if out {
                    "out".to_string()
                } else {
                    format!("{} dice", game.count(player))
                }
            ))
            .background_color(colour_of(playing.colours[player]))
            .color(egui::Color32::BLACK)
            .size(15.0);
            if out {
                text = text.strikethrough();
            }
            ui.label(text);
        }
    });
}

fn history(ui: &mut egui::Ui, playing: &Playing) {
    ui.label(egui::RichText::new("This round").strong());
    if playing.game.history().is_empty() {
        ui.label("no claims yet");
        return;
    }
    for Entry {
        player,
        dice_held,
        claim,
    } in playing.game.history()
    {
        ui.label(
            egui::RichText::new(format!(
                "  {} dice   at least {} × {}  ",
                dice_held, claim.quantity, claim.face
            ))
            .background_color(colour_of(playing.colours[*player]))
            .color(egui::Color32::BLACK)
            .size(15.0),
        );
    }
}

fn my_dice(ui: &mut egui::Ui, playing: &Playing) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Your dice").strong());
        for die in playing.game.dice_of(HUMAN) {
            ui.label(
                egui::RichText::new(format!("  {die}  "))
                    .background_color(colour_of(playing.colours[HUMAN]))
                    .color(egui::Color32::BLACK)
                    .size(22.0),
            );
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

    let outcome = playing
        .game
        .apply(call, &mut playing.dice)
        .expect("only legal calls are offered");

    match outcome {
        Outcome::Continues => None,
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

            playing.revealed = snapshot;
            playing.verdict = Some(said);
            if over {
                Some(Screen::Over)
            } else {
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

    table(ui, playing);
    ui.separator();
    my_dice(ui, playing);
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

    let chance = view.chance_of(playing.pending);
    ui.label(
        egui::RichText::new(format!(
            "at least {} × {} is {:.1}% likely",
            playing.pending.quantity,
            playing.pending.face,
            chance * 100.0
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
    for player in 0..playing.colours.len() {
        let dice = playing.revealed.get(player).cloned().unwrap_or_default();
        if dice.is_empty() {
            continue;
        }
        ui.horizontal(|ui| {
            ui.label(format!("{:<12}", name_of(player)));
            for die in dice {
                ui.label(
                    egui::RichText::new(format!("  {die}  "))
                        .background_color(colour_of(playing.colours[player]))
                        .color(egui::Color32::BLACK)
                        .size(20.0),
                );
            }
        });
    }
    ui.separator();
    table(ui, playing);
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
    ui.label(egui::RichText::new("Finishing order").strong());
    for (place, player) in playing.game.ranking().iter().enumerate() {
        ui.label(
            egui::RichText::new(format!("  {}. {}  ", place + 1, name_of(*player)))
                .background_color(colour_of(playing.colours[*player]))
                .color(egui::Color32::BLACK)
                .size(17.0),
        );
    }
    ui.separator();
    if tap(ui, "New match", true) {
        return Some(Screen::Setup {
            colour: playing.colours[HUMAN],
            opponents: playing.colours.len() - 1,
        });
    }
    None
}
