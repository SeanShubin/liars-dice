//! Round seed derivation, commitment, and verification.
//!
//! Commit, deal, reveal. A relay commits to a round before any claim is made, deals each
//! seat its dice, and reveals the seed when the round resolves. Anyone holding the log can
//! then recompute every die and check it against what was recorded.
//!
//! Realizes /specification/dice-integrity.md.
//!
//! Pure: no I/O, no clock, no knowledge of rules, claims or players. Integer arithmetic
//! only, because a face determines the content of an event and
//! /specification/architecture.md forbids such a value touching floating point.
//!
//! What this does not do, stated plainly so the scheme is not read as a stronger promise
//! than it makes: a commitment stops the dice being altered after it is published. It does
//! not stop a dealer choosing which seed to publish, by deriving several and keeping the
//! one it likes. Preventing that needs every player to contribute to the seed with their
//! contributions committed first. /specification/dice-integrity.md says the relay is
//! trusted to deal fairly, so that is out of scope.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// The deployment's long-lived secret, from which every seed derives.
///
/// Held outside the code and outside the log. /specification/architecture.md allows the
/// relay to hold exactly this between messages, which is what lets a replacement relay
/// recompute a round in progress rather than storing one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Secret(pub [u8; 32]);

/// A round's seed. Secret until the round resolves, then written to the log.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Seed(pub [u8; 32]);

/// A commitment to a seed, written to the log before any claim in the round.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Commitment(pub [u8; 32]);

/// The largest multiple of six that fits in a byte.
///
/// A byte at or above this is discarded rather than reduced. Without that, `byte % 6`
/// gives faces one to four on 43 of the 256 values and faces five and six on 42 — a 2.4%
/// edge on low faces, in a game whose whole premise is counting faces.
const UNBIASED_CEILING: u8 = 252;

const FACES: u8 = 6;

impl Secret {
    /// Read a secret from its bytes, as a deployment would from its parameter store.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Read a secret from 64 hexadecimal digits, which is how a parameter store holds it.
    ///
    /// `None` rather than a default, and the caller is expected to fail: a secret that was
    /// invented because the real one could not be read would make every commitment in
    /// every match unverifiable, and nothing would say so.
    pub fn from_hex(text: &str) -> Option<Self> {
        unhex(text).map(Secret)
    }

    /// Deliberately not `Display`, and deliberately not public.
    ///
    /// There is no way to render a secret, because the only reasons to render one are to
    /// log it or to send it, and both are the one thing this type exists to prevent.
    #[cfg(test)]
    fn to_hex(self) -> String {
        hex(&self.0)
    }
}

impl Seed {
    /// Lower case hexadecimal, which is how the seed appears in the log.
    pub fn to_hex(self) -> String {
        hex(&self.0)
    }

    /// Read a seed back from a log line. `None` if the text is not 64 hexadecimal digits.
    pub fn from_hex(text: &str) -> Option<Self> {
        unhex(text).map(Seed)
    }
}

impl Commitment {
    pub fn to_hex(self) -> String {
        hex(&self.0)
    }

    pub fn from_hex(text: &str) -> Option<Self> {
        unhex(text).map(Commitment)
    }
}

fn hex(bytes: &[u8; 32]) -> String {
    let mut text = String::with_capacity(64);
    for byte in bytes {
        text.push(char::from_digit(u32::from(byte >> 4), 16).expect("a nibble is a hex digit"));
        text.push(char::from_digit(u32::from(byte & 0x0f), 16).expect("a nibble is a hex digit"));
    }
    text
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    let digits: Vec<char> = text.chars().collect();
    for (at, byte) in bytes.iter_mut().enumerate() {
        let high = digits[at * 2].to_digit(16)?;
        let low = digits[at * 2 + 1].to_digit(16)?;
        *byte = (high * 16 + low) as u8;
    }
    Some(bytes)
}

fn mac(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac takes a key of any length");
    mac.update(message);
    mac.finalize().into_bytes().into()
}

/// The seed for one round of one match.
///
/// Derived rather than stored. A relay keeps no per-round state, so a relay that is
/// stopped and replaced recomputes the round it was in the middle of.
pub fn seed_for(secret: &Secret, match_id: &str, round: u32) -> Seed {
    let mut message = Vec::with_capacity(match_id.len() + 5);
    message.extend_from_slice(match_id.as_bytes());
    message.push(0);
    message.extend_from_slice(&round.to_be_bytes());
    Seed(mac(&secret.0, &message))
}

/// What is published before any claim in the round.
pub fn commit(seed: &Seed) -> Commitment {
    let mut hasher = Sha256::new();
    hasher.update(seed.0);
    Commitment(hasher.finalize().into())
}

/// Whether a revealed seed is the one that was committed to.
pub fn verify(commitment: &Commitment, seed: &Seed) -> bool {
    commit(seed) == *commitment
}

