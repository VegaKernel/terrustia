//! Trees, at generation.
//!
//! A generated world had none. Not "fewer than vanilla" — none at all, which is the first thing
//! anyone notices and the last thing they can work around, because wood is the first material in
//! the game and half the crafting tree starts with it.
//!
//! `growth.rs` already grows a sapling into a tree at runtime, but deliberately as a plain trunk:
//! every tile `frameX = 0`, no branches, no roots, no canopy. That renders as a bare pole. It is
//! fine for one tree somebody planted and wrong for a forest.
//!
//! So the frames are transcribed from `WorldGen.GrowTree` (`WorldGen.cs:30048`). There is no
//! algorithm to understand: roughly 330 of that function's 493 lines are literal `frameX`/`frameY`
//! pairs, one per (segment style, variant). What follows is that table, and the rules for choosing
//! between its rows.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::Tile;

use super::World;

/// The tree tile.
const TREE: u16 = 5;

/// Ground a tree will grow on: forest, corrupt, jungle, mushroom, hallow, snow and crimson grass.
///
/// `WorldGen.IsTileTypeFitForTree`.
fn fit_for_tree(block: u16) -> bool {
    matches!(block, 2 | 23 | 60 | 70 | 109 | 147 | 199)
}

/// One trunk segment's frames, by variant.
///
/// Three variants per style, which is what `genRand.Next(3)` picks between; the style itself comes
/// from `genRand.Next(10)`. Styles 5, 6 and 7 grow a branch — left, right, and both — and vanilla
/// refuses to put two branches on the same side in consecutive segments, which is why the caller
/// tracks what the last one did.
const TRUNK: [[(i16, i16); 3]; 8] = [
    // 0 and 8, 9 fall to the default: a plain trunk.
    [(0, 0), (0, 22), (0, 44)],
    [(0, 66), (0, 88), (0, 110)],
    [(22, 0), (22, 22), (22, 44)],
    [(44, 66), (44, 88), (44, 110)],
    [(22, 66), (22, 88), (22, 110)],
    // 5: a branch to the left.
    [(88, 0), (88, 22), (88, 44)],
    // 6: a branch to the right.
    [(66, 66), (66, 88), (66, 110)],
    // 7: both.
    [(110, 66), (110, 88), (110, 110)],
];

/// A left branch's own tile, in its two forms — weighted two-to-one toward the first
/// (`WorldGen.cs:30271-30294`, `genRand.Next(3) < 2`). Grows beside whichever trunk segment
/// rolled style 5 or 7, at that segment's own row: a *second* tile, not a frame on the trunk.
///
/// This table used to be called `ROOT_LEFT` and be written at the tree's base, immediately
/// overwritten there by `BASE_LEFT` a few lines later — a dead write on frames that were never a
/// root at all. Every generated tree had branch stubs on its trunk and nothing beside them.
const BRANCH_LEFT: [[(i16, i16); 3]; 2] = [
    [(44, 198), (44, 220), (44, 242)],
    [(66, 0), (66, 22), (66, 44)],
];

/// A right branch's own tile, the same way — `WorldGen.cs:30314-30357`.
const BRANCH_RIGHT: [[(i16, i16); 3]; 2] = [
    [(66, 198), (66, 220), (66, 242)],
    [(88, 66), (88, 88), (88, 110)],
];

/// The base of the trunk, which differs by which roots it has.
const BASE: [[(i16, i16); 3]; 4] = [
    // Both roots.
    [(88, 132), (88, 154), (88, 176)],
    // Left only.
    [(0, 132), (0, 154), (0, 176)],
    // Right only.
    [(66, 132), (66, 154), (66, 176)],
    // Neither.
    [(22, 132), (22, 154), (22, 176)],
];

/// The canopy, in its two forms.
const TOP: [[(i16, i16); 3]; 2] = [
    [(22, 198), (22, 220), (22, 242)],
    [(0, 198), (0, 220), (0, 242)],
];

