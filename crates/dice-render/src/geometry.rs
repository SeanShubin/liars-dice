//! The die mesh: a cube with its edges and corners rounded to a single radius.
//!
//! A smaller cube with a sphere of that radius swept over its surface. That gives exactly
//! three kinds of surface, each with a closed form — flat squares, quarter cylinders along
//! the edges, and sphere octants at the corners — so the mesh is constructed analytically
//! rather than approximated. Flat faces are therefore exactly flat, which matters because
//! the glyph sits on them.
//!
//! The mesh is produced in two parts, one per material. A triangle's material is known
//! from which part generated it, so there is nothing to classify and nothing to tune.
//!
//! Seams are exact rather than close. A fillet's tangent line is a face's boundary edge,
//! sharing both endpoints; a fillet's end arc is a corner patch's boundary arc, generated
//! by the same function at the same angles.

use bevy::asset::RenderAssetUsages;
use bevy::math::{Vec2, Vec3};
use bevy::render::mesh::{Indices, Mesh, PrimitiveTopology};

use crate::settings::{HALF_EXTENT, Shape};

/// Columns in the face texture atlas.
pub const ATLAS_COLUMNS: u32 = 3;
/// Rows in the face texture atlas.
pub const ATLAS_ROWS: u32 = 2;

/// Keeps sampling inside a cell, so neighbouring cells cannot bleed across a face edge.
const CELL_INSET: f32 = 0.003;

/// One face of the die: its value, its outward normal, and the local axes the glyph is
/// laid out in. `u` cross `v` equals `normal`, so counter-clockwise winding in local
/// coordinates faces outward.
pub struct FaceFrame {
    pub value: u32,
    pub normal: Vec3,
    pub u: Vec3,
    pub v: Vec3,
}

/// The six faces.
///
/// Opposite faces sum to seven. The die is right-handed: with one up and two toward the
/// viewer, three is on the right. The local axes also fix pip orientation — the six's
/// columns run along its local `v`, which points at the five and two faces, and the two
/// and three diagonals run from local top-right to local bottom-left.
pub fn faces() -> [FaceFrame; 6] {
    [
        FaceFrame {
            value: 6,
            normal: Vec3::Z,
            u: Vec3::X,
            v: Vec3::Y,
        },
        FaceFrame {
            value: 1,
            normal: Vec3::NEG_Z,
            u: Vec3::NEG_X,
            v: Vec3::Y,
        },
        FaceFrame {
            value: 5,
            normal: Vec3::Y,
            u: Vec3::X,
            v: Vec3::NEG_Z,
        },
        FaceFrame {
            value: 2,
            normal: Vec3::NEG_Y,
            u: Vec3::X,
            v: Vec3::Z,
        },
        FaceFrame {
            value: 4,
            normal: Vec3::X,
            u: Vec3::NEG_Z,
            v: Vec3::Y,
        },
        FaceFrame {
            value: 3,
            normal: Vec3::NEG_X,
            u: Vec3::Z,
            v: Vec3::Y,
        },
    ]
}

#[derive(Default)]
struct Builder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Builder {
    fn push(&mut self, position: Vec3, normal: Vec3, uv: Vec2) -> u32 {
        let index = self.positions.len() as u32;
        self.positions.push(position.to_array());
        self.normals.push(normal.to_array());
        self.uvs.push(uv.to_array());
        index
    }

    fn triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.triangle(a, b, c);
        self.triangle(a, c, d);
    }

    fn into_mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// Unit directions along a quarter turn from `from` to `to`, at even angles, both ends
/// included.
///
/// Every curved part of the die is built from this one function, which is why the fillets
/// and the corner patches meet exactly instead of nearly.
fn quarter_turn(from: Vec3, to: Vec3, segments: u32) -> Vec<Vec3> {
    let segments = segments.max(1);
    (0..=segments)
        .map(|step| {
            let angle = std::f32::consts::FRAC_PI_2 * step as f32 / segments as f32;
            from * angle.cos() + to * angle.sin()
        })
        .collect()
}

/// Maps a point in a face's local coordinates, normalised so the face spans minus one to
/// one, into that face's cell of the atlas.
fn atlas_uv(value: u32, local: Vec2) -> Vec2 {
    let column = ((value - 1) % ATLAS_COLUMNS) as f32;
    let row = ((value - 1) / ATLAS_COLUMNS) as f32;
    let s = (local.x + 1.0) * 0.5;
    let t = (1.0 - local.y) * 0.5;
    let s = CELL_INSET + s * (1.0 - 2.0 * CELL_INSET);
    let t = CELL_INSET + t * (1.0 - 2.0 * CELL_INSET);
    Vec2::new(
        (column + s) / ATLAS_COLUMNS as f32,
        (row + t) / ATLAS_ROWS as f32,
    )
}

