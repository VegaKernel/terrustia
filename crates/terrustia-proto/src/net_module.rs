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
/// `NetParticlesModule`, tenth in the registration order (`NetworkInitializer.cs:20`).
pub const MODULE_PARTICLES: u16 = 9;
/// `BannerSystem.NetBannersModule`, twelfth in the registration order.
pub const MODULE_BANNERS: u16 = 11;
/// `CraftingRequests.NetCraftingRequestsModule`, thirteenth in the registration order
/// (`NetworkInitializer.cs:23`).
pub const MODULE_CRAFTING_REQUESTS: u16 = 12;

/// Read a module frame's leading id without decoding anything past it — for modules this server
/// only relays byte for byte rather than has any opinion about the contents of.
pub fn peek_module_id(payload: &[u8]) -> Result<u16> {
    PacketReader::new(payload).u16()
}

/// Wrap an already-decoded module payload (leading module id plus body, exactly as it arrived)
/// back into a full packet `82` frame, unchanged.
///
/// Used for the two modules a dedicated server relays verbatim to every other client rather than
/// acting on: ping (`NetPingModule.cs:16-28`: `Main.dedServ` broadcasts the deserialized position
/// straight back out) and particles (`NetParticlesModule.cs:17-31`: `Main.netMode == 2` broadcasts
/// the deserialized type and settings straight back out). Neither `Deserialize` mutates anything
/// it read before re-serializing, so relaying the original bytes untouched is exactly what
/// deserializing fully and re-serializing the same values would produce — without this crate
/// needing to model `ParticleOrchestraSettings`' own variable shape at all.
pub fn relay_module(payload: &[u8]) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.bytes(payload);
    w.finish()
}

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

/// Module 12: "craft this recipe, taking whatever I am short of from these nearby chests."
///
/// This is not the ordinary crafting path — an ordinary craft is entirely a client-side decision,
/// the server never sees it. This module exists only for the shortfall a player's own inventory
/// (and bank chests, which this server does not model as anything other than an ordinary chest)
/// could not cover: the client pre-consumes what it can reach locally, then asks the server —
/// which alone knows the true, currently-contested contents of a chest nobody has fully opened —
/// to make up the rest from specific nearby chests (`CraftingRequests.cs:142-173`).
///
/// One entry from the client's ingredient list. `itemIdOrRecipeGroup` is either a literal item id,
/// or — when it is `>= RECIPE_GROUP_OFFSET` — a vanilla `RecipeGroup`'s fake id
/// (`Recipe.RequiredItemEntry.IsRecipeGroup`, `Recipe.cs:21-23`): "any wood," "any iron bar," and
/// so on. This server has no `RecipeGroup` table (it is not generated by `terrustia-codegen`, and
/// building one is out of this fix's scope), so [`CraftIngredient::is_recipe_group`] is exposed
/// for a caller to recognise the case it cannot verify, rather than silently mismatching it — see
/// that method's own doc for what a caller must do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CraftIngredient {
    pub item_id_or_group: i32,
    pub stack: i32,
}

impl CraftIngredient {
    /// `RecipeGroup.FakeItemIdOffset` (`RecipeGroup.cs:11`).
    pub const RECIPE_GROUP_OFFSET: i32 = 1_000_000;

    /// Whether this entry names a `RecipeGroup` rather than one literal item — the case this
    /// server cannot verify against real chest contents without a `RecipeGroup` table it does not
    /// have. A caller must treat this as "cannot confirm the shortfall is covered," which for a
    /// request that must be either fully approved or not at all means denying the whole request
    /// rather than guessing: `CraftingRequests.HandleRequest` (`CraftingRequests.cs:308-321`)
    /// only ever approves when *every* entry's `CountMatches` clears its `stack`, so one
    /// unverifiable entry is exactly as fatal to approval as one that is short.
    pub fn is_recipe_group(&self) -> bool {
        self.item_id_or_group >= Self::RECIPE_GROUP_OFFSET
    }
}