/// The pieces beside the base, which are part of the root rather than the trunk.
const BASE_LEFT: [(i16, i16); 3] = [(44, 132), (44, 154), (44, 176)];
const BASE_RIGHT: [(i16, i16); 3] = [(22, 132), (22, 154), (22, 176)];

/// Grow a tree upward from the ground tile at `(x, y)`.
///
/// `(x, y)` is the *ground*, not the first trunk tile: the trunk stands above it. Returns whether
/// one grew, so a caller can count them without inspecting the world afterwards.
pub fn grow(world: &mut World, x: i32, y: i32, rng: &mut SmallRng) -> bool {
    let ground = world.tile(x, y);
    if !ground.is_active() || !fit_for_tree(ground.block) {
        return false;
    }
    // A tree needs a bank, not a ledge: one of its neighbours has to be the same ground.
    let flanked = [-1, 1].iter().any(|dx| {
        let side = world.tile(x + dx, y);
        side.is_active() && fit_for_tree(side.block)
    });
    if !flanked {
        return false;
    }

    let height = rng.random_range(5..17);
    // Vanilla checks a box two wider than the trunk and four taller, because the canopy and the
    // branches reach outside the single column the trunk occupies.
    for dy in 1..=height + 4 {
        for dx in -2..=2 {
            if !world.in_bounds(x + dx, y - dy) {
                return false;
            }
            let space = world.tile(x + dx, y - dy);
            if space.is_active() || space.liquid > 0 {
                return false;
            }
        }
    }

    // --- the trunk ----------------------------------------------------------------------------
    //
    // Segments are chosen one at a time from the top of the ground upward. A branch may not repeat
    // on the same side two segments running, and the first and last segments are always plain —
    // a branch growing straight out of the ground or into the canopy looks wrong.
    let (mut branched_left, mut branched_right) = (false, false);
    for step in 0..height {
        let at_end = step == 0 || step == height - 1;
        let mut style = if at_end { 0 } else { rng.random_range(0..10) };
        // Re-roll rather than clamp, which is what vanilla does, so the distribution matches.
        let mut guard = 0;
        while guard < 16
            && (((style == 5 || style == 7) && branched_left)
                || ((style == 6 || style == 7) && branched_right))
        {
            style = rng.random_range(0..10);
            guard += 1;
        }
        branched_left = style == 5 || style == 7;
        branched_right = style == 6 || style == 7;

        let variant = rng.random_range(0..3);
        // Styles past the table are the plain trunk, exactly as the `default` arm is.
        let row = TRUNK.get(style as usize).unwrap_or(&TRUNK[0]);
        let (fx, fy) = row[variant as usize];
        let trunk_row = y - 1 - step;
        world.set_tile(x, trunk_row, Tile::framed(TREE, fx, fy));

        // A branch is a real tile beside the trunk, at the same row — not merely implied by the
        // trunk's own joint frame. `WorldGen.cs:30271-30357`: each side is independent (a style-7
        // segment gets both) and, on each, the first frame form wins twice as often as the
        // second.
        if branched_left {
            let form = usize::from(rng.random_range(0..3) >= 2);
            let (fx, fy) = BRANCH_LEFT[form][rng.random_range(0..3) as usize];
            world.set_tile(x - 1, trunk_row, Tile::framed(TREE, fx, fy));
        }
        if branched_right {
            let form = usize::from(rng.random_range(0..3) >= 2);
            let (fx, fy) = BRANCH_RIGHT[form][rng.random_range(0..3) as usize];
            world.set_tile(x + 1, trunk_row, Tile::framed(TREE, fx, fy));
        }
    }

    // --- the roots ----------------------------------------------------------------------------
    //
    // A root only grows where the ground beside the trunk would hold one. Nothing is written to
    // either tile yet — the `BASE_LEFT`/`BASE_RIGHT` step below is what actually places a root,
    // and always did; this only decides *whether* one grows here.
    let root_row = y - 1;
    let mut has_left = false;
    let mut has_right = false;
    if root_grows_here(world, x - 1, y) && rng.random_range(0..3) < 2 {
        has_left = true;
    }
    if root_grows_here(world, x + 1, y) && rng.random_range(0..3) < 2 {
        has_right = true;
    }

    // The base has to agree with the roots that actually grew, or the trunk appears to float.
    let base = match (has_left, has_right) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 3,
    };
    let (fx, fy) = BASE[base][rng.random_range(0..3) as usize];
    world.set_tile(x, root_row, Tile::framed(TREE, fx, fy));
    if has_left {
        let (fx, fy) = BASE_LEFT[rng.random_range(0..3) as usize];
        world.set_tile(x - 1, root_row, Tile::framed(TREE, fx, fy));
    }
    if has_right {
        let (fx, fy) = BASE_RIGHT[rng.random_range(0..3) as usize];
        world.set_tile(x + 1, root_row, Tile::framed(TREE, fx, fy));
    }

    // --- the canopy ---------------------------------------------------------------------------
    let form = usize::from(rng.random_range(0..2) == 0);
    let (fx, fy) = TOP[form][rng.random_range(0..3) as usize];
    world.set_tile(x, y - height, Tile::framed(TREE, fx, fy));
    true
}

