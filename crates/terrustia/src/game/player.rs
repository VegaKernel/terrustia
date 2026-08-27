use std::{
    collections::{HashSet, VecDeque},
    net::SocketAddr,
};

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
    /// Whether this player is currently sitting (packet 13's own `bitsByte26[2]`).
    ///
    /// Real vanilla checks this every frame a player sits (`PlayerSittingHelper.UpdateSitting`) to
    /// see whether they are on the one specific chair that turns the nearby Clothier into a
    /// red-hatted Skeletron; nothing else in this project reads it yet.
    pub sitting: bool,
    /// Which hotbar slot (0-9) is currently selected, from the same packet.
    ///
    /// Paired with `inventory` to answer "what item is this player currently holding" — needed for
    /// the same red-hat Skeletron check above, which only fires while the selected item is the
    /// Clothier Voodoo Doll.
    pub selected_item: u8,
    /// Which town NPC this player has open, if any. A shop needs to know.
    pub talking_to: Option<u8>,
    /// Which way this player is looking.
    ///
    /// Only a wiring tool reads it, and only to decide which way its path turns the corner — but
    /// getting that wrong lays the wire along the wrong two sides of the rectangle, which is
    /// obvious the moment it happens.
    pub facing_right: bool,
    /// How many of the Angler's quests this character has finished.
    ///
    /// Character state rather than world state, so the server remembers what it is told and
    /// passes it on. The Angler's reward tiers are gated on it, which is why it has to reach
    /// every client rather than staying with the one that owns the character.
    pub angler_quests: i32,
    /// ...and their accumulated golf score, which travels in the same message.
    pub golf_score: i32,
    /// What this player is carrying, by slot.
    ///
    /// Sparse: a client sends a slot only when it holds something or when it has just been
    /// emptied, and the great majority of a player's three hundred and ninety-five slots are empty
    /// for their whole session. Storing the whole array per player would be most of a megabyte of
    /// nothing across a full server.
    pub inventory: std::collections::HashMap<u16, terrustia_proto::inventory::SyncEquipment>,
    /// Whether a valid `Hello` has been seen.
    ///
    /// The password exchange happens while the connection is still in `Greeting`, so without this
    /// a client could skip the version check by sending only a password.
    pub greeted: bool,
    /// Whether this connection has cleared the server password, if one is set.
    pub password_ok: bool,
    /// Whether this player has PvP enabled.
    pub pvp: bool,
    /// Vanilla's tile-edit spam counters, which this server was missing entirely.
    ///
    /// `RemoteClient` keeps a float per kind, bumps it on every edit packet, decays it each tick,
    /// and boots the connection past a ceiling — 100 for placing, 500 for breaking, 50 for
    /// liquid. Not having them was a *regression from vanilla*, not merely a place where we are
    /// as trusting as vanilla is, which is why it belongs inside "match vanilla's trust model"
    /// rather than outside it.
    ///
    /// Floats because the decay rates are fractional (0.3 a tick for placing) and rounding them
    /// to integers would change how long a burst is tolerated.
    pub spam_place: f32,
    pub spam_break: f32,
    pub spam_liquid: f32,
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
    /// Sections still owed to this client from its initial world stream, drained a few at a time
    /// off the tick by `drain_section_streams` rather than sent in one synchronous burst inside the
    /// `SpawnTileData` packet handler — a first join can want up to ~39 of them, and sending them
    /// all in one call used to block every other player's tick for the whole burst.
    pub pending_sections: VecDeque<(i32, i32)>,
}

impl Player {
    /// Whether this player is on the same machine as the server.
    ///
    /// The game's whole test for who counts as the host, in `NetMessage.
    /// DoesPlayerSlotCountAsAHost`, is whether the socket's far end is the loopback address. It
    /// gates packet 139, and through it a handful of things the client treats as the host's to
    /// decide.
    pub fn is_local(&self) -> bool {
        self.addr.ip().is_loopback()
    }

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
            sitting: false,
            selected_item: 0,
            talking_to: None,
            facing_right: true,
            angler_quests: 0,
            golf_score: 0,
            inventory: std::collections::HashMap::new(),
            greeted: false,
            password_ok: false,
            pvp: false,
            spam_place: 0.0,
            spam_break: 0.0,
            spam_liquid: 0.0,
            buffs: None,
            zone: None,
            open_chest: -1,
            sent_sections: HashSet::new(),
            pending_sections: VecDeque::new(),
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
