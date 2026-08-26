//! Jungle shrines: small hollow huts sited on jungle grass, each holding a chest and a torch.
//!
//! Transcribed from the `JungleShrines` pass (`WorldGen.cs:16071-16226`) and the chest half of
//! `ChestsInJungleShrines` (`WorldGen.cs:17323-17357`) — the loot-table half of that second pass
//! reuses [`super::structures::biome_chest_loot`]'s jungle branch, already built and tested for
//! ordinary buried jungle chests; see that function's own doc comment for the one real
//! simplification this makes.
//!
//! **One real deviation from vanilla's own siting logic.** Vanilla picks a rough half of the world
//! — whichever side the dungeon is *not* on — because at the point this pass runs, vanilla itself
//! has not yet carved a precise jungle region to check against. This generator's [`Layout`]
//! already knows the real jungle band before any pass runs, so shrines are sited directly against
//! `layout.jungle` rather than re-deriving a coarse approximation of it — strictly more accurate
//! than what vanilla's own heuristic stands in for, not a loosening of it.
//!
//! `Main.tileSolid[137] = false` at the very end of vanilla's pass (clearing a runtime solidity
//! override torch placement had set) has no counterpart here: this engine's tile-solidity table is
//! a fixed compile-time bitmask (`terrustia_proto::tile_solid`), not a mutable per-session array,
//! so there is nothing to reset.

use super::layout::Layout;
use super::rand::UnifiedRandom;
use super::structure_map::{Rect, StructureMap};
use super::structures;
use super::tiles;
use crate::world::World;
use terrustia_proto::Tile;

/// The five wood/brick materials a shrine's walls and roof are built from, and the wall behind
/// them — `GenVars.jungleHut`'s five outcomes (`WorldGen.cs:11345-11363`), paired with the matching
/// wall the interior gets lined with (`WorldGen.cs:16120-16140`).
const HUT_MATERIALS: [(u16, u16); 5] = [
    (tiles::IRIDESCENT_BRICK, tiles::walls::IRIDESCENT_BRICK),
    (tiles::MUDSTONE, tiles::walls::MUDSTONE_BRICK),
    (tiles::RICH_MAHOGANY, tiles::walls::RICH_MAHOGANY),
    (tiles::TIN_BRICK, tiles::walls::TIN_BRICK),
    (tiles::GOLD_BRICK, tiles::walls::GOLD_BRICK),
];

/// The `JungleShrines` + chest-loot half of `ChestsInJungleShrines` pass: scatter small hollow
/// huts across the jungle, each with a chest and a torch.
///
/// Returns how many shrines were placed.
pub fn scatter(
    world: &mut World,
    layout: &Layout,
    structures: &mut StructureMap,
    rand: &mut UnifiedRandom,
) -> usize {
    // The clearance scan below reaches 30 tiles either side of a candidate site, and the search
    // needs real room inside `layout.jungle` to draw from — a guard against the tiny synthetic
    // worlds several unrelated tests build, the same shape `traps.rs`/`oasis.rs` already need for
    // their own search bands.
    if layout.jungle.width() <= 90 || layout.height <= (layout.surface + layout.rock) / 2 + 420 {
        return 0;
    }

    let (material, wall) = HUT_MATERIALS[rand.next_max(HUT_MATERIALS.len() as i32) as usize];

    let scaled = f64::from(rand.next_range(7, 12)) * f64::from(layout.width) / 4200.0;
    let mut placed = 0usize;
    let mut i = 0;
    while (i as f64) < scaled {
        let mut tries = 0;
        loop {
            tries += 1;
            if tries > layout.width * 10 {
                // Vanilla's own giving-up condition (`num4 > Main.maxTilesX * 10`) skips to the
                // next shrine rather than looping forever on a jungle with no room left.
                i += 1;
                break;
            }
            let x = rand.next_range(
                layout.jungle.from + 45,
                (layout.jungle.to - 45).max(layout.jungle.from + 46),
            );
            let y = rand.next_range((layout.surface + layout.rock) / 2, layout.height - 400);
            let half_w = rand.next_range(2, 4);
            let half_h = rand.next_range(2, 4);
            let area = Rect::new(x - half_w - 1, y - half_h - 1, half_w + 1, half_h + 1);

            let on_jungle_grass =
                world.tile(x, y).is_active() && world.tile(x, y).block == tiles::JUNGLE_GRASS;
            if !on_jungle_grass {
                continue;
            }
            let mut blocked = false;
            for j in (x - 30..x + 30).step_by(3) {
                for k in (y - 30..y + 30).step_by(3) {
                    if !world.in_bounds(j, k) {
                        blocked = true;
                        continue;
                    }
                    let t = world.tile(j, k);
                    if t.is_active()
                        && matches!(
                            t.block,
                            tiles::HIVE
                                | tiles::HONEY_BLOCK
                                | tiles::LIHZAHRD_BRICK
                                | tiles::IRIDESCENT_BRICK
                                | tiles::MUDSTONE
                        )
                    {
                        blocked = true;
                    }
                    if t.wall == tiles::walls::HIVE || t.wall == tiles::walls::LIHZAHRD_BRICK {
                        blocked = true;
                    }
                }
            }
            if !blocked && !structures.can_place(world, area, 1) {
                blocked = true;
            }
            if blocked {
                continue;
            }

            build_shrine(
                world, structures, rand, x, y, half_w, half_h, material, wall, area,
            );

            // `add_chest` needs solid ground directly beneath its placement point: the interior's
            // hollow runs down to `y + half_h`, and the outer shell's own bottom row, `y + half_h
            // + 1`, is the first solid tile below that — vanilla's own floor-gap loops
            // (`WorldGen.cs:16172-16185`) never actually reach that row, leaving it the real floor
            // a chest (and vanilla's `AddBuriedChest`, searching from the shrine's centre) lands
            // on.
            let chest_x = x + rand.next_max(2);
            let chest_y = y + half_h;
            let loot =
                structures::biome_chest_loot(layout, chest_x, chest_y, rand).unwrap_or_default();
            structures::add_chest(world, chest_x, chest_y, loot, rand);

            placed += 1;
            i += 1;
            break;
        }
    }
    placed
}

