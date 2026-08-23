//! Procedural world generation.
//!
//! Deliberately simple: a rolling surface, a dirt shell over stone, a scattering of ore, and caves.
//! It exists so the connect path is testable before the `.wld` reader lands, and it only uses tile
//! types that are neither frame-important nor batching-exempt, which keeps the section encoder on
//! its simplest path.

pub mod manifest;
pub mod passes;
pub mod rand;

pub use passes::compare_against;

use ::rand::{Rng, SeedableRng, rngs::SmallRng};
use terrustia_proto::Tile;

use super::World;

/// Tile ids used by the generator.
mod tiles {
    pub const DIRT: u16 = 0;
    pub const STONE: u16 = 1;
    pub const GRASS: u16 = 2;
    pub const IRON: u16 = 6;
    pub const COPPER: u16 = 7;
    pub const GOLD: u16 = 8;
    pub const SILVER: u16 = 9;
}

/// Wall ids used by the generator.
mod walls {
    pub const DIRT: u16 = 2;
    pub const STONE: u16 = 1;
}

/// Standard "small" world dimensions.
pub const SMALL_WIDTH: i32 = 4200;
pub const SMALL_HEIGHT: i32 = 1200;

/// Generate a world of the given size.
pub fn generate(width: i32, height: i32, name: impl Into<String>, seed: u64) -> World {
    let mut world = World::empty(width, height, name);
    let mut rng = SmallRng::seed_from_u64(seed);

    world.id = rng.random();
    rng.fill(&mut world.unique_id);
    world.crimson = rng.random_bool(0.5);

    // Surface sits around a third of the way down, as it does in a vanilla world.
    let base = (height as f32 / 3.0).round() as i32;
    let surface_at = |x: i32| -> i32 {
        let x = x as f32;
        let rolling = (x / 190.0).sin() * 14.0 + (x / 47.0).sin() * 5.0 + (x / 13.0).sin() * 1.5;
        base + rolling.round() as i32
    };

    let dirt_depth = 48;
    for x in 0..width {
        let top = surface_at(x);
        for y in top..height {
            let depth = y - top;
            let block = if depth == 0 {
                tiles::GRASS
            } else if depth < dirt_depth {
                tiles::DIRT
            } else {
                tiles::STONE
            };

            // Walls start just below the surface so the top layer still shows sky behind it.
            let wall = if depth < 2 {
                0
            } else if depth < dirt_depth {
                walls::DIRT
            } else {
                walls::STONE
            };

            let mut tile = Tile::block(block);
            tile.wall = wall;
            world.set_tile(x, y, tile);
        }
    }

    carve_caves(&mut world, &mut rng, base, dirt_depth);
    scatter_ore(&mut world, &mut rng, base, dirt_depth);

    // Drop the spawn onto the surface at the middle of the map.
    world.spawn_x = (width / 2) as i16;
    world.spawn_y = surface_at(width / 2) as i16;
    world.surface = base as i16;
    world.rock_layer = (base + dirt_depth) as i16;

    // Clear a small pocket so a player cannot spawn inside the ground.
    for x in (world.spawn_x as i32 - 6)..=(world.spawn_x as i32 + 6) {
        for y in (world.spawn_y as i32 - 8)..world.spawn_y as i32 {
            let mut tile = world.tile(x, y);
            tile.flags.set(terrustia_proto::TileFlags::ACTIVE, false);
            tile.block = 0;
            world.set_tile(x, y, tile);
        }
    }

    world
}

/// Hollow out wandering tunnels below the dirt line.
fn carve_caves(world: &mut World, rng: &mut SmallRng, base: i32, dirt_depth: i32) {
    let cave_count = (world.width() / 120).max(1);
    for _ in 0..cave_count {
        let mut x = rng.random_range(0..world.width()) as f32;
        let mut y = rng.random_range((base + dirt_depth)..world.height()) as f32;
        let mut angle: f32 = rng.random_range(0.0..std::f32::consts::TAU);
        let length = rng.random_range(120..600);

        for _ in 0..length {
            angle += rng.random_range(-0.25..0.25);
            x += angle.cos() * 1.5;
            y += angle.sin() * 1.5;
            if x < 1.0
                || y < (base + 4) as f32
                || x >= (world.width() - 1) as f32
                || y >= (world.height() - 1) as f32
            {
                break;
            }

            let radius = rng.random_range(2..5);
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx * dx + dy * dy > radius * radius {
                        continue;
                    }
                    let (cx, cy) = (x as i32 + dx, y as i32 + dy);
                    let mut tile = world.tile(cx, cy);
                    tile.flags.set(terrustia_proto::TileFlags::ACTIVE, false);
                    tile.block = 0;
                    world.set_tile(cx, cy, tile);
                }
            }
        }
    }
}

