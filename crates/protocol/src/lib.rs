//! Messages and the event log format.
//!
//! An event is one line: a command and its arguments, separated by spaces. The line is the
//! format — there is no second representation to keep in step.
//!
//! Nothing derivable is written down. A claim does not record how many dice the claimer
//! held, and a resolution does not record how many dice actually matched, because both
//! follow from the `roll` lines above them. A field that repeats what can be derived is a
//! field that can disagree with it, so /specification/architecture.md's requirement that
//! all match state be derivable from the event log is met by writing only what cannot be
//! derived.
//!
//! Two fields break that rule on purpose. `winner` tells a reader the log is a whole match
//! rather than a truncated one. And the odds on a claim are derivable only by redoing the
//! whole probability calculation, which is too much work to do by eye — reading a column of
//! them is how a pattern in how a player bids becomes visible at all.
//!
//! The `lucky` and `unlucky` lines are the third exception, for the same reason: spotting
//! that a player keeps surviving claims they should have lost means comparing every claim
//! against its resolution, which nobody does by eye.
//!
//! Those odds are the only rounded thing in the log. They are for a person to read and
//! must never be read back into a decision; a replay recomputes them exactly from the
//! rolls. /specification/architecture.md forbids a value crossing from the floating point
//! side to the exact side, and this is the field that would do it.
//!
//! Writing the log is an effect and belongs to a composition root. This crate only says
//! what an event is and how it reads.

use core::fmt;

/// A moment in UTC, to the millisecond.
///
/// Converted from the epoch with integer arithmetic, so there is no dependency and no
/// rounding. The calendar algorithm is the standard days-to-civil one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Moment {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
}

impl Moment {
    pub fn from_unix_nanos(nanos: u64) -> Self {
        let milliseconds = nanos / 1_000_000;
        let seconds = (milliseconds / 1_000) as i64;
        let millisecond = (milliseconds % 1_000) as u32;

        let days = seconds.div_euclid(86_400);
        let of_day = seconds.rem_euclid(86_400) as u32;
        let (year, month, day) = civil_from_days(days);

        Self {
            year,
            month,
            day,
            hour: of_day / 3_600,
            minute: (of_day / 60) % 60,
            second: of_day % 60,
            millisecond,
        }
    }

    /// Back to the epoch, exactly reversing [`Moment::from_unix_nanos`].
    ///
    /// A moment holds no sub-millisecond part, so this is the inverse of that constructor
    /// rather than of the nanoseconds it was handed: whatever nanos went in, these come
    /// back truncated to the millisecond, and feeding them back in yields the same moment.
    /// Storage wants a number, and a reversible pair of functions is a better thing to
    /// store than a formatted date something would have to parse.
    pub fn to_unix_nanos(&self) -> u64 {
        let days = days_from_civil(self.year, self.month, self.day);
        let seconds = days * 86_400
            + i64::from(self.hour) * 3_600
            + i64::from(self.minute) * 60
            + i64::from(self.second);
        let milliseconds = seconds * 1_000 + i64::from(self.millisecond);
        // The multiply happens in u64 because nanoseconds run past i64 in the year 2262
        // and the epoch type is u64. A moment before 1970 has no u64 representation at
        // all, and clamps rather than wrapping into the distant future.
        milliseconds.max(0) as u64 * 1_000_000
    }

    /// A file name stem whose text order is its chronological order.
    ///
    /// Every field is fixed width and zero padded, and the separators are the same at
    /// every position, so sorting the directory as text sorts it by when each match
    /// started. Colons would be the conventional separator for the time but cannot appear
    /// in a file name on Windows.
    pub fn file_stem(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}-{:03}",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.millisecond
        )
    }
}

impl fmt::Display for Moment {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.millisecond
        )
    }
}

/// Days since the epoch to a calendar date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// The inverse of [`civil_from_days`]: the standard days-from-civil algorithm.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 } as i64;
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The version of the log format and of the messages that carry it.
///
/// A client cannot join a match or load a save whose version differs from its own, so no
/// match spans two versions and no log holds two. The `version` line says which.
pub const PROTOCOL_VERSION: u32 = 5;

/// A seat at a match.
///
/// Events refer to players by seat and never by name. A name is mutable — a Discord
/// display name can change between two matches — and a log keyed on one would disagree
/// with itself over time.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Seat(pub u8);

