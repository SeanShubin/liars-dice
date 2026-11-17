//! Game rules. Pure and deterministic. No input, no output, no rendering.
//!
//! Realizes /specification/game-rules.md. Rolling is the only non-determinism and it
//! arrives through [`Rolls`], which is the single pluggable abstraction
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
        winner: usize,
        lost: Option<usize>,
        gained: Option<usize>,
        eliminated: Option<usize>,
        truth: u32,
        over: bool,
    },
}

/// The source of every roll. The only non-determinism in the model.
pub trait Rolls {
    /// A face value from one to six.
    fn face(&mut self) -> u8;
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

/// A match: who holds how many dice, what they rolled, and the round in progress.
#[derive(Clone, Debug)]
pub struct Match {
    counts: Vec<u32>,
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
    pub fn start(
        order: Vec<usize>,
        dice_each: u32,
        variants: Variants,
        rolls: &mut impl Rolls,
    ) -> Self {
        let players = order.len();
        let mut game = Self {
            counts: vec![dice_each; players],
            rolled: vec![Vec::new(); players],
            current: order[0],
            order,
            variants,
            history: Vec::new(),
            eliminated: Vec::new(),
        };
        game.roll(rolls);
        game
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

    pub fn count(&self, player: usize) -> u32 {
        self.counts[player]
    }

    pub fn dice_of(&self, player: usize) -> &[u8] {
        &self.rolled[player]
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
        self.rolled
            .iter()
            .map(|dice| count_face(dice, face, self.variants.ones_wild))
            .sum()
    }

    /// Apply a call. A raise passes the turn; liar and spot on end the round, resolve it,
    /// and roll again.
    pub fn apply(&mut self, call: Call, rolls: &mut impl Rolls) -> Result<Outcome, Rejected> {
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
                // The caller wins the exchange if and only if their claim is true, and
                // their claim is that the previous claim is false.
                let (winner, lost) = if claim_held {
                    (previous.player, self.current)
                } else {
                    (self.current, previous.player)
                };
                Ok(self.resolve(winner, Some(lost), None, truth, rolls))
            }
            Call::SpotOn => {
                let previous = *self.history.last().expect("checked");
                let truth = self.truth(previous.claim.face);
                let exact = truth == previous.claim.quantity;
                if exact {
                    let winner = self.current;
                    Ok(self.resolve(winner, None, Some(winner), truth, rolls))
                } else {
                    let winner = previous.player;
                    Ok(self.resolve(winner, Some(self.current), None, truth, rolls))
                }
            }
        }
    }

    fn resolve(
        &mut self,
        winner: usize,
        lost: Option<usize>,
        gained: Option<usize>,
        truth: u32,
        rolls: &mut impl Rolls,
    ) -> Outcome {
        if let Some(player) = gained {
            self.counts[player] += 1;
        }
        let mut eliminated = None;
        if let Some(player) = lost {
            self.counts[player] -= 1;
            if self.counts[player] == 0 {
                self.eliminated.push(player);
                eliminated = Some(player);
            }
        }

        self.history.clear();
        let over = self.over();
        if !over {
            // The winner of an exchange makes the first claim of the next round. A winner
            // never loses dice, so a winner is never the eliminated player.
            self.current = winner;
            self.roll(rolls);
        } else {
            self.rolled = vec![Vec::new(); self.counts.len()];
        }

        Outcome::Resolved {
            winner,
            lost,
            gained,
            eliminated,
            truth,
            over,
        }
    }

    /// All players roll their dice.
    fn roll(&mut self, rolls: &mut impl Rolls) {
        for player in 0..self.counts.len() {
            let held = self.counts[player];
            self.rolled[player] = (0..held).map(|_| rolls.face()).collect();
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

    impl Rolls for Scripted {
        fn face(&mut self) -> u8 {
            let face = self.faces[self.at % self.faces.len()];
            self.at += 1;
            face
        }
    }

    fn scripted(faces: &[u8]) -> Scripted {
        Scripted {
            faces: faces.to_vec(),
            at: 0,
        }
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
        let game = Match::start(vec![0, 1], 5, Variants::default(), &mut rolls);
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
        let game = Match::start(vec![0, 1], 5, Variants::default(), &mut rolls);
        assert_eq!(game.check(Call::Liar), Err(Rejected::NoPreviousClaim));
        assert_eq!(game.check(Call::SpotOn), Err(Rejected::NoPreviousClaim));
    }

    #[test]
    fn a_raise_must_be_stronger() {
        let mut rolls = scripted(&[3]);
        let mut game = Match::start(vec![0, 1], 5, Variants::default(), &mut rolls);
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
        let game = Match::start(vec![0, 1], 5, Variants { ones_wild: true }, &mut rolls);
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
        let mut game = Match::start(vec![0, 1, 2], 3, Variants::default(), &mut rolls);
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
                    actual < previous.quantity
                        || actual == previous.quantity
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
        let mut game = Match::start(vec![0, 1], 2, Variants::default(), &mut rolls);
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
                winner,
                lost,
                truth,
                ..
            } => {
                assert_eq!(truth, 0);
                assert_eq!(lost, Some(0));
                assert_eq!(winner, 1);
            }
            other => panic!("expected a resolution, got {other:?}"),
        }
        assert_eq!(game.count(0), 1);
        assert_eq!(game.count(1), 2);
        assert_eq!(game.current(), 1);
    }

    #[test]
    fn calling_liar_wrongly_costs_the_caller_a_die() {
        let mut rolls = scripted(&[5]);
        let mut game = Match::start(vec![0, 1], 2, Variants::default(), &mut rolls);
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
        assert_eq!(game.current(), 0);
    }

    #[test]
    fn a_correct_spot_on_gains_a_die() {
        let mut rolls = scripted(&[5]);
        let mut game = Match::start(vec![0, 1], 2, Variants::default(), &mut rolls);
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
        let mut game = Match::start(vec![0, 1], 1, Variants::default(), &mut rolls);
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
        let mut game = Match::start(vec![0, 1], 1, Variants::default(), &mut rolls);
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
        let mut game = Match::start(vec![0, 1, 2], 1, Variants::default(), &mut rolls);
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
    }

    #[test]
    fn the_history_records_who_claimed_and_how_many_dice_they_held() {
        let mut rolls = scripted(&[3]);
        let mut game = Match::start(vec![0, 1], 5, Variants::default(), &mut rolls);
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
        let mut game = Match::start(vec![0, 1], 3, Variants::default(), &mut rolls);
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
}
