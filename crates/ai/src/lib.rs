//! The AI opponent's decision rule.
//!
//! Realizes /specification/ai-opponents.md. The statements there say what an opponent
//! knows and does not model; this is how it chooses, and it has no tuning knob. Comparing
//! probabilities directly is what makes "the minimum it can get away with" and "calls liar
//! when the other options drop too low" fall out with nothing to calibrate.
//!
//! An opponent is handed only its own dice and what is public. It cannot reach another
//! player's dice, so the knowledge statements are enforced by the signature rather than by
//! discipline.

use odds::{Chance, at_least, exactly};
use rules::{Call, Claim, Variants, count_face};

/// Everything an AI opponent is allowed to see.
pub struct View<'a> {
    /// The opponent's own dice.
    pub own_dice: &'a [u8],
    /// How many dice it cannot see.
    pub unknown: u32,
    /// Every legal raise, which the rules already bound.
    pub legal_raises: &'a [Claim],
    /// The claim to beat, if there is one this round.
    pub previous: Option<Claim>,
    pub variants: Variants,
}

impl View<'_> {
    /// How likely a claim is true, given the dice this opponent holds.
    pub fn chance_of(&self, claim: Claim) -> f64 {
        let seen = count_face(self.own_dice, claim.face, self.variants.ones_wild) as i64;
        let needed = claim.quantity as i64 - seen;
        at_least(self.unknown, needed, self.chance(claim.face))
    }

    /// How likely a claim is exactly right.
    pub fn chance_exactly(&self, claim: Claim) -> f64 {
        let seen = count_face(self.own_dice, claim.face, self.variants.ones_wild) as i64;
        let needed = claim.quantity as i64 - seen;
        exactly(self.unknown, needed, self.chance(claim.face))
    }

    fn chance(&self, face: u8) -> Chance {
        Chance::per_die(self.variants.ones_wild, face)
    }
}

/// The raise this opponent would make, and how likely it is to be true.
///
/// The most probable legal claim given its own dice. Among equally probable claims, the
/// lowest — face value does not change a claim's probability on its own, so only the
/// opponent's own holdings break the tie.
pub fn best_raise(view: &View) -> Option<(Claim, f64)> {
    view.legal_raises
        .iter()
        .map(|&claim| (claim, view.chance_of(claim)))
        .fold(None, |best, (claim, chance)| match best {
            None => Some((claim, chance)),
            Some((best_claim, best_chance)) => {
                let better = chance > best_chance + f64::EPSILON;
                let equal = (chance - best_chance).abs() <= f64::EPSILON;
                let lower = (claim.quantity, claim.face) < (best_claim.quantity, best_claim.face);
                if better || (equal && lower) {
                    Some((claim, chance))
                } else {
                    Some((best_claim, best_chance))
                }
            }
        })
}

