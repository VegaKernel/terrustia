//! Packet `82` carries a "net module" identified by a leading `u16`.
//!
//! Module ids come from the registration order in `Terraria.Initializers.NetworkInitializer`, so
//! they shift whenever a module is inserted. Verified against the 1.4.5.7 build.

use crate::{
    error::{ProtoError, Result},
    id,
    net_text::NetworkText,
    reader::PacketReader,
    writer::PacketWriter,
};

pub const MODULE_LIQUID: u16 = 0;
pub const MODULE_TEXT: u16 = 1;
pub const MODULE_PING: u16 = 2;
/// `BannerSystem.NetBannersModule`, twelfth in the registration order.
pub const MODULE_BANNERS: u16 = 11;

/// How many banner slots the game's `killCount` array has.
///
/// Confirmed on the wire rather than counted from a table: a real server's full-state module opens
/// with `0x0125`, which is 293.
pub const BANNER_SLOTS: usize = 293;

/// One tile whose liquid has changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiquidChange {
    pub x: i32,
    pub y: i32,
    /// How full the tile is, 0 to 255.
    pub amount: u8,
    /// 0 water, 1 lava, 2 honey, 3 shimmer.
    pub kind: u8,
}

/// The most liquid changes that fit in one module-0 frame.
///
/// The count is a `u16`, and each entry is six bytes, so the ceiling is really the frame limit
/// rather than the counter. Kept well under it so a settling ocean splits across frames instead of
/// producing one the writer refuses.
pub const MAX_LIQUID_CHANGES: usize = 1000;

/// Module 0: liquid levels, as the game sends them.
///
/// This is the message the client expects for water moving, and it is a sixth the size of the tile
/// squares that would otherwise carry the same news — a settling pool dirties a stripe of tiles
/// every tick, so the difference is the difference between a trickle of traffic and a flood.
///
/// The coordinate is packed into one `i32` as `(x << 16) | y`, which is why a world wider than
/// 65535 tiles could not be described by it. Terraria's largest is 8400.
pub fn liquid_changes(changes: &[LiquidChange]) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_LIQUID).u16(changes.len() as u16);
    for change in changes {
        w.i32(((change.x & 0xFFFF) << 16) | (change.y & 0xFFFF))
            .u8(change.amount)
            .u8(change.kind);
    }
    w.finish()
}

/// `NetTeleportPylonModule`, ninth in the registration order.
pub const MODULE_PYLON: u16 = 8;

/// What a module-8 frame is saying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PylonMessage {
    /// Server to client: this pylon exists, put it on the map.
    Added = 0,
    /// Server to client: it does not any more.
    Removed = 1,
    /// Client to server: take me to it.
    RequestTeleport = 2,
}

/// One pylon, as module 8 describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pylon {
    pub x: i16,
    pub y: i16,
    /// Which biome's pylon: 0 surface purity, 1 jungle, 2 hallow, 3 underground, 4 beach,
    /// 5 desert, 6 snow, 7 glowing mushroom, 8 victory.
    pub kind: u8,
}

impl Pylon {
    /// The Victory pylon, the one kind that needs no townsfolk around it.
    pub const VICTORY: u8 = 8;
}

/// Module 8: a pylon appeared or vanished.
///
/// The client keeps its own list and draws the travel map from it. A pylon it was never told about
/// is scenery: standing next to it opens a map with nowhere to go.
pub fn pylon_message(message: PylonMessage, pylon: Pylon) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_PYLON)
        .u8(message as u8)
        .i16(pylon.x)
        .i16(pylon.y)
        .u8(pylon.kind);
    w.finish()
}

/// Read a module-8 frame, returning `None` for any other module.
pub fn decode_pylon_message(payload: &[u8]) -> Result<Option<(PylonMessage, Pylon)>> {
    let mut r = PacketReader::new(payload);
    if r.u16()? != MODULE_PYLON {
        return Ok(None);
    }
    let message = match r.u8()? {
        0 => PylonMessage::Added,
        1 => PylonMessage::Removed,
        2 => PylonMessage::RequestTeleport,
        other => {
            return Err(ProtoError::OutOfRange {
                field: "pylon message type",
                value: i64::from(other),
            });
        }
    };
    Ok(Some((
        message,
        Pylon {
            x: r.i16()?,
            y: r.i16()?,
            kind: r.u8()?,
        },
    )))
}

