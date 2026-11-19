//! The screens' panels, each one drawn from a single seat's view of the match.
//!
//! Every panel here takes a [`View`] and a [`Seating`] and nothing else. That is the whole
//! safety argument for showing a match on a shared screen: `View::dice_of` returns `None`
//! for a seat that is not its own until the round is revealed, so the dice a panel must not
//! draw are not reachable from what it was handed. A panel cannot leak by being written
//! carelessly, only by being handed the wrong view.

use ai::Seen;
use egui::{Grid, RichText, Ui};
use rules::{Call, Claim, Public, Resolution, View};

use crate::seating::Seating;
use crate::widget::{claim_box, dialled_value, die, shield, tap};

/// Every seat and the dice it holds, in a grid so the columns line up across seats.
///
/// A die this seat may not see is drawn as an unrevealed box rather than left out, so
/// `How many dice each player holds is shown` without saying what they are.
///
/// Shield tokens take a column of their own, before the dice, and only while somebody at
/// the table holds one. That is `How many shield tokens a player holds is shown only when
/// they hold at least one`, decided for the whole table rather than per seat so every row
/// has the same columns and the dice stay aligned across seats.
pub fn table(ui: &mut Ui, view: &View, seating: &Seating) {
    let public = view.public();
    let turn = (!public.over()).then(|| public.current());
    let shielded = any_shields(public);
    Grid::new("table").spacing([5.0, 5.0]).show(ui, |ui| {
        // Turn order rather than seat number, so who attacks whom is visible.
        for &seat in public.order() {
            let out = public.eliminated(seat);
            let mut name = RichText::new(seating.name(seat)).size(15.0);
            if out {
                name = name.strikethrough();
            }
            if turn == Some(seat) {
                name = name.strong();
            }
            ui.label(name);
            ui.label(if turn == Some(seat) {
                RichText::new("turn").size(13.0).strong()
            } else {
                RichText::new("")
            });

            if shielded {
                shield(ui, seating.colour(seat), public.shields(seat));
            }

            let held = public.count(seat) as usize;
            if held == 0 && out {
                ui.label(RichText::new("out").size(14.0).weak());
            }
            // `None` for a die this seat may not see. The faces, when there are any, are
            // already sliced to the count the seat holds.
            let faces = view.dice_of(seat);
            for index in 0..held {
                die(
                    ui,
                    seating.colour(seat),
                    faces.and_then(|dice| dice.get(index).copied()),
                );
            }
            ui.end_row();
        }
    });
}

/// Whether anybody at the table holds a shield token.
///
/// What decides the table's shield column. A function of the public counts and nothing
/// else, so every row of a draw gets the same answer.
pub fn any_shields(public: &Public) -> bool {
    public.order().iter().any(|&seat| public.shields(seat) > 0)
}

/// Whose turn it is, what there is to beat, and how many dice are in play.
pub fn standing(ui: &mut Ui, view: &View, seating: &Seating) {
    let public = view.public();
    let whose = if public.over() {
        "match over".to_string()
    } else if public.current() == view.seat() {
        "Your turn".to_string()
    } else {
        format!("{}'s turn", seating.name(public.current()))
    };
    let beat = match public.previous_claim() {
        Some(claim) => format!("to beat: at least {} x {}", claim.quantity, claim.face),
        None => "nothing to beat yet".to_string(),
    };
    ui.label(
        RichText::new(format!(
            "{whose}   -   {beat}   -   {} dice in play",
            public.dice_in_play()
        ))
        .size(16.0),
    );
}

