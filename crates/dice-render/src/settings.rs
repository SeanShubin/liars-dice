//! Every appearance parameter the prototype exposes.
//!
//! Colors are plain arrays rather than engine color types so that a settings panel can
//! edit them without this crate knowing anything about a settings panel.

/// The die's overall half-extent. Fixed; the rounding is expressed relative to it.
pub const HALF_EXTENT: f32 = 1.0;

/// Below this, the rounding is too small to see.
pub const MIN_ROUNDING: f32 = 0.01;

/// At one the die would be a sphere, with no flat face left to put a glyph on.
pub const MAX_ROUNDING: f32 = 0.9;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shape {
    /// The radius every edge and corner is rounded to. The overall half-extent stays at
    /// `HALF_EXTENT`, so this and the inner cube's half-extent sum to it.
    pub rounding: f32,
    /// Segments per quarter arc, shared by the fillets and the corners so their seams
    /// meet exactly.
    pub arc_segments: u32,
    /// Concentric rings per corner patch.
    pub corner_rings: u32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Surface {
    pub color: [f32; 3],
    pub speckle_color: [f32; 3],
    /// Fraction of the surface covered by speckles.
    pub speckle_amount: f32,
    /// Speckle frequency.
    pub speckle_scale: f32,
    /// Height amplitude of the speckles, giving the rough three dimensional appearance.
    pub bump_strength: f32,
    /// Zero is a mirror, one is entirely flat.
    pub roughness: f32,
    pub metallic: f32,
    pub seed: u32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Glyph {
    pub color: [f32; 3],
    /// Pip radius, in units where the face is two wide.
    pub radius: f32,
    /// How deep a pip is pressed into the face.
    pub depth: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Settings {
    pub shape: Shape,
    pub face: Surface,
    pub bevel: Surface,
    pub glyph: Glyph,
    pub ground: Surface,
}

impl Shape {
    /// The rounding radius, kept inside the range where both a flat face and a visible
    /// rounding exist.
    pub fn radius(&self) -> f32 {
        self.rounding.clamp(MIN_ROUNDING, MAX_ROUNDING) * HALF_EXTENT
    }

    /// Half-extent of the inner cube the rounding is swept over.
    pub fn inner(&self) -> f32 {
        HALF_EXTENT - self.radius()
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shape: Shape {
                rounding: 0.15,
                arc_segments: 8,
                corner_rings: 3,
            },
            face: Surface {
                color: [0.88, 0.86, 0.80],
                speckle_color: [0.42, 0.40, 0.36],
                speckle_amount: 0.12,
                speckle_scale: 40.0,
                bump_strength: 0.15,
                roughness: 0.45,
                metallic: 0.0,
                seed: 1,
            },
            bevel: Surface {
                color: [0.82, 0.80, 0.74],
                speckle_color: [0.38, 0.36, 0.33],
                speckle_amount: 0.10,
                speckle_scale: 30.0,
                bump_strength: 0.20,
                roughness: 0.55,
                metallic: 0.0,
                seed: 2,
            },
            glyph: Glyph {
                color: [0.10, 0.09, 0.09],
                radius: 0.2,
                depth: 0.6,
            },
            ground: Surface {
                color: [0.09, 0.32, 0.18],
                speckle_color: [0.06, 0.24, 0.13],
                speckle_amount: 0.5,
                speckle_scale: 180.0,
                bump_strength: 0.35,
                roughness: 0.95,
                metallic: 0.0,
                seed: 3,
            },
        }
    }
}
