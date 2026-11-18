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

use ai::Seen;
use integrity::{Secret, Seed};
use protocol::{Account, Event, Moment, PROTOCOL_VERSION, Seat};
use render_minimal::palette::{PALETTE, assign, ink};
use rules::{Call, Claim, Claimed, Entry, Game, Resolution, SEAT, Variants};

const MAX_PLAYERS: usize = 10;
/// Every die is drawn in a box this size, shown or not, so the dice line up in a grid.
const DIE: f32 = 32.0;
/// Every claim is drawn in a box this size, filled or not, so the claims line up too.
const CLAIM: [f32; 2] = [58.0, 30.0];
/// Where the audit logs go.
const LOGS: &str = "logs";
/// An outcome this unlikely or worse is called out in the log as luck.
///
/// Five per cent is a one in twenty outcome. How often that actually fires depends on how
/// often a confident claim gets challenged at all, which is a question for a few real logs
/// rather than for reasoning, so this is a starting point rather than a settled number.
const LUCK: f64 = 5.0;
/// Opponents get names rather than numbers, so a log line reads as a sentence and so the
/// screen does too. One token each, because the log is space separated.
const OPPONENTS: [&str; 9] = [
    "Alice", "Bob", "Carol", "Dave", "Erin", "Frank", "Grace", "Heidi", "Ivan",
];

/// The match's audit log: one file per match, one event per line.
///
/// Writing is an effect, so it lives in the composition root. What an event is and how it
/// reads belongs to `protocol`.
struct Audit {
    file: Option<std::fs::File>,
}

