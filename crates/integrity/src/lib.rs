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

fn hex_of(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from_digit((byte >> 4) as u32, 16).expect("a nibble"));
        text.push(char::from_digit((byte & 0x0f) as u32, 16).expect("a nibble"));
    }
    text
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

/// The name of a match, derived from who asked for it and which request it was.
///
/// Not a counter. A count is a function of global history, so reading one to name a match
/// means reading the whole log table, and resending a request that was never answered
/// makes a second match. This is a function of the intent instead: the same account and
/// the same nonce name the same match, every time, so a resend is recognisable rather
/// than duplicative.
///
/// The nonce need not be secret or unpredictable, only distinct per request. Guessing a
/// match id reveals nothing, because every seed is [`seed_for`] under a secret the client
/// does not have.
///
/// Sixteen bytes, which is as collision-resistant as anything this will ever hold, and
/// short enough to paste into a terminal.
pub fn name_for(secret: &Secret, account: &str, nonce: &str) -> String {
    // Tagged and NUL-separated, so this message space and `seed_for`'s cannot overlap and
    // no pair of (account, nonce) can be rearranged into another.
    let mut message = Vec::new();
    message.extend_from_slice(b"match");
    message.push(0);
    message.extend_from_slice(account.as_bytes());
    message.push(0);
    message.extend_from_slice(nonce.as_bytes());
    hex_of(&mac(&secret.0, &message)[..16])
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

/// How long a minted ticket is good for.
///
/// Long enough that nobody re-authenticates inside one sitting, short enough that a leaked
/// one stops working. What a stolen ticket buys is a seat in a ranked match under somebody
/// else's name, which today is worth nothing because nothing consumes ranked — so this is
/// a number to revisit when something does, and it is one constant in one place.
pub const TICKET_SECONDS: u64 = 24 * 60 * 60;

/// What a ticket proves.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ticket {
    /// The account, as `protocol::account_text` writes it.
    pub account: String,
    /// The display name to record at the join.
    pub name: String,
    /// The Discord username, which Discord keeps unique where a display name is not.
    ///
    /// Proved rather than sent, for the same reason the display name is: a client that
    /// could assert a username could claim to be anybody. It is what lets a reader
    /// tell two players with one display name apart, and nothing else can.
    pub username: String,
}

/// Mint a ticket: an account, a display name, a username and an expiry, with a MAC over
/// all four.
///
/// `<payload-hex>.<mac-hex>`. Hex rather than plain text because a display name may
/// contain anything at all, including whatever separator a readable format would have
/// picked; and one separator rather than four because then there is exactly one way to
/// parse it. The whole payload is covered, so no field can be moved into another.
///
/// Here rather than in the exchange that calls it, because this crate already owns the
/// HMAC, the secret and the tagging convention — and a second place doing HMAC is a second
/// place to get it wrong. The ticket is a third tag beside `match` and the round seed.
///
/// `expires_at` is seconds since the epoch, read by a composition root and passed in. It
/// is the only clock reading that reaches a ticket, and it never reaches a log.
pub fn ticket(
    secret: &Secret,
    account: &str,
    name: &str,
    username: &str,
    expires_at: u64,
) -> String {
    let payload = ticket_payload(account, name, username, expires_at);
    format!(
        "{}.{}",
        hex_of(&payload),
        hex_of(&mac(&secret.0, &tagged(&payload)))
    )
}

/// The account and name a ticket proves, or `None` if it proves nothing.
///
/// `None` covers every way a ticket can fail: it did not parse, its MAC does not verify
/// under this secret, or it had expired by `now`. One answer rather than three on purpose
/// — a caller that could tell a forged ticket from an expired one is a caller that can be
/// asked which it was, and the client already knows its own ticket's expiry because the
/// exchange told it.
pub fn check_ticket(secret: &Secret, ticket: &str, now: u64) -> Option<Ticket> {
    let (payload, tag) = ticket.split_once('.')?;
    let payload = unhex_any(payload)?;
    let tag = unhex_any(tag)?;
    // Constant time, through the MAC's own comparison. Comparing two byte slices with
    // `==` would stop at the first difference, which is a measurement of how much of a
    // forgery was right.
    let mut candidate =
        HmacSha256::new_from_slice(&secret.0).expect("hmac takes a key of any length");
    candidate.update(&tagged(&payload));
    candidate.verify_slice(&tag).ok()?;

    let (account, name, username, expires_at) = ticket_fields(&payload)?;
    if expires_at <= now {
        return None;
    }
    Some(Ticket {
        account,
        name,
        username,
    })
}

