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
fn patch_clock(header: &mut [u8], preserved: &super::objects::PreservedWorld, world: &World) {
    let write = |header: &mut [u8], at: usize, value: &[u8]| {
        if let Some(slot) = header.get_mut(at..at + value.len()) {
            slot.copy_from_slice(value);
        }
    };
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