impl fmt::Display for Seat {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "#{}", self.0)
    }
}

/// Who is behind a seat.
///
/// Three cases because there are three kinds of player. A desktop game has no
/// authentication, so the human at the keyboard has no account id; an AI player has no
/// account at all; a web player has a Discord snowflake. An optional id would make the
/// first two indistinguishable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Account {
    /// A web player, authenticated with Discord. The snowflake, not the display name.
    Discord(String),
    /// An AI player. Desktop games only.
    Ai,
    /// The human at the keyboard of a desktop game.
    Local,
}

impl fmt::Display for Account {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Account::Discord(id) => write!(out, "discord:{}", token(id)),
            Account::Ai => write!(out, "ai"),
            Account::Local => write!(out, "local"),
        }
    }
}

/// One thing that happened, in the order it happened.
///
/// Not `Eq`, because the odds on a claim are a float. That is a fair warning: a float does
/// not belong in anything compared for equality, and these are only ever written out.
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    /// The protocol version this log is written in. The first line of every log.
    Version {
        protocol: u32,
    },
    /// When the match started, in UTC.
    Started {
        at: Moment,
    },
    /// A player created the match and may start it, bar from it, and save it.
    Host {
        seat: Seat,
    },
    /// The host barred a seat from joining. A barred player is not shown the match.
    Bar {
        seat: Seat,
    },
    /// The host lifted a bar.
    Lift {
        seat: Seat,
    },
    /// The host started the match. Nobody may join after this.
    Start,
    /// A seat has seen the round end and is ready for the reveal.
    ///
    /// The relay reveals a round's seed once every connected seat has said this. It is a
    /// count of messages, not a judgement about the round, which is how the relay times
    /// the reveal without knowing any game rule.
    RoundOver {
        seat: Seat,
    },
    /// A player left the match.
    ///
    /// Membership, not presence. Losing a connection says nothing permanent and writes
    /// nothing; this is a player saying they are done, and it belongs in the log beside
    /// the `join` that put them there. The rules read it as an elimination; the relay
    /// reads it as one fewer participant to wait for.
    Left {
        seat: Seat,
    },
    /// A player takes a seat, with the name and colour that identify them to a reader and
    /// the account behind them. The only event carrying a name: a name is recorded once,
    /// when a player joins, and no later line repeats it.
    Join {
        seat: Seat,
        name: String,
        colour: String,
        account: Account,
    },
    /// Turn order, which is chosen at random when the match starts.
    PlayerOrder(Vec<Seat>),
    /// One per variant, so a replay knows which rules were in force.
    Variant {
        name: &'static str,
        on: bool,
    },
    /// The dealer's commitment to this round, published before any claim in it. The
    /// digest of the round's seed, in lower case hexadecimal.
    Commit {
        digest: String,
    },
    /// The round's seed, published when the round resolves. Everything dealt in the
    /// round derives from it, so a reader can recompute every die and check it against
    /// the deal lines above.
    Reveal {
        seed: String,
    },
    /// Every seat is dealt the dice limit, whatever its player holds. The held dice are
    /// the prefix; the rest are not in play. One of these per seat per round.
    Deal {
        seat: Seat,
        dice: Vec<u8>,
    },
    /// A raise: at least this quantity of this face.
    ///
    /// `odds` is the percentage chance the claim is true, from what the claimer could see
    /// when they made it.
    Claim {
        seat: Seat,
        quantity: u32,
        face: u8,
        odds: f64,
    },
    /// A claim that the previous claim is false. `odds` is the chance that is so.
    Liar {
        seat: Seat,
        odds: f64,
    },
    /// A claim that the previous claim is exact. `odds` is the chance that is so.
    SpotOn {
        seat: Seat,
        odds: f64,
    },
    LoseDie {
        seat: Seat,
    },
    GainDie {
        seat: Seat,
    },
    /// A correct spot on at the dice limit gains a token rather than a sixth die.
    GainShield {
        seat: Seat,
    },
    /// A token prevented a die loss and was spent doing so.
    SpendShield {
        seat: Seat,
    },
    Eliminate {
        seat: Seat,
    },
    /// A claim that came true against the odds. `odds` is how likely what happened was.
    ///
    /// Derivable from the claim's own odds and the resolution, and written anyway: a run
    /// of these is how a reader sees that a player has been riding their luck.
    Lucky {
        seat: Seat,
        odds: f64,
    },
    /// A claim that failed when it should have held. `odds` is how likely what happened
    /// was.
    Unlucky {
        seat: Seat,
        odds: f64,
    },
    /// Who makes the first claim of the next round.
    NextTurn {
        seat: Seat,
    },
    /// The last player remaining. Derivable, and written anyway: it is how a reader knows
    /// the log is a whole match rather than a truncated one.
    Winner {
        seat: Seat,
    },
}