/// Read a module-0 payload back into the changes it describes.
///
/// Returns `None` for any other module, so a caller can hand it every packet 82 it sees.
pub fn decode_liquid_changes(payload: &[u8]) -> Result<Option<Vec<LiquidChange>>> {
    let mut r = PacketReader::new(payload);
    if r.u16()? != MODULE_LIQUID {
        return Ok(None);
    }
    let count = usize::from(r.u16()?);
    let mut changes = Vec::with_capacity(count.min(MAX_LIQUID_CHANGES));
    for _ in 0..count {
        let packed = r.i32()?;
        changes.push(LiquidChange {
            x: (packed >> 16) & 0xFFFF,
            y: packed & 0xFFFF,
            amount: r.u8()?,
            kind: r.u8()?,
        });
    }
    Ok(Some(changes))
}

/// Module 11, message 0: every banner's kill count and claim count at once.
///
/// Sent as a player joins. Without it the client's bestiary shows nought kills for everything,
/// however many the world has recorded — the counts live only on the server, and there is no other
/// message that carries them.
///
/// `claimable` is what the game hands out when a threshold is crossed and the player has not
/// collected the banner yet. This server drops the banner as an item on the spot instead, so it
/// has nothing to claim later and sends zeroes; the counts, which are what the bestiary actually
/// displays, are real.
pub fn banners_full_state(
    kills: &[u32; BANNER_SLOTS],
    claimable: &[u16; BANNER_SLOTS],
) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_BANNERS).u8(0).i16(BANNER_SLOTS as i16);
    for count in kills {
        // The game stores these as signed ints, and a count large enough to wrap is one no world
        // will ever reach; saturating keeps the wire value sane if one somehow does.
        w.i32((*count).min(i32::MAX as u32) as i32);
    }
    w.i16(BANNER_SLOTS as i16);
    for count in claimable {
        w.u16(*count);
    }
    w.finish()
}

/// Module 11, message 1: one banner's kill count has changed.
///
/// Sent on every kill that counts towards a banner, so the bestiary's counter ticks up while the
/// player watches rather than only on their next join.
pub fn banner_kill_count(banner: u16, kills: u32) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_BANNERS)
        .u8(1)
        .i16(banner as i16)
        .i32(kills.min(i32::MAX as u32) as i32);
    w.finish()
}

/// A chat message as the client sends it: a command name, then the text.
///
/// The command is usually `Say`; `Emote` and the party/whisper commands use the same shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingChat {
    pub command: String,
    pub text: String,
}

impl IncomingChat {
    /// Parse a packet `82` payload, returning `None` when it is some other module.
    pub fn decode(payload: &[u8]) -> Result<Option<Self>> {
        let mut r = PacketReader::new(payload);
        if r.u16()? != MODULE_TEXT {
            return Ok(None);
        }
        Ok(Some(Self {
            command: r.string()?,
            text: r.string()?,
        }))
    }

    /// Whether this is ordinary chat rather than an emote or a party command.
    pub fn is_say(&self) -> bool {
        self.command.eq_ignore_ascii_case("Say")
    }
}

/// Build the server-to-client form of a chat line.
pub fn chat_broadcast(author: u8, text: &NetworkText, color: [u8; 3]) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_TEXT).u8(author);
    text.write(&mut w);
    w.rgb(color);
    w.finish()
}

/// The slot value the client renders as "from the server" rather than from a player.
pub const SERVER_AUTHOR: u8 = 255;

/// Reject a chat line that is empty or absurdly long before it reaches other players.
pub fn validate_chat(text: &str, max_len: usize) -> Result<()> {
    if text.is_empty() || text.len() > max_len {
        return Err(ProtoError::OutOfRange {
            field: "chat length",
            value: text.len() as i64,
        });
    }
    Ok(())
}

