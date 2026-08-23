use std::collections::HashSet;

use terrustia_proto::{
    SectionBounds, SectionExtras, Tile,
    packets::{WorldData, WorldFlags},
    section::{SECTION_HEIGHT, SECTION_WIDTH},
};

use super::objects::{Chest, MAX_CHESTS, MAX_SIGNS, PreservedWorld, Sign};
use super::progress::Progress;

/// Ticks of daylight, then of night, matching the vanilla clock.
pub const DAY_LENGTH: i32 = 54_000;
pub const NIGHT_LENGTH: i32 = 32_400;

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
    tiles: Vec<Tile>,
    pub spawn_x: i16,
    pub spawn_y: i16,
    pub surface: i16,
    pub rock_layer: i16,
    pub name: String,
    pub id: i32,
    pub unique_id: [u8; 16],
    pub time: i32,
    pub day_time: bool,
    /// A blood moon changes behaviour, not just lighting: fighters that would open a door break it
    /// down instead, and the night spawn pool widens.
    pub blood_moon: bool,
    /// A solar eclipse: a daytime event, and the only time Mothron and its brood appear.
    pub eclipse: bool,
    pub moon_phase: u8,
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
    pub moon_type: u8,
    pub tree_x: [i32; 3],
    pub tree_style: [u8; 4],
    pub cave_back_x: [i32; 3],
    pub cave_back_style: [u8; 4],
    pub ice_back_style: u8,
    pub jungle_back_style: u8,
    pub hell_back_style: u8,
    /// Chests and signs are announced in the trailer of whichever section contains them, so a
    /// loaded world has to keep them alongside the tiles.
    ///
    /// These are sparse: an index is the id that travels on the wire, so removing an entry leaves
    /// a hole rather than renumbering everything after it.
    pub chests: Vec<Option<Chest>>,
    pub signs: Vec<Option<Sign>>,
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
    dirty_sections: HashSet<(i32, i32)>,
    track_dirty: bool,
}

