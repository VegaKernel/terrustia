//! Water, lava and honey, and what they do when nothing is holding them up.
//!
//! Liquid does not simulate everywhere. It simulates where something has just changed — a tile
//! mined, a bucket poured, a neighbour that moved — and each change wakes its neighbours, so a
//! disturbance spreads outward and then stops. That is what makes it affordable: a still ocean
//! costs nothing at all.
//!
//! The flow itself is two steps. First everything that can fall does, all the way, into whatever
//! room is below it. Then whatever is left levels sideways, and the levelling is not pairwise: it
//! averages across seven tiles where it can, five where it cannot, three where it cannot manage
//! that, and two as a last resort. Averaging wide is what makes a pool find its level in a few
//! ticks rather than creeping one tile at a time.
//!
//! Where two different liquids meet, they react rather than mixing: water on lava is obsidian,
//! honey on lava is crispy honey block, and honey on water is a honey block.

use std::collections::VecDeque;

use terrustia_proto::tile::{Liquid, Tile};

/// The most tiles the simulation will touch in one tick.
///
/// A cavern flooding is the worst case, and it is better for it to take several ticks than for one
/// tick to take several frames.
pub const BUDGET: usize = 8_000;

/// The most tiles that may be waiting at once.
///
/// The queue takes duplicates because checking is dearer than visiting twice, so a determined
/// flood can push far more in than comes out. Dropping the excess is right rather than merely
/// cheap: a tile that is genuinely still moving will be woken again by its neighbour on the next
/// tick, and one that is not was going to do nothing anyway.
pub const QUEUE_CAP: usize = 200_000;

/// How full a tile has to be before it counts as a full block of liquid.
const FULL: u8 = 255;
/// Lava moves at a fifth of water's pace, and honey slower still.
const LAVA_DELAY: u8 = 5;
const HONEY_DELAY: u8 = 10;

/// What a tile turns into where two liquids meet.
mod reaction {
    /// Water on lava.
    pub const OBSIDIAN: u16 = 56;
    /// Honey on lava.
    pub const CRISPY_HONEY: u16 = 230;
    /// Honey on water.
    pub const HONEY_BLOCK: u16 = 229;
    /// Shimmer on anything at all — `Liquid.GetLiquidMergeTypes`'s own shimmer branch always
    /// makes this block and always survives as shimmer, whatever the other liquid was.
    pub const AETHERIUM: u16 = 659;
}

/// A queue of tiles whose liquid may still need to move.
#[derive(Debug, Default)]
pub struct Liquids {
    queue: VecDeque<(i32, i32)>,
    /// A per-tile delay, so lava and honey creep rather than run.
    settling: std::collections::HashMap<(i32, i32), u8>,
}

/// What one tick of settling changed, so the caller can tell clients about it.
#[derive(Debug, Default)]
pub struct Settled {
    /// Tiles whose liquid changed.
    pub changed: Vec<(i32, i32)>,
    /// Tiles that turned to stone where two liquids met.
    pub reacted: Vec<(i32, i32, u16)>,
    /// Tiles liquid just arrived on that were carrying something — `Liquid.AddWater`'s own
    /// `CheckLavaDeath`/`CheckWaterDeath` check (`Liquid.cs:1192-1215`), which kills whatever is
    /// sitting there (a torch, a plant, and a good deal else) the instant liquid touches it.
    ///
    /// Reported rather than resolved here: vanilla decides *which* furniture actually dies from
    /// a per-type flag pair (`TileObjectData`'s own `WaterDeath`/`LavaDeath`) this project has no
    /// table for, so every active tile liquid newly reaches is listed — the caller, which does
    /// have (or can build) that table, is the one that can tell a torch from a chest.
    pub drowned: Vec<(i32, i32)>,
}

/// What the simulation needs of the world: read a tile, write one back.
pub trait LiquidWorld {
    fn tile(&self, x: i32, y: i32) -> Tile;
    fn set_tile(&mut self, x: i32, y: i32, tile: Tile);
    fn width(&self) -> i32;
    fn height(&self) -> i32;
}

