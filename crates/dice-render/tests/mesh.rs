//! The checks /implementation/die-rendering-prototype.md calls for.
//!
//! Acceptance is visual and cannot be automated. What can be automated is that the mesh
//! is a closed surface, that it faces outward, that it is reproducible, and that the six
//! faces are laid out and drawn distinctly.

use std::collections::HashMap;

use bevy::math::Vec3;
use bevy::render::mesh::{Indices, Mesh, VertexAttributeValues};

use dice_render::geometry::{ATLAS_COLUMNS, ATLAS_ROWS, corners_mesh, faces, faces_mesh};
use dice_render::settings::Settings;
use dice_render::texture::{FACE_PIXELS, face_base_color};

fn positions(mesh: &Mesh) -> Vec<Vec3> {
    mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(VertexAttributeValues::as_float3)
        .expect("positions")
        .iter()
        .map(|p| Vec3::from_array(*p))
        .collect()
}

fn uvs(mesh: &Mesh) -> Vec<[f32; 2]> {
    match mesh.attribute(Mesh::ATTRIBUTE_UV_0).expect("uvs") {
        VertexAttributeValues::Float32x2(values) => values.clone(),
        other => panic!("unexpected uv format: {other:?}"),
    }
}

fn triangles(mesh: &Mesh) -> Vec<[u32; 3]> {
    match mesh.indices().expect("indices") {
        Indices::U32(flat) => flat.chunks(3).map(|c| [c[0], c[1], c[2]]).collect(),
        Indices::U16(flat) => flat
            .chunks(3)
            .map(|c| [c[0] as u32, c[1] as u32, c[2] as u32])
            .collect(),
    }
}

/// Positions are compared after quantising, because the face meshes and the corner
/// meshes arrive at the shared boundary by different routes.
fn key(point: Vec3) -> (i64, i64, i64) {
    let scale = 10_000.0;
    (
        (point.x * scale).round() as i64,
        (point.y * scale).round() as i64,
        (point.z * scale).round() as i64,
    )
}

#[test]
fn the_die_is_a_closed_surface() {
    let shape = Settings::default().shape;
    let mut edges: HashMap<[(i64, i64, i64); 2], usize> = HashMap::new();

    for mesh in [faces_mesh(&shape), corners_mesh(&shape)] {
        let points = positions(&mesh);
        for triangle in triangles(&mesh) {
            for step in 0..3 {
                let a = key(points[triangle[step] as usize]);
                let b = key(points[triangle[(step + 1) % 3] as usize]);
                let mut edge = [a, b];
                edge.sort();
                *edges.entry(edge).or_default() += 1;
            }
        }
    }

    let open: Vec<_> = edges
        .iter()
        .filter(|&(_, &count)| count != 2)
        .map(|(edge, count)| (*edge, *count))
        .collect();
    assert!(
        open.is_empty(),
        "{} edges are not shared by exactly two triangles, for example {:?}",
        open.len(),
        open.first()
    );
}

#[test]
fn every_triangle_faces_outward() {
    let shape = Settings::default().shape;
    for mesh in [faces_mesh(&shape), corners_mesh(&shape)] {
        let points = positions(&mesh);
        for triangle in triangles(&mesh) {
            let a = points[triangle[0] as usize];
            let b = points[triangle[1] as usize];
            let c = points[triangle[2] as usize];
            let outward = (b - a).cross(c - a);
            let centroid = (a + b + c) / 3.0;
            assert!(
                outward.dot(centroid) > 0.0,
                "a triangle at {centroid:?} is wound inward"
            );
        }
    }
}

#[test]
fn mesh_generation_is_reproducible() {
    let shape = Settings::default().shape;
    assert_eq!(
        positions(&faces_mesh(&shape)),
        positions(&faces_mesh(&shape))
    );
    assert_eq!(
        positions(&corners_mesh(&shape)),
        positions(&corners_mesh(&shape))
    );
}

#[test]
fn each_face_stays_inside_its_own_cell_of_the_atlas() {
    let shape = Settings::default().shape;
    let mesh = faces_mesh(&shape);
    let coordinates = uvs(&mesh);

    // The faces are emitted in the order faces() returns them, one block each.
    let per_face = coordinates.len() / 6;
    for (index, face) in faces().iter().enumerate() {
        let column = ((face.value - 1) % ATLAS_COLUMNS) as f32;
        let row = ((face.value - 1) / ATLAS_COLUMNS) as f32;
        let u_range = (
            column / ATLAS_COLUMNS as f32,
            (column + 1.0) / ATLAS_COLUMNS as f32,
        );
        let v_range = (row / ATLAS_ROWS as f32, (row + 1.0) / ATLAS_ROWS as f32);

        for uv in &coordinates[index * per_face..(index + 1) * per_face] {
            assert!(
                uv[0] >= u_range.0 && uv[0] <= u_range.1,
                "face {} sampled u {} outside {:?}",
                face.value,
                uv[0],
                u_range
            );
            assert!(
                uv[1] >= v_range.0 && uv[1] <= v_range.1,
                "face {} sampled v {} outside {:?}",
                face.value,
                uv[1],
                v_range
            );
        }
    }
}

#[test]
fn the_six_faces_are_drawn_differently() {
    let settings = Settings::default();
    let atlas = face_base_color(&settings.face, &settings.glyph);
    let data = atlas.data.as_ref().expect("atlas pixels");
    let width = FACE_PIXELS * ATLAS_COLUMNS;

    let cell = |value: u32| -> Vec<u8> {
        let column = (value - 1) % ATLAS_COLUMNS;
        let row = (value - 1) / ATLAS_COLUMNS;
        let mut pixels = Vec::new();
        for y in 0..FACE_PIXELS {
            let start = (((row * FACE_PIXELS + y) * width + column * FACE_PIXELS) * 4) as usize;
            pixels.extend_from_slice(&data[start..start + (FACE_PIXELS * 4) as usize]);
        }
        pixels
    };

    for left in 1..=6u32 {
        for right in (left + 1)..=6u32 {
            assert_ne!(
                cell(left),
                cell(right),
                "faces {left} and {right} are identical"
            );
        }
    }
}
