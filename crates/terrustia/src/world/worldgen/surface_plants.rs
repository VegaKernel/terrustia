//! Flowers, mushrooms, herbs and sunflowers, at generation.
//!
//! The rest of worldgen Tier 1's "Wave A" — the small, easy surface-decoration passes.
//! `structures.rs::greenery()` already places the base plant tile (3) on forest grass, which is
//! vanilla's `GrassPlantsEvilPlantsAndPumpkins`; these four passes decorate what is already there
//! (flowers, mushroom caps) or add a new self-contained object (herbs, sunflowers) rather than
//! touching grass conversion itself.
//!
//! Same discipline as `trees.rs` and `lakes.rs`: transcribe the frame arithmetic and the siting
//! rules literally, cite the vanilla line numbers, and say plainly where something was
//! deliberately simplified rather than silently dropping it.

use terrustia_proto::{Tile, TileFlags};

use super::World;
use super::rand::UnifiedRandom;

/// `WorldGen.cs:20610`, `GenPassNameID.Flowers`.
///
/// Patches of already-placed plant tiles (block 3) get a flower frame and sometimes upgrade to
/// tile 73 (a tall flower). Only the non-remix path is ported — remix worlds are a special seed
/// this server does not generate.
///
/// **Deliberately not ported**: vanilla's `else if` branch, which additionally converts bare ore
/// or stone found just under cleared ground into grass and places a *fresh* plant there, calling
/// `KillTile` on whatever was in the way. That branch has drop-item side effects that only make
/// sense mid-game, not at generation, and it is a terrain-cleanup behaviour bundled into the same
/// closure rather than something a player would recognise as "the flowers pass" specifically. The
/// player-visible core — existing plants sometimes bloom into flowers — is what's here.
///
/// Returns how many plant tiles were upgraded.
pub fn flowers(world: &mut World, rand: &mut UnifiedRandom) -> usize {
    const PLANTS: u16 = 3;
    const TALL_FLOWER: u16 = 73;
    // `genRand.NextFromList<int>(21, 24, 27, 30, 33, 36, 39, 42)` — one of eight flower colour
    // columns in the sprite sheet.
    const STYLES: [i32; 8] = [21, 24, 27, 30, 33, 36, 39, 42];

    let width = world.width();
    if width <= 200 {
        return 0;
    }
    let surface = i32::from(world.surface);
    let attempts = ((f64::from(width)) * 0.004) as i32;
    let mut upgraded = 0usize;

    for _ in 0..attempts {
        let cx = rand.next_range(100, width - 100);
        let half_w = rand.next_range(15, 30);
        let half_h = rand.next_range(15, 30);

        // Walk down from the top of the surface band to the first solid tile in this column,
        // exactly as vanilla's `for (l = num4; l < worldSurface - num4 - 1; l++)` does — an
        // empty range here (a very shallow world) simply finds nothing, matching vanilla falling
        // out of the loop without placing anything.
        let mut anchor = None;
        for l in half_h..(surface - half_h - 1) {
            if world.tile(cx, l).is_active() {
                anchor = Some(l);
                break;
            }
        }
        let Some(l0) = anchor else { continue };

        let style = STYLES[rand.next_max(STYLES.len() as i32) as usize];

        for m in (cx - half_w)..(cx + half_w) {
            for n in (l0 - half_h)..(l0 + half_h) {
                let tile = world.tile(m, n);
                if !tile.is_active() || tile.block != PLANTS {
                    continue;
                }
                let mut grown = tile;
                grown.frame_x = ((style + rand.next_max(3)) * 18) as i16;
                if rand.next_max(3) != 0 {
                    grown.block = TALL_FLOWER;
                }
                world.set_tile(m, n, grown);
                upgraded += 1;
            }
        }
    }
    upgraded
}

