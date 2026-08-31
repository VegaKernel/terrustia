//! Water, lava and honey, and what they do when nothing is holding them up.
//!
//! Liquid does not simulate everywhere. It simulates where something has just changed — a tile
//! mined, a bucket poured, a neighbour that moved — and each change wakes its neighbours, so a
//! disturbance spreads outward and then stops. That is what makes it affordable: a still ocean
//! costs nothing at all.
//!
//! What is waiting is a *set*, not a bag: a tile is either pending or it is not, tracked by one bit
//! per tile, exactly as vanilla's `checkingLiquid` flag makes `Main.liquid[]` hold each cell at most
//! once (`Liquid.cs:1172,1181` on add, `:1136,:1587` on service). A tile also wakes the tile
//! **above** it whenever its own amount changes (`Liquid.cs:947-966`), which is the only thing that
//! tells a column its floor has just dropped away.
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

/// The most tiles the simulation will touch in one tick, with nobody connected.
///
/// Vanilla's own per-pass slice is `curMaxLiquid / cycles` (`Liquid.cs:1073-1091`), and both halves
/// of it move with the player count, so see [`budget_for`]: this is that expression at zero
/// players.
pub const BUDGET: usize = 2_500;

/// Vanilla's per-pass liquid slice for a given number of connected players.
///
/// `UpdateLiquid` recomputes both terms every pass on a dedicated server (`Liquid.cs:993-1012`):
///
/// ```text
/// cycles        = 10 + players / 3
/// curMaxLiquid  = 25000 - players * 250
/// ```
///
/// and then works through `curMaxLiquid / cycles` cells of the active set per pass
/// (`Liquid.cs:1073-1077`). The player loop only counts the first 15 slots, so the count saturates
/// there: the slice runs from 2,500 at nobody connected down to 1,416 at fifteen or more. Liquid
/// gets *less* of the frame as the server gets busier, which is the point.
pub fn budget_for(players: usize) -> usize {
    let players = players.min(15);
    (25_000 - players * 250) / (10 + players / 3)
}

/// The most tiles that may be waiting at once.
///
/// A ceiling on memory, not on correctness. Each tile is in the queue at most once (see
/// [`Liquids::wake`]), so this can only bite when that many *distinct* tiles are moving at the same
/// instant, which a 40,000-tile release measured at 51,619. Dropping a wake here would strand
/// liquid with air beneath it and nothing left to wake it, so the number is set where the
/// simulation cannot reach it rather than where it is merely tidy.
pub const QUEUE_CAP: usize = 1_000_000;

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

/// The set of tiles whose liquid may still need to move, in the order they were woken.
#[derive(Debug, Default)]
pub struct Liquids {
    queue: VecDeque<(i32, i32)>,
    /// A per-tile delay, so lava and honey creep rather than run.
    settling: std::collections::HashMap<(i32, i32), u8>,
    /// Tiles that just received a fall and must sit out the next pass — `Tile.skipLiquid`, set at
    /// `Liquid.cs:588-589` and consumed at `Liquid.cs:1105-1112`. This is half of what keeps liquid
    /// from running several tiles a tick (the other half is the every-other-tick `skipCount` gate
    /// in `tick_liquids`); a fall marks the tile it landed on and the tile it left, so a column
    /// advances one tile every second pass rather than every pass (L3-09).
    skip: std::collections::HashSet<(i32, i32)>,
    /// One bit per tile, set exactly while that tile is in `queue`. This is vanilla's
    /// `checkingLiquid` flag (`Liquid.cs:1172,1181,1136,1587`), which is what keeps `Main.liquid[]`
    /// holding each
    /// cell at most once. A bitset rather than a hash set: the membership test is on the hot path
    /// (a single `level` can wake fourteen tiles), and one word per 64 tiles is 630 KB for a
    /// 4200x1200 world, which is cheaper than the hashing would be.
    queued: Vec<u64>,
    /// The world `queued` is indexed against, learned on the first [`Liquids::tick`].
    width: i32,
    height: i32,
    /// Connected players, for [`budget_for`].
    players: usize,
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

