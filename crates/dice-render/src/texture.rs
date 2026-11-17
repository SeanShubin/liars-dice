//! Procedurally generated textures.
//!
//! A "texture" in the specification is not one image. It is a set of physically based
//! channels, and the settings expose those channels rather than asking for finished
//! images. That is what lets every appearance setting be a continuous value changed
//! while the prototype runs.
//!
//! Pips are drawn into the face texture and given depth by normal perturbation rather
//! than by geometry. Real dice have drilled pips, but modelling them would add hundreds
//! of surfaces per die and would put the pips outside the glyph texture.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::math::{Vec2, Vec3};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::geometry::{ATLAS_COLUMNS, ATLAS_ROWS, faces};
use crate::noise::fbm;
use crate::settings::{Glyph, HALF_EXTENT, Surface};

/// Pixels across one face's cell of the atlas.
pub const FACE_PIXELS: u32 = 256;
/// Pixels across the bevel texture.
pub const BEVEL_PIXELS: u32 = 256;
/// Pixels across the ground texture.
pub const GROUND_PIXELS: u32 = 512;

const OCTAVES: u32 = 4;

/// Which cells of the three by three grid carry a pip, as (column, row) from the top
/// left of the face in its local frame.
///
/// The two and three diagonals run from top-right to bottom-left, and the six's columns
/// run vertically. Those orientations are not standardised across manufactured dice;
/// these are taken from a physical one.
fn pip_cells(value: u32) -> &'static [(u32, u32)] {
    match value {
        1 => &[(1, 1)],
        2 => &[(2, 0), (0, 2)],
        3 => &[(2, 0), (1, 1), (0, 2)],
        4 => &[(0, 0), (2, 0), (0, 2), (2, 2)],
        5 => &[(0, 0), (2, 0), (1, 1), (0, 2), (2, 2)],
        6 => &[(0, 0), (0, 1), (0, 2), (2, 0), (2, 1), (2, 2)],
        _ => &[],
    }
}

/// Pip centres sit a quarter of the face width from the middle.
fn pip_centre(column: u32, row: u32) -> Vec2 {
    Vec2::new(
        (-0.5 + 0.5 * column as f32) * HALF_EXTENT,
        (0.5 - 0.5 * row as f32) * HALF_EXTENT,
    )
}

/// How much of a pip covers a point, smoothed over about a texel so the edge is not
/// stepped.
fn pip_coverage(value: u32, point: Vec2, glyph: &Glyph, texel: f32) -> f32 {
    let mut coverage = 0.0f32;
    for &(column, row) in pip_cells(value) {
        let distance = (point - pip_centre(column, row)).length();
        let edge = ((glyph.radius - distance) / (texel * 1.5)).clamp(0.0, 1.0);
        coverage = coverage.max(edge);
    }
    coverage
}

/// Speckle coverage at a point, in the range zero to one.
fn speckle(point: Vec2, surface: &Surface) -> f32 {
    let sample = fbm(
        point.x * surface.speckle_scale,
        point.y * surface.speckle_scale,
        surface.seed,
        OCTAVES,
    );
    let threshold = surface.speckle_amount.clamp(0.0, 1.0);
    let width = 0.06;
    ((threshold - sample) / width + 0.5).clamp(0.0, 1.0)
}

/// Surface height at a point. Speckles raise it; a pip presses it in.
fn height(point: Vec2, surface: &Surface, glyph: Option<(u32, &Glyph, f32)>) -> f32 {
    let bumps = fbm(
        point.x * surface.speckle_scale,
        point.y * surface.speckle_scale,
        surface.seed,
        OCTAVES,
    ) - 0.5;
    let mut value = bumps * surface.bump_strength;
    if let Some((face_value, glyph, texel)) = glyph {
        value -= glyph.depth * pip_coverage(face_value, point, glyph, texel);
    }
    value
}

fn encode_normal(normal: Vec3) -> [u8; 4] {
    let encoded = normal.normalize() * 0.5 + Vec3::splat(0.5);
    [
        (encoded.x * 255.0) as u8,
        (encoded.y * 255.0) as u8,
        (encoded.z * 255.0) as u8,
        255,
    ]
}

fn encode_color(color: Vec3) -> [u8; 4] {
    [
        (color.x.clamp(0.0, 1.0) * 255.0) as u8,
        (color.y.clamp(0.0, 1.0) * 255.0) as u8,
        (color.z.clamp(0.0, 1.0) * 255.0) as u8,
        255,
    ]
}

