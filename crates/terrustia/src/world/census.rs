//! How much of the world is hallow, corruption and crimson.
//!
//! The Dryad reads all three out when you talk to her, and they are the only measure a player has
//! of whether the evil is winning. The client cannot work them out for itself — it only ever holds
//! the sections it has asked for — so the server has to count and tell it, in packet 57.
//!
//! Ported from `WorldGen.CountTiles` and `WorldGen.AddUpAlignmentCounts`. Three things about the
//! original are easy to miss and are all deliberate here:
//!
//! * **The surface counts five times over.** Rows from 40 down to the surface are weighted `5`, and
//!   everything below to `maxTilesY - 40` is weighted `1`. Corruption on the surface is what a
//!   player sees, so it is what the number is mostly about.
//! * **Dirt is invisible to it.** Tile type 0 is skipped outright, so soil never reaches the
//!   denominator however much of the world is made of it. It does not interrupt the run the game
//!   batches its counting into either — though that part is only an optimisation, since splitting
//!   a run in two adds the same weight to the same type as leaving it whole.
//! * **The denominator is not every solid tile.** It is six specific types — grass, stone, jungle
//!   grass, sand, ice and the flower variant of grass — plus every hallow, corrupt and crimson
//!   tile. Ore, wood and brick are not in it at all.
//!
//! One column is counted per tick, as the game does. A full sweep of a small world therefore takes
//! about a minute of play, which is the same freshness the real game offers.

use super::World;

/// Tile types counted towards each alignment, from `TileID.Sets.*CountCollection`.
///
/// Ten each rather than the eight in the matching `Corrupt`/`Hallow`/`Crimson` boolean sets: the
/// counting lists also take the two "thorny bush and vine" types, which spread the biome without
/// being part of it.
const HALLOW: [u16; 10] = [109, 492, 117, 116, 164, 402, 403, 115, 110, 113];
const CORRUPT: [u16; 10] = [23, 661, 25, 112, 163, 398, 400, 636, 24, 32];
const CRIMSON: [u16; 10] = [199, 662, 203, 234, 200, 399, 401, 205, 201, 352];

/// The plain tiles that make up the rest of the denominator.
///
/// Grass, the flowering grass variant, stone, jungle grass, sand and ice.
const NEUTRAL: [u16; 6] = [2, 477, 1, 60, 53, 161];

/// A sweep of the world in progress.
///
/// Two sets of totals, because a sweep takes thousands of ticks and reporting a half-counted world
/// would swing the percentages wildly as the count crossed a biome. The published figures come
/// from the last completed sweep while the next one accumulates behind them, which is what the
/// game does with its `total*` and `total*2` pairs.
#[derive(Debug, Clone)]
pub struct Census {
    /// Which column the sweep is up to.
    column: i32,
    /// Weighted tile counts for the sweep under way, indexed by tile type.
    running: Vec<i32>,
    /// Totals from the sweep in progress.
    hallow: i64,
    corrupt: i64,
    crimson: i64,
    solid: i64,
    /// What the last completed sweep found, as whole percentages.
    pub percent_hallow: u8,
    pub percent_corrupt: u8,
    pub percent_crimson: u8,
    /// Set for one tick when a sweep completes and the figures are worth broadcasting.
    pub just_finished: bool,
}

impl Census {
    pub fn new(tile_count: u16) -> Self {
        Self {
            column: 0,
            running: vec![0; usize::from(tile_count)],
            hallow: 0,
            corrupt: 0,
            crimson: 0,
            solid: 0,
            percent_hallow: 0,
            percent_corrupt: 0,
            percent_crimson: 0,
            just_finished: false,
        }
    }

    /// Count one column, advancing the sweep. Call once per tick.
    pub fn tick(&mut self, world: &World) {
        self.just_finished = false;
        if world.width() <= 0 {
            return;
        }
        self.count_column(world, self.column);
        self.column += 1;
        if self.column >= world.width() {
            self.column = 0;
            self.publish();
        }
    }

    /// Run a whole sweep in one go, so a freshly loaded world has figures before its first minute
    /// is up rather than reporting nought of everything to whoever joins first.
    pub fn sweep(&mut self, world: &World) {
        for x in 0..world.width() {
            self.count_column(world, x);
        }
        self.column = 0;
        self.publish();
    }

