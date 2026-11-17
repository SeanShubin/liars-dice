//! The probability model. Pure, and knows nothing of rules or turns.
//!
//! Every probability in the game is one question: given the dice I can see, how likely is
//! it that at least so many of the dice I cannot see show a particular face?
//!
//! Realizes /specification/game-minimal.md — a probability accounts only for how many dice
//! are unknown and how faces are distributed.
//!
//! Only addition, subtraction, multiplication and division are used. IEEE 754 specifies
//! those exactly, so the same inputs give bit-identical results on every platform. Nothing
//! here calls a transcendental function, and binomial coefficients are accumulated with
//! integer arithmetic rather than computed from factorials — see
//! /specification/architecture.md, which keeps both out of the model.

/// How likely one unseen die counts toward a face, as a fraction.
///
/// A fraction rather than a float so the exact oracle and the shipping path share one
/// definition of the chance rather than two that might drift.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chance {
    pub hits: u32,
    pub faces: u32,
}

impl Chance {
    /// One unseen die counts toward `face` if it shows that face, or if it shows a one and
    /// ones are wild. A claim cannot name the one face when ones are wild, so that case
    /// never reaches here.
    pub fn per_die(ones_wild: bool, face: u8) -> Self {
        let hits = if ones_wild && face != 1 { 2 } else { 1 };
        Self { hits, faces: 6 }
    }

    fn hit(&self) -> f64 {
        self.hits as f64 / self.faces as f64
    }

    fn miss(&self) -> f64 {
        (self.faces - self.hits) as f64 / self.faces as f64
    }
}

/// The chance that at least `needed` of `unknown` dice count toward the face.
///
/// `needed` is what the claim asks for beyond what the asker can already see, so zero or
/// less means the claim is already true.
pub fn at_least(unknown: u32, needed: i64, chance: Chance) -> f64 {
    if needed <= 0 {
        return 1.0;
    }
    let needed = needed as u32;
    if needed > unknown {
        return 0.0;
    }
    (needed..=unknown)
        .map(|hits| term(unknown, hits, chance))
        .sum()
}

/// The chance that exactly `needed` of `unknown` dice count toward the face.
pub fn exactly(unknown: u32, needed: i64, chance: Chance) -> f64 {
    if needed < 0 {
        return 0.0;
    }
    let needed = needed as u32;
    if needed > unknown {
        return 0.0;
    }
    term(unknown, needed, chance)
}

/// One term of the binomial: the chance of exactly `hits` out of `trials`.
///
/// The coefficient is accumulated by alternating multiplication and division so the
/// running value stays small, rather than building a factorial that would overflow.
fn term(trials: u32, hits: u32, chance: Chance) -> f64 {
    let hit = chance.hit();
    let miss = chance.miss();
    let mut value = 1.0f64;
    for step in 0..hits {
        value = value * (trials - step) as f64 / (step + 1) as f64;
    }
    for _ in 0..hits {
        value *= hit;
    }
    for _ in 0..(trials - hits) {
        value *= miss;
    }
    value
}

/// The same answer in exact integers, as a numerator over a denominator.
///
/// `None` when it would overflow, which happens at fifty unknown dice. Ten players holding
/// five dice each is fifty dice and the asker holds at least one, so this covers the
/// opening position of every match — enough to be the oracle the floating point path is
/// tested against.
pub fn at_least_exact(unknown: u32, needed: i64, chance: Chance) -> Option<(u128, u128)> {
    let denominator = checked_pow(chance.faces as u128, unknown)?;
    if needed <= 0 {
        return Some((denominator, denominator));
    }
    let needed = needed as u32;
    if needed > unknown {
        return Some((0, denominator));
    }
    let mut total: u128 = 0;
    for hits in needed..=unknown {
        total = total.checked_add(exact_term(unknown, hits, chance)?)?;
    }
    Some((total, denominator))
}

