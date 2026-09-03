use terrustia_proto::{
    SectionBounds, SectionExtras, Tile,
    packets::{WorldData, WorldFlags},
    section::{SECTION_HEIGHT, SECTION_WIDTH},
};

use super::objects::{Chest, MAX_CHESTS, MAX_SIGNS, PreservedWorld, Sign};
use super::progress::Progress;
use super::worldgen::secret_seed::SecretSeeds;

/// Ticks of daylight, then of night, matching the vanilla clock.
pub const DAY_LENGTH: i32 = 54_000;
pub const NIGHT_LENGTH: i32 = 32_400;

/// A flag per section of the world.
///
/// Deliberately a flat array rather than a set. A world has a few hundred sections at most, so
/// this is under a kilobyte and every operation is an index — where the `HashSet<(i32, i32)>` it
/// replaced hashed a coordinate pair on every single tile write, overwhelmingly to re-mark a
/// section that was already marked.
#[derive(Clone, Default)]
struct SectionFlags {
    /// Row-major, `wide` sections across.
    flags: Vec<bool>,
    wide: usize,
    /// How many are set, so "did anything change" costs nothing.
    marked: usize,
}

impl SectionFlags {
    fn new(wide: usize, tall: usize) -> Self {
        Self {
            flags: vec![false; wide * tall],
            wide,
            marked: 0,
        }
    }

    fn mark(&mut self, section_x: i32, section_y: i32) {
        let at = section_y as usize * self.wide + section_x as usize;
        if let Some(flag) = self.flags.get_mut(at)
            && !*flag
        {
            *flag = true;
            self.marked += 1;
        }
    }

    /// The marked sections, clearing them as it goes.
    fn drain(&mut self) -> Vec<(i32, i32)> {
        let mut out = Vec::with_capacity(self.marked);
        if self.marked > 0 {
            for (at, flag) in self.flags.iter_mut().enumerate() {
                if *flag {
                    *flag = false;
                    out.push(((at % self.wide) as i32, (at / self.wide) as i32));
                }
            }
            self.marked = 0;
        }
        out
    }

    /// The same walk, stopping once `cap` sections have been cleared.
    ///
    /// For spreading a snapshot refresh over several ticks instead of paying for it in one. The
    /// order is whatever the flat array happens to be in, which is fine: the caller cares only
    /// that every marked section is eventually handed over, not which comes first.
    fn drain_upto(&mut self, cap: usize) -> Vec<(i32, i32)> {
        let want = cap.min(self.marked);
        let mut out = Vec::with_capacity(want);
        if want > 0 {
            for (at, flag) in self.flags.iter_mut().enumerate() {
                if *flag {
                    *flag = false;
                    out.push(((at % self.wide) as i32, (at / self.wide) as i32));
                    if out.len() == want {
                        break;
                    }
                }
            }
            self.marked -= out.len();
        }
        out
    }

    fn clear(&mut self) {
        if self.marked > 0 {
            self.flags.fill(false);
            self.marked = 0;
        }
    }
}

/// The authoritative world.
///
/// Owned exclusively by the game task, so no interior mutability or locking is needed.
///
/// `Clone` exists for exactly one caller — [`World::snapshot`] — and is not cheap: a large world
/// is sixty megabytes of tiles. Use the named method rather than `.clone()`, so that a copy this
/// size never happens by accident.
#[derive(Clone)]
pub struct World {
    width: i32,
    height: i32,
    tiles: super::packed::TileStore,
    pub spawn_x: i16,
    pub spawn_y: i16,
    pub surface: i16,
    pub rock_layer: i16,
    pub name: String,
    pub id: i32,
    pub unique_id: [u8; 16],
    pub time: i32,
    pub day_time: bool,
    /// A blood moon changes behaviour, not just lighting: a polite door-opener like a zombie, which
    /// cannot get through a closed door on an ordinary night, forces it open on a blood moon (the
    /// door survives — only a Goblin Peon smashes it), and the night spawn pool widens.
    pub blood_moon: bool,
    /// A solar eclipse: a daytime event, and the only time Mothron and its brood appear.
    pub eclipse: bool,
    pub moon_phase: u8,
    /// `Main.halloween` and `Main.xMas`: the real-world calendar, not world state.
    ///
    /// Neither is saved, seeded or sent: the game recomputes both from the wall clock every dawn
    /// (`Main.cs:66375-66376`) and a client does the same for itself on receiving world data
    /// (`MessageBuffer.cs:660-661`). They are held here because that is what `Main` does with them
    /// and because the ambient spawner reads them alongside every other world flag.
    ///
    /// Deliberately left `false` by every constructor rather than read from the clock there:
    /// [`Self::refresh_calendar`] is called from the server's own boot and dawn, so a test that
    /// builds a `World` gets the same answer in October as in June. See
    /// [`crate::world::calendar`] for the date rules and the one disclosed narrowing (UTC).
    pub halloween: bool,
    pub xmas: bool,
    /// Whether it is raining, how long the shower has left, and how hard it is coming down.
    ///
    /// Rain is not only weather: it changes which town NPCs will go outside, and several routines
    /// read it directly.
    pub raining: bool,
    pub rain_time: i32,
    pub max_rain: f32,
    /// The sandstorm, which the file records separately from the wind that raises it.
    ///
    /// Kept on the world so a save resumes the storm it was in rather than a calm desert, and so
    /// the two severities — where it is now and where it is heading — survive with it.
    pub sandstorm: bool,
    pub sandstorm_time: i32,
    pub sandstorm_severity: f32,
    pub sandstorm_intended_severity: f32,
    /// Where the dungeon is, when the world file said. Hardmode's stripes steer around it, the
    /// jungle sits opposite it, and the Old Man waits at its door.
    pub dungeon_x: Option<i32>,
    pub dungeon_y: Option<i32>,
    /// Which moon is up, if either. Kept on the world because the client is told about it.
    pub pumpkin_moon: bool,
    pub snow_moon: bool,
    /// The wind the world is blowing, signed: positive blows east.
    ///
    /// Several ported routines read it and have been reading nothing but calm until now.
    pub wind: f32,
    pub crimson: bool,
    /// Which ore each tier settled on: copper, iron, silver, gold, cobalt, mythril, adamantite,
    /// in the order the file and packet 7 both use. `-1` means that tier has not been chosen.
    ///
    /// A world picks one of a pair for each tier — copper or tin, cobalt or palladium — and the
    /// choice has to be remembered, because the world is already full of the one it picked. The
    /// hardmode three are decided by the first three altars broken, and until then they are `-1`:
    /// that sentinel is load-bearing, since `SmashAltar` only rolls a tier when it sees it, and a
    /// `0` there makes the game spray tile type 0 — dirt — instead of ore.
    pub ore_tiers: [i16; 7],
    /// What the world has already been through.
    ///
    /// These are not bookkeeping: routines read them. A wall creeper behaves one way before the
    /// wall falls and another after; a town NPC's stock, a spawn pool and half the hardmode
    /// roster all turn on what is recorded here.
    pub progress: Progress,
    pub game_mode: u8,
    pub world_gen_version: u64,
    /// The seed as it was typed, which is what the generator is started from.
    ///
    /// Kept because it is the other half of the parity oracle: a reference world names both the
    /// seed it was built from and the state of the generator after every pass.
    pub seed_text: String,
    /// Which of vanilla's real secret-seed flags this world has active, if any.
    ///
    /// Real vanilla persists these as their own bytes in the `.wld` header (`WorldFile.cs`'s own
    /// `SaveWorldFlags`/`LoadWorldFlags`) rather than re-deriving them from `seed_text` on every
    /// load — load-bearing for the two numeric-only triggers (Drunk World, one of Celebrationmk10's
    /// two alternates), which have no literal string to re-derive from, and in general because the
    /// flags a world was *actually generated with* are the ground truth, not whatever a fresh
    /// re-match against `seed_text` would produce. See `worldgen::secret_seed`'s own module doc.
    pub secret_seeds: SecretSeeds,
    pub moon_type: u8,
    pub tree_x: [i32; 3],
    pub tree_style: [u8; 4],
    pub cave_back_x: [i32; 3],
    pub cave_back_style: [u8; 4],
    pub ice_back_style: u8,
    pub jungle_back_style: u8,
    pub hell_back_style: u8,
    /// One backdrop style per biome, in packet 7's order: the four forest variants, then corrupt,
    /// jungle, snow, hallow, crimson, desert, ocean, mushroom and underworld.
    ///
    /// The world file keeps these in two separate runs and this parser used to skip both, so every
    /// loaded world served its players the biome-zero backdrop everywhere. Purely what the player
    /// sees — no routine reads them — but "the sky is wrong in every biome" is not nothing, and it
    /// was found by diffing our packet 7 against a real server's on the same world file.
    pub backgrounds: [u8; 13],
    /// One tree-top variation per biome area, likewise dropped on load until now.
    pub tree_tops: [u8; 13],
    /// How many clouds are in the sky. The file stores it as a short; the packet sends a byte.
    pub num_clouds: u8,
    /// Chests and signs are announced in the trailer of whichever section contains them, so a
    /// loaded world has to keep them alongside the tiles.
    ///
    /// These are sparse: an index is the id that travels on the wire, so removing an entry leaves
    /// a hole rather than renumbering everything after it.
    pub chests: Vec<Option<Chest>>,
    pub signs: Vec<Option<Sign>>,
    /// The townsfolk who live here, and which of them have been through shimmer.
    ///
    /// Kept on the world because the save owns them: the server's live roster is rebuilt from
    /// this at startup and written back into it before each save, so a Guide who moved into a
    /// house is still there — with the same name — after a restart.
    pub town_npcs: Vec<super::objects::TownNpc>,
    pub shimmered_town_npcs: Vec<i32>,
    /// The Lunar Pillars, as the file's own second `SaveNPCs` list remembers them: type and
    /// position only, no name or home. Rebuilt from the live roster before every save
    /// (`record_lunar_pillars`) and put back as live NPCs at startup (`restore_lunar_pillars`),
    /// the same round trip `town_npcs` gets — without it a save mid-Lunar-Apocalypse drops the
    /// pillars outright, and the next load's first tick reads their absence as every standing
    /// tower having just been beaten.
    pub saved_npcs: Vec<super::objects::SavedNpc>,
    /// The Journey (creative) mode powers real vanilla persists per world -
    /// `CreativePowerManager.SaveToWorld`'s six `IPersistentPerWorldContent` powers out of its
    /// fifteen (`CreativePowerManager.cs:90-104` for the registration order each id comes from).
    /// Mirrored from `self.journey` before every save and back again at startup, the same way
    /// `town_npcs` mirrors the live roster; see `game/journey.rs`'s own module doc for why the
    /// other nine powers have nothing to hold here.
    pub journey_freeze_time: bool,
    pub journey_freeze_rain: bool,
    pub journey_freeze_wind: bool,
    pub journey_stop_biome_spread: bool,
    /// `ModifyTimeRate`'s raw 0.0-1.0 slider position, not the derived rate.
    pub journey_time_rate_slider: f32,
    /// `DifficultySliderPower`'s raw 0.0-1.0 slider position, not the derived multiplier.
    pub journey_difficulty_slider: f32,
    /// How many of each banner's enemy have been killed, by banner index.
    ///
    /// On the world rather than the server because the save carries it: a hundred zombies killed
    /// before a restart still count towards the banner afterwards. Sparse, because most of the
    /// four hundred banners are never touched in one world.
    pub banner_kills: std::collections::HashMap<u16, u32>,
    /// The furniture that remembers something: item frames, mannequins, logic sensors, pylons.
    ///
    /// World state rather than server state, for two reasons that both bite. They are written to
    /// the world file, so a server that keeps them to itself loses every pylon on restart; and
    /// they ride the section stream, so the section builder has to be able to see them or a
    /// joining client is told about none of the ones that were already there.
    pub tile_entities: Vec<terrustia_proto::tile_entity::TileEntity>,
    /// The id the next tile entity placed will be given.
    ///
    /// Ids are handed out rather than derived from position, because a client refers to an
    /// entity by id in every message about it and the id has to survive the entity being moved
    /// out from under it.
    pub next_tile_entity: i32,
    /// Parts of the save this server does not model, kept so saving stays lossless.
    pub preserved: Option<PreservedWorld>,
    /// Sections whose tiles changed since anything last looked.
    ///
    /// Encoded sections are cached, so something has to say when one goes stale. Tracking is off
    /// during generation and loading, where every tile is written once and the whole world is new
    /// anyway; five million set inserts there would cost more than the cache saves.
    /// One flag per section, rather than a set of coordinates.
    ///
    /// This used to be a `HashSet<(i32, i32)>`, which meant SipHashing a pair of `i32`s on *every*
    /// tile write — up to tens of thousands a tick under liquid load, nearly all of them
    /// re-inserting a key that was already there. A world has only a few hundred sections
    /// (21 x 8 on a small one, 42 x 16 on a large), so a flag each is well under a kilobyte and
    /// costs one indexed byte write instead of a hash.
    dirty_sections: SectionFlags,
    /// Sections changed since the last snapshot was taken, so the next one can copy only those.
    ///
    /// Kept apart from `dirty_sections` because the two are drained on completely different
    /// rhythms: section streaming clears its set every tick, a snapshot happens every few minutes.
    changed_since_snapshot: SectionFlags,
    track_dirty: bool,
}

