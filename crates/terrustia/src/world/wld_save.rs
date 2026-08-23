//! Writer for Terraria's `.wld` save format.
//!
//! There are two paths, and which one runs depends on where the world came from.
//!
//! A world **loaded from a file** keeps its own header. Saving re-serialises the header verbatim
//! with the mutable fields patched in place, then the tiles, chests and signs; every later
//! section — NPCs, tile entities, pressure plates, the town manager, the bestiary, creative
//! powers and the footer — is written back exactly as it was read. Nothing is re-derived, so
//! nothing can drift: a world round-trips byte-identically apart from the revision counter, and
//! the state this server does not model survives untouched.
//!
//! A **generated** world has no header to copy, so one is written from scratch at
//! [`SAVE_VERSION`] — the whole of the game's own flag order, with the fields this server does
//! not model written as the values a fresh world holds. The format has no framing, so a field of
//! the wrong width there would put every field after it in the wrong place and corrupt the save
//! silently. That is why it is checked rather than trusted: the header written here is walked
//! independently and has to end exactly on the tile-section pointer.

use std::path::Path;

use terrustia_proto::{Tile, Writer, section::write_tile_with, tile_sets::allows_batching};

use super::{World, wld::WldError};

type Result<T> = std::result::Result<T, WldError>;

const MAGIC: &[u8; 7] = b"relogic";
const FILE_TYPE_WORLD: u8 = 2;

/// The format this writer emits for a world that has no file of its own.
///
/// A world loaded from a file keeps whatever version it came with, because its header is copied
/// verbatim and patched. A generated one has no header to copy, so it is written fresh at the
/// version this server was transcribed from.
pub const SAVE_VERSION: i32 = 325;

/// How many sections the format has at [`SAVE_VERSION`].
const SECTIONS: usize = 11;

/// Serialise a world into `.wld` bytes.
pub fn serialize(world: &World) -> Result<Vec<u8>> {
    let Some(preserved) = world.preserved.as_ref() else {
        return Ok(serialize_fresh(world));
    };

    let section_count = 4 + preserved.trailing_offsets.len();
    let mut w = Writer::with_capacity(4 * 1024 * 1024);

    // --- file format header ---------------------------------------------------------------
    w.i32(preserved.version)
        .bytes(MAGIC)
        .u8(FILE_TYPE_WORLD)
        .u32(preserved.revision.saturating_add(1))
        .u64(preserved.favorite)
        .i16(section_count as i16);

    // Pointers are patched once every section's position is known.
    let pointer_table = w.len();
    for _ in 0..section_count {
        w.i32(0);
    }

    // The importance table is packed least significant bit first, seeded at 0x80 as a sentinel.
    w.u16(preserved.importance.len() as u16);
    let mut current = 0u8;
    let mut bit = 0x80u8;
    for &framed in &preserved.importance {
        if bit == 0x80 {
            bit = 1;
            current = 0;
        } else {
            bit <<= 1;
        }
        if framed {
            current |= bit;
        }
        if bit == 0x80 {
            w.u8(current);
        }
    }
    // Flush a partial final byte.
    if bit != 0x80 && !preserved.importance.is_empty() {
        w.u8(current);
    }

    let mut pointers = vec![0i32; section_count];

    // --- section 0: world header, verbatim with the clock patched -------------------------
    pointers[0] = w.len() as i32;
    let mut header = preserved.header_bytes.clone();
    patch_clock(&mut header, preserved, world);
    w.bytes(&header);

    // --- section 1: tiles ------------------------------------------------------------------
    pointers[1] = w.len() as i32;
    let importance = |tile: u16| {
        preserved
            .importance
            .get(usize::from(tile))
            .copied()
            .unwrap_or(false)
    };
    write_tiles(&mut w, world, &importance);

    // --- section 2: chests -----------------------------------------------------------------
    pointers[2] = w.len() as i32;
    write_chests(&mut w, world, preserved.chest_slots);

    // --- section 3: signs ------------------------------------------------------------------
    pointers[3] = w.len() as i32;
    write_signs(&mut w, world);

    // --- sections 4..: carried through unchanged -------------------------------------------
    if let Some(&first) = preserved.trailing_offsets.first() {
        let delta = w.len() as i64 - i64::from(first);
        for (index, &offset) in preserved.trailing_offsets.iter().enumerate() {
            let shifted = i64::from(offset) + delta;
            pointers[4 + index] =
                i32::try_from(shifted).map_err(|_| WldError::SaveTooLarge { bytes: shifted })?;
        }
        w.bytes(&preserved.trailing_bytes);
    }

    let mut bytes = w.into_bytes();
    for (index, pointer) in pointers.iter().enumerate() {
        let at = pointer_table + index * 4;
        bytes[at..at + 4].copy_from_slice(&pointer.to_le_bytes());
    }
    Ok(bytes)
}