/// The chance that each claim this seat could make is true.
///
/// Three rows, one per button. The standing claim's own chance is shown rather than the
/// chance that calling liar is right, because those are complements and the claim is the
/// thing on the table — but it is the number the liar decision turns on.
pub fn odds(ui: &mut Ui, view: &View, dialled: Claim) {
    let previous = view.previous_claim();
    ui.label(RichText::new("Odds").strong());
    Grid::new("odds").spacing([10.0, 4.0]).show(ui, |ui| {
        let row = |ui: &mut Ui, what: &str, claim: String, chance: Option<f64>| {
            ui.label(RichText::new(what).size(15.0));
            ui.label(RichText::new(claim).size(15.0));
            ui.label(
                RichText::new(match chance {
                    Some(percent) => format!("{percent:.1}%"),
                    None => "-".to_string(),
                })
                .size(16.0)
                .strong(),
            );
            ui.end_row();
        };

        match previous {
            Some(claim) => row(
                ui,
                "their claim",
                format!("at least {} x {}", claim.quantity, claim.face),
                Some(readout::percent(&view.chance_of(claim))),
            ),
            None => row(ui, "their claim", "none yet".to_string(), None),
        }

        row(
            ui,
            "your raise",
            format!("at least {} x {}", dialled.quantity, dialled.face),
            Some(readout::percent(&view.chance_of(dialled))),
        );

        match previous {
            Some(claim) => row(
                ui,
                "spot on",
                format!("exactly {} x {}", claim.quantity, claim.face),
                Some(readout::percent(&view.chance_exactly(claim))),
            ),
            None => row(ui, "spot on", "not available".to_string(), None),
        }
    });
}

/// This round's claims: seats down the side, trips around the table across the top.
///
/// A seat's first claim sits in the first column, its second in the second, and so on, so a
/// column is one time around the table and a row is one player's escalation. Reading along
/// a row shows how far one player pushed.
///
/// Rows start with whoever opened the round rather than with the match's turn order, so the
/// first column reads straight down in the order the claims were actually made.
pub fn history(ui: &mut Ui, view: &View, seating: &Seating) {
    let public = view.public();
    let entries = public.history();
    ui.label(RichText::new("This round").strong());
    if entries.is_empty() {
        ui.label("no claims yet");
        return;
    }

    let mut claims: Vec<Vec<Claim>> = vec![Vec::new(); seating.len()];
    for entry in entries {
        if let Some(row) = claims.get_mut(entry.player) {
            row.push(entry.claim);
        }
    }
    let trips = claims.iter().map(Vec::len).max().unwrap_or(0);

    let order = public.order();
    let opener = entries[0].player;
    let from = order.iter().position(|&seat| seat == opener).unwrap_or(0);
    let rotated: Vec<usize> = order[from..]
        .iter()
        .chain(order[..from].iter())
        .copied()
        .collect();

    Grid::new("history").spacing([4.0, 4.0]).show(ui, |ui| {
        ui.label("");
        for trip in 1..=trips {
            ui.label(RichText::new(format!("{trip}")).size(13.0).weak());
        }
        ui.end_row();

        for &seat in &rotated {
            // A seat that never claimed and is out of the match has nothing to show.
            if claims[seat].is_empty() && public.eliminated(seat) {
                continue;
            }
            ui.label(RichText::new(seating.name(seat)).size(14.0));
            for trip in 0..trips {
                claim_box(ui, seating.colour(seat), claims[seat].get(trip).copied());
            }
            ui.end_row();
        }
    });
}

/// Who finished where, once the match is over.
pub fn ranking(ui: &mut Ui, view: &View, seating: &Seating) {
    ui.label(RichText::new("Finishing order").strong());
    Grid::new("ranking").spacing([5.0, 5.0]).show(ui, |ui| {
        for (place, &seat) in view.public().ranking().iter().enumerate() {
            ui.label(RichText::new(format!("{}. {}", place + 1, seating.name(seat))).size(16.0));
            die(ui, seating.colour(seat), None);
            ui.end_row();
        }
    });
}

/// Which single steps a dialler may offer from a claim.
///
/// Separate from drawing the buttons so the rule can be tested without a screen. A step is
/// offered only when the claim it lands on is legal, which is where `An invalid claim
/// cannot be made` is enforced: before the number exists rather than after it is checked.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Steps {
    pub quantity_down: bool,
    pub quantity_up: bool,
    pub face_down: bool,
    pub face_up: bool,
}