impl World {
    /// An empty world of the given tile dimensions.
    pub fn empty(width: i32, height: i32, name: impl Into<String>) -> Self {
        assert!(width > 0 && height > 0, "world dimensions must be positive");
        Self {
            width,
            height,
            tiles: super::packed::TileStore::new(width, height),
            spawn_x: (width / 2) as i16,
            spawn_y: (height / 3) as i16,
            surface: (height / 3) as i16,
            rock_layer: (height / 2) as i16,
            name: name.into(),
            id: 1,
            unique_id: [0; 16],
            time: 13_500,
            day_time: true,
            blood_moon: false,
            eclipse: false,
            moon_phase: 0,
            halloween: false,
            xmas: false,
            dungeon_x: None,
            dungeon_y: None,
            pumpkin_moon: false,
            snow_moon: false,
            raining: false,
            rain_time: 0,
            max_rain: 0.0,
            sandstorm: false,
            sandstorm_time: 0,
            sandstorm_severity: 0.0,
            sandstorm_intended_severity: 0.0,
            wind: 0.0,
            crimson: false,
            ore_tiers: [-1; 7],
            progress: Progress::default(),
            game_mode: 0,
            world_gen_version: 0,
            seed_text: String::new(),
            secret_seeds: SecretSeeds::none(),
            moon_type: 0,
            // Anchor the parallax layers past the right edge so no tree or cave background is
            // drawn over the playfield.
            tree_x: [width; 3],
            tree_style: [0; 4],
            cave_back_x: [width; 3],
            cave_back_style: [0; 4],
            ice_back_style: 0,
            jungle_back_style: 0,
            hell_back_style: 0,
            backgrounds: [0; 13],
            tree_tops: [0; 13],
            num_clouds: 0,
            chests: Vec::new(),
            signs: Vec::new(),
            town_npcs: Vec::new(),
            shimmered_town_npcs: Vec::new(),
            saved_npcs: Vec::new(),
            journey_freeze_time: false,
            journey_freeze_rain: false,
            journey_freeze_wind: false,
            journey_stop_biome_spread: false,
            journey_time_rate_slider: 0.0,
            journey_difficulty_slider: 0.0,
            banner_kills: std::collections::HashMap::new(),
            tile_entities: Vec::new(),
            next_tile_entity: 0,
            preserved: None,
            // Section counts truncate exactly as the client's do; +1 keeps a world whose size is
            // not an exact multiple from marking past the end.
            dirty_sections: SectionFlags::new(
                (width / SECTION_WIDTH).max(0) as usize + 1,
                (height / SECTION_HEIGHT).max(0) as usize + 1,
            ),
            changed_since_snapshot: SectionFlags::new(
                (width / SECTION_WIDTH).max(0) as usize + 1,
                (height / SECTION_HEIGHT).max(0) as usize + 1,
            ),
            track_dirty: false,
        }
    }

    /// The chest anchored exactly at a tile, if there is one.
    pub fn chest_at(&self, x: i16, y: i16) -> Option<(i16, &Chest)> {
        self.chests
            .iter()
            .enumerate()
            .find_map(|(id, slot)| match slot {
                Some(chest) if chest.x == x && chest.y == y => Some((id as i16, chest)),
                _ => None,
            })
    }

    pub fn chest(&self, id: i16) -> Option<&Chest> {
        self.chests.get(usize::try_from(id).ok()?)?.as_ref()
    }

    pub fn chest_mut(&mut self, id: i16) -> Option<&mut Chest> {
        self.chests.get_mut(usize::try_from(id).ok()?)?.as_mut()
    }

    /// Place a chest, reusing the lowest free index the way vanilla's fixed array does.
    pub fn add_chest(&mut self, chest: Chest) -> Option<i16> {
        if let Some(id) = self.chests.iter().position(Option::is_none) {
            self.chests[id] = Some(chest);
            return i16::try_from(id).ok();
        }
        if self.chests.len() >= MAX_CHESTS {
            return None;
        }
        self.chests.push(Some(chest));
        i16::try_from(self.chests.len() - 1).ok()
    }

    pub fn remove_chest(&mut self, id: i16) -> Option<Chest> {
        self.chests.get_mut(usize::try_from(id).ok()?)?.take()
    }

    /// The sign anchored at or adjacent to a tile.
    ///
    /// Signs occupy a 2x2 block and the client may report any of the four tiles, so the anchor is
    /// matched with a one-tile tolerance.
    pub fn sign_at(&self, x: i16, y: i16) -> Option<(i16, &Sign)> {
        self.signs
            .iter()
            .enumerate()
            .find_map(|(id, slot)| match slot {
                Some(sign) if (sign.x - x).abs() <= 1 && (sign.y - y).abs() <= 1 => {
                    Some((id as i16, sign))
                }
                _ => None,
            })
    }

    pub fn sign_mut(&mut self, id: i16) -> Option<&mut Sign> {
        self.signs.get_mut(usize::try_from(id).ok()?)?.as_mut()
    }

    pub fn add_sign(&mut self, sign: Sign) -> Option<i16> {
        if let Some(id) = self.signs.iter().position(Option::is_none) {
            self.signs[id] = Some(sign);
            return i16::try_from(id).ok();
        }
        if self.signs.len() >= MAX_SIGNS {
            return None;
        }
        self.signs.push(Some(sign));
        i16::try_from(self.signs.len() - 1).ok()
    }

