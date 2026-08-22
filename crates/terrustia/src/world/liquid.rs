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
        self.queue.push_back((x, y));
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

        let span: Vec<i32> = (x - reach..=x + reach).collect();
        // Already level to within a drop: leave it alone. Without this the spare unit that
        // levelling cannot divide evenly gets handed back and forth between neighbours forever,
        // and a still pool costs as much as a flooding one.
        let levels: Vec<u8> = span.iter().map(|&sx| world.tile(sx, y).liquid).collect();
        let flat = levels.iter().max().copied().unwrap_or(0)
            - levels.iter().min().copied().unwrap_or(0)
            <= 1;
        if flat && here.liquid >= 3 {
            return;
        }
        let mut total: i32 = span
            .iter()
            .map(|&sx| i32::from(world.tile(sx, y).liquid))
            .sum();
        // A very thin film is allowed to lose its last drop rather than spreading forever, which
        // is what stops a puddle creeping across a whole cavern.
        if here.liquid < 3 {
            total -= 1;
        }

        // The share is floored and the remainder handed out from the middle outward. The game
        // rounds each tile independently, which quietly creates liquid every time a pool settles;
        // dividing exactly costs nothing visible and means a world cannot flood itself.
        let n = span.len() as i32;
        let each = (total / n).clamp(0, i32::from(FULL));
        let mut spare = (total - each * n).max(0);
        // Middle first, then alternating outward, so the extra sits under the source.
        let mut order: Vec<i32> = vec![x];
        for step in 1..=reach {
            order.push(x - step);
            order.push(x + step);
        }

        for sx in order {
            let mut level = each;
            if spare > 0 && level < i32::from(FULL) {
                level += 1;
                spare -= 1;
            }
            let mut tile = world.tile(sx, y);
            if i32::from(tile.liquid) == level {
                continue;
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

    /// Two different liquids touching. Returns whether something turned to stone.
    fn react(&mut self, world: &mut impl LiquidWorld, x: i32, y: i32, out: &mut Settled) -> bool {
        let here = world.tile(x, y);
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (nx, ny) = (x + dx, y + dy);
            let other = world.tile(nx, ny);
            if other.liquid == 0 || other.liquid_kind == here.liquid_kind {
                continue;
            }
            let Some(block) = product(here.liquid_kind, other.liquid_kind) else {
                continue;
            };
            // The lava is what turns to stone; the other liquid is spent doing it.
            let (sx, sy) = if here.liquid_kind == Liquid::Lava {
                (x, y)
            } else {
                (nx, ny)
            };
            let (ox, oy) = if (sx, sy) == (x, y) { (nx, ny) } else { (x, y) };

            world.set_tile(sx, sy, Tile::block(block));
            let mut spent = world.tile(ox, oy);
            spent.liquid = spent.liquid.saturating_sub(FULL / 2);
            world.set_tile(ox, oy, spent);
            out.reacted.push((sx, sy, block));
            out.changed.push((ox, oy));
            self.disturb(sx, sy);
            self.disturb(ox, oy);
            return true;
        }
        false
    }
}

/// What two liquids make where they meet. `None` means they simply do not react.
fn product(a: Liquid, b: Liquid) -> Option<u16> {
    let pair = |x: Liquid, y: Liquid| (a == x && b == y) || (a == y && b == x);
    if pair(Liquid::Water, Liquid::Lava) {
        Some(reaction::OBSIDIAN)
    } else if pair(Liquid::Honey, Liquid::Lava) {
        Some(reaction::CRISPY_HONEY)
    } else if pair(Liquid::Honey, Liquid::Water) {
        Some(reaction::HONEY_BLOCK)
    } else {
        None
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
}