impl Liquids {
    /// Wake a tile and everything around it. This is what a mined block or a poured bucket does.
    pub fn disturb(&mut self, x: i32, y: i32) {
        for (dx, dy) in [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)] {
            self.wake(x + dx, y + dy);
        }
    }

    /// Wake one tile.
    pub fn wake(&mut self, x: i32, y: i32) {
        // The queue can hold a tile more than once; the cost of checking is higher than the cost
        // of visiting one twice, and a visit with nothing to do is nearly free.
        if self.queue.len() < QUEUE_CAP {
            self.queue.push_back((x, y));
        }
    }

    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// One tick: settle up to the budget, and report what moved.
    ///
    /// Only the tiles already waiting at the start of the tick are processed. Anything woken while
    /// settling waits for the next one — otherwise a single tile could be visited hundreds of
    /// times in one tick, and lava's delay would be spent within it rather than across seconds.
    pub fn tick(&mut self, world: &mut impl LiquidWorld) -> Settled {
        let mut out = Settled::default();
        let generation = self.queue.len().min(BUDGET);
        for _ in 0..generation {
            let Some((x, y)) = self.queue.pop_front() else {
                break;
            };
            self.settle(world, x, y, &mut out);
        }
        out
    }

    /// Settle one tile.
    fn settle(&mut self, world: &mut impl LiquidWorld, x: i32, y: i32, out: &mut Settled) {
        if x < 1 || y < 1 || x >= world.width() - 1 || y >= world.height() - 1 {
            return;
        }
        let here = world.tile(x, y);
        if here.liquid == 0 {
            self.settling.remove(&(x, y));
            return;
        }
        // Liquid inside a solid block is not liquid any more.
        if solid(here) {
            let mut cleared = here;
            cleared.liquid = 0;
            world.set_tile(x, y, cleared);
            out.changed.push((x, y));
            return;
        }

        // Anything of a different kind next to it reacts rather than flowing.
        if self.react(world, x, y, out) {
            return;
        }

        // Lava and honey take their time. The delay is per tile, so a lava fall does not
        // accelerate merely by being long.
        let delay = match here.liquid_kind {
            Liquid::Lava => LAVA_DELAY,
            Liquid::Honey => HONEY_DELAY,
            _ => 0,
        };
        if delay > 0 {
            let waited = self.settling.entry((x, y)).or_insert(0);
            if *waited < delay {
                *waited += 1;
                self.queue.push_back((x, y));
                return;
            }
            *waited = 0;
        }

        // A lava tile burns the grass around it every real tick it is processed, not only the
        // first time it stops moving — `Liquid.DelWater`'s own per-tick call, not a one-shot
        // "just arrived" hook.
        if here.liquid_kind == Liquid::Lava {
            lava_burn(world, x, y, out);
        }

        if self.fall(world, x, y, out) {
            return;
        }
        self.level(world, x, y, out);
    }

    /// Everything it can pour into the tile below, it pours. Returns whether it emptied.
    fn fall(&mut self, world: &mut impl LiquidWorld, x: i32, y: i32, out: &mut Settled) -> bool {
        let here = world.tile(x, y);
        let below = world.tile(x, y + 1);
        if solid(below) || below.liquid == FULL {
            return false;
        }
        if below.liquid > 0 && below.liquid_kind != here.liquid_kind {
            return false;
        }
        let room = FULL - below.liquid;
        let moved = room.min(here.liquid);
        if moved == 0 {
            return false;
        }
        // Liquid arriving on something that was dry a moment ago — `Liquid.AddWater`'s own
        // `CheckLavaDeath`/`CheckWaterDeath`; see [`Settled::drowned`] for why this only reports
        // rather than resolves it.
        if below.liquid == 0 && below.is_active() {
            out.drowned.push((x, y + 1));
        }

        let mut here = here;
        let mut below = below;
        here.liquid -= moved;
        below.liquid += moved;
        below.liquid_kind = here.liquid_kind;
        world.set_tile(x, y, here);
        world.set_tile(x, y + 1, below);
        out.changed.push((x, y));
        out.changed.push((x, y + 1));

        self.wake(x, y + 1);
        // Emptying a tile lets its neighbours flow in behind it.
        self.wake(x - 1, y);
        self.wake(x + 1, y);
        if here.liquid > 0 {
            self.wake(x, y);
        }
        here.liquid == 0
    }

    /// Level sideways, across as many tiles as will take it.
    fn level(&mut self, world: &mut impl LiquidWorld, x: i32, y: i32, out: &mut Settled) {
        let here = world.tile(x, y);
        let kind = here.liquid_kind;
        // How far out the level reaches: a tile joins only if it is open, empty of anything else,
        // and reachable through the ones between.
        let mut reach = 0;
        for step in 1..=3 {
            if !open_for(world, x - step, y, kind) || !open_for(world, x + step, y, kind) {
                break;
            }
            // A tile past the first only joins if there is already liquid there to join with.
            if step > 1
                && (world.tile(x - step, y).liquid == 0 || world.tile(x + step, y).liquid == 0)
            {
                break;
            }
            reach = step;
        }
        // A nearly full tile does not level wide: it would only shove its own level about.
        if here.liquid > 250 {
            reach = reach.min(1);
        }

        if reach == 0 {
            self.level_one_side(world, x, y, out);
            return;
        }

        // Middle first, then alternating outward — the order the spare unit is handed out in
        // below. Building the tile list once here, in this order, and reusing it for both the
        // flatness/total check and the final write loop reads each tile once instead of up to
        // three times (the three separate `span`/`levels`/`order` passes this replaces each did
        // their own `world.tile` calls), and needs no heap allocation: `reach` is bounded to
        // 1..=3 here (0 already returned above), so at most 7 tiles, which fits on the stack.
        let mut positions = [0i32; 7];
        positions[0] = x;
        let mut n = 1usize;
        for step in 1..=reach {
            positions[n] = x - step;
            positions[n + 1] = x + step;
            n += 2;
        }
        let positions = &positions[..n];

        let mut levels = [0u8; 7];
        for (slot, &sx) in levels.iter_mut().zip(positions) {
            *slot = world.tile(sx, y).liquid;
        }
        let levels = &levels[..n];

        // Already level to within a drop: leave it alone. Without this the spare unit that
        // levelling cannot divide evenly gets handed back and forth between neighbours forever,
        // and a still pool costs as much as a flooding one.
        let flat = *levels.iter().max().unwrap() - *levels.iter().min().unwrap() <= 1;
        if flat && here.liquid >= 3 {
            return;
        }
        let mut total: i32 = levels.iter().map(|&l| i32::from(l)).sum();
        // A very thin film is allowed to lose its last drop rather than spreading forever, which
        // is what stops a puddle creeping across a whole cavern.
        if here.liquid < 3 {
            total -= 1;
        }

        // The share is floored and the remainder handed out from the middle outward. The game
        // rounds each tile independently, which quietly creates liquid every time a pool settles;
        // dividing exactly costs nothing visible and means a world cannot flood itself.
        let n_i32 = n as i32;
        let each = (total / n_i32).clamp(0, i32::from(FULL));
        let mut spare = (total - each * n_i32).max(0);

        for &sx in positions {
            let mut level = each;
            if spare > 0 && level < i32::from(FULL) {
                level += 1;
                spare -= 1;
            }
            let mut tile = world.tile(sx, y);
            if i32::from(tile.liquid) == level {
                continue;
            }
            if tile.liquid == 0 && level > 0 && tile.is_active() {
                out.drowned.push((sx, y));
            }
            tile.liquid = level as u8;
            tile.liquid_kind = kind;
            world.set_tile(sx, y, tile);
            out.changed.push((sx, y));
            self.wake(sx, y);
            self.wake(sx, y + 1);
        }
    }

    /// Nothing on both sides: pour into whichever single side will take it.
    fn level_one_side(&mut self, world: &mut impl LiquidWorld, x: i32, y: i32, out: &mut Settled) {
        let here = world.tile(x, y);
        let kind = here.liquid_kind;
        for side in [-1, 1] {
            if !open_for(world, x + side, y, kind) {
                continue;
            }
            let neighbour = world.tile(x + side, y);
            if here.liquid.abs_diff(neighbour.liquid) <= 1 && here.liquid >= 3 {
                continue;
            }
            let mut total = i32::from(here.liquid) + i32::from(neighbour.liquid);
            if here.liquid < 3 {
                total -= 1;
            }
            let each = (total / 2).clamp(0, i32::from(FULL));
            if each == i32::from(neighbour.liquid) {
                continue;
            }
            if neighbour.liquid == 0 && each > 0 && neighbour.is_active() {
                out.drowned.push((x + side, y));
            }
            // The leftover stays here, so nothing is created or destroyed on an odd total.
            let mut here = here;
            let mut neighbour = neighbour;
            neighbour.liquid = each as u8;
            neighbour.liquid_kind = kind;
            here.liquid = (total - each).clamp(0, i32::from(FULL)) as u8;
            world.set_tile(x, y, here);
            world.set_tile(x + side, y, neighbour);
            out.changed.push((x, y));
            out.changed.push((x + side, y));
            self.wake(x + side, y);
            self.wake(x + side, y + 1);
            return;
        }
    }

    /// Two different liquids touching. Returns whether something turned into a merge block.
    ///
    /// Transcribed from `Liquid.LiquidCheck` (`Liquid.cs:1234-1320`), which is not the pairwise
    /// "any neighbour of a different kind reacts" rule the old version of this function used.
    /// Left, right and above are checked together first: every one of them holding a different
    /// kind has its liquid **zeroed regardless of the amount** — vanilla does this before it ever
    /// looks at the total — and only once their combined amount reaches 24 does a merge block
    /// appear, planted at *this* tile and clearing this tile's own liquid too. A tile with nothing
    /// foreign on any of those three sides falls through to a second, separate check against the
    /// tile directly below: not summed with the other three (there is only the one), and its
    /// merge block lands *below* rather than here, with both tiles' liquid zeroed.
    ///
    /// Two things vanilla's own version of this checks that this does not, both simplifications
    /// rather than oversights: the merge-block placement is refused unless this tile (or, for the
    /// below case, the tile below) is inactive, standing in for vanilla's `!tile.active() ||
    /// tileObsidianKill[tile.type]` — this project has no `tileObsidianKill` table, so the handful
    /// of furniture types that check would additionally allow are not covered; and the below case
    /// skips vanilla's `IsAContainer`/cuttable-plant special cases entirely, requiring the tile
    /// below to simply be inactive.
    fn react(&mut self, world: &mut impl LiquidWorld, x: i32, y: i32, out: &mut Settled) -> bool {
        let here = world.tile(x, y);

        let mut foreign = 0i32;
        let (mut water, mut lava, mut honey, mut shimmer) = (false, false, false, false);
        let mut any_differs = false;
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            let mut side = world.tile(nx, ny);
            if side.liquid == 0 {
                continue;
            }
            match side.liquid_kind {
                Liquid::Water => water = true,
                Liquid::Lava => lava = true,
                Liquid::Honey => honey = true,
                Liquid::Shimmer => shimmer = true,
            }
            if side.liquid_kind != here.liquid_kind {
                any_differs = true;
                foreign += i32::from(side.liquid);
                side.liquid = 0;
                world.set_tile(nx, ny, side);
                out.changed.push((nx, ny));
                self.disturb(nx, ny);
            }
        }

        if any_differs {
            let (block, kind) = merge_result(here.liquid_kind, water, lava, honey, shimmer);
            if foreign < 24 || kind == here.liquid_kind || here.is_active() {
                return false;
            }
            world.set_tile(x, y, Tile::block(block));
            out.reacted.push((x, y, block));
            out.changed.push((x, y));
            self.disturb(x, y);
            return true;
        }

        // Nothing foreign to either side or above: the one check left is straight down, which
        // reacts on its own rather than being summed in with the other three.
        let below = world.tile(x, y + 1);
        if below.liquid == 0 || below.liquid_kind == here.liquid_kind || below.is_active() {
            return false;
        }
        if here.liquid < 24 {
            // Too little to make anything: it simply evaporates, matching `LiquidCheck`'s own
            // `tile5.liquid < 24` branch, which clears the source without creating a block.
            let mut cleared = here;
            cleared.liquid = 0;
            cleared.liquid_kind = Liquid::Water;
            world.set_tile(x, y, cleared);
            out.changed.push((x, y));
            return true;
        }
        let (water, lava, honey, shimmer) = match below.liquid_kind {
            Liquid::Water => (true, false, false, false),
            Liquid::Lava => (false, true, false, false),
            Liquid::Honey => (false, false, true, false),
            Liquid::Shimmer => (false, false, false, true),
        };
        let (block, kind) = merge_result(here.liquid_kind, water, lava, honey, shimmer);
        if kind == here.liquid_kind {
            return false;
        }
        world.set_tile(x, y + 1, Tile::block(block));
        let mut cleared = here;
        cleared.liquid = 0;
        world.set_tile(x, y, cleared);
        out.reacted.push((x, y + 1, block));
        out.changed.push((x, y));
        out.changed.push((x, y + 1));
        self.disturb(x, y);
        self.disturb(x, y + 1);
        true
    }
}