/// The six flat faces, as one mesh sharing one atlas material.
pub fn faces_mesh(shape: &Shape) -> Mesh {
    let inner = shape.inner();
    let mut builder = Builder::default();

    for face in faces() {
        let corners = [
            Vec2::new(-1.0, -1.0),
            Vec2::new(1.0, -1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(-1.0, 1.0),
        ];
        let ring: Vec<u32> = corners
            .iter()
            .map(|&corner| {
                let position = face.normal * HALF_EXTENT
                    + face.u * corner.x * inner
                    + face.v * corner.y * inner;
                builder.push(position, face.normal, atlas_uv(face.value, corner))
            })
            .collect();
        builder.quad(ring[0], ring[1], ring[2], ring[3]);
    }

    builder.into_mesh()
}

/// The twelve edge fillets and eight corner patches, as one mesh sharing one bevel
/// material.
pub fn bevel_mesh(shape: &Shape) -> Mesh {
    let inner = shape.inner();
    let radius = shape.radius();
    let segments = shape.arc_segments.max(1);
    let rings = shape.corner_rings.max(1);
    let axes = [Vec3::X, Vec3::Y, Vec3::Z];

    let mut builder = Builder::default();

    // Twelve fillets: a quarter cylinder along each edge.
    for (index, &along) in axes.iter().enumerate() {
        let first_axis = axes[(index + 1) % 3];
        let second_axis = axes[(index + 2) % 3];
        for first_sign in [1.0f32, -1.0] {
            for second_sign in [1.0f32, -1.0] {
                let from = first_axis * first_sign;
                let to = second_axis * second_sign;
                let axis_line = from * inner + to * inner;

                // The surface's two tangents are the turn direction and the edge
                // direction; their cross product has to come out as the outward normal.
                let flipped = to.cross(along).dot(from) < 0.0;

                let columns: Vec<[u32; 2]> = quarter_turn(from, to, segments)
                    .iter()
                    .enumerate()
                    .map(|(step, &direction)| {
                        let turn = step as f32 / segments as f32;
                        let mut pair = [0u32; 2];
                        for (end, &distance) in [-inner, inner].iter().enumerate() {
                            let position = axis_line + along * distance + direction * radius;
                            let uv = Vec2::new(turn, (distance / inner + 1.0) * 0.5);
                            pair[end] = builder.push(position, direction, uv);
                        }
                        pair
                    })
                    .collect();

                for step in 0..segments as usize {
                    let near = columns[step];
                    let far = columns[step + 1];
                    if flipped {
                        builder.quad(near[1], far[1], far[0], near[0]);
                    } else {
                        builder.quad(near[0], far[0], far[1], near[1]);
                    }
                }
            }
        }
    }

    // Eight corner patches: an octant of a sphere at each corner of the inner cube.
    for x_sign in [1.0f32, -1.0] {
        for y_sign in [1.0f32, -1.0] {
            for z_sign in [1.0f32, -1.0] {
                let toward_x = Vec3::X * x_sign;
                let toward_y = Vec3::Y * y_sign;
                let toward_z = Vec3::Z * z_sign;
                let centre = Vec3::new(x_sign * inner, y_sign * inner, z_sign * inner);

                // The boundary is the three quarter turns the adjacent fillets end on.
                let mut boundary = Vec::new();
                for (from, to) in [
                    (toward_x, toward_y),
                    (toward_y, toward_z),
                    (toward_z, toward_x),
                ] {
                    let mut turn = quarter_turn(from, to, segments);
                    turn.pop();
                    boundary.extend(turn);
                }

                let apex_direction = (toward_x + toward_y + toward_z).normalize();
                if (boundary[0] - apex_direction)
                    .cross(boundary[1] - apex_direction)
                    .dot(apex_direction)
                    < 0.0
                {
                    boundary.reverse();
                }

                let apex = builder.push(
                    centre + apex_direction * radius,
                    apex_direction,
                    Vec2::new(0.5, 0.5),
                );

                // Parameterised as a disc, so the bevel texture is sampled without a seam
                // and without projecting anything.
                let count = boundary.len();
                let mut previous: Vec<u32> = Vec::new();
                for ring in 1..=rings {
                    let distance = ring as f32 / rings as f32;
                    let current: Vec<u32> = boundary
                        .iter()
                        .enumerate()
                        .map(|(step, &edge)| {
                            let direction = apex_direction.lerp(edge, distance).normalize();
                            let angle = std::f32::consts::TAU * step as f32 / count as f32;
                            let uv = Vec2::new(
                                0.5 + 0.5 * distance * angle.cos(),
                                0.5 + 0.5 * distance * angle.sin(),
                            );
                            builder.push(centre + direction * radius, direction, uv)
                        })
                        .collect();

                    if previous.is_empty() {
                        for step in 0..count {
                            builder.triangle(apex, current[step], current[(step + 1) % count]);
                        }
                    } else {
                        for step in 0..count {
                            let next = (step + 1) % count;
                            builder.quad(
                                previous[step],
                                current[step],
                                current[next],
                                previous[next],
                            );
                        }
                    }
                    previous = current;
                }
            }
        }
    }

    builder.into_mesh()
}
