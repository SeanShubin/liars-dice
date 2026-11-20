//! Game rules. Pure and deterministic. No input, no output, no rendering, no randomness.
//!
//! Realizes /specification/game-rules.md and /implementation/game-state.md.
//!
//! Three things and the operations between them:
//!
//! - [`Public`] is what every player knows. Every client holds an identical copy, so two
//!   clients disagreeing about it is a bug rather than a consequence of knowing different
//!   things.
//! - [`Game`] is the whole game: `Public` plus what dice are known. A seat's dice are an
//!   `Option`, so ignorance is in the type rather than in a convention to remember.
//! - [`View`] is one player's view of a game: a borrow and a seat. It hides every other
//!   seat's dice **even when the game knows them**, which is what lets a harness holding
//!   the whole game still prove something about a client that does not.
//!
//! Dice arrive through [`Game::install`]. This crate deals nothing and rolls nothing, so
//! the non-determinism lives entirely in a composition root, which is what
//! /specification/architecture.md asks for.

/// Variant settings, fixed for a whole match.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Variants {
    pub ones_wild: bool,
}

/// A raise: at least this many of this face among all players.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Claim {
    pub quantity: u32,
    pub face: u8,
}

/// What a player does on their turn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Call {
    Raise(Claim),
    Liar,
    SpotOn,
}

/// One claim of the current round: who claimed, how many dice they held, what they said.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub player: usize,
    pub dice_held: u32,
    pub claim: Claim,
}

/// What recording a call did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Claimed {
    /// The turn passed and the round goes on.
    Continues,
    /// Liar or spot on was called. The round is closed and waits for its dice.
    RoundEnded,
}

/// What a resolution did. Exactly one player is affected by it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Resolution {
    /// Who makes the first claim of the next round.
    pub opens: usize,
    /// The one player the resolution affected: they lost a die, gained a die, gained a
    /// shield token, or spent one. Reported rather than left to be re-derived, because a
    /// spent token is not recoverable from the other fields.
    pub affected: usize,
    pub lost: Option<usize>,
    pub gained: Option<usize>,
    pub shield_gained: Option<usize>,
    pub shield_spent: Option<usize>,
    pub eliminated: Option<usize>,
    /// How many dice actually counted toward the disputed claim.
    pub truth: u32,
    pub over: bool,
}

/// Why a call was rejected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rejected {
    NotThisPlayersTurn,
    NoPreviousClaim,
    QuantityExceedsDiceInPlay,
    FaceOutOfRange,
    OnesNotClaimableWhenWild,
    NotStrongerThanPreviousClaim,
    MatchIsOver,
    RoundHasEnded,
}

/// A seat's dice were already installed for this round.
///
/// A late or duplicated reveal must not be able to rewrite a round that has been played.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AlreadyInstalled;

/// Faces are numbered one through six.
pub const FACES: std::ops::RangeInclusive<u8> = 1..=6;

/// Each player starts with five dice and holds at most five. The same five, so it is one
/// constant rather than a parameter a match could be started above.
pub const DICE_LIMIT: u32 = 5;

/// How many faces a seat is dealt: the dice limit, whatever its player holds.
pub const SEAT: usize = DICE_LIMIT as usize;

/// What every player knows.
///
/// Derivable from the event log by anyone, and identical in every client. This is the
/// state /specification/architecture.md means by "every client derives the same match
/// state from the same events".
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Public {
    counts: Vec<u32>,
    shields: Vec<u32>,
    order: Vec<usize>,
    eliminated: Vec<usize>,
    history: Vec<Entry>,
    variants: Variants,
    current: usize,
    round: u32,
}

impl Public {
    pub fn variants(&self) -> Variants {
        self.variants
    }

    pub fn current(&self) -> usize {
        self.current
    }

    /// Which round the match is in, counting from zero.
    pub fn round(&self) -> u32 {
        self.round
    }

    pub fn history(&self) -> &[Entry] {
        &self.history
    }

    /// Turn order, including players who have been eliminated. Fixed at match start.
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    pub fn players(&self) -> usize {
        self.counts.len()
    }

    pub fn count(&self, player: usize) -> u32 {
        self.counts[player]
    }

    /// How many shield tokens this player holds. Every player can see this for every
    /// player.
    pub fn shields(&self, player: usize) -> u32 {
        self.shields[player]
    }

    pub fn eliminated(&self, player: usize) -> bool {
        self.counts[player] == 0
    }

    /// Players still in the match, in turn order.
    pub fn alive(&self) -> Vec<usize> {
        self.order
            .iter()
            .copied()
            .filter(|&player| !self.eliminated(player))
            .collect()
    }

    pub fn dice_in_play(&self) -> u32 {
        self.counts.iter().sum()
    }

    pub fn over(&self) -> bool {
        self.alive().len() <= 1
    }