    /// Wake one tile, unless it is already waiting.
    ///
    /// `Liquid.AddWater` refuses a cell whose `checkingLiquid` flag is set (`Liquid.cs:1172`) and
    /// sets it on the ones it takes (`:1181`), so a cell sits in `Main.liquid[]` at most once. That
    /// dedup is not an optimisation: without it a single `level` emits up to fourteen wakes while
    /// consuming one slot of the pass budget, the queue grows faster than it drains, and the only
    /// thing that ends the flood is the cap silently discarding wakes, which is what leaves water
    /// hanging in mid-air with nothing left to wake it. Measured on a probe over the unfixed code:
    /// a 6,000-tile release peaked at 69,398 entries against 7,894 with the dedup, serviced the
    /// same cells 18.7 times over, and stranded 1,511 tiles it could no longer reach.
    ///
    /// First wake wins, matching vanilla: a tile already waiting keeps its place in the order
    /// rather than being pushed to the back.
    pub fn wake(&mut self, x: i32, y: i32) {
        if self.width <= 0 {
            // No tick has run yet, so there is no world to index a bit by. These are re-run
            // through here, and so deduped and capped, the moment `tick` learns the dimensions;
            // the cap is deliberately not applied to them here, because a bulk pre-load (worldgen's
            // settle pass wakes every liquid tile in the world before it ticks once) must not be
            // truncated before it has had its chance to collapse to distinct tiles.
            self.queue.push_back((x, y));
            return;
        }
        // Outside the world there is nothing to settle: `settle` drops these on sight, so keeping
        // them would only cost a queue slot.
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let bit = y as usize * self.width as usize + x as usize;
        let (word, mask) = (bit / 64, 1u64 << (bit % 64));
        if self.queued[word] & mask != 0 || self.queue.len() >= QUEUE_CAP {
            return;
        }
        self.queued[word] |= mask;
        self.queue.push_back((x, y));
    }

    /// Clear a tile's pending bit as it leaves the queue: `DelWater`'s own
    /// `checkingLiquid(false)` (`Liquid.cs:1587`).
    fn unmark(&mut self, x: i32, y: i32) {
        if self.width <= 0 || x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let bit = y as usize * self.width as usize + x as usize;
        self.queued[bit / 64] &= !(1u64 << (bit % 64));
    }

    /// Point the pending bitset at this world, folding anything already waiting back through
    /// [`Liquids::wake`] so it is deduped and bounds-checked like everything after it.
    ///
    /// Called at the top of every [`Liquids::tick`] and does nothing once the dimensions match, so
    /// the cost is one comparison a tick. A world that changes size (a test that grows its cave, a
    /// server that loads a different world) rebuilds from the queue, which is the only authority on
    /// what is waiting.
    fn size_to(&mut self, world: &impl LiquidWorld) {
        let (width, height) = (world.width(), world.height());
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        let bits = (width.max(0) as usize) * (height.max(0) as usize);
        self.queued.clear();
        self.queued.resize(bits.div_ceil(64), 0);
        let waiting = std::mem::take(&mut self.queue);
        for (x, y) in waiting {
            self.wake(x, y);
        }
    }

    /// How many players are connected, which is what sizes the per-pass budget
    /// (see [`budget_for`]).
    pub fn set_player_count(&mut self, players: usize) {
        self.players = players;
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
        self.size_to(world);
        let mut out = Settled::default();
        let generation = self.queue.len().min(budget_for(self.players));
        for _ in 0..generation {
            let Some((x, y)) = self.queue.pop_front() else {
                break;
            };
            self.unmark(x, y);
            // `Tile.skipLiquid`: a tile that took a fall last pass sits this one out and clears the
            // flag, exactly as `UpdateLiquid` does (`Liquid.cs:1105-1112`). It goes straight back
            // into the queue for the pass after, so the tile is only delayed, never dropped.
            if self.skip.remove(&(x, y)) {
                self.wake(x, y);
                continue;
            }
            self.settle(world, x, y, &mut out);
        }
        out
    }

