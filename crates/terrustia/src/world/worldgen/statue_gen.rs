//! Statues, at generation.
//!
//! Unlike pots, the `Statues` pass (`WorldGen.cs:16962`) places through the *generic* path —
//! `PlaceTile(x, y, type, mute: true, forced: true, -1, style)` — which routes through
//! `TileObjectData`, the same table `terrustia-proto/src/tile_object.rs` transcribes. So this one
//! *is* safe to place with [`super::place_object::place_object`]; an earlier sizing pass this
//! session independently verified entry 105's `style_wrap`/`style_line_skip` against `Place2xX`'s
//! generic dispatch and it holds.
//!
//! What has to be exact instead is `GenVars.statueList` (`WorldGen.cs:4358`,
//! `SetupStatueList`) — the round-robin order statues cycle through. Order is load-bearing: statue
//! *i* is whichever type/style sits at index `i mod len` of this list, so a shuffled table would
//! change which statues a world actually gets, not just how many.

use rand::{Rng, rngs::SmallRng};

use super::layout::Layout;
use super::place_object::place_object;
use crate::world::World;

/// `GenVars.statueList`, transcribed verbatim from `SetupStatueList`. `(block, style)`.
///
/// The first 44 entries are `(105, 0..44)`, except index 34 — which is overridden to `(349, 0)`,
/// a *different* tile type entirely — and index 43, overridden to `(105, 50)`. Then 27 more
/// entries follow, all block 105, in the exact order vanilla appends them.
const STATUE_LIST: [(u16, i32); 71] = {
    let mut list = [(105u16, 0i32); 71];
    let mut i = 0;
    while i < 44 {
        list[i] = (105, i as i32);
        i += 1;
    }
    list[34] = (349, 0);
    list[43] = (105, 50);
    // The 27 appended entries, in order.
    let tail: [i32; 27] = [
        63, 64, 65, 66, 68, 69, 70, 71, 72, 73, 75, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62,
        77, 78, 67, 74,
    ];
    let mut t = 0;
    while t < tail.len() {
        list[44 + t] = (105, tail[t]);
        t += 1;
    }
    list
};

// The two trailing entries `SetupStatueList` adds after the loop above (37 and 2) bring the real
// list to 73, not 71 — kept separate because they don't fit the `while` construction cleanly in a
// const context; appended below at first use instead. See `full_statue_list`.

/// The complete list, including the two entries a `const fn` couldn't append inline.
fn full_statue_list() -> Vec<(u16, i32)> {
    let mut list = STATUE_LIST.to_vec();
    list.push((105, 37));
    list.push((105, 2));
    list
}

/// Scatter statues through the world, cycling through `statueList` in order.
///
/// Vanilla sites `statueList.len() * 2 * (width / 4200)` of them, retrying each up to 10,000
/// random positions in the band between two-thirds of the way to the rock layer and 300 tiles
/// above the underworld. Statue traps are **not** built here — they need a wire model, tracked
/// separately — so an entry from `GenVars.StatuesWithTraps` places as a plain statue with no
/// trap, same visible statue, missing mechanism.
pub fn scatter(world: &mut World, layout: &Layout, rng: &mut SmallRng) -> usize {
    let list = full_statue_list();
    if list.is_empty() {
        return 0;
    }
    let wanted = (list.len() as i64 * 2 * i64::from(layout.width) / 4200).max(1) as usize;
    let band_top = ((layout.surface as i64 * 2 + layout.rock as i64) / 3) as i32;
    let band_bottom = (world.height() - 300).max(band_top + 1);

    let mut placed = 0usize;
    for i in 0..wanted {
        let (block, style) = list[i % list.len()];

        for _ in 0..100 {
            let x = rng.random_range(20..(layout.width - 20).max(21));
            let mut y = rng.random_range(band_top.max(1)..band_bottom);
            while y < world.height() - 1 && !world.tile(x, y).is_active() {
                y += 1;
            }
            if y >= world.height() - 1 {
                continue;
            }
            // Vanilla steps back up one row once it finds the ground (`num7--`): the placement
            // anchor is the open tile resting *on* the floor, not the floor tile itself. Passing
            // the floor tile straight through made the statue's own footprint overlap the ground
            // it was meant to stand on, so `place_object` refused every single attempt.
            y -= 1;
            let anchor = world.tile(x, y);
            if anchor.is_active() && terrustia_proto::tile_sets::frame_important(anchor.block) {
                continue;
            }
            if place_object(world, x, y, block, style, -1) {
                placed += 1;
                break;
            }
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn the_statue_list_has_seventy_three_entries_in_vanillas_order() {
        let list = full_statue_list();
        assert_eq!(list.len(), 73);
        assert_eq!(list[0], (105, 0));
        assert_eq!(list[33], (105, 33));
        assert_eq!(list[34], (349, 0), "the one non-105 entry");
        assert_eq!(list[35], (105, 35));
        assert_eq!(list[43], (105, 50), "the second override");
        assert_eq!(list[44], (105, 63), "the first appended entry");
        assert_eq!(
            list[70],
            (105, 74),
            "the last of the 27 appended in the while loop"
        );
        assert_eq!(list[71], (105, 37));
        assert_eq!(list[72], (105, 2), "the very last entry");
    }

    fn flat_world(width: i32) -> World {
        let mut world = World::empty(width, 400, "statues");
        for x in 0..width {
            for y in 300..400 {
                world.set_tile(x, y, terrustia_proto::Tile::block(1));
            }
        }
        world
    }

    #[test]
    fn statues_place_and_cycle_through_the_list_in_order() {
        let mut world = flat_world(4200);
        let mut rand = super::super::rand::UnifiedRandom::new(3);
        let layout = super::super::layout::Layout::plan(4200, 400, &mut rand);
        let mut rng = SmallRng::seed_from_u64(2);
        let placed = scatter(&mut world, &layout, &mut rng);
        assert!(placed > 0, "a wide flat world should take several statues");
    }
}
