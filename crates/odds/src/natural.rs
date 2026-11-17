//! A non-negative integer of unbounded size.
//!
//! The probability model is exact, and the numbers it needs are the number of faces raised
//! to the number of unknown dice. That passes a hundred and twenty-eight bits at fifty
//! dice, and nothing in the rules caps the dice in play, so a fixed width would only move
//! the ceiling rather than remove it. This type removes it.
//!
//! It is the operations a binomial needs and nothing else: add, subtract, multiply, divide
//! by a small divisor, raise a small base to a power, and compare. Every one of them is
//! exact. There is no rounding anywhere in this file, which is the point of it.
//!
//! Digits are base two to the thirty-two, least significant first, with no leading zero
//! digit. A value therefore has exactly one representation, so equality is digit equality
//! and comparison is a digit walk.

use std::cmp::Ordering;

/// Bits per digit.
const SHIFT: u32 = 32;

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Natural {
    digits: Vec<u32>,
}

impl Natural {
    pub fn zero() -> Self {
        Self { digits: Vec::new() }
    }

    pub fn one() -> Self {
        Self::new(1)
    }

    pub fn new(value: u64) -> Self {
        Self::trimmed(vec![value as u32, (value >> SHIFT) as u32])
    }

    /// `base` raised to `exponent`.
    ///
    /// Repeated multiplication by a single digit rather than squaring: the exponent is the
    /// number of dice, so the loop is short and the digit multiply is the cheap one.
    pub fn power(base: u32, exponent: u32) -> Self {
        let mut value = Self::one();
        for _ in 0..exponent {
            value = value.times(base);
        }
        value
    }

    fn trimmed(mut digits: Vec<u32>) -> Self {
        while digits.last() == Some(&0) {
            digits.pop();
        }
        Self { digits }
    }

    pub fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    pub fn add(&self, other: &Self) -> Self {
        let width = self.digits.len().max(other.digits.len());
        let mut digits = Vec::with_capacity(width + 1);
        let mut carry = 0u64;
        for index in 0..width {
            let total = self.digit(index) + other.digit(index) + carry;
            digits.push(total as u32);
            carry = total >> SHIFT;
        }
        if carry > 0 {
            digits.push(carry as u32);
        }
        Self::trimmed(digits)
    }

    /// `self` less `other`.
    ///
    /// # Panics
    /// If `other` is the larger. A negative count is not a thing this type can hold, and
    /// the model only ever subtracts a count of outcomes from the total number of
    /// outcomes, so reaching this would mean the model is wrong rather than that the
    /// caller wants wrapping.
    pub fn sub(&self, other: &Self) -> Self {
        assert!(self >= other, "a natural number cannot go below zero");
        let mut digits = Vec::with_capacity(self.digits.len());
        let mut borrow = 0i64;
        for index in 0..self.digits.len() {
            let mut difference = self.digit(index) as i64 - other.digit(index) as i64 - borrow;
            borrow = if difference < 0 {
                difference += 1i64 << SHIFT;
                1
            } else {
                0
            };
            digits.push(difference as u32);
        }
        Self::trimmed(digits)
    }

    pub fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        let mut digits = vec![0u32; self.digits.len() + other.digits.len()];
        for (left_at, &left) in self.digits.iter().enumerate() {
            let mut carry = 0u64;
            for (right_at, &right) in other.digits.iter().enumerate() {
                let at = left_at + right_at;
                let total = digits[at] as u64 + left as u64 * right as u64 + carry;
                digits[at] = total as u32;
                carry = total >> SHIFT;
            }
            let mut at = left_at + other.digits.len();
            while carry > 0 {
                let total = digits[at] as u64 + carry;
                digits[at] = total as u32;
                carry = total >> SHIFT;
                at += 1;
            }
        }
        Self::trimmed(digits)
    }

    /// `self` multiplied by one digit.
    pub fn times(&self, factor: u32) -> Self {
        if factor == 0 || self.is_zero() {
            return Self::zero();
        }
        let mut digits = Vec::with_capacity(self.digits.len() + 1);
        let mut carry = 0u64;
        for index in 0..self.digits.len() {
            let total = self.digit(index) * factor as u64 + carry;
            digits.push(total as u32);
            carry = total >> SHIFT;
        }
        if carry > 0 {
            digits.push(carry as u32);
        }
        Self::trimmed(digits)
    }

    /// `self` divided by one digit, rounded down.
    ///
    /// The model divides only where the division comes out exact — a binomial coefficient
    /// times the next row's numerator is always divisible by that row's denominator — so
    /// rounding down is never reached in practice. It is defined rather than left to
    /// chance so that this type has no partial operations beyond the documented panic.
    ///
    /// # Panics
    /// If `divisor` is zero.
    pub fn divided_by(&self, divisor: u32) -> Self {
        assert!(divisor != 0, "cannot divide by zero");
        let mut digits = vec![0u32; self.digits.len()];
        let mut remainder = 0u64;
        for index in (0..self.digits.len()).rev() {
            let current = (remainder << SHIFT) | self.digit(index);
            digits[index] = (current / divisor as u64) as u32;
            remainder = current % divisor as u64;
        }
        Self::trimmed(digits)
    }

    /// How many bits the value occupies. Zero occupies none.
    pub fn bit_len(&self) -> u32 {
        match self.digits.last() {
            None => 0,
            Some(&top) => (self.digits.len() as u32 - 1) * SHIFT + (SHIFT - top.leading_zeros()),
        }
    }

    /// The leading bits, as `(significand, scale)`, where the value is `significand`
    /// shifted left by `scale` bits.
    ///
    /// Exact when the value fits in sixty-four bits, in which case `scale` is zero.
    /// Otherwise the significand holds the top sixty-four bits and the rest are dropped,
    /// so the pair is the value rounded toward zero.
    ///
    /// This type does no rounding itself and has no opinion about approximation. It
    /// exposes this so that something outside the model — a readout a person looks at —
    /// can do the rounding on its own side of that boundary, reading one exact value
    /// rather than running a second implementation of the model.
    pub fn significand(&self) -> (u64, u32) {
        let bits = self.bit_len();
        if bits == 0 {
            return (0, 0);
        }
        let scale = bits.saturating_sub(64);
        let mut significand = 0u64;
        for index in (0..self.digits.len()).rev() {
            let lowest_bit = index as u32 * SHIFT;
            if lowest_bit + SHIFT <= scale {
                break;
            }
            if lowest_bit >= scale {
                significand |= self.digit(index) << (lowest_bit - scale);
            } else {
                significand |= self.digit(index) >> (scale - lowest_bit);
                break;
            }
        }
        (significand, scale)
    }

    fn digit(&self, index: usize) -> u64 {
        self.digits.get(index).copied().unwrap_or(0) as u64
    }
}