/// `WorldGen.cs:20762`, `GenPassNameID.Mushrooms` — the easiest pass in the game. It rewrites
/// `frameX` on *existing* tiles: 3 or 24 (forest/corrupt plants) become 144, and 201 (crimson
/// plants) becomes 270. No new tile is ever placed.
///
/// Returns how many tiles were touched.
pub fn mushrooms(world: &mut World, rand: &mut UnifiedRandom) -> usize {
    const PLANTS: u16 = 3;
    const CORRUPT_PLANTS: u16 = 24;
    const CRIMSON_PLANTS: u16 = 201;

    let width = world.width();
    let height = world.height();
    if width <= 40 {
        return 0;
    }
    let surface = i32::from(world.surface);
    let attempts = ((f64::from(width)) * 0.002) as i32;
    let mut touched = 0usize;

    for _ in 0..attempts {
        let cx = rand.next_range(20, width - 20);
        let half_w = rand.next_range(4, 10);
        let half_h = rand.next_range(15, 30);

        let mut anchor = None;
        for l in 1..(surface - 1) {
            if world.tile(cx, l).is_active() {
                anchor = Some(l);
                break;
            }
        }
        let Some(l0) = anchor else { continue };

        for m in (cx - half_w)..(cx + half_w) {
            if m < 10 || m > width - 10 {
                continue;
            }
            for n in (l0 - half_h)..(l0 + half_h) {
                if n < 0 || n > height - 10 {
                    continue;
                }
                let tile = world.tile(m, n);
                if !tile.is_active() {
                    continue;
                }
                let new_frame_x = match tile.block {
                    PLANTS | CORRUPT_PLANTS => 144,
                    CRIMSON_PLANTS => 270,
                    _ => continue,
                };
                let mut grown = tile;
                grown.frame_x = new_frame_x;
                world.set_tile(m, n, grown);
                touched += 1;
            }
        }
    }
    touched
}

/// `WorldGen.cs:20127`/`PlantAlch` (`WorldGen.cs:46300`), `GenPassNameID.AlchemyHerbs`.
///
/// This is mostly wiring: pick a column, pick a depth band, walk down to the first solid tile,
/// and hand off to [`crate::world::growth::plant_herb`] — which already implements the exact
/// site validation vanilla's `PlantAlch` does (ground active, air above, no liquid above) and the
/// exact density thinning (vanilla: no more than 5 herbs already within a `15 * (W/4200)` box;
/// `plant_herb`'s own 12-tile-radius check is functionally the same rule for a standard-sized
/// world), so none of that needs re-implementing here.
///
/// Returns how many herbs were planted or ripened.
pub fn herbs(world: &mut World, rand: &mut UnifiedRandom) -> usize {
    let width = world.width();
    let height = world.height();
    if width <= 40 || height <= 40 {
        return 0;
    }
    let surface = i32::from(world.surface);
    let rock = i32::from(world.rock_layer);
    let attempts = ((f64::from(width)) * 1.7) as i32;
    let mut planted = 0usize;

    for _ in 0..attempts {
        let x = rand.next_range(20, width - 20);
        // The same three-way depth band `PlantAlch` rolls: deep, below-surface, or anywhere —
        // checked in that order, each only rolled if the one before it didn't fire.
        let mut y = if rand.next_max(40) == 0 {
            rand.next_range((rock + height) / 2, height - 20)
        } else if rand.next_max(10) != 0 {
            rand.next_range(surface, height - 20)
        } else {
            rand.next_range(20, height - 20)
        };
        while y < height - 20 && !world.tile(x, y).is_active() {
            y += 1;
        }
        if crate::world::growth::plant_herb(world, x, y).is_some() {
            planted += 1;
        }
    }
    planted
}

/// `WorldGen.cs:20061`, `GenPassNameID.SunflowersPart2`.
///
/// Returns how many sunflowers were placed.
pub fn sunflowers(world: &mut World, rand: &mut UnifiedRandom) -> usize {
    const GRASS: u16 = 2;

    let width = world.width();
    if width <= 20 {
        return 0;
    }
    let surface_bottom = i32::from(world.surface) - 1;
    let attempts = ((f64::from(width)) * 0.002) as i32;
    let mut placed = 0usize;

    for _ in 0..attempts {
        let centre = rand.next_max(width);
        let x0 = (centre - rand.next_max(10) - 7).max(0);
        let x1 = (centre + rand.next_max(10) + 7).min(width - 1);

        for j in x0..x1 {
            for k in 1..surface_bottom {
                let tile = world.tile(j, k);
                if tile.block == GRASS
                    && tile.is_active()
                    && !world.tile(j, k - 1).is_active()
                    && place_sunflower(world, j, k - 1, rand)
                {
                    placed += 1;
                }
                if world.tile(j, k).is_active() {
                    break;
                }
            }
        }
    }
    placed
}

