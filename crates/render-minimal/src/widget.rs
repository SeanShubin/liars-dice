//! The pieces every screen is built from.
//!
//! Each one draws a fixed size box whether or not it has anything to put in it, which is
//! what keeps dice and claims lined up in a grid across players.

use egui::{
    Align2, Button, Color32, FontId, RichText, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2,
};
use rules::Claim;

use crate::palette::{PALETTE, ink};

/// Every die is drawn in a box this size, shown or not, so the dice line up in a grid.
pub const DIE: f32 = 32.0;
/// Every claim is drawn in a box this size, filled or not, so the claims line up too.
pub const CLAIM: [f32; 2] = [58.0, 30.0];

/// A palette swatch as a colour.
pub fn colour_of(index: usize) -> Color32 {
    let [r, g, b] = PALETTE[index % PALETTE.len()].rgb;
    Color32::from_rgb(r, g, b)
}

/// Whichever of black or white reads against that swatch.
pub fn ink_of(index: usize) -> Color32 {
    let [r, g, b] = ink(PALETTE[index % PALETTE.len()]);
    Color32::from_rgb(r, g, b)
}

/// A tap target big enough for a finger.
///
/// `Every gameplay interaction is a single touch`, so every control on every screen is one
/// of these and none of them is smaller.
pub fn tap(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        Button::new(RichText::new(label).size(20.0)).min_size([64.0, 44.0].into()),
    )
    .clicked()
}

/// One die: a fixed size box in its owner's colour, with a border and a value in whichever
/// ink contrasts with that colour.
///
/// The border is what makes a black die visible against a dark background and a white one
/// visible against a light one, whatever the theme does. `face` is `None` for a die whose
/// value this screen may not see; the box is the same size either way, so the dice line up
/// in a grid whether they are shown or not.
pub fn die(ui: &mut Ui, swatch: usize, face: Option<u8>) {
    let (rect, _) = ui.allocate_exact_size(vec2(DIE, DIE), Sense::hover());
    let ink = ink_of(swatch);
    let painter = ui.painter();
    painter.rect_filled(rect, 5.0, colour_of(swatch));
    painter.rect_stroke(rect, 5.0, Stroke::new(1.5, ink), StrokeKind::Inside);
    // A question mark rather than a dot: a dot reads as a die showing one.
    let text = face.map_or_else(|| "?".to_string(), |value| value.to_string());
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(17.0),
        ink,
    );
}

/// One player's shield tokens: a shield in their colour with how many they hold.
///
/// A pentagon pointing down rather than a rounded box, because a rounded box with a
/// numeral in it is a die, and a die is what this must not be read as. `count` of zero
/// draws nothing and still takes its space, so a table where one player holds a token and
/// the others do not keeps its columns lined up.
pub fn shield(ui: &mut Ui, swatch: usize, count: u32) {
    let (rect, _) = ui.allocate_exact_size(vec2(DIE, DIE), Sense::hover());
    if count == 0 {
        return;
    }
    let body = rect.shrink(2.0);
    let shoulder = body.top() + body.height() * 0.58;
    let outline = vec![
        pos2(body.left(), body.top()),
        pos2(body.right(), body.top()),
        pos2(body.right(), shoulder),
        pos2(body.center().x, body.bottom()),
        pos2(body.left(), shoulder),
    ];
    let ink = ink_of(swatch);
    let painter = ui.painter();
    painter.add(Shape::convex_polygon(
        outline,
        colour_of(swatch),
        Stroke::new(1.5, ink),
    ));
    painter.text(
        // Above the geometric centre: the point at the bottom pulls the centre of the
        // shape down, and the number wants to sit in the wide part.
        pos2(body.center().x, body.top() + body.height() * 0.38),
        Align2::CENTER_CENTER,
        count,
        FontId::proportional(15.0),
        ink,
    );
}

/// A dialled number, centred in a box the height of a tap target.
///
/// Fixed width, so a two digit quantity does not shift the button beside it, and the same
/// height as the buttons, so the rows line up vertically as well as across.
pub fn dialled_value(ui: &mut Ui, value: u32) {
    let (rect, _) = ui.allocate_exact_size(vec2(48.0, 44.0), Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        value,
        FontId::proportional(24.0),
        ui.visuals().text_color(),
    );
}

/// A claim in its claimer's colour.
///
/// The grid position already says who claimed, so the claimer's dice do not have to be
/// drawn beside it. An empty cell still takes its space, which is what keeps the columns
/// lined up.
pub fn claim_box(ui: &mut Ui, swatch: usize, claim: Option<Claim>) {
    let (rect, _) = ui.allocate_exact_size(vec2(CLAIM[0], CLAIM[1]), Sense::hover());
    let Some(claim) = claim else {
        return;
    };
    let ink = ink_of(swatch);
    let painter = ui.painter();
    painter.rect_filled(rect, 5.0, colour_of(swatch));
    painter.rect_stroke(rect, 5.0, Stroke::new(1.5, ink), StrokeKind::Inside);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        format!("{} x {}", claim.quantity, claim.face),
        FontId::proportional(15.0),
        ink,
    );
}

/// The whole palette as swatches, with the chosen one drawn larger.
///
/// Used wherever a colour is picked: a desktop game's setup screen, and a hotseat lobby
/// seating one player at a time.
pub fn palette_picker(ui: &mut Ui, chosen: &mut usize) {
    egui::Grid::new("palette")
        .spacing([6.0, 6.0])
        .show(ui, |ui| {
            for (index, swatch) in PALETTE.iter().enumerate() {
                let selected = index == *chosen;
                let size = if selected { 52.0 } else { 44.0 };
                if ui
                    .add(
                        Button::new("")
                            .fill(colour_of(index))
                            .min_size([size, size].into()),
                    )
                    .on_hover_text(swatch.name)
                    .clicked()
                {
                    *chosen = index;
                }
                if index % 5 == 4 {
                    ui.end_row();
                }
            }
        });
}
