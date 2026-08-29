//! Living trees: rare, very tall Living Wood trunks with branches and a taproot, plus the real
//! `LivingTreeWalls` pass that fills wall behind them.
//!
//! Transcribed from `GrowLivingTree` (`WorldGen.cs:28255-28891`, 636 lines) and `LivingTreeWalls`
//! (`WorldGen.cs:15804-15836`).
//!
//! **`LivingTreeWalls` is transcribed faithfully in full** — genuinely the small pass it looks
//! like: any tile touching a Living Wood (191) tile, whose full 3×3 neighbourhood is entirely
//! Living Wood or already-`LivingWoodUnsafe` (244) wall, gets that wall too.
//!
//! **`GrowLivingTree` is not.** Real vanilla trees are two systems layered together: the visible
//! trunk-and-branches shape carved above ground (what's transcribed here — the alternating
//! narrow-left/narrow-right growth that gives a trunk its taper, the periodic branch spawns off
//! each narrowing step, a taproot growing down from the base), *and* a second, much larger system
//! that turns every trunk/branch/root endpoint the first system records into a candidate for
//! `GrowLivingTree_MakePassage` (a 400-1000-tile wandering root passage, occasionally opening into
//! a full `GrowLivingTreePassageRoom` secret room) — genuinely its own several-hundred-line
//! subsystem, not a small detail on top of the visible tree. Not transcribed here, along with
//! every secret-seed variant (`extraLivingTrees`, `noSurface`, `errorWorld`, the "patch" mode used
//! elsewhere) and the driving pass's own elaborate dungeon/marble/granite/moss-cave exclusion
//! zones, simplified here to the same "the whole footprint has to already be clear" gate every
//! other siting pass in this generator uses. What a tree grown here still gets that a bare
//! trunk-shape wouldn't: one real hollow chamber partway up, holding a chest — not vanilla's own
//! room-placement mechanism (that lives in the skipped passage system), but the same reasoning
//! `underground_cabins.rs`/`underworld_ruins.rs` already used for their own chests: an explorable
//! structure needs *something* to find inside it.

use super::layout::Layout;
use super::rand::UnifiedRandom;
use super::structures;
use super::tiles;
use crate::world::World;
use terrustia_proto::{Tile, TileFlags};

/// `TileID.LivingWood` / `WallID.LivingWoodUnsafe` — the one material a living tree is built from.
const LIVING_WOOD: u16 = 191;
const LIVING_WOOD_WALL: u16 = 244;

/// A tile written by [`grow_trunk`] the tree still needs to become a branch base — recorded the
/// same way vanilla's own `array`/`array3` pair does, just as a `Vec` instead of two parallel
/// fixed arrays.
struct Branch {
    x: i32,
    y: i32,
    /// -1 grows the branch further left, 1 further right — vanilla's own `array3`.
    dir: i32,
    /// The trunk's width at the moment this branch was recorded, which is what scales how far it
    /// reaches (`array4`, read by the branch-growth loop as `width * (1..3)`).
    width: i32,
}

fn wood(world: &mut World, x: i32, y: i32) {
    // Vanilla's own guard: never overwrite a dungeon-brick wall — see `Main.wallDungeon`'s own
    // callers throughout this function, all of which skip a write rather than carve through one.
    if matches!(
        world.tile(x, y).wall,
        tiles::walls::BLUE_DUNGEON | tiles::walls::GREEN_DUNGEON | tiles::walls::PINK_DUNGEON
    ) {
        return;
    }
    let mut t = world.tile(x, y);
    t.block = LIVING_WOOD;
    t.flags.set(TileFlags::ACTIVE, true);
    t.flags.set(TileFlags::HALF_BRICK, false);
    t.slope = 0;
    world.set_tile(x, y, t);
}

/// The trunk itself: narrows as it climbs, alternating which side steps inward, recording a
/// branch point every 5-15 rows — `GrowLivingTree`'s own main growth loop
/// (`WorldGen.cs:28389-28521`), the secret-seed variance and platform-carving side effect
/// dropped.
fn grow_trunk(
    world: &mut World,
    x: i32,
    y: i32,
    rand: &mut UnifiedRandom,
) -> (Vec<Branch>, i32, i32, i32) {
    let mut left = x - rand.next_range(2, 3);
    let mut right = x + rand.next_range(2, 3);
    if rand.next_max(5) == 0 {
        if rand.next_bool() {
            left -= 1;
        } else {
            right += 1;
        }
    }

    let mut cy = y;
    let mut growing = true;
    let mut narrow_countdown = rand.next_range(-8, -4);
    let mut side = rand.next_max(2);
    let mut reroll_at = rand.next_range(5, 15);
    let mut left_shrink_budget = left;
    let mut right_shrink_budget = right;
    let mut branches = Vec::new();

    while growing {
        narrow_countdown += 1;
        if narrow_countdown > reroll_at {
            reroll_at = rand.next_range(5, 15);
            narrow_countdown = 0;
            let branch_y = cy + rand.next_max(5);
            if rand.next_max(5) == 0 {
                side = 1 - side;
            }
            if side == 0 {
                branches.push(Branch {
                    x: left,
                    y: branch_y,
                    dir: -1,
                    width: right - left,
                });
                if rand.next_max(2) == 0 {
                    left += 1;
                }
                left_shrink_budget += 1;
                side = 1;
            } else {
                branches.push(Branch {
                    x: right,
                    y: branch_y,
                    dir: 1,
                    width: right - left,
                });
                if rand.next_max(2) == 0 {
                    right -= 1;
                }
                right_shrink_budget -= 1;
                side = 0;
            }
            if left_shrink_budget == right_shrink_budget {
                growing = false;
            }
        }
        for m in left..=right {
            wood(world, m, cy);
        }
        cy -= 1;
        if y - cy > 400 {
            // Vanilla has no explicit cap here — the narrowing itself always terminates the loop
            // eventually — but a synthetic or pathological world could starve `left`/`right`
            // toward each other so slowly this never converges. A generation pass looping forever
            // is worse than a tree slightly shorter than vanilla's own would ever produce.
            growing = false;
        }
    }
    (branches, left, right, cy)
}

