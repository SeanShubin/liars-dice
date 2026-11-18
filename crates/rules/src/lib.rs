//! Game rules. Pure and deterministic. No input, no output, no rendering.
//!
//! Realizes /specification/game-rules.md. Rolling is the only non-determinism and it
//! arrives through [`Deals`], which is the single pluggable abstraction
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

/// One claim of the current round, as the history shows it: who claimed, how many dice
/// they held, and what they claimed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub player: usize,
    pub dice_held: u32,
    pub claim: Claim,
}

/// What happened when a call was applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// The round goes on and the turn has passed.
    Continues,
    /// The round ended. `truth` is how many dice actually counted toward the disputed
    /// claim.
    Resolved {
        /// Who makes the first claim of the next round.
        opens: usize,
        /// The one player the resolution affected: they lost a die, gained a die, gained
        /// a shield token, or spent one. Reported rather than left to be re-derived,
        /// because a spent token is not recoverable from the other fields.
        affected: usize,
        lost: Option<usize>,
        gained: Option<usize>,
        shield_gained: Option<usize>,
        shield_spent: Option<usize>,
        eliminated: Option<usize>,
        truth: u32,
        over: bool,
    },
}

/// The source of every die. The only non-determinism in the model.
///
/// A whole seat at a time, and told which round and which seat it is dealing. A source
/// that hands out one face at a time would have to produce them in whatever order `roll`
/// happens to ask — a coupling nothing states and a refactor could break in silence. The
/// integrity scheme keys its derivation on the round and the seat, so those are the two
/// things it is given.
pub trait Deals {
    /// Every face dealt to one seat: the dice limit, whatever that seat's player holds.
    fn seat(&mut self, round: u32, seat: usize) -> [u8; DICE_LIMIT as usize];
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
}

/// Faces are numbered one through six.
pub const FACES: std::ops::RangeInclusive<u8> = 1..=6;

/// Each player starts with five dice and holds at most five. The same five, so it is one
/// constant rather than a parameter a match could be started above.
pub const DICE_LIMIT: u32 = 5;

/// A match: who holds how many dice, what they rolled, and the round in progress.
#[derive(Clone, Debug)]
pub struct Match {
    counts: Vec<u32>,
    shields: Vec<u32>,
    round: u32,
    rolled: Vec<Vec<u8>>,
    order: Vec<usize>,
    variants: Variants,
    current: usize,
    history: Vec<Entry>,
    eliminated: Vec<usize>,
}

impl Match {
    /// `order` is the turn order, which is chosen at random when the match starts and not
    /// reordered afterwards. Players are identified by their index into it.
    pub fn start(order: Vec<usize>, variants: Variants, deals: &mut impl Deals) -> Self {
        let players = order.len();
        let mut game = Self {
            counts: vec![DICE_LIMIT; players],
            shields: vec![0; players],
            round: 0,
            rolled: vec![Vec::new(); players],
            current: order[0],
            order,
            variants,
            history: Vec::new(),
            eliminated: Vec::new(),
        };
        game.roll(deals);
        game
    }

    /// Which round the match is in, counting from zero. The dealer is keyed on this,
    /// so it is part of the model rather than a detail of whoever is logging.
    pub fn round(&self) -> u32 {
        self.round
    }