    /// Turn the running totals into percentages and start the next sweep.
    fn publish(&mut self) {
        let share = |part: i64| -> u8 {
            if self.solid <= 0 {
                return 0;
            }
            let rounded = (part as f64 / self.solid as f64 * 100.0).round() as i64;
            // The game's floor: a biome that exists at all reports at least one per cent, so a
            // sliver of corruption is never indistinguishable from none.
            if rounded == 0 && part > 0 {
                1
            } else {
                rounded.clamp(0, 100) as u8
            }
        };
        self.percent_hallow = share(self.hallow);
        self.percent_corrupt = share(self.corrupt);
        self.percent_crimson = share(self.crimson);
        self.just_finished = true;

        self.hallow = 0;
        self.corrupt = 0;
        self.crimson = 0;
        self.solid = 0;
    }

    /// Tally one column into the running totals.
    fn count_column(&mut self, world: &World, x: i32) {
        // Type of the run in progress and how much weight has accumulated in it. Both persist
        // across the two bands, exactly as the game's do.
        let mut run_type = 0u16;
        let mut run = 0i32;

        let surface = i32::from(world.surface) + 1;
        for (from, to, weight) in [(40, surface, 5), (surface, world.height() - 40, 1)] {
            for y in from..to {
                let tile = world.tile(x, y);
                if !tile.is_active() {
                    continue;
                }
                let block = tile.block;
                // Dirt neither counts nor breaks the run.
                if block == 0 {
                    continue;
                }
                if block == run_type {
                    run += weight;
                    continue;
                }
                self.add(run_type, run);
                run_type = block;
                run = weight;
            }
            // Each band flushes its tail, but the run type carries into the next.
            self.add(run_type, run);
            run = 0;
        }

        self.fold();
    }

    fn add(&mut self, block: u16, weight: i32) {
        if let Some(slot) = self.running.get_mut(usize::from(block)) {
            *slot += weight;
        }
    }

