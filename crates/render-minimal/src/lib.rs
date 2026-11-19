//! Minimal renderer. The two dimensional presentation.
//!
//! Realizes /specification/game-minimal.md, and /specification/dice.md for identity and
//! the palette.
//!
//! One presentation, drawn once. `apps/minimal` puts it in a window and `apps/hotseat`
//! puts it on a canvas, and they look the same because there is one copy of it rather than
//! two kept in step by hand.
//!
//! Nothing here knows what a delivery is. Every function takes a `&mut egui::Ui` and
//! leaves windows, canvases, event loops and main functions to whoever composed the app.
//! That is what lets a desktop binary and a wasm bundle share it.
//!
//! The panels take a [`rules::View`] rather than a `rules::Game`, which is the one design
//! decision worth stating twice: a view hides another seat's dice as a type fact, so a
//! panel cannot draw what the seat may not see. On a shared screen that is the difference
//! between a rule and a hope.

pub mod palette;
pub mod panel;
pub mod seating;
pub mod widget;

pub use palette::{PALETTE, Swatch, assign, distance, ink, lightness};
pub use panel::{
    Steps, calls, dialler, history, odds, opening_claim, ranking, standing, steps_from, table,
    verdict,
};
pub use seating::Seating;
pub use widget::{
    CLAIM, DIE, claim_box, colour_of, dialled_value, die, ink_of, palette_picker, tap,
};

#[cfg(test)]
mod tests;
