//! In-memory [`Log`] and [`Connections`], so a whole match runs in one process.
//!
//! This is what lets multiplayer be proved before any infrastructure exists, and it is
//! the simulated environment /specification/architecture.md asks for: with a fixed secret
//! and a fixed message order, a match replays exactly.

use std::collections::BTreeMap;

use protocol::{Event, Seat};

use crate::{Connected, ConnectionId, Connections, Log, MatchId, Role, Taken};

#[derive(Default)]
pub struct MemoryLog {
    entries: BTreeMap<MatchId, Vec<Event>>,
}

impl MemoryLog {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Log for MemoryLog {
    fn append(&mut self, match_id: &MatchId, position: u64, event: &Event) -> Result<(), Taken> {
        let entries = self.entries.entry(match_id.clone()).or_default();
        if position != entries.len() as u64 {
            // Something is already at that position, or there is a hole before it. Either
            // way the caller's idea of the end of the log is stale.
            return Err(Taken);
        }
        entries.push(event.clone());
        Ok(())
    }

    fn read(&self, match_id: &MatchId) -> Vec<Event> {
        self.entries.get(match_id).cloned().unwrap_or_default()
    }

    fn matches(&self) -> Vec<MatchId> {
        self.entries.keys().cloned().collect()
    }
}

#[derive(Default)]
pub struct MemoryConnections {
    rows: Vec<(MatchId, Connected)>,
}

impl MemoryConnections {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Connections for MemoryConnections {
    fn of(&self, match_id: &MatchId) -> Vec<Connected> {
        self.rows
            .iter()
            .filter(|(id, _)| id == match_id)
            .map(|(_, connected)| connected.clone())
            .collect()
    }

    fn attach(&mut self, match_id: &MatchId, connection: ConnectionId, seat: Seat, role: Role) {
        self.rows
            .retain(|(_, connected)| connected.connection != connection);
        self.rows.push((
            match_id.clone(),
            Connected {
                connection,
                seat,
                role,
            },
        ));
    }

    fn detach(&mut self, connection: &ConnectionId) {
        self.rows
            .retain(|(_, connected)| &connected.connection != connection);
    }
}