    /// Record a tile whose liquid amount just changed, and wake the tile above it.
    ///
    /// `Liquid.Update`'s own tail (`Liquid.cs:947-966`): a pass that leaves a tile holding a
    /// different amount than it started with calls `AddWater(x, y - 1)`, and `DelWater` does the
    /// same for a tile that is no longer nearly full (`Liquid.cs:1518-1521`). Nothing else in the
    /// simulation propagates upward: `fall` tells the tile below and the two beside it, and `level`
    /// tells the tiles it wrote and what is under them. So without this, a column that drains from
    /// the bottom never learns its floor has gone, goes quiet, and hangs there. Measured on a probe
    /// over the unfixed code: a 6,000-tile release down an open shaft left 1,511 tiles in mid-air
    /// with an empty queue, against none once this wake exists.
    fn changed(&mut self, out: &mut Settled, x: i32, y: i32) {
        out.changed.push((x, y));
        self.wake(x, y - 1);
    }

    /// A tile with no liquid left has nothing to remember, so drop its per-tile state.
    fn forget(&mut self, x: i32, y: i32) {
        self.settling.remove(&(x, y));
        self.skip.remove(&(x, y));
    }

    /// Settle one tile.
    fn settle(&mut self, world: &mut impl LiquidWorld, x: i32, y: i32, out: &mut Settled) {
        // Never touch anything outside the world.
        if x < 0 || y < 0 || x >= world.width() || y >= world.height() {
            return;
        }
        let here = world.tile(x, y);
        if here.liquid == 0 {
            self.forget(x, y);
            return;
        }
        // Liquid inside a solid block is not liquid any more. This is a data-integrity cleanup, not
        // a flow, so it runs regardless of how close to the border the tile sits — a generated
        // world must not keep liquid trapped in rock even at its edges.
        if solid(here) {
            let mut cleared = here;
            cleared.liquid = 0;
            world.set_tile(x, y, cleared);
            self.changed(out, x, y);
            self.forget(x, y);
            return;
        }
        // L3-17: liquid refuses to *flow* within five tiles of the world border — `Liquid.AddWater`'s
        // own bounds test (`Liquid.cs:1172`), the gate every woken tile passes through in vanilla.
        if x < 5 || y < 5 || x >= world.width() - 5 || y >= world.height() - 5 {
            return;
        }

        // L3-07: below the underworld layer, water boils away two units a pass (`Liquid.cs:468-
        // 476`). Only water (liquid type 0) evaporates; lava and honey down here are left alone.
        // `UnderworldLayer` is `maxTilesY - 200`; a world too short to have one (every real world
        // is far taller than 200, but a unit-test one is not) has no underworld and no boiling.
        let underworld = world.height() - 200;
        if here.liquid_kind == Liquid::Water && underworld > 0 && y > underworld {
            let gone = here.liquid.min(2);
            let mut evaporated = here;
            evaporated.liquid -= gone;
            world.set_tile(x, y, evaporated);
            self.changed(out, x, y);
            if evaporated.liquid == 0 {
                self.forget(x, y);
                return;
            }
            // It keeps boiling on the following passes until it is gone.
            self.wake(x, y);
        }

        let here = world.tile(x, y);
        // L3-08: only lava, honey and shimmer initiate a merge — `LavaCheck`/`HoneyCheck`/
        // `ShimmerCheck` are the sole `LiquidCheck` entry points (`Liquid.cs:482-566,1456-1479`).
        // A water tile with a different liquid beside it does not react itself; it only wakes that
        // neighbour so the neighbour reacts on its own turn, which is why obsidian forms on the
        // lava's tile rather than the water's.
        if here.liquid_kind == Liquid::Water {
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let n = world.tile(x + dx, y + dy);
                if n.liquid > 0 && n.liquid_kind != Liquid::Water {
                    self.wake(x + dx, y + dy);
                }
            }
        } else if self.react(world, x, y, out) {
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
                self.wake(x, y);
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
        self.changed(out, x, y);
        // The tile below gained: what sits above *it* is this tile, which the wake calls just
        // below already cover, so it needs no upward wake of its own.
        out.changed.push((x, y + 1));

        // `Tile.skipLiquid(true)` on both the tile that received the fall and the one it left
        // (`Liquid.cs:588-589`): each sits out the next pass, so a column advances one tile every
        // second pass rather than every pass (L3-09).
        self.skip.insert((x, y + 1));
        self.skip.insert((x, y));

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
    ///
    /// The averaging span itself is the simplified model's own: it spreads across a symmetric run
    /// of up to seven tiles and divides exactly (the remainder handed out from the middle outward,
    /// below), which the module was deliberately built to do so a pool converges and conserves
    /// rather than the way a literal port of `Liquid.Update`'s per-tile `Math.Round` levelling
    /// behaves. That faithful port was measured, as a whole unit with vanilla's `kill`/`DelWater`/
    /// `stuckCount` machinery, in `world/liquid_faithful.rs`: it converges without draining or
    /// thrashing, but it is not conservative — its `Math.Round` *creates* water (the L3-12
    /// duplication), so it cannot meet the "no creation" criterion this model holds to. That
    /// measured non-conservation is the documented seam for L3-11 (the asymmetric 4-tile case) and
    /// L3-12 (the rounding); this model keeps exact division instead.
    ///
    /// Nothing here destroys liquid. This model used to carry a narrowed thin-film drain (a film
    /// under three units lost a unit a pass, standing in for vanilla's fuller `DelWater` drain of
    /// every 2..19 film with an outlet, `Liquid.cs:1510-1516`), on the stated grounds that it was
    /// what stopped a puddle creeping across a whole cavern forever. It was not: creep is a CPU
    /// cost, not a correctness one, and it was expensive only because every touched tile re-woke
    /// itself with no dedup. It also could not be reached by the "already flat, leave it alone"
    /// return just below, which was gated on `>= 3`, so a *stable* film bled a unit a pass until it
    /// was gone. That was the entire measured conservation loss: 4.0% on a 50-tile release down an
    /// open shaft and 44.1% on a 200-tile one. With the flat check applying at any level, a film
    /// reaches a rest state instead, stops changing, stops waking, and stays. Measured across
    /// eight releases from 50 to 40,000 tiles, walled and open: exactly zero loss.
    ///
    /// The `<= 1` tolerance in that flat check is what bounds the creep: a film of one unit beside
    /// an empty tile is already flat to within a drop and does not spread at all.
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

        // Middle first, then alternating outward, which is the order that breaks ties when the
        // spare unit is handed out below. Building the tile list once here, and reusing it for
        // both the total and the final write loop reads each tile once instead of up to three times
        // (the three separate `span`/`levels`/`order` passes this replaces each did their
        // own `world.tile` calls), and needs no heap allocation: `reach` is bounded to
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

        let total: i32 = levels.iter().map(|&l| i32::from(l)).sum();

        // The share is floored and the remainder handed to whichever tiles already hold the most.
        // The game rounds each tile independently, which quietly creates liquid every time a pool
        // settles; dividing exactly costs nothing visible and means a world cannot flood itself.
        //
        // Who gets the spare is what makes a still pool free. Handing it out from the middle
        // outward, as this used to, means a span that is *already* one unit high somewhere has that
        // unit taken off its neighbour and put back on the centre every single pass: the tile
        // changes, so it wakes, so it is levelled again, forever. Giving it to the tile that has it
        // already makes such a span a fixed point that writes nothing and wakes nothing. It also
        // removes the need for the "flat to within a drop, leave it alone" tolerance that stood in
        // for this before, which is what used to leave a long shallow pool resting on a gradient
        // (measured: a 20-tile pool settling to 19 at one end and 15 at the other) rather than
        // actually level.
        let n_i32 = n as i32;
        let each = (total / n_i32).clamp(0, i32::from(FULL));
        let spare = (total - each * n_i32).max(0);
        let mut extra = [false; 7];
        // At most seven slots and at most six spare units, so a scan is cheaper than a sort and
        // needs no allocation. Ties go to the earliest slot, which is the middle-first order the
        // positions were built in.
        for _ in 0..spare {
            let mut best = usize::MAX;
            for i in 0..n {
                if !extra[i] && (best == usize::MAX || levels[i] > levels[best]) {
                    best = i;
                }
            }
            if best == usize::MAX {
                break;
            }
            extra[best] = true;
        }

        for (i, &sx) in positions.iter().enumerate() {
            let level = (each + i32::from(extra[i])).min(i32::from(FULL));
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
            // The whole neighbourhood, not just this tile and what is under it. Vanilla keeps a
            // cell in `Main.liquid[]` for another `10 + players/3` passes after it stops changing
            // (the `kill` counter, `Liquid.cs:963,1096-1101`), which is how news of a *later*
            // change beside it still reaches it. This simulation drops a tile the instant it writes
            // nothing, so without telling the neighbours directly a pool comes to rest on a
            // gradient: the centre of a span often writes nothing while its neighbours move, so it
            // is never woken again and never learns its own window has changed underneath it.
            self.disturb(sx, y);
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
            if here.liquid.abs_diff(neighbour.liquid) <= 1 {
                continue;
            }
            let total = i32::from(here.liquid) + i32::from(neighbour.liquid);
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
            self.disturb(x, y);
            self.disturb(x + side, y);
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
            self.changed(out, x, y);
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
            Liquid::Water => unreachable!("guarded by this_kind != Water just above"),
        };
        kind = Liquid::Water;
    }
    if this_kind != Liquid::Lava && lava {
        block = match this_kind {
            Liquid::Water => reaction::OBSIDIAN,
            Liquid::Honey => reaction::CRISPY_HONEY,
            Liquid::Shimmer => reaction::AETHERIUM,
            Liquid::Lava => unreachable!("guarded by this_kind != Lava just above"),
        };
        kind = Liquid::Lava;
    }
    if this_kind != Liquid::Honey && honey {
        block = match this_kind {
            Liquid::Water => reaction::HONEY_BLOCK,
            Liquid::Lava => reaction::CRISPY_HONEY,
            Liquid::Shimmer => reaction::AETHERIUM,
            Liquid::Honey => unreachable!("guarded by this_kind != Honey just above"),
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
    if x < 5 || x >= world.width() - 5 {
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

    /// L3-07: below the underworld layer water boils away, while water at ordinary depths does not
    /// (`Liquid.cs:468-476`).
    ///
    /// Fails before the fix: `settle` had no depth check, so a pocket of water deep in the
    /// underworld lasted exactly as long as any other.
    #[test]
    fn underworld_water_boils_away() {
        let mut cave = Cave::new();
        cave.height = 400; // tall enough to have an underworld at `height - 200 = 200`.
        // Two fully boxed pockets: one below the underworld line, one well above it.
        for (bx, by) in [(20, 350), (20, 100)] {
            cave.tiles.insert((bx - 1, by), Tile::block(1));
            cave.tiles.insert((bx + 1, by), Tile::block(1));
            cave.tiles.insert((bx, by - 1), Tile::block(1));
            cave.tiles.insert((bx, by + 1), Tile::block(1));
            cave.pour(bx, by, Liquid::Water, 100);
        }
        let mut liquids = Liquids::default();
        liquids.disturb(20, 350);
        liquids.disturb(20, 100);
        run(&mut cave, &mut liquids, 200);

        assert_eq!(
            cave.liquid_at(20, 350),
            0,
            "the underworld pocket should have boiled dry"
        );
        assert_eq!(
            cave.liquid_at(20, 100),
            100,
            "but the ordinary-depth pocket should be untouched"
        );
    }

    /// L3-08: a water tile touching lava does not merge on its own — only lava, honey and shimmer
    /// call `LiquidCheck`, so the obsidian lands on the lava's tile, not the water's
    /// (`Liquid.cs:497-566`).
    ///
    /// Fails before the fix: `settle` ran the merge for every tile, so a water tile processed
    /// before its lava neighbour turned *itself* to obsidian.
    #[test]
    fn water_does_not_merge_on_its_own_tile() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        cave.pour(20, 19, Liquid::Lava, FULL);
        cave.pour(21, 19, Liquid::Water, FULL);
        // A wall boxes the water against the lava so it cannot level away before the lava reacts.
        cave.set_tile(22, 19, Tile::block(1));
        // Disturb only the water tile, so it is the one processed first: before the fix it reacted
        // there and then, planting obsidian on its own tile.
        liquids.disturb(21, 19);
        run(&mut cave, &mut liquids, 40);

        assert_eq!(
            cave.tile(20, 19).block,
            reaction::OBSIDIAN,
            "obsidian should be on the lava's own tile"
        );
        assert!(
            !cave.tile(21, 19).is_active(),
            "and never on the water's own tile"
        );
    }

    /// L3-17: liquid refuses to run within five tiles of the world border — `Liquid.AddWater`'s
    /// own bounds test (`Liquid.cs:1172`).
    ///
    /// Fails before the fix: the guard was one tile, so a full tile at x=2 still fell away.
    #[test]
    fn liquid_does_not_run_within_five_of_the_border() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        // x=2 is inside the five-tile margin; there is open air all the way down to the floor.
        cave.pour(2, 5, Liquid::Water, FULL);
        liquids.disturb(2, 5);
        run(&mut cave, &mut liquids, 50);
        assert_eq!(
            cave.liquid_at(2, 5),
            FULL,
            "liquid this close to the edge should stay frozen"
        );
        assert_eq!(
            cave.liquid_at(2, 6),
            0,
            "and nothing below it should have moved"
        );
    }

    /// L3-09: a falling column advances one tile every second pass, not every pass — the
    /// `skipLiquid` flag (`Liquid.cs:588-589,1105-1112`), which is half of what keeps liquid from
    /// running roughly four times too fast (the every-other-tick `skipCount` gate, tested at the
    /// server level, is the other half).
    ///
    /// Fails before the fix: with no skip flag a column fell a tile every pass, reaching this far
    /// down in half the passes.
    #[test]
    fn skip_liquid_halves_the_fall_rate() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        // y=6 is clear of the five-tile top margin; the floor is at y=20.
        cave.pour(20, 6, Liquid::Water, FULL);
        liquids.disturb(20, 6);
        run(&mut cave, &mut liquids, 10);
        let deepest = (6..20)
            .filter(|y| cave.liquid_at(20, *y) > 0)
            .max()
            .unwrap_or(6);
        assert!(
            deepest <= 12,
            "ten passes should carry the column only about five tiles with the skip flag, \
             but it reached y={deepest}"
        );
        // And it does still get there in the end.
        run(&mut cave, &mut liquids, 100);
        assert!(
            (1..39).any(|x| cave.liquid_at(x, 19) > 0),
            "the column should reach the floor eventually"
        );
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
        assert_eq!(cave.total(), before, "settling is exactly conservative");
    }