    /// Move this column's per-type counts into the alignment totals and clear them.
    fn fold(&mut self) {
        let mut take = |types: &[u16]| -> i64 {
            let mut sum = 0i64;
            for &block in types {
                if let Some(slot) = self.running.get_mut(usize::from(block)) {
                    sum += i64::from(*slot);
                    *slot = 0;
                }
            }
            sum
        };
        let hallow = take(&HALLOW);
        let corrupt = take(&CORRUPT);
        let crimson = take(&CRIMSON);
        let neutral = take(&NEUTRAL);

        self.hallow += hallow;
        self.corrupt += corrupt;
        self.crimson += crimson;
        self.solid += neutral + hallow + corrupt + crimson;

        // Everything else counted this column is discarded, as the game discards it: the whole
        // array is cleared after each fold, so wood and ore never reach the denominator.
        self.running.iter_mut().for_each(|slot| *slot = 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    fn world_of(block: u16, rows: i32) -> World {
        let mut world = World::empty(200, 400, "census");
        world.surface = 100;
        for x in 0..200 {
            for y in 40..40 + rows {
                world.set_tile(x, y, Tile::block(block));
            }
        }
        world
    }

    #[test]
    fn a_world_of_plain_stone_is_nought_per_cent_of_everything() {
        let world = world_of(1, 200);
        let mut census = Census::new(terrustia_proto::tile_sets::TILE_COUNT);
        census.sweep(&world);
        assert_eq!(
            (
                census.percent_hallow,
                census.percent_corrupt,
                census.percent_crimson
            ),
            (0, 0, 0)
        );
    }

    #[test]
    fn a_world_of_ebonstone_is_wholly_corrupt() {
        // Ebonstone is both an alignment tile and part of the denominator, so a world made only of
        // it is a hundred per cent corrupt rather than a division by nothing.
        let world = world_of(25, 200);
        let mut census = Census::new(terrustia_proto::tile_sets::TILE_COUNT);
        census.sweep(&world);
        assert_eq!(census.percent_corrupt, 100);
        assert_eq!(census.percent_hallow, 0);
    }

    #[test]
    fn wood_is_not_in_the_denominator() {
        // A world of wood has no countable tiles at all, so the percentages stay at nought rather
        // than the count dividing by a number that includes it.
        let world = world_of(30, 200);
        let mut census = Census::new(terrustia_proto::tile_sets::TILE_COUNT);
        census.sweep(&world);
        assert_eq!(census.percent_corrupt, 0);
        assert_eq!(census.percent_hallow, 0);
    }

    #[test]
    fn the_surface_counts_five_times_as_heavily() {
        // Half the columns corrupt on the surface, the other half hallow underground, in equal
        // tile counts. The surface half should come out five times the size.
        let mut world = World::empty(200, 400, "weighting");
        world.surface = 100;
        for x in 0..200 {
            if x % 2 == 0 {
                for y in 50..90 {
                    world.set_tile(x, y, Tile::block(25)); // ebonstone, above the surface line
                }
            } else {
                for y in 150..190 {
                    world.set_tile(x, y, Tile::block(117)); // pearlstone, below it
                }
            }
        }
        let mut census = Census::new(terrustia_proto::tile_sets::TILE_COUNT);
        census.sweep(&world);
        assert_eq!(
            census.percent_corrupt, 83,
            "surface corruption should outweigh equal underground hallow five to one"
        );
        assert_eq!(census.percent_hallow, 17);
    }

    #[test]
    fn a_sliver_of_corruption_reports_one_per_cent_rather_than_none() {
        let mut world = World::empty(200, 400, "sliver");
        world.surface = 100;
        for x in 0..200 {
            for y in 150..190 {
                world.set_tile(x, y, Tile::block(1));
            }
        }
        // A single corrupt tile against eight thousand stone rounds to nought.
        world.set_tile(5, 160, terrustia_proto::Tile::block(25));

        let mut census = Census::new(terrustia_proto::tile_sets::TILE_COUNT);
        census.sweep(&world);
        assert_eq!(census.percent_corrupt, 1);
    }

    #[test]
    fn dirt_is_skipped_rather_than_counted() {
        // A world that is mostly soil would otherwise have a denominator dominated by tiles the
        // game never counts, and every biome percentage would come out a fraction of its real
        // size. Counted directly rather than through `sweep`, which clears the totals as it
        // publishes them.
        let mut world = World::empty(4, 400, "dirt");
        world.surface = 100;
        for y in 150..153 {
            world.set_tile(0, y, Tile::block(1)); // stone
        }
        for y in 150..190 {
            world.set_tile(1, y, Tile::block(0)); // dirt, and plenty of it
        }

        let mut census = Census::new(terrustia_proto::tile_sets::TILE_COUNT);
        for x in 0..world.width() {
            census.count_column(&world, x);
        }
        assert_eq!(
            census.solid, 3,
            "only the three stone tiles should be in the denominator"
        );
    }

    /// The startup sweep has to be cheap enough to run before the first player connects.
    ///
    /// It walks every tile of the world once, which on a large world is twenty million of them.
    /// Measured rather than assumed: a startup pause nobody budgeted for is exactly the kind of
    /// thing that gets discovered by a player wondering why the server takes a while to come up.
    #[test]
    fn a_full_sweep_of_a_large_world_is_quick() {
        let mut world = World::empty(8400, 2400, "large");
        world.surface = 800;
        // Filled rather than empty: an empty world skips the counting entirely, which would make
        // this measure nothing at all.
        for x in 0..8400 {
            for y in 800..1600 {
                world.set_tile(x, y, Tile::block(1));
            }
        }

        let started = std::time::Instant::now();
        let mut census = Census::new(terrustia_proto::tile_sets::TILE_COUNT);
        census.sweep(&world);
        let elapsed = started.elapsed();

        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "a full sweep of a large world took {elapsed:?}"
        );
        println!("full sweep of 8400x2400: {elapsed:?}");
    }

    #[test]
    fn a_ticked_sweep_reaches_the_same_answer_as_a_whole_one() {
        // The per-tick path is the one that actually runs; a divergence between it and the sweep
        // used at startup would mean the Dryad's figures changed a minute after anyone joined.
        let world = world_of(25, 200);
        let mut all_at_once = Census::new(terrustia_proto::tile_sets::TILE_COUNT);
        all_at_once.sweep(&world);

        let mut by_tick = Census::new(terrustia_proto::tile_sets::TILE_COUNT);
        for _ in 0..world.width() {
            by_tick.tick(&world);
        }
        assert!(by_tick.just_finished, "the sweep should have wrapped");
        assert_eq!(by_tick.percent_corrupt, all_at_once.percent_corrupt);
        assert_eq!(by_tick.percent_hallow, all_at_once.percent_hallow);
        assert_eq!(by_tick.percent_crimson, all_at_once.percent_crimson);
    }
}