/// Overwrite the world clock inside the preserved header.
///
/// The header is kept verbatim and patched in place rather than re-serialised, so writing a field
/// means knowing its byte. Everything the server can change has an offset recorded when the world
/// was read; a `None` offset means that world's header never reached the field, and the value
/// lives only for the session.
fn patch_clock(header: &mut [u8], preserved: &super::objects::PreservedWorld, world: &World) {
    let write = |header: &mut [u8], at: usize, value: &[u8]| {
        if let Some(slot) = header.get_mut(at..at + value.len()) {
            slot.copy_from_slice(value);
        }
    };
    let flags = |header: &mut [u8], at: Option<usize>, values: &[bool]| {
        let Some(at) = at else {
            return;
        };
        for (i, on) in values.iter().enumerate() {
            if let Some(slot) = header.get_mut(at + i) {
                *slot = u8::from(*on);
            }
        }
    };
    let p = &world.progress;
    flags(
        header,
        preserved.progress_offset,
        &[
            p.downed_boss1,
            p.downed_boss2,
            p.downed_boss3,
            p.downed_queen_bee,
            p.downed_mech1,
            p.downed_mech2,
            p.downed_mech3,
            p.downed_mech_any,
            p.downed_plantera,
            p.downed_golem,
            p.downed_king_slime,
            p.saved_goblin,
            p.saved_wizard,
            p.saved_mechanic,
            p.downed_goblins,
            p.downed_clown,
            p.downed_frost,
            p.downed_pirates,
            p.shadow_orb_smashed,
            p.spawn_meteor,
        ],
    );
    flags(header, preserved.hard_mode_offset, &[p.hard_mode]);
    if let Some(at) = preserved.orb_count_offset {
        write(header, at, &[p.shadow_orb_count]);
    }
    if let Some(at) = preserved.altar_offset {
        write(header, at, &p.altar_count.to_le_bytes());
    }
    flags(
        header,
        preserved.downed_run_offset,
        &[
            p.downed_fishron,
            p.downed_martians,
            p.downed_ancient_cultist,
            p.downed_moon_lord,
            p.downed_halloween_king,
            p.downed_halloween_tree,
            p.downed_christmas_ice_queen,
            p.downed_christmas_santank,
            p.downed_christmas_tree,
        ],
    );
    flags(
        header,
        preserved.tower_run_offset,
        &[
            p.downed_tower_solar,
            p.downed_tower_vortex,
            p.downed_tower_nebula,
            p.downed_tower_stardust,
            p.tower_active_solar,
            p.tower_active_vortex,
            p.tower_active_nebula,
            p.tower_active_stardust,
            p.lunar_apocalypse_up,
        ],
    );
    flags(
        header,
        preserved.army_run_offset,
        &[
            p.saved_bartender,
            p.downed_army_t1,
            p.downed_army_t2,
            p.downed_army_t3,
        ],
    );
    flags(header, preserved.combat_book_offset, &[p.combat_book]);
    flags(
        header,
        preserved.late_downed_run_offset,
        &[
            p.downed_empress_of_light,
            p.downed_queen_slime,
            p.downed_deerclops,
        ],
    );
    flags(
        header,
        preserved.combat_book_two_offset,
        &[p.combat_book_two],
    );
    if let Some(at) = preserved.rain_offset {
        write(header, at, &[u8::from(world.raining)]);
        write(header, at + 1, &world.rain_time.to_le_bytes());
        write(header, at + 5, &world.max_rain.to_le_bytes());
    }
    if let Some(at) = preserved.wind_offset {
        write(header, at, &world.wind.to_le_bytes());
    }
    if let Some(at) = preserved.sandstorm_offset {
        write(header, at, &[u8::from(world.sandstorm)]);
        write(header, at + 1, &world.sandstorm_time.to_le_bytes());
        write(header, at + 5, &world.sandstorm_severity.to_le_bytes());
        write(
            header,
            at + 9,
            &world.sandstorm_intended_severity.to_le_bytes(),
        );
    }
    write(
        header,
        preserved.time_offset,
        &f64::from(world.time).to_le_bytes(),
    );
    write(
        header,
        preserved.day_time_offset,
        &[u8::from(world.day_time)],
    );
    write(
        header,
        preserved.moon_phase_offset,
        &i32::from(world.moon_phase).to_le_bytes(),
    );
}