impl Ord for Natural {
    fn cmp(&self, other: &Self) -> Ordering {
        self.digits
            .len()
            .cmp(&other.digits.len())
            .then_with(|| self.digits.iter().rev().cmp(other.digits.iter().rev()))
    }
}

impl PartialOrd for Natural {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_has_one_representation() {
        assert_eq!(Natural::new(0), Natural::zero());
        assert_eq!(Natural::new(1).times(0), Natural::zero());
        assert_eq!(Natural::new(7).sub(&Natural::new(7)), Natural::zero());
        assert!(Natural::zero().is_zero());
        assert_eq!(Natural::zero().bit_len(), 0);
    }

    #[test]
    fn arithmetic_agrees_with_the_machine_integers_it_outgrows() {
        let values = [0u64, 1, 2, 5, 255, 65_535, u32::MAX as u64, u64::MAX / 3];
        for left in values {
            for right in values {
                let big = Natural::new(left);
                let small = Natural::new(right);
                assert_eq!(big.cmp(&small), left.cmp(&right), "{left} ? {right}");
                if let Some(sum) = left.checked_add(right) {
                    assert_eq!(big.add(&small), Natural::new(sum), "{left} + {right}");
                }
                if let Some(product) = left.checked_mul(right) {
                    assert_eq!(big.mul(&small), Natural::new(product), "{left} * {right}");
                }
                if left >= right {
                    let difference = Natural::new(left - right);
                    assert_eq!(big.sub(&small), difference, "{left} - {right}");
                }
            }
        }
    }

    #[test]
    fn a_digit_multiply_agrees_with_a_full_multiply() {
        let value = Natural::power(6, 40);
        for factor in [1u32, 2, 5, 6, 97, u32::MAX] {
            assert_eq!(value.times(factor), value.mul(&Natural::new(factor as u64)));
        }
    }

    #[test]
    fn dividing_undoes_multiplying() {
        let value = Natural::power(6, 30).add(&Natural::new(12_345));
        for divisor in [1u32, 2, 3, 7, 64, 1_000_003] {
            assert_eq!(value.times(divisor).divided_by(divisor), value);
        }
    }

    #[test]
    fn a_power_grows_past_every_machine_integer() {
        // Six to the fiftieth is where a hundred and twenty-eight bits runs out, which is
        // the ceiling this type exists to remove.
        assert!(Natural::power(6, 49).bit_len() <= 128);
        assert!(Natural::power(6, 50).bit_len() > 128);
        assert!(Natural::power(6, 500).bit_len() > 1_000);
        assert_eq!(Natural::power(6, 0), Natural::one());
        assert_eq!(Natural::power(6, 1), Natural::new(6));
    }

    #[test]
    fn a_power_is_the_product_of_its_parts() {
        for exponent in 0..40u32 {
            assert_eq!(
                Natural::power(6, exponent).times(6),
                Natural::power(6, exponent + 1)
            );
        }
    }

    #[test]
    fn the_significand_rebuilds_a_small_value_exactly() {
        for value in [0u64, 1, 6, 255, u32::MAX as u64, u64::MAX] {
            let (significand, scale) = Natural::new(value).significand();
            assert_eq!(scale, 0, "{value} should need no scale");
            assert_eq!(significand, value);
        }
    }

    #[test]
    fn the_significand_brackets_a_large_value() {
        for exponent in 1..200u32 {
            let value = Natural::power(6, exponent);
            let (significand, scale) = value.significand();
            let floor = Natural::new(significand).mul(&Natural::power(2, scale));
            let ceiling = floor.add(&Natural::power(2, scale));
            assert!(
                floor <= value,
                "six to the {exponent} fell below its significand"
            );
            assert!(
                value < ceiling,
                "six to the {exponent} rose above its significand"
            );
            if value.bit_len() > 64 {
                assert_eq!(
                    significand.leading_zeros(),
                    0,
                    "six to the {exponent} left the significand unnormalised"
                );
            }
        }
    }
}
