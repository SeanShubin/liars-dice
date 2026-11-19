//! The hotseat harness: a whole match in one browser, every seat driven by a virtual
//! player, so the deployment can be exercised without a second person.
//!
//! Composition root. The table is `crates/table`, the rules are `crates/rules`, and the
//! only thing this adds is a surface to watch it through and a button to step it.
//!
//! Hotseat is a harness, not a game. Hidden information makes it nonsense to play
//! seriously on one screen — which is why it shows every seat at once rather than
//! pretending otherwise. What it proves is that the hosting, the build and the match all
//! work, which is everything except a second human and a socket.

use eframe::egui;
use integrity::Secret;
use rules::{Call, Public};
use table::{Ai, Decides, Scripted, Table};

const SEATS: usize = 3;

struct Harness {
    table: Table,
    secret: u8,
    log_shown: usize,
}

impl Harness {
    fn new(secret: u8) -> Self {
        Self {
            table: seat(secret),
            secret,
            log_shown: 12,
        }
    }

    fn restart(&mut self, secret: u8) {
        self.secret = secret;
        self.table = seat(secret);
    }
}

fn seat(secret: u8) -> Table {
    let names = ["Alice", "Bob", "Carol", "Dave", "Erin"];
    Table::seat(
        Secret::from_bytes([secret; 32]),
        (0..SEATS)
            .map(|at| {
                let driver: Box<dyn Decides> = if at == 0 {
                    Box::new(Scripted {
                        calls: Vec::new(),
                        at: 0,
                    })
                } else {
                    Box::new(Ai)
                };
                (names[at].to_string(), driver)
            })
            .collect(),
    )
}

impl eframe::App for Harness {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Liars Dice — hotseat harness");
            ui.label(
                "Every seat is a virtual player. Nothing here is networked: the relay, \
                 the clients and the drivers all run in this tab.",
            );
            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("Take a turn").clicked() {
                    self.table.take_turn();
                }
                if ui.button("Play to the end").clicked() {
                    self.table.play(5_000);
                }
                if ui.button("New match").clicked() {
                    let next = self.secret.wrapping_add(1);
                    self.restart(next);
                }
                ui.label(format!("match secret {}", self.secret));
            });

            ui.separator();
            match self.table.players[0].client.public() {
                Some(public) => table_of(ui, &self.table, public),
                None => {
                    ui.label("the match has not started");
                }
            }

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Event log").strong());
                ui.add(egui::Slider::new(&mut self.log_shown, 4..=60).text("lines"));
            });
            let log = self.table.log();
            let from = log.len().saturating_sub(self.log_shown);
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for event in &log[from..] {
                        ui.label(egui::RichText::new(event.to_string()).monospace());
                    }
                });
        });
    }
}

/// Every seat, as that seat sees itself. A harness shows all of them at once; a game
/// never would.
fn table_of(ui: &mut egui::Ui, table: &Table, public: &Public) {
    ui.horizontal(|ui| {
        ui.label(format!("round {}", public.round()));
        if public.over() {
            ui.label(
                egui::RichText::new(format!("winner: seat {}", public.ranking()[0]))
                    .strong()
                    .color(egui::Color32::from_rgb(120, 200, 120)),
            );
        } else {
            ui.label(format!("seat {} to claim", public.current()));
        }
        ui.label(format!("{} dice in play", public.dice_in_play()));
    });

    egui::Grid::new("seats")
        .spacing([14.0, 6.0])
        .show(ui, |ui| {
            ui.label(egui::RichText::new("seat").strong());
            ui.label(egui::RichText::new("dice").strong());
            ui.label(egui::RichText::new("shields").strong());
            ui.label(egui::RichText::new("its own view").strong());
            ui.end_row();

            for player in &table.players {
                let seat = player.client.seat().0 as usize;
                let mine = player
                    .client
                    .view()
                    .and_then(|view| view.own_dice().map(|dice| dice.to_vec()))
                    .unwrap_or_default();
                let marker = if public.current() == seat && !public.over() {
                    "▶ "
                } else {
                    "  "
                };
                ui.label(format!("{marker}{seat}"));
                ui.label(format!("{}", public.count(seat)));
                ui.label(format!("{}", public.shields(seat)));
                ui.label(
                    egui::RichText::new(if public.eliminated(seat) {
                        "out".to_string()
                    } else {
                        mine.iter()
                            .map(|face| face.to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .monospace(),
                );
                ui.end_row();
            }
        });

    ui.add_space(6.0);
    ui.label(egui::RichText::new("This round").strong());
    if public.history().is_empty() {
        ui.label("no claims yet");
    }
    for entry in public.history() {
        ui.label(
            egui::RichText::new(format!(
                "seat {} claimed at least {} × {}",
                entry.player, entry.claim.quantity, entry.claim.face
            ))
            .monospace(),
        );
    }
    if let Some(claim) = public.previous_claim() {
        ui.label(format!(
            "to beat: {} × {}  ({} legal raises)",
            claim.quantity,
            claim.face,
            public.legal_raises().len()
        ));
    }
    let _ = Call::Liar;
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    eframe::run_native(
        "Liars Dice hotseat",
        eframe::NativeOptions::default(),
        Box::new(|_| Ok(Box::new(Harness::new(11)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast;

    let document = web_sys::window()
        .expect("a window")
        .document()
        .expect("a document");
    let canvas = document
        .get_element_by_id("table")
        .expect("the page has a canvas called table")
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .expect("that element is a canvas");

    wasm_bindgen_futures::spawn_local(async {
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|_| Ok(Box::new(Harness::new(11)))),
            )
            .await
            .expect("the harness started");
    });
}