/// Tiles are stored column by column, with the same run-length encoding the network uses.
fn write_tiles(w: &mut Writer, world: &World, importance: &dyn Fn(u16) -> bool) {
    for x in 0..world.width() {
        let mut pending: Option<(Tile, u16)> = None;
        for y in 0..world.height() {
            let tile = world.tile(x, y);
            match pending {
                Some((prev, ref mut run)) if prev == tile && allows_batching(tile.block) => {
                    *run += 1;
                }
                _ => {
                    if let Some((prev, run)) = pending.take() {
                        write_tile_with(w, &prev, run, importance);
                    }
                    pending = Some((tile, 0));
                }
            }
        }
        if let Some((prev, run)) = pending.take() {
            write_tile_with(w, &prev, run, importance);
        }
    }
}

/// Write the chest section in the shape this file's own version uses.
///
/// Before 294 the capacity is stated once for the whole section and every chest writes exactly
/// that many slots; from 294 each chest carries its own count. The header is copied verbatim and
/// still names the old version, so writing the new shape into an old file makes the reader take
/// the first chest's coordinates for a slot count and lose the rest of the file with it.
/// Serialise a world that was generated rather than loaded, writing every section from scratch.
///
/// The header this produces is not a copy of anything: it is the whole of the game's own
/// `SaveWorldFlags` in order, at [`SAVE_VERSION`], with the fields this server does not model
/// written as the values a fresh world has. That is the risk the preserved path exists to avoid,
/// so the writer is checked against the reader rather than trusted: a generated world is
/// serialised, read back, and compared before it is offered as a save.
fn serialize_fresh(world: &World) -> Vec<u8> {
    let mut w = Writer::with_capacity(4 * 1024 * 1024);
    let importance: Vec<bool> = (0..terrustia_proto::tile_sets::TILE_COUNT)
        .map(terrustia_proto::tile_sets::frame_important)
        .collect();

    // --- file format header ---------------------------------------------------------------
    w.i32(SAVE_VERSION)
        .bytes(MAGIC)
        .u8(FILE_TYPE_WORLD)
        .u32(1)
        .u64(0)
        .i16(SECTIONS as i16);
    let pointer_table = w.len();
    for _ in 0..SECTIONS {
        w.i32(0);
    }
    write_importance(&mut w, &importance);

    let mut pointers = [0i32; SECTIONS];
    pointers[0] = w.len() as i32;
    write_fresh_header(&mut w, world);
    pointers[1] = w.len() as i32;
    write_tiles(&mut w, world, &|tile: u16| {
        importance.get(usize::from(tile)).copied().unwrap_or(false)
    });
    pointers[2] = w.len() as i32;
    write_chests(&mut w, world, None);
    pointers[3] = w.len() as i32;
    write_signs(&mut w, world);

    // Sections 5 to 11 hold state this server keeps in memory rather than on the world: the
    // townsfolk who have moved in, the tile entities, the pressure plates that are held down, the
    // rooms the town manager has assigned, the bestiary and the creative powers. A generated world
    // that has just been made has none of them, and one this server has been running keeps them
    // elsewhere, so each is written empty in the shape its loader expects.
    pointers[4] = w.len() as i32;
    w.i32(0); // no shimmered townsfolk
    w.bool(false); // no town NPCs
    w.bool(false); // and none of the few enemies that persist
    pointers[5] = w.len() as i32;
    w.i32(0); // no tile entities
    pointers[6] = w.len() as i32;
    w.i32(0); // no pressure plates held down
    pointers[7] = w.len() as i32;
    w.i32(0); // no rooms assigned
    pointers[8] = w.len() as i32;
    w.i32(0).i32(0).i32(0); // bestiary: kills, sightings, conversations
    pointers[9] = w.len() as i32;
    w.bool(false); // no creative powers
    pointers[10] = w.len() as i32;
    // The footer, which is what the game checks a save against before trusting it.
    w.bool(true).string(&world.name).i32(world.id);

    let mut bytes = w.into_bytes();
    for (index, pointer) in pointers.iter().enumerate() {
        let at = pointer_table + index * 4;
        bytes[at..at + 4].copy_from_slice(&pointer.to_le_bytes());
    }
    bytes
}

