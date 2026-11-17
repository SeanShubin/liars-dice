//! Generates whole appearances that are worth looking at.
//!
//! Random values in every channel independently produce clashes almost every time. What
//! this does instead is pick one base hue, derive the rest from it by a harmonic
//! relationship, and then enforce the two contrasts that matter: pips against their face,
//! and the die against the ground.
//!
//! Generation is a pure function of a seed, so an appearance can be returned to by
//! entering the seed that produced it.

use crate::noise::hash;
use crate::settings::Settings;

struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Self {
        Self(hash(seed ^ 0x5bf0_3635))
    }

    fn next(&mut self) -> u32 {
        self.0 = hash(self.0);
        self.0
    }

    /// Zero to one.
    fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / ((1u32 << 24) as f32)
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    fn chance(&mut self, probability: f32) -> bool {
        self.unit() < probability
    }

    fn pick<T: Copy>(&mut self, options: &[T]) -> T {
        options[self.next() as usize % options.len()]
    }
}

/// How the ground's hue relates to the die's.
///
/// These are the standard harmonies. Anything else is an arbitrary hue pair, which is
/// where clashes come from.
fn harmony(rng: &mut Rng) -> f32 {
    let scheme = rng.pick(&[0u32, 1, 2, 3, 4, 5]);
    let sign = if rng.chance(0.5) { 1.0 } else { -1.0 };
    match scheme {
        0 => 0.0,                          // monochrome
        1 => sign * rng.range(20.0, 40.0), // analogous
        2 => 180.0,                        // complementary
        3 => sign * 150.0,                 // split complementary
        4 => sign * 120.0,                 // triadic
        _ => sign * 90.0,                  // square
    }
}

/// Hue in degrees, saturation and lightness zero to one, to an sRGB triple.
fn hsl(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let hue = hue.rem_euclid(360.0);
    let saturation = saturation.clamp(0.0, 1.0);
    let lightness = lightness.clamp(0.0, 1.0);
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (red, green, blue) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let base = lightness - chroma / 2.0;
    [red + base, green + base, blue + base]
}

/// Moves `value` away from `anchor` until they differ by at least `minimum`.
fn separate(value: f32, anchor: f32, minimum: f32) -> f32 {
    if (value - anchor).abs() >= minimum {
        return value;
    }
    let away = if anchor > 0.5 {
        anchor - minimum
    } else {
        anchor + minimum
    };
    away.clamp(0.03, 0.97)
}