/// What a liquid becomes, and what block it makes, where it merges with another kind present
/// nearby — transcribed from `Liquid.GetLiquidMergeTypes` (`Liquid.cs:1386-1454`). The four checks
/// run in a fixed order and each can overwrite what an earlier one decided, exactly as vanilla's
/// own un-early-exiting sequence does: that is what makes shimmer win over honey, honey over lava,
/// and lava over water whenever more than one is present around the same tile at once.
fn merge_result(
    this_kind: Liquid,
    water: bool,
    lava: bool,
    honey: bool,
    shimmer: bool,
) -> (u16, Liquid) {
    let mut block = reaction::OBSIDIAN;
    let mut kind = this_kind;
    if this_kind != Liquid::Water && water {
        block = match this_kind {
            Liquid::Lava => reaction::OBSIDIAN,
            Liquid::Honey => reaction::HONEY_BLOCK,
            Liquid::Shimmer => reaction::AETHERIUM,
            Liquid::Water => unreachable!(),
        };
        kind = Liquid::Water;
    }
    if this_kind != Liquid::Lava && lava {
        block = match this_kind {
            Liquid::Water => reaction::OBSIDIAN,
            Liquid::Honey => reaction::CRISPY_HONEY,
            Liquid::Shimmer => reaction::AETHERIUM,
            Liquid::Lava => unreachable!(),
        };
        kind = Liquid::Lava;
    }
    if this_kind != Liquid::Honey && honey {
        block = match this_kind {
            Liquid::Water => reaction::HONEY_BLOCK,
            Liquid::Lava => reaction::CRISPY_HONEY,
            Liquid::Shimmer => reaction::AETHERIUM,
            Liquid::Honey => unreachable!(),
        };
        kind = Liquid::Honey;
    }
    if this_kind != Liquid::Shimmer && shimmer {
        block = reaction::AETHERIUM;
        kind = Liquid::Shimmer;
    }
    (block, kind)
}