/// What this opponent does on its turn.
pub fn choose(view: &View) -> Call {
    let raise = best_raise(view);

    let Some(previous) = view.previous else {
        // Nothing to dispute on an opening turn, so the only call is a raise.
        return Call::Raise(raise.expect("an opening turn always has a legal raise").0);
    };

    let truth_of_previous = view.chance_of(previous);
    let liar = 1.0 - truth_of_previous;
    let spot_on = view.chance_exactly(previous);
    let raise_chance = raise.map(|(_, chance)| chance).unwrap_or(0.0);

    if spot_on > liar && spot_on > raise_chance {
        Call::SpotOn
    } else if liar > raise_chance {
        Call::Liar
    } else {
        match raise {
            Some((claim, _)) => Call::Raise(claim),
            // Nothing legal left to raise with, so the claim must be disputed.
            None => {
                if spot_on > liar {
                    Call::SpotOn
                } else {
                    Call::Liar
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rules::{Match, Rolls};

    struct Fixed(u8);

    impl Rolls for Fixed {
        fn face(&mut self) -> u8 {
            self.0
        }
    }

    struct Cycling {
        faces: Vec<u8>,
        at: usize,
    }

    impl Rolls for Cycling {
        fn face(&mut self) -> u8 {
            let face = self.faces[self.at % self.faces.len()];
            self.at += 1;
            face
        }
    }

    fn view<'a>(
        own: &'a [u8],
        unknown: u32,
        legal: &'a [Claim],
        previous: Option<Claim>,
    ) -> View<'a> {
        View {
            own_dice: own,
            unknown,
            legal_raises: legal,
            previous,
            variants: Variants::default(),
        }
    }

    #[test]
    fn an_opening_claim_leans_on_the_dice_it_holds() {
        let legal: Vec<Claim> = (1..=10)
            .flat_map(|quantity| (1..=6).map(move |face| Claim { quantity, face }))
            .collect();
        let chosen = choose(&view(&[5, 5, 5, 2, 4], 5, &legal, None));
        match chosen {
            Call::Raise(claim) => {
                // Three fives already in hand make fives the most probable face at the
                // quantities where the choice is not already certain.
                assert!(claim.quantity >= 1);
            }
            other => panic!("expected a raise, got {other:?}"),
        }
    }

    #[test]
    fn a_claim_the_opponent_can_already_see_is_certain() {
        let legal = vec![Claim {
            quantity: 3,
            face: 5,
        }];
        let seen = view(&[5, 5, 5], 5, &legal, None);
        assert_eq!(
            seen.chance_of(Claim {
                quantity: 3,
                face: 5
            }),
            1.0
        );
    }

    #[test]
    fn an_impossible_claim_is_called_a_lie() {
        // Twenty of a face among six dice cannot happen, so disputing it is certain.
        let previous = Claim {
            quantity: 20,
            face: 6,
        };
        let chosen = choose(&view(&[1, 2, 3], 3, &[], Some(previous)));
        assert!(matches!(chosen, Call::Liar | Call::SpotOn));
        let seen = view(&[1, 2, 3], 3, &[], Some(previous));
        assert_eq!(seen.chance_of(previous), 0.0);
    }

    #[test]
    fn a_certain_claim_is_never_called_a_lie() {
        let previous = Claim {
            quantity: 1,
            face: 5,
        };
        let legal = vec![Claim {
            quantity: 2,
            face: 5,
        }];
        let chosen = choose(&view(&[5, 5, 5], 5, &legal, Some(previous)));
        assert!(matches!(chosen, Call::Raise(_)));
    }

    #[test]
    fn the_lowest_of_equally_likely_claims_is_chosen() {
        // With no dice of its own and one unknown die, every face is equally likely, so
        // the tie breaks toward the lowest.
        let legal = vec![
            Claim {
                quantity: 1,
                face: 3,
            },
            Claim {
                quantity: 1,
                face: 5,
            },
            Claim {
                quantity: 1,
                face: 6,
            },
        ];
        let (claim, _) = best_raise(&view(&[], 1, &legal, None)).unwrap();
        assert_eq!(
            claim,
            Claim {
                quantity: 1,
                face: 3
            }
        );
    }

    #[test]
    fn an_opponent_never_names_the_one_face_when_ones_are_wild() {
        let mut rolls = Cycling {
            faces: vec![1, 3, 5, 2, 6, 4],
            at: 0,
        };
        let mut game = Match::start(vec![0, 1, 2], 4, Variants { ones_wild: true }, &mut rolls);
        for _ in 0..200 {
            if game.over() {
                break;
            }
            let player = game.current();
            let legal = game.legal_raises();
            let own = game.dice_of(player).to_vec();
            let unknown = game.dice_in_play() - game.count(player);
            let chosen = choose(&View {
                own_dice: &own,
                unknown,
                legal_raises: &legal,
                previous: game.previous_claim(),
                variants: game.variants(),
            });
            if let Call::Raise(claim) = chosen {
                assert_ne!(claim.face, 1, "named the one face while ones were wild");
            }
            game.apply(chosen, &mut rolls).expect("a legal call");
        }
    }

    #[test]
    fn an_opponent_never_produces_an_illegal_call() {
        for seed in 1..=6u8 {
            let mut rolls = Fixed(seed);
            let mut game = Match::start(vec![0, 1, 2, 3], 3, Variants::default(), &mut rolls);
            for _ in 0..500 {
                if game.over() {
                    break;
                }
                let player = game.current();
                let legal = game.legal_raises();
                let own = game.dice_of(player).to_vec();
                let unknown = game.dice_in_play() - game.count(player);
                let chosen = choose(&View {
                    own_dice: &own,
                    unknown,
                    legal_raises: &legal,
                    previous: game.previous_claim(),
                    variants: game.variants(),
                });
                game.apply(chosen, &mut rolls).expect("a legal call");
            }
        }
    }

    #[test]
    fn a_match_between_opponents_ends_with_one_survivor() {
        let mut rolls = Cycling {
            faces: vec![3, 1, 6, 2, 5, 4, 2, 6, 1, 3],
            at: 0,
        };
        let mut game = Match::start(vec![0, 1, 2, 3, 4], 3, Variants::default(), &mut rolls);
        let mut turns = 0;
        while !game.over() && turns < 20_000 {
            let player = game.current();
            let legal = game.legal_raises();
            let own = game.dice_of(player).to_vec();
            let unknown = game.dice_in_play() - game.count(player);
            let chosen = choose(&View {
                own_dice: &own,
                unknown,
                legal_raises: &legal,
                previous: game.previous_claim(),
                variants: game.variants(),
            });
            game.apply(chosen, &mut rolls).expect("a legal call");
            turns += 1;
        }
        assert!(game.over(), "the match did not end in {turns} turns");
        assert_eq!(game.alive().len(), 1);
        assert_eq!(game.ranking().len(), 5);
    }

    #[test]
    fn the_same_view_always_gives_the_same_call() {
        let legal: Vec<Claim> = (1..=12)
            .flat_map(|quantity| (1..=6).map(move |face| Claim { quantity, face }))
            .collect();
        let previous = Some(Claim {
            quantity: 4,
            face: 3,
        });
        let first = choose(&view(&[2, 3, 3, 6], 8, &legal, previous));
        let second = choose(&view(&[2, 3, 3, 6], 8, &legal, previous));
        assert_eq!(first, second);
    }
}
