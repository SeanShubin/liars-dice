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
    let day_of_era = (shifted - era * 146_097) as i64;
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

/// One thing that happened, in the order it happened.
///
/// Not `Eq`, because the odds on a claim are a float. That is a fair warning: a float does
/// not belong in anything compared for equality, and these are only ever written out.
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    /// When the match started, in UTC.
    Started {
        at: Moment,
    },
    /// A player joins, with the colour that identifies them.
    AddPlayer {
        name: String,
        colour: String,
    },
    /// Turn order, which is chosen at random when the match starts.
    PlayerOrder(Vec<String>),
    /// One per variant, so a replay knows which rules were in force.
    Variant {
        name: &'static str,
        on: bool,
    },
    /// All players roll. One of these per player per round.
    Roll {
        player: String,
        dice: Vec<u8>,
    },
    /// A raise: at least this quantity of this face.
    ///
    /// `odds` is the percentage chance the claim is true, from what the claimer could see
    /// when they made it.
    Claim {
        player: String,
        quantity: u32,
        face: u8,
        odds: f64,
    },
    /// A claim that the previous claim is false. `odds` is the chance that is so.
    Liar {
        player: String,
        odds: f64,
    },
    /// A claim that the previous claim is exact. `odds` is the chance that is so.
    SpotOn {
        player: String,
        odds: f64,
    },
    LoseDie {
        player: String,
    },
    GainDie {
        player: String,
    },
    Eliminate {
        player: String,
    },
    /// A claim that came true against the odds. `odds` is how likely what happened was.
    ///
    /// Derivable from the claim's own odds and the resolution, and written anyway: a run
    /// of these is how a reader sees that a player has been riding their luck.
    Lucky {
        player: String,
        odds: f64,
    },
    /// A claim that failed when it should have held. `odds` is how likely what happened
    /// was.
    Unlucky {
        player: String,
        odds: f64,
    },
    /// Who makes the first claim of the next round.
    NextTurn {
        player: String,
    },
    /// The last player remaining. Derivable, and written anyway: it is how a reader knows
    /// the log is a whole match rather than a truncated one.
    Winner {
        player: String,
    },
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

fn tokens(values: &[String]) -> String {
    values
        .iter()
        .map(|value| token(value))
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
            Event::Started { at } => write!(out, "started {at}"),
            Event::AddPlayer { name, colour } => {
                write!(out, "add-player {} {}", token(name), token(colour))
            }
            Event::PlayerOrder(order) => write!(out, "player-order {}", tokens(order)),
            Event::Variant { name, on } => {
                write!(
                    out,
                    "variant {} {}",
                    token(name),
                    if *on { "on" } else { "off" }
                )
            }
            Event::Roll { player, dice } => {
                write!(out, "roll {} {}", token(player), faces(dice))
            }
            Event::Claim {
                player,
                quantity,
                face,
                odds,
            } => write!(
                out,
                "claim {} {quantity} {face} {}",
                token(player),
                percent(*odds)
            ),
            Event::Liar { player, odds } => {
                write!(out, "claim {} liar {}", token(player), percent(*odds))
            }
            Event::SpotOn { player, odds } => {
                write!(out, "claim {} spot-on {}", token(player), percent(*odds))
            }
            Event::Lucky { player, odds } => {
                write!(out, "lucky {} {}", token(player), percent(*odds))
            }
            Event::Unlucky { player, odds } => {
                write!(out, "unlucky {} {}", token(player), percent(*odds))
            }
            Event::LoseDie { player } => write!(out, "lose-die {}", token(player)),
            Event::GainDie { player } => write!(out, "gain-die {}", token(player)),
            Event::Eliminate { player } => write!(out, "eliminate {}", token(player)),
            Event::NextTurn { player } => write!(out, "next-turn {}", token(player)),
            Event::Winner { player } => write!(out, "winner {}", token(player)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> String {
        name.to_string()
    }

    #[test]
    fn the_epoch_reads_as_itself() {
        let moment = Moment::from_unix_nanos(0);
        assert_eq!(moment.to_string(), "1970-01-01T00:00:00.000Z");
        assert_eq!(moment.file_stem(), "1970-01-01T00-00-00-000");
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
            Event::Started {
                at: Moment::from_unix_nanos(1_700_000_000_000_000_000),
            },
            Event::AddPlayer {
                name: named("Alice"),
                colour: named("red"),
            },
            Event::PlayerOrder(vec![named("Alice"), named("Bob")]),
            Event::Variant {
                name: "ones-wild",
                on: false,
            },
            Event::Roll {
                player: named("Alice"),
                dice: vec![1, 3, 2, 6, 6],
            },
            Event::Claim {
                player: named("Alice"),
                quantity: 3,
                face: 6,
                odds: 38.207,
            },
            Event::Liar {
                player: named("Bob"),
                odds: 61.793,
            },
            Event::LoseDie {
                player: named("Bob"),
            },
            Event::NextTurn {
                player: named("Bob"),
            },
        ];
        let lines: Vec<String> = events.iter().map(Event::to_string).collect();
        assert_eq!(
            lines,
            vec![
                "started 2023-11-14T22:13:20.000Z",
                "add-player Alice red",
                "player-order Alice Bob",
                "variant ones-wild off",
                "roll Alice 1 3 2 6 6",
                "claim Alice 3 6 38.21%",
                "claim Bob liar 61.79%",
                "lose-die Bob",
                "next-turn Bob",
            ]
        );
    }

    #[test]
    fn the_three_kinds_of_claim_share_one_command() {
        assert_eq!(
            Event::Claim {
                player: named("Carol"),
                quantity: 5,
                face: 2,
                odds: 7.0,
            }
            .to_string(),
            "claim Carol 5 2 7.00%"
        );
        assert_eq!(
            Event::Liar {
                player: named("Dave"),
                odds: 93.0
            }
            .to_string(),
            "claim Dave liar 93.00%"
        );
        assert_eq!(
            Event::SpotOn {
                player: named("Dave"),
                odds: 0.004
            }
            .to_string(),
            "claim Dave spot-on 0.00%"
        );
    }

    #[test]
    fn every_event_is_one_line_of_whitespace_separated_tokens() {
        let events = vec![
            Event::AddPlayer {
                name: named("light blue player"),
                colour: named("light blue"),
            },
            Event::PlayerOrder(vec![named("dark green"), named("Bob")]),
            Event::Roll {
                player: named("a b"),
                dice: vec![1],
            },
            Event::GainDie {
                player: named("Dave"),
            },
            Event::Eliminate {
                player: named("Dave"),
            },
            Event::Winner {
                player: named("Alice"),
            },
        ];
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
            Event::AddPlayer {
                name: named("Mary Anne"),
                colour: named("light blue"),
            }
            .to_string(),
            "add-player Mary-Anne light-blue"
        );
        assert_eq!(
            Event::NextTurn {
                player: named("   ")
            }
            .to_string(),
            "next-turn -"
        );
    }

    #[test]
    fn luck_reads_as_the_chance_of_what_happened() {
        assert_eq!(
            Event::Lucky {
                player: named("Dave"),
                odds: 1.2
            }
            .to_string(),
            "lucky Dave 1.20%"
        );
        assert_eq!(
            Event::Unlucky {
                player: named("Alice"),
                odds: 3.704
            }
            .to_string(),
            "unlucky Alice 3.70%"
        );
    }

    #[test]
    fn a_roll_records_every_die_in_order() {
        assert_eq!(
            Event::Roll {
                player: named("Dave"),
                dice: vec![4, 5, 5, 1, 2]
            }
            .to_string(),
            "roll Dave 4 5 5 1 2"
        );
    }
}
