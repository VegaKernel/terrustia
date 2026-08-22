//! Typed packets for the connection handshake and player sync.
//!
//! Field order is transcribed from the 1.4.5.7 client's `MessageBuffer.GetData` and
//! `NetMessage.SendData`; see `docs/protocol-notes.md`. Reordering anything here does not produce
//! an error at the client, it produces a silent hang, so the golden-byte tests below pin the
//! layouts rather than trusting the round-trips alone.

use crate::{
    error::{ProtoError, Result},
    id,
    net_text::NetworkText,
    reader::PacketReader,
    writer::{PacketWriter, Writer},
};

/// Packet `1`: the first thing a client sends, carrying its protocol version string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub version: String,
}

impl Hello {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        Ok(Self {
            version: r.string()?,
        })
    }

    /// Whether this client speaks the release this server implements.
    pub fn is_supported(&self) -> bool {
        self.version == id::VERSION_STRING
    }
}

/// Packet `2`: disconnect with a reason the client displays.
pub fn kick(reason: &NetworkText) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::KICK);
    reason.write(&mut w);
    w.finish()
}

/// Packet `3`: assign the client its player slot.
///
/// The trailing bool is new in 1.4.5; a 1.4.4 client expected only the slot byte.
pub fn player_info(slot: u8, check_bytes_in_client_loop: bool) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::PLAYER_INFO);
    w.u8(slot).bool(check_bytes_in_client_loop);
    w.finish()
}

/// Packet `8`: the client asking for the tiles around a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnTileData {
    pub x: i32,
    pub y: i32,
    pub team: u8,
}

impl SpawnTileData {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        Ok(Self {
            x: r.i32()?,
            y: r.i32()?,
            team: r.u8()?,
        })
    }
}

/// Packet `9`: extends the client's loading bar and sets its caption.
pub fn status_text(steps: i32, text: &NetworkText, flags: u8) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::STATUS_TEXT_SIZE);
    w.i32(steps);
    text.write(&mut w);
    w.u8(flags);
    w.finish()
}

/// Packet `12`: where and how a player enters the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerSpawn {
    pub player: u8,
    pub spawn_x: i16,
    pub spawn_y: i16,
    pub respawn_timer: i32,
    pub deaths_pve: i16,
    pub deaths_pvp: i16,
    pub team: u8,
    pub context: u8,
}

impl PlayerSpawn {
    /// `PlayerSpawnContext.SpawningIntoWorld`, the context used when first joining.
    pub const CONTEXT_SPAWNING_INTO_WORLD: u8 = 1;

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        Ok(Self {
            player: r.u8()?,
            spawn_x: r.i16()?,
            spawn_y: r.i16()?,
            respawn_timer: r.i32()?,
            deaths_pve: r.i16()?,
            deaths_pvp: r.i16()?,
            team: r.u8()?,
            context: r.u8()?,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::PLAYER_SPAWN);
        w.u8(self.player)
            .i16(self.spawn_x)
            .i16(self.spawn_y)
            .i32(self.respawn_timer)
            .i16(self.deaths_pve)
            .i16(self.deaths_pvp)
            .u8(self.team)
            .u8(self.context);
        w.finish()
    }
}

/// Packet `14`: announce that a player slot is occupied, or has been vacated.
pub fn player_active(player: u8, active: bool) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::PLAYER_ACTIVE);
    w.u8(player).bool(active);
    w.finish()
}

/// Packet `16`: current and maximum life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerHealth {
    pub player: u8,
    pub life: i16,
    pub life_max: i16,
}

impl PlayerHealth {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        Ok(Self {
            player: r.u8()?,
            life: r.i16()?,
            life_max: r.i16()?,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::PLAYER_LIFE_MANA);
        w.u8(self.player).i16(self.life).i16(self.life_max);
        w.finish()
    }
}

/// Packet `42`: current and maximum mana.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerMana {
    pub player: u8,
    pub mana: i16,
    pub mana_max: i16,
}

impl PlayerMana {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        Ok(Self {
            player: r.u8()?,
            mana: r.i16()?,
            mana_max: r.i16()?,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::PLAYER_MANA);
        w.u8(self.player).i16(self.mana).i16(self.mana_max);
        w.finish()
    }
}