/// Whether the tile beside a trunk will hold a root.
fn root_grows_here(world: &World, x: i32, y: i32) -> bool {
    let ground = world.tile(x, y);
    ground.is_active() && fit_for_tree(ground.block) && !world.tile(x, y - 1).is_active()
}

/// Plant a forest across the surface.
///
/// `WorldGen.AddTrees` sweeps every column, tries to grow one, and then skips one or two at random
/// so a forest is not a picket fence. Density is deliberately vanilla's: about one attempt per
/// column, most of which fail because the ground is not grass or the space is not clear.
pub fn plant_forest(world: &mut World, rng: &mut SmallRng) -> usize {
    let mut grown = 0;
    let mut x = 1;
    let surface = world.surface as i32;
    while x < world.width() - 1 {
        // Find the ground in this column, within the band a forest can occupy.
        let top = (surface - 200).max(1);
        let bottom = (surface + 50).min(world.height() - 2);
        for y in top..bottom {
            if world.tile(x, y).is_active() {
                if fit_for_tree(world.tile(x, y).block) && grow(world, x, y, rng) {
                    grown += 1;
                }
                break;
            }
        }
        // Vanilla skips one or two columns after each attempt.
        x += 1 + i32::from(rng.random_range(0..2) == 0);
    }
    grown
}

/// Hang vines from the undersides of grass, and grow cacti in the sand.
///
/// Both reuse the runtime growers in `growth.rs`, which already know the rules — a vine only hangs
/// from grass of its own biome, a cactus only stands on sand with room above. Calling them at
/// generation is the difference between a jungle that looks like a jungle and one that looks like
/// a green cave.
///
/// Returns how many of each grew.
pub fn plant_undergrowth(world: &mut World, rng: &mut SmallRng) -> (usize, usize) {
    use super::growth;

    let (mut vines, mut cacti) = (0, 0);
    let surface = world.surface as i32;
    let top = (surface - 200).max(1);
    let bottom = (surface + 400).min(world.height() - 2);

    for x in 1..world.width() - 1 {
        // The whole column, not just its topmost tile. A vine hangs from grass with *air beneath
        // it* — an overhang, a ledge, a cave roof — and the topmost tile of a column never has
        // that by definition, which is why scanning only the surface grew exactly none.
        let mut cactus_here = false;
        for y in top..bottom {
            let here = world.tile(x, y);
            if !here.is_active() {
                continue;
            }
            if !world.tile(x, y + 1).is_active() && rng.random_range(0..5) < 2 {
                // Each call extends a vine by one tile, so a vine of any length needs several.
                let length = rng.random_range(1..10);
                let mut at = (x, y);
                for _ in 0..length {
                    match growth::grow_vine(world, at.0, at.1) {
                        Some(next) => {
                            at = next;
                            vines += 1;
                        }
                        None => break,
                    }
                }
            }
            // One cactus per column at most, on the first sand with room above it. `grow_cactus`
            // takes the *sand*, not the air over it — passing the air was the other reason none
            // grew.
            if !cactus_here
                && here.block == 53
                && !world.tile(x, y - 1).is_active()
                && rng.random_range(0..25) == 0
            {
                for _ in 0..rng.random_range(1..4) {
                    if growth::grow_cactus(world, x, y).is_some() {
                        cacti += 1;
                    }
                }
                cactus_here = true;
            }
        }
    }
    (vines, cacti)
}