impl Audit {
    /// Named for when the match started, so sorting the directory as text sorts it by
    /// time.
    ///
    /// Created rather than truncated: two matches starting in the same millisecond would
    /// otherwise share a name and the second would erase the first.
    fn open(directory: &std::path::Path, at: Moment) -> Self {
        let opened = std::fs::create_dir_all(directory).and_then(|()| {
            let stem = at.file_stem();
            let mut attempt = 1;
            loop {
                let name = if attempt == 1 {
                    format!("match-{stem}.log")
                } else {
                    format!("match-{stem}-{attempt}.log")
                };
                match std::fs::File::create_new(directory.join(name)) {
                    Err(clash) if clash.kind() == std::io::ErrorKind::AlreadyExists => {
                        attempt += 1;
                    }
                    other => return other,
                }
            }
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
        if let Some(file) = self.file.as_mut()
            && let Err(reason) = writeln!(file, "{event}")
        {
            warn!("audit log write failed: {reason}");
            self.file = None;
        }
    }

    /// The seed of the round that just ended, so a reader can recompute every die of it
    /// and check them against the deal lines above.
    fn record_reveal(&mut self, dealer: &Dealer, round: u32) {
        self.record(Event::Reveal {
            seed: dealer.seed(round).to_hex(),
        });
    }

    /// The commitment to the round, then the round. The commitment comes first because
    /// every die in a round is fixed before any claim in that round, and its position in
    /// the log is what says so.
    fn record_deal(&mut self, game: &Game, dealer: &Dealer, players: usize) {
        self.record(Event::Commit {
            digest: integrity::commit(&dealer.seed(game.public().round())).to_hex(),
        });
        self.record_deal_lines(game, players);
    }

    fn record_deal_lines(&mut self, game: &Game, players: usize) {
        for player in 0..players {
            let dice = game
                .dealt(player)
                .map(|seat| seat.to_vec())
                .unwrap_or_default();
            if !dice.is_empty() {
                self.record(Event::Deal {
                    seat: seat_of(player),
                    dice,
                });
            }
        }
    }
}

/// The only non-determinism in the game, and it lives in the composition root.
///
/// It no longer deals: the dice come from the integrity scheme, which commits to them
/// before any claim and reveals them when the round resolves. What is left is the two
/// things a match needs decided once — the turn order and the match secret.
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

impl Dice {
    /// A match secret, which every round's seed derives from.
    fn secret(&mut self) -> Secret {
        let mut bytes = [0u8; 32];
        for chunk in bytes.chunks_mut(8) {
            chunk.copy_from_slice(&self.next().to_be_bytes());
        }
        Secret::from_bytes(bytes)
    }
}

/// Deals a seat at a time from the match secret, as the relay will.
///
/// A desktop game has no relay, so it keeps the secret itself. The point of running the
/// same path here is that the log of a desktop match verifies by the same procedure as a
/// networked one, and the scheme is exercised constantly rather than only in production.
struct Dealer {
    secret: Secret,
    match_id: String,
}

impl Dealer {
    fn seed(&self, round: u32) -> Seed {
        integrity::seed_for(&self.secret, &self.match_id, round)
    }
}

impl Dealer {
    /// Every face for one seat of one round. The rules take no dealer now: a composition
    /// root deals and installs, which is what keeps `rules` free of non-determinism.
    fn seat(&self, round: u32, seat: usize) -> [u8; SEAT] {
        let dealt = integrity::deal(&self.seed(round), seat as u8, SEAT);
        dealt.try_into().expect("the dice limit of faces")
    }

    /// Deal every seat of the round the game is in.
    fn deal(&self, game: &mut Game) {
        let round = game.public().round();
        for seat in 0..game.public().players() {
            let _ = game.install(seat, self.seat(round, seat));
        }
    }
}

/// A match in progress, with everything the screens need.
struct Playing {
    game: Game,
    colours: Vec<usize>,
    dealer: Dealer,
    /// The claim the human is dialling in. Always legal.
    pending: Claim,
    /// Dice as they were when the round ended, for the reveal.
    revealed: Vec<Vec<u8>>,
    /// The round's claims, kept because resolving a round clears them from the match.
    round: Vec<Entry>,
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
    let at = Moment::from_unix_nanos(seed);
    let mut dice = Dice::new(seed);
    let dealer = Dealer {
        secret: dice.secret(),
        match_id: at.file_stem(),
    };

    // Turn order is chosen at random when the match starts.
    let mut order: Vec<usize> = (0..players).collect();
    for position in (1..players).rev() {
        let swap = (dice.next() % (position as u64 + 1)) as usize;
        order.swap(position, swap);
    }

    let variants = Variants::default();
    let mut game = Game::start(order.clone(), variants);
    dealer.deal(&mut game);
    let pending = first_legal(&game);

    let mut audit = Audit::open(std::path::Path::new(LOGS), at);
    audit.record(Event::Version {
        protocol: PROTOCOL_VERSION,
    });
    audit.record(Event::Started { at });
    for player in 0..players {
        audit.record(Event::Join {
            seat: seat_of(player),
            name: name_of(player),
            colour: PALETTE[colours[player]].name.to_string(),
            account: account_of(player),
        });
    }
    audit.record(Event::PlayerOrder(
        order.iter().map(|&player| seat_of(player)).collect(),
    ));
    audit.record(Event::Variant {
        name: "ones-wild",
        on: variants.ones_wild,
    });
    audit.record_deal(&game, &dealer, players);

    Playing {
        game,
        colours,
        dealer,
        pending,
        revealed: Vec::new(),
        round: Vec::new(),
        verdict: None,
        audit,
    }
}

fn first_legal(game: &Game) -> Claim {
    game.public()
        .legal_raises()
        .first()
        .copied()
        .unwrap_or(Claim {
            quantity: 1,
            face: 1,
        })
}

/// A player's seat. The minimal desktop game seats players in index order, so the two
/// are the same number; the conversion lives here so no other place assumes it.
fn seat_of(player: usize) -> Seat {
    Seat(player as u8)
}

/// Who is behind a seat. A desktop game has no authentication, so the human has no
/// account id and every opponent is an AI player.
fn account_of(player: usize) -> Account {
    if player == HUMAN {
        Account::Local
    } else {
        Account::Ai
    }
}

fn name_of(player: usize) -> String {
    if player == HUMAN {
        "You".to_string()
    } else {
        OPPONENTS
            .get(player - 1)
            .map(|name| name.to_string())
            .unwrap_or_else(|| format!("Player{player}"))
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
        // A question mark rather than a dot: a dot reads as a die showing one.
        None => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(17.0),
                ink,
            );
        }
    }
}