/// Module 6: Journey (creative) mode powers.
/// `Terraria.GameContent.NetModules.NetCreativePowersModule`, seventh in the registration order
/// (`NetworkInitializer.RegisterAll`: 0 Liquid, 1 Text, 2 Ping, 3 Ambience, 4 Bestiary,
/// 5 CreativeUnlocks, 6 CreativePowers — the id is the 0-based registration counter). It was
/// previously `4`, which is `NetBestiaryModule`: with that value a real client's creative-power
/// requests were dropped and its bestiary packets misrouted here (and vice-versa outbound).
pub const MODULE_CREATIVE_POWERS: u16 = 6;

/// Power ids, in `CreativePowerManager`'s own registration order
/// (`CreativePowerManager.cs:90-104`) — the order *is* the wire format, a power's id is its
/// registration index, not a label chosen for readability.
///
/// All fifteen are decoded by [`decode_creative_power`]: `FREEZE_TIME`, the four `START_*`
/// buttons, `FREEZE_RAIN`, `FREEZE_WIND`, `STOP_BIOME_SPREAD`, `MODIFY_WIND`/`MODIFY_RAIN`/
/// `MODIFY_TIME_RATE`/`DIFFICULTY`, and `GODMODE`/`FAR_PLACEMENT_RANGE`/`SPAWN_RATE`. `DIFFICULTY`
/// is the same `ASharedSliderPower` wire shape as `MODIFY_TIME_RATE` — real vanilla's own
/// `DifficultySliderPower : ASharedSliderPower` — its interesting part is entirely on the
/// gameplay side (`game/journey.rs`'s `difficulty_multiplier`, and `server.rs`'s
/// `effective_difficulty()`), not the wire.
pub mod power {
    pub const FREEZE_TIME: u16 = 0;
    pub const START_DAY: u16 = 1;
    pub const START_NOON: u16 = 2;
    pub const START_NIGHT: u16 = 3;
    pub const START_MIDNIGHT: u16 = 4;
    pub const GODMODE: u16 = 5;
    pub const MODIFY_WIND: u16 = 6;
    pub const MODIFY_RAIN: u16 = 7;
    pub const MODIFY_TIME_RATE: u16 = 8;
    pub const FREEZE_RAIN: u16 = 9;
    pub const FREEZE_WIND: u16 = 10;
    pub const FAR_PLACEMENT_RANGE: u16 = 11;
    pub const DIFFICULTY: u16 = 12;
    pub const STOP_BIOME_SPREAD: u16 = 13;
    pub const SPAWN_RATE: u16 = 14;
}

/// The four `ASharedButtonPower`s, in registration order — used to recognise a button id without
/// repeating the list at every call site.
const BUTTON_POWERS: [u16; 4] = [
    power::START_DAY,
    power::START_NOON,
    power::START_NIGHT,
    power::START_MIDNIGHT,
];

/// The four `ASharedTogglePower`s this server models the effect of. `GODMODE`/
/// `FAR_PLACEMENT_RANGE` are also toggles on the wire, but per-player (a 255-entry bit-packed
/// array, not this single-bool shape) — see [`power`]'s own doc for why they are excluded here.
const TOGGLE_POWERS: [u16; 4] = [
    power::FREEZE_TIME,
    power::FREEZE_RAIN,
    power::FREEZE_WIND,
    power::STOP_BIOME_SPREAD,
];

/// The four `ASharedSliderPower`s this server models the effect of. `SPAWN_RATE` is also a
/// slider on the wire, but per-player — see [`PER_PLAYER_SLIDER_POWERS`].
const SLIDER_POWERS: [u16; 4] = [
    power::MODIFY_WIND,
    power::MODIFY_RAIN,
    power::MODIFY_TIME_RATE,
    power::DIFFICULTY,
];

/// The two `APerPlayerTogglePower`s.
const PER_PLAYER_TOGGLE_POWERS: [u16; 2] = [power::GODMODE, power::FAR_PLACEMENT_RANGE];