/// Sprinkle small ore veins through the stone.
fn scatter_ore(world: &mut World, rng: &mut SmallRng, base: i32, dirt_depth: i32) {
    let ores = [tiles::COPPER, tiles::IRON, tiles::SILVER, tiles::GOLD];
    let vein_count = world.width() * 2;

    for _ in 0..vein_count {
        let ore = ores[rng.random_range(0..ores.len())];
        let x = rng.random_range(0..world.width());
        let y = rng.random_range((base + dirt_depth)..world.height());
        let size = rng.random_range(3..12);

        let (mut cx, mut cy) = (x, y);
        for _ in 0..size {
            if world.tile(cx, cy).block == tiles::STONE && world.tile(cx, cy).is_active() {
                let mut tile = world.tile(cx, cy);
                tile.block = ore;
                world.set_tile(cx, cy, tile);
            }
            cx += rng.random_range(-1..=1);
            cy += rng.random_range(-1..=1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::tile_sets::{allows_batching, frame_important};

    fn small_test_world() -> World {
        // Full-size generation is slow under a debug build; a slice exercises the same code.
        generate(600, 400, "Test", 42)
    }

    #[test]
    fn the_surface_separates_sky_from_ground() {
        let world = small_test_world();
        let x = 300;
        let surface = world.surface as i32;

        // Well above the surface is open sky.
        assert!(!world.tile(x, surface - 40).is_active());
        // Well below it is solid.
        assert!(world.tile(x, surface + 100).is_active());
    }

    #[test]
    fn generated_tiles_stay_on_the_encoder_fast_path() {
        // Every generated type must be plain and batchable, or the section encoder would need
        // frame data we do not produce.
        let world = small_test_world();
        for y in 0..world.height() {
            for x in 0..world.width() {
                let tile = world.tile(x, y);
                if tile.is_active() {
                    assert!(
                        !frame_important(tile.block),
                        "type {} is framed",
                        tile.block
                    );
                    assert!(
                        allows_batching(tile.block),
                        "type {} blocks batching",
                        tile.block
                    );
                }
            }
        }
    }

    #[test]
    fn spawn_is_clear_of_ground() {
        let world = small_test_world();
        let (sx, sy) = (world.spawn_x as i32, world.spawn_y as i32);
        for y in (sy - 8)..sy {
            assert!(!world.tile(sx, y).is_active(), "spawn blocked at y={y}");
        }
    }

    #[test]
    fn generation_is_deterministic_for_a_seed() {
        let a = generate(200, 200, "a", 7);
        let b = generate(200, 200, "b", 7);
        for y in 0..200 {
            for x in 0..200 {
                assert_eq!(a.tile(x, y), b.tile(x, y), "tile ({x}, {y}) differs");
            }
        }
    }

    #[test]
    fn different_seeds_produce_different_worlds() {
        let a = generate(200, 200, "a", 1);
        let b = generate(200, 200, "b", 2);
        let differences = (0..200)
            .flat_map(|y| (0..200).map(move |x| (x, y)))
            .filter(|(x, y)| a.tile(*x, *y) != b.tile(*x, *y))
            .count();
        assert!(differences > 0, "seeds produced identical worlds");
    }

    #[test]
    fn ore_is_underground_and_never_breaks_the_surface() {
        // `rock_layer` is a single nominal depth while the real dirt/stone boundary undulates with
        // the terrain, so the invariant worth asserting is positional relative to each column's
        // own surface, not to that one number.
        let world = small_test_world();
        let is_ore = |x: i32, y: i32| {
            let tile = world.tile(x, y);
            tile.is_active() && matches!(tile.block, 6..=9)
        };

        let mut ore_seen = 0usize;
        for x in 0..world.width() {
            let surface = (0..world.height())
                .find(|y| world.tile(x, *y).is_active())
                .unwrap_or(world.height());
            for y in 0..world.height() {
                if is_ore(x, y) {
                    ore_seen += 1;
                    assert!(
                        y > surface,
                        "ore at ({x}, {y}) is at or above the surface {surface}"
                    );
                }
            }
        }
        assert!(ore_seen > 0, "no ore was generated at all");
    }
}
