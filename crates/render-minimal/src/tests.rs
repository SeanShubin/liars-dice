//! What a panel may draw, asserted over a whole match rather than at one moment.
//!
//! There are no pixels here. A panel is a function of what it was handed, so the things
//! worth testing are properties of that: which dice are reachable from the view a screen
//! is given, and which claims a dialler can produce. Both are the statements the
//! presentation is answerable for, and neither needs a window to check.

use integrity::Secret;
use rules::{Call, Claim, Claimed, Game, SEAT, Variants};
use std::collections::BTreeSet;

use crate::{opening_claim, steps_from};

const SEATS: usize = 3;
/// Enough states that a passing assertion is evidence rather than a coincidence.
const ENOUGH: usize = 20;

fn deal_round(game: &mut Game, secret: &Secret, round: u32) {
    let seed = integrity::seed_for(secret, "render-minimal", round);
    for seat in 0..SEATS {
        let dice = integrity::deal(&seed, seat as u8, SEAT);
        let _ = game.install(
            seat,
            <[u8; SEAT]>::try_from(dice.as_slice()).expect("the dice limit"),
        );
    }
}

/// Play a whole match with AI in every seat, showing the witness every state it passes
/// through — including each round's reveal, which is the state between `resolve` and
/// `next_round` that the reveal screen draws.
fn play_a_match(mut witness: impl FnMut(&Game)) {
    let secret = Secret::from_bytes([7; 32]);
    let mut game = Game::start((0..SEATS).collect(), Variants::default());
    deal_round(&mut game, &secret, 0);

    let mut guard = 0;
    loop {
        guard += 1;
        assert!(guard < 5_000, "the match would not finish");
        witness(&game);
        if game.public().over() {
            return;
        }
        let seat = game.public().current();
        let call = ai::choose(&game.view(seat));
        match game.claim(call) {
            Ok(Claimed::Continues) => {}
            Ok(Claimed::RoundEnded) => {
                let resolution = game.resolve().expect("every die is known");
                // The reveal state: settled, every die visible, the claims still there.
                witness(&game);
                if resolution.over {
                    return;
                }
                game.next_round(resolution.opens);
                let round = game.public().round();
                deal_round(&mut game, &secret, round);
            }
            Err(reason) => panic!("the ai offered an illegal call: {reason:?}"),
        }
    }
}

/// The leak test. Not that the panels are careful, but that the dice they would have to
/// draw to leak are not reachable from what they are given.
#[test]
fn a_seat_cannot_reach_another_seats_dice_before_the_reveal() {
    let mut checked = 0;
    play_a_match(|game| {
        if game.revealed() {
            return;
        }
        checked += 1;
        for seat in 0..SEATS {
            let view = game.view(seat);
            for other in 0..SEATS {
                if other == seat {
                    continue;
                }
                assert_eq!(
                    view.dice_of(other),
                    None,
                    "seat {seat} could see seat {other}'s dice mid-round"
                );
            }
        }
    });
    assert!(
        checked > ENOUGH,
        "only {checked} live states, too few to be evidence"
    );
}

/// `Resolving a round reveals every player's dice` — the other half, without which the
/// first test would also pass on a view that showed nobody anything.
#[test]
fn every_seat_reaches_every_seats_dice_once_the_round_is_revealed() {
    let mut revealed = 0;
    play_a_match(|game| {
        if !game.revealed() {
            return;
        }
        revealed += 1;
        for seat in 0..SEATS {
            let view = game.view(seat);
            for other in 0..SEATS {
                assert!(
                    view.dice_of(other).is_some(),
                    "seat {seat} could not see seat {other} after the reveal"
                );
            }
        }
    });
    assert!(revealed > 0, "no round was revealed");
}

/// Both halves of `The local player can reach every legal claim` and `An invalid claim
/// cannot be made`, over every standing claim a real match produced.
#[test]
fn the_dialler_reaches_every_legal_claim_and_offers_no_illegal_one() {
    let mut states = 0;
    play_a_match(|game| {
        if game.public().over() || game.revealed() {
            return;
        }
        let view = game.view(game.public().current());
        let legal: BTreeSet<(u32, u8)> = view
            .legal_raises()
            .iter()
            .map(|claim| (claim.quantity, claim.face))
            .collect();
        if legal.is_empty() {
            return;
        }
        states += 1;

        let start = opening_claim(&view);
        assert!(
            legal.contains(&(start.quantity, start.face)),
            "the dialler starts on {start:?}, which is not legal"
        );

        // Walk everywhere the steppers can go, checking each landing is legal.
        let mut reached = BTreeSet::from([(start.quantity, start.face)]);
        let mut pending = vec![start];
        while let Some(at) = pending.pop() {
            let steps = steps_from(&view, at);
            let mut go = |claim: Claim| {
                assert!(
                    legal.contains(&(claim.quantity, claim.face)),
                    "the dialler offered {claim:?}, which is not legal"
                );
                if reached.insert((claim.quantity, claim.face)) {
                    pending.push(claim);
                }
            };
            if steps.quantity_down {
                go(Claim {
                    quantity: at.quantity - 1,
                    ..at
                });
            }
            if steps.quantity_up {
                go(Claim {
                    quantity: at.quantity + 1,
                    ..at
                });
            }
            if steps.face_down {
                go(Claim {
                    face: at.face - 1,
                    ..at
                });
            }
            if steps.face_up {
                go(Claim {
                    face: at.face + 1,
                    ..at
                });
            }
        }

        assert_eq!(
            reached, legal,
            "the dialler could not reach every legal claim"
        );
    });
    assert!(
        states > ENOUGH,
        "only {states} standing claims, too few to be evidence"
    );
}

/// A liar call is always available once there is something to dispute, and never before.
/// The calls panel reads this off the view, so it is the view's answer that matters.
#[test]
fn liar_and_spot_on_are_available_exactly_when_there_is_a_claim_to_dispute() {
    play_a_match(|game| {
        if game.public().over() || game.revealed() {
            return;
        }
        let public = game.public();
        let disputable = public.previous_claim().is_some();
        assert_eq!(
            public.check(Call::Liar).is_ok(),
            disputable,
            "liar did not match whether there was a claim"
        );
        assert_eq!(
            public.check(Call::SpotOn).is_ok(),
            disputable,
            "spot on did not match whether there was a claim"
        );
    });
}
