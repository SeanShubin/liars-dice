//! The die mesh: a cube intersected with a sphere.
//!
//! Every curved point lies on the one sphere and every flat point lies on one of six
//! planes, so the mesh is constructed analytically rather than approximated. Flat faces
//! are therefore exactly flat, which matters because the glyph sits on them.
//!
//! The mesh is produced in two parts, one per material. A triangle's material is known
//! from which part generated it, so there is nothing to classify and nothing to tune.

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

/// Points along a circular arc in two dimensions, both ends included.
fn arc_2d(radius: f32, from: f32, to: f32, segments: u32) -> Vec<Vec2> {
    let segments = segments.max(1);
    (0..=segments)
        .map(|step| {
            let angle = from + (to - from) * step as f32 / segments as f32;
            Vec2::new(radius * angle.cos(), radius * angle.sin())
        })
        .collect()
}

/// The outline of one face, counter-clockwise in local coordinates.
///
/// Four straight segments where the face meets a cube edge, and four arcs where the
/// sphere cuts a corner away. The straight segments need no points of their own; they
/// are the chords between consecutive arcs.
fn face_outline(shape: &Shape) -> Vec<Vec2> {
    let circle = shape.face_circle();
    let w = shape.edge_offset();
    let h = HALF_EXTENT;
    let segments = shape.arc_segments;

    let corners = [
        (Vec2::new(w, -h), Vec2::new(h, -w)),
        (Vec2::new(h, w), Vec2::new(w, h)),
        (Vec2::new(-w, h), Vec2::new(-h, w)),
        (Vec2::new(-h, -w), Vec2::new(-w, -h)),
    ];

    let mut outline = Vec::new();
    for (start, end) in corners {
        let from = start.y.atan2(start.x);
        let mut to = end.y.atan2(end.x);
        while to < from {
            to += std::f32::consts::TAU;
        }
        outline.extend(arc_2d(circle, from, to, segments));
    }
    outline
}

/// Maps a point in a face's local coordinates into that face's cell of the atlas.
fn atlas_uv(value: u32, local: Vec2) -> Vec2 {
    let column = ((value - 1) % ATLAS_COLUMNS) as f32;
    let row = ((value - 1) / ATLAS_COLUMNS) as f32;
    let s = (local.x / HALF_EXTENT + 1.0) * 0.5;
    let t = (1.0 - local.y / HALF_EXTENT) * 0.5;
    let s = CELL_INSET + s * (1.0 - 2.0 * CELL_INSET);
    let t = CELL_INSET + t * (1.0 - 2.0 * CELL_INSET);
    Vec2::new(
        (column + s) / ATLAS_COLUMNS as f32,
        (row + t) / ATLAS_ROWS as f32,
    )
}

/// The six flat faces, as one mesh sharing one atlas material.
pub fn faces_mesh(shape: &Shape) -> Mesh {
    let mut builder = Builder::default();
    let outline = face_outline(shape);

    for face in faces() {
        let to_world =
            |local: Vec2| face.normal * HALF_EXTENT + face.u * local.x + face.v * local.y;

        let centre = builder.push(
            to_world(Vec2::ZERO),
            face.normal,
            atlas_uv(face.value, Vec2::ZERO),
        );
        let ring: Vec<u32> = outline
            .iter()
            .map(|&local| builder.push(to_world(local), face.normal, atlas_uv(face.value, local)))
            .collect();

        for step in 0..ring.len() {
            builder.triangle(centre, ring[step], ring[(step + 1) % ring.len()]);
        }
    }

    builder.into_mesh()
}

/// Points along a circular arc in a plane, both ends included.
///
/// The plane is given by its `axis`. Both endpoints must already lie on the circle.
fn arc_3d(centre: Vec3, from: Vec3, to: Vec3, axis: Vec3, segments: u32) -> Vec<Vec3> {
    let segments = segments.max(1);
    let radial = from - centre;
    let radius = radial.length();
    let e1 = radial / radius;
    let e2 = axis.normalize().cross(e1);
    let target = to - centre;
    let mut sweep = target.dot(e2).atan2(target.dot(e1));
    if sweep > std::f32::consts::PI {
        sweep -= std::f32::consts::TAU;
    }
    (0..=segments)
        .map(|step| {
            let angle = sweep * step as f32 / segments as f32;
            centre + radius * (angle.cos() * e1 + angle.sin() * e2)
        })
        .collect()
}

/// The eight curved corner patches, as one mesh sharing one bevel material.
pub fn corners_mesh(shape: &Shape) -> Mesh {
    let radius = shape.radius();
    let w = shape.edge_offset();
    let h = HALF_EXTENT;
    let rings = shape.corner_rings.max(1);
    let segments = shape.arc_segments;

    let mut builder = Builder::default();

    for sx in [1.0f32, -1.0] {
        for sy in [1.0f32, -1.0] {
            for sz in [1.0f32, -1.0] {
                // The three points where two cube planes and the sphere meet.
                let on_yz = Vec3::new(sx * w, sy * h, sz * h);
                let on_xz = Vec3::new(sx * h, sy * w, sz * h);
                let on_xy = Vec3::new(sx * h, sy * h, sz * w);

                let mut outline = Vec::new();
                for (centre, from, to, axis) in [
                    (Vec3::new(0.0, 0.0, sz * h), on_yz, on_xz, Vec3::Z * sz),
                    (Vec3::new(sx * h, 0.0, 0.0), on_xz, on_xy, Vec3::X * sx),
                    (Vec3::new(0.0, sy * h, 0.0), on_xy, on_yz, Vec3::Y * sy),
                ] {
                    let mut arc = arc_3d(centre, from, to, axis, segments);
                    arc.pop();
                    outline.extend(arc);
                }

                let apex_direction = Vec3::new(sx, sy, sz).normalize();

                // Wind so the triangles face outward.
                let first = (outline[0] - apex_direction * radius)
                    .cross(outline[1] - apex_direction * radius);
                if first.dot(apex_direction) < 0.0 {
                    outline.reverse();
                }

                let apex =
                    builder.push(apex_direction * radius, apex_direction, Vec2::new(0.5, 0.5));

                // Each patch is parameterised as a disc, so the bevel texture is sampled
                // without a seam and without projecting anything.
                let count = outline.len();
                let mut previous: Vec<u32> = Vec::new();
                for ring in 1..=rings {
                    let distance = ring as f32 / rings as f32;
                    let current: Vec<u32> = outline
                        .iter()
                        .enumerate()
                        .map(|(step, &boundary)| {
                            let position = (apex_direction * radius)
                                .lerp(boundary, distance)
                                .normalize()
                                * radius;
                            let angle = std::f32::consts::TAU * step as f32 / count as f32;
                            let uv = Vec2::new(
                                0.5 + 0.5 * distance * angle.cos(),
                                0.5 + 0.5 * distance * angle.sin(),
                            );
                            builder.push(position, position.normalize(), uv)
                        })
                        .collect();

                    if previous.is_empty() {
                        for step in 0..count {
                            builder.triangle(apex, current[step], current[(step + 1) % count]);
                        }
                    } else {
                        for step in 0..count {
                            let next = (step + 1) % count;
                            builder.triangle(previous[step], current[step], current[next]);
                            builder.triangle(previous[step], current[next], previous[next]);
                        }
                    }
                    previous = current;
                }
            }
        }
    }

    builder.into_mesh()
}