/// The three fields, length-prefixed.
///
/// Length-prefixed and not separated, which is where this differs from [`name_for`]. A
/// separator has to be a byte that cannot appear in any field, and here there is no such
/// byte: a display name is a `String` and may hold anything including a NUL, and an
/// expiry's big-endian bytes are mostly zeros for every plausible timestamp. The first
/// version of this did separate on NUL and split inside the expiry, which the round-trip
/// test caught immediately. Lengths have no such case.
/// What a ticket says, without checking whether it is true.
///
/// For a client reading its own ticket, and for nothing else. A client has to know which
/// account the relay will call it, so that it can recognise its own `join` coming back —
/// and it holds no secret, so it cannot verify. Reading your own ticket is not a security
/// operation.
///
/// Never for deciding anything. The name is deliberately not `ticket_of` or `read_ticket`:
/// these are claims, and [`check_ticket`] is the only function in this crate that turns a
/// claim into a fact. A relay that called this instead would accept any ticket anybody
/// typed.
pub fn ticket_claims(ticket: &str) -> Option<Ticket> {
    let payload = unhex_any(ticket.split_once('.')?.0)?;
    let (account, name, username, _) = ticket_fields(&payload)?;
    Some(Ticket {
        account,
        name,
        username,
    })
}

fn ticket_payload(account: &str, name: &str, username: &str, expires_at: u64) -> Vec<u8> {
    let mut payload = Vec::new();
    for field in [account.as_bytes(), name.as_bytes(), username.as_bytes()] {
        payload.extend_from_slice(&(field.len() as u32).to_be_bytes());
        payload.extend_from_slice(field);
    }
    payload.extend_from_slice(&expires_at.to_be_bytes());
    payload
}

/// The inverse of [`ticket_payload`], or `None` if the bytes are not one.
///
/// Exact: the expiry must be the last eight bytes and nothing may follow it, so a payload
/// with anything appended is not a payload. The MAC has already been verified by the time
/// this runs, so a failure here means a ticket this build minted and cannot read — worth
/// refusing loudly rather than guessing at.
fn ticket_fields(payload: &[u8]) -> Option<(String, String, String, u64)> {
    let mut rest = payload;
    let mut text = Vec::new();
    for _ in 0..3 {
        let length = u32::from_be_bytes(rest.get(..4)?.try_into().ok()?) as usize;
        let body = rest.get(4..4 + length)?;
        text.push(String::from_utf8(body.to_vec()).ok()?);
        rest = rest.get(4 + length..)?;
    }
    let expiry: [u8; 8] = rest.try_into().ok()?;
    let mut text = text.into_iter();
    Some((
        text.next()?,
        text.next()?,
        text.next()?,
        u64::from_be_bytes(expiry),
    ))
}

/// Tagged, so a ticket's message space cannot overlap a match name's or a seed's.
fn tagged(payload: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(payload.len() + 7);
    message.extend_from_slice(b"ticket");
    message.push(0);
    message.extend_from_slice(payload);
    message
}