/// An account as text, losslessly: `discord:<snowflake>`, `ai`, or `local`.
///
/// Not `Display`, which renders for the archive's line format and passes the id through
/// [`token`]. This is the canonical form — the one a match id is derived from and the one
/// a frame on the socket carries — so it must round-trip exactly. Here rather than in
/// `wire` because the format is `protocol`'s business and more than one crate needs it:
/// `relay` derives a match id from it and cannot depend on `wire`.
pub fn account_text(account: &Account) -> String {
    match account {
        Account::Discord(id) => format!("discord:{id}"),
        Account::Ai => "ai".to_string(),
        Account::Local => "local".to_string(),
    }
}

/// The inverse of [`account_text`]. `None` if the text names no account.
pub fn account_from_text(text: &str) -> Option<Account> {
    match text {
        "ai" => Some(Account::Ai),
        "local" => Some(Account::Local),
        // Split once, so a colon inside an id cannot change who the account is.
        other => match other.split_once(':') {
            Some(("discord", id)) => Some(Account::Discord(id.to_string())),
            _ => None,
        },
    }
}

/// Collapses anything that would split a line into a single token.
///
/// Names and colours come from elsewhere, so the format defends itself rather than
/// trusting them. A log with a broken line is worse than a log with a renamed player.
fn token(text: &str) -> String {
    let collapsed: String = text
        .trim()
        .chars()
        .map(|character| {
            if character.is_whitespace() {
                '-'
            } else {
                character
            }
        })
        .collect();
    if collapsed.is_empty() {
        "-".to_string()
    } else {
        collapsed
    }
}

