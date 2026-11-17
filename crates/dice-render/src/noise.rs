//! Deterministic value noise.
//!
//! Explicitly seeded. Never reads the clock, so the same settings always produce the
//! same texture.

pub(crate) fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

fn lattice(ix: i32, iy: i32, seed: u32) -> f32 {
    let a = hash((ix as u32).wrapping_mul(0x9e37_79b9));
    let b = hash((iy as u32).wrapping_mul(0x85eb_ca6b));
    let h = hash(a ^ b ^ seed);
    (h >> 8) as f32 / ((1u32 << 24) as f32)
}

/// Value noise with smooth interpolation, in the range zero to one.
pub fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let ix = x0 as i32;
    let iy = y0 as i32;
    let v00 = lattice(ix, iy, seed);
    let v10 = lattice(ix + 1, iy, seed);
    let v01 = lattice(ix, iy + 1, seed);
    let v11 = lattice(ix + 1, iy + 1, seed);
    let a = v00 + (v10 - v00) * sx;
    let b = v01 + (v11 - v01) * sx;
    a + (b - a) * sy
}

/// Several octaves of value noise, in the range zero to one.
pub fn fbm(x: f32, y: f32, seed: u32, octaves: u32) -> f32 {
    let mut sum = 0.0;
    let mut norm = 0.0;
    let mut amplitude = 0.5;
    let mut frequency = 1.0;
    for octave in 0..octaves.max(1) {
        sum += amplitude
            * value_noise(
                x * frequency,
                y * frequency,
                seed.wrapping_add(octave.wrapping_mul(7919)),
            );
        norm += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    sum / norm
}