/// Packet `18`: world clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSet {
    pub day_time: bool,
    pub time: i32,
    pub sun_mod_y: i16,
    pub moon_mod_y: i16,
}

impl TimeSet {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::SET_TIME);
        w.bool(self.day_time)
            .i32(self.time)
            .i16(self.sun_mod_y)
            .i16(self.moon_mod_y);
        w.finish()
    }
}

/// Packets `49` and `129`: both are bare signals with no payload.
pub fn empty(message_id: u8) -> Result<Vec<u8>> {
    PacketWriter::new(message_id).finish()
}

/// Packet `13`: a client's control and position update.
///
/// Only the fields the server actually needs are parsed. The trailing optional blocks (velocity,
/// mount, potion-of-return positions, camera target) depend on flag bits, so the payload is
/// re-broadcast verbatim rather than re-serialised — see [`rewrite_owner`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerControls {
    pub player: u8,
    pub control_flags: [u8; 4],
    pub selected_item: u8,
    pub position: (f32, f32),
    pub velocity: Option<(f32, f32)>,
}

impl PlayerControls {
    /// Bit 2 of the second control byte says a velocity pair follows.
    const HAS_VELOCITY: u8 = 0x04;

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        let player = r.u8()?;
        let control_flags = [r.u8()?, r.u8()?, r.u8()?, r.u8()?];
        let selected_item = r.u8()?;
        let position = r.vec2()?;
        let velocity = if control_flags[1] & Self::HAS_VELOCITY != 0 {
            Some(r.vec2()?)
        } else {
            None
        };
        Ok(Self {
            player,
            control_flags,
            selected_item,
            position,
            velocity,
        })
    }

    pub fn facing_right(&self) -> bool {
        self.control_flags[0] & 0x40 != 0
    }
}

/// Rewrite the leading player-slot byte of a payload so a relayed packet is attributed to the
/// sender rather than to whatever slot the client claimed.
///
/// Clients are not trusted to report their own slot; every relayed packet goes through this.
pub fn rewrite_owner(message_id: u8, payload: &[u8], owner: u8) -> Result<Vec<u8>> {
    if payload.is_empty() {
        return Err(ProtoError::Eof {
            offset: 0,
            needed: 1,
            available: 0,
        });
    }
    let mut w = PacketWriter::new(message_id);
    w.u8(owner).bytes(&payload[1..]);
    w.finish()
}

/// Named positions inside the eleven world-flag bytes of packet `7`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorldFlags(pub [u8; 11]);

impl WorldFlags {
    fn set(&mut self, byte: usize, bit: u8, on: bool) {
        if on {
            self.0[byte] |= 1 << bit;
        } else {
            self.0[byte] &= !(1 << bit);
        }
    }

    /// Selects crimson over corruption as the world's evil biome.
    pub fn set_crimson(&mut self, on: bool) {
        self.set(1, 5, on);
    }

    pub fn set_hardmode(&mut self, on: bool) {
        self.set(0, 4, on);
    }

    /// Server-side characters change how the client treats its inventory; leave this off unless
    /// the server actually stores player data.
    pub fn set_server_side_character(&mut self, on: bool) {
        self.set(0, 6, on);
    }
}

/// Packet `7`: everything the client needs to know about the world before tiles arrive.
///
/// The payload is exactly 159 bytes plus the encoded world name and any extra spawn points.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldData {
    pub time: i32,
    pub day_time: bool,
    pub blood_moon: bool,
    pub eclipse: bool,
    pub moon_phase: u8,
    pub max_tiles_x: i16,
    pub max_tiles_y: i16,
    pub spawn_tile_x: i16,
    pub spawn_tile_y: i16,
    pub world_surface: i16,
    pub rock_layer: i16,
    pub world_id: i32,
    pub world_name: String,
    pub game_mode: u8,
    pub unique_id: [u8; 16],
    pub world_gen_version: u64,
    pub moon_type: u8,
    /// Background styles, in the order the client applies them: 0, 10, 11, 12, 1..9.
    pub backgrounds: [u8; 13],
    pub ice_back_style: u8,
    pub jungle_back_style: u8,
    pub hell_back_style: u8,
    pub wind_speed_target: f32,
    pub num_clouds: u8,
    pub tree_x: [i32; 3],
    pub tree_style: [u8; 4],
    pub cave_back_x: [i32; 3],
    pub cave_back_style: [u8; 4],
    /// One byte per `TreeTopsInfo.AreaId`, of which there are 13.
    pub tree_tops: [u8; 13],
    pub max_raining: f32,
    pub flags: WorldFlags,
    pub sundial_cooldown: u8,
    pub moondial_cooldown: u8,
    /// copper, iron, silver, gold, cobalt, mythril, adamantite.
    pub ore_tiers: [i16; 7],
    pub invasion_type: i8,
    pub lobby_id: u64,
    pub sandstorm_severity: f32,
    pub extra_spawn_points: Vec<(i16, i16)>,
}

