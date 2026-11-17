//! Every appearance parameter the prototype exposes.
//!
//! Colors are plain arrays rather than engine color types so that a settings panel can
//! edit them without this crate knowing anything about a settings panel.

/// The cube half-extent. Fixed; the sphere radius is expressed relative to it.
pub const HALF_EXTENT: f32 = 1.0;

/// Below this radius the sphere reaches the cube's edges and they round over.
pub const MIN_RADIUS: f32 = std::f32::consts::SQRT_2;

/// At or above this radius the sphere misses the corners and the die is a plain cube.
pub const MAX_RADIUS: f32 = 1.732_050_8;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shape {
    /// Sphere radius. Clamped to the band where the faces stay flat and the corners are
    /// clipped.
    pub sphere_radius: f32,
    /// Segments per corner arc.
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
    pub fn radius(&self) -> f32 {
        self.sphere_radius
            .clamp(MIN_RADIUS + 0.0005, MAX_RADIUS - 0.0005)
    }

    /// Radius of the circle where the sphere meets a face plane.
    pub fn face_circle(&self) -> f32 {
        let r = self.radius();
        (r * r - HALF_EXTENT * HALF_EXTENT).sqrt()
    }

    /// Distance from a face's centre line to where an arc meets a cube edge.
    pub fn edge_offset(&self) -> f32 {
        let r = self.radius();
        (r * r - 2.0 * HALF_EXTENT * HALF_EXTENT).sqrt()
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shape: Shape {
                sphere_radius: 1.55,
                arc_segments: 12,
                corner_rings: 4,
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