/// The hut itself: an outer shell, a hollow interior lined with wall, a torch, a floor gap, a
/// short drip of mud below, and a tapering roof above. `WorldGen.cs:16118-16213`.
#[allow(clippy::too_many_arguments)]
fn build_shrine(
    world: &mut World,
    structures: &mut StructureMap,
    rand: &mut UnifiedRandom,
    x: i32,
    y: i32,
    half_w: i32,
    half_h: i32,
    material: u16,
    wall: u16,
    area: Rect,
) {
    // The outer shell: one tile thick, solid.
    for l in (x - half_w - 1)..=(x + half_w + 1) {
        for m in (y - half_h - 1)..=(y + half_h + 1) {
            world.set_tile(l, m, Tile::block(material));
        }
    }
    // Hollowed out and lined with wall.
    for n in (x - half_w)..=(x + half_w) {
        for k in (y - half_h)..=(y + half_h) {
            let mut t = Tile::AIR;
            t.wall = wall;
            world.set_tile(n, k, t);
        }
    }

    // A torch somewhere inside — vanilla retries `PlaceTile`'s own legality check up to a hundred
    // times; the interior here is freshly hollowed by the loop just above, so a fixed interior
    // point is already guaranteed clear and needs no retry.
    world.set_tile(x, y, Tile::framed(tiles::TORCH, 0, 0));

    // The floor gap: vanilla's own two overlapping clear loops, transcribed as written even
    // though the second is a strict subset of the first's row range.
    for l in (x - half_w - 1)..=(x + half_w + 1) {
        for m in (y + half_h - 2)..=(y + half_h) {
            world.set_tile(l, m, Tile::AIR);
        }
    }
    for l in (x - half_w - 1)..=(x + half_w + 1) {
        for m in (y + half_h - 2)..=(y + half_h - 1) {
            world.set_tile(l, m, Tile::AIR);
        }
    }

    // A short drip of mud hanging below the hut, up to four tiles, stopping at the first tile
    // already occupied.
    for l in (x - half_w - 1)..=(x + half_w + 1) {
        let mut budget = 4;
        let mut m = y + half_h + 2;
        while budget > 0 && world.in_bounds(l, m) && !world.tile(l, m).is_active() {
            world.set_tile(l, m, Tile::block(tiles::MUD));
            m += 1;
            budget -= 1;
        }
    }

    // A tapering roof above, narrowing by one or two columns each row until nothing is left.
    let mut roof_half = half_w - rand.next_range(1, 3);
    let mut roof_y = y - half_h - 2;
    while roof_half > -1 {
        for l in (x - roof_half - 1)..=(x + roof_half + 1) {
            world.set_tile(l, roof_y, Tile::block(material));
        }
        roof_half -= rand.next_range(1, 3);
        roof_y -= 1;
    }

    structures.add_protected_structure(area, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    /// A wide jungle band, thick enough to site a shrine, on a world large enough for the search
    /// bands not to invert.
    fn jungle_world(width: i32, height: i32) -> (World, Layout) {
        let mut world = World::empty(width, height, "jungle-shrines");
        let mut rand = UnifiedRandom::new(1);
        let mut layout = Layout::plan(width, height, &mut rand);
        layout.jungle = super::super::layout::Band {
            from: width / 4,
            to: width / 4 + 900,
        };
        for x in layout.jungle.from..layout.jungle.to {
            for y in (layout.surface + layout.rock) / 2 - 100..layout.height - 300 {
                world.set_tile(x, y, Tile::block(tiles::JUNGLE_GRASS));
            }
        }
        (world, layout)
    }

    #[test]
    fn a_shrine_is_hollow_with_a_chest_and_a_torch() {
        let (mut world, layout) = jungle_world(4200, 1200);
        let mut structures = StructureMap::new();
        let mut rand = UnifiedRandom::new(42);
        let made = scatter(&mut world, &layout, &mut structures, &mut rand);
        assert!(
            made > 0,
            "a wide jungle band should take at least one shrine"
        );

        let chests = world.chests.iter().flatten().count();
        assert!(chests > 0, "no chest was placed in any shrine");

        let mut torches = 0;
        let mut hollow = 0;
        for x in layout.jungle.from..layout.jungle.to {
            for y in 0..world.height() {
                let t = world.tile(x, y);
                if t.is_active() && t.block == tiles::TORCH {
                    torches += 1;
                }
                if !t.is_active() && t.wall != 0 && tiles::walls::GOLD_BRICK <= t.wall {
                    hollow += 1;
                }
            }
        }
        assert!(torches > 0, "no torch was placed in any shrine");
        assert!(hollow > 0, "no hollow, walled interior was carved");
    }

    #[test]
    fn a_small_world_returns_zero_rather_than_panicking() {
        let mut world = World::empty(300, 200, "tiny");
        let mut rand = UnifiedRandom::new(1);
        let mut layout = Layout::plan(300, 200, &mut rand);
        layout.jungle = super::super::layout::Band { from: 50, to: 90 };
        let mut structures = StructureMap::new();
        assert_eq!(scatter(&mut world, &layout, &mut structures, &mut rand), 0);
    }
}
