use std::{collections::HashSet, net::SocketAddr};

use bytes::Bytes;
use tokio::sync::mpsc;

/// How far through the handshake a connection has progressed.
///
/// Ordered, so a handler can require "at least this far" with a comparison. Packets that arrive out
/// of order are dropped rather than trusted: a client is free to send anything at any time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConnState {
    /// Socket accepted; the version string has not been checked yet.
    Greeting,
    /// Version accepted and a slot assigned.
    SlotAssigned,
    /// Appearance received, so the player has a name.
    Identified,
    /// Packet 7 sent.
    WorldSent,
    /// Sections streamed and `StartPlaying` sent.
    TilesSent,
    /// Spawned and visible to everyone else.
    Playing,
}

/// Server-side view of one connected client.
pub struct Player {
    pub slot: u8,
    pub addr: SocketAddr,
    pub out: mpsc::Sender<Bytes>,
    pub state: ConnState,
    pub name: String,
    pub uuid: Option<String>,
    pub position: (f32, f32),
    /// How fast they were moving at the last position update.
    ///
    /// Derived rather than received: the client sends a position and a velocity, but the velocity
    /// it sends is its own idea of the frame, so tracking the difference between updates is what
    /// the routines that lead their target actually want.
    pub velocity: (f32, f32),
    pub life: i16,
    pub life_max: i16,
    /// Ticks of invulnerability left after a hit.
    ///
    /// Without this a player standing in a zombie would take sixty hits a second. The game gives
    /// everyone a brief grace period after any hit, and it is what makes contact damage survivable.
    pub immune_ticks: i32,
    pub mana: i16,
    pub mana_max: i16,
    pub team: u8,
    /// The client's own packet 4 payload, replayed verbatim to describe this player to others.
    ///
    /// Appearance carries dozens of fields (dyes, accessory visibility, per-slot colours) that the
    /// server has no reason to model; relaying the original bytes keeps every one of them intact.
    pub appearance: Option<Bytes>,
    /// The most recent packet 13 payload, so a joining player sees everyone in their real pose.
    pub last_controls: Option<Bytes>,
    /// Whether a valid `Hello` has been seen.
    ///
    /// The password exchange happens while the connection is still in `Greeting`, so without this
    /// a client could skip the version check by sending only a password.
    pub greeted: bool,
    /// Whether this connection has cleared the server password, if one is set.
    pub password_ok: bool,
    /// Whether this player has PvP enabled.
    pub pvp: bool,
    /// The client's own buff packet, replayed so others see the same buff icons.
    pub buffs: Option<Bytes>,
    /// The client's last biome-zone packet.
    pub zone: Option<Bytes>,
    /// Which chest this player currently has open, or -1 for none.
    ///
    /// Vanilla refuses to open a chest another player is already in, so the server has to know.
    pub open_chest: i16,
    /// Sections already streamed to this client.
    ///
    /// Since 1.4.5 the client pulls sections with packet 159 as it moves, rather than the server
    /// pushing them from player positions. It re-asks freely, so this is what stops a walk back
    /// and forth from resending megabytes of tiles.
    pub sent_sections: HashSet<(i32, i32)>,
}

impl Player {
    pub fn new(slot: u8, addr: SocketAddr, out: mpsc::Sender<Bytes>) -> Self {
        Self {
            slot,
            addr,
            out,
            state: ConnState::Greeting,
            name: format!("Player {slot}"),
            uuid: None,
            position: (0.0, 0.0),
            velocity: (0.0, 0.0),
            life: 100,
            immune_ticks: 0,
            life_max: 100,
            mana: 20,
            mana_max: 20,
            team: 0,
            appearance: None,
            last_controls: None,
            greeted: false,
            password_ok: false,
            pvp: false,
            buffs: None,
            zone: None,
            open_chest: -1,
            sent_sections: HashSet::new(),
        }
    }

    /// Whether this player is in the world and should receive and generate broadcasts.
    pub fn is_playing(&self) -> bool {
        self.state == ConnState::Playing
    }

    /// Advance the handshake, never backwards.
    ///
    /// A client that re-sends an earlier packet must not be able to rewind its own state and
    /// replay a stage.
    pub fn advance_to(&mut self, state: ConnState) {
        if state > self.state {
            self.state = state;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_player() -> Player {
        let (tx, _rx) = mpsc::channel(4);
        Player::new(1, "127.0.0.1:1".parse().unwrap(), tx)
    }

    #[test]
    fn state_only_moves_forward() {
        let mut p = test_player();
        p.advance_to(ConnState::WorldSent);
        assert_eq!(p.state, ConnState::WorldSent);

        p.advance_to(ConnState::SlotAssigned);
        assert_eq!(p.state, ConnState::WorldSent, "state must not rewind");
    }

    #[test]
    fn a_fresh_player_is_not_playing() {
        assert!(!test_player().is_playing());
    }

    #[test]
    fn states_are_ordered_along_the_handshake() {
        assert!(ConnState::Greeting < ConnState::SlotAssigned);
        assert!(ConnState::SlotAssigned < ConnState::Identified);
        assert!(ConnState::Identified < ConnState::WorldSent);
        assert!(ConnState::WorldSent < ConnState::TilesSent);
        assert!(ConnState::TilesSent < ConnState::Playing);
    }
}