/// Byte count of a `WorldData` payload before the world name and extra spawn points.
pub const WORLD_DATA_FIXED_LEN: usize = 159;

impl WorldData {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::WORLD_DATA);
        self.write_payload(&mut w);
        w.finish()
    }

    fn write_payload(&self, w: &mut Writer) {
        let day_flags = u8::from(self.day_time)
            | (u8::from(self.blood_moon) << 1)
            | (u8::from(self.eclipse) << 2);

        w.i32(self.time)
            .u8(day_flags)
            .u8(self.moon_phase)
            .i16(self.max_tiles_x)
            .i16(self.max_tiles_y)
            .i16(self.spawn_tile_x)
            .i16(self.spawn_tile_y)
            .i16(self.world_surface)
            .i16(self.rock_layer)
            .i32(self.world_id)
            .string(&self.world_name)
            .u8(self.game_mode)
            .bytes(&self.unique_id)
            .u64(self.world_gen_version)
            .u8(self.moon_type)
            .bytes(&self.backgrounds)
            .u8(self.ice_back_style)
            .u8(self.jungle_back_style)
            .u8(self.hell_back_style)
            .f32(self.wind_speed_target)
            .u8(self.num_clouds);

        for x in self.tree_x {
            w.i32(x);
        }
        w.bytes(&self.tree_style);
        for x in self.cave_back_x {
            w.i32(x);
        }
        w.bytes(&self.cave_back_style)
            .bytes(&self.tree_tops)
            .f32(self.max_raining)
            .bytes(&self.flags.0)
            .u8(self.sundial_cooldown)
            .u8(self.moondial_cooldown);

        for tier in self.ore_tiers {
            w.i16(tier);
        }
        w.i8(self.invasion_type)
            .u64(self.lobby_id)
            .f32(self.sandstorm_severity)
            .u8(self.extra_spawn_points.len() as u8);
        for (x, y) in &self.extra_spawn_points {
            w.i16(*x).i16(*y);
        }
    }
}

