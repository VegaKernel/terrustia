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
        "the late header stopped making sense at byte {at}: a count of {count} where a small \
         list was expected, so the reader is no longer where it thinks it is"
    )]
    LateHeaderOutOfStep { at: usize, count: i64 },

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

    let mut chest_slots = None;
    if file.sections.len() > 2 {
        seek(&mut r, bytes, file.sections[2], 2)?;
        world.chests = read_chests(&mut r, file.version, &mut chest_slots)?;
    }
    if file.sections.len() > 3 {
        seek(&mut r, bytes, file.sections[3], 3)?;
        world.signs = read_signs(&mut r)?;
    }

    // Sections 4 onwards hold townsfolk, tile entities, pressure plates, the town manager, the
    // bestiary and creative powers. Each is sliced out on its own so the one this server models —
    // the tile entities — can be rewritten from its own state while the rest pass through.
    let mut trailing_sections = Vec::new();
    if file.sections.len() > 4 {
        for (nth, &start) in file.sections[4..].iter().enumerate() {
            // Each section runs to the start of the next; the last runs to the end of the file,
            // taking the footer with it.
            let end = file
                .sections
                .get(5 + nth)
                .map_or(bytes.len(), |&next| next as usize);
            let (start, end) = (start as usize, end.min(bytes.len()));
            trailing_sections.push(
                bytes
                    .get(start..end.max(start))
                    .unwrap_or_default()
                    .to_vec(),
            );
        }
    }

    // Section 5 is the tile entities: pylons, item frames, mannequins, logic sensors. Read rather
    // than carried, because a pylon a client cannot be told about is a pylon nobody can use, and
    // because carrying them through means a pylon placed on this server is lost on the next save.
    if let Some(section) = trailing_sections.get(1) {
        let mut r = PacketReader::new(section);
        world.tile_entities = read_tile_entities(&mut r).unwrap_or_default();
        world.next_tile_entity = world
            .tile_entities
            .iter()
            .map(|e| e.id + 1)
            .max()
            .unwrap_or(0);
    }

    world.preserved = Some(PreservedWorld {
        version: file.version,
        chest_slots,
        revision: file.revision,
        favorite: file.favorite,
        header_bytes,
        time_offset: offsets.time,
        day_time_offset: offsets.day_time,
        moon_phase_offset: offsets.moon_phase,
        progress_offset: offsets.progress,
        hard_mode_offset: offsets.hard_mode,
        altar_offset: offsets.altar,
        orb_count_offset: offsets.orb_count,
        downed_run_offset: offsets.late.downed_run,
        tower_run_offset: offsets.late.tower_run,
        rain_offset: offsets.late.rain,
        wind_offset: offsets.late.wind,
        sandstorm_offset: offsets.late.sandstorm,
        army_run_offset: offsets.late.army_run,
        combat_book_offset: offsets.late.combat_book,
        late_downed_run_offset: offsets.late.late_downed_run,
        combat_book_two_offset: offsets.late.combat_book_two,
        trailing_sections,
        importance: file.importance,
    });

    if let Some(manifest) = world
        .preserved
        .as_ref()
        .and_then(|p| crate::world::worldgen::manifest::Manifest::from_header(&p.header_bytes))
    {
        debug!(
            passes = manifest.passes.len(),
            version = manifest.version.as_deref().unwrap_or("?"),
            "world carries a generation manifest"
        );
    }

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
    /// The run of twenty booleans beginning at `downedBoss1`.
    progress: Option<usize>,
    hard_mode: Option<usize>,
    altar: Option<usize>,
    orb_count: Option<usize>,
    late: LateOffsets,
}

/// Where in the header the flags past the invasion block live.
///
/// They are recorded rather than re-derived because saving preserves the header verbatim and
/// patches it in place: writing a flag means knowing the byte, and the byte is only knowable by
/// having walked there.
#[derive(Debug, Default, Clone, Copy)]
pub struct LateOffsets {
    /// The run of nine "downed" booleans beginning at `downedFishron`.
    pub downed_run: Option<usize>,
    /// The run of nine pillar flags: four beaten, four standing, and the apocalypse itself.
    pub tower_run: Option<usize>,
    /// The weather block: raining, how long for, and how hard.
    pub rain: Option<usize>,
    /// The wind the world is blowing toward.
    pub wind: Option<usize>,
    /// The sandstorm block: happening, how long for, and its two severities.
    pub sandstorm: Option<usize>,
    /// The bartender, then the three Old One's Army tiers.
    pub army_run: Option<usize>,
    /// The first combat book, which sits alone between two blocks that are not flags.
    pub combat_book: Option<usize>,
    /// The Empress of Light, Queen Slime and Deerclops, in that order.
    pub late_downed_run: Option<usize>,
    /// The second combat book, after the run of unlocked town-NPC spawns.
    pub combat_book_two: Option<usize>,
}

