//! A whole networked match, driven and checked in one process.
//!
//! These are the tests /implementation/coordinator.md asks for. Every one of them goes
//! through the real relay, the real protocol and the real commitments; the only thing
//! simulated is the transport.

use super::*;
use integrity::Secret;
use integrity::{Commitment, Seed};
use protocol::{Event, Seat};
use rules::SEAT;

fn secret() -> Secret {
    Secret::from_bytes([11u8; 32])
}

fn ai_table(players: usize) -> Table {
    let names = ["Alice", "Bob", "Carol", "Dave", "Erin"];
    Table::seat(
        secret(),
        (0..players)
            .map(|at| (names[at].to_string(), Box::new(Ai) as Box<dyn Decides>))
            .collect(),
    )
}

/// Check an invariant after every turn of a whole match.
fn play_checking(table: &mut Table, limit: usize, check: &mut impl FnMut(&Table)) -> usize {
    let mut turns = 0;
    check(table);
    while turns < limit && table.take_turn() {
        check(table);
        turns += 1;
    }
    turns
}

#[test]
fn a_whole_match_is_played_over_the_relay_and_ends_with_one_survivor() {
    let mut table = ai_table(3);
    table.play(5_000);
    assert!(table.over(), "the match did not end");
    let public = table.players[0].client.public().expect("a started match");
    assert_eq!(public.alive().len(), 1);
    assert_eq!(public.ranking().len(), 3);
    for player in &table.players {
        assert!(
            player.client.complaints().is_empty(),
            "a client found the log disagreeing with itself: {:?}",
            player.client.complaints()
        );
    }
}

/// Every client derives the same match state from the same events. Three clients that
/// each saw only their own dice still agree about everything public.
#[test]
fn every_client_agrees_about_everything_public() {
    let mut table = ai_table(3);
    play_checking(&mut table, 5_000, &mut |table| {
        let first = table.players[0].client.public().cloned();
        for player in &table.players[1..] {
            assert_eq!(
                player.client.public().cloned(),
                first,
                "two clients disagree about public state"
            );
        }
    });
}

/// The secrecy property, asserted over every turn of a whole match rather than at one
/// moment: a client never holds another seat's dice before that round is revealed.
#[test]
fn a_client_never_sees_another_seat_before_the_reveal() {
    let mut table = ai_table(3);
    play_checking(&mut table, 5_000, &mut |table| {
        for player in &table.players {
            let Some(view) = player.client.view() else {
                continue;
            };
            if player.client.game().is_some_and(rules::Game::revealed) {
                continue;
            }
            let mine = player.client.seat().0 as usize;
            for seat in 0..view.public().players() {
                if seat != mine {
                    assert_eq!(
                        view.dice_of(seat),
                        None,
                        "seat {mine} could see seat {seat} before the reveal"
                    );
                }
            }
        }
    });
}

/// Deterministic from one secret: the same table plays the same match twice.
#[test]
fn the_same_secret_plays_the_same_match() {
    let mut first = ai_table(3);
    first.play(5_000);
    let mut second = ai_table(3);
    second.play(5_000);

    let left: Vec<String> = first.log().iter().map(Event::to_string).collect();
    let right: Vec<String> = second.log().iter().map(Event::to_string).collect();
    assert_eq!(left, right, "the same secret produced a different match");
    assert!(left.len() > 50, "a whole match is more than a few lines");
}

/// A seat's behaviour is a property of its driver and of nothing else. A scripted driver
/// and an AI share a table and neither is treated differently by anything.
#[test]
fn a_scripted_driver_and_an_ai_share_a_table() {
    let mut table = Table::seat(
        secret(),
        vec![
            (
                "Scripted".to_string(),
                Box::new(Scripted {
                    calls: vec![
                        Call::Raise(rules::Claim {
                            quantity: 1,
                            face: 2,
                        }),
                        Call::Raise(rules::Claim {
                            quantity: 2,
                            face: 2,
                        }),
                        Call::Liar,
                    ],
                    at: 0,
                }) as Box<dyn Decides>,
            ),
            ("Alice".to_string(), Box::new(Ai)),
            ("Bob".to_string(), Box::new(Ai)),
        ],
    );
    table.play(5_000);
    assert!(table.over());
    // The log records a seat, a name and an account. It records nothing about what chose
    // the calls, which is the whole reason a virtual player proves anything.
    let text: Vec<String> = table.log().iter().map(Event::to_string).collect();
    assert!(
        !text.iter().any(|line| line.contains("script")),
        "the driver leaked into the log"
    );
}