/// The faces dealt to one seat.
///
/// Keyed by seat, so one seat's faces say nothing about another's. Faces come from a
/// counter-mode stream by rejection sampling; see `UNBIASED_CEILING`.
pub fn deal(seed: &Seed, seat: u8, count: usize) -> Vec<u8> {
    let mut faces = Vec::with_capacity(count);
    let mut counter: u32 = 0;
    while faces.len() < count {
        let mut message = [0u8; 5];
        message[0] = seat;
        message[1..].copy_from_slice(&counter.to_be_bytes());
        for byte in mac(&seed.0, &message) {
            if faces.len() == count {
                break;
            }
            if byte < UNBIASED_CEILING {
                faces.push(byte % FACES + 1);
            }
        }
        counter += 1;
    }
    faces
}

/// A permutation of `len` items, derived from a seed.
///
/// Turn order is chosen at random when the match starts, and like everything else here it
/// is derived rather than stored, so anyone holding the log can recompute it. Fisher-Yates
/// with the same rejection sampling the dice use, so every ordering is equally likely.
pub fn shuffle(seed: &Seed, len: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    // Drawn from a stream keyed apart from any seat, so an ordering tells nobody anything
    // about anyone's dice.
    let mut stream: Vec<u8> = Vec::new();
    let mut at = 0;
    let mut counter: u32 = 0;
    for position in (1..len).rev() {
        let span = (position + 1) as u16;
        // The largest multiple of `span` that fits in a byte. A draw at or above it would
        // favour the low positions, exactly as it would favour low faces.
        let ceiling = 256 - (256 % span);
        let draw = loop {
            if at == stream.len() {
                let mut message = [0u8; 5];
                message[0] = ORDER_STREAM;
                message[1..].copy_from_slice(&counter.to_be_bytes());
                stream = mac(&seed.0, &message).to_vec();
                at = 0;
                counter += 1;
            }
            let byte = u16::from(stream[at]);
            at += 1;
            if byte < ceiling {
                break byte % span;
            }
        };
        order.swap(position, draw as usize);
    }
    order
}

/// Keyed apart from every seat, which are numbered from zero.
const ORDER_STREAM: u8 = 0xff;

#[cfg(test)]
mod tests {
    /// `assert!` rather than `assert_eq!` throughout: `Secret` implements no `Debug`, so
    /// a failing comparison cannot print it. That is the point of the type, and a test is
    /// not a reason to weaken it.
    #[test]
    fn a_secret_reads_back_from_its_hexadecimal() {
        let secret = Secret::from_bytes([7; 32]);
        assert!(Secret::from_hex(&secret.to_hex()) == Some(secret));
    }

    /// A parameter store holding something that is not a secret must not produce one.
    #[test]
    fn text_that_is_not_a_secret_is_not_read_as_one() {
        assert!(Secret::from_hex("").is_none());
        assert!(Secret::from_hex("not hexadecimal").is_none());
        assert!(Secret::from_hex(&"ab".repeat(31)).is_none(), "too short");
        assert!(Secret::from_hex(&"ab".repeat(33)).is_none(), "too long");
    }

    use super::*;

    fn secret() -> Secret {
        Secret::from_bytes([7u8; 32])
    }

    #[test]
    fn a_seed_is_the_same_every_time_it_is_derived() {
        // The relay-restart property: a replacement process with the same secret
        // recomputes the round it inherited rather than needing it stored.
        let first = seed_for(&secret(), "match-1", 3);
        let second = seed_for(&secret(), "match-1", 3);
        assert_eq!(first, second);
        assert_eq!(deal(&first, 0, 5), deal(&second, 0, 5));
    }

    #[test]
    fn different_rounds_and_matches_are_unrelated() {
        let one = seed_for(&secret(), "match-1", 1);
        let next_round = seed_for(&secret(), "match-1", 2);
        let other_match = seed_for(&secret(), "match-2", 1);
        assert_ne!(one, next_round);
        assert_ne!(one, other_match);
        assert_ne!(next_round, other_match);
    }

    #[test]
    fn a_different_secret_deals_a_different_match() {
        let theirs = Secret::from_bytes([8u8; 32]);
        assert_ne!(
            seed_for(&secret(), "match-1", 1),
            seed_for(&theirs, "match-1", 1)
        );
    }

    #[test]
    fn a_commitment_verifies_against_its_seed_and_no_other() {
        let seed = seed_for(&secret(), "match-1", 1);
        let other = seed_for(&secret(), "match-1", 2);
        let commitment = commit(&seed);
        assert!(verify(&commitment, &seed));
        assert!(!verify(&commitment, &other));
    }

