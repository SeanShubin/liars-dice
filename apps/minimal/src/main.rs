//! The minimal game. Mechanics only: two dimensional, one human against AI opponents.
//!
//! Composition root. No behavior: the rules come from `rules`, the probabilities from
//! `odds`, the opponents from `ai`, the palette from `render-minimal`. What lives here is
//! the window, the screens, and the one place non-determinism enters.
//!
//! Realizes /specification/game-minimal.md.

use eframe::egui;

use std::io::Write;

use ai::Seen;
use integrity::{Secret, Seed};
use protocol::{Account, Event, Moment, PROTOCOL_VERSION, Seat};
use render_minimal::palette::{PALETTE, assign};
use render_minimal::{
    Seating, calls, dialler, history, odds, opening_claim, palette_picker, ranking, standing,
    table, tap, verdict,
};
use rules::{Call, Claim, Claimed, Game, Resolution, SEAT, Variants};

const MAX_PLAYERS: usize = 10;
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
                eprintln!("no audit log: {reason}");
                Self { file: None }
            }
        }
    }

    fn record(&mut self, event: Event) {
        if let Some(file) = self.file.as_mut()
            && let Err(reason) = writeln!(file, "{event}")
        {
            eprintln!("audit log write failed: {reason}");
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
    seating: Seating,
    dealer: Dealer,
    /// The claim the human is dialling in. Always legal.
    pending: Claim,
    /// Which seat opens the next round, once this one has been settled and shown.
    ///
    /// Set when a round resolves, taken when the player moves on. Holding it here is what
    /// keeps the game on the round being shown: `Game::resolve` reveals every die and
    /// leaves the round's claims alone, and it is `Game::next_round` that clears them, so
    /// until that call the reveal screen reads the real match rather than a snapshot of
    /// one.
    opens: Option<usize>,
    verdict: Option<String>,
    audit: Audit,
}

/// The human is always player zero. Turn order is a shuffled permutation, so where they
/// sit varies.
const HUMAN: usize = 0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Setup { colour: usize, opponents: usize },
    Play,
    Reveal,
    Over,
}

/// The window: which screen is showing, and the match behind it.
///
/// The match is held separately from the screen so that changing screens never moves it.
struct Minimal {
    screen: Screen,
    playing: Option<Playing>,
}

fn main() -> eframe::Result {
    eframe::run_native(
        "Liars Dice",
        eframe::NativeOptions::default(),
        Box::new(|_| {
            Ok(Box::new(Minimal {
                screen: Screen::Setup {
                    colour: 1,
                    opponents: 3,
                },
                playing: None,
            }))
        }),
    )
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
    let pending = opening_claim(&game.view(HUMAN));

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
        seating: Seating::new((0..players).map(name_of).collect(), colours),
        dealer,
        pending,
        opens: None,
        verdict: None,
        audit,
    }
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

impl eframe::App for Minimal {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.screen(ui));
        });
    }
}

impl Minimal {
    /// One screen at a time. Which one is a value, so the transitions are all in one
    /// place rather than spread through the screens that cause them.
    fn screen(&mut self, ui: &mut egui::Ui) {
        let mut next: Option<Screen> = None;
        let mut current = self.screen;
        match &mut current {
            Screen::Setup { colour, opponents } => {
                if let Some(started) = setup(ui, colour, opponents) {
                    self.playing = Some(started);
                    next = Some(Screen::Play);
                } else {
                    next = Some(Screen::Setup {
                        colour: *colour,
                        opponents: *opponents,
                    });
                }
            }
            Screen::Play => {
                if let Some(playing) = self.playing.as_mut() {
                    next = play(ui, playing);
                }
            }
            Screen::Reveal => {
                if let Some(playing) = self.playing.as_mut() {
                    next = reveal(ui, playing);
                }
            }
            Screen::Over => {
                if let Some(playing) = self.playing.as_mut() {
                    next = over(ui, playing);
                }
            }
        }
        if let Some(screen) = next {
            self.screen = screen;
        }
    }
}

fn setup(ui: &mut egui::Ui, colour: &mut usize, opponents: &mut usize) -> Option<Playing> {
    ui.heading("Your dice");
    ui.label(PALETTE[*colour].name);

    palette_picker(ui, colour);

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
    playing.pending = opening_claim(&playing.game.view(HUMAN));
    None
}

/// Apply a call and move to the reveal if the round ended.
fn apply(playing: &mut Playing, call: Call) -> Option<Screen> {
    let disputed = playing.game.public().previous_claim();
    let actor = playing.game.public().current();
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
            let resolution = playing
                .game
                .resolve()
                .expect("a desktop game knows every die");
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
            } = resolution;
            let claim = disputed.expect("a resolution always disputes a claim");
            let said = verdict(&playing.seating, actor, call, claim, &resolution);

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
                // The game stays on the round that just ended, because that is the
                // round the reveal screen shows. Beginning the next one is what the
                // `Next round` button does.
                playing.opens = Some(opens);
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

    let view = playing.game.view(HUMAN);
    table(ui, &view, &playing.seating);
    ui.separator();
    standing(ui, &view, &playing.seating);
    history(ui, &view, &playing.seating);
    ui.separator();

    ui.label(egui::RichText::new("Your claim").strong());
    let mut pending = playing.pending;
    dialler(ui, &view, &mut pending);
    odds(ui, &view, pending);
    ui.separator();
    let call = calls(ui, &view, pending);
    playing.pending = pending;

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
    let view = playing.game.view(HUMAN);
    table(ui, &view, &playing.seating);
    ui.separator();
    standing(ui, &view, &playing.seating);
    history(ui, &view, &playing.seating);
    ui.separator();
    // The round that just ended is written to the log, then the next one is begun, dealt
    // and committed to. Settling a round does not deal the next: a client is dealt by the
    // relay and a desktop game by its own dealer, so both do it here rather than in the
    // rules.
    if tap(ui, "Next round", true)
        && let Some(opens) = playing.opens.take()
    {
        let players = playing.seating.len();
        playing
            .audit
            .record_reveal(&playing.dealer, playing.game.public().round());
        playing.game.next_round(opens);
        playing.dealer.deal(&mut playing.game);
        playing
            .audit
            .record_deal(&playing.game, &playing.dealer, players);
        playing.verdict = None;
        playing.pending = opening_claim(&playing.game.view(HUMAN));
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
    let view = playing.game.view(HUMAN);
    table(ui, &view, &playing.seating);
    ui.separator();
    history(ui, &view, &playing.seating);
    ui.separator();
    ranking(ui, &view, &playing.seating);
    ui.separator();
    if tap(ui, "New match", true) {
        return Some(Screen::Setup {
            colour: playing.seating.colour(HUMAN),
            opponents: playing.seating.len() - 1,
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
