//! In-memory [`Log`] and [`Connections`], so a whole match runs in one process.
//!
//! This is what lets multiplayer be proved before any infrastructure exists, and it is
//! the simulated environment /specification/architecture.md asks for: with a fixed secret
//! and a fixed message order, a match replays exactly.

use std::collections::BTreeMap;

use protocol::{Event, Moment, Seat};

use crate::{Connected, ConnectionId, Connections, Log, MatchId, Recording, Role, Taken};

/// One match's events, and when the last of them was recorded.
///
/// One entry rather than two maps, because a map of events and a map of times are two
/// things that have to agree about which matches exist.
#[derive(Default)]
struct Entry {
    events: Vec<Event>,
    last: Option<Moment>,
}

#[derive(Default)]
pub struct MemoryLog {
    entries: BTreeMap<MatchId, Entry>,
}

impl MemoryLog {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Log for MemoryLog {
    fn append(
        &mut self,
        match_id: &MatchId,
        from: u64,
        events: &[Event],
        now: Moment,
    ) -> Result<(), Taken> {
        let entry = self.entries.entry(match_id.clone()).or_default();
        if from != entry.events.len() as u64 {
            // Something is already at that position, or there is a hole before it. Either
            // way the caller's idea of the end of the log is stale, and so is whatever it
            // decided from reading it.
            return Err(Taken);
        }
        // Checked before anything is pushed, so this is all or nothing without needing to
        // undo a partial write.
        entry.events.extend(events.iter().cloned());
        // The stamp belongs to the rows this call wrote, so an append of nothing leaves
        // the match as inactive as it was. Otherwise a message that decided to write
        // nothing would keep a dead match alive.
        if !events.is_empty() {
            entry.last = Some(now);
        }
        Ok(())
    }

    fn read(&self, match_id: &MatchId) -> Recording {
        match self.entries.get(match_id) {
            Some(entry) => Recording {
                events: entry.events.clone(),
                last: entry.last,
            },
            None => Recording {
                events: Vec::new(),
                last: None,
            },
        }
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
        // This socket leaves wherever else it was, and whoever else was on this seat
        // leaves it. See the contract on `Connections::attach`.
        self.rows.retain(|(id, connected)| {
            connected.connection != connection && !(id == match_id && connected.seat == seat)
        });
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