    /// Finishing order, best first: the survivor, then the eliminated in reverse order of
    /// elimination. A player lasts longer than another if the other is eliminated first.
    pub fn ranking(&self) -> Vec<usize> {
        let mut ranked = self.alive();
        ranked.extend(self.eliminated.iter().rev().copied());
        ranked
    }

    /// The claim a call must beat, if there is one this round.
    pub fn previous_claim(&self) -> Option<Claim> {
        self.history.last().map(|entry| entry.claim)
    }

    /// The player the current player is in an exchange with.
    pub fn previous_player(&self) -> Option<usize> {
        self.history.last().map(|entry| entry.player)
    }

    /// The next player in turn order, skipping anyone eliminated.
    pub fn next_player(&self, from: usize) -> usize {
        let alive = self.alive();
        let at = alive.iter().position(|&player| player == from).unwrap_or(0);
        alive[(at + 1) % alive.len()]
    }

    /// Whether a raise is legal. Needs no dice, so any client can answer it at any time.
    pub fn check_raise(&self, claim: Claim) -> Result<(), Rejected> {
        if !FACES.contains(&claim.face) {
            return Err(Rejected::FaceOutOfRange);
        }
        if self.variants.ones_wild && claim.face == 1 {
            return Err(Rejected::OnesNotClaimableWhenWild);
        }
        if claim.quantity < 1 || claim.quantity > self.dice_in_play() {
            return Err(Rejected::QuantityExceedsDiceInPlay);
        }
        match self.previous_claim() {
            None => Ok(()),
            Some(previous) => {
                let stronger = claim.quantity > previous.quantity
                    || (claim.quantity == previous.quantity && claim.face > previous.face);
                if stronger {
                    Ok(())
                } else {
                    Err(Rejected::NotStrongerThanPreviousClaim)
                }
            }
        }
    }

    /// Every legal raise, in ascending order.
    pub fn legal_raises(&self) -> Vec<Claim> {
        let mut claims = Vec::new();
        for quantity in 1..=self.dice_in_play() {
            for face in FACES {
                let claim = Claim { quantity, face };
                if self.check_raise(claim).is_ok() {
                    claims.push(claim);
                }
            }
        }
        claims
    }

    /// Whether a call can be made at all. Liar and spot on both refer to the previous
    /// claim, so neither is available when there is none.
    pub fn check(&self, call: Call) -> Result<(), Rejected> {
        if self.over() {
            return Err(Rejected::MatchIsOver);
        }
        match call {
            Call::Raise(claim) => self.check_raise(claim),
            Call::Liar | Call::SpotOn => {
                if self.previous_claim().is_some() {
                    Ok(())
                } else {
                    Err(Rejected::NoPreviousClaim)
                }
            }
        }
    }
}

/// The whole game: what everyone knows, and what dice are known.
///
/// A complete game — an archiver's, a replay's, a desktop game's — has every seat's dice.
/// A client mid-round has only its own. Both run the same operations; they differ only in
/// when [`Game::resolve`] starts returning `Some`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Game {
    public: Public,
    dice: Vec<Option<[u8; SEAT]>>,
    /// The call that closed the round, waiting for the dice that settle it.
    pending: Option<Call>,
    /// Whether this round's dice have been shown to everyone.
    revealed: bool,
    /// The resolution, once computed. Kept so that asking twice answers twice with the
    /// same thing rather than applying it twice: several things may want to know how a
    /// round went, and none of them should move a die by asking.
    settled: Option<Resolution>,
}

impl Game {
    /// `order` is the turn order, chosen at random when the match starts and not
    /// reordered afterwards. Players are identified by their index into it.
    ///
    /// No dice yet: deal them with [`Game::install`].
    pub fn start(order: Vec<usize>, variants: Variants) -> Self {
        let players = order.len();
        Self {
            public: Public {
                counts: vec![DICE_LIMIT; players],
                shields: vec![0; players],
                current: order[0],
                order,
                eliminated: Vec::new(),
                history: Vec::new(),
                variants,
                round: 0,
            },
            dice: vec![None; players],
            pending: None,
            revealed: false,
            settled: None,
        }
    }

    pub fn public(&self) -> &Public {
        &self.public
    }