/// Three clients in one process, each driven separately, which is what a hotseat match
/// is underneath. See /implementation/hotseat.md for the screen over it.
#[test]
fn several_clients_in_one_process_is_hotseat() {
    let mut table = Table::seat(
        secret(),
        (0..3)
            .map(|at| {
                (
                    format!("Seat{at}"),
                    Box::new(Scripted {
                        calls: Vec::new(),
                        at: 0,
                    }) as Box<dyn Decides>,
                )
            })
            .collect(),
    );
    table.play(5_000);
    assert!(table.over(), "three scripted seats played a whole match");
}

/// A client refuses to send an illegal call rather than sending it.
#[test]
fn a_client_will_not_send_an_illegal_call() {
    let mut table = ai_table(2);
    let seat = table.players[0].client.public().expect("started").current();
    let player = &mut table.players[seat];
    // Spot on with nothing on the table refers to a claim that does not exist.
    assert!(
        player.client.act(Call::SpotOn, 0.0).is_err(),
        "a client must refuse an illegal call at the point it is made"
    );
}

/// And ignores one delivered to it, leaving its state untouched.
#[test]
fn a_client_ignores_an_illegal_claim_it_receives() {
    let mut table = ai_table(3);
    table.take_turn();
    let before = table.players[1].client.public().cloned();
    // A claim from a seat whose turn it is not.
    let out_of_turn = Event::Liar {
        seat: Seat(99),
        odds: 50.0,
    };
    let intents = table.players[1].client.receive(&out_of_turn);
    assert!(intents.is_empty(), "an illegal claim asked for something");
    assert_eq!(
        table.players[1].client.public().cloned(),
        before,
        "an illegal claim changed a client's state"
    );
}

/// The log verifies from itself: every round's seed matches the commitment published
/// before any claim in it, and recomputes every face on every deal line.
#[test]
fn the_match_log_verifies_from_itself() {
    let mut table = ai_table(3);
    table.play(5_000);

    struct Round {
        digest: String,
        seed: Option<String>,
        deals: Vec<(u8, Vec<u8>)>,
    }
    let mut rounds: Vec<Round> = Vec::new();
    for event in table.log() {
        match event {
            Event::Commit { digest } => rounds.push(Round {
                digest,
                seed: None,
                deals: Vec::new(),
            }),
            Event::Deal { seat, dice } => {
                if let Some(round) = rounds.last_mut() {
                    round.deals.push((seat.0, dice));
                }
            }
            Event::Reveal { seed } => {
                if let Some(round) = rounds.iter_mut().rev().find(|round| round.seed.is_none()) {
                    round.seed = Some(seed);
                }
            }
            _ => {}
        }
    }

    assert!(rounds.len() > 1, "a match takes more than one round");
    let mut checked = 0;
    for (number, round) in rounds.iter().enumerate() {
        let Some(text) = round.seed.as_ref() else {
            // The last round of a finished match ends with a winner rather than a new
            // deal, so a trailing unrevealed round is possible.
            continue;
        };
        let seed = Seed::from_hex(text).expect("a seed");
        let digest = Commitment::from_hex(&round.digest).expect("a digest");
        assert!(
            integrity::verify(&digest, &seed),
            "round {number}: the revealed seed is not the one committed to"
        );
        for (seat, faces) in &round.deals {
            assert_eq!(
                integrity::deal(&seed, *seat, SEAT),
                *faces,
                "round {number} seat {seat}: the log disagrees with its own seed"
            );
        }
        checked += 1;
    }
    assert!(checked > 0, "no round was verified");
}