/// One term as an exact integer numerator over `faces` to the power of `trials`.
pub fn exact_term(trials: u32, hits: u32, chance: Chance) -> Option<u128> {
    let coefficient = binomial(trials, hits)?;
    let hit_part = checked_pow(chance.hits as u128, hits)?;
    let miss_part = checked_pow((chance.faces - chance.hits) as u128, trials - hits)?;
    coefficient.checked_mul(hit_part)?.checked_mul(miss_part)
}

fn binomial(trials: u32, hits: u32) -> Option<u128> {
    let hits = hits.min(trials - hits);
    let mut value: u128 = 1;
    for step in 0..hits {
        value = value.checked_mul((trials - step) as u128)? / (step + 1) as u128;
    }
    Some(value)
}

fn checked_pow(base: u128, exponent: u32) -> Option<u128> {
    let mut value: u128 = 1;
    for _ in 0..exponent {
        value = value.checked_mul(base)?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Chance = Chance { hits: 1, faces: 6 };
    const WILD: Chance = Chance { hits: 2, faces: 6 };

    #[test]
    fn a_claim_already_satisfied_is_certain() {
        assert_eq!(at_least(10, 0, PLAIN), 1.0);
        assert_eq!(at_least(10, -3, PLAIN), 1.0);
        assert_eq!(at_least(0, 0, PLAIN), 1.0);
    }

    #[test]
    fn a_claim_beyond_the_unknown_dice_is_impossible() {
        assert_eq!(at_least(3, 4, PLAIN), 0.0);
        assert_eq!(at_least(0, 1, PLAIN), 0.0);
    }

    #[test]
    fn one_die_matches_the_chance_per_die() {
        assert!((at_least(1, 1, PLAIN) - 1.0 / 6.0).abs() < 1e-12);
        assert!((at_least(1, 1, WILD) - 1.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn ones_wild_doubles_the_chance_for_other_faces() {
        assert_eq!(Chance::per_die(true, 5).hits, 2);
        assert_eq!(Chance::per_die(false, 5).hits, 1);
        assert_eq!(Chance::per_die(true, 1).hits, 1);
    }

    #[test]
    fn a_probability_never_rises_as_the_quantity_rises() {
        for unknown in 0..30u32 {
            for chance in [PLAIN, WILD] {
                let mut previous = 1.0;
                for needed in 0..=(unknown as i64 + 2) {
                    let value = at_least(unknown, needed, chance);
                    assert!(
                        value <= previous + 1e-12,
                        "unknown {unknown} needed {needed} rose from {previous} to {value}"
                    );
                    previous = value;
                }
            }
        }
    }

    #[test]
    fn every_outcome_accounts_for_all_the_probability() {
        for unknown in 0..25u32 {
            for chance in [PLAIN, WILD] {
                let total: f64 = (0..=unknown)
                    .map(|hits| exactly(unknown, hits as i64, chance))
                    .sum();
                assert!(
                    (total - 1.0).abs() < 1e-9,
                    "unknown {unknown} summed to {total}"
                );
            }
        }
    }

    /// The reason the floating point path is allowed to ship: it agrees with exact integer
    /// arithmetic everywhere the exact path can reach.
    #[test]
    fn floating_point_agrees_with_exact_integers() {
        for unknown in 0..=49u32 {
            for chance in [PLAIN, WILD] {
                for needed in 0..=unknown as i64 {
                    let (numerator, denominator) =
                        at_least_exact(unknown, needed, chance).expect("fits");
                    let exact = numerator as f64 / denominator as f64;
                    let approximate = at_least(unknown, needed, chance);
                    assert!(
                        (exact - approximate).abs() < 1e-9,
                        "unknown {unknown} needed {needed}: exact {exact} against {approximate}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_exact_path_reaches_the_opening_position_and_no_further() {
        assert!(at_least_exact(49, 10, PLAIN).is_some());
        assert!(at_least_exact(50, 10, PLAIN).is_none());
    }

    #[test]
    fn the_same_question_always_gives_the_same_answer() {
        for unknown in 0..40u32 {
            for needed in 0..=unknown as i64 {
                assert_eq!(
                    at_least(unknown, needed, PLAIN),
                    at_least(unknown, needed, PLAIN)
                );
            }
        }
    }
}