/// One branch, grown outward and slightly wandering from where the trunk recorded it —
/// `GrowLivingTree`'s own branch-growth loop (`WorldGen.cs:28522-28592`), the "occasionally stamp
/// an extra wood tile beside the path" cosmetic detail dropped.
fn grow_branch(world: &mut World, branch: &Branch, rand: &mut UnifiedRandom) {
    let mut x = branch.x + branch.dir;
    let mut y = branch.y;
    wood(world, x, y + 1);
    let length =
        (f64::from(branch.width) * (1.0 + f64::from(rand.next_range(20, 30)) * 0.1)) as i32;
    for _ in 0..length.max(0) {
        wood(world, x, y);
        if rand.next_max(10) == 0 {
            y += if rand.next_bool() { 1 } else { -1 };
        } else {
            x += branch.dir;
        }
    }
}

/// The taproot: a single column growing straight down from the trunk's base — `GrowLivingTree`'s
/// own root loop (`WorldGen.cs:28594-28606`), stripped of its own side-branches (a second,
/// smaller copy of the same branch-wander shape [`grow_branch`] already covers).
fn grow_taproot(world: &mut World, x: i32, y: i32, width: i32, rand: &mut UnifiedRandom) {
    let mut cy = y;
    let mut length = rand.next_range(width * 3, width * 5).max(0);
    while length > 0 && cy < y + 400 {
        wood(world, x, cy);
        length -= 1;
        cy += 1;
    }
}

/// One hollow chamber partway up the trunk, holding a chest — not vanilla's own mechanism (see
/// the module doc), just enough that a tree explored in-game has something inside it.
fn carve_chamber(world: &mut World, x: i32, y: i32, layout: &Layout, rand: &mut UnifiedRandom) {
    let (w, h) = (5, 4);
    for dx in -w..=w {
        for dy in 0..h {
            let (cx, cy) = (x + dx, y - dy);
            if dx.abs() == w || dy == h - 1 {
                wood(world, cx, cy);
            } else {
                let t = Tile {
                    wall: LIVING_WOOD_WALL,
                    ..Tile::default()
                };
                world.set_tile(cx, cy, t);
            }
        }
    }
    let loot = structures::cavern_loot(layout, y, rand);
    structures::add_chest(world, x - 1, y - 1, loot, rand);
}

/// The whole footprint a tree's trunk box needs clear before it can grow — a simplified stand-in
/// for `GrowLivingTree`'s own dungeon/marble/granite/moss-cave exclusion checks: this generator
/// doesn't track those as named regions distinct from the tiles they're made of, so "nothing
/// active in the box" (the same basic gate the real check falls back to when none of its named
/// exclusions apply) is what's transcribed.
fn site_is_clear(world: &World, x: i32, y: i32) -> bool {
    for k in (x - 50)..(x + 50) {
        for l in 5..(y - 5) {
            if world.tile(k, l).is_active() {
                return false;
            }
        }
    }
    true
}

/// `GrowLivingTree`'s own driving loop: pick a candidate x away from the world's centre margin,
/// find the real surface, check the footprint is clear, and grow. Returns how many were placed.
pub fn scatter(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) -> usize {
    let margin = 400;
    if layout.width <= margin * 2 || layout.surface <= 200 {
        return 0;
    }
    let wanted = rand.next_max(3); // vanilla's own `Next(0, 2*width/4200 + 1)`, roughly 0-2 here
    let center = layout.width / 2;
    let dead_zone = 200;
    let mut placed = 0usize;
    for _ in 0..wanted {
        let mut tries = 0;
        while tries < layout.width / 2 {
            tries += 1;
            let x = rand.next_range(margin, layout.width - margin);
            if x > center - dead_zone && x < center + dead_zone {
                continue;
            }
            let mut y = 0;
            while !world.tile(x, y).is_active() && y < layout.surface {
                y += 1;
            }
            // Vanilla's own check is a plain `type == 0` (bare Dirt) — widened to also accept
            // `GRASS`, the same fix `oasis.rs`/`pyramids.rs` already needed for their own
            // material checks: this generator's own terrain is grass-covered by the time this
            // pass runs (matching vanilla's real post-`GrassSpread` state too), so literal bare
            // dirt essentially never appears as the topmost active tile — measured at every real
            // surface find in a real generated world landing on `GRASS` or a biome-specific
            // material, never `DIRT`, before this fix.
            if y >= layout.surface
                || y <= 150
                || !matches!(world.tile(x, y).block, tiles::DIRT | tiles::GRASS)
            {
                continue;
            }
            y -= 1;
            if !site_is_clear(world, x, y) {
                continue;
            }
            let (branches, left, right, top) = grow_trunk(world, x, y, rand);
            for branch in &branches {
                grow_branch(world, branch, rand);
            }
            grow_taproot(world, (left + right) / 2, y + 1, right - left, rand);
            if y - top > 20 {
                carve_chamber(world, (left + right) / 2, y - (y - top) / 2, layout, rand);
            }
            placed += 1;
            break;
        }
    }
    placed
}

