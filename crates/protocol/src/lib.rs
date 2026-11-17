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
//! Writing the log is an effect and belongs to a composition root. This crate only says
//! what an event is and how it reads.

use core::fmt;

/// One thing that happened, in the order it happened.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
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
    Claim {
        player: String,
        quantity: u32,
        face: u8,
    },
    /// A claim that the previous claim is false.
    Liar {
        player: String,
    },
    /// A claim that the previous claim is exact.
    SpotOn {
        player: String,
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
            } => write!(out, "claim {} {quantity} {face}", token(player)),
            Event::Liar { player } => write!(out, "claim {} liar", token(player)),
            Event::SpotOn { player } => write!(out, "claim {} spot-on", token(player)),
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
    fn a_whole_round_reads_as_the_log_it_should() {
        let events = vec![
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
            },
            Event::Liar {
                player: named("Bob"),
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
                "add-player Alice red",
                "player-order Alice Bob",
                "variant ones-wild off",
                "roll Alice 1 3 2 6 6",
                "claim Alice 3 6",
                "claim Bob liar",
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
                face: 2
            }
            .to_string(),
            "claim Carol 5 2"
        );
        assert_eq!(
            Event::Liar {
                player: named("Dave")
            }
            .to_string(),
            "claim Dave liar"
        );
        assert_eq!(
            Event::SpotOn {
                player: named("Dave")
            }
            .to_string(),
            "claim Dave spot-on"
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
