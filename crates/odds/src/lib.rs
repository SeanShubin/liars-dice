//! The probability model. Pure, exact, and knows nothing of rules or turns.
//!
//! Every probability in the game is one question: given the dice I can see, how likely is
//! it that at least so many of the dice I cannot see show a particular face?
//!
//! Realizes /specification/game-minimal.md — a probability accounts only for how many dice
//! are unknown and how faces are distributed.
//!
//! Nothing here rounds and nothing here approximates. /specification/architecture.md says
//! that a value deciding the content of an event is computed exactly, and the claim an
//! opponent makes is such a value, so a probability is an integer count of outcomes over
//! an integer count of outcomes, unbounded on both sides. Every comparison the decision
//! rule needs is then an integer comparison that cannot disagree with the arithmetic that
//! produced it. Turning one of these into a number for a person to read belongs to
//! `crates/readout`, on the display side of that boundary; the dependency only runs that
//! way, so a decision cannot be made from a rounded value even by accident.

mod natural;

pub use natural::Natural;

use std::cmp::Ordering;

/// How likely one unseen die counts toward a face: how many of its faces do, out of how
/// many it has.
///
/// A pair of counts rather than a quotient, so the chance per die is exact for the same
/// reason everything else here is.
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

    fn misses(&self) -> u32 {
        self.faces - self.hits
    }
}

/// An exact probability: how many equally likely outcomes answer the question, over how
/// many outcomes there are.
///
/// Ordering is by cross multiplication, so two of these compare correctly whatever
/// denominators they were built with, and equality is defined by that same ordering rather
/// than by how the fraction happens to be written. Within a single decision every
/// probability carries the same denominator and the comparison costs only a digit walk.
#[derive(Clone, Debug)]
pub struct Odds {
    numerator: Natural,
    denominator: Natural,
}

impl Odds {
    /// A probability written as the fraction `numerator` over `denominator`.
    ///
    /// # Panics
    /// If `denominator` is zero.
    pub fn ratio(numerator: u64, denominator: u64) -> Self {
        assert!(denominator != 0, "a probability needs outcomes to count");
        Self {
            numerator: Natural::new(numerator),
            denominator: Natural::new(denominator),
        }
    }

    pub fn certain() -> Self {
        Self::ratio(1, 1)
    }

    pub fn impossible() -> Self {
        Self::ratio(0, 1)
    }

    pub fn numerator(&self) -> &Natural {
        &self.numerator
    }

    pub fn denominator(&self) -> &Natural {
        &self.denominator
    }

    /// How likely the question is answered the other way.
    pub fn complement(&self) -> Self {
        Self {
            numerator: self.denominator.sub(&self.numerator),
            denominator: self.denominator.clone(),
        }
    }
}

impl PartialEq for Odds {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Odds {}

impl Ord for Odds {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.denominator == other.denominator {
            self.numerator.cmp(&other.numerator)
        } else {
            self.numerator
                .mul(&other.denominator)
                .cmp(&other.numerator.mul(&self.denominator))
        }
    }
}