    /// A column falling a long way conserves every drop, at every fall distance.
    ///
    /// This is the shape the two older conservation tests could not see: they used 40x30 worlds,
    /// pools of at most 180 tiles and *zero* fall distance. A fall down a shaft wide enough to
    /// level in leaves a thin remainder behind on the way, and each of those remainders used to
    /// bleed a unit a pass until it was gone. The shaft has to be more than one tile wide for that
    /// to happen at all: a single-tile column moves its whole 255 down each time and leaves no film
    /// to lose, which is why a narrower version of this test passes against the bug.
    ///
    /// Fails before the fix, measured: 20, 33 and 33 units lost of 3,825 (0.52%, 0.86%, 0.86%) at
    /// the three distances.
    #[test]
    fn a_long_fall_loses_nothing() {
        for drop in [5i32, 40, 200] {
            // Tall enough that the floor clears the underworld boil band at `height - 200`, which
            // destroys water on purpose and would look like a conservation bug.
            let height = drop + 260;
            let floor = drop + 20;
            let mut cave = Cave::new();
            cave.height = height;
            cave.width = 60;
            for x in 0..60 {
                cave.tiles.insert((x, floor), Tile::block(1));
            }
            // A five-wide shaft: the water levels as well as falls, and levelling is what leaves
            // the films behind.
            for y in 0..=floor {
                cave.tiles.insert((27, y), Tile::block(1));
                cave.tiles.insert((33, y), Tile::block(1));
            }
            let mut liquids = Liquids::default();
            for x in 28..=32 {
                for y in 10..13 {
                    cave.pour(x, y, Liquid::Water, FULL);
                    liquids.disturb(x, y);
                }
            }
            let total = |cave: &Cave| -> i32 {
                (0..60)
                    .flat_map(|x| (0..height).map(move |y| (x, y)))
                    .map(|(x, y)| i32::from(cave.liquid_at(x, y)))
                    .sum()
            };
            let before = total(&cave);
            run(&mut cave, &mut liquids, (drop as usize + 40) * 8);
            assert_eq!(total(&cave), before, "a {drop}-tile fall lost liquid");
        }
    }