/// The one `APerPlayerSliderPower`.
const PER_PLAYER_SLIDER_POWERS: [u16; 1] = [power::SPAWN_RATE];

/// `APerPlayerTogglePower`'s own `SubMessageType` (`CreativePowers.cs`, nested in that class) —
/// `SyncOnePlayer` is the only one this server ever needs to *decode*: `SyncEveryone` is what
/// `OnPlayerJoining` sends server→client, never something a real client sends inbound.
const SYNC_ONE_PLAYER: u8 = 1;

/// A decoded module-4 packet, as far as this server understands it today.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CreativePowerMessage {
    /// One of the four day/noon/night/midnight buttons (`ASharedButtonPower`). No payload beyond
    /// the power id itself — `DeserializeNetMessage` triggers `UsePower()` on receipt, nothing
    /// else to read.
    Button(u16),
    /// One of the four shared on/off powers (`ASharedTogglePower`). Carries the requested state.
    Toggle(u16, bool),
    /// One of the four shared sliders (`ASharedSliderPower`). Carries the raw 0.0–1.0 slider
    /// position — each power's own `UpdateInfoFromSliderValueCache` remaps that into its actual
    /// effect (`ModifyTimeRate`'s 1×–24× rate, `ModifyWindDirectionAndStrength`'s -0.8..0.8 lerp,
    /// `ModifyRainPower`'s rain strength read as-is, `DifficultySliderPower`'s 0.5×–3× strength
    /// multiplier), which is deliberately kept out of the proto crate — that remapping is
    /// gameplay, not wire format.
    Slider(u16, f32),
    /// A per-player toggle request (`Godmode`/`FarPlacementRange`) — the `SyncOnePlayer`
    /// sub-message, the only shape a client ever sends (see [`SYNC_ONE_PLAYER`]'s own doc). The
    /// player index the client sent is **not** carried here: `DeserializeNetMessage`'s own
    /// dedicated-server branch always substitutes the real sender's slot instead (`Main.netMode ==
    /// 2` — a client cannot toggle Godmode for somebody else), so the caller supplies that slot
    /// itself from the connection the packet arrived on, never trusts the wire for it.
    PerPlayerToggle(u16, bool),
    /// A per-player slider request (`SpawnRate`) — no sub-message type byte at all in this shape,
    /// just a player index (also not carried here, same reason as `PerPlayerToggle`) then the raw
    /// value.
    PerPlayerSlider(u16, f32),
}

/// Read a module-4 frame. Returns `None` for any other module, and also for a power id this
/// server does not model the wire shape of yet (real ids, just not decoded — not out-of-range).
pub fn decode_creative_power(payload: &[u8]) -> Result<Option<CreativePowerMessage>> {
    let mut r = PacketReader::new(payload);
    if r.u16()? != MODULE_CREATIVE_POWERS {
        return Ok(None);
    }
    let power_id = r.u16()?;
    if BUTTON_POWERS.contains(&power_id) {
        return Ok(Some(CreativePowerMessage::Button(power_id)));
    }
    if TOGGLE_POWERS.contains(&power_id) {
        return Ok(Some(CreativePowerMessage::Toggle(power_id, r.bool()?)));
    }
    if SLIDER_POWERS.contains(&power_id) {
        return Ok(Some(CreativePowerMessage::Slider(power_id, r.f32()?)));
    }
    if PER_PLAYER_TOGGLE_POWERS.contains(&power_id) {
        if r.u8()? != SYNC_ONE_PLAYER {
            return Ok(None); // a SyncEveryone (or anything else) inbound is not a real request
        }
        let _player_index = r.u8()?; // discarded — see PerPlayerToggle's own doc
        return Ok(Some(CreativePowerMessage::PerPlayerToggle(
            power_id,
            r.bool()?,
        )));
    }
    if PER_PLAYER_SLIDER_POWERS.contains(&power_id) {
        let _player_index = r.u8()?; // discarded — see PerPlayerSlider's own doc
        return Ok(Some(CreativePowerMessage::PerPlayerSlider(
            power_id,
            r.f32()?,
        )));
    }
    Ok(None)
}

