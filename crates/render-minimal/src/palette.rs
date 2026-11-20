//! The dice colour palette, and the assignment that keeps players apart.
//!
//! Realizes /specification/dice.md:
//! - a player is identified by the appearance of their dice
//! - each player's dice appearance is distinct within a match
//! - a colour chosen by the human is not used for any other player
//! - opponents' dice colours maximise the smallest difference between any two players'
//!   colours
//! - the palette hues are red, green, blue, cyan, magenta and yellow
//! - the palette has a light, a base and a dark variant of each hue
//! - the palette includes black and white
//!
//! Difference is measured in Oklab rather than in sRGB, where equal numeric distances are
//! not equally visible. This is presentation, so the transcendental functions the
//! conversion needs are where the architecture statements allow them.

/// One palette entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Swatch {
    pub name: &'static str,
    pub rgb: [u8; 3],
}

const fn swatch(name: &'static str, r: u8, g: u8, b: u8) -> Swatch {
    Swatch {
        name,
        rgb: [r, g, b],
    }
}

/// Twenty colours: six hues at sixty degree spacing, each with a light, a base and a dark
/// variant, plus black and white.
///
/// Deliberately larger than the ten player limit. The assignment never puts pink next to
/// red unless it has run out of better options, so extra entries only widen the gap at
/// every player count.
pub const PALETTE: [Swatch; 20] = [
    swatch("pink", 255, 150, 160),
    swatch("red", 219, 42, 42),
    swatch("maroon", 120, 20, 26),
    swatch("cream", 250, 240, 170),
    swatch("yellow", 234, 208, 48),
    swatch("olive", 128, 112, 28),
    swatch("light green", 158, 230, 148),
    swatch("green", 48, 168, 70),
    swatch("dark green", 20, 90, 44),
    swatch("pale cyan", 170, 236, 240),
    swatch("cyan", 58, 190, 200),
    swatch("teal", 20, 100, 110),
    swatch("light blue", 150, 180, 246),
    swatch("blue", 54, 90, 210),
    swatch("navy", 24, 40, 110),
    swatch("orchid", 226, 160, 230),
    swatch("magenta", 190, 60, 180),
    swatch("purple", 94, 30, 110),
    swatch("white", 246, 246, 246),
    swatch("black", 28, 28, 28),
];

/// The palette entry a colour name refers to.
///
/// A match carries a colour as the name a player chose, not as an index: an index means
/// nothing outside this crate and a log is read by people. The relay refuses a colour
/// another seat has taken, so the names in one match are distinct, and so are the swatches
/// this maps them to.
pub fn index_of(name: &str) -> Option<usize> {
    PALETTE.iter().position(|swatch| swatch.name == name)
}

fn to_linear(channel: u8) -> f32 {
    let value = channel as f32 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB to Oklab, which is near enough perceptually uniform that euclidean distance in it
/// tracks how different two colours look.
///
/// The matrix constants are kept at the precision Bjorn Ottosson publishes them at, so
/// they can be compared against the source by eye. `f32` rounds each one at compile time;
/// writing the rounded values here would hide where they came from to no effect. These
/// are a displayed colour, which /specification/architecture.md allows to be floating
/// point, and nothing derived from them reaches an event.
#[expect(
    clippy::excessive_precision,
    reason = "published constants, kept comparable to their source"
)]
fn oklab(rgb: [u8; 3]) -> [f32; 3] {
    let r = to_linear(rgb[0]);
    let g = to_linear(rgb[1]);
    let b = to_linear(rgb[2]);

    let long = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let medium = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let short = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;

    let long = long.cbrt();
    let medium = medium.cbrt();
    let short = short.cbrt();

    [
        0.2104542553 * long + 0.7936177850 * medium - 0.0040720468 * short,
        1.9779984951 * long - 2.4285922050 * medium + 0.4505937099 * short,
        0.0259040371 * long + 0.7827717662 * medium - 0.8086757660 * short,
    ]
}

/// Oklab lightness, which is how light a colour looks rather than how large its channel
/// values are.
pub fn lightness(swatch: Swatch) -> f32 {
    oklab(swatch.rgb)[0]
}

/// The colour to draw on top of a swatch: whichever of black or white is further from it.
///
/// Without this a navy die carries navy-black lettering and a black die disappears into
/// the background. The choice is made from perceived lightness rather than guessed at.
pub fn ink(swatch: Swatch) -> [u8; 3] {
    const BLACK: [u8; 3] = [16, 16, 18];
    const WHITE: [u8; 3] = [250, 250, 250];
    let black = distance(
        swatch,
        Swatch {
            name: "ink",
            rgb: BLACK,
        },
    );
    let white = distance(
        swatch,
        Swatch {
            name: "ink",
            rgb: WHITE,
        },
    );
    if white > black { WHITE } else { BLACK }
}

/// How different two palette entries look.
pub fn distance(left: Swatch, right: Swatch) -> f32 {
    let a = oklab(left.rgb);
    let b = oklab(right.rgb);
    let dl = a[0] - b[0];
    let da = a[1] - b[1];
    let db = a[2] - b[2];
    (dl * dl + da * da + db * db).sqrt()
}

fn distances() -> [[f32; 20]; 20] {
    let mut table = [[0.0f32; 20]; 20];
    for left in 0..20 {
        for right in 0..20 {
            table[left][right] = distance(PALETTE[left], PALETTE[right]);
        }
    }
    table
}