/// What the late header says about the weather.
#[derive(Debug, Default, Clone, Copy)]
struct Weather {
    raining: bool,
    rain_time: i32,
    max_rain: f32,
    wind: f32,
    sandstorm: bool,
    sandstorm_time: i32,
    severity: f32,
    intended_severity: f32,
}

/// Walk the header past the invasion block, picking up the flags the server actually uses.
///
/// Everything here is positional: there is no framing, so a field read at the wrong width puts
/// every flag after it in the wrong place. The two variable-length lists in the middle are why it
/// cannot simply be seeked into.
fn read_late_header(
    r: &mut PacketReader<'_>,
    version: i32,
    progress: &mut Progress,
    weather: &mut Weather,
    offsets: &mut LateOffsets,
    section_start: usize,
) -> Result<()> {
    let _slime_rain_time = num(r.f64(), r)?;
    let _sundial_cooldown = num(r.u8(), r)?;

    offsets.rain = Some(r.position() - section_start);
    weather.raining = num(r.bool(), r)?;
    weather.rain_time = num(r.i32(), r)?;
    weather.max_rain = num(r.f32(), r)?;

    // The hardmode ore tiers the world rolled when the wall fell.
    for _ in 0..3 {
        num(r.i32(), r)?;
    }
    // Eight background styles.
    for _ in 0..8 {
        num(r.u8(), r)?;
    }
    let _cloud_bg_active = num(r.i32(), r)?;
    let _num_clouds = num(r.i16(), r)?;
    offsets.wind = Some(r.position() - section_start);
    weather.wind = num(r.f32(), r)?;

    // Who has already handed in an angler quest today: a list of names.
    let anglers = num(r.i32(), r)?;
    if !(0..=255).contains(&anglers) {
        return Err(WldError::LateHeaderOutOfStep {
            at: r.position(),
            count: i64::from(anglers),
        });
    }
    for _ in 0..anglers {
        num(r.string(), r)?;
    }
    progress.saved_angler = num(r.bool(), r)?;
    let _angler_quest = num(r.i32(), r)?;
    progress.saved_stylist = num(r.bool(), r)?;
    progress.saved_tax_collector = num(r.bool(), r)?;
    progress.saved_golfer = num(r.bool(), r)?;
    let _invasion_size_start = num(r.i32(), r)?;
    let _cultist_delay = num(r.i32(), r)?;

    // The banner kill counts, then — only from 289 — the claimable banners.
    //
    // The second list is the one version gate in this whole run that is easy to miss, because a
    // world that predates it usually has nothing after the kill counts that looks wrong: the two
    // bytes read for a count that is not there come back as zero, no items follow, and every flag
    // from here to the end of the header is silently two bytes out. That misplaces the Moon Lord,
    // the cultist and the four pillars on any world older than 1.4.4.9.
    let kinds = num(r.i16(), r)?;
    if !(0..=10_000).contains(&kinds) {
        return Err(WldError::LateHeaderOutOfStep {
            at: r.position(),
            count: i64::from(kinds),
        });
    }
    for _ in 0..kinds {
        num(r.i32(), r)?;
    }
    if version >= 289 {
        let claimable = num(r.i16(), r)?;
        if !(0..=10_000).contains(&claimable) {
            return Err(WldError::LateHeaderOutOfStep {
                at: r.position(),
                count: i64::from(claimable),
            });
        }
        for _ in 0..claimable {
            num(r.u16(), r)?;
        }
    }

    let _fast_forward_to_dawn = num(r.bool(), r)?;
    offsets.downed_run = Some(r.position() - section_start);
    for flag in [
        &mut progress.downed_fishron,
        &mut progress.downed_martians,
        &mut progress.downed_ancient_cultist,
        &mut progress.downed_moon_lord,
        &mut progress.downed_halloween_king,
        &mut progress.downed_halloween_tree,
        &mut progress.downed_christmas_ice_queen,
        &mut progress.downed_christmas_santank,
        &mut progress.downed_christmas_tree,
    ] {
        *flag = num(r.bool(), r)?;
    }
    offsets.tower_run = Some(r.position() - section_start);
    for flag in [
        &mut progress.downed_tower_solar,
        &mut progress.downed_tower_vortex,
        &mut progress.downed_tower_nebula,
        &mut progress.downed_tower_stardust,
        &mut progress.tower_active_solar,
        &mut progress.tower_active_vortex,
        &mut progress.tower_active_nebula,
        &mut progress.tower_active_stardust,
        &mut progress.lunar_apocalypse_up,
    ] {
        *flag = num(r.bool(), r)?;
    }

    // A party in progress, and who is celebrating.
    let _party_manual = num(r.bool(), r)?;
    let _party_genuine = num(r.bool(), r)?;
    let _party_cooldown = num(r.i32(), r)?;
    let partiers = num(r.i32(), r)?;
    if !(0..=1_000).contains(&partiers) {
        return Err(WldError::LateHeaderOutOfStep {
            at: r.position(),
            count: i64::from(partiers),
        });
    }
    for _ in 0..partiers {
        num(r.i32(), r)?;
    }

    offsets.sandstorm = Some(r.position() - section_start);
    weather.sandstorm = num(r.bool(), r)?;
    weather.sandstorm_time = num(r.i32(), r)?;
    weather.severity = num(r.f32(), r)?;
    weather.intended_severity = num(r.f32(), r)?;

    // The Old One's Army: the bartender who starts it, then the three tiers it has lost.
    offsets.army_run = Some(r.position() - section_start);
    for flag in [
        &mut progress.saved_bartender,
        &mut progress.downed_army_t1,
        &mut progress.downed_army_t2,
        &mut progress.downed_army_t3,
    ] {
        *flag = num(r.bool(), r)?;
    }

    // Five more background styles.
    for _ in 0..5 {
        num(r.u8(), r)?;
    }
    offsets.combat_book = Some(r.position() - section_start);
    progress.combat_book = num(r.bool(), r)?;

    // Lantern night: its cooldown, then three flags about the night to come.
    num(r.i32(), r)?;
    for _ in 0..3 {
        num(r.bool(), r)?;
    }

    // One tree-top variation per biome, counted rather than fixed.
    let tree_tops = num(r.i32(), r)?;
    if !(0..=1_000).contains(&tree_tops) {
        return Err(WldError::LateHeaderOutOfStep {
            at: r.position(),
            count: i64::from(tree_tops),
        });
    }
    for _ in 0..tree_tops {
        num(r.i32(), r)?;
    }

    // Forced holidays for today, the four ore tiers the wall handed out, and the three pets.
    for _ in 0..2 {
        num(r.bool(), r)?;
    }
    for _ in 0..4 {
        num(r.i32(), r)?;
    }
    for _ in 0..3 {
        num(r.bool(), r)?;
    }

    offsets.late_downed_run = Some(r.position() - section_start);
    for flag in [
        &mut progress.downed_empress_of_light,
        &mut progress.downed_queen_slime,
        &mut progress.downed_deerclops,
    ] {
        *flag = num(r.bool(), r)?;
    }

    // Nine town NPCs whose arrival has been unlocked by other means, then the second book.
    for _ in 0..9 {
        num(r.bool(), r)?;
    }
    offsets.combat_book_two = Some(r.position() - section_start);
    progress.combat_book_two = num(r.bool(), r)?;

    Ok(())
}