/// Hex of any length, for a payload whose size is not known in advance.
fn unhex_any(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let digits: Vec<char> = text.chars().collect();
    digits
        .chunks(2)
        .map(|pair| {
            let high = pair[0].to_digit(16)?;
            let low = pair[1].to_digit(16)?;
            Some((high * 16 + low) as u8)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    /// `assert!` rather than `assert_eq!` throughout: `Secret` implements no `Debug`, so
    /// a failing comparison cannot print it. That is the point of the type, and a test is
    /// not a reason to weaken it.
    /// A ticket round-trips: minted for an account, a name and a username, verified
    /// back to the same three.
    ///
    /// The display name and the username differ here on purpose. They are the same
    /// field in Discord's older scheme and different fields in its current one, and a
    /// test that used one string for both would pass against an encoder that dropped
    /// either of them.
    #[test]
    fn a_ticket_proves_the_account_the_name_and_the_username_it_was_minted_for() {
        let secret = Secret::from_bytes([11; 32]);
        // A name with a space, because the archive's line format would hyphenate it and a
        // ticket must not: the relay records what the ticket says.
        let minted = ticket(
            &secret,
            "discord:450853396908343307",
            "Mary Jane",
            "mj",
            1_000,
        );
        assert_eq!(
            check_ticket(&secret, &minted, 999),
            Some(Ticket {
                account: "discord:450853396908343307".to_string(),
                name: "Mary Jane".to_string(),
                username: "mj".to_string(),
            })
        );
    }

    /// A name holding the separator, or a dot, or nothing at all.
    ///
    /// Hex encoding is what makes these impossible rather than merely unlikely, and a
    /// test over them is what says the encoding is doing the job the format depends on.
    #[test]
    fn a_name_cannot_be_rearranged_into_another_field() {
        let secret = Secret::from_bytes([11; 32]);
        for name in ["", ".", "a.b", "a\0b", "discord:9", "\u{1f600} dice", "   "] {
            // The same awkward text in both name positions, so a field that leaked into
            // its neighbour would be caught whichever direction it leaked.
            let minted = ticket(&secret, "discord:1", name, name, 1_000);
            let checked = check_ticket(&secret, &minted, 0).expect("{name:?} did not verify");
            assert_eq!(checked.account, "discord:1", "{name:?} moved the account");
            assert_eq!(checked.name, name, "{name:?} did not come back");
            assert_eq!(checked.username, name, "{name:?} lost its username");
        }
    }

    /// A tampered ticket is refused — every field, one at a time.
    ///
    /// One at a time on purpose: a passing test then means the MAC covers all of them
    /// rather than most of them.
    #[test]
    fn every_field_is_covered_by_the_mac() {
        let secret = Secret::from_bytes([11; 32]);
        let honest = ticket(&secret, "discord:1", "Ada", "ada", 1_000);
        let forgeries = [
            // Each is the same ticket with one field changed and the original MAC kept.
            (
                "the account",
                ticket(&secret, "discord:2", "Ada", "ada", 1_000),
            ),
            (
                "the name",
                ticket(&secret, "discord:1", "Eve", "ada", 1_000),
            ),
            (
                "the username",
                ticket(&secret, "discord:1", "Ada", "eve", 1_000),
            ),
            (
                "the expiry",
                ticket(&secret, "discord:1", "Ada", "ada", 2_000),
            ),
        ];
        let tag = honest.split_once('.').expect("a tag").1;
        for (what, other) in forgeries {
            let payload = other.split_once('.').expect("a payload").0;
            let forged = format!("{payload}.{tag}");
            assert_ne!(forged, honest, "{what} was not actually changed");
            assert_eq!(
                check_ticket(&secret, &forged, 0),
                None,
                "{what} is not covered by the mac"
            );
        }
    }

    /// An expired ticket is refused, with the clock injected rather than slept through.
    #[test]
    fn a_ticket_expires() {
        let secret = Secret::from_bytes([11; 32]);
        let minted = ticket(&secret, "discord:1", "Ada", "ada", 1_000);
        assert!(check_ticket(&secret, &minted, 999).is_some(), "still good");
        assert_eq!(
            check_ticket(&secret, &minted, 1_000),
            None,
            "a ticket is not good at the instant it expires"
        );
        assert_eq!(check_ticket(&secret, &minted, 1_001), None);
    }

    /// A ticket from another secret is refused.
    ///
    /// The case that matters if the relay and the exchange ever disagree about which
    /// secret they share, which is the one way this scheme fails silently rather than
    /// loudly.
    #[test]
    fn a_ticket_from_another_secret_proves_nothing() {
        let minted = ticket(
            &Secret::from_bytes([11; 32]),
            "discord:1",
            "Ada",
            "ada",
            1_000,
        );
        assert_eq!(
            check_ticket(&Secret::from_bytes([12; 32]), &minted, 0),
            None
        );
    }

    /// Nothing that is not a ticket is mistaken for one.
    #[test]
    fn rubbish_is_not_a_ticket() {
        let secret = Secret::from_bytes([11; 32]);
        for text in [
            "",
            ".",
            "..",
            "not hex.not hex",
            "ab.cd",
            // A valid payload with no tag at all.
            &hex_of(&ticket_payload("discord:1", "Ada", "ada", 1_000)),
            // A tag of the right shape over nothing.
            &format!("{}.{}", "", "00".repeat(32)),
        ] {
            assert_eq!(check_ticket(&secret, text, 0), None, "{text:?} verified");
        }
    }

    /// The tag is what keeps a ticket out of the other two message spaces.
    ///
    /// Without it a value signed as a match name could be presented as a ticket, or the
    /// reverse. Neither is reachable through the public functions, so this checks the
    /// property directly on the message.
    #[test]
    fn a_ticket_shares_no_message_space_with_a_match_name_or_a_seed() {
        let secret = Secret::from_bytes([11; 32]);
        let payload = ticket_payload("discord:1", "Ada", "ada", 1_000);
        assert_ne!(
            hex_of(&mac(&secret.0, &tagged(&payload))),
            hex_of(&mac(&secret.0, &payload)),
            "the tag is not reaching the mac"
        );
    }

    /// The same request names the same match; a different one names a different match.
    #[test]
    fn a_name_is_a_function_of_who_asked_and_which_request_it_was() {
        let secret = Secret::from_bytes([7; 32]);
        let name = name_for(&secret, "discord:1", "n");
        assert_eq!(name, name_for(&secret, "discord:1", "n"), "not idempotent");
        assert_eq!(name.len(), 32, "sixteen bytes of hexadecimal");
        assert!(name.chars().all(|c| c.is_ascii_hexdigit()));

        assert_ne!(name, name_for(&secret, "discord:2", "n"), "another account");
        assert_ne!(name, name_for(&secret, "discord:1", "m"), "another request");
        assert_ne!(
            name,
            name_for(&Secret::from_bytes([8; 32]), "discord:1", "n"),
            "another secret"
        );
    }

    /// The separator is what stops one pair being rearranged into another.
    #[test]
    fn the_account_and_the_nonce_cannot_be_run_together() {
        let secret = Secret::from_bytes([7; 32]);
        assert_ne!(
            name_for(&secret, "ab", "c"),
            name_for(&secret, "a", "bc"),
            "the boundary between the account and the nonce must be unambiguous"
        );
    }

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