    /// The chests and signs whose anchor tile falls inside `bounds`.
    pub fn extras_for(&self, bounds: SectionBounds) -> SectionExtras {
        let contains = |x: i16, y: i16| {
            let (x, y) = (i32::from(x), i32::from(y));
            x >= bounds.x
                && x < bounds.x + i32::from(bounds.width)
                && y >= bounds.y
                && y < bounds.y + i32::from(bounds.height)
        };
        let mut chests: Vec<_> = self
            .chests
            .iter()
            .enumerate()
            .filter_map(|(id, slot)| slot.as_ref().map(|c| (id as i16, c)))
            .filter(|(_, c)| contains(c.x, c.y))
            .map(|(id, c)| c.info(id))
            .collect();
        let mut signs: Vec<_> = self
            .signs
            .iter()
            .enumerate()
            .filter_map(|(id, slot)| slot.as_ref().map(|s| (id as i16, s)))
            .filter(|(_, s)| contains(s.x, s.y))
            .map(|(id, s)| s.info(id))
            .collect();

        // Vanilla collects these as its row-major tile walk reaches each anchor tile, so the
        // trailer is ordered by row and then column. Matching that ordering keeps our section
        // bytes identical to the game's for the same world.
        chests.sort_by_key(|c| (c.y, c.x));
        signs.sort_by_key(|s| (s.y, s.x));

        let mut tile_entities: Vec<_> = self
            .tile_entities
            .iter()
            .filter(|e| contains(e.x, e.y))
            .cloned()
            .collect();
        tile_entities.sort_by_key(|e| (e.y, e.x));

        SectionExtras {
            chests,
            signs,
            tile_entities,
        }
    }

    /// A copy of the whole world, for saving it without holding up the tick.
    ///
    /// Serialising a large world takes about fifty-five milliseconds, against a tick budget of
    /// sixteen and a half. Doing it on the game task means every autosave drops three or four
    /// ticks, which players feel as a stutter every five minutes. Doing it on another thread
    /// means having something that thread can own, and this is it.
    ///
    /// The copy is atomic with respect to the tick, so the save can never catch the world halfway
    /// through an edit. A torn save is much worse than a slow one.
    ///
    /// It costs about thirty milliseconds on a large world, which is still twice a tick, and the
    /// obvious next step — keeping a buffer between saves so its pages stay warm — was tried and
    /// **made it worse**: 33 ms, then 41, then 45 as the extra eighty megabytes pushed the live
    /// world further out of residency. See `docs/performance.md`.
    pub fn snapshot(&self) -> Self {
        let mut copy = self.clone();
        // The copy is never served to anybody, so section caching is dead weight on it.
        copy.dirty_sections.clear();
        copy.track_dirty = false;
        copy
    }

    /// Overwrite `self` with another world's state, reusing the allocations already here.
    ///
    /// Used to take a snapshot into a buffer the server already owns. A fresh `snapshot()` asks
    /// for a new forty-megabyte mapping and faults in every page as it copies; writing into pages
    /// that are already mapped costs the memcpy alone. On the game task that is the difference
    /// between one number and a much worse, much more variable one.
    ///
    /// Deliberately exhaustive, and deliberately not `Clone::clone_from`: the derived one is
    /// `*self = source.clone()`, which allocates exactly as much as it was meant to avoid.
    /// Bring a snapshot buffer up to date, copying only the tiles that changed.
    ///
    /// The expensive part of a snapshot is copying forty megabytes of tiles, and on a server that
    /// is not being actively dug through, almost none of them have changed since the last one.
    /// `changed_since_snapshot` says which sections have, so this copies those and leaves the rest
    /// alone.
    ///
    /// Returns how many sections it copied, which the caller logs — a number that suddenly equals
    /// every section in the world means the incremental path has silently stopped working.
    ///
    /// **The buffer must already hold this world's state as of the last call**, or the sections it
    /// skips will be stale. `snapshot_into` is the only way to get one, and it takes the flags with
    /// it, so the two cannot drift apart.
    pub fn refresh_snapshot(&mut self, buffer: &mut Self) -> usize {
        let sections = self.changed_since_snapshot.drain();
        self.copy_sections_into(buffer, &sections);
        buffer.tiles.copy_side_tables_from(&self.tiles);
        buffer.copy_everything_but_tiles_from(self);
        sections.len()
    }

    /// Copy up to `cap` changed sections' **tiles** into a snapshot buffer, and nothing else.
    ///
    /// The half of [`Self::refresh_snapshot`] that scales with how much the world has changed,
    /// exposed so a save can pay for it a few sections at a time across several ticks rather than
    /// all at once. Measured on a real 4200x1200 world, a section costs about fourteen
    /// microseconds warm and closer to a hundred cold, against a fixed refresh cost - the side and
    /// object tables, below - of twenty microseconds however many sections changed. So the fixed
    /// part is worth paying once at the end, and only this part is worth spreading.
    ///
    /// Deliberately does **not** touch the side or object tables. Those are copied wholesale from
    /// the live world by the final `refresh_snapshot`, so copying them here would be work thrown
    /// away, and doing it *instead* of there would leave the buffer's objects newer than its
    /// tiles, which is exactly the torn save this is meant to avoid.
    ///
    /// Tearing is not a risk the other way either: [`Self::set_tile`] re-marks any section it
    /// touches, so a section copied here and then edited is simply copied again. A buffer whose
    /// [`Self::snapshot_pending`] has reached zero is bit-identical to the live world at that
    /// instant, however many ticks it took to assemble.
    ///
    /// Returns how many sections it copied.
    pub fn pre_copy_snapshot_tiles(&mut self, buffer: &mut Self, cap: usize) -> usize {
        let sections = self.changed_since_snapshot.drain_upto(cap);
        self.copy_sections_into(buffer, &sections);
        sections.len()
    }

    /// How many sections a snapshot refresh would have to copy if it ran right now.
    pub fn snapshot_pending(&self) -> usize {
        self.changed_since_snapshot.marked
    }

    fn copy_sections_into(&self, buffer: &mut Self, sections: &[(i32, i32)]) {
        for &(sx, sy) in sections {
            let x0 = sx * SECTION_WIDTH;
            let y0 = sy * SECTION_HEIGHT;
            buffer.tiles.copy_rect_from(
                &self.tiles,
                x0,
                y0,
                x0 + SECTION_WIDTH,
                y0 + SECTION_HEIGHT,
            );
        }
    }

    /// Whether a buffer can be refreshed rather than rebuilt.
    pub fn snapshot_is_incremental(&self) -> bool {
        self.track_dirty
    }

    pub fn copy_state_from(&mut self, source: &Self) {
        self.tiles.copy_from(&source.tiles);
        self.copy_everything_but_tiles_from(source);
    }

    /// Everything a snapshot needs except the tile array.
    ///
    /// Split out because the incremental path copies only some tiles but always all of this —
    /// chests, signs, residents and the header state are thousands of bytes, not tens of
    /// megabytes, so there is nothing to gain by being clever about them.
    fn copy_everything_but_tiles_from(&mut self, source: &Self) {
        self.chests.clone_from(&source.chests);
        self.signs.clone_from(&source.signs);
        self.town_npcs.clone_from(&source.town_npcs);
        self.shimmered_town_npcs
            .clone_from(&source.shimmered_town_npcs);
        self.saved_npcs.clone_from(&source.saved_npcs);
        self.banner_kills.clone_from(&source.banner_kills);
        self.tile_entities.clone_from(&source.tile_entities);
        self.preserved.clone_from(&source.preserved);
        self.name.clone_from(&source.name);
        self.seed_text.clone_from(&source.seed_text);

        // Everything else is plain scalars and small arrays; copying them wholesale keeps this in
        // step with the struct without listing each one twice.
        self.width = source.width;
        self.height = source.height;
        self.spawn_x = source.spawn_x;
        self.spawn_y = source.spawn_y;
        self.surface = source.surface;
        self.rock_layer = source.rock_layer;
        self.id = source.id;
        self.unique_id = source.unique_id;
        self.time = source.time;
        self.day_time = source.day_time;
        self.blood_moon = source.blood_moon;
        self.eclipse = source.eclipse;
        self.moon_phase = source.moon_phase;
        self.raining = source.raining;
        self.rain_time = source.rain_time;
        self.max_rain = source.max_rain;
        self.sandstorm = source.sandstorm;
        self.sandstorm_time = source.sandstorm_time;
        self.sandstorm_severity = source.sandstorm_severity;
        self.sandstorm_intended_severity = source.sandstorm_intended_severity;
        self.dungeon_x = source.dungeon_x;
        self.dungeon_y = source.dungeon_y;
        self.pumpkin_moon = source.pumpkin_moon;
        self.snow_moon = source.snow_moon;
        self.wind = source.wind;
        self.crimson = source.crimson;
        self.ore_tiers = source.ore_tiers;
        self.progress = source.progress;
        self.game_mode = source.game_mode;
        self.world_gen_version = source.world_gen_version;
        self.moon_type = source.moon_type;
        self.tree_x = source.tree_x;
        self.tree_style = source.tree_style;
        self.cave_back_x = source.cave_back_x;
        self.cave_back_style = source.cave_back_style;
        self.ice_back_style = source.ice_back_style;
        self.jungle_back_style = source.jungle_back_style;
        self.hell_back_style = source.hell_back_style;
        self.backgrounds = source.backgrounds;
        self.tree_tops = source.tree_tops;
        self.num_clouds = source.num_clouds;
        self.next_tile_entity = source.next_tile_entity;
        self.journey_freeze_time = source.journey_freeze_time;
        self.journey_freeze_rain = source.journey_freeze_rain;
        self.journey_freeze_wind = source.journey_freeze_wind;
        self.journey_stop_biome_spread = source.journey_stop_biome_spread;
        self.journey_time_rate_slider = source.journey_time_rate_slider;
        self.journey_difficulty_slider = source.journey_difficulty_slider;

        // A copy is never served to anybody, so section caching is dead weight on it.
        self.dirty_sections.clear();
        self.track_dirty = false;
    }