impl Default for WorldData {
    /// A plain, freshly generated small world at dawn.
    fn default() -> Self {
        Self {
            time: 13500,
            day_time: true,
            blood_moon: false,
            eclipse: false,
            moon_phase: 0,
            max_tiles_x: 4200,
            max_tiles_y: 1200,
            spawn_tile_x: 2100,
            spawn_tile_y: 300,
            world_surface: 350,
            rock_layer: 500,
            world_id: 1,
            world_name: "Terrustia".into(),
            game_mode: 0,
            unique_id: [0; 16],
            world_gen_version: 0,
            moon_type: 0,
            backgrounds: [0; 13],
            ice_back_style: 0,
            jungle_back_style: 0,
            hell_back_style: 0,
            wind_speed_target: 0.0,
            num_clouds: 0,
            tree_x: [4200, 4200, 4200],
            tree_style: [0; 4],
            cave_back_x: [4200, 4200, 4200],
            cave_back_style: [0; 4],
            tree_tops: [0; 13],
            max_raining: 0.0,
            flags: WorldFlags::default(),
            sundial_cooldown: 0,
            moondial_cooldown: 0,
            ore_tiers: [7, 6, 9, 8, 108, 111, 112],
            invasion_type: 0,
            lobby_id: 0,
            sandstorm_severity: 0.0,
            extra_spawn_points: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(frame: &[u8]) -> &[u8] {
        &frame[3..]
    }

    #[test]
    fn hello_recognises_only_this_release() {
        let mut w = Writer::new();
        w.string("Terraria325");
        let hello = Hello::decode(w.as_slice()).unwrap();
        assert!(hello.is_supported());

        let mut w = Writer::new();
        w.string("Terraria279"); // 1.4.4.9
        assert!(!Hello::decode(w.as_slice()).unwrap().is_supported());
    }

    #[test]
    fn player_info_carries_the_slot_and_the_new_flag() {
        // 1.4.5 added the trailing bool; a 1.4.4-shaped 4-byte frame would desync the client.
        let frame = player_info(3, false).unwrap();
        assert_eq!(frame, vec![5, 0, id::PLAYER_INFO, 3, 0]);
        assert_eq!(payload(&frame).len(), 2);
    }

    #[test]
    fn empty_signals_are_bare_headers() {
        for msg in [id::INITIAL_SPAWN, id::FINISHED_CONNECTING_TO_SERVER] {
            let frame = empty(msg).unwrap();
            assert_eq!(frame, vec![3, 0, msg]);
        }
    }

    #[test]
    fn spawn_tile_data_decodes_position_and_team() {
        let mut w = Writer::new();
        w.i32(2100).i32(300).u8(2);
        assert_eq!(
            SpawnTileData::decode(w.as_slice()).unwrap(),
            SpawnTileData {
                x: 2100,
                y: 300,
                team: 2
            }
        );
    }

    #[test]
    fn spawn_tile_data_rejects_a_1_4_4_shaped_payload() {
        // Without the team byte the packet is one byte short; better to error than to read past.
        let mut w = Writer::new();
        w.i32(2100).i32(300);
        assert!(SpawnTileData::decode(w.as_slice()).is_err());
    }

    #[test]
    fn player_spawn_round_trips() {
        let spawn = PlayerSpawn {
            player: 2,
            spawn_x: 100,
            spawn_y: 200,
            respawn_timer: 0,
            deaths_pve: 1,
            deaths_pvp: 2,
            team: 0,
            context: PlayerSpawn::CONTEXT_SPAWNING_INTO_WORLD,
        };
        let frame = spawn.encode().unwrap();
        assert_eq!(PlayerSpawn::decode(payload(&frame)).unwrap(), spawn);
        // 1 + 2 + 2 + 4 + 2 + 2 + 1 + 1
        assert_eq!(payload(&frame).len(), 15);
    }

    #[test]
    fn health_and_mana_round_trip() {
        let health = PlayerHealth {
            player: 1,
            life: 340,
            life_max: 400,
        };
        assert_eq!(
            PlayerHealth::decode(payload(&health.encode().unwrap())).unwrap(),
            health
        );

        let mana = PlayerMana {
            player: 1,
            mana: 20,
            mana_max: 40,
        };
        assert_eq!(
            PlayerMana::decode(payload(&mana.encode().unwrap())).unwrap(),
            mana
        );
    }

    #[test]
    fn time_set_is_nine_bytes() {
        let frame = TimeSet {
            day_time: true,
            time: 13500,
            sun_mod_y: 0,
            moon_mod_y: 0,
        }
        .encode()
        .unwrap();
        // 1 + 4 + 2 + 2
        assert_eq!(payload(&frame).len(), 9);
        assert_eq!(frame[3], 1);
    }

    #[test]
    fn player_controls_omits_velocity_when_the_flag_is_clear() {
        let mut w = Writer::new();
        w.u8(0).u8(0x40).u8(0).u8(0).u8(0).u8(0).vec2(100.0, 200.0);
        let controls = PlayerControls::decode(w.as_slice()).unwrap();
        assert_eq!(controls.position, (100.0, 200.0));
        assert_eq!(controls.velocity, None);
        assert!(controls.facing_right());
    }

    #[test]
    fn player_controls_reads_velocity_when_the_flag_is_set() {
        let mut w = Writer::new();
        w.u8(0)
            .u8(0)
            .u8(0x04)
            .u8(0)
            .u8(0)
            .u8(0)
            .vec2(100.0, 200.0)
            .vec2(-1.5, 3.0);
        let controls = PlayerControls::decode(w.as_slice()).unwrap();
        assert_eq!(controls.velocity, Some((-1.5, 3.0)));
        assert!(!controls.facing_right());
    }

    #[test]
    fn relayed_packets_are_attributed_to_the_sender() {
        // The client claims slot 7; the server must stamp its real slot instead.
        let claimed = [7u8, 0xAA, 0xBB];
        let frame = rewrite_owner(id::PLAYER_CONTROLS, &claimed, 2).unwrap();
        assert_eq!(payload(&frame), &[2, 0xAA, 0xBB]);
        assert_eq!(frame[2], id::PLAYER_CONTROLS);
    }

    #[test]
    fn relaying_an_empty_payload_is_an_error() {
        assert!(rewrite_owner(id::PLAYER_CONTROLS, &[], 0).is_err());
    }

    #[test]
    fn world_data_payload_is_exactly_the_documented_size() {
        // This is the packet that silently hangs a client when a field drifts, so its size is
        // pinned against the byte count derived from the client's reader.
        let world = WorldData {
            world_name: String::new(),
            ..Default::default()
        };
        let frame = world.encode().unwrap();
        // An empty name still costs its one-byte length prefix.
        assert_eq!(payload(&frame).len(), WORLD_DATA_FIXED_LEN + 1);

        let named = WorldData::default();
        assert_eq!(
            payload(&named.encode().unwrap()).len(),
            WORLD_DATA_FIXED_LEN + 1 + named.world_name.len()
        );
    }

    #[test]
    fn world_data_leads_with_time_and_day_flags() {
        let world = WorldData {
            blood_moon: true,
            eclipse: true,
            ..Default::default()
        };
        let frame = world.encode().unwrap();
        let p = payload(&frame);

        assert_eq!(i32::from_le_bytes([p[0], p[1], p[2], p[3]]), 13500);
        // bit0 dayTime, bit1 bloodMoon, bit2 eclipse
        assert_eq!(p[4], 0b0000_0111);
        assert_eq!(p[5], 0); // moon phase
        assert_eq!(i16::from_le_bytes([p[6], p[7]]), 4200);
        assert_eq!(i16::from_le_bytes([p[8], p[9]]), 1200);
    }

    #[test]
    fn world_data_ends_with_the_extra_spawn_point_list() {
        // Added in 1.4.5 and easy to forget; without it the client reads past the payload.
        let world = WorldData {
            extra_spawn_points: vec![(10, 20), (30, 40)],
            ..Default::default()
        };
        let frame = world.encode().unwrap();
        let p = payload(&frame);
        let tail = &p[p.len() - 9..];
        assert_eq!(tail[0], 2);
        assert_eq!(i16::from_le_bytes([tail[1], tail[2]]), 10);
        assert_eq!(i16::from_le_bytes([tail[7], tail[8]]), 40);
    }

    #[test]
    fn world_flag_bits_land_where_the_client_reads_them() {
        let mut flags = WorldFlags::default();
        flags.set_crimson(true);
        assert_eq!(flags.0[1], 0b0010_0000);

        flags.set_hardmode(true);
        flags.set_server_side_character(true);
        assert_eq!(flags.0[0], 0b0101_0000);

        flags.set_server_side_character(false);
        assert_eq!(flags.0[0], 0b0001_0000);
    }

    #[test]
    fn kick_carries_a_readable_reason() {
        let frame = kick(&NetworkText::literal("nope")).unwrap();
        assert_eq!(frame[2], id::KICK);
        let mut r = PacketReader::new(payload(&frame));
        assert_eq!(NetworkText::read(&mut r).unwrap().text, "nope");
    }
}

/// Packet `17`: a single-tile edit.
///
/// The third field is overloaded: for the destroy actions it is a "fail" flag where 1 means the
/// tile was only damaged rather than removed, for placements it is the tile or wall type, and for
/// [`TileAction::Slope`] it is the slope index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileManipulation {
    pub action: u8,
    pub x: i16,
    pub y: i16,
    pub arg: i16,
    pub style: u8,
}

/// The tile actions this server understands, from `MessageBuffer`'s case 17 dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileAction {
    KillTile,
    PlaceTile,
    KillWall,
    PlaceWall,
    /// Destroy without dropping an item.
    KillTileNoItem,
    PlaceWire,
    KillWire,
    /// Hammer a block into a half brick.
    PoundTile,
    PlaceActuator,
    KillActuator,
    PlaceWire2,
    KillWire2,
    PlaceWire3,
    KillWire3,
    SlopeTile,
    PlaceWire4,
    KillWire4,
    /// Anything the vertical slice does not model.
    Other(u8),
}