    /// This seat's view of the game. Hides every other seat's dice until the round is
    /// revealed, whatever this game happens to know.
    pub fn view(&self, seat: usize) -> View<'_> {
        View { game: self, seat }
    }

    /// Take a seat's dice for this round.
    ///
    /// Rejected if that seat is already dealt, so a late or duplicated reveal cannot
    /// rewrite a round. Whether the dice agree with the round's seed is the caller's
    /// obligation: this crate knows nothing of seeds.
    /// A player has left the match, so they hold no dice and take no further turn.
    ///
    /// An external input rather than a resolution. Who is taking part is coordination —
    /// the relay's business, and it records a departure without knowing what one means.
    /// This is the rules answering it: `A player who leaves a match is eliminated`.
    ///
    /// Immediate, not deferred to the round's end. The alternative deadlocks: a player
    /// who leaves while it is their turn would have to be waited for until a boundary
    /// that only their own turn can reach. The cost is that a claim already made this
    /// round is judged against a smaller pool of dice than it was made against, which can
    /// turn a true claim false through nobody's fault. Nothing a departing player can
    /// gain by it — they are out either way — but somebody else can lose by it, and that
    /// is the trade.
    ///
    /// Idempotent, and a no-op for a player who is already out, which is the common case:
    /// an eliminated player leaving is the whole reason this exists.
    pub fn concede(&mut self, player: usize) {
        if player >= self.public.counts.len() || self.public.counts[player] == 0 {
            return;
        }
        self.public.counts[player] = 0;
        self.public.eliminated.push(player);
        // The turn moves on if it was theirs, because `next_player` skips the eliminated
        // and nobody else may act in their place.
        if self.public.current == player {
            self.public.current = self.public.next_player(player);
        }
    }

    pub fn install(&mut self, seat: usize, dice: [u8; SEAT]) -> Result<(), AlreadyInstalled> {
        if self.dice[seat].is_some() {
            return Err(AlreadyInstalled);
        }
        self.dice[seat] = Some(dice);
        Ok(())
    }

    /// Every face dealt to a seat, in play or not.
    ///
    /// For writing the event log and for verifying it later, where what was dealt is the
    /// thing being attested. Never for display and never for counting: a [`View`] is what
    /// answers "what may this seat see", and it answers differently.
    pub fn dealt(&self, seat: usize) -> Option<[u8; SEAT]> {
        self.dice[seat]
    }

    /// Whether every seat's dice are known, which is what [`Game::resolve`] waits for.
    pub fn dice_known(&self) -> bool {
        self.dice.iter().all(Option::is_some)
    }

    /// Whether this round's dice have been shown to everyone.
    pub fn revealed(&self) -> bool {
        self.revealed
    }

    /// Whether a call has closed the round and it is waiting to be settled.
    pub fn round_ended(&self) -> bool {
        self.pending.is_some()
    }

    /// Record a call. A raise passes the turn; liar and spot on close the round.
    ///
    /// Closing a round does not settle it, because the dice that settle it may not have
    /// arrived. See [`Game::resolve`].
    pub fn claim(&mut self, call: Call) -> Result<Claimed, Rejected> {
        if self.pending.is_some() {
            return Err(Rejected::RoundHasEnded);
        }
        self.public.check(call)?;
        match call {
            Call::Raise(claim) => {
                self.public.history.push(Entry {
                    player: self.public.current,
                    dice_held: self.public.counts[self.public.current],
                    claim,
                });
                self.public.current = self.public.next_player(self.public.current);
                Ok(Claimed::Continues)
            }
            Call::Liar | Call::SpotOn => {
                self.pending = Some(call);
                Ok(Claimed::RoundEnded)
            }
        }
    }

    /// How many dice across the whole table count toward a face.
    ///
    /// `None` while any seat's dice are unknown, which is every client's situation until
    /// the round is revealed.
    pub fn truth(&self, face: u8) -> Option<u32> {
        if !self.dice_known() {
            return None;
        }
        Some(
            (0..self.public.players())
                .map(|player| count_face(self.held(player), face, self.public.variants.ones_wild))
                .sum(),
        )
    }

    /// Settle the round that a liar or spot on closed.
    ///
    /// `None` until every seat's dice are known. A desktop game, which deals locally,
    /// gets `Some` on the first ask; a client gets `None` until the relay reveals. Both
    /// run this same code, differing only in when the dice arrive.
    pub fn resolve(&mut self) -> Option<Resolution> {
        if let Some(settled) = self.settled {
            return Some(settled);
        }
        let call = self.pending?;
        let previous = self.public.history.last().copied()?;
        let truth = self.truth(previous.claim.face)?;
        let current = self.public.current;

        let (mut lost, mut gained) = match call {
            Call::Liar => {
                // Whoever was wrong loses a die: the caller if the claim held, otherwise
                // the player who made it.
                let claim_held = truth >= previous.claim.quantity;
                let loser = if claim_held { current } else { previous.player };
                (Some(loser), None)
            }
            Call::SpotOn => {
                if truth == previous.claim.quantity {
                    (None, Some(current))
                } else {
                    (Some(current), None)
                }
            }
            Call::Raise(_) => unreachable!("a raise does not close a round"),
        };

        let mut shield_gained = None;
        let mut shield_spent = None;

        // A correct claim below the dice limit gains a die; at the limit it gains a
        // shield token instead, which is what holds the limit without a check elsewhere.
        if let Some(player) = gained {
            if self.public.counts[player] < DICE_LIMIT {
                self.public.counts[player] += 1;
            } else {
                self.public.shields[player] += 1;
                shield_gained = Some(player);
                gained = None;
            }
        }

        // A shield token prevents its holder's next die loss, and is spent doing so.
        let mut eliminated = None;
        if let Some(player) = lost {
            if self.public.shields[player] > 0 {
                self.public.shields[player] -= 1;
                shield_spent = Some(player);
                lost = None;
            } else {
                self.public.counts[player] -= 1;
                if self.public.counts[player] == 0 {
                    self.public.eliminated.push(player);
                    eliminated = Some(player);
                }
            }
        }

        // Exactly one player in an exchange is affected by its resolution. Exactly one of
        // these four is set, because exactly one was set on the way in and each branch
        // above moves it rather than adding to it.
        let affected = gained
            .or(lost)
            .or(shield_gained)
            .or(shield_spent)
            .expect("exactly one player in an exchange is affected by its resolution");

        // The affected player makes the first claim of the next round. If that player is
        // eliminated, the other player in the exchange does. The other player lost no
        // dice this round, so they are never also eliminated.
        let opens = if self.public.counts[affected] == 0 {
            if affected == current {
                previous.player
            } else {
                current
            }
        } else {
            affected
        };

        self.revealed = true;
        let resolution = Resolution {
            opens,
            affected,
            lost,
            gained,
            shield_gained,
            shield_spent,
            eliminated,
            truth,
            over: self.public.over(),
        };
        self.settled = Some(resolution);
        Some(resolution)
    }

    /// Clear the round and begin the next. The dice are forgotten, so the next round must
    /// be installed before it can be settled.
    pub fn next_round(&mut self, opens: usize) {
        self.public.history.clear();
        self.public.current = opens;
        self.public.round += 1;
        self.dice = vec![None; self.public.players()];
        self.pending = None;
        self.revealed = false;
        self.settled = None;
    }

    /// The dice this player holds: the prefix of what their seat was dealt. Every seat is
    /// dealt the dice limit, so the rest are not in play and must never be counted.
    fn held(&self, player: usize) -> &[u8] {
        match &self.dice[player] {
            Some(dealt) => &dealt[..self.public.counts[player] as usize],
            None => &[],
        }
    }
}