/// Replaces every appearance setting. Leaves the shape alone, so appearances stay
/// comparable from one press to the next.
pub fn random(seed: u32, settings: &mut Settings) {
    let mut rng = Rng::new(seed);

    let die_hue = rng.range(0.0, 360.0);
    let ground_hue = die_hue + harmony(&mut rng);

    // Most dice are pale with dark pips; some are the reverse. Choosing once and taking
    // opposite ends for the face and the pips is what guarantees a readable face.
    let pale = rng.chance(0.72);

    // Saturation stays low. A fully saturated die reads as boiled sweet rather than dice.
    let die_saturation = if pale {
        rng.range(0.03, 0.20)
    } else {
        rng.range(0.06, 0.34)
    };
    let die_lightness = if pale {
        rng.range(0.78, 0.93)
    } else {
        rng.range(0.09, 0.22)
    };

    // Speckles share the hue — a speckle in a different hue reads as dirt — and differ
    // only in lightness and a little saturation.
    let speckle_lightness = if pale {
        die_lightness - rng.range(0.22, 0.42)
    } else {
        die_lightness + rng.range(0.14, 0.30)
    };
    let speckle_saturation = (die_saturation + rng.range(0.04, 0.18)).min(0.55);

    // Pips usually stay in the die's hue family and occasionally take the harmony
    // partner. Either way lightness is forced apart from the face.
    let pip_hue = if rng.chance(0.78) {
        die_hue
    } else {
        ground_hue
    };
    let pip_lightness = separate(
        if pale {
            rng.range(0.05, 0.17)
        } else {
            rng.range(0.83, 0.95)
        },
        die_lightness,
        0.45,
    );
    let pip_saturation = rng.range(0.0, 0.22);

    // The ground is the largest area on screen, so its saturation is held down and its
    // lightness is kept clear of the die's, or the die stops reading as an object.
    let ground_saturation = rng.range(0.10, 0.42);
    let ground_lightness = separate(rng.range(0.10, 0.70), die_lightness, 0.28);
    let ground_speckle_lightness = if ground_lightness > 0.5 {
        ground_lightness - rng.range(0.10, 0.22)
    } else {
        ground_lightness + rng.range(0.06, 0.16)
    };

    settings.face.color = hsl(die_hue, die_saturation, die_lightness);
    settings.face.speckle_color = hsl(die_hue, speckle_saturation, speckle_lightness);
    settings.face.speckle_amount = rng.range(0.0, 0.32);
    settings.face.speckle_scale = rng.range(14.0, 90.0);
    settings.face.bump_strength = rng.range(0.0, 0.32);
    settings.face.roughness = rng.range(0.18, 0.80);
    settings.face.metallic = if rng.chance(0.12) {
        rng.range(0.1, 0.45)
    } else {
        0.0
    };
    settings.face.seed = rng.next() % 64;

    // The bevel is the same material as the face, caught at a different angle: the same
    // hue, a little darker, a little less saturated.
    settings.bevel.color = hsl(
        die_hue,
        die_saturation * 0.85,
        (die_lightness - 0.05).clamp(0.03, 0.97),
    );
    settings.bevel.speckle_color = hsl(
        die_hue,
        speckle_saturation * 0.9,
        speckle_lightness.clamp(0.03, 0.97),
    );
    settings.bevel.speckle_amount = (settings.face.speckle_amount * rng.range(0.7, 1.2)).min(1.0);
    settings.bevel.speckle_scale = settings.face.speckle_scale * rng.range(0.6, 1.0);
    settings.bevel.bump_strength = settings.face.bump_strength * rng.range(0.8, 1.6);
    settings.bevel.roughness = (settings.face.roughness + rng.range(0.0, 0.18)).min(1.0);
    settings.bevel.metallic = settings.face.metallic;
    settings.bevel.seed = rng.next() % 64;

    settings.glyph.color = hsl(pip_hue, pip_saturation, pip_lightness);
    settings.glyph.radius = rng.range(0.15, 0.26);
    settings.glyph.depth = rng.range(0.3, 1.2);

    settings.ground.color = hsl(ground_hue, ground_saturation, ground_lightness);
    settings.ground.speckle_color = hsl(
        ground_hue,
        (ground_saturation + rng.range(0.0, 0.12)).min(0.6),
        ground_speckle_lightness.clamp(0.03, 0.97),
    );
    settings.ground.speckle_amount = rng.range(0.25, 0.70);
    settings.ground.speckle_scale = rng.range(90.0, 260.0);
    settings.ground.bump_strength = rng.range(0.18, 0.60);
    settings.ground.roughness = rng.range(0.82, 1.0);
    settings.ground.metallic = 0.0;
    settings.ground.seed = rng.next() % 64;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Perceived lightness, weighting green most heavily, as the eye does.
    fn luminance(color: [f32; 3]) -> f32 {
        0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2]
    }

    #[test]
    fn pips_always_contrast_with_their_face() {
        for seed in 0..2000 {
            let mut settings = Settings::default();
            random(seed, &mut settings);
            let gap = (luminance(settings.face.color) - luminance(settings.glyph.color)).abs();
            assert!(
                gap > 0.25,
                "seed {seed} gives a face and pips only {gap} apart"
            );
        }
    }

    #[test]
    fn the_die_always_reads_against_the_ground() {
        for seed in 0..2000 {
            let mut settings = Settings::default();
            random(seed, &mut settings);
            let gap = (luminance(settings.face.color) - luminance(settings.ground.color)).abs();
            assert!(
                gap > 0.04,
                "seed {seed} gives a die and ground only {gap} apart"
            );
        }
    }

    #[test]
    fn colours_stay_in_range() {
        for seed in 0..2000 {
            let mut settings = Settings::default();
            random(seed, &mut settings);
            for color in [
                settings.face.color,
                settings.face.speckle_color,
                settings.bevel.color,
                settings.bevel.speckle_color,
                settings.glyph.color,
                settings.ground.color,
                settings.ground.speckle_color,
            ] {
                for channel in color {
                    assert!(
                        (0.0..=1.0).contains(&channel),
                        "seed {seed} produced {channel}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_seed_always_gives_the_same_appearance() {
        let mut first = Settings::default();
        let mut second = Settings::default();
        random(4242, &mut first);
        random(4242, &mut second);
        assert_eq!(first, second);
    }

    #[test]
    fn different_seeds_give_different_appearances() {
        let mut first = Settings::default();
        let mut second = Settings::default();
        random(1, &mut first);
        random(2, &mut second);
        assert_ne!(first, second);
    }
}