/// The real `LivingTreeWalls` pass, transcribed in full: any tile touching a Living Wood tile,
/// whose full 3×3 neighbourhood is entirely Living Wood or already this wall, gets it too.
pub fn scatter_walls(world: &mut World, layout: &Layout) {
    for x in 25..(layout.width - 25) {
        for y in 25..layout.surface {
            let near_wood = world.tile(x, y).block == LIVING_WOOD
                || world.tile(x, y - 1).block == LIVING_WOOD
                || world.tile(x - 1, y).block == LIVING_WOOD
                || world.tile(x + 1, y).block == LIVING_WOOD
                || world.tile(x, y + 1).block == LIVING_WOOD;
            if !near_wood {
                continue;
            }
            let mut all_wood = true;
            for k in (x - 1)..=(x + 1) {
                for l in (y - 1)..=(y + 1) {
                    if k == x && l == y {
                        continue;
                    }
                    let t = world.tile(k, l);
                    let is_wood = t.is_active() && t.block == LIVING_WOOD;
                    if !is_wood && t.wall != LIVING_WOOD_WALL {
                        all_wood = false;
                    }
                }
            }
            if all_wood {
                let mut t = world.tile(x, y);
                t.wall = LIVING_WOOD_WALL;
                world.set_tile(x, y, t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::worldgen::rand::UnifiedRandom as Rand;

    #[test]
    fn a_small_world_returns_zero_rather_than_panicking() {
        let mut world = World::empty(300, 200, "living-trees-test");
        let mut rand = Rand::new(1);
        let mut layout = Layout::plan(300, 200, &mut rand);
        layout.surface = 100;
        assert_eq!(scatter(&mut world, &layout, &mut rand), 0);
        scatter_walls(&mut world, &layout); // must not panic either, on the same small world
    }

    /// Against a real generated world, matching `underground_cabins.rs`/`underworld_ruins.rs`'s
    /// own precedent: `grow_trunk` needs a real dirt surface and a genuinely clear 100-wide
    /// footprint above it, neither of which a hand-built fixture reproduces without duplicating
    /// most of `terrain.rs` — the real pipeline already has both.
    #[test]
    fn a_tree_grows_a_real_tall_trunk_with_walls_behind_it() {
        // Living trees are probabilistic and a world can legitimately grow none, so this does not
        // pin a single seed: any faithful change to a draw earlier in generation (e.g. gem caves
        // now placing their scattered gems) shifts the whole downstream `rand` sequence — the exact
        // "not seed-identical" drift the generator's own module doc discloses — and re-pinning one
        // seed each time it moves is a losing game. Take the first of a handful of seeds that grows
        // one, and run the trunk/wall checks against that world.
        let (world, _built) = [999u64, 4242, 12345, 7, 2024, 31337]
            .into_iter()
            .map(|seed| super::super::build(4200, 1200, "living-trees-test", seed))
            .find(|(_, built)| built.living_trees > 0)
            .expect("at least one of several seeds should grow a living tree");

        let mut wood_tiles = 0;
        let mut walled = 0;
        for x in 0..world.width() {
            for y in 0..world.height() {
                let t = world.tile(x, y);
                if t.is_active() && t.block == LIVING_WOOD {
                    wood_tiles += 1;
                    if t.wall == LIVING_WOOD_WALL {
                        walled += 1;
                    }
                }
                // `scatter_walls` also walls inactive neighbours, not just the trunk itself.
                if !t.is_active() && t.wall == LIVING_WOOD_WALL {
                    walled += 1;
                }
            }
        }
        assert!(
            wood_tiles > 100,
            "expected a real tall trunk, got {wood_tiles} Living Wood tiles"
        );
        assert!(
            walled > 0,
            "expected LivingTreeWalls to have filled in real wall behind the tree"
        );
    }

    /// Real placement counts on real generated worlds — not asserted, just printed. Run with
    /// `cargo test -p terrustia --lib living_trees::tests::measure_on_real_worlds -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn measure_on_real_worlds() {
        for seed in [999u64, 4242, 12345] {
            let (_world, built) = super::super::build(4200, 1200, "measure", seed);
            eprintln!("seed {seed}: living_trees={}", built.living_trees);
        }
    }
}