fn read_world_header(
    r: &mut PacketReader<'_>,
    version: i32,
    section_start: usize,
) -> Result<(World, HeaderOffsets)> {
    let name = num(r.string(), r)?;
    let seed_text = num(r.string(), r)?;
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

    let mut offsets = HeaderOffsets {
        time: r.position() - section_start,
        day_time: r.position() - section_start + 8,
        moon_phase: r.position() - section_start + 9,
        progress: None,
        hard_mode: None,
        altar: None,
        orb_count: None,
        late: LateOffsets::default(),
    };
    let time = num(r.f64(), r)?;
    let day_time = num(r.bool(), r)?;
    let moon_phase = num(r.i32(), r)?;
    let _blood_moon = num(r.bool(), r)?;
    let _eclipse = num(r.bool(), r)?;
    let dungeon_x = num(r.i32(), r)?;
    let dungeon_y = num(r.i32(), r)?;
    let crimson = num(r.bool(), r)?;

    // What the world has already been through. These have to be read in file order, and they are
    // read rather than skipped because routines, spawn pools and shops all ask about them.
    let mut progress = Progress::default();
    let mut world_weather = Weather::default();
    offsets.progress = Some(r.position() - section_start);
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
    offsets.orb_count = Some(r.position() - section_start);
    progress.shadow_orb_count = num(r.u8(), r)?;
    offsets.altar = Some(r.position() - section_start);
    progress.altar_count = num(r.i32(), r)?;
    let hard_mode_at = r.position();
    offsets.hard_mode = Some(hard_mode_at - section_start);
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
    // The rest of the header is read for the flags that matter and skipped for the rest. It has
    // to be walked rather than seeked because two of the runs are variable-length lists.
    let mut late = LateOffsets::default();
    if let Err(error) = read_late_header(
        r,
        version,
        &mut progress,
        &mut world_weather,
        &mut late,
        section_start,
    ) {
        // A header that runs out is not fatal: the tile pointer is what actually finds the tiles,
        // and everything read up to here is already good. It only means the late flags stay at
        // their defaults, which is what an older world would have anyway.
        debug!(?error, "world header ended before the late flags");
    }

    // Everything past this point is skipped: the tile section pointer takes us straight to the
    // next section regardless of how long the rest of the header is.

    offsets.late = late;
    let mut world = World::empty(width, height, name);
    world.dungeon_x = Some(dungeon_x);
    world.dungeon_y = Some(dungeon_y);
    world.raining = world_weather.raining;
    world.rain_time = world_weather.rain_time;
    world.max_rain = world_weather.max_rain;
    world.wind = world_weather.wind;
    world.sandstorm = world_weather.sandstorm;
    world.sandstorm_time = world_weather.sandstorm_time;
    world.sandstorm_severity = world_weather.severity;
    world.sandstorm_intended_severity = world_weather.intended_severity;
    world.id = id;
    world.unique_id = unique_id;
    world.world_gen_version = world_gen_version;
    world.seed_text = seed_text;
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

fn read_chests(
    r: &mut PacketReader<'_>,
    version: i32,
    shared: &mut Option<i16>,
) -> Result<Vec<Option<Chest>>> {
    let count = num(r.i16(), r)?;
    // Before 294 every chest had the same capacity; since then each carries its own. Which it was
    // has to be remembered, because saving writes the section back in this file's own shape.
    let shared_slots = if version < 294 {
        let slots = num(r.i16(), r)?;
        *shared = Some(slots);
        i32::from(slots)
    } else {
        *shared = None;
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

/// Read section 5: the furniture that remembers something.
///
/// A count, then each entity in its file form — with its id, and with a logic sensor's state,
/// neither of which the network form carries.
///
/// A truncated or unrecognised section gives up rather than failing the whole load. It is the
/// difference between "this world has an item frame this build does not know about" and "this
/// world will not open", and the first is much the better answer.
fn read_tile_entities(
    r: &mut PacketReader<'_>,
) -> Result<Vec<terrustia_proto::tile_entity::TileEntity>> {
    let count = num(r.i32(), r)?;
    let mut entities = Vec::with_capacity(count.clamp(0, 1 << 16) as usize);
    for _ in 0..count.max(0) {
        match terrustia_proto::tile_entity::TileEntity::read(r, false) {
            Ok(entity) => entities.push(entity),
            Err(_) => break,
        }
    }
    Ok(entities)
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

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Writer;

    /// The whole late header, as a world of `version` writes it.
    ///
    /// Only the fields the reader distinguishes are given real values; the rest are the right
    /// widths filled with zero, which is what makes an off-by-two visible rather than plausible.
    fn late_tail(version: i32) -> Vec<u8> {
        let mut w = Writer::new();
        // The slime rain clock and the sundial, then the rain the reader keeps.
        w.f64(0.0).u8(0);
        w.bool(true).i32(600).f32(0.9);
        // Three hardmode ore tiers, eight background styles, the clouds, then the wind.
        for _ in 0..3 {
            w.i32(0);
        }
        for _ in 0..8 {
            w.u8(0);
        }
        w.i32(0).i16(0).f32(-0.3);
        // Nobody has handed in an angler quest, and the five saved townsfolk.
        w.i32(0);
        w.bool(false).i32(0).bool(false).bool(false).bool(false);
        w.i32(0).i32(0);
        // Banner kill counts, then the claimable list that only exists from 289.
        w.i16(2).i32(7).i32(9);
        if version >= 289 {
            w.i16(1).u16(1234);
        }
        w.bool(false); // fast forward to dawn
        // The nine "downed" flags: only the Moon Lord, the fourth, is set.
        for i in 0..9 {
            w.bool(i == 3);
        }
        // The nine pillar flags: only the vortex tower is standing, the sixth.
        for i in 0..9 {
            w.bool(i == 5);
        }
        // A party nobody is at.
        w.bool(false).bool(false).i32(0).i32(0);
        // The sandstorm, which is the first thing after the party with a recognisable value.
        w.bool(true).i32(4321).f32(0.25).f32(0.5);
        // The bartender and the three army tiers.
        w.bool(true).bool(true).bool(false).bool(false);
        // Five background styles, then the first combat book.
        for _ in 0..5 {
            w.u8(0);
        }
        w.bool(true);
        // Lantern night, then thirteen tree tops.
        w.i32(0).bool(false).bool(false).bool(false);
        w.i32(13);
        for _ in 0..13 {
            w.i32(1);
        }
        // Forced holidays, the four ore tiers, the three pets.
        w.bool(false).bool(false);
        for tier in [7, 167, 9, 169] {
            w.i32(tier);
        }
        for _ in 0..3 {
            w.bool(false);
        }
        // The empress, the queen and Deerclops: only the queen is down.
        w.bool(false).bool(true).bool(false);
        // Nine unlocked town spawns, then the second combat book.
        for _ in 0..9 {
            w.bool(false);
        }
        w.bool(false);
        w.into_bytes()
    }

    fn walk(version: i32) -> (Progress, Weather) {
        let bytes = late_tail(version);
        let mut r = PacketReader::new(&bytes);
        let mut progress = Progress::default();
        let mut weather = Weather::default();
        let mut offsets = LateOffsets::default();
        read_late_header(
            &mut r,
            version,
            &mut progress,
            &mut weather,
            &mut offsets,
            0,
        )
        .unwrap_or_else(|e| panic!("version {version}: {e}"));
        (progress, weather)
    }

    /// A world older than 289 has no claimable-banner list, and reading one anyway puts every
    /// flag after it two bytes out.
    ///
    /// The two bytes come back as a count of zero, so nothing fails loudly: the world simply
    /// reports the wrong bosses. Both versions have to land on the same answers.
    #[test]
    fn the_claimable_banner_list_is_gated_on_its_version() {
        for version in [MIN_VERSION, 288, 289, 319] {
            let (p, weather) = walk(version);
            assert!(p.downed_moon_lord, "{version}: the moon lord");
            assert!(!p.downed_fishron, "{version}: fishron, which is not down");
            assert!(
                p.tower_active_vortex,
                "{version}: the vortex tower standing"
            );
            assert!(!p.downed_tower_solar, "{version}: solar, never beaten");
            assert!(
                weather.raining && weather.rain_time == 600,
                "{version}: rain"
            );
            assert_eq!(weather.wind, -0.3, "{version}: wind");
            assert!(weather.sandstorm, "{version}: the sandstorm");
            assert_eq!(weather.sandstorm_time, 4321, "{version}");
            assert_eq!(weather.severity, 0.25, "{version}");
            assert!(p.saved_bartender, "{version}: the bartender");
            assert!(p.downed_army_t1 && !p.downed_army_t2, "{version}: the army");
            assert!(p.combat_book && !p.combat_book_two, "{version}: the books");
            assert!(p.downed_queen_slime, "{version}: the queen");
            assert!(!p.downed_empress_of_light, "{version}: the empress");
            assert!(!p.downed_deerclops, "{version}: deerclops");
        }
    }
}