impl World {
    /// An empty world of the given tile dimensions.
    pub fn empty(width: i32, height: i32, name: impl Into<String>) -> Self {
        assert!(width > 0 && height > 0, "world dimensions must be positive");
        Self {
            width,
            height,
            tiles: vec![Tile::AIR; (width * height) as usize],
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
            progress: Progress::default(),
            game_mode: 0,
            world_gen_version: 0,
            seed_text: String::new(),
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
            chests: Vec::new(),
            signs: Vec::new(),
            tile_entities: Vec::new(),
            next_tile_entity: 0,
            preserved: None,
            dirty_sections: HashSet::new(),
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
    /// Prefer [`World::copy_into`] where a buffer can be kept: a fresh allocation of sixty
    /// megabytes costs about thirty milliseconds, and almost none of that is the copying.
    pub fn snapshot(&self) -> Self {
        let mut copy = self.clone();
        // The copy is never served to anybody, so section caching is dead weight on it.
        copy.dirty_sections.clear();
        copy.track_dirty = false;
        copy
    }

    /// Copy this world over an existing one, reusing its tile allocation.
    ///
    /// The point is the tiles, and the cost being avoided is not what it looks like. A fresh
    /// [`World::snapshot`] of a large world measured **31 ms** on the tick — still twice the
    /// budget — and almost none of that was the copying. It was the operating system faulting in
    /// sixty megabytes of *new* pages. Copying into a buffer whose pages have already been
    /// touched skips all of it.
    ///
    /// The implementation looks roundabout and is deliberately so. The obvious version would list
    /// the fields to copy, and the day somebody adds a field and forgets to list it, saves start
    /// silently losing it — a far worse bug than a slow tick. So the derived `Clone` still does
    /// the work, and the tiles are simply moved out of the way while it runs: no field can be
    /// forgotten, because none is named.
    pub fn copy_into(&mut self, into: &mut Self) {
        // Set our own tiles aside so cloning us copies only the small fields.
        let ours = std::mem::take(&mut self.tiles);
        let mut copy = self.clone();
        self.tiles = ours;

        // Reuse the buffer's tile allocation rather than the fresh one `clone` would give.
        let mut buffer = std::mem::take(&mut into.tiles);
        if buffer.len() == self.tiles.len() {
            buffer.copy_from_slice(&self.tiles);
        } else {
            buffer.clear();
            buffer.extend_from_slice(&self.tiles);
        }
        copy.tiles = buffer;

        // The copy is never served to anybody, so section caching is dead weight on it.
        copy.dirty_sections.clear();
        copy.track_dirty = false;
        *into = copy;
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
            self.tiles[(y * self.width + x) as usize]
        } else {
            Tile::AIR
        }
    }

    /// Replace a tile. Returns false when the position lies outside the world.
    pub fn set_tile(&mut self, x: i32, y: i32, tile: Tile) -> bool {
        if !self.in_bounds(x, y) {
            return false;
        }
        self.tiles[(y * self.width + x) as usize] = tile;
        if self.track_dirty {
            self.dirty_sections.insert(self.section_of(x, y));
        }
        true
    }

    /// Begin recording which sections change, once the world is fully built.
    pub fn start_tracking_changes(&mut self) {
        self.dirty_sections.clear();
        self.track_dirty = true;
    }

    /// Take the set of sections that changed since the last call.
    pub fn take_dirty_sections(&mut self) -> Vec<(i32, i32)> {
        self.dirty_sections.drain().collect()
    }

    pub fn sections_x(&self) -> i32 {
        // `i32::div_ceil` is still unstable, and both operands are known positive here.
        (self.width + SECTION_WIDTH - 1) / SECTION_WIDTH
    }

    pub fn sections_y(&self) -> i32 {
        (self.height + SECTION_HEIGHT - 1) / SECTION_HEIGHT
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

    /// Advance the clock by one tick, rolling over between day and night.
    pub fn tick_time(&mut self) {
        self.time += 1;
        let limit = if self.day_time {
            DAY_LENGTH
        } else {
            NIGHT_LENGTH
        };
        if self.time >= limit {
            self.time = 0;
            self.day_time = !self.day_time;
            if self.day_time {
                self.moon_phase = (self.moon_phase + 1) % 8;
                // A blood moon lasts one night and is over by morning.
                self.blood_moon = false;
            }
        }
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
            ..WorldData::default()
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
    fn section_bounds_are_clamped_to_the_world_edge() {
        // A world that is not a whole number of sections wide must not describe tiles past its end.
        let w = World::empty(250, 200, "t");
        assert_eq!(w.sections_x(), 2);
        assert_eq!(w.sections_y(), 2);

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
        w.tick_time();
        assert!(!w.day_time);
        assert_eq!(w.time, 0);
    }

    #[test]
    fn a_full_day_advances_the_moon_phase_once() {
        let mut w = World::empty(10, 10, "t");
        w.day_time = false;
        w.time = NIGHT_LENGTH - 1;
        let before = w.moon_phase;
        w.tick_time();
        assert!(w.day_time);
        assert_eq!(w.moon_phase, (before + 1) % 8);
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
    /// This is the guard on [`World::copy_into`]. That method reuses a buffer's tile allocation
    /// to keep a save off the tick, and the tempting way to write it is to list the fields to
    /// copy — which works right up until somebody adds a field and forgets to list it, at which
    /// point saves silently start losing it.
    ///
    /// Comparing the *saved bytes* rather than the fields is what makes that impossible to miss:
    /// anything a save records is covered, whether or not this test knows the field exists.
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
        world.tile_entities.push(terrustia_proto::tile_entity::TileEntity::new(
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

        // A buffer shaped like some *other* world, to prove the copy replaces everything rather
        // than merging into what was already there.
        let mut buffer = crate::world::worldgen::generate(400, 300, "somewhere else", 99);
        buffer.time = 999;
        buffer.progress.hard_mode = false;
        world.copy_into(&mut buffer);

        let copied = wld_save::serialize(&buffer).expect("the copy should save");
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

    /// ...and the buffer is genuinely reused, which is the entire point of the method.
    #[test]
    fn copying_reuses_the_buffers_tiles() {
        let mut world = crate::world::worldgen::generate(400, 300, "reuse", 3);
        let mut buffer = world.snapshot();
        let before = buffer.tiles.as_ptr();
        world.copy_into(&mut buffer);
        assert_eq!(
            before,
            buffer.tiles.as_ptr(),
            "the tile allocation should be the same one, or nothing has been saved"
        );
    }
}