    /// A tile that drains wakes the tile above it, so a column follows its own floor down instead
    /// of hanging in the air once its neighbours go quiet (`Liquid.cs:947-966`).
    ///
    /// Fails before the fix: nothing in the simulation ever woke a tile above, so only the tiles
    /// disturbed at the start ever moved. The block detached one row at a time and the rest was
    /// left strung out down the shaft with an empty queue and no way back.
    #[test]
    fn a_draining_tile_wakes_the_one_above_it() {
        let mut cave = Cave::new();
        cave.height = 400;
        cave.width = 60;
        for x in 0..60 {
            cave.tiles.insert((x, 180), Tile::block(1));
        }
        // A one-tile shaft, so the only thing that can happen is falling: no sideways spreading to
        // muddy what the test is about.
        for y in 0..=180 {
            cave.tiles.insert((29, y), Tile::block(1));
            cave.tiles.insert((31, y), Tile::block(1));
        }
        let mut liquids = Liquids::default();
        // A stack of ten, so only the bottom one can fall on the first pass: every tile above it
        // has to be told, in turn, that its own floor has gone.
        for y in 20..30 {
            cave.pour(30, y, Liquid::Water, FULL);
            liquids.disturb(30, y);
        }
        run(&mut cave, &mut liquids, 2_000);

        assert_eq!(liquids.pending(), 0, "it should have come to rest");
        for y in 20..170 {
            assert_eq!(
                cave.liquid_at(30, y),
                0,
                "nothing should still be hanging at y={y}"
            );
        }
        assert_eq!(
            (170..180)
                .map(|y| cave.liquid_at(30, y))
                .collect::<Vec<_>>(),
            vec![FULL; 10],
            "all ten should be resting on the floor"
        );
    }