/// One player's view of a game.
///
/// The lens that makes a test over a complete game mean something about a client with
/// partial knowledge: another seat's dice stay hidden until the round is revealed, whether
/// or not the underlying game holds them.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    game: &'a Game,
    seat: usize,
}

impl<'a> View<'a> {
    pub fn seat(&self) -> usize {
        self.seat
    }

    pub fn public(&self) -> &'a Public {
        &self.game.public
    }

    /// This seat's own dice. `None` before they are dealt.
    pub fn own_dice(&self) -> Option<&'a [u8]> {
        self.game.dice[self.seat].as_ref()?;
        Some(self.game.held(self.seat))
    }

    /// Another seat's dice, which stay hidden until the round is revealed.
    ///
    /// This is the one line that makes a virtual player's success say anything about a
    /// real one: the game may know, and the view still does not tell.
    pub fn dice_of(&self, seat: usize) -> Option<&'a [u8]> {
        if seat == self.seat || self.game.revealed {
            self.game.dice[seat].as_ref()?;
            Some(self.game.held(seat))
        } else {
            None
        }
    }

    /// How many dice this seat cannot see.
    pub fn unknown(&self) -> u32 {
        self.public().dice_in_play() - self.public().count(self.seat)
    }

    pub fn legal_raises(&self) -> Vec<Claim> {
        self.public().legal_raises()
    }

    pub fn previous_claim(&self) -> Option<Claim> {
        self.public().previous_claim()
    }

    pub fn variants(&self) -> Variants {
        self.public().variants()
    }

    /// Whether it is this seat's turn.
    pub fn is_my_turn(&self) -> bool {
        self.public().current() == self.seat
    }
}