fn seats(values: &[Seat]) -> String {
    values
        .iter()
        .map(|seat| seat.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The one rounded field in the log, to two places so a column of them can be scanned.
fn percent(odds: f64) -> String {
    format!("{odds:.2}%")
}

fn faces(values: &[u8]) -> String {
    values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

impl fmt::Display for Event {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Version { protocol } => write!(out, "version {protocol}"),
            Event::Started { at } => write!(out, "started {at}"),
            Event::Join {
                seat,
                name,
                colour,
                account,
            } => write!(
                out,
                "join {seat} {} {} {account}",
                token(name),
                token(colour)
            ),
            Event::Host { seat } => write!(out, "host {seat}"),
            Event::Bar { seat } => write!(out, "bar {seat}"),
            Event::Lift { seat } => write!(out, "lift {seat}"),
            Event::Left { seat } => write!(out, "left {seat}"),
            Event::Start => write!(out, "start"),
            Event::RoundOver { seat } => write!(out, "round-over {seat}"),
            Event::PlayerOrder(order) => write!(out, "player-order {}", seats(order)),
            Event::Variant { name, on } => {
                write!(
                    out,
                    "variant {} {}",
                    token(name),
                    if *on { "on" } else { "off" }
                )
            }
            Event::Commit { digest } => write!(out, "commit {}", token(digest)),
            Event::Reveal { seed } => write!(out, "reveal {}", token(seed)),
            Event::Deal { seat, dice } => write!(out, "deal {seat} {}", faces(dice)),
            Event::Claim {
                seat,
                quantity,
                face,
                odds,
            } => write!(out, "claim {seat} {quantity} {face} {}", percent(*odds)),
            Event::Liar { seat, odds } => write!(out, "claim {seat} liar {}", percent(*odds)),
            Event::SpotOn { seat, odds } => {
                write!(out, "claim {seat} spot-on {}", percent(*odds))
            }
            Event::Lucky { seat, odds } => write!(out, "lucky {seat} {}", percent(*odds)),
            Event::Unlucky { seat, odds } => write!(out, "unlucky {seat} {}", percent(*odds)),
            Event::LoseDie { seat } => write!(out, "lose-die {seat}"),
            Event::GainDie { seat } => write!(out, "gain-die {seat}"),
            Event::GainShield { seat } => write!(out, "gain-shield {seat}"),
            Event::SpendShield { seat } => write!(out, "spend-shield {seat}"),
            Event::Eliminate { seat } => write!(out, "eliminate {seat}"),
            Event::NextTurn { seat } => write!(out, "next-turn {seat}"),
            Event::Winner { seat } => write!(out, "winner {seat}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> String {
        name.to_string()
    }

    /// One of every variant that refers to a player, so a test over "all of them" cannot
    /// silently miss one added later.
    fn every_player_event() -> Vec<Event> {
        let seat = Seat(3);
        vec![
            Event::Host { seat },
            Event::Bar { seat },
            Event::Lift { seat },
            Event::Left { seat },
            Event::RoundOver { seat },
            Event::Join {
                seat,
                name: named("Alice"),
                colour: named("red"),
                account: Account::Discord(named("450853396908343307")),
            },
            Event::PlayerOrder(vec![Seat(2), Seat(0), seat]),
            Event::Deal {
                seat,
                dice: vec![1, 3, 2, 6, 6],
            },
            Event::Claim {
                seat,
                quantity: 3,
                face: 6,
                odds: 38.207,
            },
            Event::Liar { seat, odds: 61.793 },
            Event::SpotOn { seat, odds: 0.004 },
            Event::LoseDie { seat },
            Event::GainDie { seat },
            Event::GainShield { seat },
            Event::SpendShield { seat },
            Event::Eliminate { seat },
            Event::Lucky { seat, odds: 1.2 },
            Event::Unlucky { seat, odds: 3.704 },
            Event::NextTurn { seat },
            Event::Winner { seat },
        ]
    }

    #[test]
    fn the_epoch_reads_as_itself() {
        let moment = Moment::from_unix_nanos(0);
        assert_eq!(moment.to_string(), "1970-01-01T00:00:00.000Z");
        assert_eq!(moment.file_stem(), "1970-01-01T00-00-00-000");
    }

    /// The two conversions are each other's inverse, which is the only reason storage is
    /// allowed to keep a number instead of a date. Stepping by a prime number of
    /// milliseconds crosses leap days, century years and month ends without choosing
    /// which ones to try.
    #[test]
    fn the_epoch_and_the_calendar_are_inverses() {
        let step = 900_000_011_u64;
        for index in 0..10_000 {
            let nanos = index * step * 1_000_000;
            let moment = Moment::from_unix_nanos(nanos);
            assert_eq!(
                Moment::from_unix_nanos(moment.to_unix_nanos()),
                moment,
                "{moment} did not survive the trip back to the epoch"
            );
            assert_eq!(
                moment.to_unix_nanos(),
                nanos,
                "{moment} came back as a different instant"
            );
        }
    }

    /// The canonical form round-trips, including the cases `Display` would flatten.
    #[test]
    fn an_account_reads_back_from_its_canonical_text() {
        for account in [
            Account::Discord("450853396908343307".to_string()),
            // A colon in the id, which a split on every colon would mangle.
            Account::Discord("a:b".to_string()),
            Account::Ai,
            Account::Local,
        ] {
            let text = account_text(&account);
            assert_eq!(account_from_text(&text), Some(account), "{text}");
        }
        assert_eq!(account_from_text("steam:7"), None);
        assert_eq!(account_from_text(""), None);
    }

    #[test]
    fn the_epoch_is_zero_in_both_directions() {
        assert_eq!(Moment::from_unix_nanos(0).to_unix_nanos(), 0);
    }

    #[test]
    fn a_day_later_is_the_next_day() {
        let moment = Moment::from_unix_nanos(86_400 * 1_000_000_000);
        assert_eq!(moment.to_string(), "1970-01-02T00:00:00.000Z");
    }

    #[test]
    fn the_calendar_handles_leap_days() {
        // 2024-02-29T12:34:56Z
        let moment = Moment::from_unix_nanos(1_709_210_096_000_000_000);
        assert_eq!(moment.year, 2024);
        assert_eq!(moment.month, 2);
        assert_eq!(moment.day, 29);
        assert_eq!(moment.hour, 12);
        assert_eq!(moment.minute, 34);
        assert_eq!(moment.second, 56);
    }

    #[test]
    fn milliseconds_are_kept() {
        let moment = Moment::from_unix_nanos(1_234_567_891_000_000);
        assert_eq!(moment.millisecond, 891);
        assert!(moment.to_string().ends_with(".891Z"));
    }

    /// The property that makes a directory listing useful: text order is time order.
    #[test]
    fn file_names_sort_in_the_order_the_matches_started() {
        let moments: Vec<Moment> = [
            0u64,
            1_000_000,
            999_000_000,
            1_000_000_000,
            86_399_999_000_000,
            86_400_000_000_000,
            1_700_000_000_000_000_000,
            1_709_210_096_000_000_000,
            4_102_444_800_000_000_000,
        ]
        .iter()
        .map(|&nanos| Moment::from_unix_nanos(nanos))
        .collect();

        let stems: Vec<String> = moments.iter().map(Moment::file_stem).collect();
        let mut sorted = stems.clone();
        sorted.sort();
        assert_eq!(stems, sorted, "text order diverged from time order");

        for pair in stems.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{} did not sort before {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn a_file_name_holds_nothing_a_file_system_rejects() {
        // Digits, hyphens and the date separator, and nothing else. That rules out every
        // character a file system reserves rather than listing them.
        for nanos in [0u64, 1_700_000_000_000_000_000, 4_102_444_800_000_000_000] {
            let stem = Moment::from_unix_nanos(nanos).file_stem();
            assert!(
                stem.chars()
                    .all(|c| c.is_ascii_digit() || c == '-' || c == 'T'),
                "{stem} holds something other than digits, hyphens and T"
            );
        }
    }

    #[test]
    fn a_whole_round_reads_as_the_log_it_should() {
        let events = vec![
            Event::Version {
                protocol: PROTOCOL_VERSION,
            },
            Event::Started {
                at: Moment::from_unix_nanos(1_700_000_000_000_000_000),
            },
            Event::Join {
                seat: Seat(0),
                name: named("Alice"),
                colour: named("red"),
                account: Account::Discord(named("450853396908343307")),
            },
            Event::Join {
                seat: Seat(1),
                name: named("Bob"),
                colour: named("blue"),
                account: Account::Ai,
            },
            Event::PlayerOrder(vec![Seat(0), Seat(1)]),
            Event::Variant {
                name: "ones-wild",
                on: false,
            },
            Event::Deal {
                seat: Seat(0),
                dice: vec![1, 3, 2, 6, 6],
            },
            Event::Claim {
                seat: Seat(0),
                quantity: 3,
                face: 6,
                odds: 38.207,
            },
            Event::Liar {
                seat: Seat(1),
                odds: 61.793,
            },
            Event::LoseDie { seat: Seat(1) },
            Event::NextTurn { seat: Seat(1) },
        ];
        let lines: Vec<String> = events.iter().map(Event::to_string).collect();
        assert_eq!(
            lines,
            vec![
                // Not the literal, so bumping the version is not a line to edit here.
                &format!("version {PROTOCOL_VERSION}"),
                "started 2023-11-14T22:13:20.000Z",
                "join #0 Alice red discord:450853396908343307",
                "join #1 Bob blue ai",
                "player-order #0 #1",
                "variant ones-wild off",
                "deal #0 1 3 2 6 6",
                "claim #0 3 6 38.21%",
                "claim #1 liar 61.79%",
                "lose-die #1",
                "next-turn #1",
            ]
        );
    }

    /// The test for `No value derives from a player's name`. A name belongs on a join
    /// line and nowhere else, so every argument of every other event must be a seat, a
    /// number, a percentage, or one of the two fixed claim keywords. A name field added
    /// to any variant later fails here.
    #[test]
    fn a_name_appears_on_a_join_line_and_on_no_other_line() {
        fn is_structural(argument: &str) -> bool {
            argument.starts_with('#')
                || argument == "liar"
                || argument == "spot-on"
                || argument.trim_end_matches('%').parse::<f64>().is_ok()
        }
        for event in every_player_event() {
            let line = event.to_string();
            if line.starts_with("join ") {
                continue;
            }
            for argument in line.split(' ').skip(1) {
                assert!(
                    is_structural(argument),
                    "{line} carries {argument}, which is neither a seat nor a number"
                );
            }
        }
    }

    #[test]
    fn every_event_that_refers_to_a_player_names_a_seat() {
        for event in every_player_event() {
            let line = event.to_string();
            assert!(
                line.contains('#'),
                "{line} refers to a player without naming a seat"
            );
        }
    }

    #[test]
    fn a_seat_is_distinguishable_from_a_quantity_or_a_face() {
        assert_eq!(Seat(3).to_string(), "#3");
        assert_eq!(
            Event::Claim {
                seat: Seat(3),
                quantity: 1,
                face: 3,
                odds: 50.0,
            }
            .to_string(),
            "claim #3 1 3 50.00%",
            "the seat must not read as one more bare number"
        );
    }

    #[test]
    fn the_three_kinds_of_account_read_differently() {
        let rendered: Vec<String> = [
            Account::Discord(named("450853396908343307")),
            Account::Ai,
            Account::Local,
        ]
        .iter()
        .map(Account::to_string)
        .collect();
        assert_eq!(rendered, vec!["discord:450853396908343307", "ai", "local"]);
    }

    #[test]
    fn the_three_kinds_of_claim_share_one_command() {
        assert_eq!(
            Event::Claim {
                seat: Seat(2),
                quantity: 5,
                face: 2,
                odds: 7.0,
            }
            .to_string(),
            "claim #2 5 2 7.00%"
        );
        assert_eq!(
            Event::Liar {
                seat: Seat(4),
                odds: 93.0
            }
            .to_string(),
            "claim #4 liar 93.00%"
        );
        assert_eq!(
            Event::SpotOn {
                seat: Seat(4),
                odds: 0.004
            }
            .to_string(),
            "claim #4 spot-on 0.00%"
        );
    }

    #[test]
    fn a_shield_token_gained_and_spent_each_read_as_their_own_line() {
        assert_eq!(
            Event::GainShield { seat: Seat(1) }.to_string(),
            "gain-shield #1"
        );
        assert_eq!(
            Event::SpendShield { seat: Seat(1) }.to_string(),
            "spend-shield #1"
        );
    }

    #[test]
    fn every_event_is_one_line_of_whitespace_separated_tokens() {
        let mut events = every_player_event();
        events.push(Event::Version { protocol: 1 });
        events.push(Event::Variant {
            name: "ones-wild",
            on: true,
        });
        events.push(Event::Join {
            seat: Seat(0),
            name: named("light blue player"),
            colour: named("light blue"),
            account: Account::Local,
        });
        for event in events {
            let line = event.to_string();
            assert!(!line.contains('\n'), "{line} spans more than one line");
            let count = line.split(' ').count();
            assert!(
                line.split_whitespace().count() == count,
                "{line} has runs of whitespace that change its token count"
            );
        }
    }

    /// A name with a space in it would otherwise split into two arguments and silently
    /// change what the line says.
    #[test]
    fn a_name_that_would_break_a_line_is_collapsed_instead() {
        assert_eq!(
            Event::Join {
                seat: Seat(0),
                name: named("Mary Anne"),
                colour: named("light blue"),
                account: Account::Local,
            }
            .to_string(),
            "join #0 Mary-Anne light-blue local"
        );
        assert_eq!(
            Event::Join {
                seat: Seat(1),
                name: named("   "),
                colour: named("red"),
                account: Account::Ai,
            }
            .to_string(),
            "join #1 - red ai"
        );
    }

    #[test]
    fn luck_reads_as_the_chance_of_what_happened() {
        assert_eq!(
            Event::Lucky {
                seat: Seat(4),
                odds: 1.2
            }
            .to_string(),
            "lucky #4 1.20%"
        );
        assert_eq!(
            Event::Unlucky {
                seat: Seat(0),
                odds: 3.704
            }
            .to_string(),
            "unlucky #0 3.70%"
        );
    }

    /// The commitment comes before any claim in the round and the reveal after it
    /// resolves. The format carries both as one token of hexadecimal.
    #[test]
    fn a_round_is_committed_to_and_then_revealed() {
        let digest = "a".repeat(64);
        let seed = "b".repeat(64);
        assert_eq!(
            Event::Commit {
                digest: digest.clone()
            }
            .to_string(),
            format!("commit {digest}")
        );
        assert_eq!(
            Event::Reveal { seed: seed.clone() }.to_string(),
            format!("reveal {seed}")
        );
    }

    #[test]
    fn a_deal_records_the_whole_seat_in_order() {
        assert_eq!(
            Event::Deal {
                seat: Seat(4),
                dice: vec![4, 5, 5, 1, 2]
            }
            .to_string(),
            "deal #4 4 5 5 1 2"
        );
    }
}