/// Turn order is chosen at random when the match starts, and recorded so a replay knows
/// it. Two matches under different secrets should not always agree.
#[test]
fn turn_order_is_recorded_and_depends_on_the_match() {
    let order_of = |secret: Secret| -> Vec<Seat> {
        let table = Table::seat(
            secret,
            (0..5)
                .map(|at| {
                    (
                        format!("Seat{at}"),
                        Box::new(Scripted {
                            calls: Vec::new(),
                            at: 0,
                        }) as Box<dyn Decides>,
                    )
                })
                .collect(),
        );
        table
            .log()
            .into_iter()
            .find_map(|event| match event {
                Event::PlayerOrder(order) => Some(order),
                _ => None,
            })
            .expect("a match records its turn order")
    };
    let first = order_of(Secret::from_bytes([1u8; 32]));
    let second = order_of(Secret::from_bytes([2u8; 32]));
    assert_eq!(first.len(), 5);
    let mut sorted: Vec<u8> = first.iter().map(|seat| seat.0).collect();
    sorted.sort();
    assert_eq!(sorted, vec![0, 1, 2, 3, 4], "every seat is ordered once");
    assert_ne!(first, second, "two matches ordered identically");
}

/// A driver with no answer, which is what a person is until they tap something.
struct Waits;

impl Decides for Waits {
    fn choose(&mut self, _view: &View) -> Option<Call> {
        None
    }
}

fn table_with_a_person_in_seat(seat: usize, players: usize) -> Table {
    let names = ["Alice", "Bob", "Carol", "Dave", "Erin"];
    Table::seat(
        secret(),
        (0..players)
            .map(|at| {
                let driver: Box<dyn Decides> = if at == seat {
                    Box::new(Waits)
                } else {
                    Box::new(Ai)
                };
                (names[at].to_string(), driver)
            })
            .collect(),
    )
}

/// Stepping stops at the seat that has to be asked, and asking changes nothing. A screen
/// draws between these two facts, so a stop that quietly advanced the match would show a
/// player a position they never had.
#[test]
fn advancing_stops_at_a_seat_with_no_answer_and_changes_nothing_by_stopping() {
    let mut table = table_with_a_person_in_seat(0, 3);

    let waiting = table.advance();
    assert_eq!(
        waiting,
        Waiting::Call(0),
        "the table should be waiting on the seat nobody is driving"
    );

    let log = table.log();
    let public = table.players[0]
        .client
        .public()
        .expect("the match has started")
        .clone();
    for _ in 0..5 {
        assert_eq!(
            table.advance(),
            Waiting::Call(0),
            "the answer should not move"
        );
    }
    assert_eq!(table.log(), log, "stopping wrote to the log");
    assert_eq!(
        table.players[0].client.public(),
        Some(&public),
        "stopping changed the match"
    );
}

/// The state a reveal screen is drawn from: settled, so every die is visible through every
/// view, and not yet left behind, so the round's claims are still there to list.
#[test]
fn advancing_stops_on_a_settled_round_with_its_dice_and_claims_still_readable() {
    let mut table = table_with_a_person_in_seat(0, 3);
    let mut guard = 0;
    let resolution = loop {
        guard += 1;
        assert!(guard < 500, "no round resolved");
        match table.advance() {
            Waiting::Outcome(resolution) => break resolution,
            // The person has to claim for the round to get anywhere, so take the lowest
            // raise available and carry on.
            Waiting::Call(at) => {
                let call = {
                    let view = table.players[at].client.view().expect("a view");
                    match view.legal_raises().first() {
                        Some(&claim) => Call::Raise(claim),
                        None => Call::Liar,
                    }
                };
                let intent = table.players[at]
                    .client
                    .act(call, 0.0)
                    .expect("the call was offered as legal");
                let sends = table.send(at, intent);
                table.deliver(sends);
            }
            Waiting::Over => panic!("the match ended before a round resolved"),
        }
    };

    for player in &table.players {
        let view = player.client.view().expect("a view");
        assert!(
            !view.public().history().is_empty(),
            "the round's claims were cleared before the outcome was shown"
        );
        for seat in 0..table.players.len() {
            assert!(
                view.dice_of(seat).is_some(),
                "seat {} could not see seat {seat} on a settled round",
                player.client.seat().0
            );
        }
    }

    // And it keeps saying so until the round is left behind.
    assert_eq!(table.advance(), Waiting::Outcome(resolution));
}