    /// Drop the caches a copy has no use for.
    ///
    /// A snapshot exists to be written to disk and nothing else, so the dirty-section set is dead
    /// weight on it — and a reused buffer would otherwise carry the previous save's set around.
    pub fn shrink_caches(&mut self) {
        self.dirty_sections.clear();
        self.track_dirty = false;
    }

    /// The tile store, for measuring what copying it costs.
    ///
    /// Exposed only so `examples/snapcost` can weigh the parts of a snapshot against each other.
    /// Nothing in the server reads tiles this way.
    pub fn tiles_for_measurement(&self) -> &super::packed::TileStore {
        &self.tiles
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height
    }

    /// The tile at `(x, y)`, or air outside the world.
    ///
    /// Returning air rather than panicking matters because section rectangles are clamped to the
    /// world only approximately, and a client may request a position near the edge.
    pub fn tile(&self, x: i32, y: i32) -> Tile {
        if self.in_bounds(x, y) {
            self.tiles.get(x, y)
        } else {
            Tile::AIR
        }
    }

    /// Replace a tile. Returns false when the position lies outside the world.
    ///
    /// Deliberately does **not** check whether the tile is already what it is being set to. That
    /// looks like free savings on the marking below and is not: instrumenting this on an idle
    /// fresh 4200x1200 world over six consecutive 20-second autosaves counted 150 to 260 real tile
    /// changes per window and **zero** rewrites of an existing value. Every gameplay write is a
    /// real change, so the check would cost a `TileStore::get` per write and save nothing.
    ///
    /// What those same six windows did show is that 150 to 260 changed tiles mark 24 to 37
    /// sections, and a section is 200x150 = 30,000 tiles: an amplification of about 5,000x, which
    /// is the whole reason a save on an idle world is expensive. Tracking changed tiles rather
    /// than the sections they sit in is in `TODO.md`, with the measurements.
    pub fn set_tile(&mut self, x: i32, y: i32, tile: Tile) -> bool {
        if !self.in_bounds(x, y) {
            return false;
        }
        self.tiles.set(x, y, tile);
        if self.track_dirty {
            let (sx, sy) = self.section_of(x, y);
            self.dirty_sections.mark(sx, sy);
            self.changed_since_snapshot.mark(sx, sy);
        }
        true
    }

    /// What the tiles cost in memory: the array, then each side table.
    pub fn tile_footprint(&self) -> (usize, usize, usize) {
        self.tiles.footprint()
    }

    /// Begin recording which sections change, once the world is fully built.
    pub fn start_tracking_changes(&mut self) {
        self.dirty_sections.clear();
        self.track_dirty = true;
    }

    /// Take the set of sections that changed since the last call.
    pub fn take_dirty_sections(&mut self) -> Vec<(i32, i32)> {
        self.dirty_sections.drain()
    }

    /// How many sections wide the world is, counted the way the client counts.
    ///
    /// `Main.maxSectionsX = Main.maxTilesX / 200` truncates, so rounding up here would invent a
    /// section the client has no array slot for and would never ask about. Terraria's own sizes
    /// are all exact multiples, and [`crate::config::Config::validate`] keeps generated ones that
    /// way too, so this only ever discards a remainder that should not exist.
    pub fn sections_x(&self) -> i32 {
        self.width / SECTION_WIDTH
    }

    pub fn sections_y(&self) -> i32 {
        self.height / SECTION_HEIGHT
    }

    /// The section grid coordinates containing a tile position.
    pub fn section_of(&self, x: i32, y: i32) -> (i32, i32) {
        (x / SECTION_WIDTH, y / SECTION_HEIGHT)
    }

    /// Section bounds clamped so the rectangle never runs past the world edge.
    pub fn section_bounds(&self, section_x: i32, section_y: i32) -> SectionBounds {
        let mut bounds = SectionBounds::of_section(section_x, section_y);
        bounds.width = (self.width - bounds.x).clamp(0, SECTION_WIDTH) as i16;
        bounds.height = (self.height - bounds.y).clamp(0, SECTION_HEIGHT) as i16;
        bounds
    }

    /// Advance the clock by `rate` ticks, rolling over between day and night — `rate` is always 1
    /// except under Journey mode's `ModifyTimeRate` (1×–24×, `GameServer::tick`'s own caller
    /// applies the slider). A `while` rather than a single `if`: at the top of that range a whole
    /// short night can pass inside one call, and a single check would only ever cross it once.
    pub fn tick_time(&mut self, rate: i32) {
        self.time += rate;
        loop {
            let limit = if self.day_time {
                DAY_LENGTH
            } else {
                NIGHT_LENGTH
            };
            if self.time < limit {
                break;
            }
            self.time -= limit;
            self.day_time = !self.day_time;
            if self.day_time {
                self.moon_phase = (self.moon_phase + 1) % 8;
                // A blood moon lasts one night and is over by morning.
                self.blood_moon = false;
            }
        }
    }

    /// Re-read the real-world calendar, as `Main.checkXMas`/`Main.checkHalloween` do
    /// (`Main.cs:66375-66376`, the dawn block; `WorldGen.cs:6906-6907` and `:11267-11268` on load
    /// and generation).
    ///
    /// One clock read per in-game dawn, which is once every twenty-four real minutes at the default
    /// time rate, so nothing on the tick path pays for it.
    pub fn refresh_calendar(&mut self) {
        let (month, day) = super::calendar::today();
        self.halloween = super::calendar::is_halloween(month, day);
        self.xmas = super::calendar::is_xmas(month, day);
    }

    /// Build the packet `7` payload describing this world.
    ///
    /// The flag block is filled in from the world's own history rather than left blank. The client
    /// drives real behaviour off it — which shops open, what a Dryad will talk about, whether the
    /// map draws an event — so a server that sends zeroes leaves every client believing it has
    /// joined a brand-new world however far along the save actually is.
    pub fn world_data(&self) -> WorldData {
        use terrustia_proto::packets::WorldFlag as F;
        let mut flags = WorldFlags::default();
        let p = &self.progress;
        for (flag, on) in [
            (F::Crimson, self.crimson),
            (F::HardMode, p.hard_mode),
            (F::ShadowOrbSmashed, p.shadow_orb_smashed),
            (F::DownedBoss1, p.downed_boss1),
            (F::DownedBoss2, p.downed_boss2),
            (F::DownedBoss3, p.downed_boss3),
            (F::DownedClown, p.downed_clown),
            (F::DownedPlantera, p.downed_plantera),
            (F::DownedMech1, p.downed_mech1),
            (F::DownedMech2, p.downed_mech2),
            (F::DownedMech3, p.downed_mech3),
            (F::DownedMechAny, p.downed_mech_any),
            (F::DownedKingSlime, p.downed_king_slime),
            (F::DownedQueenBee, p.downed_queen_bee),
            (F::DownedFishron, p.downed_fishron),
            (F::DownedMartians, p.downed_martians),
            (F::DownedAncientCultist, p.downed_ancient_cultist),
            (F::DownedMoonLord, p.downed_moon_lord),
            (F::DownedHalloweenKing, p.downed_halloween_king),
            (F::DownedHalloweenTree, p.downed_halloween_tree),
            (F::DownedChristmasIceQueen, p.downed_christmas_ice_queen),
            (F::DownedChristmasSantank, p.downed_christmas_santank),
            (F::DownedChristmasTree, p.downed_christmas_tree),
            (F::DownedGolem, p.downed_golem),
            (F::DownedPirates, p.downed_pirates),
            (F::DownedFrostLegion, p.downed_frost),
            (F::DownedGoblins, p.downed_goblins),
            (F::DownedTowerSolar, p.downed_tower_solar),
            (F::DownedTowerVortex, p.downed_tower_vortex),
            (F::DownedTowerNebula, p.downed_tower_nebula),
            (F::DownedTowerStardust, p.downed_tower_stardust),
            (F::DownedDeerclops, p.downed_deerclops),
            (F::DownedEmpressOfLight, p.downed_empress_of_light),
            (F::DownedQueenSlime, p.downed_queen_slime),
            (F::PumpkinMoon, self.pumpkin_moon),
            (F::SnowMoon, self.snow_moon),
            // The Old One's Army is three separate victories and the Tavernkeep's stock turns on
            // which have happened. These were parsed out of the world file and then never told to
            // anyone, so a world that had beaten tier three still shopped like a fresh one.
            (F::DownedArmyTier1, p.downed_army_t1),
            (F::DownedArmyTier2, p.downed_army_t2),
            (F::DownedArmyTier3, p.downed_army_t3),
            // Advanced Combat Techniques, both volumes, which permanently toughen the townsfolk.
            (F::CombatBookUsed, p.combat_book),
            (F::CombatBookTwoUsed, p.combat_book_two),
            // The three bound town slimes, once freed. A client that still believes the flag is
            // false keeps offering the "free me" interaction for a slime that already moved in.
            (F::UnlockedSlimeOldSpawn, p.unlocked_slime_old),
            (F::UnlockedSlimePurpleSpawn, p.unlocked_slime_purple),
            (F::UnlockedSlimeYellowSpawn, p.unlocked_slime_yellow),
            (F::Sandstorm, self.sandstorm),
            // Every real secret-seed flag with its own client-visible `WorldFlag` bit (this
            // project's own model has no bit for `NoTraps`/`Skyblock` — neither has a client-side
            // rendering difference in real vanilla, only a generation-time one). Real vanilla
            // clients already know how to render each of these on their own once told; nothing
            // else in this server needs to change for a connecting client to get a special seed's
            // own atmosphere (darkness, colour grading, sprite variants) for free.
            (F::DrunkWorld, self.secret_seeds.drunk),
            (F::GetGoodWorld, self.secret_seeds.get_good),
            (F::TenthAnniversary, self.secret_seeds.tenth_anniversary),
            (F::DontStarve, self.secret_seeds.dont_starve),
            (F::NotTheBees, self.secret_seeds.not_the_bees),
            (F::RemixWorld, self.secret_seeds.remix),
        ] {
            flags.set_flag(flag, on);
        }

        WorldData {
            wind_speed_target: self.wind,
            // The client reads rain from its strength, so a dry world has to send nought rather
            // than the strength of whatever the last shower was.
            max_raining: if self.raining { self.max_rain } else { 0.0 },
            time: self.time,
            day_time: self.day_time,
            blood_moon: self.blood_moon,
            eclipse: self.eclipse,
            moon_phase: self.moon_phase,
            max_tiles_x: self.width as i16,
            max_tiles_y: self.height as i16,
            spawn_tile_x: self.spawn_x,
            spawn_tile_y: self.spawn_y,
            world_surface: self.surface,
            rock_layer: self.rock_layer,
            world_id: self.id,
            world_name: self.name.clone(),
            unique_id: self.unique_id,
            flags,
            game_mode: self.game_mode,
            world_gen_version: self.world_gen_version,
            moon_type: self.moon_type,
            tree_x: self.tree_x,
            tree_style: self.tree_style,
            cave_back_x: self.cave_back_x,
            cave_back_style: self.cave_back_style,
            ice_back_style: self.ice_back_style,
            jungle_back_style: self.jungle_back_style,
            hell_back_style: self.hell_back_style,
            backgrounds: self.backgrounds,
            tree_tops: self.tree_tops,
            num_clouds: self.num_clouds,
            // The client reads these for the Guide's hardmode ore hint, so a world that settled on
            // palladium has to say so rather than reporting whatever the default happened to be.
            ore_tiers: self.ore_tiers,
            // Release 326 sends the dungeon entrance here. A world loaded from a file that predates
            // the field, or one still being generated, has no position to give — nought is what the
            // game itself writes for an unplaced dungeon, and it is only ever a cosmetic loss
            // (the client uses it for the map's dungeon marker), unlike leaving the field out.
            dungeon_x: self.dungeon_x.unwrap_or(0) as i16,
            dungeon_y: self.dungeon_y.unwrap_or(0) as i16,
            ..WorldData::default()
        }
    }
}