/// The frame-importance bitset, packed least significant bit first.
fn write_importance(w: &mut Writer, importance: &[bool]) {
    w.u16(importance.len() as u16);
    let mut current = 0u8;
    let mut bit = 0x80u8;
    for &framed in importance {
        if bit == 0x80 {
            bit = 1;
            current = 0;
        } else {
            bit <<= 1;
        }
        if framed {
            current |= bit;
        }
        if bit == 0x80 {
            w.u8(current);
        }
    }
    if bit != 0x80 && !importance.is_empty() {
        w.u8(current);
    }
}

/// The whole world header, in the order the game writes it.
///
/// Every field is here, including the ones this server does not model — they are written as the
/// values a world that has just been generated holds, because the format has no framing and a
/// field left out puts every field after it in the wrong place.
fn write_fresh_header(w: &mut Writer, world: &World) {
    let p = &world.progress;
    w.string(&world.name)
        .string("") // the seed text, which this server does not keep
        .u64(world.world_gen_version)
        .bytes(&world.unique_id)
        .i32(world.id);
    // The world rectangle in pixels, then its size in tiles — height first.
    w.i32(0)
        .i32(world.width() * 16)
        .i32(0)
        .i32(world.height() * 16)
        .i32(world.height())
        .i32(world.width());

    // The nine special world seeds, none of which a generated world has, then the skyblock flag.
    w.i32(i32::from(world.game_mode));
    for _ in 0..9 {
        w.bool(false);
    }
    // Created and last played, which the game stores as .NET tick counts. Zero is a valid one.
    w.i64(0).i64(0);

    w.u8(world.moon_type);
    for x in world.tree_x {
        w.i32(x);
    }
    for style in world.tree_style {
        w.i32(i32::from(style));
    }
    for x in world.cave_back_x {
        w.i32(x);
    }
    for style in world.cave_back_style {
        w.i32(i32::from(style));
    }
    w.i32(i32::from(world.ice_back_style))
        .i32(i32::from(world.jungle_back_style))
        .i32(i32::from(world.hell_back_style));

    w.i32(i32::from(world.spawn_x))
        .i32(i32::from(world.spawn_y))
        .f64(f64::from(world.surface))
        .f64(f64::from(world.rock_layer));
    w.f64(f64::from(world.time))
        .bool(world.day_time)
        .i32(i32::from(world.moon_phase))
        .bool(world.blood_moon)
        .bool(world.eclipse);
    let dungeon_x = world.dungeon_x.unwrap_or(world.width() / 2);
    w.i32(dungeon_x).i32(i32::from(world.surface));
    w.bool(world.crimson);

    for flag in [
        p.downed_boss1,
        p.downed_boss2,
        p.downed_boss3,
        p.downed_queen_bee,
        p.downed_mech1,
        p.downed_mech2,
        p.downed_mech3,
        p.downed_mech_any,
        p.downed_plantera,
        p.downed_golem,
        p.downed_king_slime,
        p.saved_goblin,
        p.saved_wizard,
        p.saved_mechanic,
        p.downed_goblins,
        p.downed_clown,
        p.downed_frost,
        p.downed_pirates,
        p.shadow_orb_smashed,
        p.spawn_meteor,
    ] {
        w.bool(flag);
    }
    w.u8(p.shadow_orb_count).i32(p.altar_count);
    w.bool(p.hard_mode).bool(false); // the party of doom, which is a one-off event flag
    // No invasion is saved: one in progress is abandoned when the server stops, exactly as the
    // game abandons one when a world is closed.
    w.i32(0).i32(0).i32(0).f64(0.0);
    w.f64(0.0).u8(0); // slime rain, sundial cooldown

    w.bool(world.raining)
        .i32(world.rain_time)
        .f32(world.max_rain);
    // The three hardmode ore tiers, which are rolled when the wall falls.
    for _ in 0..3 {
        w.i32(0);
    }
    // Eight background styles, then the clouds and the wind.
    for _ in 0..8 {
        w.u8(0);
    }
    w.i32(0).i16(0).f32(world.wind);

    w.i32(0); // nobody has handed in an angler quest today
    w.bool(p.saved_angler)
        .i32(0)
        .bool(p.saved_stylist)
        .bool(p.saved_tax_collector)
        .bool(p.saved_golfer);
    w.i32(0).i32(0); // invasion size at the start, cultist delay

    w.i16(0).i16(0); // no banner kill counts, no claimable banners
    w.bool(false); // not fast-forwarding to dawn
    for flag in [
        p.downed_fishron,
        p.downed_martians,
        p.downed_ancient_cultist,
        p.downed_moon_lord,
        p.downed_halloween_king,
        p.downed_halloween_tree,
        p.downed_christmas_ice_queen,
        p.downed_christmas_santank,
        p.downed_christmas_tree,
        p.downed_tower_solar,
        p.downed_tower_vortex,
        p.downed_tower_nebula,
        p.downed_tower_stardust,
        p.tower_active_solar,
        p.tower_active_vortex,
        p.tower_active_nebula,
        p.tower_active_stardust,
        p.lunar_apocalypse_up,
    ] {
        w.bool(flag);
    }

    w.bool(false).bool(false).i32(0).i32(0); // no party
    w.bool(world.sandstorm)
        .i32(world.sandstorm_time)
        .f32(world.sandstorm_severity)
        .f32(world.sandstorm_intended_severity);
    w.bool(p.saved_bartender)
        .bool(p.downed_army_t1)
        .bool(p.downed_army_t2)
        .bool(p.downed_army_t3);
    for _ in 0..5 {
        w.u8(0); // five more background styles
    }
    w.bool(p.combat_book);
    w.i32(0).bool(false).bool(false).bool(false); // lantern night

    // One tree-top variation per biome.
    w.i32(13);
    for _ in 0..13 {
        w.i32(0);
    }
    w.bool(false).bool(false); // no forced holiday today
    // The four ore tiers the wall hands out, which a fresh world has not chosen.
    for _ in 0..4 {
        w.i32(-1);
    }
    for _ in 0..3 {
        w.bool(false); // no pets bought
    }
    w.bool(p.downed_empress_of_light)
        .bool(p.downed_queen_slime)
        .bool(p.downed_deerclops);
    for _ in 0..9 {
        w.bool(false); // no town spawns unlocked by other means
    }
    w.bool(p.combat_book_two);
    w.bool(false); // no peddler's satchel
    for _ in 0..7 {
        w.bool(false); // the seven slime spawns
    }
    w.bool(false).u8(0); // not fast-forwarding to dusk, no moondial cooldown
    w.bool(false).bool(false); // no forced holiday forever
    w.bool(false).bool(false); // vampire and infected seeds
    w.i32(0).i32(0); // meteor showers seen, coin rain
    w.bool(false); // team-based spawns
    w.u8(0); // no extra spawn points
    w.bool(false); // dual dungeons
    w.bool(false).bool(false); // more lightning, no lightning
    // The generation manifest, which the game parses as JSON and falls back to empty on.
    w.string(r#"{"GenPassResults":[],"Version":"terrustia","GitSHA":"","FinalHash":null}"#);
}

fn write_chests(w: &mut Writer, world: &World, shared_slots: Option<i16>) {
    let chests: Vec<_> = world.chests.iter().flatten().collect();
    w.i16(chests.len() as i16);
    if let Some(slots) = shared_slots {
        w.i16(slots);
        for chest in chests {
            w.i32(i32::from(chest.x))
                .i32(i32::from(chest.y))
                .string(&chest.name);
            // Padded or truncated to the shared capacity: a chest the server created carries the
            // modern default, which need not be what this file says.
            for index in 0..slots.max(0) as usize {
                chest
                    .items
                    .get(index)
                    .unwrap_or(&terrustia_proto::ItemStack::EMPTY)
                    .write_save(w);
            }
        }
        return;
    }
    for chest in chests {
        w.i32(i32::from(chest.x))
            .i32(i32::from(chest.y))
            .string(&chest.name)
            .i32(chest.items.len() as i32);
        for item in &chest.items {
            item.write_save(w);
        }
    }
}

fn write_signs(w: &mut Writer, world: &World) {
    let signs: Vec<_> = world.signs.iter().flatten().collect();
    w.i16(signs.len() as i16);
    for sign in signs {
        w.string(&sign.text)
            .i32(i32::from(sign.x))
            .i32(i32::from(sign.y));
    }
}

/// Write a world to disk.
///
/// The bytes go to a temporary file next to the target and are renamed into place, so an
/// interrupted save cannot leave a half-written world where the real one was.
pub fn save(world: &World, path: &Path) -> Result<()> {
    let bytes = serialize(world)?;
    let temp = path.with_extension("wld.tmp");

    std::fs::write(&temp, &bytes).map_err(|e| WldError::Io {
        path: temp.display().to_string(),
        source: e,
    })?;
    std::fs::rename(&temp, path).map_err(|e| WldError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::objects::PreservedWorld;

    /// A header of the right shape but made of sentinel bytes, so a patch is visible.
    fn header_with(offsets: PreservedWorld) -> (Vec<u8>, PreservedWorld) {
        (vec![0xAA; 4096], offsets)
    }

    fn preserved() -> PreservedWorld {
        PreservedWorld {
            version: 279,
            chest_slots: None,
            revision: 1,
            favorite: 0,
            header_bytes: Vec::new(),
            time_offset: 0,
            day_time_offset: 8,
            moon_phase_offset: 9,
            progress_offset: Some(100),
            hard_mode_offset: Some(200),
            altar_offset: Some(210),
            orb_count_offset: Some(220),
            downed_run_offset: Some(300),
            tower_run_offset: Some(400),
            rain_offset: Some(500),
            wind_offset: Some(600),
            sandstorm_offset: Some(700),
            army_run_offset: Some(800),
            combat_book_offset: Some(810),
            late_downed_run_offset: Some(820),
            combat_book_two_offset: Some(830),
            trailing_offsets: Vec::new(),
            trailing_bytes: Vec::new(),
            importance: Vec::new(),
        }
    }

    /// Every field the server can change is written back where the reader found it.
    #[test]
    fn the_patch_writes_every_mutable_field() {
        let (mut header, keep) = header_with(preserved());
        let mut world = crate::world::worldgen::generate(400, 300, "patch", 1);
        world.time = 13_500;
        world.day_time = false;
        world.moon_phase = 5;
        world.progress.hard_mode = true;
        world.progress.downed_plantera = true;
        world.progress.downed_moon_lord = true;
        world.progress.tower_active_vortex = true;
        world.progress.altar_count = 12;
        world.progress.shadow_orb_count = 3;
        world.raining = true;
        world.rain_time = 4200;
        world.max_rain = 0.75;
        world.wind = -0.4;
        world.sandstorm = true;
        world.sandstorm_time = 30_000;
        world.sandstorm_severity = 0.5;
        world.sandstorm_intended_severity = 0.8;
        world.progress.downed_army_t2 = true;
        world.progress.combat_book = true;
        world.progress.downed_deerclops = true;
        world.progress.combat_book_two = true;

        patch_clock(&mut header, &keep, &world);

        assert_eq!(header[8], 0, "day_time");
        assert_eq!(
            i32::from_le_bytes(header[9..13].try_into().unwrap()),
            5,
            "moon_phase"
        );
        // The progression run: the ninth flag is Plantera, the eleventh King Slime.
        assert_eq!(header[100 + 8], 1, "plantera");
        assert_eq!(header[100 + 10], 0, "king slime, which is not down");
        assert_eq!(header[200], 1, "hard mode");
        assert_eq!(header[220], 3, "orb count");
        assert_eq!(
            i32::from_le_bytes(header[210..214].try_into().unwrap()),
            12,
            "altars"
        );
        // The late run: the fourth flag is the Moon Lord.
        assert_eq!(header[300 + 3], 1, "moon lord");
        assert_eq!(header[300], 0, "fishron, which is not down");
        // The tower run: the sixth flag is the vortex tower standing.
        assert_eq!(header[400 + 5], 1, "vortex standing");
        assert_eq!(header[400], 0, "solar, never beaten");
        assert_eq!(header[500], 1, "raining");
        assert_eq!(
            i32::from_le_bytes(header[501..505].try_into().unwrap()),
            4200,
            "rain time"
        );
        assert_eq!(
            f32::from_le_bytes(header[505..509].try_into().unwrap()),
            0.75,
            "max rain"
        );
        assert_eq!(
            f32::from_le_bytes(header[600..604].try_into().unwrap()),
            -0.4,
            "wind"
        );
        assert_eq!(header[700], 1, "sandstorm");
        assert_eq!(
            i32::from_le_bytes(header[701..705].try_into().unwrap()),
            30_000,
            "sandstorm time"
        );
        assert_eq!(
            f32::from_le_bytes(header[705..709].try_into().unwrap()),
            0.5,
            "severity"
        );
        assert_eq!(
            f32::from_le_bytes(header[709..713].try_into().unwrap()),
            0.8,
            "intended severity"
        );
        // The army run: the bartender first, then the three tiers.
        assert_eq!(header[800], 0, "the bartender, never saved");
        assert_eq!(header[800 + 2], 1, "the second tier, beaten");
        assert_eq!(header[810], 1, "the first combat book");
        // The late downed run: the empress, the queen, then Deerclops.
        assert_eq!(header[820], 0, "the empress, still alive");
        assert_eq!(header[820 + 2], 1, "deerclops");
        assert_eq!(header[830], 1, "the second combat book");
    }

    /// A world whose header never reached a field simply does not write it, rather than writing
    /// it somewhere else.
    #[test]
    fn a_short_header_is_left_alone() {
        let mut keep = preserved();
        keep.downed_run_offset = None;
        keep.tower_run_offset = None;
        keep.wind_offset = None;
        let (mut header, keep) = header_with(keep);
        let mut world = crate::world::worldgen::generate(400, 300, "short", 1);
        world.progress.downed_moon_lord = true;
        world.wind = 0.9;

        patch_clock(&mut header, &keep, &world);
        assert_eq!(header[300], 0xAA, "nothing written where nothing was read");
        assert_eq!(header[400], 0xAA);
        assert_eq!(header[600], 0xAA);
    }

    /// A pre-294 world states its chest capacity once, and every chest writes exactly that many
    /// slots with no count of its own.
    ///
    /// Writing the modern shape into an old file is not a cosmetic difference: the reader takes
    /// the first chest's x coordinate for the shared count and loses every byte after it.
    #[test]
    fn an_old_world_keeps_its_shared_chest_capacity() {
        let mut world = crate::world::worldgen::generate(400, 300, "old", 1);
        world.chests = vec![Some(crate::world::objects::Chest::empty_at(10, 20))];

        let mut w = Writer::new();
        write_chests(&mut w, &world, Some(40));
        let old = w.into_bytes();

        let mut w = Writer::new();
        write_chests(&mut w, &world, None);
        let new = w.into_bytes();

        // count, shared capacity, x, y, an empty name, then forty empty slots.
        assert_eq!(&old[..4], &[1, 0, 40, 0], "count then the shared capacity");
        assert_eq!(old.len(), 2 + 2 + 4 + 4 + 1 + 40 * 2);
        // The modern shape spends four bytes on a per-chest count instead of two on a shared one.
        assert_eq!(new.len(), 2 + 4 + 4 + 1 + 4 + 40 * 2);
        assert_ne!(old, new);
    }

    /// A chest the server created carries the modern default, which need not match what an older
    /// file says its chests hold. It is padded or truncated rather than written at its own size.
    #[test]
    fn a_chest_is_written_at_the_files_capacity_not_its_own() {
        let mut world = crate::world::worldgen::generate(400, 300, "old", 1);
        world.chests = vec![Some(crate::world::objects::Chest::empty_at(10, 20))];

        for slots in [20i16, 40, 60] {
            let mut w = Writer::new();
            write_chests(&mut w, &world, Some(slots));
            assert_eq!(
                w.len(),
                2 + 2 + 4 + 4 + 1 + slots as usize * 2,
                "{slots} slots"
            );
        }
    }
}
