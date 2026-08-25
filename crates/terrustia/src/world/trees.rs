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

/// The left-hand root, in its two forms.
const ROOT_LEFT: [[(i16, i16); 3]; 2] = [
    [(44, 198), (44, 220), (44, 242)],
    [(66, 0), (66, 22), (66, 44)],
];

/// The right-hand root.
const ROOT_RIGHT: [[(i16, i16); 3]; 2] = [
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
        world.set_tile(x, y - 1 - step, Tile::framed(TREE, fx, fy));
    }

    // --- the roots ----------------------------------------------------------------------------
    //
    // A root only grows where the ground beside the trunk would hold one.
    let root_row = y - 1;
    let mut has_left = false;
    let mut has_right = false;
    if root_grows_here(world, x - 1, y) && rng.random_range(0..3) < 2 {
        let form = usize::from(rng.random_range(0..2) == 0);
        let (fx, fy) = ROOT_LEFT[form][rng.random_range(0..3) as usize];
        world.set_tile(x - 1, root_row, Tile::framed(TREE, fx, fy));
        has_left = true;
    }
    if root_grows_here(world, x + 1, y) && rng.random_range(0..3) < 2 {
        let form = usize::from(rng.random_range(0..2) == 0);
        let (fx, fy) = ROOT_RIGHT[form][rng.random_range(0..3) as usize];
        world.set_tile(x + 1, root_row, Tile::framed(TREE, fx, fy));
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
}