/// Does every field of [`World`] have a decided route back to disk?
///
/// This module exists because two did not, and nobody noticed for months. `ore_tiers` was chosen
/// when an altar broke and then dropped on every save of a loaded world — which is worse than
/// forgetting a setting, because the header kept reading `-1` for "not chosen" and the *next*
/// altar rolled a second tier, leaving one world with two different ores sprayed through it.
/// `banner_kills` went the same way, while a comment three lines from the code that lost it
/// claimed it survived a restart.
///
/// Both were found by reading. Reading is the wrong instrument: it finds the fields you happen to
/// look at. So the check is mechanical instead — the destructure below has no `..`, which means
/// **adding a field to `World` will not compile until someone says here what happens to it when
/// the world is saved**. That forced decision is the entire point of this module.
#[cfg(test)]
mod persistence {
    use super::*;

    /// Where a field's value goes when a loaded world is written back.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Fate {
        /// Lives in the world header, and so needs an offset in `PreservedWorld` and a write in
        /// `patch_clock`. Getting this wrong is silent data loss.
        Header,
        /// Written as its own section of the file, from our own model.
        Section,
        /// Not stored: rebuilt from the tiles or from another field on load.
        Derived,
        /// Deliberately not saved. Lives only for as long as the server is up.
        Session,
    }

    #[test]
    fn every_world_field_has_a_decided_fate() {
        let world = World::empty(80, 60, "audit");

        // No `..` here, on purpose. A new field breaks this line, and the person who added it has
        // to choose one of the four fates below rather than discovering months later that their
        // field never reached the disk.
        let World {
            width,
            height,
            tiles,
            spawn_x,
            spawn_y,
            surface,
            rock_layer,
            name,
            id,
            unique_id,
            time,
            day_time,
            blood_moon,
            eclipse,
            moon_phase,
            raining,
            rain_time,
            max_rain,
            sandstorm,
            sandstorm_time,
            sandstorm_severity,
            sandstorm_intended_severity,
            dungeon_x,
            dungeon_y,
            pumpkin_moon,
            snow_moon,
            wind,
            crimson,
            ore_tiers,
            progress,
            game_mode,
            world_gen_version,
            seed_text,
            secret_seeds,
            moon_type,
            tree_x,
            tree_style,
            cave_back_x,
            cave_back_style,
            ice_back_style,
            jungle_back_style,
            hell_back_style,
            backgrounds,
            tree_tops,
            num_clouds,
            chests,
            signs,
            town_npcs,
            shimmered_town_npcs,
            saved_npcs,
            journey_freeze_time,
            journey_freeze_rain,
            journey_freeze_wind,
            journey_stop_biome_spread,
            journey_time_rate_slider,
            journey_difficulty_slider,
            banner_kills,
            tile_entities,
            next_tile_entity,
            preserved,
            dirty_sections,
            changed_since_snapshot,
            track_dirty,
            halloween,
            xmas,
        } = &world;

        let fates: &[(&str, Fate, &dyn std::fmt::Debug)] = &[
            // --- the header ------------------------------------------------------------------
            ("time", Fate::Header, time),
            ("day_time", Fate::Header, day_time),
            ("moon_phase", Fate::Header, moon_phase),
            ("raining", Fate::Header, raining),
            ("rain_time", Fate::Header, rain_time),
            ("max_rain", Fate::Header, max_rain),
            ("sandstorm", Fate::Header, sandstorm),
            ("sandstorm_time", Fate::Header, sandstorm_time),
            ("sandstorm_severity", Fate::Header, sandstorm_severity),
            (
                "sandstorm_intended_severity",
                Fate::Header,
                sandstorm_intended_severity,
            ),
            ("wind", Fate::Header, wind),
            ("ore_tiers", Fate::Header, ore_tiers),
            ("banner_kills", Fate::Header, banner_kills),
            ("progress", Fate::Header, progress),
            // --- sections of their own -------------------------------------------------------
            ("tiles", Fate::Section, &std::ptr::from_ref(tiles)),
            ("chests", Fate::Section, chests),
            ("signs", Fate::Section, signs),
            ("town_npcs", Fate::Section, town_npcs),
            ("shimmered_town_npcs", Fate::Section, shimmered_town_npcs),
            ("saved_npcs", Fate::Section, saved_npcs),
            ("journey_freeze_time", Fate::Section, journey_freeze_time),
            ("journey_freeze_rain", Fate::Section, journey_freeze_rain),
            ("journey_freeze_wind", Fate::Section, journey_freeze_wind),
            (
                "journey_stop_biome_spread",
                Fate::Section,
                journey_stop_biome_spread,
            ),
            (
                "journey_time_rate_slider",
                Fate::Section,
                journey_time_rate_slider,
            ),
            (
                "journey_difficulty_slider",
                Fate::Section,
                journey_difficulty_slider,
            ),
            ("tile_entities", Fate::Section, tile_entities),
            // --- carried in the header, but never changed while the server runs ---------------
            //
            // These ride through in the preserved bytes untouched. They need no offset precisely
            // because nothing mutates them; if that ever stops being true they become `Header`.
            ("width", Fate::Derived, width),
            ("height", Fate::Derived, height),
            ("spawn_x", Fate::Derived, spawn_x),
            ("spawn_y", Fate::Derived, spawn_y),
            ("surface", Fate::Derived, surface),
            ("rock_layer", Fate::Derived, rock_layer),
            ("name", Fate::Derived, name),
            ("id", Fate::Derived, id),
            ("unique_id", Fate::Derived, unique_id),
            ("crimson", Fate::Derived, crimson),
            ("game_mode", Fate::Derived, game_mode),
            ("world_gen_version", Fate::Derived, world_gen_version),
            ("seed_text", Fate::Derived, seed_text),
            ("secret_seeds", Fate::Derived, secret_seeds),
            ("moon_type", Fate::Derived, moon_type),
            ("tree_x", Fate::Derived, tree_x),
            ("tree_style", Fate::Derived, tree_style),
            ("cave_back_x", Fate::Derived, cave_back_x),
            ("cave_back_style", Fate::Derived, cave_back_style),
            ("ice_back_style", Fate::Derived, ice_back_style),
            ("jungle_back_style", Fate::Derived, jungle_back_style),
            ("hell_back_style", Fate::Derived, hell_back_style),
            ("backgrounds", Fate::Derived, backgrounds),
            ("tree_tops", Fate::Derived, tree_tops),
            ("num_clouds", Fate::Derived, num_clouds),
            ("dungeon_x", Fate::Derived, dungeon_x),
            ("dungeon_y", Fate::Derived, dungeon_y),
            ("next_tile_entity", Fate::Derived, next_tile_entity),
            ("blood_moon", Fate::Header, blood_moon),
            ("eclipse", Fate::Header, eclipse),
            // --- this session only -----------------------------------------------------------
            //
            // The two seasonal moons, unlike blood_moon/eclipse above, are not resumed on purpose:
            // vanilla does keep them in its header too, but the seasonal-event branch that reads
            // them is not implemented here, so there is nothing meaningful to resume into yet.
            ("pumpkin_moon", Fate::Session, pumpkin_moon),
            ("snow_moon", Fate::Session, snow_moon),
            // Halloween and Christmas are read off the wall clock, not out of the file: vanilla
            // saves neither, and recomputes both from `DateTime.Now` on load, on generation and at
            // every dawn (`Main.cs:66375-66376`). Saving them would be worse than useless, because
            // a world stored in October would come back haunted in June.
            ("halloween", Fate::Session, halloween),
            ("xmas", Fate::Session, xmas),
            ("preserved", Fate::Session, &preserved.is_some()),
            ("dirty_sections", Fate::Session, &dirty_sections.marked),
            (
                "changed_since_snapshot",
                Fate::Session,
                &changed_since_snapshot.marked,
            ),
            ("track_dirty", Fate::Session, track_dirty),
        ];

        // Every field, exactly once. The destructure guarantees none is missing; this guarantees
        // none was listed twice under two different fates.
        let mut seen: Vec<&str> = fates.iter().map(|(name, _, _)| *name).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "a field is classified twice");

        // Whatever else changes, these must stay `Header`: they are the ones that were lost.
        for field in ["ore_tiers", "banner_kills", "blood_moon", "eclipse"] {
            let (_, fate, _) = fates
                .iter()
                .find(|(name, _, _)| *name == field)
                .unwrap_or_else(|| panic!("{field} is no longer classified at all"));
            assert_eq!(
                *fate,
                Fate::Header,
                "{field} must reach the header, or an altar's choice is lost again"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_bounds_reads_are_air_and_writes_are_refused() {
        let mut w = World::empty(10, 10, "t");
        assert_eq!(w.tile(-1, 0), Tile::AIR);
        assert_eq!(w.tile(0, 10), Tile::AIR);
        assert!(!w.set_tile(10, 0, Tile::block(1)));
        assert!(w.set_tile(9, 9, Tile::block(1)));
        assert_eq!(w.tile(9, 9), Tile::block(1));
    }

    #[test]
    fn a_ragged_world_is_counted_the_way_the_client_counts() {
        // The client sizes its grid with `maxTilesX / 200`, which truncates, so a partial section
        // along the far edge does not exist as far as it is concerned. Counting it here would
        // offer a section the client has no slot for and will never request. Config refuses these
        // sizes outright, so this only pins the arithmetic.
        let w = World::empty(250, 200, "t");
        assert_eq!(w.sections_x(), 1);
        assert_eq!(w.sections_y(), 1);
    }

    #[test]
    fn section_bounds_are_clamped_to_the_world_edge() {
        // Asked directly for the ragged section anyway, the bounds still stop at the world's edge
        // rather than describing tiles past its end.
        let w = World::empty(250, 200, "t");
        let last = w.section_bounds(1, 1);
        assert_eq!(last.x, 200);
        assert_eq!(last.y, 150);
        assert_eq!(last.width, 50);
        assert_eq!(last.height, 50);
    }

    #[test]
    fn a_whole_section_keeps_its_full_size() {
        let w = World::empty(400, 300, "t");
        let bounds = w.section_bounds(0, 0);
        assert_eq!((bounds.width, bounds.height), (200, 150));
    }

    #[test]
    fn the_clock_rolls_from_day_into_night() {
        let mut w = World::empty(10, 10, "t");
        w.day_time = true;
        w.time = DAY_LENGTH - 1;
        w.tick_time(1);
        assert!(!w.day_time);
        assert_eq!(w.time, 0);
    }

    #[test]
    fn a_full_day_advances_the_moon_phase_once() {
        let mut w = World::empty(10, 10, "t");
        w.day_time = false;
        w.time = NIGHT_LENGTH - 1;
        let before = w.moon_phase;
        w.tick_time(1);
        assert!(w.day_time);
        assert_eq!(w.moon_phase, (before + 1) % 8);
    }

    /// Journey mode's `ModifyTimeRate` can push the clock up to 24 ticks in one call — enough, at
    /// the top of a short night, to cross into the next day and partway through it in a single
    /// `tick_time`. A single `if` would only ever cross the boundary once and lose the remainder;
    /// this is exactly the bug a `while`-shaped fix would leave in place if it only ran one lap.
    #[test]
    fn a_large_rate_can_cross_a_day_night_boundary_within_one_call() {
        let mut w = World::empty(10, 10, "t");
        w.day_time = false;
        w.time = NIGHT_LENGTH - 5;
        w.tick_time(20);
        assert!(w.day_time, "20 ticks should have crossed the 5 remaining");
        assert_eq!(
            w.time, 15,
            "the other 15 ticks should carry into the new day"
        );
    }

    #[test]
    fn section_extras_are_filtered_and_ordered_like_the_game() {
        let mut w = World::empty(400, 300, "t");
        let chest = |x: i16, y: i16, name: &str| {
            Some(Chest {
                x,
                y,
                name: name.into(),
                items: Vec::new(),
            })
        };
        // Two chests inside section (0, 0), plus one that belongs to the next section along.
        w.chests = vec![
            chest(189, 96, "low"),
            chest(98, 22, "high"),
            chest(250, 22, "other section"),
        ];
        w.signs = vec![
            Some(Sign {
                x: 10,
                y: 90,
                text: "b".into(),
            }),
            Some(Sign {
                x: 10,
                y: 10,
                text: "a".into(),
            }),
        ];

        let extras = w.extras_for(w.section_bounds(0, 0));
        assert_eq!(extras.chests.len(), 2, "the far chest should be excluded");
        // Vanilla collects these as its row-major walk reaches them, so lower y comes first.
        assert_eq!(extras.chests[0].name, "high");
        assert_eq!(extras.chests[1].name, "low");
        assert_eq!(extras.signs[0].text, "a");
        assert_eq!(extras.signs[1].text, "b");
    }

    #[test]
    fn world_data_reflects_the_world() {
        let mut w = World::empty(4200, 1200, "Terrustia");
        w.crimson = true;
        let data = w.world_data();
        assert_eq!(data.max_tiles_x, 4200);
        assert_eq!(data.max_tiles_y, 1200);
        assert_eq!(data.world_name, "Terrustia");
        assert_eq!(data.flags.0[1] & 0x20, 0x20, "crimson flag");
    }

    /// Every secret-seed flag with a real client-visible `WorldFlag` bit reaches a connecting
    /// client's own packet 7 — a generated (or loaded) special-seed world used to send an
    /// ordinary-looking flag block regardless of `World::secret_seeds`, since nothing read the
    /// field at all. `NoTraps`/`Everything`("get fixed boi")/`Skyblock` are deliberately not
    /// checked here: they have no `WorldFlag` variant in this project's own packet model yet
    /// (extending it would grow the flag block's own byte count, a larger, separate change) — see
    /// `plan.md`'s own note on this fix for the disclosed reason.
    #[test]
    fn world_data_carries_every_client_visible_secret_seed_flag() {
        use crate::world::worldgen::secret_seed::SecretSeeds;
        let mut w = World::empty(400, 300, "t");
        w.secret_seeds = SecretSeeds {
            drunk: true,
            get_good: true,
            tenth_anniversary: true,
            dont_starve: true,
            not_the_bees: true,
            remix: true,
            ..SecretSeeds::none()
        };
        let data = w.world_data();
        assert_eq!(data.flags.0[6] & 0x10, 0x10, "drunk world flag");
        assert_eq!(
            data.flags.0[6] & 0x80,
            0x80,
            "for the worthy (get_good) flag"
        );
        assert_eq!(data.flags.0[7] & 0x01, 0x01, "celebrationmk10 flag");
        assert_eq!(data.flags.0[7] & 0x02, 0x02, "don't starve flag");
        assert_eq!(data.flags.0[7] & 0x08, 0x08, "not the bees flag");
        assert_eq!(data.flags.0[7] & 0x10, 0x10, "remix flag");
    }

    /// The converse: an ordinary world (no secret seed) must not spuriously set any of these six
    /// bits — a wrong `position()` entry could set the right bit but in a byte shared with an
    /// unrelated flag, which only a check against a real "nothing active" baseline would catch.
    #[test]
    fn an_ordinary_world_sets_none_of_the_secret_seed_flags() {
        let w = World::empty(400, 300, "t");
        let data = w.world_data();
        assert_eq!(data.flags.0[6] & 0b1001_0000, 0, "byte 6 secret-seed bits");
        assert_eq!(data.flags.0[7] & 0b0001_1011, 0, "byte 7 secret-seed bits");
    }

    /// The three freed town slimes reach the client, in `bitsByte12`'s own bit positions
    /// (`NetMessage.cs:362-374`): green, old, purple, rainbow, red, yellow, copper, dusk.
    ///
    /// Fails before the fix, when these flags did not exist: byte 8 was always zero, so a client
    /// on a world whose slimes had all moved in still believed every one of them was out there
    /// waiting to be freed. The empty baseline is checked too, because a wrong bit index sets a
    /// real bit in the right byte and only a "nothing is on" case catches that.
    #[test]
    fn world_data_carries_the_freed_town_slimes() {
        let mut w = World::empty(400, 300, "t");
        assert_eq!(data_byte8(&w), 0, "a fresh world has freed nothing");

        w.progress.unlocked_slime_old = true;
        assert_eq!(data_byte8(&w), 0b0000_0010, "the old slime is bit 1");
        w.progress.unlocked_slime_old = false;

        w.progress.unlocked_slime_purple = true;
        assert_eq!(data_byte8(&w), 0b0000_0100, "the purple slime is bit 2");
        w.progress.unlocked_slime_purple = false;

        w.progress.unlocked_slime_yellow = true;
        assert_eq!(data_byte8(&w), 0b0010_0000, "the yellow slime is bit 5");
    }

    fn data_byte8(w: &World) -> u8 {
        w.world_data().flags.0[8]
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn nothing_is_tracked_until_the_world_is_built() {
        let mut w = World::empty(400, 300, "t");
        w.set_tile(10, 10, Tile::block(1));
        assert!(
            w.take_dirty_sections().is_empty(),
            "generation should not pay for change tracking"
        );
    }

    #[test]
    fn edits_mark_their_section_dirty() {
        let mut w = World::empty(400, 300, "t");
        w.start_tracking_changes();
        w.set_tile(10, 10, Tile::block(1));
        w.set_tile(250, 200, Tile::block(1));

        let mut dirty = w.take_dirty_sections();
        dirty.sort_unstable();
        assert_eq!(dirty, vec![(0, 0), (1, 1)]);
    }

    #[test]
    fn taking_the_set_clears_it() {
        let mut w = World::empty(400, 300, "t");
        w.start_tracking_changes();
        w.set_tile(10, 10, Tile::block(1));
        assert_eq!(w.take_dirty_sections().len(), 1);
        assert!(w.take_dirty_sections().is_empty(), "should not repeat");
    }

    #[test]
    fn repeated_edits_in_one_section_collapse() {
        let mut w = World::empty(400, 300, "t");
        w.start_tracking_changes();
        for x in 0..50 {
            w.set_tile(x, 10, Tile::block(1));
        }
        assert_eq!(w.take_dirty_sections(), vec![(0, 0)]);
    }

    #[test]
    fn an_out_of_bounds_write_marks_nothing() {
        let mut w = World::empty(400, 300, "t");
        w.start_tracking_changes();
        assert!(!w.set_tile(-1, 0, Tile::block(1)));
        assert!(w.take_dirty_sections().is_empty());
    }
}

impl crate::world::liquid::LiquidWorld for World {
    fn tile(&self, x: i32, y: i32) -> Tile {
        World::tile(self, x, y)
    }

    fn set_tile(&mut self, x: i32, y: i32, tile: Tile) {
        World::set_tile(self, x, y, tile);
    }

    fn width(&self) -> i32 {
        World::width(self)
    }

    fn height(&self) -> i32 {
        World::height(self)
    }
}

impl crate::world::hardmode::OreWorld for World {
    fn tile(&self, x: i32, y: i32) -> Tile {
        World::tile(self, x, y)
    }

    fn set_tile(&mut self, x: i32, y: i32, tile: Tile) {
        World::set_tile(self, x, y, tile);
    }

    fn width(&self) -> i32 {
        World::width(self)
    }

    fn height(&self) -> i32 {
        World::height(self)
    }
}

impl crate::world::wiring::WiredWorld for World {
    fn tile(&self, x: i32, y: i32) -> Tile {
        World::tile(self, x, y)
    }

    fn set_tile(&mut self, x: i32, y: i32, tile: Tile) {
        World::set_tile(self, x, y, tile);
    }

    fn width(&self) -> i32 {
        World::width(self)
    }

    fn height(&self) -> i32 {
        World::height(self)
    }

    // The trait defaults for these two are deliberately conservative (surface at row zero, Plantera
    // never down), which is the right answer for a bare test board and the wrong one for a real
    // world: left unoverridden they froze `actuation_allowed`'s Lihzahrd guard permanently shut, so
    // a temple wall stayed unactuatable *after* Plantera fell, and they would do the same to the
    // dungeon teleporter guard (`Wiring.cs:1554-1557`) that reads them next door. The real world has
    // both values already; it simply never handed them over.
    fn surface_y(&self) -> i32 {
        i32::from(self.surface)
    }

    fn downed_plantera(&self) -> bool {
        self.progress.downed_plantera
    }
}

#[cfg(test)]
mod flag_tests {
    use super::*;
    use terrustia_proto::packets::WorldFlag as F;

    fn bit(data: &WorldData, flag: F) -> bool {
        // The flag positions are private to the packet, so this reads them the way a client does:
        // by asking for a world with only that one flag set and seeing which bit moved.
        let mut probe = WorldFlags::default();
        probe.set_flag(flag, true);
        data.flags
            .0
            .iter()
            .zip(probe.0.iter())
            .any(|(sent, wanted)| *wanted != 0 && sent & wanted != 0)
    }

    /// Every flag the world knows about reaches the client.
    #[test]
    fn the_flag_block_carries_the_world_history() {
        let mut world = crate::world::worldgen::generate(400, 300, "flags", 1);
        world.crimson = true;
        world.progress.hard_mode = true;
        world.progress.downed_plantera = true;
        world.progress.downed_moon_lord = true;
        world.progress.downed_tower_nebula = true;
        world.pumpkin_moon = true;

        let data = world.world_data();
        for flag in [
            F::Crimson,
            F::HardMode,
            F::DownedPlantera,
            F::DownedMoonLord,
            F::DownedTowerNebula,
            F::PumpkinMoon,
        ] {
            assert!(bit(&data, flag), "{flag:?} did not reach the client");
        }
        for flag in [
            F::DownedGolem,
            F::SnowMoon,
            F::DownedBoss1,
            F::DownedTowerSolar,
        ] {
            assert!(!bit(&data, flag), "{flag:?} was set and should not be");
        }
    }

    /// The flags a shop reads reach the client.
    ///
    /// These were parsed out of the world file, kept on `Progress`, and then never sent: the
    /// client decides what the Tavernkeep stocks and how tough the townsfolk are from this block,
    /// so a world that had beaten the Old One's Army to tier three still traded like a fresh one.
    #[test]
    fn the_flag_block_carries_what_the_shops_read() {
        let mut world = crate::world::worldgen::generate(400, 300, "shops", 3);
        world.progress.downed_army_t1 = true;
        world.progress.downed_army_t2 = true;
        world.progress.downed_army_t3 = true;
        world.progress.combat_book = true;
        world.progress.combat_book_two = true;
        world.sandstorm = true;

        let data = world.world_data();
        for flag in [
            F::DownedArmyTier1,
            F::DownedArmyTier2,
            F::DownedArmyTier3,
            F::CombatBookUsed,
            F::CombatBookTwoUsed,
            F::Sandstorm,
        ] {
            assert!(bit(&data, flag), "{flag:?} did not reach the client");
        }
    }

    /// A blank world sends a blank flag block, so nothing is set by accident.
    #[test]
    fn a_fresh_world_sends_nothing() {
        let world = crate::world::worldgen::generate(400, 300, "fresh", 2);
        let data = world.world_data();
        let crimson_only = world.crimson;
        let set: u32 = data.flags.0.iter().map(|b| b.count_ones()).sum();
        assert_eq!(
            set,
            u32::from(crimson_only),
            "a fresh world set {set} flags: {:?}",
            data.flags.0
        );
    }

    /// Every flag a town NPC's shop gates on reaches the client.
    ///
    /// Shops are not a server concern: a vanilla client builds one locally from these flags the
    /// moment you talk to somebody, and the server sends nothing about it. That makes this the
    /// whole of shop support, and it is easy to break silently — a missing flag does not fail, it
    /// just quietly closes a shop that should be open.
    ///
    /// The list is the twenty-two conditions `Chest.SetupShop` actually reads.
    #[test]
    fn every_shop_gate_reaches_the_client() {
        let mut world = crate::world::worldgen::generate(400, 300, "shops", 5);
        world.crimson = true;
        world.blood_moon = true;
        world.eclipse = true;
        let p = &mut world.progress;
        p.hard_mode = true;
        p.downed_boss1 = true;
        p.downed_boss2 = true;
        p.downed_boss3 = true;
        p.downed_king_slime = true;
        p.downed_clown = true;
        p.downed_frost = true;
        p.downed_pirates = true;
        p.downed_golem = true;
        p.downed_martians = true;
        p.downed_plantera = true;
        p.downed_mech1 = true;
        p.downed_mech2 = true;
        p.downed_mech3 = true;
        p.downed_mech_any = true;
        p.downed_ancient_cultist = true;
        p.downed_moon_lord = true;
        p.downed_queen_slime = true;
        p.downed_tower_solar = true;

        let data = world.world_data();
        assert!(data.blood_moon, "blood moon");
        assert!(data.eclipse, "eclipse");
        for flag in [
            F::HardMode,
            F::DownedBoss1,
            F::DownedBoss2,
            F::DownedBoss3,
            F::DownedKingSlime,
            F::DownedClown,
            F::DownedFrostLegion,
            F::DownedPirates,
            F::DownedGolem,
            F::DownedMartians,
            F::DownedPlantera,
            F::DownedMech1,
            F::DownedMech2,
            F::DownedMech3,
            F::DownedMechAny,
            F::DownedAncientCultist,
            F::DownedMoonLord,
            F::DownedQueenSlime,
            F::DownedTowerSolar,
            F::Crimson,
        ] {
            assert!(
                bit(&data, flag),
                "{flag:?} would close a shop that should be open"
            );
        }
    }

    /// The rain the client is told about is the rain that is actually falling.
    #[test]
    fn a_dry_world_reports_no_rain() {
        let mut world = crate::world::worldgen::generate(400, 300, "rain", 3);
        world.max_rain = 0.8;
        world.raining = false;
        assert_eq!(world.world_data().max_raining, 0.0, "not raining, no rain");

        world.raining = true;
        assert_eq!(world.world_data().max_raining, 0.8);
    }

    /// The wind reaches the client too, both ways.
    #[test]
    fn the_wind_reaches_the_client() {
        let mut world = crate::world::worldgen::generate(400, 300, "wind", 4);
        world.wind = -0.42;
        assert_eq!(world.world_data().wind_speed_target, -0.42);
    }

    /// A copy made for saving must serialise to exactly the same bytes as the original.
    ///
    /// [`World::snapshot`] is what a background save writes out, so anything it fails to carry is
    /// silently lost from every save. Comparing the *saved bytes* rather than the fields is what
    /// makes that impossible to miss: anything a save records is covered, whether or not this
    /// test knows the field exists.
    #[test]
    fn a_copy_for_saving_saves_identically() {
        use crate::world::{Chest, Sign, wld_save};
        use terrustia_proto::{ItemStack, Tile};

        let mut world = crate::world::worldgen::generate(400, 300, "copies", 11);
        // Put something in every part of the world a save touches, so a lost field shows up.
        world.set_tile(100, 100, Tile::block(1));
        world.set_tile(101, 100, Tile::framed(21, 0, 0));
        world.chests = vec![Some(Chest {
            x: 101,
            y: 100,
            name: "Loot".into(),
            items: vec![ItemStack::new(3507, 1, 81), ItemStack::EMPTY],
        })];
        world.signs = vec![Some(Sign {
            x: 50,
            y: 50,
            text: "hello".into(),
        })];
        world
            .tile_entities
            .push(terrustia_proto::tile_entity::TileEntity::new(
                0,
                terrustia_proto::tile_entity::EntityKind::TeleportationPylon,
                60,
                60,
            ));
        world.next_tile_entity = 1;
        world.time = 12_345;
        world.day_time = false;
        world.blood_moon = true;
        world.wind = 0.375;
        world.progress.hard_mode = true;
        world.progress.altar_count = 7;
        world.progress.downed_plantera = true;

        let original = wld_save::serialize(&world).expect("the original should save");

        let copy = world.snapshot();
        let copied = wld_save::serialize(&copy).expect("the copy should save");
        assert_eq!(
            original.len(),
            copied.len(),
            "a copy for saving produced a different-sized file"
        );
        assert!(
            original == copied,
            "a copy for saving did not serialise identically; a field is being lost"
        );
    }
}

/// Is an incrementally-refreshed snapshot identical to a freshly-copied one?
///
/// It has to be, exactly. The snapshot is what gets written to disk, so a section this skips
/// because it thinks nothing changed there is a section of somebody's world silently rolled back
/// to whatever it held at the last save. That is a worse failure than the cost it is avoiding, so
/// it is checked against a full copy tile by tile rather than trusted.
#[cfg(test)]
mod incremental_snapshot {
    use super::*;

    fn world_with_some_terrain() -> World {
        let mut w = World::empty(600, 450, "incremental");
        for x in 0..600 {
            for y in 200..210 {
                w.set_tile(x, y, Tile::block(1));
            }
        }
        w.start_tracking_changes();
        w
    }

    /// Every tile, after edits scattered across several sections.
    #[test]
    fn a_refreshed_snapshot_matches_a_full_copy() {
        let mut world = world_with_some_terrain();
        let mut buffer = world.snapshot();

        // Edits in a few sections, including the corners of one and a run spanning a boundary.
        world.set_tile(0, 0, Tile::block(25));
        world.set_tile(199, 149, Tile::block(30));
        world.set_tile(200, 150, Tile::block(39));
        world.set_tile(599, 449, Tile::block(38));
        for x in 190..215 {
            world.set_tile(x, 300, Tile::block(37));
        }
        world.time = 4321;
        world.progress.hard_mode = true;

        let copied = world.refresh_snapshot(&mut buffer);
        assert!(
            copied > 0,
            "edits were made, so something should have been copied"
        );

        let full = world.snapshot();
        for y in 0..world.height() {
            for x in 0..world.width() {
                assert_eq!(
                    buffer.tile(x, y),
                    full.tile(x, y),
                    "tile {x},{y} differs between an incremental snapshot and a full one"
                );
            }
        }
        assert_eq!(buffer.time, 4321, "header state comes across too");
        assert!(buffer.progress.hard_mode);
    }

    /// Nothing changed means nothing copied — the case that makes this worth doing at all.
    #[test]
    fn an_untouched_world_copies_no_sections() {
        let mut world = world_with_some_terrain();
        let mut buffer = world.snapshot();
        world.refresh_snapshot(&mut buffer);

        assert_eq!(
            world.refresh_snapshot(&mut buffer),
            0,
            "a world nobody touched should cost no tile copying at all"
        );
    }

    /// Two refreshes in a row, with edits between, must not lose the first round's changes.
    #[test]
    fn successive_refreshes_accumulate() {
        let mut world = world_with_some_terrain();
        let mut buffer = world.snapshot();

        world.set_tile(10, 10, Tile::block(40));
        world.refresh_snapshot(&mut buffer);
        world.set_tile(400, 400, Tile::block(41));
        world.refresh_snapshot(&mut buffer);

        assert_eq!(
            buffer.tile(10, 10).block,
            40,
            "the first edit must still be there"
        );
        assert_eq!(buffer.tile(400, 400).block, 41, "and so must the second");
    }

    /// An edit that only changes a side table — paint, or a frame — still has to come across.
    #[test]
    fn side_table_changes_survive() {
        let mut world = world_with_some_terrain();
        let mut buffer = world.snapshot();

        let mut painted = Tile::block(1);
        painted.color = 12;
        world.set_tile(50, 205, painted);
        let mut framed = Tile::framed(21, 36, 0);
        framed.color = 3;
        world.set_tile(300, 205, framed);

        world.refresh_snapshot(&mut buffer);

        assert_eq!(buffer.tile(50, 205).color, 12, "paint");
        assert_eq!(buffer.tile(300, 205).frame_x, 36, "frames");
        assert_eq!(buffer.tile(300, 205).color, 3);
    }

    /// A refresh paid for a few sections at a time comes out where the one-shot one does.
    ///
    /// The cap has to hold, the count left has to fall by exactly what was taken, and once it
    /// reaches zero the buffer must already hold every edit - so the `refresh_snapshot` that
    /// follows finds nothing left to copy.
    #[test]
    fn a_capped_pre_copy_gets_there_a_few_sections_at_a_time() {
        let mut world = world_with_some_terrain();
        let mut buffer = world.snapshot();
        world.refresh_snapshot(&mut buffer);

        // One edit in every section of the world: 3 across by 3 down.
        let all = (world.sections_x() * world.sections_y()) as usize;
        for sy in 0..world.sections_y() {
            for sx in 0..world.sections_x() {
                world.set_tile(sx * SECTION_WIDTH, sy * SECTION_HEIGHT, Tile::block(25));
            }
        }
        assert_eq!(world.snapshot_pending(), all);

        assert_eq!(
            world.pre_copy_snapshot_tiles(&mut buffer, 2),
            2,
            "the cap has to be honoured"
        );
        assert_eq!(world.snapshot_pending(), all - 2);

        let mut rounds = 1;
        while world.snapshot_pending() > 0 {
            world.pre_copy_snapshot_tiles(&mut buffer, 2);
            rounds += 1;
        }
        assert_eq!(rounds, all.div_ceil(2), "and nothing may be skipped");

        assert_eq!(
            world.refresh_snapshot(&mut buffer),
            0,
            "a drained world leaves the firing tick no tiles to copy"
        );
        for sy in 0..world.sections_y() {
            for sx in 0..world.sections_x() {
                assert_eq!(
                    buffer.tile(sx * SECTION_WIDTH, sy * SECTION_HEIGHT).block,
                    25,
                    "section {sx},{sy} never made it across"
                );
            }
        }
    }

    /// A section edited after it was pre-copied is copied again, which is what makes the buffer
    /// a point-in-time image of the tick the drain finished rather than a smear across ticks.
    #[test]
    fn an_edit_after_a_pre_copy_re_marks_the_section() {
        let mut world = world_with_some_terrain();
        let mut buffer = world.snapshot();
        world.refresh_snapshot(&mut buffer);

        world.set_tile(10, 10, Tile::block(40));
        assert_eq!(world.pre_copy_snapshot_tiles(&mut buffer, 8), 1);
        assert_eq!(world.snapshot_pending(), 0);

        world.set_tile(10, 10, Tile::block(41));
        assert_eq!(
            world.snapshot_pending(),
            1,
            "the section has to go back on the list"
        );
        world.refresh_snapshot(&mut buffer);
        assert_eq!(buffer.tile(10, 10).block, 41, "and the later edit must win");
    }
}