    pub fn variants(&self) -> Variants {
        self.variants
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn history(&self) -> &[Entry] {
        &self.history
    }

    /// Turn order, including players who have been eliminated. Fixed at match start, since
    /// players are not reordered during a match.
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    pub fn count(&self, player: usize) -> u32 {
        self.counts[player]
    }

    /// The dice this player holds: the prefix of what their seat was dealt. Every seat is
    /// dealt the dice limit so that a relay can deal without knowing the rules, so the
    /// rest of the seat's dice are not in play and must never be shown or counted.
    pub fn dice_of(&self, player: usize) -> &[u8] {
        let held = self.counts[player] as usize;
        // After the match ends nothing is dealt, so a survivor holds dice that were not
        // rolled. Take whichever is shorter rather than panicking on the slice.
        let dealt = self.rolled[player].len();
        &self.rolled[player][..held.min(dealt)]
    }

    /// Every face dealt to this player's seat, in play or not.
    ///
    /// For writing the event log and for verifying it later, where what was dealt is the
    /// thing being attested. Never for display and never for counting: use `dice_of`,
    /// which returns only what the player holds.
    pub fn dealt_of(&self, player: usize) -> &[u8] {
        &self.rolled[player]
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
            .filter(|&p| !self.eliminated(p))
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
        let at = alive.iter().position(|&p| p == from).unwrap_or(0);
        alive[(at + 1) % alive.len()]
    }

    /// Whether a raise is legal. A claim must be possible according to the number of dice
    /// in play, must be stronger than the previous claim, and may not name the one face
    /// when ones are wild.
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

    /// How many dice across the whole table count toward a face.
    pub fn truth(&self, face: u8) -> u32 {
        (0..self.counts.len())
            .map(|player| count_face(self.dice_of(player), face, self.variants.ones_wild))
            .sum()
    }

    /// Apply a call. A raise passes the turn; liar and spot on end the round, resolve it,
    /// and roll again.
    pub fn apply(&mut self, call: Call, deals: &mut impl Deals) -> Result<Outcome, Rejected> {
        self.check(call)?;
        match call {
            Call::Raise(claim) => {
                self.history.push(Entry {
                    player: self.current,
                    dice_held: self.counts[self.current],
                    claim,
                });
                self.current = self.next_player(self.current);
                Ok(Outcome::Continues)
            }
            Call::Liar => {
                let previous = *self.history.last().expect("checked");
                let truth = self.truth(previous.claim.face);
                let claim_held = truth >= previous.claim.quantity;
                // Whoever was wrong loses a die: the caller if the claim held, otherwise
                // the player who made it.
                let lost = if claim_held {
                    self.current
                } else {
                    previous.player
                };
                let pair = (self.current, previous.player);
                Ok(self.resolve(pair, Some(lost), None, truth, deals))
            }
            Call::SpotOn => {
                let previous = *self.history.last().expect("checked");
                let truth = self.truth(previous.claim.face);
                let exact = truth == previous.claim.quantity;
                let pair = (self.current, previous.player);
                if exact {
                    Ok(self.resolve(pair, None, Some(self.current), truth, deals))
                } else {
                    Ok(self.resolve(pair, Some(self.current), None, truth, deals))
                }
            }
        }
    }

    /// `pair` is the two players in the exchange: the one who ended the round and the
    /// previous player.
    fn resolve(
        &mut self,
        pair: (usize, usize),
        mut lost: Option<usize>,
        mut gained: Option<usize>,
        truth: u32,
        deals: &mut impl Deals,
    ) -> Outcome {
        let mut shield_gained = None;
        let mut shield_spent = None;

        // A correct claim below the dice limit gains a die; at the limit it gains a
        // shield token instead, which is what holds the limit without a check elsewhere.
        if let Some(player) = gained {
            if self.counts[player] < DICE_LIMIT {
                self.counts[player] += 1;
            } else {
                self.shields[player] += 1;
                shield_gained = Some(player);
                gained = None;
            }
        }

        // A shield token prevents its holder's next die loss, and is spent doing so.
        let mut eliminated = None;
        if let Some(player) = lost {
            if self.shields[player] > 0 {
                self.shields[player] -= 1;
                shield_spent = Some(player);
                lost = None;
            } else {
                self.counts[player] -= 1;
                if self.counts[player] == 0 {
                    self.eliminated.push(player);
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
        let opens = if self.counts[affected] == 0 {
            if affected == pair.0 { pair.1 } else { pair.0 }
        } else {
            affected
        };

        self.history.clear();
        let over = self.over();
        if !over {
            self.current = opens;
            self.round += 1;
            self.roll(deals);
        } else {
            self.rolled = vec![Vec::new(); self.counts.len()];
        }

        Outcome::Resolved {
            opens,
            affected,
            lost,
            gained,
            shield_gained,
            shield_spent,
            eliminated,
            truth,
            over,
        }
    }

    /// Deal the dice limit to every seat, regardless of how many dice its player holds.
    /// A relay can do this without knowing the rules, which it would otherwise have to
    /// replay every resolution to learn. A player uses the prefix; see `dice_of`.
    fn roll(&mut self, deals: &mut impl Deals) {
        for player in 0..self.counts.len() {
            self.rolled[player] = deals.seat(self.round, player).to_vec();
        }
    }
}

/// How many of these dice count toward a face. With ones wild, a one counts toward every
/// face but the one face.
pub fn count_face(dice: &[u8], face: u8, ones_wild: bool) -> u32 {
    dice.iter()
        .filter(|&&die| die == face || (ones_wild && face != 1 && die == 1))
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scripted source of rolls, so a test says what the dice are.
    struct Scripted {
        faces: Vec<u8>,
        at: usize,
    }

    impl Deals for Scripted {
        fn seat(&mut self, _round: u32, _seat: usize) -> [u8; DICE_LIMIT as usize] {
            let mut dealt = [0u8; DICE_LIMIT as usize];
            for face in dealt.iter_mut() {
                *face = self.faces[self.at % self.faces.len()];
                self.at += 1;
            }
            dealt
        }
    }

    fn scripted(faces: &[u8]) -> Scripted {
        Scripted {
            faces: faces.to_vec(),
            at: 0,
        }
    }

    /// Put a match into a given position and deal it. These tests are about resolutions,
    /// not about the twenty turns it would take to play a player down to their last die.
    /// A child module may reach the parent's private fields, so no production seam is
    /// opened to make this possible.
    fn holding(game: &mut Match, counts: &[u32], shields: &[u32], deals: &mut impl Deals) {
        assert_eq!(counts.len(), game.counts.len());
        assert_eq!(shields.len(), game.counts.len());
        game.counts = counts.to_vec();
        game.shields = shields.to_vec();
        game.roll(deals);
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
        let mut rolls = scripted(&[3]);
        let game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        assert!(
            game.check_raise(Claim {
                quantity: 1,
                face: 1
            })
            .is_ok()
        );
        assert!(
            game.check_raise(Claim {
                quantity: 10,
                face: 6
            })
            .is_ok()
        );
        assert_eq!(
            game.check_raise(Claim {
                quantity: 11,
                face: 6
            }),
            Err(Rejected::QuantityExceedsDiceInPlay)
        );
    }

    #[test]
    fn liar_and_spot_on_need_a_previous_claim() {
        let mut rolls = scripted(&[3]);
        let game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        assert_eq!(game.check(Call::Liar), Err(Rejected::NoPreviousClaim));
        assert_eq!(game.check(Call::SpotOn), Err(Rejected::NoPreviousClaim));
    }

    #[test]
    fn a_raise_must_be_stronger() {
        let mut rolls = scripted(&[3]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        game.apply(
            Call::Raise(Claim {
                quantity: 3,
                face: 4,
            }),
            &mut rolls,
        )
        .unwrap();
        assert!(
            game.check_raise(Claim {
                quantity: 3,
                face: 5
            })
            .is_ok()
        );
        assert!(
            game.check_raise(Claim {
                quantity: 4,
                face: 1
            })
            .is_ok()
        );
        assert_eq!(
            game.check_raise(Claim {
                quantity: 3,
                face: 4
            }),
            Err(Rejected::NotStrongerThanPreviousClaim)
        );
        assert_eq!(
            game.check_raise(Claim {
                quantity: 2,
                face: 6
            }),
            Err(Rejected::NotStrongerThanPreviousClaim)
        );
    }

    #[test]
    fn the_one_face_is_unclaimable_when_ones_are_wild() {
        let mut rolls = scripted(&[3]);
        let game = Match::start(vec![0, 1], Variants { ones_wild: true }, &mut rolls);
        assert_eq!(
            game.check_raise(Claim {
                quantity: 1,
                face: 1
            }),
            Err(Rejected::OnesNotClaimableWhenWild)
        );
        assert!(game.legal_raises().iter().all(|claim| claim.face != 1));
    }

    #[test]
    fn a_true_claim_is_always_available() {
        // Every legal position reachable in a small match leaves at least one true call.
        let mut rolls = scripted(&[2, 5, 3, 6, 1, 4]);
        let mut game = Match::start(vec![0, 1, 2], Variants::default(), &mut rolls);
        for _ in 0..40 {
            if game.over() {
                break;
            }
            let truth_of = |face: u8| game.truth(face);
            let any_true = match game.previous_claim() {
                None => game
                    .legal_raises()
                    .iter()
                    .any(|claim| truth_of(claim.face) >= claim.quantity),
                Some(previous) => {
                    let actual = truth_of(previous.face);
                    actual <= previous.quantity
                        || game
                            .legal_raises()
                            .iter()
                            .any(|claim| truth_of(claim.face) >= claim.quantity)
                }
            };
            assert!(any_true, "no true call available");
            let claim = game.legal_raises()[0];
            game.apply(Call::Raise(claim), &mut rolls).unwrap();
        }
    }

    #[test]
    fn calling_liar_correctly_costs_the_previous_player_a_die() {
        let mut rolls = scripted(&[2]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        holding(&mut game, &[2, 2], &[0, 0], &mut rolls);
        // Every die is a two, so there are no fives at all.
        game.apply(
            Call::Raise(Claim {
                quantity: 3,
                face: 5,
            }),
            &mut rolls,
        )
        .unwrap();
        let outcome = game.apply(Call::Liar, &mut rolls).unwrap();
        match outcome {
            Outcome::Resolved {
                opens, lost, truth, ..
            } => {
                assert_eq!(truth, 0);
                assert_eq!(lost, Some(0));
                // Player zero's dice count changed, so player zero opens the next round.
                assert_eq!(opens, 0);
            }
            other => panic!("expected a resolution, got {other:?}"),
        }
        assert_eq!(game.count(0), 1);
        assert_eq!(game.count(1), 2);
        assert_eq!(game.current(), 0);
    }

    #[test]
    fn calling_liar_wrongly_costs_the_caller_a_die() {
        let mut rolls = scripted(&[5]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        holding(&mut game, &[2, 2], &[0, 0], &mut rolls);
        // Every die is a five, so there are four of them.
        game.apply(
            Call::Raise(Claim {
                quantity: 3,
                face: 5,
            }),
            &mut rolls,
        )
        .unwrap();
        game.apply(Call::Liar, &mut rolls).unwrap();
        assert_eq!(game.count(0), 2);
        assert_eq!(game.count(1), 1);
        // Player one's dice count changed, so player one opens the next round.
        assert_eq!(game.current(), 1);
    }

    #[test]
    fn a_correct_spot_on_gains_a_die() {
        let mut rolls = scripted(&[5]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        holding(&mut game, &[2, 2], &[0, 0], &mut rolls);
        game.apply(
            Call::Raise(Claim {
                quantity: 4,
                face: 5,
            }),
            &mut rolls,
        )
        .unwrap();
        game.apply(Call::SpotOn, &mut rolls).unwrap();
        assert_eq!(game.count(1), 3);
        assert_eq!(game.current(), 1);
    }

    #[test]
    fn a_player_with_no_dice_is_eliminated_and_the_match_ends() {
        let mut rolls = scripted(&[2]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        holding(&mut game, &[1, 1], &[0, 0], &mut rolls);
        game.apply(
            Call::Raise(Claim {
                quantity: 2,
                face: 5,
            }),
            &mut rolls,
        )
        .unwrap();
        let outcome = game.apply(Call::Liar, &mut rolls).unwrap();
        assert!(matches!(
            outcome,
            Outcome::Resolved {
                eliminated: Some(0),
                over: true,
                ..
            }
        ));
        assert!(game.over());
        assert!(game.eliminated(0));
        assert_eq!(game.ranking(), vec![1, 0]);
    }

    #[test]
    fn a_match_cannot_be_played_after_it_is_over() {
        let mut rolls = scripted(&[2]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        holding(&mut game, &[1, 1], &[0, 0], &mut rolls);
        game.apply(
            Call::Raise(Claim {
                quantity: 2,
                face: 5,
            }),
            &mut rolls,
        )
        .unwrap();
        game.apply(Call::Liar, &mut rolls).unwrap();
        assert_eq!(game.check(Call::Liar), Err(Rejected::MatchIsOver));
    }

    #[test]
    fn eliminated_players_are_skipped_in_turn_order() {
        let mut rolls = scripted(&[2]);
        let mut game = Match::start(vec![0, 1, 2], Variants::default(), &mut rolls);
        holding(&mut game, &[1, 1, 1], &[0, 0, 0], &mut rolls);
        game.apply(
            Call::Raise(Claim {
                quantity: 3,
                face: 5,
            }),
            &mut rolls,
        )
        .unwrap();
        game.apply(Call::Liar, &mut rolls).unwrap();
        assert!(game.eliminated(0));
        assert_eq!(game.alive(), vec![1, 2]);
        assert_eq!(game.next_player(1), 2);
        assert_eq!(game.next_player(2), 1);
        // Player zero's dice changed but zero is eliminated, so the other player in the
        // exchange opens instead.
        assert_eq!(game.current(), 1);
    }

    #[test]
    fn the_history_records_who_claimed_and_how_many_dice_they_held() {
        let mut rolls = scripted(&[3]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        let claim = Claim {
            quantity: 2,
            face: 3,
        };
        game.apply(Call::Raise(claim), &mut rolls).unwrap();
        assert_eq!(
            game.history(),
            &[Entry {
                player: 0,
                dice_held: 5,
                claim
            }]
        );
    }

    #[test]
    fn the_history_is_only_the_current_round() {
        let mut rolls = scripted(&[2]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        game.apply(
            Call::Raise(Claim {
                quantity: 1,
                face: 5,
            }),
            &mut rolls,
        )
        .unwrap();
        assert_eq!(game.history().len(), 1);
        game.apply(Call::Liar, &mut rolls).unwrap();
        assert!(game.history().is_empty());
    }

    // --- The dice limit and shield tokens -------------------------------------------

    /// Drive a round to a resolution in which the raiser is punished: the current player
    /// raises something false, the next player calls liar, and the raiser is affected.
    fn false_claim_punished(game: &mut Match, deals: &mut impl Deals) -> Outcome {
        // Every die is a two in these tests, so no fives exist and any claim of fives is
        // false. One five is always within the dice in play.
        game.apply(
            Call::Raise(Claim {
                quantity: 1,
                face: 5,
            }),
            deals,
        )
        .expect("a legal raise");
        game.apply(Call::Liar, deals).expect("a legal call")
    }

    #[test]
    fn a_correct_spot_on_at_the_dice_limit_gains_a_shield_token_instead_of_a_die() {
        let mut rolls = scripted(&[5]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        holding(&mut game, &[DICE_LIMIT, DICE_LIMIT], &[0, 0], &mut rolls);
        // Ten dice, every one a five, so a claim of exactly ten fives is spot on.
        game.apply(
            Call::Raise(Claim {
                quantity: 10,
                face: 5,
            }),
            &mut rolls,
        )
        .unwrap();
        let outcome = game.apply(Call::SpotOn, &mut rolls).unwrap();
        match outcome {
            Outcome::Resolved {
                affected,
                gained,
                shield_gained,
                opens,
                ..
            } => {
                assert_eq!(gained, None, "no die is gained at the limit");
                assert_eq!(shield_gained, Some(1));
                assert_eq!(affected, 1);
                assert_eq!(opens, 1, "the affected player opens the next round");
            }
            other => panic!("expected a resolution, got {other:?}"),
        }
        assert_eq!(game.count(1), DICE_LIMIT, "still at the limit");
        assert_eq!(game.shields(1), 1);
    }

    #[test]
    fn a_shield_token_prevents_a_die_loss_and_is_spent_doing_so() {
        let mut rolls = scripted(&[2]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        holding(&mut game, &[3, 3], &[1, 0], &mut rolls);
        let outcome = false_claim_punished(&mut game, &mut rolls);
        match outcome {
            Outcome::Resolved {
                affected,
                lost,
                shield_spent,
                eliminated,
                ..
            } => {
                assert_eq!(lost, None, "the die is not lost");
                assert_eq!(shield_spent, Some(0));
                assert_eq!(affected, 0);
                assert_eq!(eliminated, None);
            }
            other => panic!("expected a resolution, got {other:?}"),
        }
        assert_eq!(game.count(0), 3, "the die is kept");
        assert_eq!(game.shields(0), 0, "the token is spent");
    }

    #[test]
    fn a_player_on_their_last_die_survives_a_loss_with_a_shield_token() {
        let mut rolls = scripted(&[2]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        holding(&mut game, &[1, 3], &[1, 0], &mut rolls);
        false_claim_punished(&mut game, &mut rolls);
        assert!(!game.eliminated(0), "the token kept them in the match");
        assert_eq!(game.count(0), 1);
        assert_eq!(game.shields(0), 0);
        assert!(!game.over());
    }

    #[test]
    fn without_a_token_the_same_loss_takes_the_die() {
        let mut rolls = scripted(&[2]);
        let mut game = Match::start(vec![0, 1], Variants::default(), &mut rolls);
        holding(&mut game, &[1, 3], &[0, 0], &mut rolls);
        false_claim_punished(&mut game, &mut rolls);
        assert!(game.eliminated(0));
        assert!(game.over());
    }

    // --- Invariants ------------------------------------------------------------------

    /// Play a match out, taking the first legal raise and calling liar on every fifth
    /// turn, checking an invariant at every step. Deterministic, so a failure is
    /// reproducible from the face sequence alone.
    fn play_out(faces: &[u8], players: usize, check: &mut impl FnMut(&Match, Option<Outcome>)) {
        let mut rolls = scripted(faces);
        let order: Vec<usize> = (0..players).collect();
        let mut game = Match::start(order, Variants::default(), &mut rolls);
        check(&game, None);
        let mut turn = 0;
        while !game.over() && turn < 2_000 {
            let call = if turn % 5 == 4 && game.previous_claim().is_some() {
                Call::Liar
            } else {
                match game.legal_raises().first() {
                    Some(&claim) => Call::Raise(claim),
                    None => Call::Liar,
                }
            };
            let outcome = game.apply(call, &mut rolls).expect("a legal call");
            check(&game, Some(outcome));
            turn += 1;
        }
    }

    #[test]
    fn exactly_one_player_is_affected_by_every_resolution() {
        for faces in [
            &[1u8, 2, 3, 4, 5, 6][..],
            &[2, 2, 5, 1, 6, 3, 4][..],
            &[6, 5, 4, 3, 2, 1][..],
        ] {
            for players in 2..=5 {
                play_out(faces, players, &mut |game, outcome| {
                    if let Some(Outcome::Resolved {
                        affected,
                        lost,
                        gained,
                        shield_gained,
                        shield_spent,
                        ..
                    }) = outcome
                    {
                        let set = [lost, gained, shield_gained, shield_spent]
                            .iter()
                            .filter(|slot| slot.is_some())
                            .count();
                        assert_eq!(set, 1, "exactly one of the four cases holds");
                        assert!(affected < game.order().len());
                    }
                });
            }
        }
    }

    #[test]
    fn no_player_ever_holds_more_than_the_dice_limit() {
        for players in 2..=5 {
            play_out(&[5, 5, 5, 5, 5, 5], players, &mut |game, _| {
                for player in game.order() {
                    assert!(
                        game.count(*player) <= DICE_LIMIT,
                        "a player holds more than the dice limit"
                    );
                }
            });
        }
    }

    #[test]
    fn a_player_is_never_shown_more_dice_than_they_hold() {
        for players in 2..=5 {
            play_out(&[3, 1, 4, 1, 5, 2], players, &mut |game, _| {
                for player in game.order() {
                    assert!(
                        game.dice_of(*player).len() as u32 <= game.count(*player),
                        "a seat's undealt dice leaked into play"
                    );
                }
            });
        }
    }

    #[test]
    fn a_true_call_is_always_available_to_the_current_player() {
        for players in 2..=5 {
            play_out(&[2, 4, 6, 1, 3, 5], players, &mut |game, _| {
                if game.over() {
                    return;
                }
                let available = match game.previous_claim() {
                    // With no previous claim, some raise must be true.
                    None => game
                        .legal_raises()
                        .iter()
                        .any(|claim| game.truth(claim.face) >= claim.quantity),
                    // With one, liar or spot on is true whenever no raise is: the
                    // previous claim is short, exact, or beatable.
                    Some(previous) => {
                        let actual = game.truth(previous.face);
                        actual <= previous.quantity
                            || game
                                .legal_raises()
                                .iter()
                                .any(|claim| game.truth(claim.face) >= claim.quantity)
                    }
                };
                assert!(available, "no true call available to the current player");
            });
        }
    }
}
