//! Minimal renderer. The two dimensional presentation.
//!
//! What lives here is the dice colour palette and the assignment that keeps players apart.
//! Realizes /specification/dice.md — identity and the palette.

pub mod palette;

pub use palette::{PALETTE, Swatch, assign, distance};