/// A real recipe needs no more than this many distinct ingredient entries
/// (`Recipe.maxRequirements`, `Recipe.cs:56`) — used only to size the initial `Vec`, not to
/// reject a larger claimed count; a hostile claim past this still just runs out of payload to
/// read and errors, the same as any other truncated packet.
pub const MAX_CRAFT_REQUEST_ITEMS: usize = 15;
/// Defensive cap on the chest-list `Vec`'s initial capacity. Not a vanilla constant — there is no
/// fixed ceiling on how many chests may sit in craft range — just enough headroom that an honest
/// request never reallocates without honouring an arbitrary claimed count up front.
pub const MAX_CRAFT_REQUEST_CHESTS: usize = 256;

/// A decoded module-12 request:
/// `CraftingRequests.NetCraftingRequestsModule.DeserializeRequest` (`CraftingRequests.cs:51-67`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraftRequest {
    pub items: Vec<CraftIngredient>,
    /// Chest indices the client believes hold the shortfall. `None` is what vanilla's own `(num3
    /// < 0) ? null : Main.chest[num3]` (`CraftingRequests.cs:64`) becomes here — a chest gone
    /// stale client-side, dropped rather than resolved.
    pub chests: Vec<Option<i16>>,
}

/// Read a module-12 request frame. Returns `None` for any other module.
///
/// Only the server-bound shape (`NetCraftingRequestsModule.Deserialize`'s `Main.netMode == 2`
/// branch, `CraftingRequests.cs:74-85`) is decoded — the client-bound response
/// ([`craft_response`]) is this server's own to *write*, never to read back.
pub fn decode_craft_request(payload: &[u8]) -> Result<Option<CraftRequest>> {
    let mut r = PacketReader::new(payload);
    if r.u16()? != MODULE_CRAFTING_REQUESTS {
        return Ok(None);
    }
    let item_count = r.var_u32()? as usize;
    let mut items = Vec::with_capacity(item_count.min(MAX_CRAFT_REQUEST_ITEMS));
    for _ in 0..item_count {
        // `itemIdOrRecipeGroup` is a plain four-byte int (`writer.Write(item.itemIdOrRecipeGroup)`
        // — not 7-bit encoded like everything else in this frame); only the stack that follows it
        // is (`CraftingRequests.cs:33-34`).
        let item_id_or_group = r.i32()?;
        let stack = r.var_u32()? as i32;
        items.push(CraftIngredient {
            item_id_or_group,
            stack,
        });
    }
    let chest_count = r.var_u32()? as usize;
    let mut chests = Vec::with_capacity(chest_count.min(MAX_CRAFT_REQUEST_CHESTS));
    for _ in 0..chest_count {
        // `Read7BitEncodedInt` hands back a signed `int` by reinterpreting the bit pattern, not by
        // truncating a `uint` — `as i32` on a same-width `u32` is exactly that reinterpretation in
        // Rust, so a chest index a client sent as -1 round-trips back to -1, not to some huge
        // positive number.
        let index = r.var_u32()? as i32;
        chests.push(if index < 0 {
            None // `(num3 < 0) ? null : Main.chest[num3]` (`CraftingRequests.cs:64`)
        } else {
            i16::try_from(index).ok()
        });
    }
    Ok(Some(CraftRequest { items, chests }))
}