/// Grass, jungle grass, mushroom grass and their evil counterparts around a lava tile burn away
/// every real tick that tile is processed — `Liquid.DelWater`'s own literal table
/// (`Liquid.cs:1552-1569`). Ordinary grasses (plain, corrupt, hallowed, crimson, and the two golf
/// variants) burn to dirt; the jungle grasses (plain, mushroom, and their evil-biome forms) burn
/// to mud, which is what a lava flow through an underground jungle actually leaves behind.
fn lava_burn(world: &mut impl LiquidWorld, x: i32, y: i32, out: &mut Settled) {
    for bx in x - 1..=x + 1 {
        for by in y - 1..=y + 1 {
            let tile = world.tile(bx, by);
            if !tile.is_active() {
                continue;
            }
            let burned = match tile.block {
                2 | 23 | 109 | 199 | 477 | 492 => 0u16,
                60 | 70 | 661 | 662 => 59,
                _ => continue,
            };
            let mut gone = tile;
            gone.block = burned;
            world.set_tile(bx, by, gone);
            out.changed.push((bx, by));
        }
    }
}

/// Whether a tile will take liquid of this kind.
fn open_for(world: &impl LiquidWorld, x: i32, y: i32, kind: Liquid) -> bool {
    if x < 1 || x >= world.width() - 1 {
        return false;
    }
    let tile = world.tile(x, y);
    !solid(tile) && (tile.liquid == 0 || tile.liquid_kind == kind)
}

