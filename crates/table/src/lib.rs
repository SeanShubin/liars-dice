//! A whole networked match in one process.
//!
//! The relay, every client and every driver, with the real protocol between them and
//! nothing simulated but the transport. This is where multiplayer is proved before any
//! infrastructure exists, which keeps the pipeline off the critical path.
//!
//! The arrangement is reusable rather than a composition root of its own: `apps/local`
//! plays a match at a terminal and `apps/hotseat` plays one in a browser, and both want
//! the same table. Swapping `relay::memory` for a transport that speaks over a socket is
//! the only difference between this and a real networked match.

pub use coordinator::{Client, Decides, Intent};
use integrity::Secret;
use protocol::{Account, Event, PROTOCOL_VERSION, Seat};
use relay::memory::{MemoryConnections, MemoryLog};
use relay::{ConnectionId, Log, MatchId, Message, Relay};
use rules::{Call, View};

/// A seat at the table: a client, and whatever is driving it.
pub struct Player {
    pub connection: ConnectionId,
    pub client: Client,
    pub driver: Box<dyn Decides>,
}

/// An AI opponent, deciding from what its seat may see.
pub struct Ai;

impl Decides for Ai {
    fn choose(&mut self, view: &View) -> Option<Call> {
        Some(ai::choose(view))
    }
}

/// A fixed sequence of calls, for a test that wants an exact match.
///
/// Once the script runs out it takes the lowest legal raise, and calls liar when there is
/// no raise left. Without that last part a table of these never ends a round: every seat
/// raises by the smallest step until no raise is legal, and then nobody disputes.
pub struct Scripted {
    pub calls: Vec<Call>,
    pub at: usize,
}

impl Decides for Scripted {
    fn choose(&mut self, view: &View) -> Option<Call> {
        let scripted = self.calls.get(self.at).copied();
        self.at += 1;
        if let Some(call) = scripted
            && view.public().check(call).is_ok()
        {
            return Some(call);
        }
        match view.legal_raises().first() {
            Some(&claim) => Some(Call::Raise(claim)),
            None if view.previous_claim().is_some() => Some(Call::Liar),
            None => None,
        }
    }
}

/// A table: one relay, and every player at it.
pub struct Table {
    pub relay: Relay<MemoryLog, MemoryConnections>,
    pub players: Vec<Player>,
    pub match_id: MatchId,
}

impl Table {
    /// Host a match, seat everyone, and start it.
    pub fn seat(secret: Secret, drivers: Vec<(String, Box<dyn Decides>)>) -> Self {
        let mut relay = Relay::new(MemoryLog::new(), MemoryConnections::new(), secret);
        let colours = ["red", "blue", "green", "white", "black"];
        let mut players = Vec::new();
        let mut match_id: Option<String> = None;

        for (at, (name, driver)) in drivers.into_iter().enumerate() {
            let connection = ConnectionId(format!("connection-{at}"));
            let account = Account::Discord(format!("account-{at}"));
            let colour = colours[at % colours.len()].to_string();
            let outcome = match &match_id {
                None => relay.receive(
                    &connection,
                    Message::Host {
                        account,
                        name,
                        colour,
                        version: PROTOCOL_VERSION,
                    },
                ),
                Some(id) => relay.receive(
                    &connection,
                    Message::Join {
                        match_id: MatchId(id.to_string()),
                        account,
                        name,
                        colour,
                        version: PROTOCOL_VERSION,
                    },
                ),
            };
            assert_eq!(outcome.refused, None, "a player could not be seated");
            if let Some(created) = outcome.created {
                match_id = Some(created.0.clone());
            }
            players.push(Player {
                connection,
                client: Client::new(Seat(at as u8)),
                driver,
            });
        }

        let match_id = MatchId(match_id.expect("a match was created"));
        let mut table = Self {
            relay,
            players,
            match_id,
        };
        let host = table.players[0].connection.clone();
        let started = table.relay.receive(
            &host,
            Message::Start {
                match_id: table.match_id.clone(),
            },
        );
        assert_eq!(started.refused, None, "the match could not be started");
        table.deliver(started.sends);
        table
    }

