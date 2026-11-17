//! Messages and the event log format.
//!
//! An event is one line of the log, and the line is the format — there is no second
//! representation to keep in step. Every field a reader needs to reconstruct what happened
//! is on the line, so /specification/architecture.md's requirement that all match state be
//! derivable from the event log holds for a file a person can read.
//!
//! Writing the log is an effect and belongs to a composition root. This crate only says
//! what an event is and how it reads.

use core::fmt;

/// One thing that happened, in the order it happened.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    MatchStarted {
        players: usize,
        /// Turn order, as player numbers.
        order: Vec<usize>,
        /// One colour name per player, by player number.
        colours: Vec<String>,
        dice_each: u32,
        ones_wild: bool,
    },
    /// All players roll. One of these per player per round.
    Rolled {
        player: usize,
        dice: Vec<u8>,
    },
    Claimed {
        player: usize,
        dice_held: u32,
        quantity: u32,
        face: u8,
    },
    /// A round-ending call, with how many dice actually counted toward the disputed claim.
    Called {
        player: usize,
        /// `liar` or `spot on`.
        call: &'static str,
        quantity: u32,
        face: u8,
        truth: u32,
    },
    LostDie {
        player: usize,
        remaining: u32,
    },
    GainedDie {
        player: usize,
        remaining: u32,
    },
    Eliminated {
        player: usize,
    },
    /// Finishing order, best first.
    MatchEnded {
        ranking: Vec<usize>,
    },
}

fn numbers(values: &[usize]) -> String {
    values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn faces(values: &[u8]) -> String {
    values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

impl fmt::Display for Event {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::MatchStarted {
                players,
                order,
                colours,
                dice_each,
                ones_wild,
            } => write!(
                out,
                "match-started players={players} order={} colours={} dice-each={dice_each} ones-wild={ones_wild}",
                numbers(order),
                colours.join(",")
            ),
            Event::Rolled { player, dice } => {
                write!(out, "rolled player={player} dice={}", faces(dice))
            }
            Event::Claimed {
                player,
                dice_held,
                quantity,
                face,
            } => write!(
                out,
                "claimed player={player} dice-held={dice_held} quantity={quantity} face={face}"
            ),
            Event::Called {
                player,
                call,
                quantity,
                face,
                truth,
            } => write!(
                out,
                "called player={player} call={call} quantity={quantity} face={face} truth={truth}"
            ),
            Event::LostDie { player, remaining } => {
                write!(out, "lost-die player={player} remaining={remaining}")
            }
            Event::GainedDie { player, remaining } => {
                write!(out, "gained-die player={player} remaining={remaining}")
            }
            Event::Eliminated { player } => write!(out, "eliminated player={player}"),
            Event::MatchEnded { ranking } => {
                write!(out, "match-ended ranking={}", numbers(ranking))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_reads_as_one_line() {
        let events = [
            Event::MatchStarted {
                players: 2,
                order: vec![1, 0],
                colours: vec!["red".into(), "navy".into()],
                dice_each: 5,
                ones_wild: false,
            },
            Event::Rolled {
                player: 0,
                dice: vec![3, 1, 6, 6, 2],
            },
            Event::Claimed {
                player: 1,
                dice_held: 5,
                quantity: 3,
                face: 6,
            },
            Event::Called {
                player: 0,
                call: "liar",
                quantity: 3,
                face: 6,
                truth: 2,
            },
            Event::LostDie {
                player: 1,
                remaining: 4,
            },
            Event::GainedDie {
                player: 0,
                remaining: 6,
            },
            Event::Eliminated { player: 1 },
            Event::MatchEnded {
                ranking: vec![0, 1],
            },
        ];
        for event in events {
            let line = event.to_string();
            assert!(!line.contains('\n'), "{line} spans more than one line");
            assert!(!line.is_empty());
        }
    }

    #[test]
    fn a_line_names_every_field_it_carries() {
        let line = Event::Claimed {
            player: 2,
            dice_held: 4,
            quantity: 3,
            face: 5,
        }
        .to_string();
        assert_eq!(line, "claimed player=2 dice-held=4 quantity=3 face=5");
    }

    #[test]
    fn a_roll_records_every_die() {
        let line = Event::Rolled {
            player: 0,
            dice: vec![1, 2, 3],
        }
        .to_string();
        assert!(line.ends_with("dice=1,2,3"));
    }
}