fn image_from(
    width: u32,
    height: u32,
    format: TextureFormat,
    mut pixel: impl FnMut(u32, u32) -> [u8; 4],
) -> Image {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&pixel(x, y));
        }
    }
    Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        format,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
}

/// Converts a pixel inside a cell to that face's local coordinates.
fn cell_local(x: u32, y: u32, cell: u32) -> Vec2 {
    let column = x % cell;
    let row = y % cell;
    Vec2::new(
        ((column as f32 + 0.5) / cell as f32 * 2.0 - 1.0) * HALF_EXTENT,
        (1.0 - (row as f32 + 0.5) / cell as f32 * 2.0) * HALF_EXTENT,
    )
}

fn value_at(x: u32, y: u32, cell: u32) -> u32 {
    let column = x / cell;
    let row = y / cell;
    row * ATLAS_COLUMNS + column + 1
}

/// Base colour for all six faces, as one atlas.
pub fn face_base_color(face: &Surface, glyph: &Glyph) -> Image {
    let cell = FACE_PIXELS;
    let texel = 2.0 * HALF_EXTENT / cell as f32;
    let base = Vec3::from_array(face.color);
    let fleck = Vec3::from_array(face.speckle_color);
    let ink = Vec3::from_array(glyph.color);
    image_from(
        cell * ATLAS_COLUMNS,
        cell * ATLAS_ROWS,
        TextureFormat::Rgba8UnormSrgb,
        |x, y| {
            let local = cell_local(x, y, cell);
            let value = value_at(x, y, cell);
            let mut color = base.lerp(fleck, speckle(local, face));
            color = color.lerp(ink, pip_coverage(value, local, glyph, texel));
            encode_color(color)
        },
    )
}

/// Normals for all six faces, as one atlas. Carries both the speckle roughness and the
/// depth of the pips.
pub fn face_normal_map(face: &Surface, glyph: &Glyph) -> Image {
    let cell = FACE_PIXELS;
    let texel = 2.0 * HALF_EXTENT / cell as f32;
    image_from(
        cell * ATLAS_COLUMNS,
        cell * ATLAS_ROWS,
        TextureFormat::Rgba8Unorm,
        |x, y| {
            let local = cell_local(x, y, cell);
            let value = value_at(x, y, cell);
            let glyph_at = Some((value, glyph, texel));
            let step = texel;
            let right = height(local + Vec2::new(step, 0.0), face, glyph_at);
            let left = height(local - Vec2::new(step, 0.0), face, glyph_at);
            let up = height(local + Vec2::new(0.0, step), face, glyph_at);
            let down = height(local - Vec2::new(0.0, step), face, glyph_at);
            let slope = Vec3::new(
                -(right - left) / (2.0 * step),
                -(up - down) / (2.0 * step),
                1.0,
            );
            encode_normal(slope)
        },
    )
}

/// Base colour for the curved corner patches.
pub fn surface_base_color(surface: &Surface, pixels: u32) -> Image {
    let base = Vec3::from_array(surface.color);
    let fleck = Vec3::from_array(surface.speckle_color);
    image_from(pixels, pixels, TextureFormat::Rgba8UnormSrgb, |x, y| {
        let point = Vec2::new(
            (x as f32 + 0.5) / pixels as f32,
            (y as f32 + 0.5) / pixels as f32,
        );
        encode_color(base.lerp(fleck, speckle(point, surface)))
    })
}

/// Normals for a speckled surface with no glyph.
pub fn surface_normal_map(surface: &Surface, pixels: u32) -> Image {
    let step = 1.0 / pixels as f32;
    image_from(pixels, pixels, TextureFormat::Rgba8Unorm, |x, y| {
        let point = Vec2::new(
            (x as f32 + 0.5) / pixels as f32,
            (y as f32 + 0.5) / pixels as f32,
        );
        let right = height(point + Vec2::new(step, 0.0), surface, None);
        let left = height(point - Vec2::new(step, 0.0), surface, None);
        let up = height(point + Vec2::new(0.0, step), surface, None);
        let down = height(point - Vec2::new(0.0, step), surface, None);
        let slope = Vec3::new(
            -(right - left) / (2.0 * step),
            -(up - down) / (2.0 * step),
            1.0,
        );
        encode_normal(slope)
    })
}

/// Every face value appears exactly once in the atlas.
pub fn atlas_covers_every_face() -> bool {
    let mut seen = [false; 7];
    for face in faces() {
        seen[face.value as usize] = true;
    }
    seen[1..].iter().all(|&found| found)
}