/// Resuming is what leaves a settled round behind, and it begins the round the resolution
/// named rather than one of the table's choosing.
#[test]
fn resuming_begins_the_round_the_resolution_named() {
    let mut table = ai_table(3);
    // Every seat is driven, so the first thing the table stops for is a round ending.
    let resolution = match table.advance() {
        Waiting::Outcome(resolution) => resolution,
        other => panic!("expected a round to resolve, got {other:?}"),
    };

    let before = table.players[0].client.public().expect("started").round();
    table.resume();
    let after = table.players[0].client.public().expect("started");

    assert_eq!(after.round(), before + 1, "resuming did not begin a round");
    assert_eq!(
        after.current(),
        resolution.opens,
        "the wrong seat opened the round"
    );
    assert!(
        after.history().is_empty(),
        "the new round began with the old round's claims"
    );
}

/// Stepping and resuming play the same match as the one-call-at-a-time loop, event for
/// event. The two ways of driving a table are two shapes of the same thing, which is what
/// lets a screen use one and a test use the other.
#[test]
fn stepping_plays_the_same_match_as_taking_turns() {
    let mut stepped = ai_table(3);
    let mut guard = 0;
    loop {
        guard += 1;
        assert!(guard < 5_000, "the stepped match would not finish");
        match stepped.advance() {
            Waiting::Outcome(_) => stepped.resume(),
            Waiting::Over => break,
            Waiting::Call(at) => panic!("an ai seat had no answer at seat {at}"),
        }
    }

    let mut turned = ai_table(3);
    turned.play(5_000);

    assert!(stepped.over() && turned.over(), "both matches should end");
    assert_eq!(
        stepped.log(),
        turned.log(),
        "the two ways of driving a table disagreed"
    );
}

/// Every round is played with dice, not just the first.
///
/// The relay reveals a round and opens the next in one breath, so the next round's deal
/// arrives while the client is still showing the round that ended. It used to be dropped —
/// refused by the round in progress, then forgotten by `next_round` — and every round
/// after the first was played by players who could not see their own dice. The AI claimed
/// from an empty hand, the screen had nothing to draw, and the match still finished,
/// because the reveal at the end of each round installed the dice it had been missing.
/// Nothing failed. It was only wrong.
#[test]
fn every_round_is_dealt_before_it_is_played() {
    let mut table = ai_table(3);
    let mut rounds = 0;
    let mut guard = 0;

    loop {
        guard += 1;
        assert!(guard < 5_000, "the match would not finish");
        match table.advance() {
            Waiting::Over => break,
            Waiting::Call(at) => panic!("an ai seat had no answer at seat {at}"),
            Waiting::Outcome(_) => {
                table.resume();
                if table.over() {
                    break;
                }
                rounds += 1;
                for player in &table.players {
                    let seat = player.client.seat().0 as usize;
                    let view = player.client.view().expect("a view");
                    if view.public().eliminated(seat) {
                        continue;
                    }
                    // Its own dice, and only its own: a client is not sent another seat's
                    // until the reveal, so `dice_known` is false here and should be.
                    let own = view.own_dice().unwrap_or(&[]);
                    assert_eq!(
                        own.len(),
                        view.public().count(seat) as usize,
                        "seat {seat} cannot see the dice it holds in round {}",
                        view.public().round()
                    );
                }
            }
        }
    }

    assert!(rounds > 3, "only {rounds} rounds, too few to be evidence");
}

/// And every round's reveal is checked against the commitment published before it.
///
/// The commitment for the next round arrives with that round's deals, which is before the
/// last round is left behind — so clearing it when the next round begins threw away the
/// only thing the next reveal could be checked against. A tampered seed would have been
/// believed from round two onwards.
#[test]
fn a_tampered_seed_is_caught_in_a_later_round_too() {
    let mut table = ai_table(3);
    // Get past the first round, so the commitment under test is one that arrived while an
    // earlier round was still being shown.
    let mut rounds = 0;
    while rounds < 2 {
        match table.advance() {
            Waiting::Outcome(_) => {
                table.resume();
                rounds += 1;
            }
            other => panic!("the match ended early: {other:?}"),
        }
    }

    let client = &mut table.players[0].client;
    assert!(
        client.complaints().is_empty(),
        "an honest match should raise no complaint"
    );
    // A seed that is not the one this round committed to.
    client.receive(&Event::Reveal {
        seed: "ab".repeat(32),
    });
    assert!(
        client
            .complaints()
            .iter()
            .any(|wrong| matches!(wrong, Wrong::SeedDoesNotMatchCommitment)),
        "a seed that does not match the commitment went unnoticed in round {rounds}"
    );
}