impl TileManipulation {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        Ok(Self {
            action: r.u8()?,
            x: r.i16()?,
            y: r.i16()?,
            arg: r.i16()?,
            style: r.u8()?,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::TILE_MANIPULATION);
        w.u8(self.action)
            .i16(self.x)
            .i16(self.y)
            .i16(self.arg)
            .u8(self.style);
        w.finish()
    }

    pub fn kind(&self) -> TileAction {
        match self.action {
            0 => TileAction::KillTile,
            1 => TileAction::PlaceTile,
            2 => TileAction::KillWall,
            3 => TileAction::PlaceWall,
            4 => TileAction::KillTileNoItem,
            5 => TileAction::PlaceWire,
            6 => TileAction::KillWire,
            7 => TileAction::PoundTile,
            8 => TileAction::PlaceActuator,
            9 => TileAction::KillActuator,
            10 => TileAction::PlaceWire2,
            11 => TileAction::KillWire2,
            12 => TileAction::PlaceWire3,
            13 => TileAction::KillWire3,
            14 => TileAction::SlopeTile,
            16 => TileAction::PlaceWire4,
            17 => TileAction::KillWire4,
            other => TileAction::Other(other),
        }
    }

    /// For the destroy actions, whether the tile actually went away.
    ///
    /// A pickaxe swing that only damages a block sends `arg == 1`; treating that as a break would
    /// delete blocks on the first hit.
    pub fn destroyed(&self) -> bool {
        self.arg != 1
    }
}

