//! Turning an exact probability into a number a person can read.
//!
//! Realizes the display side of /specification/architecture.md. A value that decides the
//! content of an event is computed exactly; a value that only reaches a person may round.
//! For the probability model this crate is the whole of the second kind, and it is a
//! separate crate so that the direction of the dependency is what keeps the two apart.
//! `crates/odds` and `crates/ai` do not depend on this, so an opponent cannot decide from
//! a rounded number even by accident — that is a compile error rather than something to
//! remember.
//!
//! There is still one probability model. Nothing here recomputes anything. It reads the
//! exact value `crates/odds` produced and scales it, so the figure a person is shown is
//! the figure an opponent in their seat would have decided from, and there is no second
//! implementation to drift.

use odds::Odds;

/// An exact probability as a fraction between zero and one.
///
/// This is where the one rounding happens. A probability smaller than the smallest
/// representable fraction reads as zero, which is the honest display of it.
pub fn fraction(chance: &Odds) -> f64 {
    let (top, top_scale) = chance.numerator().significand();
    if top == 0 {
        return 0.0;
    }
    let (bottom, bottom_scale) = chance.denominator().significand();
    let scale = top_scale as i32 - bottom_scale as i32;
    (top as f64 / bottom as f64) * 2.0f64.powi(scale)
}

/// The same, out of a hundred.
pub fn percent(chance: &Odds) -> f64 {
    fraction(chance) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use odds::{Chance, at_least, exactly};

    const PLAIN: Chance = Chance { hits: 1, faces: 6 };
    const WILD: Chance = Chance { hits: 2, faces: 6 };

    #[test]
    fn the_ends_of_the_range_read_exactly() {
        assert_eq!(fraction(&Odds::certain()), 1.0);
        assert_eq!(fraction(&Odds::impossible()), 0.0);
        assert_eq!(percent(&Odds::certain()), 100.0);
        assert_eq!(percent(&Odds::impossible()), 0.0);
        assert_eq!(fraction(&at_least(30, 0, PLAIN)), 1.0);
        assert_eq!(fraction(&at_least(30, 31, PLAIN)), 0.0);
    }

    #[test]
    fn a_simple_fraction_reads_as_itself() {
        assert!((fraction(&Odds::ratio(1, 6)) - 1.0 / 6.0).abs() < 1e-15);
        assert!((fraction(&Odds::ratio(1, 3)) - 1.0 / 3.0).abs() < 1e-15);
        assert!((fraction(&Odds::ratio(1, 2)) - 0.5).abs() < 1e-15);
        assert!((percent(&Odds::ratio(1, 4)) - 25.0).abs() < 1e-13);
    }

    /// The readout agrees with the obvious floating point binomial wherever that one is
    /// still trustworthy. The exact value is what ships; this says the scaling down to
    /// something readable does not distort it.
    #[test]
    fn the_readout_agrees_with_a_direct_calculation() {
        for unknown in 0..=40u32 {
            for chance in [PLAIN, WILD] {
                let hit = chance.hits as f64 / chance.faces as f64;
                let miss = 1.0 - hit;
                for needed in 1..=unknown {
                    let mut direct = 0.0f64;
                    for hits in needed..=unknown {
                        let mut term = 1.0f64;
                        for step in 0..hits {
                            term = term * (unknown - step) as f64 / (step + 1) as f64;
                        }
                        term *= hit.powi(hits as i32) * miss.powi((unknown - hits) as i32);
                        direct += term;
                    }
                    let read = fraction(&at_least(unknown, needed as i64, chance));
                    assert!(
                        (read - direct).abs() < 1e-12,
                        "unknown {unknown} needed {needed}: read {read} against {direct}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_probability_too_small_to_write_reads_as_zero_rather_than_failing() {
        // Six hundred dice puts the answer far below the smallest fraction there is.
        let tiny = exactly(600, 600, PLAIN);
        let read = fraction(&tiny);
        assert!(read.is_finite(), "the readout produced no number");
        assert_eq!(read, 0.0);
    }

    #[test]
    fn the_readout_never_leaves_the_range_it_claims() {
        for unknown in 0..=60u32 {
            for needed in 0..=unknown as i64 + 1 {
                for chance in [PLAIN, WILD] {
                    let read = fraction(&at_least(unknown, needed, chance));
                    assert!(
                        (0.0..=1.0).contains(&read),
                        "unknown {unknown} needed {needed} read {read}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_readout_never_rises_as_the_quantity_rises() {
        for unknown in 0..=50u32 {
            for chance in [PLAIN, WILD] {
                let mut previous = 1.0;
                for needed in 0..=unknown as i64 {
                    let read = fraction(&at_least(unknown, needed, chance));
                    assert!(read <= previous, "unknown {unknown} needed {needed} rose");
                    previous = read;
                }
            }
        }
    }
}