/// Module 12, server to client: whether the shortfall could be pulled from the chests offered.
/// `NetCraftingRequestsModule.WriteResponse` (`CraftingRequests.cs:44-49`).
pub fn craft_response(approved: bool) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_CRAFTING_REQUESTS).bool(approved);
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

    /// Pins both new module ids to `NetworkInitializer.Load`'s real registration order
    /// (`NetworkInitializer.cs:9-26`): 0 Liquid, 1 Text, 2 Ping, 3 Ambience, 4 Bestiary,
    /// 5 CreativeUnlocks, 6 CreativePowers, 7 CreativeUnlocksPlayerReport, 8 TeleportPylon,
    /// **9 Particles**, 10 CreativePowerPermissions, 11 Banners, **12 CraftingRequests**.
    #[test]
    fn the_new_module_ids_are_the_vanilla_registration_index() {
        assert_eq!(MODULE_PARTICLES, 9);
        assert_eq!(MODULE_CRAFTING_REQUESTS, 12);
    }

    /// `relay_module` reproduces the module-82 envelope byte for byte, which is what makes it
    /// equivalent to vanilla's own deserialize-then-reserialize relay (see its own doc for why).
    #[test]
    fn relay_module_wraps_the_payload_unchanged() {
        let mut w = Writer::new();
        w.u16(MODULE_PING).f32(12.0).f32(34.0);
        let payload = w.as_slice().to_vec();

        let frame = relay_module(&payload).unwrap();
        assert_eq!(frame[2], id::NET_MODULES);
        assert_eq!(&frame[3..], payload.as_slice());
    }

    #[test]
    fn peek_module_id_reads_the_leading_id_only() {
        let mut w = Writer::new();
        w.u16(MODULE_PARTICLES).u8(1).u8(2).u8(3);
        assert_eq!(peek_module_id(w.as_slice()).unwrap(), MODULE_PARTICLES);
    }

    /// A request naming two literal items and one chest, checked field by field against
    /// `NetCraftingRequestsModule.WriteRequest`'s own shape (`CraftingRequests.cs:27-42`): the
    /// item id is a plain four-byte int, everything else (both counts, the stack, the chest
    /// index) is 7-bit encoded.
    #[test]
    fn decodes_a_craft_request_with_literal_items_and_a_chest() {
        let mut w = Writer::new();
        w.u16(MODULE_CRAFTING_REQUESTS);
        w.var_u32(2); // item count
        w.i32(9).var_u32(5); // 5x item 9 (Wood)
        w.i32(3335).var_u32(1); // 1x item 3335
        w.var_u32(1); // chest count
        w.var_u32(4); // chest index 4

        let request = decode_craft_request(w.as_slice()).unwrap().unwrap();
        assert_eq!(
            request.items,
            vec![
                CraftIngredient {
                    item_id_or_group: 9,
                    stack: 5
                },
                CraftIngredient {
                    item_id_or_group: 3335,
                    stack: 1
                },
            ]
        );
        assert_eq!(request.chests, vec![Some(4)]);
    }

    /// `Main.chest[num3]` becomes `null` when the client's index is negative
    /// (`CraftingRequests.cs:63-64`); this server has no `Main.chest` to index into with a
    /// negative number regardless, so it drops the same slot to `None`.
    #[test]
    fn a_negative_chest_index_decodes_to_none() {
        let mut w = Writer::new();
        w.u16(MODULE_CRAFTING_REQUESTS);
        w.var_u32(0); // no items
        w.var_u32(1); // one chest
        // 7-bit encoding a negative `int` writes it as the reinterpreted `uint`, the same as
        // .NET's own `Write7BitEncodedInt(int)` does — five bytes for -1.
        w.var_u32(u32::from_le_bytes((-1i32).to_le_bytes()));

        let request = decode_craft_request(w.as_slice()).unwrap().unwrap();
        assert_eq!(request.chests, vec![None]);
    }

    /// `RequiredItemEntry.IsRecipeGroup` (`Recipe.cs:21`): any id at or past the fake-item-id
    /// offset names a `RecipeGroup`, not a literal item.
    #[test]
    fn a_fake_item_id_is_recognised_as_a_recipe_group() {
        let literal = CraftIngredient {
            item_id_or_group: 9,
            stack: 1,
        };
        let group = CraftIngredient {
            item_id_or_group: CraftIngredient::RECIPE_GROUP_OFFSET + 3,
            stack: 1,
        };
        assert!(!literal.is_recipe_group());
        assert!(group.is_recipe_group());
    }

    #[test]
    fn ignores_other_modules_for_craft_requests_too() {
        let mut w = Writer::new();
        w.u16(MODULE_TEXT).var_u32(0).var_u32(0);
        assert_eq!(decode_craft_request(w.as_slice()).unwrap(), None);
    }

    #[test]
    fn a_craft_response_round_trips_the_approval_bit() {
        for approved in [true, false] {
            let frame = craft_response(approved).unwrap();
            let mut r = PacketReader::new(&frame[3..]);
            assert_eq!(r.u16().unwrap(), MODULE_CRAFTING_REQUESTS);
            assert_eq!(r.bool().unwrap(), approved);
        }
    }
}
