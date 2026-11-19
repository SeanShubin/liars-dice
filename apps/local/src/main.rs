//! A whole networked match, played at a terminal.
//!
//! Composition root. No behavior: the table is `crates/table`.

use integrity::Secret;
use table::{Ai, Decides, Table};

fn main() {
    let mut table = Table::seat(
        Secret::from_bytes([11u8; 32]),
        vec![
            ("Alice".to_string(), Box::new(Ai) as Box<dyn Decides>),
            ("Bob".to_string(), Box::new(Ai)),
            ("Carol".to_string(), Box::new(Ai)),
        ],
    );
    let turns = table.play(5_000);
    let public = table.players[0]
        .client
        .public()
        .expect("the match started")
        .clone();
    println!("{turns} turns, {} events", table.log().len());
    println!("ranking: {:?}", public.ranking());
}
