//! Dice mesh, materials, glyphs, and ground material. Shared by the pretty renderer and
//! the prototype.
//!
//! Realizes /specification/dice.md and /specification/ground.md. Knows nothing of app
//! structure, settings panels, or input — see /implementation/die-rendering-prototype.md.

pub mod geometry;
pub mod noise;
pub mod palette;
pub mod settings;
pub mod texture;

pub use geometry::{bevel_mesh, faces, faces_mesh};
pub use settings::{Glyph, Settings, Shape, Surface};
pub use texture::{
    BEVEL_PIXELS, GROUND_PIXELS, face_base_color, face_normal_map, surface_base_color,
    surface_normal_map,
};