/// Whether a tile holds liquid out. Platforms do not.
fn solid(tile: Tile) -> bool {
    tile.is_active()
        && terrustia_proto::tile_solid::solid(tile.block)
        && !terrustia_proto::tile_solid::solid_top(tile.block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A small world with an implicit stone floor and walls, so tests only place what they mean.
    struct Cave {
        tiles: HashMap<(i32, i32), Tile>,
        width: i32,
        height: i32,
    }

    impl Cave {
        fn new() -> Self {
            let mut tiles = HashMap::new();
            // A floor at y = 20, and walls at x = 0 and x = 39.
            for x in 0..40 {
                tiles.insert((x, 20), Tile::block(1));
            }
            for y in 0..21 {
                tiles.insert((0, y), Tile::block(1));
                tiles.insert((39, y), Tile::block(1));
            }
            Self {
                tiles,
                width: 40,
                height: 30,
            }
        }

        fn pour(&mut self, x: i32, y: i32, kind: Liquid, amount: u8) {
            let mut tile = Tile::AIR;
            tile.liquid = amount;
            tile.liquid_kind = kind;
            self.tiles.insert((x, y), tile);
        }

        fn liquid_at(&self, x: i32, y: i32) -> u8 {
            self.tile(x, y).liquid
        }

        fn total(&self) -> i32 {
            (0..self.width)
                .flat_map(|x| (0..self.height).map(move |y| (x, y)))
                .map(|(x, y)| i32::from(self.liquid_at(x, y)))
                .sum()
        }
    }

    impl LiquidWorld for Cave {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.tiles.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
        fn set_tile(&mut self, x: i32, y: i32, tile: Tile) {
            self.tiles.insert((x, y), tile);
        }
        fn width(&self) -> i32 {
            self.width
        }
        fn height(&self) -> i32 {
            self.height
        }
    }

    fn run(cave: &mut Cave, liquids: &mut Liquids, ticks: usize) {
        for _ in 0..ticks {
            liquids.tick(cave);
        }
    }

    /// Water in mid-air falls to the floor.
    #[test]
    fn water_falls() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        cave.pour(20, 5, Liquid::Water, FULL);
        liquids.disturb(20, 5);
        run(&mut cave, &mut liquids, 100);

        assert_eq!(cave.liquid_at(20, 5), 0, "nothing left in the air");
        assert!(cave.liquid_at(20, 19) > 0, "and something on the floor");
    }

    /// A column of water spreads into a pool rather than standing up.
    #[test]
    fn a_column_becomes_a_pool() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        for y in 10..20 {
            cave.pour(20, y, Liquid::Water, FULL);
        }
        for y in 10..20 {
            liquids.disturb(20, y);
        }
        run(&mut cave, &mut liquids, 400);

        let deepest = (1..39).map(|x| cave.liquid_at(x, 19)).max().unwrap_or(0);
        let wet = (1..39).filter(|x| cave.liquid_at(*x, 19) > 0).count();
        assert!(wet > 5, "it should have spread: {wet} tiles wide");
        assert!(deepest < FULL, "and not stayed a column: {deepest}");
    }

    /// Nothing is created or destroyed by settling.
    #[test]
    fn liquid_is_conserved() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        for y in 8..20 {
            for x in 15..25 {
                cave.pour(x, y, Liquid::Water, FULL);
            }
        }
        let before = cave.total();
        for y in 8..20 {
            for x in 15..25 {
                liquids.disturb(x, y);
            }
        }
        run(&mut cave, &mut liquids, 1000);
        let after = cave.total();
        // Levelling rounds, and a film under three is allowed to evaporate, so a little is lost —
        // but only a little, and never gained.
        assert!(after <= before, "liquid was created: {before} -> {after}");
        assert!(
            after > before * 95 / 100,
            "too much was lost: {before} -> {after}"
        );
    }

    /// A pool finds its level rather than sitting in a heap.
    #[test]
    fn a_pool_finds_its_level() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        for x in 10..30 {
            cave.pour(x, 19, Liquid::Water, if x < 12 { FULL } else { 0 });
        }
        for x in 10..30 {
            liquids.disturb(x, 19);
        }
        run(&mut cave, &mut liquids, 2000);

        let levels: Vec<u8> = (10..30).map(|x| cave.liquid_at(x, 19)).collect();
        let spread = levels.iter().max().unwrap() - levels.iter().min().unwrap();
        assert!(spread <= 3, "it should be level: {levels:?}");
    }

    /// Water on lava makes obsidian, and the pair is spent doing it.
    #[test]
    fn water_and_lava_make_obsidian() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        cave.pour(20, 19, Liquid::Lava, FULL);
        cave.pour(21, 19, Liquid::Water, FULL);
        liquids.disturb(20, 19);
        run(&mut cave, &mut liquids, 20);

        let made = cave.tile(20, 19);
        assert!(made.is_active(), "the lava should have set");
        assert_eq!(made.block, reaction::OBSIDIAN);
    }

    /// Honey on lava makes crispy honey; honey on water makes a honey block.
    #[test]
    fn honey_reacts_with_both() {
        for (other, expected) in [
            (Liquid::Lava, reaction::CRISPY_HONEY),
            (Liquid::Water, reaction::HONEY_BLOCK),
        ] {
            let mut cave = Cave::new();
            let mut liquids = Liquids::default();
            cave.pour(20, 19, other, FULL);
            cave.pour(21, 19, Liquid::Honey, FULL);
            liquids.disturb(20, 19);
            liquids.disturb(21, 19);
            run(&mut cave, &mut liquids, 40);
            let made = (19..22)
                .map(|x| cave.tile(x, 19))
                .find(|t| t.is_active())
                .expect("something should have set");
            assert_eq!(made.block, expected, "{other:?} and honey");
        }
    }

    /// Shimmer reacts with every other liquid, always making Aetherium and always surviving as
    /// shimmer itself — `Liquid.GetLiquidMergeTypes`'s own shimmer branch, which always runs last
    /// and so always wins. A side effect of porting the real merge-type cascade for L1 rather than
    /// a fix of its own (L4, MINOR): the old three-pair `product()` table had no notion of
    /// shimmer at all.
    #[test]
    fn shimmer_reacts_with_everything_into_aetherium() {
        for other in [Liquid::Water, Liquid::Lava, Liquid::Honey] {
            let mut cave = Cave::new();
            let mut liquids = Liquids::default();
            cave.pour(20, 19, other, FULL);
            cave.pour(21, 19, Liquid::Shimmer, FULL);
            liquids.disturb(20, 19);
            liquids.disturb(21, 19);
            run(&mut cave, &mut liquids, 40);
            let made = (19..22)
                .map(|x| cave.tile(x, 19))
                .find(|t| t.is_active())
                .unwrap_or_else(|| panic!("nothing reacted for {other:?}"));
            assert_eq!(made.block, reaction::AETHERIUM, "{other:?} and shimmer");
        }
    }

    /// Two of the same kind do not react at all.
    #[test]
    fn like_liquids_do_not_react() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        cave.pour(20, 19, Liquid::Water, FULL);
        cave.pour(21, 19, Liquid::Water, FULL);
        liquids.disturb(20, 19);
        run(&mut cave, &mut liquids, 40);
        assert!(!cave.tile(20, 19).is_active());
        assert!(!cave.tile(21, 19).is_active());
    }

    /// A small amount of foreign liquid on each side does not react at all — vanilla requires the
    /// *combined* foreign amount to reach 24, not merely "some contact" between two kinds.
    ///
    /// Fails on the code before this fix, which reacted to any two different liquids touching
    /// regardless of how little of either was actually there.
    #[test]
    fn small_amounts_of_foreign_liquid_do_not_react() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        cave.pour(20, 19, Liquid::Lava, 10);
        cave.pour(21, 19, Liquid::Water, 10);
        liquids.disturb(20, 19);
        liquids.disturb(21, 19);
        run(&mut cave, &mut liquids, 20);

        assert!(
            !cave.tile(20, 19).is_active() && !cave.tile(21, 19).is_active(),
            "10 and 10 together are under the 24 threshold, so nothing should have reacted"
        );
    }

    /// Lava dripping onto water directly below it reacts too, separately from the lateral check,
    /// and — same as the lateral case — lands its merge block on the *other* liquid's own tile.
    ///
    /// Fails on the code before this fix, which always planted the block on the lava's own tile
    /// regardless of which side the other liquid was on.
    #[test]
    fn lava_over_water_reacts_and_lands_on_the_waters_own_tile() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        cave.pour(20, 18, Liquid::Lava, FULL);
        cave.pour(20, 19, Liquid::Water, FULL);
        liquids.disturb(20, 18);
        liquids.disturb(20, 19);
        run(&mut cave, &mut liquids, 20);

        assert!(
            cave.tile(20, 19).is_active(),
            "the merge block should be on the water's own tile"
        );
        assert_eq!(cave.tile(20, 19).block, reaction::OBSIDIAN);
    }

    /// Liquid arriving on something active — a torch here — is reported, so a caller with the
    /// real per-type table can decide whether it dies.
    ///
    /// Fails on the code before this fix: `Settled::drowned` did not exist and nothing was ever
    /// reported when liquid reached an occupied tile.
    #[test]
    fn liquid_arriving_on_something_active_is_reported() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        // A torch standing where the water is about to land — active, but not solid, so liquid
        // still reaches it rather than being blocked outright.
        cave.set_tile(20, 19, Tile::framed(4, 0, 0));
        cave.pour(20, 5, Liquid::Water, FULL);
        liquids.disturb(20, 5);

        let mut reported = false;
        for _ in 0..200 {
            if liquids.tick(&mut cave).drowned.contains(&(20, 19)) {
                reported = true;
            }
        }
        assert!(
            reported,
            "liquid arriving on the torch should have been reported"
        );
    }

    /// Settled lava burns the grass and jungle grass around it into dirt and mud —
    /// `Liquid.DelWater`'s own literal table.
    ///
    /// Fails on the code before this fix: lava simply sat next to grass forever, with nothing
    /// converting it at all.
    #[test]
    fn settled_lava_burns_grass_and_jungle_grass_around_it() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        cave.tiles.insert((19, 19), Tile::block(2)); // ordinary grass, one side
        cave.tiles.insert((21, 19), Tile::block(60)); // jungle grass, the other side
        cave.pour(20, 19, Liquid::Lava, FULL);
        liquids.disturb(20, 19);
        run(&mut cave, &mut liquids, 40);

        assert_eq!(
            cave.tile(19, 19).block,
            0,
            "ordinary grass should have burned to dirt"
        );
        assert_eq!(
            cave.tile(21, 19).block,
            59,
            "jungle grass should have burned to mud"
        );
    }

    /// Lava creeps: it takes longer to cover the same ground than water does.
    #[test]
    fn lava_moves_slower_than_water() {
        let spread_after = |kind: Liquid, ticks: usize| {
            let mut cave = Cave::new();
            let mut liquids = Liquids::default();
            for y in 14..20 {
                cave.pour(20, y, kind, FULL);
            }
            for y in 14..20 {
                liquids.disturb(20, y);
            }
            run(&mut cave, &mut liquids, ticks);
            (1..39).filter(|x| cave.liquid_at(*x, 19) > 0).count()
        };
        let water = spread_after(Liquid::Water, 30);
        let lava = spread_after(Liquid::Lava, 30);
        assert!(lava < water, "lava {lava} should trail water {water}");
    }

    /// The queue is bounded however hard it is pushed.
    #[test]
    fn the_queue_cannot_grow_without_end() {
        let mut liquids = Liquids::default();
        for i in 0..(QUEUE_CAP * 2) {
            liquids.wake(i as i32 % 400, i as i32 % 400);
        }
        assert_eq!(liquids.pending(), QUEUE_CAP);
    }

    /// Still water is free: nothing to do means nothing queued.
    #[test]
    fn a_settled_pool_costs_nothing() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        for x in 10..30 {
            cave.pour(x, 19, Liquid::Water, 100);
        }
        for x in 10..30 {
            liquids.disturb(x, 19);
        }
        run(&mut cave, &mut liquids, 3000);
        assert_eq!(
            liquids.pending(),
            0,
            "a settled pool should stop waking itself"
        );
    }

    /// Liquid trapped inside a block is simply gone, rather than being stuck there forever.
    #[test]
    fn liquid_in_a_block_is_gone() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        let mut buried = Tile::block(1);
        buried.liquid = FULL;
        buried.liquid_kind = Liquid::Water;
        cave.set_tile(20, 10, buried);
        liquids.disturb(20, 10);
        run(&mut cave, &mut liquids, 10);
        assert_eq!(cave.liquid_at(20, 10), 0);
    }

    /// Pins `level()`'s exact per-tile output for a deterministic settle, tile by tile — not just
    /// an aggregate ("it spread", "it's flat"), which is all the tests above check. Nothing in
    /// this file uses randomness, so an exact assertion is the sharpest pin available: any change
    /// to read order, caching, or the middle-out spare-unit distribution that alters a single
    /// tile's result fails this immediately, which is exactly what a read/allocation change to
    /// `level` needs guarding against.
    #[test]
    fn an_uneven_five_wide_pool_settles_to_an_exact_pinned_shape() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        // Walls flanking the pour directly, so the pool is confined to exactly these 5 tiles
        // rather than spreading into open air further out — `Cave::new`'s own boundary walls at
        // x=0/39 are too far away to confine anything, as the first version of this test found
        // the hard way (350 units spread across most of the cave instead of just these 5 tiles).
        cave.set_tile(17, 19, Tile::block(1));
        cave.set_tile(23, 19, Tile::block(1));
        for (x, amount) in [(18, 40u8), (19, 40), (20, 251), (21, 10), (22, 10)] {
            cave.pour(x, 19, Liquid::Water, amount);
        }
        for x in 18..=22 {
            liquids.disturb(x, 19);
        }
        run(&mut cave, &mut liquids, 50);

        let settled: Vec<u8> = (18..=22).map(|x| cave.liquid_at(x, 19)).collect();
        assert_eq!(
            settled,
            // 351 does not divide evenly by 5 (70 remainder 1) — the odd unit lands on x=20, the
            // settle's own origin tile, because the middle-first ordering hands the spare out
            // there before it ever reaches a neighbour. This is exactly the property a read/order
            // regression in `level`'s tile list would break silently: two of the wrong three
            // Vecs agreeing on a value that happened to still add up right is a real risk with a
            // total this close to dividing evenly, which is why this seed is deliberately *not*
            // the more obviously-round 350 the first version of this test used.
            vec![70, 70, 71, 70, 70],
            "351 total across these 5 tiles, walled in on both sides, should settle 70 everywhere \
             except the one spare unit on the origin tile x=20 — if this changes, either level()'s \
             behavior changed or the fixed total above did"
        );
    }

    /// Not a correctness test — a manual timing run for `level`'s read/allocation reduction,
    /// against a realistic flooded-cavern load rather than a synthetic microbenchmark. Run with
    /// `cargo test --release -p terrustia --lib world::liquid::tests::time_a_large_settle -- \
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "manual timing run, not part of the normal suite"]
    fn time_a_large_settle() {
        let mut cave = Cave::new();
        cave.width = 200;
        cave.height = 50;
        for x in 0..200 {
            cave.tiles.insert((x, 49), Tile::block(1));
        }
        for y in 0..50 {
            cave.tiles.insert((0, y), Tile::block(1));
            cave.tiles.insert((199, y), Tile::block(1));
        }
        let mut liquids = Liquids::default();
        for x in 5..195 {
            for y in 5..40 {
                cave.pour(x, y, Liquid::Water, 200);
            }
        }
        for x in 5..195 {
            for y in 5..40 {
                liquids.disturb(x, y);
            }
        }
        let start = std::time::Instant::now();
        for _ in 0..300 {
            liquids.tick(&mut cave);
        }
        let elapsed = start.elapsed();
        println!("300 ticks over a 190x35 flooded pool: {elapsed:?}");
    }
}