/// How many of these dice count toward a face. With ones wild, a one counts toward every
/// face but the one face.
pub fn count_face(dice: &[u8], face: u8, ones_wild: bool) -> u32 {
    dice.iter()
        .filter(|&&value| value == face || (ones_wild && value == 1 && face != 1))
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dice as a test says they are. Every seat gets the same faces unless told otherwise,
    /// which keeps a test about resolution from also being about dealing.
    fn deal(game: &mut Game, faces: &[u8]) {
        let mut dealt = [0u8; SEAT];
        for (at, face) in dealt.iter_mut().enumerate() {
            *face = faces[at % faces.len()];
        }
        for seat in 0..game.public().players() {
            let _ = game.install(seat, dealt);
        }
    }

    /// A game with every seat dealt the same face, ready to play.
    fn game_of(players: usize, face: u8) -> Game {
        let mut game = Game::start((0..players).collect(), Variants::default());
        deal(&mut game, &[face]);
        game
    }

    /// Put a game into a given position. These tests are about resolutions, not about the
    /// twenty turns it would take to play a player down to their last die.
    fn holding(game: &mut Game, counts: &[u32], shields: &[u32]) {
        assert_eq!(counts.len(), game.public.counts.len());
        game.public.counts = counts.to_vec();
        game.public.shields = shields.to_vec();
    }

    /// Close the round with a call and settle it in one step, as a complete game can.
    fn settle(game: &mut Game, call: Call) -> Resolution {
        assert_eq!(game.claim(call), Ok(Claimed::RoundEnded));
        game.resolve().expect("a complete game settles at once")
    }

    #[test]
    fn counting_without_wild_ones_counts_only_the_face() {
        assert_eq!(count_face(&[1, 1, 5, 5, 5], 5, false), 3);
        assert_eq!(count_face(&[1, 1, 5, 5, 5], 1, false), 2);
    }

    #[test]
    fn counting_with_wild_ones_counts_ones_too() {
        assert_eq!(count_face(&[1, 1, 5, 5, 5], 5, true), 5);
        assert_eq!(count_face(&[1, 1, 5, 5, 5], 1, true), 2);
    }

    #[test]
    fn an_opening_claim_may_be_anything_possible() {
        let game = game_of(2, 3);
        let public = game.public();
        assert!(
            public
                .check_raise(Claim {
                    quantity: 1,
                    face: 1
                })
                .is_ok()
        );
        assert!(
            public
                .check_raise(Claim {
                    quantity: 10,
                    face: 6
                })
                .is_ok()
        );
        assert_eq!(
            public.check_raise(Claim {
                quantity: 11,
                face: 6
            }),
            Err(Rejected::QuantityExceedsDiceInPlay)
        );
    }

    #[test]
    fn liar_and_spot_on_need_a_previous_claim() {
        let game = game_of(2, 3);
        assert_eq!(
            game.public().check(Call::Liar),
            Err(Rejected::NoPreviousClaim)
        );
        assert_eq!(
            game.public().check(Call::SpotOn),
            Err(Rejected::NoPreviousClaim)
        );
    }

    #[test]
    fn a_raise_must_be_stronger() {
        let mut game = game_of(2, 3);
        game.claim(Call::Raise(Claim {
            quantity: 3,
            face: 4,
        }))
        .unwrap();
        let public = game.public();
        assert!(
            public
                .check_raise(Claim {
                    quantity: 3,
                    face: 5
                })
                .is_ok()
        );
        assert!(
            public
                .check_raise(Claim {
                    quantity: 4,
                    face: 1
                })
                .is_ok()
        );
        assert_eq!(
            public.check_raise(Claim {
                quantity: 3,
                face: 4
            }),
            Err(Rejected::NotStrongerThanPreviousClaim)
        );
        assert_eq!(
            public.check_raise(Claim {
                quantity: 2,
                face: 6
            }),
            Err(Rejected::NotStrongerThanPreviousClaim)
        );
    }

    #[test]
    fn the_one_face_is_unclaimable_when_ones_are_wild() {
        let mut game = Game::start(vec![0, 1], Variants { ones_wild: true });
        deal(&mut game, &[3]);
        assert_eq!(
            game.public().check_raise(Claim {
                quantity: 1,
                face: 1
            }),
            Err(Rejected::OnesNotClaimableWhenWild)
        );
        assert!(
            game.public()
                .legal_raises()
                .iter()
                .all(|claim| claim.face != 1)
        );
    }

    #[test]
    fn calling_liar_correctly_costs_the_previous_player_a_die() {
        let mut game = game_of(2, 2);
        holding(&mut game, &[2, 2], &[0, 0]);
        // Every die is a two, so there are no fives at all.
        game.claim(Call::Raise(Claim {
            quantity: 3,
            face: 5,
        }))
        .unwrap();
        let resolution = settle(&mut game, Call::Liar);
        assert_eq!(resolution.truth, 0);
        assert_eq!(resolution.lost, Some(0));
        // Player zero was affected, so player zero opens the next round.
        assert_eq!(resolution.opens, 0);
        assert_eq!(game.public().count(0), 1);
        assert_eq!(game.public().count(1), 2);
    }

    #[test]
    fn calling_liar_wrongly_costs_the_caller_a_die() {
        let mut game = game_of(2, 5);
        holding(&mut game, &[2, 2], &[0, 0]);
        // Every die is a five, so there are four of them.
        game.claim(Call::Raise(Claim {
            quantity: 3,
            face: 5,
        }))
        .unwrap();
        let resolution = settle(&mut game, Call::Liar);
        assert_eq!(game.public().count(0), 2);
        assert_eq!(game.public().count(1), 1);
        assert_eq!(resolution.opens, 1);
    }

    #[test]
    fn a_correct_spot_on_gains_a_die() {
        let mut game = game_of(2, 5);
        holding(&mut game, &[2, 2], &[0, 0]);
        game.claim(Call::Raise(Claim {
            quantity: 4,
            face: 5,
        }))
        .unwrap();
        let resolution = settle(&mut game, Call::SpotOn);
        assert_eq!(game.public().count(1), 3);
        assert_eq!(resolution.opens, 1);
    }

    #[test]
    fn a_player_with_no_dice_is_eliminated_and_the_match_ends() {
        let mut game = game_of(2, 2);
        holding(&mut game, &[1, 1], &[0, 0]);
        game.claim(Call::Raise(Claim {
            quantity: 2,
            face: 5,
        }))
        .unwrap();
        let resolution = settle(&mut game, Call::Liar);
        assert_eq!(resolution.eliminated, Some(0));
        assert!(resolution.over);
        assert!(game.public().over());
        assert!(game.public().eliminated(0));
        assert_eq!(game.public().ranking(), vec![1, 0]);
    }

    #[test]
    fn a_match_cannot_be_played_after_it_is_over() {
        let mut game = game_of(2, 2);
        holding(&mut game, &[1, 1], &[0, 0]);
        game.claim(Call::Raise(Claim {
            quantity: 2,
            face: 5,
        }))
        .unwrap();
        settle(&mut game, Call::Liar);
        assert_eq!(game.public().check(Call::Liar), Err(Rejected::MatchIsOver));
    }

    #[test]
    fn eliminated_players_are_skipped_in_turn_order() {
        let mut game = game_of(3, 2);
        holding(&mut game, &[1, 1, 1], &[0, 0, 0]);
        game.claim(Call::Raise(Claim {
            quantity: 3,
            face: 5,
        }))
        .unwrap();
        let resolution = settle(&mut game, Call::Liar);
        assert!(game.public().eliminated(0));
        assert_eq!(game.public().alive(), vec![1, 2]);
        assert_eq!(game.public().next_player(1), 2);
        assert_eq!(game.public().next_player(2), 1);
        // Player zero was affected but is eliminated, so the other player in the exchange
        // opens instead.
        assert_eq!(resolution.opens, 1);
    }

    #[test]
    fn the_history_records_who_claimed_and_how_many_dice_they_held() {
        let mut game = game_of(2, 3);
        let claim = Claim {
            quantity: 2,
            face: 3,
        };
        game.claim(Call::Raise(claim)).unwrap();
        assert_eq!(
            game.public().history(),
            &[Entry {
                player: 0,
                dice_held: 5,
                claim
            }]
        );
    }

    #[test]
    fn the_history_is_only_the_current_round() {
        let mut game = game_of(2, 2);
        game.claim(Call::Raise(Claim {
            quantity: 1,
            face: 5,
        }))
        .unwrap();
        assert_eq!(game.public().history().len(), 1);
        let resolution = settle(&mut game, Call::Liar);
        game.next_round(resolution.opens);
        assert!(game.public().history().is_empty());
    }

    // --- The dice limit and shield tokens -------------------------------------------

    #[test]
    fn a_correct_spot_on_at_the_dice_limit_gains_a_shield_token_instead_of_a_die() {
        let mut game = game_of(2, 5);
        // Ten dice, every one a five, so a claim of exactly ten fives is spot on.
        game.claim(Call::Raise(Claim {
            quantity: 10,
            face: 5,
        }))
        .unwrap();
        let resolution = settle(&mut game, Call::SpotOn);
        assert_eq!(resolution.gained, None, "no die is gained at the limit");
        assert_eq!(resolution.shield_gained, Some(1));
        assert_eq!(resolution.affected, 1);
        assert_eq!(resolution.opens, 1);
        assert_eq!(game.public().count(1), DICE_LIMIT, "still at the limit");
        assert_eq!(game.public().shields(1), 1);
    }

    #[test]
    fn a_shield_token_prevents_a_die_loss_and_is_spent_doing_so() {
        let mut game = game_of(2, 2);
        holding(&mut game, &[3, 3], &[1, 0]);
        game.claim(Call::Raise(Claim {
            quantity: 1,
            face: 5,
        }))
        .unwrap();
        let resolution = settle(&mut game, Call::Liar);
        assert_eq!(resolution.lost, None, "the die is not lost");
        assert_eq!(resolution.shield_spent, Some(0));
        assert_eq!(resolution.affected, 0);
        assert_eq!(resolution.eliminated, None);
        assert_eq!(game.public().count(0), 3, "the die is kept");
        assert_eq!(game.public().shields(0), 0, "the token is spent");
    }

    #[test]
    fn a_player_on_their_last_die_survives_a_loss_with_a_shield_token() {
        let mut game = game_of(2, 2);
        holding(&mut game, &[1, 3], &[1, 0]);
        game.claim(Call::Raise(Claim {
            quantity: 1,
            face: 5,
        }))
        .unwrap();
        settle(&mut game, Call::Liar);
        assert!(!game.public().eliminated(0), "the token kept them in");
        assert_eq!(game.public().count(0), 1);
        assert_eq!(game.public().shields(0), 0);
        assert!(!game.public().over());
    }

    #[test]
    fn without_a_token_the_same_loss_takes_the_die() {
        let mut game = game_of(2, 2);
        holding(&mut game, &[1, 3], &[0, 0]);
        game.claim(Call::Raise(Claim {
            quantity: 1,
            face: 5,
        }))
        .unwrap();
        settle(&mut game, Call::Liar);
        assert!(game.public().eliminated(0));
        assert!(game.public().over());
    }

    // --- What a seat may know --------------------------------------------------------

    /// The test that makes the virtual-player guarantee real: a view over a game that
    /// holds every die still tells a seat only its own.
    #[test]
    fn a_view_over_a_complete_game_hides_every_other_seat() {
        let mut game = Game::start((0..3).collect(), Variants::default());
        for seat in 0..3 {
            game.install(seat, [(seat as u8) + 1; SEAT]).unwrap();
        }
        assert!(game.dice_known(), "the game knows every die");

        let view = game.view(1);
        assert_eq!(view.own_dice(), Some(&[2u8; SEAT][..]));
        assert_eq!(view.dice_of(1), Some(&[2u8; SEAT][..]));
        assert_eq!(
            view.dice_of(0),
            None,
            "the game knows, the view does not tell"
        );
        assert_eq!(view.dice_of(2), None);
        assert_eq!(view.unknown(), 10);
    }

    #[test]
    fn a_resolution_reveals_every_seat_to_every_view() {
        let mut game = game_of(2, 4);
        assert_eq!(game.view(0).dice_of(1), None);
        game.claim(Call::Raise(Claim {
            quantity: 1,
            face: 5,
        }))
        .unwrap();
        settle(&mut game, Call::Liar);
        assert!(game.revealed());
        assert_eq!(
            game.view(0).dice_of(1),
            Some(&[4u8; SEAT][..]),
            "resolving a round reveals every player's dice"
        );
    }

    /// The delayed-dice property, which is what a client lives with every round.
    #[test]
    fn a_round_cannot_be_settled_until_every_seat_is_known() {
        let mut game = Game::start((0..3).collect(), Variants::default());
        game.install(1, [6; SEAT]).unwrap();
        game.claim(Call::Raise(Claim {
            quantity: 2,
            face: 6,
        }))
        .unwrap();
        assert_eq!(game.claim(Call::Liar), Ok(Claimed::RoundEnded));
        assert_eq!(game.truth(6), None, "truth needs every die");
        assert_eq!(game.resolve(), None, "and so does a resolution");

        game.install(0, [6; SEAT]).unwrap();
        assert_eq!(game.resolve(), None, "one seat still missing");
        game.install(2, [6; SEAT]).unwrap();
        assert!(
            game.resolve().is_some(),
            "the last seat makes the round settleable"
        );
    }

    /// Several things want to know how a round went — each client, a renderer, a log
    /// writer. None of them may move a die by asking.
    #[test]
    fn settling_a_round_twice_settles_it_once() {
        let mut game = game_of(2, 2);
        holding(&mut game, &[3, 3], &[0, 0]);
        game.claim(Call::Raise(Claim {
            quantity: 1,
            face: 5,
        }))
        .unwrap();
        assert_eq!(game.claim(Call::Liar), Ok(Claimed::RoundEnded));

        let first = game.resolve().expect("a complete game settles");
        let counts: Vec<u32> = (0..2).map(|seat| game.public().count(seat)).collect();
        for _ in 0..5 {
            assert_eq!(game.resolve(), Some(first), "the same answer every time");
        }
        let after: Vec<u32> = (0..2).map(|seat| game.public().count(seat)).collect();
        assert_eq!(counts, after, "asking again moved a die");
    }

    #[test]
    fn a_seat_cannot_be_dealt_twice_in_one_round() {
        let mut game = Game::start(vec![0, 1], Variants::default());
        assert_eq!(game.install(0, [1; SEAT]), Ok(()));
        assert_eq!(
            game.install(0, [6; SEAT]),
            Err(AlreadyInstalled),
            "a late or duplicated reveal must not rewrite a round"
        );
        assert_eq!(game.view(0).own_dice(), Some(&[1u8; SEAT][..]));
    }

    /// Lockstep over public state: the same claims produce the same `Public` whether or
    /// not the game knows every die.
    #[test]
    fn public_state_is_the_same_whether_the_dice_are_known_or_not() {
        let mut whole = game_of(3, 4);
        let mut partial = Game::start((0..3).collect(), Variants::default());
        partial.install(2, [4; SEAT]).unwrap();

        for quantity in 1..=4 {
            let call = Call::Raise(Claim { quantity, face: 4 });
            assert_eq!(whole.claim(call), partial.claim(call));
        }
        assert_eq!(
            whole.public(),
            partial.public(),
            "two clients knowing different dice still agree about everything public"
        );
        assert_eq!(
            whole.public().legal_raises(),
            partial.public().legal_raises()
        );
    }

    #[test]
    fn a_closed_round_takes_no_further_claims() {
        let mut game = game_of(2, 3);
        game.claim(Call::Raise(Claim {
            quantity: 1,
            face: 3,
        }))
        .unwrap();
        assert_eq!(game.claim(Call::Liar), Ok(Claimed::RoundEnded));
        assert_eq!(
            game.claim(Call::Raise(Claim {
                quantity: 2,
                face: 3
            })),
            Err(Rejected::RoundHasEnded)
        );
    }

    // --- Invariants ------------------------------------------------------------------

    /// Play a match out, taking the first legal raise and calling liar on every fifth
    /// turn, checking an invariant at every step.
    fn play_out(face: u8, players: usize, check: &mut impl FnMut(&Game, Option<Resolution>)) {
        let mut game = game_of(players, face);
        check(&game, None);
        let mut turn = 0;
        while !game.public().over() && turn < 2_000 {
            let call = if turn % 5 == 4 && game.public().previous_claim().is_some() {
                Call::Liar
            } else {
                match game.public().legal_raises().first() {
                    Some(&claim) => Call::Raise(claim),
                    None => Call::Liar,
                }
            };
            match game.claim(call).expect("a legal call") {
                Claimed::Continues => check(&game, None),
                Claimed::RoundEnded => {
                    let resolution = game.resolve().expect("a complete game settles");
                    check(&game, Some(resolution));
                    if !resolution.over {
                        game.next_round(resolution.opens);
                        deal(&mut game, &[face]);
                    }
                }
            }
            turn += 1;
        }
    }

    #[test]
    fn exactly_one_player_is_affected_by_every_resolution() {
        for face in [1u8, 2, 6] {
            for players in 2..=5 {
                play_out(face, players, &mut |game, resolution| {
                    if let Some(resolution) = resolution {
                        let set = [
                            resolution.lost,
                            resolution.gained,
                            resolution.shield_gained,
                            resolution.shield_spent,
                        ]
                        .iter()
                        .filter(|slot| slot.is_some())
                        .count();
                        assert_eq!(set, 1, "exactly one of the four cases holds");
                        assert!(resolution.affected < game.public().players());
                    }
                });
            }
        }
    }

    #[test]
    fn no_player_ever_holds_more_than_the_dice_limit() {
        for players in 2..=5 {
            play_out(5, players, &mut |game, _| {
                for player in 0..game.public().players() {
                    assert!(
                        game.public().count(player) <= DICE_LIMIT,
                        "a player holds more than the dice limit"
                    );
                }
            });
        }
    }

    #[test]
    fn a_player_is_never_shown_more_dice_than_they_hold() {
        for players in 2..=5 {
            play_out(3, players, &mut |game, _| {
                for player in 0..game.public().players() {
                    if let Some(dice) = game.view(player).own_dice() {
                        assert!(
                            dice.len() as u32 <= game.public().count(player),
                            "a seat's undealt dice leaked into play"
                        );
                    }
                }
            });
        }
    }

    /// `A player who leaves a match is eliminated`.
    #[test]
    fn a_player_who_concedes_holds_no_dice_and_takes_no_turn() {
        let mut game = game_of(3, 4);
        let first = game.public().current();
        game.concede(first);

        assert!(game.public().eliminated(first), "conceding is elimination");
        assert_eq!(game.public().count(first), 0);
        assert_ne!(
            game.public().current(),
            first,
            "the turn must move on, or the match halts at a seat that has gone"
        );
        assert!(
            !game.public().ranking().is_empty() && game.public().ranking().last() == Some(&first),
            "the first to go is last in the ranking: {:?}",
            game.public().ranking()
        );
    }

    /// The common case, and a no-op: somebody already out choosing to leave.
    #[test]
    fn conceding_twice_or_when_already_out_changes_nothing() {
        let mut game = game_of(3, 4);
        game.concede(2);
        let after = game.public().ranking();
        game.concede(2);
        assert_eq!(
            game.public().ranking(),
            after,
            "a second departure must not be a second elimination"
        );
        // And out of range is ignored rather than a panic: the seat comes off a wire.
        game.concede(99);
    }

    /// Two players, one leaves: the match is complete and the other has won it.
    #[test]
    fn a_match_of_two_is_over_when_one_concedes() {
        let mut game = game_of(2, 4);
        assert!(!game.public().over());
        game.concede(1);
        assert!(
            game.public().over(),
            "one player left means the match is complete"
        );
        assert_eq!(game.public().ranking(), vec![0, 1]);
    }

    #[test]
    fn a_true_call_is_always_available_to_the_current_player() {
        for players in 2..=5 {
            play_out(4, players, &mut |game, _| {
                if game.public().over() || game.round_ended() {
                    return;
                }
                let available = match game.public().previous_claim() {
                    // With no previous claim, some raise must be true.
                    None => game
                        .public()
                        .legal_raises()
                        .iter()
                        .any(|claim| game.truth(claim.face) >= Some(claim.quantity)),
                    // With one, liar or spot on is true whenever no raise is: the
                    // previous claim is short, exact, or beatable.
                    Some(previous) => {
                        let actual = game.truth(previous.face).expect("a complete game");
                        actual <= previous.quantity
                            || game
                                .public()
                                .legal_raises()
                                .iter()
                                .any(|claim| game.truth(claim.face) >= Some(claim.quantity))
                    }
                };
                assert!(available, "no true call available to the current player");
            });
        }
    }
}
