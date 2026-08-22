//! Reader for Terraria's `.wld` save format.
//!
//! Tiles are encoded exactly as they are in a network section — the same flag chain, the same
//! field order, the same run lengths — so [`terrustia_proto::section::read_tile_with`] is shared
//! between the two. The differences are that the file walks the world column by column rather than
//! row by row, and that it carries its own frame-importance table so an old save still loads after
//! the game's table changes.
//!
//! Layouts transcribed from `Terraria.IO.WorldFile` in the 1.4.5.7 build.

use std::path::Path;

use terrustia_proto::{ItemStack, PacketReader, section::read_tile_with};
use thiserror::Error;
use tracing::debug;

use super::progress::Progress;
use super::{
    World,
    objects::{Chest, PreservedWorld, Sign},
};

/// Oldest save version this reader accepts.
///
/// The format grew a field at a time across dozens of releases; rather than guess at gates we
/// never exercise, only versions from the 1.4.4 era onward are accepted and anything older is
/// refused with a clear message.
pub const MIN_VERSION: i32 = 279;

/// `"relogic"`, followed by a file-type byte.
const MAGIC: &[u8; 7] = b"relogic";
const FILE_TYPE_WORLD: u8 = 2;

#[derive(Debug, Error)]
pub enum WldError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("not a Terraria world file (magic was {found:?})")]
    BadMagic { found: Vec<u8> },

    #[error("file type {found} is not a world (expected {FILE_TYPE_WORLD})")]
    NotAWorld { found: u8 },

    #[error(
        "world format version {found} is too old; this reader handles {MIN_VERSION} and newer \
         (open and re-save the world in Terraria to upgrade it)"
    )]
    TooOld { found: i32 },

    #[error("world claims implausible dimensions {width}x{height}")]
    BadDimensions { width: i32, height: i32 },

    #[error(
        "the progression flags did not decode as flags (invasion type {invasion_type}, size \
         {invasion_size}); the header layout has changed and this reader is reading the wrong bytes"
    )]
    ProgressionOutOfStep {
        invasion_type: i32,
        invasion_size: i32,
    },

    #[error("section pointer {index} is {pointer}, outside a {len}-byte file")]
    BadSectionPointer {
        index: usize,
        pointer: i64,
        len: usize,
    },

    #[error("tile data ended early: {decoded} of {expected} tiles")]
    TruncatedTiles { decoded: usize, expected: usize },

    #[error(
        "this world was generated rather than loaded from a file, and generated worlds cannot be \
         saved yet: writing a world header from scratch means reproducing 138 further fields \
         across 26 version gates, which would corrupt a save silently if it drifted"
    )]
    CannotSaveGeneratedWorld,

    #[error("world would serialise to {bytes} bytes, past the format's 2 GiB section offsets")]
    SaveTooLarge { bytes: i64 },

    #[error("malformed data at byte {offset}: {source}")]
    Decode {
        offset: usize,
        #[source]
        source: terrustia_proto::ProtoError,
    },
}

type Result<T> = std::result::Result<T, WldError>;