/// `WorldGen.PlaceSunflower` (`WorldGen.cs:54097`), transcribed rather than routed through the
/// generic `tile_object::tile_object(27)` table entry: the frame arithmetic here is bespoke and
/// does not match `TileObject::frame_of`. The top two rows (the flower head, local rows 0–1)
/// all share **one** style, drawn once per plant; the bottom two rows (the stem, local rows 2–3)
/// draw a **fresh independent** style for every one of their four tiles. Using the generic
/// uniform-style formula would silently lose that stem variation.
fn place_sunflower(world: &mut World, x: i32, y: i32, rand: &mut UnifiedRandom) -> bool {
    const SUNFLOWER: u16 = 27;
    if y > i32::from(world.surface) - 1 {
        return false;
    }
    // A 2-wide, 4-tall footprint above the ground (rows y-3..=y), with the ground tile itself
    // (row y+1) checked separately: active, unslloped, not a half brick, and grass or hallowed
    // grass (2 or 109).
    for i in x..x + 2 {
        for j in (y - 3)..(y + 1) {
            let t = world.tile(i, j);
            if t.is_active() || t.wall > 0 {
                return false;
            }
        }
        let ground = world.tile(i, y + 1);
        if !ground.is_active()
            || ground.slope != 0
            || ground.flags.has(TileFlags::HALF_BRICK)
            || !matches!(ground.block, 2 | 109)
        {
            return false;
        }
    }

    let head_style = rand.next_max(3) as i16;
    for k in 0..2i16 {
        for l in -3..1i16 {
            let frame_x = if l <= -2 {
                k * 18 + head_style * 36
            } else {
                // A fresh, independent draw per stem tile — not `head_style`. This is the exact
                // detail the generic frame table cannot express.
                k * 18 + (rand.next_max(3) as i16) * 36
            };
            let frame_y = (l + 3) * 18;
            world.set_tile(
                x + i32::from(k),
                y + i32::from(l),
                Tile::framed(SUNFLOWER, frame_x, frame_y),
            );
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wide meadow: forest grass with air above and dirt below, sitting well *above* the
    /// nominal surface line — matching vanilla's actual geometry, where `worldSurface` marks
    /// roughly the bottom of the sky layer and real terrain pokes up into the zone above it. The
    /// scans in `flowers`/`mushrooms`/`sunflowers` all walk down from near the top of the map
    /// looking for the first solid tile *above* that line; ground placed exactly at or below
    /// `world.surface` is invisible to every one of them, which is what the first draft of these
    /// tests got wrong.
    fn meadow(width: i32) -> World {
        let mut world = World::empty(width, 500, "surface-plants");
        world.surface = 360;
        world.rock_layer = 400;
        for x in 0..width {
            world.set_tile(x, 300, Tile::block(2)); // GRASS
            for y in 301..320 {
                world.set_tile(x, y, Tile::block(0)); // DIRT
            }
        }
        world
    }

    #[test]
    fn flowers_upgrade_existing_plants_and_never_leave_a_bad_frame() {
        let mut world = meadow(1200);
        // Seed some plant tiles the pass can find and upgrade — greenery() would normally do
        // this, but this test exercises `flowers` in isolation.
        for x in 400..500 {
            world.set_tile(x, 299, Tile::framed(3, 0, 0));
        }
        let mut rand = UnifiedRandom::new(11);
        let upgraded = flowers(&mut world, &mut rand);
        assert!(upgraded > 0, "some seeded plants should have bloomed");

        for x in 0..world.width() {
            for y in 0..world.height() {
                let t = world.tile(x, y);
                if matches!(t.block, 3 | 73) && t.is_active() {
                    assert_ne!(t.frame_x, -1, "unframed plant at {x},{y}");
                    assert_ne!(t.frame_y, -1, "unframed plant at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn mushrooms_rewrite_frames_on_existing_tiles_only() {
        // Seeded across almost the whole width, not one narrow patch: `mushrooms()` only makes
        // `W * 0.002` attempts (a single one for a 600-wide world), each rolling an independent
        // random column, so a patch a fraction of the width wide is not reliably found within one
        // or two attempts. A patch this wide is, without needing a hand-picked lucky seed.
        let width = 600;
        let mut world = meadow(width);
        for x in 20..(width - 20) {
            world.set_tile(x, 299, Tile::framed(3, 0, 0));
        }
        let before_count = (0..world.width())
            .flat_map(|x| (0..world.height()).map(move |y| (x, y)))
            .filter(|&(x, y)| world.tile(x, y).is_active())
            .count();

        let mut rand = UnifiedRandom::new(3);
        let touched = mushrooms(&mut world, &mut rand);
        assert!(touched > 0, "some plants should have been touched");

        let after_count = (0..world.width())
            .flat_map(|x| (0..world.height()).map(move |y| (x, y)))
            .filter(|&(x, y)| world.tile(x, y).is_active())
            .count();
        assert_eq!(
            before_count, after_count,
            "mushrooms must never place or remove a tile"
        );

        let saw_144 = (20..(width - 20)).any(|x| {
            let t = world.tile(x, 299);
            t.block == 3 && t.frame_x == 144
        });
        assert!(
            saw_144,
            "at least one plant in the seeded patch should have become a mushroom cap"
        );
    }

    #[test]
    fn herbs_plant_on_suitable_ground_and_never_leave_a_bad_frame() {
        let mut world = meadow(1200);
        let mut rand = UnifiedRandom::new(5);
        let planted = herbs(&mut world, &mut rand);
        assert!(planted > 0, "a wide meadow should grow at least one herb");

        for x in 0..world.width() {
            for y in 0..world.height() {
                let t = world.tile(x, y);
                if matches!(t.block, 82 | 83) && t.is_active() {
                    assert_ne!(t.frame_x, -1, "unframed herb at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn sunflowers_stand_on_grass_with_a_head_and_a_stem() {
        let mut world = meadow(1200);
        let mut rand = UnifiedRandom::new(2);
        let placed = sunflowers(&mut world, &mut rand);
        assert!(
            placed > 0,
            "a wide open meadow should take at least one sunflower"
        );

        let mut found_full_plant = false;
        for x in 0..world.width() {
            for y in 0..world.height() {
                let t = world.tile(x, y);
                if t.block == 27 && t.is_active() {
                    assert_ne!(t.frame_x, -1, "unframed sunflower tile at {x},{y}");
                    assert_ne!(t.frame_y, -1, "unframed sunflower tile at {x},{y}");
                    found_full_plant = true;
                }
            }
        }
        assert!(found_full_plant, "at least one sunflower tile should exist");
    }

    /// The bespoke stem-frame rule this port exists to get right: the top two rows share one
    /// style, the bottom two rows do not — so directly calling the private placer with a fixed
    /// RNG and checking both regions is the real regression test for this function.
    #[test]
    fn a_sunflowers_head_is_uniform_and_its_stem_is_not_necessarily() {
        let mut world = meadow(40);
        // A clean, guaranteed-empty site, with its ground row comfortably above
        // `world.surface` (360) — `place_sunflower` itself refuses anywhere at or below that
        // line, matching vanilla's `y > worldSurface - 1` check.
        for x in 10..12 {
            for y in 285..301 {
                world.set_tile(x, y, Tile::AIR);
            }
            world.set_tile(x, 301, Tile::block(2));
        }
        let mut rand = UnifiedRandom::new(1);
        assert!(place_sunflower(&mut world, 10, 300, &mut rand));

        // Head: local row 0 (y-3 = row 297). `frame_x = k*18 + head_style*36`, so the two
        // columns differ by exactly the column term (18) when they share one style — not by
        // being equal outright, and not by matching modulo 36 (column 0 is always a multiple of
        // 36; column 1 never is, so that comparison could never pass).
        let head_col0 = world.tile(10, 297).frame_x;
        let head_col1 = world.tile(11, 297).frame_x;
        assert_eq!(
            head_col1 - head_col0,
            18,
            "both head tiles must be built from the same drawn style, {} bytes apart",
            18
        );
        // And the second head row (local row 1, y-2 = row 298) must agree with the first: same
        // style drawn once, not re-rolled per row within the head.
        assert_eq!(
            world.tile(10, 298).frame_x - world.tile(10, 297).frame_x,
            0,
            "the head's style must be the same across both of its rows"
        );
    }
}