    /// Hand each event to the client it was addressed to, and send on whatever that
    /// client asks for. Repeats until nothing is left to deliver.
    pub fn deliver(&mut self, mut sends: Vec<(ConnectionId, Event)>) {
        let mut guard = 0;
        while !sends.is_empty() {
            guard += 1;
            assert!(guard < 10_000, "the table would not settle");
            let mut intents: Vec<(usize, Intent)> = Vec::new();
            for (to, event) in sends.drain(..) {
                for (at, player) in self.players.iter_mut().enumerate() {
                    if player.connection == to {
                        for intent in player.client.receive(&event) {
                            intents.push((at, intent));
                        }
                    }
                }
            }
            for (at, intent) in intents {
                let outcome = self.send(at, intent);
                sends.extend(outcome);
            }
        }
    }

    fn send(&mut self, player: usize, intent: Intent) -> Vec<(ConnectionId, Event)> {
        let connection = self.players[player].connection.clone();
        let message = match intent {
            Intent::Claim(event) => Message::Claim {
                match_id: self.match_id.clone(),
                event,
            },
            Intent::RoundOver => Message::RoundOver {
                match_id: self.match_id.clone(),
            },
        };
        self.relay.receive(&connection, message).sends
    }

    /// Settle any round that can be settled, and begin the next.
    fn settle_rounds(&mut self) {
        let mut opens = None;
        for player in self.players.iter_mut() {
            if let Some(resolution) = player.client.settle() {
                opens = Some((resolution.opens, resolution.over));
            }
        }
        if let Some((opens, over)) = opens
            && !over
        {
            for player in self.players.iter_mut() {
                player.client.begin_next_round(opens);
            }
        }
    }

    /// Whether the match has ended.
    pub fn over(&self) -> bool {
        self.players
            .first()
            .and_then(|player| player.client.public())
            .is_some_and(|public| public.over())
    }

    /// Ask whoever's turn it is for a call, send it, and settle whatever follows.
    ///
    /// Returns false when nobody could move, which ends the match or exposes a stall.
    pub fn take_turn(&mut self) -> bool {
        self.settle_rounds();
        if self.over() {
            return false;
        }
        let Some(at) = (0..self.players.len()).find(|&at| self.players[at].client.is_my_turn())
        else {
            return false;
        };
        // The view borrows the client and the driver is a sibling field, so the
        // choice is made in its own scope and the view is gone before the client is
        // asked to act.
        let Some((call, odds)) = ({
            let player = &mut self.players[at];
            match player.client.view() {
                Some(view) => player
                    .driver
                    .choose(&view)
                    .map(|call| (call, odds_of(&view, call))),
                None => None,
            }
        }) else {
            return false;
        };
        let Ok(intent) = self.players[at].client.act(call, odds) else {
            return false;
        };
        let sends = self.send(at, intent);
        self.deliver(sends);
        self.settle_rounds();
        true
    }

    /// Play until the match ends or the limit is reached.
    pub fn play(&mut self, limit: usize) -> usize {
        let mut turns = 0;
        while turns < limit && self.take_turn() {
            turns += 1;
        }
        turns
    }

    pub fn log(&self) -> Vec<Event> {
        self.relay.log().read(&self.match_id)
    }
}

/// How likely the call is, from what the seat making it can see. A display value only:
/// it is written to the log for a person to read and never read back into a decision.
fn odds_of(view: &View, call: Call) -> f64 {
    use ai::Seen;
    let chance = match call {
        Call::Raise(claim) => view.chance_of(claim),
        Call::Liar => match view.previous_claim() {
            Some(claim) => view.chance_of(claim).complement(),
            None => return 0.0,
        },
        Call::SpotOn => match view.previous_claim() {
            Some(claim) => view.chance_exactly(claim),
            None => return 0.0,
        },
    };
    readout::percent(&chance)
}

#[cfg(test)]
mod tests;