/// Load a world from a `.wld` file.
pub fn load(path: &Path) -> Result<World> {
    let bytes = std::fs::read(path).map_err(|e| WldError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    parse(&bytes)
}

/// Parse an in-memory `.wld` image.
pub fn parse(bytes: &[u8]) -> Result<World> {
    let mut r = PacketReader::new(bytes);
    let file = read_file_header(&mut r, bytes.len())?;

    let header_start = file.sections[0] as usize;
    seek(&mut r, bytes, file.sections[0], 0)?;
    let (mut world, offsets) = read_world_header(&mut r, file.version, header_start)?;

    // The whole header is kept verbatim so a later save preserves the progression flags, event
    // state and sub-structures this server does not model.
    let tile_start = file.sections[1] as usize;
    let header_bytes = bytes
        .get(header_start..tile_start)
        .ok_or(WldError::BadSectionPointer {
            index: 1,
            pointer: i64::from(file.sections[1]),
            len: bytes.len(),
        })?
        .to_vec();

    seek(&mut r, bytes, file.sections[1], 1)?;
    read_tiles(&mut r, &mut world, &file.importance)?;

    if file.sections.len() > 2 {
        seek(&mut r, bytes, file.sections[2], 2)?;
        world.chests = read_chests(&mut r, file.version)?;
    }
    if file.sections.len() > 3 {
        seek(&mut r, bytes, file.sections[3], 3)?;
        world.signs = read_signs(&mut r)?;
    }

    // Sections 4 onwards hold NPCs, tile entities, pressure plates, the town manager, the bestiary
    // and creative powers. None of those are modelled here, so they are carried through untouched
    // rather than dropped.
    let (trailing_offsets, trailing_bytes) = if file.sections.len() > 4 {
        let start = file.sections[4] as usize;
        (
            file.sections[4..].to_vec(),
            bytes.get(start..).unwrap_or_default().to_vec(),
        )
    } else {
        (Vec::new(), Vec::new())
    };

    world.preserved = Some(PreservedWorld {
        version: file.version,
        revision: file.revision,
        favorite: file.favorite,
        header_bytes,
        time_offset: offsets.time,
        day_time_offset: offsets.day_time,
        moon_phase_offset: offsets.moon_phase,
        trailing_offsets,
        trailing_bytes,
        importance: file.importance,
    });

    debug!(
        version = file.version,
        width = world.width(),
        height = world.height(),
        chests = world.chests.len(),
        signs = world.signs.len(),
        "loaded world file"
    );
    Ok(world)
}

struct FileHeader {
    version: i32,
    revision: u32,
    favorite: u64,
    sections: Vec<i32>,
    /// One flag per tile type: whether it stores frame coordinates.
    importance: Vec<bool>,
}

fn read_file_header(r: &mut PacketReader<'_>, len: usize) -> Result<FileHeader> {
    let version = num(r.i32(), r)?;
    if version < MIN_VERSION {
        return Err(WldError::TooOld { found: version });
    }

    let magic = num(r.bytes(7), r)?;
    if magic != MAGIC {
        return Err(WldError::BadMagic {
            found: magic.to_vec(),
        });
    }
    let file_type = num(r.u8(), r)?;
    if file_type != FILE_TYPE_WORLD {
        return Err(WldError::NotAWorld { found: file_type });
    }
    let revision = num(r.u32(), r)?;
    let favorite = num(r.u64(), r)?;

    let section_count = num(r.i16(), r)?;
    if section_count < 4 {
        return Err(WldError::BadSectionPointer {
            index: 0,
            pointer: i64::from(section_count),
            len,
        });
    }
    let mut sections = Vec::with_capacity(section_count as usize);
    for index in 0..section_count as usize {
        let pointer = num(r.i32(), r)?;
        if pointer < 0 || pointer as usize > len {
            return Err(WldError::BadSectionPointer {
                index,
                pointer: i64::from(pointer),
                len,
            });
        }
        sections.push(pointer);
    }

    // The importance table is a bitset packed least significant bit first: the writer starts its
    // mask at 0x80 as a sentinel so the first entry pulls a byte and uses bit 0, then walks
    // 1, 2, 4 ... 0x80 before pulling the next.
    let mask_count = num(r.u16(), r)? as usize;
    let mut importance = Vec::with_capacity(mask_count);
    let mut current = 0u8;
    let mut bit = 0x80u8;
    for _ in 0..mask_count {
        if bit == 0x80 {
            current = num(r.u8(), r)?;
            bit = 1;
        } else {
            bit <<= 1;
        }
        importance.push(current & bit != 0);
    }

    Ok(FileHeader {
        version,
        revision,
        favorite,
        sections,
        importance,
    })
}

/// Offsets of the mutable clock fields, relative to the start of the header section.
struct HeaderOffsets {
    time: usize,
    day_time: usize,
    moon_phase: usize,
}

fn read_world_header(
    r: &mut PacketReader<'_>,
    version: i32,
    section_start: usize,
) -> Result<(World, HeaderOffsets)> {
    let name = num(r.string(), r)?;
    let _seed = num(r.string(), r)?;
    let world_gen_version = num(r.u64(), r)?;
    let mut unique_id = [0u8; 16];
    unique_id.copy_from_slice(num(r.bytes(16), r)?);
    let id = num(r.i32(), r)?;

    // The world rectangle in pixel coordinates; the tile dimensions follow it.
    for _ in 0..4 {
        num(r.i32(), r)?;
    }
    let height = num(r.i32(), r)?;
    let width = num(r.i32(), r)?;
    if !(10..=i32::from(i16::MAX)).contains(&width) || !(10..=i32::from(i16::MAX)).contains(&height)
    {
        return Err(WldError::BadDimensions { width, height });
    }

    // World flags, each gated on the version that introduced it.
    let game_mode = num(r.i32(), r)?;
    for gate in [222, 227, 238, 239, 241, 249, 266, 267, 302] {
        if version >= gate {
            num(r.bool(), r)?;
        }
    }
    num(r.i64(), r)?; // creation time
    if version >= 284 {
        num(r.i64(), r)?; // last played
    }

    let moon_type = num(r.u8(), r)?;
    let mut tree_x = [0i32; 3];
    for slot in &mut tree_x {
        *slot = num(r.i32(), r)?;
    }
    let mut tree_style = [0u8; 4];
    for slot in &mut tree_style {
        *slot = num(r.i32(), r)? as u8;
    }
    let mut cave_back_x = [0i32; 3];
    for slot in &mut cave_back_x {
        *slot = num(r.i32(), r)?;
    }
    let mut cave_back_style = [0u8; 4];
    for slot in &mut cave_back_style {
        *slot = num(r.i32(), r)? as u8;
    }
    let ice_back_style = num(r.i32(), r)? as u8;
    let jungle_back_style = num(r.i32(), r)? as u8;
    let hell_back_style = num(r.i32(), r)? as u8;

    let spawn_x = num(r.i32(), r)?;
    let spawn_y = num(r.i32(), r)?;
    let surface = num(r.f64(), r)?;
    let rock_layer = num(r.f64(), r)?;

    let offsets = HeaderOffsets {
        time: r.position() - section_start,
        day_time: r.position() - section_start + 8,
        moon_phase: r.position() - section_start + 9,
    };
    let time = num(r.f64(), r)?;
    let day_time = num(r.bool(), r)?;
    let moon_phase = num(r.i32(), r)?;
    let _blood_moon = num(r.bool(), r)?;
    let _eclipse = num(r.bool(), r)?;
    let _dungeon_x = num(r.i32(), r)?;
    let _dungeon_y = num(r.i32(), r)?;
    let crimson = num(r.bool(), r)?;

    // What the world has already been through. These have to be read in file order, and they are
    // read rather than skipped because routines, spawn pools and shops all ask about them.
    let mut progress = Progress::default();
    for flag in [
        &mut progress.downed_boss1,
        &mut progress.downed_boss2,
        &mut progress.downed_boss3,
        &mut progress.downed_queen_bee,
        &mut progress.downed_mech1,
        &mut progress.downed_mech2,
        &mut progress.downed_mech3,
        &mut progress.downed_mech_any,
        &mut progress.downed_plantera,
        &mut progress.downed_golem,
        &mut progress.downed_king_slime,
        &mut progress.saved_goblin,
        &mut progress.saved_wizard,
        &mut progress.saved_mechanic,
        &mut progress.downed_goblins,
        &mut progress.downed_clown,
        &mut progress.downed_frost,
        &mut progress.downed_pirates,
        &mut progress.shadow_orb_smashed,
        &mut progress.spawn_meteor,
    ] {
        *flag = num(r.bool(), r)?;
    }
    progress.shadow_orb_count = num(r.u8(), r)?;
    progress.altar_count = num(r.i32(), r)?;
    let hard_mode_at = r.position();
    progress.hard_mode = num(r.bool(), r)?;
    // Reading a little further is how the offset above is checked: if `hardMode` were even one
    // byte out, these would not decode as an invasion.
    let after_party = num(r.bool(), r)?;
    let invasion_delay = num(r.i32(), r)?;
    let invasion_size = num(r.i32(), r)?;
    let invasion_type = num(r.i32(), r)?;
    let invasion_x = num(r.f64(), r)?;
    debug!(
        hard_mode = progress.hard_mode,
        altars = progress.altar_count,
        orbs = progress.shadow_orb_count,
        after_party,
        invasion_delay,
        invasion_size,
        invasion_type,
        invasion_x,
        hard_mode_at,
        "world progression"
    );
    if !(0..=4).contains(&invasion_type) || !(0..=200_000).contains(&invasion_size) {
        return Err(WldError::ProgressionOutOfStep {
            invasion_type,
            invasion_size,
        });
    }
    // Everything past this point is skipped: the tile section pointer takes us straight to the
    // next section regardless of how long the rest of the header is.

    let mut world = World::empty(width, height, name);
    world.id = id;
    world.unique_id = unique_id;
    world.world_gen_version = world_gen_version;
    world.game_mode = game_mode.clamp(0, 3) as u8;
    world.spawn_x = spawn_x.clamp(0, width - 1) as i16;
    world.spawn_y = spawn_y.clamp(0, height - 1) as i16;
    world.surface = (surface as i32).clamp(0, height - 1) as i16;
    world.rock_layer = (rock_layer as i32).clamp(0, height - 1) as i16;
    world.time = time as i32;
    world.day_time = day_time;
    world.moon_phase = (moon_phase.rem_euclid(8)) as u8;
    world.crimson = crimson;
    world.progress = progress;
    world.moon_type = moon_type;
    world.tree_x = tree_x;
    world.tree_style = tree_style;
    world.cave_back_x = cave_back_x;
    world.cave_back_style = cave_back_style;
    world.ice_back_style = ice_back_style;
    world.jungle_back_style = jungle_back_style;
    world.hell_back_style = hell_back_style;
    Ok((world, offsets))
}

fn read_tiles(r: &mut PacketReader<'_>, world: &mut World, importance: &[bool]) -> Result<()> {
    // The file's own table wins over ours: a save written by another build may disagree, and the
    // table is what decides whether frame bytes are present.
    let framed = |tile: u16| importance.get(usize::from(tile)).copied().unwrap_or(false);

    let (width, height) = (world.width(), world.height());
    let expected = (width as usize) * (height as usize);
    let mut decoded = 0usize;

    // Column-major, unlike the row-major network sections.
    for x in 0..width {
        let mut y = 0i32;
        while y < height {
            let offset = r.position();
            let (tile, run) =
                read_tile_with(r, &framed).map_err(|source| WldError::Decode { offset, source })?;

            let count = i32::from(run) + 1;
            if y + count > height {
                return Err(WldError::TruncatedTiles {
                    decoded: decoded + count as usize,
                    expected,
                });
            }
            for _ in 0..count {
                world.set_tile(x, y, tile);
                y += 1;
                decoded += 1;
            }
        }
    }

    if decoded != expected {
        return Err(WldError::TruncatedTiles { decoded, expected });
    }
    Ok(())
}

fn read_chests(r: &mut PacketReader<'_>, version: i32) -> Result<Vec<Option<Chest>>> {
    let count = num(r.i16(), r)?;
    // Before 294 every chest had the same capacity; since then each carries its own.
    let shared_slots = if version < 294 {
        num(r.i16(), r)? as i32
    } else {
        0
    };

    let mut chests = Vec::with_capacity(count.max(0) as usize);
    for _ in 0..count.max(0) {
        let x = num(r.i32(), r)?;
        let y = num(r.i32(), r)?;
        let name = num(r.string(), r)?;
        let slots = if version >= 294 {
            num(r.i32(), r)?
        } else {
            shared_slots
        };

        let mut items = Vec::with_capacity(slots.clamp(0, 1000) as usize);
        for _ in 0..slots.max(0) {
            items.push(num(ItemStack::read_save(r), r)?);
        }

        chests.push(Some(Chest {
            x: x as i16,
            y: y as i16,
            name,
            items,
        }));
    }
    Ok(chests)
}

fn read_signs(r: &mut PacketReader<'_>) -> Result<Vec<Option<Sign>>> {
    let count = num(r.i16(), r)?;
    let mut signs = Vec::with_capacity(count.max(0) as usize);
    for _ in 0..count.max(0) {
        let text = num(r.string(), r)?;
        let x = num(r.i32(), r)?;
        let y = num(r.i32(), r)?;
        signs.push(Some(Sign {
            x: x as i16,
            y: y as i16,
            text,
        }));
    }
    Ok(signs)
}

/// Jump to a section pointer, checking it lies inside the file.
fn seek<'a>(r: &mut PacketReader<'a>, bytes: &'a [u8], pointer: i32, index: usize) -> Result<()> {
    if pointer < 0 || pointer as usize > bytes.len() {
        return Err(WldError::BadSectionPointer {
            index,
            pointer: i64::from(pointer),
            len: bytes.len(),
        });
    }
    *r = PacketReader::new(bytes);
    r.bytes(pointer as usize)
        .map_err(|source| WldError::Decode { offset: 0, source })?;
    Ok(())
}

/// Attach the current offset to a decode error.
fn num<T>(
    value: std::result::Result<T, terrustia_proto::ProtoError>,
    r: &PacketReader<'_>,
) -> Result<T> {
    value.map_err(|source| WldError::Decode {
        offset: r.position(),
        source,
    })
}