/// Line the jungle's exposed mud with grass.
///
/// Vanilla's jungle is mud walled in jungle grass wherever a cave has opened it to the air, which
/// is what makes an underground jungle green rather than brown — and it is what vines hang from.
/// Measured before this existed: a whole 4200-wide world had **two** grass tiles with air beneath
/// them, so the vine pass was correct and had nowhere to work.
///
/// Deliberately separate from `growth::spread_grass`, which is the runtime spread and only handles
/// dirt. This is a generation-time sweep over mud, and keeping them apart means changing one
/// cannot quietly change how the other behaves during play.
///
/// Returns how many tiles turned green.
pub fn grass_the_jungle(world: &mut World) -> usize {
    const MUD: u16 = 59;
    const JUNGLE_GRASS: u16 = 60;

    let mut spread = 0;
    // Collected first, then applied: converting as we go would let a tile that has just become
    // grass qualify its neighbour in the same sweep, and the green would creep through solid rock.
    let mut turning = Vec::new();
    for x in 1..world.width() - 1 {
        for y in 1..world.height() - 1 {
            if world.tile(x, y).block != MUD {
                continue;
            }
            let open = [(0, -1), (0, 1), (-1, 0), (1, 0)]
                .iter()
                .any(|(dx, dy)| !world.tile(x + dx, y + dy).is_active());
            if open {
                turning.push((x, y));
            }
        }
    }
    for (x, y) in turning {
        let mut tile = world.tile(x, y);
        tile.block = JUNGLE_GRASS;
        world.set_tile(x, y, tile);
        spread += 1;
    }
    spread
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    /// A patch of grass with air above it, which is what a tree needs.
    fn meadow(width: i32) -> World {
        let mut world = World::empty(width, 200, "trees");
        for x in 0..width {
            for y in 100..110 {
                world.set_tile(x, y, Tile::block(if y == 100 { 2 } else { 0 }));
            }
        }
        world
    }

    #[test]
    fn a_tree_has_a_trunk_a_base_and_a_canopy() {
        let mut world = meadow(40);
        let mut rng = SmallRng::seed_from_u64(1);
        assert!(grow(&mut world, 20, 100, &mut rng), "it should grow");

        // Every tile of the trunk is a tree tile, from the base upward.
        let mut trunk = 0;
        for y in (1..100).rev() {
            if world.tile(20, y).block == TREE {
                trunk += 1;
            }
        }
        assert!(trunk >= 6, "a trunk plus a canopy, got {trunk} tiles");

        // The canopy is a canopy frame, not another trunk segment. This is the whole difference
        // between a tree and the bare pole the runtime grower used to make.
        let top = (1..100)
            .find(|&y| world.tile(20, y).block == TREE)
            .expect("some tree");
        let frames = (world.tile(20, top).frame_x, world.tile(20, top).frame_y);
        assert!(
            TOP.iter().any(|row| row.contains(&frames)),
            "the topmost tile should be a canopy, got {frames:?}"
        );
    }

    /// A branch is a real tile beside the trunk, not merely implied by the trunk's own joint
    /// frame — `WorldGen.cs:30271-30357` grows a genuine second tile at `x-1` or `x+1`, on the
    /// same row, whenever a trunk segment rolls a branching style.
    ///
    /// Fails before the fix: no tile at `x-1`/`x+1` other than the tree's own two roots (at the
    /// base row only, excluded from this scan) was ever written, whatever the trunk's own frames
    /// implied — the ROOT_LEFT/ROOT_RIGHT writes that used to sit here landed on the base row and
    /// were immediately overwritten by the real root frames a few lines later, so every generated
    /// tree had branch stubs on its trunk and nothing beside them.
    #[test]
    fn branches_are_real_tiles_beside_the_trunk() {
        let mut found_branch = false;
        for seed in 0..80u64 {
            let mut world = meadow(40);
            let mut rng = SmallRng::seed_from_u64(seed);
            if !grow(&mut world, 20, 100, &mut rng) {
                continue;
            }
            // Rows 1..99: above the ground (100) and below the root row (99), so a tile found
            // here can only be a branch, never one of the tree's two roots.
            for y in 1..99 {
                if world.tile(19, y).block == TREE || world.tile(21, y).block == TREE {
                    found_branch = true;
                    break;
                }
            }
            if found_branch {
                break;
            }
        }
        assert!(
            found_branch,
            "no branch tile appeared beside a trunk in 80 tries"
        );
    }

    /// Frames must never be -1: that is what "no frame" means, and it renders as a corrupt sprite.
    #[test]
    fn every_tile_of_a_tree_is_framed() {
        let mut world = meadow(40);
        let mut rng = SmallRng::seed_from_u64(7);
        grow(&mut world, 20, 100, &mut rng);

        for y in 1..100 {
            for x in 19..=21 {
                let tile = world.tile(x, y);
                if tile.block == TREE {
                    assert_ne!(tile.frame_x, -1, "unframed tree tile at {x},{y}");
                    assert_ne!(tile.frame_y, -1, "unframed tree tile at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn a_tree_will_not_grow_without_ground_or_without_room() {
        let mut world = meadow(40);
        let mut rng = SmallRng::seed_from_u64(3);
        // Nothing underneath.
        assert!(!grow(&mut world, 20, 50, &mut rng));
        // Ground, but the space above is taken.
        world.set_tile(20, 95, Tile::block(1));
        assert!(!grow(&mut world, 20, 100, &mut rng), "no room to grow");
    }

    /// The point of the whole module: a generated surface ends up with a forest on it.
    #[test]
    fn a_forest_grows_across_the_surface() {
        let mut world = meadow(600);
        world.surface = 100;
        let mut rng = SmallRng::seed_from_u64(11);
        let grown = plant_forest(&mut world, &mut rng);
        assert!(
            grown > 20,
            "a 600-wide meadow should grow a forest, got {grown} trees"
        );
    }

    /// Mud open to the air becomes grass; mud buried in mud does not.
    ///
    /// The measurement that prompted this: before it, a whole 4200-wide world had **two** grass
    /// tiles with air beneath them, so the vine pass was correct and had nowhere to work. After,
    /// 732 — and 2,415 vine tiles where there had been nine.
    #[test]
    fn only_mud_open_to_the_air_turns_green() {
        const MUD: u16 = 59;
        let mut world = World::empty(40, 40, "jungle");
        // A solid block of mud with one face open at the top.
        for x in 10..20 {
            for y in 20..30 {
                world.set_tile(x, y, Tile::block(MUD));
            }
        }
        let spread = grass_the_jungle(&mut world);

        assert!(spread > 0, "the open face should have turned green");
        assert_eq!(world.tile(15, 20).block, 60, "the exposed top is grass");
        assert_eq!(
            world.tile(15, 25).block,
            MUD,
            "mud buried in mud stays mud, or the green creeps through solid ground"
        );
    }

    /// The sweep must not feed on itself: a tile that turns green cannot qualify its neighbour in
    /// the same pass, or the whole block converts.
    #[test]
    fn the_grass_does_not_creep_through_solid_ground() {
        const MUD: u16 = 59;
        let mut world = World::empty(40, 60, "jungle");
        for x in 10..20 {
            for y in 20..50 {
                world.set_tile(x, y, Tile::block(MUD));
            }
        }
        grass_the_jungle(&mut world);

        // Deep inside, well away from every face.
        for y in 25..45 {
            assert_eq!(
                world.tile(15, y).block,
                MUD,
                "mud at depth {y} should not have turned"
            );
        }
    }
}