    /// A one-unit film is already level with the dry tile beside it, so it stays put rather than
    /// spreading. That `<= 1` tolerance, not a drain, is what stops a puddle creeping forever.
    #[test]
    fn a_single_drop_neither_spreads_nor_evaporates() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        cave.pour(20, 19, Liquid::Water, 1);
        liquids.disturb(20, 19);
        run(&mut cave, &mut liquids, 500);

        assert_eq!(cave.liquid_at(20, 19), 1, "the drop should still be there");
        assert_eq!(cave.total(), 1, "and it should not have spread");
        assert_eq!(liquids.pending(), 0, "and should cost nothing to hold");
    }

    /// The per-pass slice tracks vanilla's `curMaxLiquid / cycles` (`Liquid.cs:993-1012`), which
    /// shrinks as players connect and saturates at fifteen.
    #[test]
    fn the_budget_matches_vanillas_own_slice() {
        assert_eq!(budget_for(0), 2_500);
        assert_eq!(budget_for(0), BUDGET);
        // 25000 - 3*250 = 24250, cycles = 10 + 1 = 11.
        assert_eq!(budget_for(3), 24_250 / 11);
        // 25000 - 15*250 = 21250, cycles = 10 + 5 = 15.
        assert_eq!(budget_for(15), 21_250 / 15);
        assert_eq!(budget_for(255), budget_for(15), "vanilla counts 15 slots");
    }

    /// A pool finds its level rather than sitting in a heap, and keeps every drop doing it.
    ///
    /// "Level" here is the per-tile property, not a global one: no two tiles next to each other
    /// differ by more than a unit. Exact integer averaging over a bounded window cannot promise
    /// more than that, and should not be asked to. A run of thirty tiles holding 510 units has no
    /// flat integer answer that every seven-tile window also agrees with, so it comes to rest on a
    /// gradient of about a unit per window width, which vanilla only avoids by rounding each tile
    /// independently and thereby creating and destroying water (the deliberate divergence pinned by
    /// `liquid_faithful`). Worldgen irons the residual out globally with its own repeated sweeps
    /// (`worldgen/liquid_settle.rs`).
    ///
    /// This used to assert a spread of 3 across x=10..30 and got it for two accidental reasons: the
    /// thin-film drain had quietly eaten six units off the shallow end, and that sub-range happened
    /// to miss both ends of the pool. Measured across the whole pool, the code before this fix
    /// settled to the same 1-per-window gradient as this one, six units lighter.
    #[test]
    fn a_pool_finds_its_level() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        for x in 10..30 {
            cave.pour(x, 19, Liquid::Water, if x < 12 { FULL } else { 0 });
        }
        let before = cave.total();
        for x in 10..30 {
            liquids.disturb(x, 19);
        }
        run(&mut cave, &mut liquids, 2000);

        // The five-tile border rule freezes x < 5 and x >= 35, so this is the whole mobile pool.
        let levels: Vec<u8> = (5..35).map(|x| cave.liquid_at(x, 19)).collect();
        assert!(
            levels.windows(2).all(|w| w[0].abs_diff(w[1]) <= 1),
            "no step between neighbours: {levels:?}"
        );
        assert!(
            levels.iter().all(|&l| l > 0 && l < 40),
            "spread wide and shallow rather than left in a heap: {levels:?}"
        );
        assert_eq!(cave.total(), before, "and nothing lost levelling");
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

    /// A tile is waiting at most once, however hard it is pushed: vanilla's `checkingLiquid`
    /// (`Liquid.cs:1172,1181`). That, not the cap, is what bounds the queue.
    ///
    /// Fails before the fix: the queue took duplicates, so a million wakes over forty tiles left a
    /// million entries waiting (well, `QUEUE_CAP` of them, the rest silently discarded, which is
    /// what stranded water in mid-air).
    #[test]
    fn a_tile_is_never_waiting_twice() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        // One tick first, so the simulation knows the world it is indexing bits against.
        liquids.tick(&mut cave);
        for _ in 0..25_000 {
            for x in 10..30 {
                liquids.wake(x, 15);
            }
        }
        assert_eq!(
            liquids.pending(),
            20,
            "twenty distinct tiles, twenty entries"
        );
        // And out-of-world wakes are not held at all: `settle` drops them on sight anyway.
        liquids.wake(-1, 15);
        liquids.wake(10, 999);
        assert_eq!(liquids.pending(), 20);
    }

    /// Waking a tile that is already waiting leaves it where it is in the order, rather than
    /// pushing it to the back. `AddWater`'s early return (`Liquid.cs:1172`) does nothing at all to
    /// a cell already flagged.
    #[test]
    fn a_repeat_wake_does_not_reorder_the_queue() {
        let mut cave = Cave::new();
        let mut liquids = Liquids::default();
        liquids.tick(&mut cave);
        for x in 10..14 {
            liquids.wake(x, 15);
        }
        liquids.wake(10, 15);
        assert_eq!(
            std::iter::from_fn(|| liquids.queue.pop_front()).collect::<Vec<_>>(),
            vec![(10, 15), (11, 15), (12, 15), (13, 15)]
        );
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