#[cfg(test)]
mod tile_manipulation_tests {
    use super::*;

    #[test]
    fn round_trips_and_is_eight_bytes() {
        let edit = TileManipulation {
            action: 1,
            x: 2100,
            y: 300,
            arg: 3,
            style: 0,
        };
        let frame = edit.encode().unwrap();
        // 1 + 2 + 2 + 2 + 1
        assert_eq!(frame.len() - 3, 8);
        assert_eq!(TileManipulation::decode(&frame[3..]).unwrap(), edit);
    }

    #[test]
    fn actions_map_to_their_meanings() {
        let at = |action| {
            TileManipulation {
                action,
                x: 0,
                y: 0,
                arg: 0,
                style: 0,
            }
            .kind()
        };
        assert_eq!(at(0), TileAction::KillTile);
        assert_eq!(at(1), TileAction::PlaceTile);
        assert_eq!(at(2), TileAction::KillWall);
        assert_eq!(at(3), TileAction::PlaceWall);
        assert_eq!(at(4), TileAction::KillTileNoItem);
        assert_eq!(at(7), TileAction::PoundTile);
        assert_eq!(at(14), TileAction::SlopeTile);
        assert_eq!(at(15), TileAction::Other(15));
        assert_eq!(at(200), TileAction::Other(200));
    }

    #[test]
    fn a_damaged_tile_is_not_a_destroyed_one() {
        let mut edit = TileManipulation {
            action: 0,
            x: 1,
            y: 1,
            arg: 1,
            style: 0,
        };
        assert!(
            !edit.destroyed(),
            "arg == 1 means the block survived the hit"
        );
        edit.arg = 0;
        assert!(edit.destroyed());
    }

    #[test]
    fn negative_coordinates_survive_the_round_trip() {
        // The field is signed; a client near the edge can send an out-of-range negative.
        let edit = TileManipulation {
            action: 0,
            x: -5,
            y: -1,
            arg: 0,
            style: 0,
        };
        assert_eq!(
            TileManipulation::decode(&edit.encode().unwrap()[3..]).unwrap(),
            edit
        );
    }
}
