//! Writer for Terraria's `.wld` save format.
//!
//! Only worlds that were **loaded from a file** can be saved. Saving re-serialises the header
//! (verbatim, with the clock patched), the tiles, the chests and the signs; every later section —
//! NPCs, tile entities, pressure plates, the town manager, the bestiary, creative powers and the
//! footer — is written back exactly as it was read.
//!
//! That restriction is deliberate. Re-creating a world header from nothing means transcribing 138
//! further fields across 26 version gates plus five nested sub-loaders, and any drift there would
//! corrupt a save silently rather than fail loudly.

use std::path::Path;

use terrustia_proto::{Tile, Writer, section::write_tile_with, tile_sets::allows_batching};

use super::{World, wld::WldError};

type Result<T> = std::result::Result<T, WldError>;

const MAGIC: &[u8; 7] = b"relogic";
const FILE_TYPE_WORLD: u8 = 2;

/// Serialise a world into `.wld` bytes.
pub fn serialize(world: &World) -> Result<Vec<u8>> {
    let preserved = world
        .preserved
        .as_ref()
        .ok_or(WldError::CannotSaveGeneratedWorld)?;

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
    write_chests(&mut w, world);

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
    if let Some(at) = preserved.rain_offset {
        write(header, at, &[u8::from(world.raining)]);
        write(header, at + 1, &world.rain_time.to_le_bytes());
        write(header, at + 5, &world.max_rain.to_le_bytes());
    }
    if let Some(at) = preserved.wind_offset {
        write(header, at, &world.wind.to_le_bytes());
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

fn write_chests(w: &mut Writer, world: &World) {
    let chests: Vec<_> = world.chests.iter().flatten().collect();
    w.i16(chests.len() as i16);
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
}