impl PartialOrd for Odds {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The chance that at least `needed` of `unknown` dice count toward the face.
///
/// `needed` is what the claim asks for beyond what the asker can already see, so zero or
/// less means the claim is already true.
pub fn at_least(unknown: u32, needed: i64, chance: Chance) -> Odds {
    let denominator = Natural::power(chance.faces, unknown);
    if needed <= 0 {
        return Odds {
            numerator: denominator.clone(),
            denominator,
        };
    }
    let needed = needed as u32;
    if needed > unknown {
        return Odds {
            numerator: Natural::zero(),
            denominator,
        };
    }
    let mut numerator = Natural::zero();
    for hits in needed..=unknown {
        numerator = numerator.add(&outcomes(unknown, hits, chance));
    }
    Odds {
        numerator,
        denominator,
    }
}

/// The chance that exactly `needed` of `unknown` dice count toward the face.
pub fn exactly(unknown: u32, needed: i64, chance: Chance) -> Odds {
    let denominator = Natural::power(chance.faces, unknown);
    if needed < 0 || needed > unknown as i64 {
        return Odds {
            numerator: Natural::zero(),
            denominator,
        };
    }
    Odds {
        numerator: outcomes(unknown, needed as u32, chance),
        denominator,
    }
}

/// How many of the `faces` to the power of `trials` equally likely outcomes have exactly
/// `hits` dice counting toward the face.
fn outcomes(trials: u32, hits: u32, chance: Chance) -> Natural {
    let mut value = binomial(trials, hits);
    for _ in 0..hits {
        value = value.times(chance.hits);
    }
    for _ in 0..(trials - hits) {
        value = value.times(chance.misses());
    }
    value
}

/// `trials` choose `hits`.
///
/// Built one row at a time rather than from factorials. Each step multiplies before it
/// divides, and the product of a binomial coefficient with the next row's numerator is
/// always divisible by that row's denominator, so every step lands on a whole number and
/// nothing is ever discarded.
fn binomial(trials: u32, hits: u32) -> Natural {
    let hits = hits.min(trials - hits);
    let mut value = Natural::one();
    for step in 0..hits {
        value = value.times(trials - step).divided_by(step + 1);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Chance = Chance { hits: 1, faces: 6 };
    const WILD: Chance = Chance { hits: 2, faces: 6 };

    #[test]
    fn a_claim_already_satisfied_is_certain() {
        assert_eq!(at_least(10, 0, PLAIN), Odds::certain());
        assert_eq!(at_least(10, -3, PLAIN), Odds::certain());
        assert_eq!(at_least(0, 0, PLAIN), Odds::certain());
    }

    #[test]
    fn a_claim_beyond_the_unknown_dice_is_impossible() {
        assert_eq!(at_least(3, 4, PLAIN), Odds::impossible());
        assert_eq!(at_least(0, 1, PLAIN), Odds::impossible());
        assert_eq!(exactly(3, 4, PLAIN), Odds::impossible());
        assert_eq!(exactly(3, -1, PLAIN), Odds::impossible());
    }

    #[test]
    fn one_die_matches_the_chance_per_die() {
        assert_eq!(at_least(1, 1, PLAIN), Odds::ratio(1, 6));
        assert_eq!(at_least(1, 1, WILD), Odds::ratio(1, 3));
    }

    #[test]
    fn a_fraction_compares_by_value_rather_than_by_how_it_is_written() {
        assert_eq!(Odds::ratio(1, 6), Odds::ratio(6, 36));
        assert_eq!(Odds::ratio(36, 36), Odds::certain());
        assert_eq!(Odds::ratio(0, 36), Odds::impossible());
        assert!(Odds::ratio(1, 3) > Odds::ratio(6, 36));
        assert!(Odds::ratio(1, 1_000_000_007) > Odds::impossible());
    }

    #[test]
    fn the_complement_is_the_other_way_round() {
        assert_eq!(Odds::certain().complement(), Odds::impossible());
        assert_eq!(Odds::impossible().complement(), Odds::certain());
        assert_eq!(Odds::ratio(1, 6).complement(), Odds::ratio(5, 6));
        let value = at_least(20, 4, WILD);
        assert_eq!(value.complement().complement(), value);
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
                let mut previous = Odds::certain();
                for needed in 0..=(unknown as i64 + 2) {
                    let value = at_least(unknown, needed, chance);
                    assert!(
                        value <= previous,
                        "unknown {unknown} needed {needed} rose above the step before it"
                    );
                    previous = value;
                }
            }
        }
    }

    #[test]
    fn every_outcome_accounts_for_all_the_probability() {
        for unknown in 0..40u32 {
            for chance in [PLAIN, WILD] {
                let mut total = Natural::zero();
                for hits in 0..=unknown {
                    let term = exactly(unknown, hits as i64, chance);
                    assert_eq!(term.denominator(), &Natural::power(chance.faces, unknown));
                    total = total.add(term.numerator());
                }
                assert_eq!(
                    total,
                    Natural::power(chance.faces, unknown),
                    "unknown {unknown} did not account for every outcome"
                );
            }
        }
    }

    /// The reason the model can afford to be exact everywhere: there is no size at which
    /// it stops working, so no state in any match has to fall back on anything.
    #[test]
    fn the_exact_path_has_no_ceiling() {
        for unknown in [49u32, 50, 100, 250] {
            let value = at_least(unknown, unknown as i64 / 2, PLAIN);
            assert!(value > Odds::impossible(), "{unknown} dice gave up");
            assert!(value < Odds::certain(), "{unknown} dice gave up");
            assert_eq!(value.denominator(), &Natural::power(6, unknown));
        }
    }

    fn natural_from(value: u128) -> Natural {
        Natural::new((value >> 64) as u64)
            .mul(&Natural::power(2, 64))
            .add(&Natural::new(value as u64))
    }

    /// Cross-checks the unbounded arithmetic against the machine integers, over the whole
    /// range where the machine integers still hold the answer. Six to the fiftieth is
    /// where they stop; the model carries on past it, which is what the test above covers.
    #[test]
    fn the_counts_agree_with_machine_integers_wherever_those_reach() {
        for unknown in 0..=49u32 {
            for chance in [PLAIN, WILD] {
                let denominator = (chance.faces as u128).pow(unknown);
                for hits in 0..=unknown {
                    let mut expected = 1u128;
                    for step in 0..hits.min(unknown - hits) {
                        expected = expected * (unknown - step) as u128 / (step + 1) as u128;
                    }
                    expected *= (chance.hits as u128).pow(hits);
                    expected *= (chance.misses() as u128).pow(unknown - hits);
                    let term = exactly(unknown, hits as i64, chance);
                    assert_eq!(
                        term.numerator(),
                        &natural_from(expected),
                        "unknown {unknown} hits {hits}"
                    );
                    assert_eq!(
                        term.denominator(),
                        &natural_from(denominator),
                        "unknown {unknown} hits {hits}"
                    );
                }
            }
        }
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