    #[test]
    fn every_seat_is_dealt_faces_in_range() {
        let seed = seed_for(&secret(), "match-1", 1);
        for seat in 0..10u8 {
            let faces = deal(&seed, seat, 5);
            assert_eq!(faces.len(), 5);
            for face in faces {
                assert!((1..=6).contains(&face), "{face} is not a die face");
            }
        }
    }

    #[test]
    fn one_seat_says_nothing_about_another() {
        let seed = seed_for(&secret(), "match-1", 1);
        let mut seen = Vec::new();
        for seat in 0..10u8 {
            let faces = deal(&seed, seat, 5);
            assert!(
                !seen.contains(&faces),
                "two seats were dealt the same five faces, which should not happen by \
                 chance across ten seats"
            );
            seen.push(faces);
        }
    }

    /// The faces must be flat. This test is only worth having if it can see the bug it
    /// exists to catch, so the biased alternative is implemented here and asserted to
    /// fail the same check.
    #[test]
    fn rejection_sampling_removes_the_modulo_bias() {
        fn spread(counts: &[u32; 7], total: u32) -> f64 {
            let expected = f64::from(total) / 6.0;
            (1..=6)
                .map(|face| (f64::from(counts[face]) - expected).abs() / expected)
                .fold(0.0, f64::max)
        }

        let total = 240_000u32;
        let mut fair = [0u32; 7];
        let mut biased = [0u32; 7];
        let mut drawn = 0u32;
        let mut round = 0u32;
        while drawn < total {
            let seed = seed_for(&secret(), "bias", round);
            for seat in 0..10u8 {
                for face in deal(&seed, seat, 24) {
                    fair[face as usize] += 1;
                    drawn += 1;
                }
                // The naive reduction, over the same bytes.
                let mut message = [0u8; 5];
                message[0] = seat;
                for byte in mac(&seed.0, &message) {
                    biased[(byte % FACES + 1) as usize] += 1;
                }
            }
            round += 1;
        }

        assert!(
            spread(&fair, drawn) < 0.02,
            "rejection sampling drifted: {fair:?}"
        );
        let biased_total: u32 = biased[1..=6].iter().sum();
        assert!(
            spread(&biased, biased_total) > spread(&fair, drawn),
            "the biased implementation was not detectably worse, so this test proves \
             nothing: {biased:?}"
        );
    }

    #[test]
    fn an_ordering_is_a_permutation_and_depends_on_the_seed() {
        let seed = seed_for(&secret(), "match-1", 0);
        let order = shuffle(&seed, 6);
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2, 3, 4, 5], "every seat appears once");
        assert_eq!(
            shuffle(&seed, 6),
            order,
            "the same seed orders the same way"
        );
        assert_ne!(
            shuffle(&seed_for(&secret(), "match-2", 0), 6),
            order,
            "a different match orders differently"
        );
        assert_eq!(shuffle(&seed, 1), vec![0]);
        assert!(shuffle(&seed, 0).is_empty());
    }

    /// Every position must be equally likely, or the player who joined first is favoured.
    #[test]
    fn an_ordering_does_not_favour_a_position() {
        let mut seen = [[0u32; 4]; 4];
        let rounds = 12_000u32;
        for round in 0..rounds {
            let order = shuffle(&seed_for(&secret(), "fair", round), 4);
            for (position, seat) in order.iter().enumerate() {
                seen[*seat][position] += 1;
            }
        }
        let expected = f64::from(rounds) / 4.0;
        for seat in seen {
            for count in seat {
                let drift = (f64::from(count) - expected).abs() / expected;
                assert!(drift < 0.08, "a seat favoured a position: {seen:?}");
            }
        }
    }

    #[test]
    fn a_seed_survives_a_trip_through_the_log() {
        let seed = seed_for(&secret(), "match-1", 9);
        let text = seed.to_hex();
        assert_eq!(text.len(), 64);
        assert_eq!(Seed::from_hex(&text), Some(seed));

        let commitment = commit(&seed);
        assert_eq!(Commitment::from_hex(&commitment.to_hex()), Some(commitment));
    }

    #[test]
    fn text_that_is_not_a_seed_is_refused() {
        assert_eq!(Seed::from_hex(""), None);
        assert_eq!(Seed::from_hex("abc"), None);
        assert_eq!(Seed::from_hex(&"z".repeat(64)), None);
    }

    /// The detection the whole scheme exists for: derive from the revealed seed, compare
    /// against what the log recorded, and disagree.
    #[test]
    fn a_tampered_deal_disagrees_with_the_revealed_seed() {
        let seed = seed_for(&secret(), "match-1", 1);
        let recorded = deal(&seed, 2, 5);
        let mut tampered = recorded.clone();
        tampered[0] = tampered[0] % 6 + 1;
        assert_eq!(deal(&seed, 2, 5), recorded);
        assert_ne!(deal(&seed, 2, 5), tampered);
    }
}