/// Encode a shared toggle power's state — the same shape `ASharedTogglePower` uses both for
/// `OnPlayerJoining` (telling a newly connected client where things stand) and for the dedicated
/// server's own re-broadcast of an accepted toggle to everyone else.
pub fn creative_power_toggle(power_id: u16, enabled: bool) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_CREATIVE_POWERS).u16(power_id).bool(enabled);
    w.finish()
}

/// Encode a shared slider power's raw value — the same shape `ASharedSliderPower` uses for both
/// `OnPlayerJoining` and the dedicated server's own re-broadcast of an accepted change.
pub fn creative_power_slider(power_id: u16, value: f32) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_CREATIVE_POWERS).u16(power_id).f32(value);
    w.finish()
}

/// Encode one player's confirmed per-player toggle state — the `SyncOnePlayer` shape
/// `SetEnabledState` broadcasts to every connected client (the toggling player included) once a
/// request is accepted.
pub fn creative_power_toggle_for_player(
    power_id: u16,
    player_index: u8,
    enabled: bool,
) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_CREATIVE_POWERS)
        .u16(power_id)
        .u8(SYNC_ONE_PLAYER)
        .u8(player_index)
        .bool(enabled);
    w.finish()
}

/// Encode the full per-player toggle state (`SyncEveryone`, bit-packed) — `OnPlayerJoining`'s own
/// shape, sent once to a newly connected client so it learns where every already-connected
/// player's toggle stands. `states` is indexed by player slot; a slot nobody occupies reads
/// `false`, the same as `_perPlayerIsEnabled`'s own C# default.
pub fn creative_power_toggle_full_state(power_id: u16, states: &[bool; 255]) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_CREATIVE_POWERS).u16(power_id).u8(0);
    for chunk in states.chunks(8) {
        let mut byte = 0u8;
        for (i, &enabled) in chunk.iter().enumerate() {
            if enabled {
                byte |= 1 << i;
            }
        }
        w.u8(byte);
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::Writer;

    /// Module 0 exactly as a real 1.4.5.8 server sent it.
    ///
    /// Two tiles of water settling near the surface of `ProbeTiny`, captured from the game's own
    /// dedicated server. The packing is the part worth pinning: the coordinate is one `i32` with
    /// **x in the high half**, which reads as a plausible position either way round on a square
    /// world and puts every splash in the wrong place on a real one.
    #[test]
    fn the_liquid_module_is_packed_the_way_a_real_server_packs_it() {
        const REAL: &[u8] = &[
            0x00, 0x00, 0x02, 0x00, 0x58, 0x01, 0xed, 0x09, 0xff, 0x00, 0x58, 0x01, 0xec, 0x09,
            0xff, 0x00,
        ];
        let frame = liquid_changes(&[
            LiquidChange {
                x: 2541,
                y: 344,
                amount: 255,
                kind: 0,
            },
            LiquidChange {
                x: 2540,
                y: 344,
                amount: 255,
                kind: 0,
            },
        ])
        .unwrap();
        assert_eq!(&frame[3..], REAL);
    }

    /// The banner module's full state, shaped exactly as a real 1.4.5.8 server sends it.
    ///
    /// Its frame was captured from the game's own dedicated server serving a fresh world, and came
    /// to 1765 bytes: the module id, a message byte, then two counted arrays of 293 entries — ints
    /// for kills and shorts for claims. Nothing about that is guessable from the field names, and
    /// a length read at the wrong width here desynchronises the client for the rest of the session.
    #[test]
    fn the_banner_full_state_is_the_shape_a_real_server_sends() {
        let kills = [0u32; BANNER_SLOTS];
        let claimable = [0u16; BANNER_SLOTS];
        let frame = banners_full_state(&kills, &claimable).unwrap();
        let payload = &frame[3..];

        assert_eq!(payload.len(), 1765, "a real server's frame was 1765 bytes");
        assert_eq!(u16::from_le_bytes([payload[0], payload[1]]), MODULE_BANNERS);
        assert_eq!(payload[2], 0, "message type 0 is the full state");
        assert_eq!(i16::from_le_bytes([payload[3], payload[4]]), 293);
        // The second length sits immediately after 293 four-byte kill counts.
        let at = 5 + BANNER_SLOTS * 4;
        assert_eq!(i16::from_le_bytes([payload[at], payload[at + 1]]), 293);
        assert_eq!(at + 2 + BANNER_SLOTS * 2, payload.len());
    }

    #[test]
    fn a_banner_kill_count_update_carries_the_banner_and_its_total() {
        let frame = banner_kill_count(7, 123).unwrap();
        let payload = &frame[3..];
        assert_eq!(u16::from_le_bytes([payload[0], payload[1]]), MODULE_BANNERS);
        assert_eq!(payload[2], 1, "message type 1 is a kill-count update");
        assert_eq!(i16::from_le_bytes([payload[3], payload[4]]), 7);
        assert_eq!(
            i32::from_le_bytes([payload[5], payload[6], payload[7], payload[8]]),
            123
        );
        assert_eq!(payload.len(), 9);
    }

    #[test]
    fn decodes_a_say_message() {
        let mut w = Writer::new();
        w.u16(MODULE_TEXT).string("Say").string("hello world");
        let chat = IncomingChat::decode(w.as_slice()).unwrap().unwrap();
        assert!(chat.is_say());
        assert_eq!(chat.text, "hello world");
    }

    #[test]
    fn ignores_other_modules() {
        let mut w = Writer::new();
        w.u16(MODULE_LIQUID).bytes(&[1, 2, 3]);
        assert_eq!(IncomingChat::decode(w.as_slice()).unwrap(), None);
    }

    #[test]
    fn broadcast_has_module_author_text_and_colour() {
        let frame = chat_broadcast(2, &NetworkText::literal("hi"), [255, 128, 0]).unwrap();
        assert_eq!(frame[2], id::NET_MODULES);
        let mut r = PacketReader::new(&frame[3..]);
        assert_eq!(r.u16().unwrap(), MODULE_TEXT);
        assert_eq!(r.u8().unwrap(), 2);
        assert_eq!(NetworkText::read(&mut r).unwrap().text, "hi");
        assert_eq!(r.rgb().unwrap(), [255, 128, 0]);
        assert!(r.is_empty());
    }

    #[test]
    fn empty_and_oversized_chat_is_refused() {
        assert!(validate_chat("", 500).is_err());
        assert!(validate_chat(&"x".repeat(501), 500).is_err());
        assert!(validate_chat("ok", 500).is_ok());
    }

    #[test]
    fn a_truncated_module_payload_is_an_error() {
        assert!(IncomingChat::decode(&[1]).is_err());
        let mut w = Writer::new();
        w.u16(MODULE_TEXT).string("Say"); // missing the text
        assert!(IncomingChat::decode(w.as_slice()).is_err());
    }

    #[test]
    fn decodes_each_of_the_four_time_skip_buttons() {
        for id in [
            power::START_DAY,
            power::START_NOON,
            power::START_NIGHT,
            power::START_MIDNIGHT,
        ] {
            let mut w = Writer::new();
            w.u16(MODULE_CREATIVE_POWERS).u16(id);
            assert_eq!(
                decode_creative_power(w.as_slice()).unwrap(),
                Some(CreativePowerMessage::Button(id)),
                "power id {id}"
            );
        }
    }

    /// Pins the module id to vanilla's registration index, NOT to `MODULE_CREATIVE_POWERS` — every
    /// other creative-power test builds its packet with that constant, so all of them pass whatever
    /// value it holds (a closed loop). `NetworkInitializer.RegisterAll` registers modules in order
    /// and their id is the 0-based counter: 0 Liquid, 1 Text, 2 Ping, 3 Ambience, 4 Bestiary,
    /// 5 CreativeUnlocks, **6 CreativePowers**. A real 1.4.5.8 client sends creative-power requests
    /// as module 6 and its Bestiary as module 4; getting this wrong routes powers to the bestiary
    /// deserializer (and vice-versa). This is the test that catches the constant drifting.
    #[test]
    fn the_creative_powers_module_id_is_the_vanilla_registration_index() {
        assert_eq!(
            MODULE_CREATIVE_POWERS, 6,
            "creative powers is the 7th-registered module (index 6); 4 is Bestiary"
        );
        // A frame carrying the real wire id 6 must decode as a creative-power request...
        let mut w = Writer::new();
        w.u16(6).u16(power::START_DAY);
        assert_eq!(
            decode_creative_power(w.as_slice()).unwrap(),
            Some(CreativePowerMessage::Button(power::START_DAY)),
            "a real client's module-6 creative-power frame must decode"
        );
        // ...and a frame carrying id 4 (Bestiary) must NOT be mistaken for one.
        let mut bestiary = Writer::new();
        bestiary.u16(4).u16(power::START_DAY);
        assert_eq!(
            decode_creative_power(bestiary.as_slice()).unwrap(),
            None,
            "module 4 is Bestiary, not creative powers"
        );
    }

    #[test]
    fn decodes_each_of_the_four_shared_toggles_with_their_state() {
        for id in [
            power::FREEZE_TIME,
            power::FREEZE_RAIN,
            power::FREEZE_WIND,
            power::STOP_BIOME_SPREAD,
        ] {
            for state in [true, false] {
                let mut w = Writer::new();
                w.u16(MODULE_CREATIVE_POWERS).u16(id).bool(state);
                assert_eq!(
                    decode_creative_power(w.as_slice()).unwrap(),
                    Some(CreativePowerMessage::Toggle(id, state)),
                    "power id {id}, state {state}"
                );
            }
        }
    }

    /// All fifteen real power ids are modelled now, so this uses an id past the real range (real
    /// vanilla only ever registers 0-14, `CreativePowerManager.cs:90-104`) — a client sending an
    /// id this server doesn't recognise should not desync (an `Err`), just be ignored.
    #[test]
    fn an_unrecognised_power_id_decodes_to_nothing_rather_than_an_error() {
        let mut w = Writer::new();
        w.u16(MODULE_CREATIVE_POWERS).u16(999).f32(0.5);
        assert_eq!(decode_creative_power(w.as_slice()).unwrap(), None);
    }

    #[test]
    fn ignores_other_modules_for_creative_powers_too() {
        let mut w = Writer::new();
        w.u16(MODULE_TEXT).u16(power::FREEZE_TIME).bool(true);
        assert_eq!(decode_creative_power(w.as_slice()).unwrap(), None);
    }

    #[test]
    fn the_toggle_encoder_round_trips_through_the_decoder() {
        let frame = creative_power_toggle(power::FREEZE_WIND, true).unwrap();
        assert_eq!(
            decode_creative_power(&frame[3..]).unwrap(),
            Some(CreativePowerMessage::Toggle(power::FREEZE_WIND, true))
        );
    }

    #[test]
    fn decodes_each_of_the_four_shared_sliders_with_their_raw_value() {
        for id in [
            power::MODIFY_WIND,
            power::MODIFY_RAIN,
            power::MODIFY_TIME_RATE,
            power::DIFFICULTY,
        ] {
            let mut w = Writer::new();
            w.u16(MODULE_CREATIVE_POWERS).u16(id).f32(0.75);
            assert_eq!(
                decode_creative_power(w.as_slice()).unwrap(),
                Some(CreativePowerMessage::Slider(id, 0.75)),
                "power id {id}"
            );
        }
    }

    #[test]
    fn the_slider_encoder_round_trips_through_the_decoder() {
        let frame = creative_power_slider(power::MODIFY_TIME_RATE, 0.5).unwrap();
        assert_eq!(
            decode_creative_power(&frame[3..]).unwrap(),
            Some(CreativePowerMessage::Slider(power::MODIFY_TIME_RATE, 0.5))
        );
    }

    #[test]
    fn decodes_a_per_player_toggle_request_ignoring_the_wire_player_index() {
        for id in [power::GODMODE, power::FAR_PLACEMENT_RANGE] {
            let mut w = Writer::new();
            // The player index on the wire (200 here) is exactly what the caller must *not* trust
            // — `PerPlayerToggle` carries only the state, the real slot comes from the connection.
            w.u16(MODULE_CREATIVE_POWERS)
                .u16(id)
                .u8(SYNC_ONE_PLAYER)
                .u8(200)
                .bool(true);
            assert_eq!(
                decode_creative_power(w.as_slice()).unwrap(),
                Some(CreativePowerMessage::PerPlayerToggle(id, true)),
                "power id {id}"
            );
        }
    }

    /// A `SyncEveryone` (sub-message `0`) is what the server itself sends on join — never a real
    /// inbound request. Decoding it as if it carried a state would silently apply the beginning of
    /// a bit-packed array as a bool.
    #[test]
    fn a_sync_everyone_submessage_is_never_decoded_as_a_request() {
        let mut w = Writer::new();
        w.u16(MODULE_CREATIVE_POWERS)
            .u16(power::GODMODE)
            .u8(0)
            .bytes(&[0u8; 32]);
        assert_eq!(decode_creative_power(w.as_slice()).unwrap(), None);
    }

    #[test]
    fn decodes_a_per_player_slider_request_ignoring_the_wire_player_index() {
        let mut w = Writer::new();
        w.u16(MODULE_CREATIVE_POWERS)
            .u16(power::SPAWN_RATE)
            .u8(200)
            .f32(0.75);
        assert_eq!(
            decode_creative_power(w.as_slice()).unwrap(),
            Some(CreativePowerMessage::PerPlayerSlider(
                power::SPAWN_RATE,
                0.75
            ))
        );
    }

    #[test]
    fn the_per_player_toggle_encoder_round_trips_through_the_decoder() {
        let frame = creative_power_toggle_for_player(power::GODMODE, 7, true).unwrap();
        assert_eq!(
            decode_creative_power(&frame[3..]).unwrap(),
            Some(CreativePowerMessage::PerPlayerToggle(power::GODMODE, true))
        );
    }

    /// The bit-packed full-state shape, checked byte by byte against what
    /// `APerPlayerTogglePower::OnPlayerJoining` actually writes: 32 bytes (`ceil(255/8)`), bit `j`
    /// of byte `i` is slot `i*8+j`'s state — including the awkward last byte, only 7 real bits
    /// wide (255 is not a multiple of 8), which a naive `255/8` (not `ceil`) would drop entirely.
    #[test]
    fn the_full_state_encoder_bit_packs_exactly_like_a_real_server() {
        let mut states = [false; 255];
        states[0] = true; // bit 0 of byte 0
        states[9] = true; // bit 1 of byte 1
        states[254] = true; // the last slot, inside the awkward 7-bit-wide final byte
        let frame = creative_power_toggle_full_state(power::FAR_PLACEMENT_RANGE, &states).unwrap();
        let payload = &frame[3..];

        assert_eq!(
            u16::from_le_bytes([payload[0], payload[1]]),
            MODULE_CREATIVE_POWERS
        );
        assert_eq!(
            u16::from_le_bytes([payload[2], payload[3]]),
            power::FAR_PLACEMENT_RANGE
        );
        assert_eq!(payload[4], 0, "sub-message 0 is SyncEveryone");
        let bits = &payload[5..];
        assert_eq!(bits.len(), 32, "ceil(255/8)");
        assert_eq!(bits[0], 0b0000_0001, "slot 0");
        assert_eq!(bits[1], 0b0000_0010, "slot 9 is bit 1 of byte 1");
        assert_eq!(
            bits[31], 0b0100_0000,
            "slot 254 is bit 6 of the 7-wide final byte"
        );
        // Every other byte should be untouched.
        for (i, &b) in bits.iter().enumerate() {
            if i != 0 && i != 1 && i != 31 {
                assert_eq!(b, 0, "byte {i} should be all zero");
            }
        }
    }
}