/// Every player and their dice, in a grid so the columns line up across players.
///
/// Only the human's values are shown while a round is live; everyone else's dice occupy
/// their slots unrevealed.
fn table(ui: &mut egui::Ui, playing: &Playing, revealed: bool) {
    let game = &playing.game;
    let turn = (!game.public().over()).then(|| game.public().current());
    egui::Grid::new("table").spacing([5.0, 5.0]).show(ui, |ui| {
        // Turn order rather than player number, so who attacks whom is visible.
        for &player in game.public().order() {
            let out = game.public().eliminated(player);
            let mut name = egui::RichText::new(name_of(player)).size(15.0);
            if out {
                name = name.strikethrough();
            }
            if turn == Some(player) {
                name = name.strong();
            }
            ui.label(name);
            ui.label(if turn == Some(player) {
                egui::RichText::new("turn").size(13.0).strong()
            } else {
                egui::RichText::new("")
            });

            let dice: Vec<u8> = if revealed {
                playing.revealed.get(player).cloned().unwrap_or_default()
            } else {
                game.view(HUMAN)
                    .dice_of(player)
                    .map(|dice| dice.to_vec())
                    .unwrap_or_default()
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

/// Whose turn it is, what there is to beat, and how many dice are in play.
fn standing(ui: &mut egui::Ui, playing: &Playing) {
    let game = &playing.game;
    let whose = if game.public().over() {
        "match over".to_string()
    } else if game.public().current() == HUMAN {
        "Your turn".to_string()
    } else {
        format!("{}'s turn", name_of(game.public().current()))
    };
    let beat = match game.public().previous_claim() {
        Some(claim) => format!("to beat: at least {} x {}", claim.quantity, claim.face),
        None => "nothing to beat yet".to_string(),
    };
    ui.label(
        egui::RichText::new(format!(
            "{whose}   -   {beat}   -   {} dice in play",
            game.public().dice_in_play()
        ))
        .size(16.0),
    );
}

/// The chance that each claim the human could make is true.
///
/// Three rows, one per button. The standing claim's own chance is shown rather than the
/// chance that calling liar is right, because those are complements and the claim is the
/// thing on the table — but it is the number the liar decision turns on.
fn odds(ui: &mut egui::Ui, view: &impl Seen, dialled: Claim, previous: Option<Claim>) {
    ui.label(egui::RichText::new("Odds").strong());
    egui::Grid::new("odds").spacing([10.0, 4.0]).show(ui, |ui| {
        let row = |ui: &mut egui::Ui, what: &str, claim: String, chance: Option<f64>| {
            ui.label(egui::RichText::new(what).size(15.0));
            ui.label(egui::RichText::new(claim).size(15.0));
            ui.label(
                egui::RichText::new(match chance {
                    Some(percent) => format!("{percent:.1}%"),
                    None => "-".to_string(),
                })
                .size(16.0)
                .strong(),
            );
            ui.end_row();
        };

        match previous {
            Some(claim) => row(
                ui,
                "their claim",
                format!("at least {} x {}", claim.quantity, claim.face),
                Some(readout::percent(&view.chance_of(claim))),
            ),
            None => row(ui, "their claim", "none yet".to_string(), None),
        }

        row(
            ui,
            "your raise",
            format!("at least {} x {}", dialled.quantity, dialled.face),
            Some(readout::percent(&view.chance_of(dialled))),
        );

        match previous {
            Some(claim) => row(
                ui,
                "spot on",
                format!("exactly {} x {}", claim.quantity, claim.face),
                Some(readout::percent(&view.chance_exactly(claim))),
            ),
            None => row(ui, "spot on", "not available".to_string(), None),
        }
    });
}

/// A dialled number, centred in a box the height of a tap target.
///
/// Fixed width, so a two digit quantity does not shift the button beside it, and the same
/// height as the buttons, so the rows line up vertically as well as across.
fn dialled_value(ui: &mut egui::Ui, value: u32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 44.0), egui::Sense::hover());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        value,
        egui::FontId::proportional(24.0),
        ui.visuals().text_color(),
    );
}

/// A claim in its claimer's colour.
///
/// The grid position already says who claimed, so the claimer's dice do not have to be
/// drawn beside it. An empty cell still takes its space, which is what keeps the columns
/// lined up.
fn claim_box(ui: &mut egui::Ui, swatch: usize, claim: Option<Claim>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(CLAIM[0], CLAIM[1]), egui::Sense::hover());
    let Some(claim) = claim else {
        return;
    };
    let ink = ink_of(swatch);
    let painter = ui.painter();
    painter.rect_filled(rect, 5.0, colour_of(swatch));
    painter.rect_stroke(
        rect,
        5.0,
        egui::Stroke::new(1.5, ink),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{} x {}", claim.quantity, claim.face),
        egui::FontId::proportional(15.0),
        ink,
    );
}

/// This round's claims: players down the side, trips around the table across the top.
///
/// A player's first claim sits in the first column, their second in the second, and so on,
/// so a column is one time around the table and a row is one player's escalation. Reading
/// along a row shows how far one player pushed.
///
/// Rows start with whoever opened the round rather than with the match's turn order, so the
/// first column reads straight down in the order the claims were actually made.
fn history(ui: &mut egui::Ui, playing: &Playing, entries: &[Entry]) {
    ui.label(egui::RichText::new("This round").strong());
    if entries.is_empty() {
        ui.label("no claims yet");
        return;
    }

    let mut claims: Vec<Vec<Claim>> = vec![Vec::new(); playing.colours.len()];
    for entry in entries {
        claims[entry.player].push(entry.claim);
    }
    let trips = claims.iter().map(Vec::len).max().unwrap_or(0);

    let order = playing.game.public().order();
    let opener = entries[0].player;
    let from = order
        .iter()
        .position(|&player| player == opener)
        .unwrap_or(0);
    let rotated: Vec<usize> = order[from..]
        .iter()
        .chain(order[..from].iter())
        .copied()
        .collect();

    egui::Grid::new("history")
        .spacing([4.0, 4.0])
        .show(ui, |ui| {
            ui.label("");
            for trip in 1..=trips {
                ui.label(egui::RichText::new(format!("{trip}")).size(13.0).weak());
            }
            ui.end_row();

            for &player in &rotated {
                // A player who never claimed and is out of the match has nothing to show.
                if claims[player].is_empty() && playing.game.public().eliminated(player) {
                    continue;
                }
                ui.label(egui::RichText::new(name_of(player)).size(14.0));
                for trip in 0..trips {
                    claim_box(
                        ui,
                        playing.colours[player],
                        claims[player].get(trip).copied(),
                    );
                }
                ui.end_row();
            }
        });
}

/// How likely the claimant's own claim was true, from what they could see when they made
/// it.
///
/// Computed here rather than passed in, so it is the same calculation whoever claimed —
/// an opponent's line and the human's line mean the same thing. The complement for a liar
/// call is taken exactly and only then rounded for writing.
fn claimant_odds(game: &Game, actor: usize, call: Call) -> f64 {
    match call {
        Call::Raise(claim) => claimant_odds_for(game, actor, claim, false),
        Call::Liar => match game.public().previous_claim() {
            Some(claim) => 100.0 - claimant_odds_for(game, actor, claim, false),
            None => 0.0,
        },
        Call::SpotOn => match game.public().previous_claim() {
            Some(claim) => claimant_odds_for(game, actor, claim, true),
            None => 0.0,
        },
    }
}

/// How likely one claim is, from what one player can see.
fn claimant_odds_for(game: &Game, player: usize, claim: Claim, exactly: bool) -> f64 {
    // The claimant's own view: their dice, and the count of what they cannot see.
    let view = game.view(player);
    if exactly {
        readout::percent(&view.chance_exactly(claim))
    } else {
        readout::percent(&view.chance_of(claim))
    }
}

/// Whether what actually happened to a claim was unlikely enough to call out.
///
/// One rule covers both directions: the measure is the chance of the outcome that
/// occurred. A claim that was ninety-seven per cent likely and failed was a three per cent
/// outcome; a two per cent claim that came true was a two per cent outcome. Same scale,
/// same meaning.
///
/// The chance is exact until this point. A standard deviation would need the normal
/// approximation, which wants far more dice than this game ever has — forty-nine unknown
/// dice at the very most, and four when a player is nearly out.
fn luck(player: usize, chance_true: f64, came_true: bool) -> Option<Event> {
    let chance_of_outcome = if came_true {
        chance_true
    } else {
        100.0 - chance_true
    };
    if chance_of_outcome > LUCK {
        return None;
    }
    let seat = seat_of(player);
    Some(if came_true {
        Event::Lucky {
            seat,
            odds: chance_of_outcome,
        }
    } else {
        Event::Unlucky {
            seat,
            odds: chance_of_outcome,
        }
    })
}

/// Advance every opponent's turn until it is the human's, or the round ends.
fn run_opponents(playing: &mut Playing) -> Option<Screen> {
    while !playing.game.public().over() && playing.game.public().current() != HUMAN {
        let player = playing.game.public().current();
        let call = ai::choose(&playing.game.view(player));
        if let Some(screen) = apply(playing, call) {
            return Some(screen);
        }
    }
    playing.pending = first_legal(&playing.game);
    None
}

/// Apply a call and move to the reveal if the round ended.
fn apply(playing: &mut Playing, call: Call) -> Option<Screen> {
    let disputed = playing.game.public().previous_claim();
    let actor = playing.game.public().current();
    // Resolving a round clears its claims from the match, so they are kept here first.
    let round: Vec<Entry> = playing.game.public().history().to_vec();
    let odds = claimant_odds(&playing.game, actor, call);
    let previous_player = playing.game.public().previous_player();
    // Judging luck needs the round's dice, which beginning the next round forgets.
    let before = playing.game.clone();

    let claimed = playing
        .game
        .claim(call)
        .expect("only legal calls are offered");

    match claimed {
        Claimed::Continues => {
            if let Call::Raise(claim) = call {
                playing.audit.record(Event::Claim {
                    seat: seat_of(actor),
                    quantity: claim.quantity,
                    face: claim.face,
                    odds,
                });
            }
            None
        }
        Claimed::RoundEnded => {
            // A desktop game holds every die, so the round settles on the first ask. A
            // networked client gets `None` here until the relay reveals the round.
            let Resolution {
                opens,
                affected: _,
                lost,
                gained,
                shield_gained,
                shield_spent,
                eliminated,
                truth,
                over,
            } = playing
                .game
                .resolve()
                .expect("a desktop game knows every die");
            // Now that the round is revealed, every seat's dice may be shown.
            let snapshot: Vec<Vec<u8>> = (0..playing.colours.len())
                .map(|player| {
                    playing
                        .game
                        .view(HUMAN)
                        .dice_of(player)
                        .map(|dice| dice.to_vec())
                        .unwrap_or_default()
                })
                .collect();
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
            if let Some(player) = shield_gained {
                said.push_str(&format!(" {} gained a shield token.", name_of(player)));
            }
            if let Some(player) = lost {
                said.push_str(&format!(" {} lost a die.", name_of(player)));
            }
            if let Some(player) = shield_spent {
                said.push_str(&format!(
                    " {} spent a shield token instead of losing a die.",
                    name_of(player)
                ));
            }
            if let Some(player) = eliminated {
                said.push_str(&format!(" {} is out.", name_of(player)));
            }

            playing.audit.record(match call {
                Call::Liar => Event::Liar {
                    seat: seat_of(actor),
                    odds,
                },
                Call::SpotOn => Event::SpotOn {
                    seat: seat_of(actor),
                    odds,
                },
                Call::Raise(_) => unreachable!("a raise does not resolve a round"),
            });
            if let Some(player) = gained {
                playing.audit.record(Event::GainDie {
                    seat: seat_of(player),
                });
            }
            if let Some(player) = shield_gained {
                playing.audit.record(Event::GainShield {
                    seat: seat_of(player),
                });
            }
            if let Some(player) = lost {
                playing.audit.record(Event::LoseDie {
                    seat: seat_of(player),
                });
            }
            if let Some(player) = shield_spent {
                playing.audit.record(Event::SpendShield {
                    seat: seat_of(player),
                });
            }
            if let Some(player) = eliminated {
                playing.audit.record(Event::Eliminate {
                    seat: seat_of(player),
                });
            }

            // The disputed claim, judged from what its maker could see. Their own dice and
            // the unknown count did not change during the round, so recomputing it now
            // gives the chance they had in front of them.
            let held = truth >= claim.quantity;
            if let Some(maker) = previous_player
                && let Some(event) =
                    luck(maker, claimant_odds_for(&before, maker, claim, false), held)
            {
                playing.audit.record(event);
            }
            // The call that ended the round, judged the same way.
            let call_held = match call {
                Call::Liar => !held,
                Call::SpotOn => truth == claim.quantity,
                Call::Raise(_) => unreachable!("a raise does not resolve a round"),
            };
            if let Some(event) = luck(actor, odds, call_held) {
                playing.audit.record(event);
            }

            playing.revealed = snapshot;
            playing.round = round;
            playing.verdict = Some(said);
            if over {
                playing
                    .audit
                    .record_reveal(&playing.dealer, playing.game.public().round());
                playing.audit.record(Event::Winner {
                    seat: seat_of(*playing.game.public().ranking().first().expect("a survivor")),
                });
                Some(Screen::Over)
            } else {
                playing.audit.record(Event::NextTurn {
                    seat: seat_of(opens),
                });
                // The round that just ended is revealed, then the next one is begun,
                // dealt, and committed to. Settling a round no longer deals the next:
                // a client is dealt by the relay, and a desktop game by its own dealer,
                // so both do it here rather than inside the rules.
                let players = playing.colours.len();
                playing
                    .audit
                    .record_reveal(&playing.dealer, playing.game.public().round());
                playing.game.next_round(opens);
                playing.dealer.deal(&mut playing.game);
                playing
                    .audit
                    .record_deal(&playing.game, &playing.dealer, players);
                Some(Screen::Reveal)
            }
        }
    }
}

fn play(ui: &mut egui::Ui, playing: &mut Playing) -> Option<Screen> {
    // Opponents act before the human on the first visit to this screen.
    if playing.game.public().current() != HUMAN
        && let Some(screen) = run_opponents(playing)
    {
        return Some(screen);
    }

    table(ui, playing, false);
    ui.separator();
    standing(ui, playing);
    history(ui, playing, playing.game.public().history());
    ui.separator();

    let game = &playing.game;
    let view = game.view(HUMAN);

    ui.label(egui::RichText::new("Your claim").strong());
    let pending = playing.pending;
    let step = |claim: Claim| game.public().check_raise(claim).is_ok();

    let mut dialled = pending;
    egui::Grid::new("dialler")
        .spacing([6.0, 6.0])
        .show(ui, |ui| {
            ui.label(egui::RichText::new("quantity").size(16.0));
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
            dialled_value(ui, pending.quantity);
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
            ui.end_row();

            ui.label(egui::RichText::new("face").size(16.0));
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
            dialled_value(ui, pending.face as u32);
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
            ui.end_row();
        });
    playing.pending = dialled;

    odds(ui, &view, playing.pending, game.public().previous_claim());

    ui.separator();
    let has_previous = game.public().previous_claim().is_some();
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
    standing(ui, playing);
    history(ui, playing, &playing.round.clone());
    ui.separator();
    if tap(ui, "Next round", true) {
        playing.revealed.clear();
        playing.round.clear();
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
    history(ui, playing, &playing.round.clone());
    ui.separator();
    ui.label(egui::RichText::new("Finishing order").strong());
    egui::Grid::new("ranking")
        .spacing([5.0, 5.0])
        .show(ui, |ui| {
            for (place, player) in playing.game.public().ranking().iter().enumerate() {
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

    /// Face fairness belongs to the integrity crate now, which does the dealing and
    /// tests its own rejection sampling against a deliberately biased implementation.
    /// What is left here is that the same seed decides the same match.
    #[test]
    fn a_seed_always_produces_the_same_match() {
        let mut first = Dice::new(99);
        let mut second = Dice::new(99);
        let drawn: Vec<u64> = (0..200).map(|_| first.next()).collect();
        let again: Vec<u64> = (0..200).map(|_| second.next()).collect();
        assert_eq!(drawn, again, "the turn order would not be reproducible");
        assert_eq!(
            first.secret().0,
            second.secret().0,
            "the same seed must deal the same match"
        );
    }

    /// The commitment is to the round that is about to be dealt, and the reveal lets a
    /// reader recompute it. This is the property the whole scheme exists for.
    #[test]
    fn a_revealed_seed_recomputes_the_dice_it_was_committed_to() {
        let mut dice = Dice::new(1234);
        let dealer = Dealer {
            secret: dice.secret(),
            match_id: "1970-01-01T00-00-00-000".to_string(),
        };
        let digest = integrity::commit(&dealer.seed(0));
        let dealt = dealer.seat(0, 2);
        assert!(integrity::verify(&digest, &dealer.seed(0)));
        assert_eq!(
            integrity::deal(&dealer.seed(0), 2, SEAT),
            dealt.to_vec(),
            "the revealed seed must recompute what was dealt"
        );
        assert_ne!(
            dealer.seat(1, 2),
            dealt,
            "a later round must not repeat the dice"
        );
    }

    #[test]
    fn an_audit_log_is_one_file_per_match_with_one_event_per_line() {
        let directory = std::env::temp_dir().join("liars-dice-audit-test");
        let _ = std::fs::remove_dir_all(&directory);

        // A millisecond apart, so the two matches do not share a file name.
        for nanos in [1_000_000u64, 2_000_000] {
            let mut audit = Audit::open(&directory, Moment::from_unix_nanos(nanos));
            audit.record(Event::Join {
                seat: Seat(0),
                name: "You".into(),
                colour: "red".into(),
                account: Account::Local,
            });
            audit.record(Event::Deal {
                seat: Seat(0),
                dice: vec![1, 2, 3, 4, 5],
            });
            audit.record(Event::Winner { seat: Seat(0) });
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
            assert_eq!(
                lines,
                vec!["join #0 You red local", "deal #0 1 2 3 4 5", "winner #0"]
            );
        }

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The property the integrity scheme exists for, end to end: play a match, write its
    /// log, then verify it holding nothing but the file. Each round's revealed seed must
    /// hash to the commitment published before any claim in that round, and must
    /// recompute every face on every deal line.
    #[test]
    fn an_archived_match_verifies_from_the_log_alone() {
        let directory = std::env::temp_dir().join("liars-dice-archive-test");
        let _ = std::fs::remove_dir_all(&directory);

        let at = Moment::from_unix_nanos(1_700_000_000_000_000_000);
        let mut dice = Dice::new(20_261_118);
        let dealer = Dealer {
            secret: dice.secret(),
            match_id: at.file_stem(),
        };
        let players = 3;
        let order: Vec<usize> = (0..players).collect();
        let mut game = Game::start(order, Variants::default());
        dealer.deal(&mut game);

        let mut audit = Audit::open(&directory, at);
        audit.record(Event::Version {
            protocol: PROTOCOL_VERSION,
        });
        audit.record(Event::Started { at });
        audit.record_deal(&game, &dealer, players);

        let mut turn = 0;
        while !game.public().over() && turn < 2_000 {
            let call = if turn % 4 == 3 && game.public().previous_claim().is_some() {
                Call::Liar
            } else {
                match game.public().legal_raises().first() {
                    Some(&claim) => Call::Raise(claim),
                    None => Call::Liar,
                }
            };
            if game.claim(call).expect("a legal call") == Claimed::RoundEnded {
                let round = game.public().round();
                let resolution = game.resolve().expect("a complete game settles at once");
                audit.record_reveal(&dealer, round);
                if !resolution.over {
                    game.next_round(resolution.opens);
                    dealer.deal(&mut game);
                    audit.record_deal(&game, &dealer, players);
                }
            }
            turn += 1;
        }
        assert!(game.public().over(), "the match should have ended");

        let file = std::fs::read_dir(&directory)
            .expect("the log directory was created")
            .map(|entry| entry.unwrap().path())
            .next()
            .expect("one log file");
        let text = std::fs::read_to_string(&file).expect("the log is readable");

        // From here on, nothing but the text of the log.
        struct Round {
            digest: String,
            seed: Option<String>,
            deals: Vec<(u8, Vec<u8>)>,
        }
        let mut rounds: Vec<Round> = Vec::new();
        for line in text.lines() {
            let mut parts = line.split(' ');
            match parts.next() {
                Some("commit") => rounds.push(Round {
                    digest: parts.next().expect("a digest").to_string(),
                    seed: None,
                    deals: Vec::new(),
                }),
                Some("deal") => {
                    let seat: u8 = parts
                        .next()
                        .expect("a seat")
                        .trim_start_matches('#')
                        .parse()
                        .expect("a seat number");
                    let faces: Vec<u8> = parts.map(|face| face.parse().expect("a face")).collect();
                    rounds
                        .last_mut()
                        .expect("a round")
                        .deals
                        .push((seat, faces));
                }
                Some("reveal") => {
                    let seed = parts.next().expect("a seed").to_string();
                    let open = rounds
                        .iter_mut()
                        .rev()
                        .find(|round| round.seed.is_none())
                        .expect("an unrevealed round");
                    open.seed = Some(seed);
                }
                _ => {}
            }
        }

        assert!(
            rounds.len() > 1,
            "the match should have taken more than one round"
        );
        for (number, round) in rounds.iter().enumerate() {
            let seed = Seed::from_hex(round.seed.as_ref().expect("every round is revealed"))
                .expect("the seed is hexadecimal");
            let digest = integrity::Commitment::from_hex(&round.digest).expect("a digest");
            assert!(
                integrity::verify(&digest, &seed),
                "round {number}: the revealed seed is not the one committed to"
            );
            assert_eq!(
                round.deals.len(),
                players,
                "round {number}: a seat was not dealt"
            );
            for (seat, faces) in &round.deals {
                assert_eq!(
                    integrity::deal(&seed, *seat, faces.len()),
                    *faces,
                    "round {number} seat {seat}: the log disagrees with its own seed"
                );
            }
        }

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A directory that cannot be opened must not take the game down with it.
    #[test]
    fn a_log_that_cannot_be_written_is_survivable() {
        let impossible = std::path::Path::new("logs").join(" ");
        let mut audit = Audit::open(&impossible, Moment::from_unix_nanos(0));
        audit.record(Event::Eliminate { seat: Seat(1) });
    }
}
