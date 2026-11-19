//! Who sits where.
//!
//! A [`rules::View`] carries the match. It does not carry the two things a screen needs to
//! draw a seat — what to call it and what colour its dice are — because neither is a rule
//! and neither affects play. This is that, and nothing else.

/// The names and dice colours of every seat, by seat number.
///
/// Built by whatever composed the match: a desktop game names one seat "You" and the rest
/// after its opponent list, a hotseat match names each seat as its player was seated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seating {
    names: Vec<String>,
    /// Indices into [`crate::palette::PALETTE`].
    colours: Vec<usize>,
}

impl Seating {
    /// # Panics
    /// If there is not exactly one colour per name. Two seats may share neither, so a
    /// mismatch is a composition bug rather than something a screen should cope with.
    pub fn new(names: Vec<String>, colours: Vec<usize>) -> Self {
        assert_eq!(
            names.len(),
            colours.len(),
            "every seat needs a name and a colour"
        );
        Self { names, colours }
    }

    /// How many seats. The number of players in the match, including those knocked out.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// What to call a seat. A seat past the end reads as its number, so a screen renders
    /// something rather than panicking if the two ever disagree.
    pub fn name(&self, seat: usize) -> &str {
        self.names.get(seat).map_or("?", String::as_str)
    }

    /// Which palette swatch a seat's dice are drawn in.
    pub fn colour(&self, seat: usize) -> usize {
        self.colours.get(seat).copied().unwrap_or(0)
    }
}