/// Assigns colours to `players` of them, with the human taking `chosen`.
///
/// Returns one palette index per player, the human's first. The subset chosen is the one
/// that maximises the smallest difference between any two players' colours — searched
/// exhaustively rather than greedily, because twenty entries and at most ten players is
/// small enough to find the best assignment rather than a good one.
pub fn assign(chosen: usize, players: usize) -> Vec<usize> {
    assert!(chosen < PALETTE.len(), "no such colour");
    assert!(players >= 1, "a match needs a player");
    assert!(players <= PALETTE.len(), "more players than colours");

    let table = distances();
    let others: Vec<usize> = (0..PALETTE.len())
        .filter(|&index| index != chosen)
        .collect();

    let mut best: Option<(f32, Vec<usize>)> = None;
    let mut combination: Vec<usize> = Vec::with_capacity(players - 1);

    search(
        &others,
        0,
        players - 1,
        &mut combination,
        &mut |picked: &[usize]| {
            let mut all = Vec::with_capacity(players);
            all.push(chosen);
            all.extend_from_slice(picked);
            let gap = smallest_gap(&all, &table);
            if best.as_ref().is_none_or(|(found, _)| gap > *found) {
                best = Some((gap, all));
            }
        },
    );

    best.expect("at least one assignment exists").1
}

fn smallest_gap(picked: &[usize], table: &[[f32; 20]; 20]) -> f32 {
    let mut smallest = f32::INFINITY;
    for left in 0..picked.len() {
        for right in (left + 1)..picked.len() {
            smallest = smallest.min(table[picked[left]][picked[right]]);
        }
    }
    smallest
}

/// Every combination of `remaining` entries from `pool`, starting at `from`.
fn search(
    pool: &[usize],
    from: usize,
    remaining: usize,
    picked: &mut Vec<usize>,
    found: &mut impl FnMut(&[usize]),
) {
    if remaining == 0 {
        found(picked);
        return;
    }
    if pool.len() - from < remaining {
        return;
    }
    for index in from..pool.len() {
        picked.push(pool[index]);
        search(pool, index + 1, remaining - 1, picked, found);
        picked.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_swatch_has_readable_ink() {
        for swatch in PALETTE {
            let chosen = Swatch {
                name: "ink",
                rgb: ink(swatch),
            };
            let gap = distance(swatch, chosen);
            assert!(
                gap > 0.35,
                "{} and its ink are only {gap} apart",
                swatch.name
            );
        }
    }

    #[test]
    fn the_ink_chosen_is_always_the_further_of_the_two() {
        for swatch in PALETTE {
            let chosen = Swatch {
                name: "ink",
                rgb: ink(swatch),
            };
            let rejected = Swatch {
                name: "ink",
                rgb: if ink(swatch)[0] == 250 {
                    [16, 16, 18]
                } else {
                    [250, 250, 250]
                },
            };
            assert!(
                distance(swatch, chosen) >= distance(swatch, rejected),
                "{} took the nearer ink",
                swatch.name
            );
        }
    }

    #[test]
    fn the_darkest_swatches_take_light_ink() {
        // Black lettering on black dice was the reported defect.
        for name in ["black", "navy", "maroon", "purple", "dark green", "teal"] {
            let swatch = PALETTE.iter().find(|s| s.name == name).copied().unwrap();
            assert_eq!(ink(swatch)[0], 250, "{name} should take light ink");
        }
    }

    #[test]
    fn the_palette_is_the_six_hues_in_three_variants_plus_black_and_white() {
        assert_eq!(PALETTE.len(), 20);
        for name in [
            "red", "green", "blue", "cyan", "magenta", "yellow", "white", "black",
        ] {
            assert!(
                PALETTE.iter().any(|swatch| swatch.name == name),
                "{name} is missing"
            );
        }
    }

    #[test]
    fn every_palette_entry_is_distinct() {
        for (left, first) in PALETTE.iter().enumerate() {
            for second in PALETTE.iter().skip(left + 1) {
                assert!(
                    distance(*first, *second) > 0.0,
                    "{} and {} are the same colour",
                    first.name,
                    second.name
                );
            }
        }
    }

    #[test]
    fn the_human_always_keeps_the_colour_they_chose() {
        for chosen in 0..PALETTE.len() {
            for players in 1..=6 {
                let assigned = assign(chosen, players);
                assert_eq!(assigned[0], chosen);
                assert_eq!(assigned.len(), players);
            }
        }
    }

    #[test]
    fn a_colour_chosen_by_the_human_is_not_used_for_any_other_player() {
        let assigned = assign(1, 8);
        let mut seen = assigned.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), assigned.len());
    }

    /// The statement is that the assignment maximises the smallest difference. This checks
    /// it against every other subset rather than trusting the search.
    #[test]
    fn no_other_assignment_has_a_larger_smallest_gap() {
        let table = distances();
        for chosen in [0usize, 7, 18, 19] {
            for players in 2..=4 {
                let assigned = assign(chosen, players);
                let ours = smallest_gap(&assigned, &table);

                let others: Vec<usize> = (0..PALETTE.len())
                    .filter(|&index| index != chosen)
                    .collect();
                let mut best = 0.0f32;
                let mut combination = Vec::new();
                search(
                    &others,
                    0,
                    players - 1,
                    &mut combination,
                    &mut |picked: &[usize]| {
                        let mut all = vec![chosen];
                        all.extend_from_slice(picked);
                        best = best.max(smallest_gap(&all, &table));
                    },
                );
                assert!(
                    ours >= best - 1e-6,
                    "chose a gap of {ours} when {best} was available"
                );
            }
        }
    }

    #[test]
    fn more_players_never_widens_the_gap() {
        let table = distances();
        let mut previous = f32::INFINITY;
        for players in 2..=10 {
            let gap = smallest_gap(&assign(1, players), &table);
            assert!(
                gap <= previous + 1e-6,
                "{players} players widened the gap from {previous} to {gap}"
            );
            previous = gap;
        }
    }

    #[test]
    fn ten_players_still_fit() {
        let assigned = assign(0, 10);
        assert_eq!(assigned.len(), 10);
    }
}