/// What a dialler showing `from` may offer.
pub fn steps_from(view: &View, from: Claim) -> Steps {
    let public = view.public();
    let legal = |claim: Claim| public.check_raise(claim).is_ok();
    Steps {
        quantity_down: from.quantity > 1
            && legal(Claim {
                quantity: from.quantity - 1,
                ..from
            }),
        quantity_up: legal(Claim {
            quantity: from.quantity + 1,
            ..from
        }),
        face_down: from.face > 1
            && legal(Claim {
                face: from.face - 1,
                ..from
            }),
        face_up: from.face < 6
            && legal(Claim {
                face: from.face + 1,
                ..from
            }),
    }
}

/// The quantity and face steppers.
///
/// `The local player can reach every legal claim` holds because every legal claim is
/// reachable from the lowest one by these steps, which is asserted over a whole match
/// rather than reasoned about.
pub fn dialler(ui: &mut Ui, view: &View, pending: &mut Claim) {
    let current = *pending;
    let steps = steps_from(view, current);
    let mut dialled = current;

    Grid::new("dialler").spacing([6.0, 6.0]).show(ui, |ui| {
        ui.label(RichText::new("quantity").size(16.0));
        if tap(ui, "-", steps.quantity_down) {
            dialled.quantity -= 1;
        }
        dialled_value(ui, current.quantity);
        if tap(ui, "+", steps.quantity_up) {
            dialled.quantity += 1;
        }
        ui.end_row();

        ui.label(RichText::new("face").size(16.0));
        if tap(ui, "-", steps.face_down) {
            dialled.face -= 1;
        }
        dialled_value(ui, current.face as u32);
        if tap(ui, "+", steps.face_up) {
            dialled.face += 1;
        }
        ui.end_row();
    });

    *pending = dialled;
}

/// Raise, liar and spot on. `None` until one is tapped.
pub fn calls(ui: &mut Ui, view: &View, pending: Claim) -> Option<Call> {
    let public = view.public();
    let has_previous = public.previous_claim().is_some();
    let mut call = None;
    ui.horizontal(|ui| {
        if tap(ui, "Raise", public.check_raise(pending).is_ok()) {
            call = Some(Call::Raise(pending));
        }
        if tap(ui, "Liar", has_previous) {
            call = Some(Call::Liar);
        }
        if tap(ui, "Spot On", has_previous) {
            call = Some(Call::SpotOn);
        }
    });
    call
}

/// The lowest claim this seat could legally raise to, which is what a dialler starts on.
pub fn opening_claim(view: &View) -> Claim {
    view.legal_raises().first().copied().unwrap_or(Claim {
        quantity: 1,
        face: 1,
    })
}

/// What happened when a round ended, as a sentence.
///
/// Every delivery says it the same way because it is one function: the call, the claim it
/// disputed, what was actually there, and then each consequence the resolution recorded, in
/// the order a person would narrate them.
pub fn verdict(
    seating: &Seating,
    actor: usize,
    call: Call,
    claim: Claim,
    resolution: &Resolution,
) -> String {
    let mut said = match call {
        Call::Liar => format!("{} called liar", seating.name(actor)),
        Call::SpotOn => format!("{} called spot on", seating.name(actor)),
        // A raise continues a round, so it never reaches a resolution.
        Call::Raise(_) => format!("{} raised", seating.name(actor)),
    };
    said.push_str(&format!(
        " on at least {} × {}. There were {}.",
        claim.quantity, claim.face, resolution.truth
    ));
    if let Some(seat) = resolution.gained {
        said.push_str(&format!(" {} gained a die.", seating.name(seat)));
    }
    if let Some(seat) = resolution.shield_gained {
        said.push_str(&format!(" {} gained a shield token.", seating.name(seat)));
    }
    if let Some(seat) = resolution.lost {
        said.push_str(&format!(" {} lost a die.", seating.name(seat)));
    }
    if let Some(seat) = resolution.shield_spent {
        said.push_str(&format!(
            " {} spent a shield token instead of losing a die.",
            seating.name(seat)
        ));
    }
    if let Some(seat) = resolution.eliminated {
        said.push_str(&format!(" {} is out.", seating.name(seat)));
    }
    said
}
