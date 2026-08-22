//! Whether one box can see another, and whether it is close enough to bother shooting.
//!
//! Both are ports of routines the game shares across many AI styles: `Collision.CanHit` and
//! `NPC.AI_GlobalFiringDistanceCheck`. They live here rather than in a style module because a
//! dozen styles ask the same two questions.

use crate::game::npc::TileView;
use terrustia_proto::tile::TileFlags;
use terrustia_proto::tile_solid::{solid, solid_top};

/// Half-extents of the box the game will shoot within, from `Main.MaxWorldViewSize` shrunk by 50
/// on every side. Everything here is in pixels.
const FIRING_HALF_WIDTH: i32 = (1920 - 100) / 2;
const FIRING_HALF_HEIGHT: i32 = (1200 - 100) / 2;

/// Whether a tile stops a line of sight.
///
/// Platforms do not: they are solid to stand on but you can see and shoot straight through them.
/// Neither do actuated blocks, which are physically absent while the actuator holds them off.
fn opaque(tiles: &impl TileView, x: i32, y: i32) -> bool {
    let tile = tiles.tile(x, y);
    tile.is_active()
        && !tile.flags.has(TileFlags::ACTUATED)
        && solid(tile.block)
        && !solid_top(tile.block)
}

/// As [`opaque`], but for the pair of tiles either side of the walk, which the game additionally
/// lets a line squeeze past when one of them is sloped or a half brick.
fn opaque_flank(tiles: &impl TileView, x: i32, y: i32) -> bool {
    let tile = tiles.tile(x, y);
    opaque(tiles, x, y) && tile.slope == 0 && !tile.flags.has(TileFlags::HALF_BRICK)
}

/// Whether the box at `from` has a clear line to the box at `to`.
///
/// This is `Collision.CanHit`, and it is deliberately not a real ray cast. The game walks one tile
/// at a time along whichever axis has further to go, and blocks only when *both* tiles flanking
/// the step are solid — so a line threads through a one-tile gap that a true ray would clip. The
/// walk stops as soon as it enters a solid tile.
pub fn can_hit(
    tiles: &impl TileView,
    from: (f32, f32),
    from_size: (i32, i32),
    to: (f32, f32),
    to_size: (i32, i32),
) -> bool {
    // The game truncates to a pixel first and only then divides, so a box straddling a tile
    // boundary lands where its own arithmetic puts it.
    let mut x = (from.0 as i32 + from_size.0 / 2) / 16;
    let mut y = (from.1 as i32 + from_size.1 / 2) / 16;
    let goal_x = (to.0 as i32 + to_size.0 / 2) / 16;
    let goal_y = (to.1 as i32 + to_size.1 / 2) / 16;

    // The original clamps into the world and gives up on a missing tile; a `TileView` reads
    // out-of-bounds as air, so the walk is bounded by a step count instead. Two tiles per pixel of
    // the largest world is far beyond any real sight line.
    for _ in 0..20_000 {
        let (run, rise) = ((x - goal_x).abs(), (y - goal_y).abs());
        if x == goal_x && y == goal_y {
            return true;
        }
        if run > rise {
            x += if x >= goal_x { -1 } else { 1 };
            if opaque_flank(tiles, x, y - 1) && opaque_flank(tiles, x, y + 1) {
                return false;
            }
        } else {
            y += if y >= goal_y { -1 } else { 1 };
            if opaque_flank(tiles, x - 1, y) && opaque_flank(tiles, x + 1, y) {
                return false;
            }
        }
        if opaque(tiles, x, y) {
            return false;
        }
    }
    false
}

/// Whether a box overlaps any solid tile.
///
/// This is `Collision.SolidCollision`. Half bricks count only in their lower half, which is why
/// the box test is against a shortened tile rather than a whole one.
pub fn solid_collision(tiles: &impl TileView, position: (f32, f32), size: (i32, i32)) -> bool {
    let left = (position.0 / 16.0) as i32 - 1;
    let right = ((position.0 + size.0 as f32) / 16.0) as i32 + 2;
    let top = (position.1 / 16.0) as i32 - 1;
    let bottom = ((position.1 + size.1 as f32) / 16.0) as i32 + 2;
    for x in left..right {
        for y in top..bottom {
            if !opaque(tiles, x, y) {
                continue;
            }
            let mut tile_top = (y * 16) as f32;
            let mut tile_height = 16.0;
            if tiles.tile(x, y).flags.has(TileFlags::HALF_BRICK) {
                tile_top += 8.0;
                tile_height -= 8.0;
            }
            let tile_left = (x * 16) as f32;
            if position.0 + size.0 as f32 > tile_left
                && position.0 < tile_left + 16.0
                && position.1 + size.1 as f32 > tile_top
                && position.1 < tile_top + tile_height
            {
                return true;
            }
        }
    }
    false
}

/// Whether a shooter is close enough to its target to fire at all.
///
/// This is `NPC.AI_GlobalFiringDistanceCheck`: a rectangle a little inside one screen, centred on
/// the target. It exists so enemies do not spend their shots at things nobody can see.
pub fn within_firing_range(shooter: (f32, f32), target: (f32, f32)) -> bool {
    let (tx, ty) = (target.0 as i32, target.1 as i32);
    let (sx, sy) = (shooter.0 as i32, shooter.1 as i32);
    // `Rectangle.Contains` is inclusive at the low edge and exclusive at the high one; the
    // asymmetry is a pixel, but it is the game's pixel.
    let left = tx - 1920 / 2 + 50;
    let top = ty - 1200 / 2 + 50;
    sx >= left
        && sx < left + FIRING_HALF_WIDTH * 2
        && sy >= top
        && sy < top + FIRING_HALF_HEIGHT * 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc::TILE;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Sparse(HashMap<(i32, i32), Tile>);

    impl Sparse {
        fn wall_column(&mut self, x: i32, ys: std::ops::RangeInclusive<i32>) {
            for y in ys {
                self.0.insert((x, y), Tile::block(1));
            }
        }
    }

    impl TileView for Sparse {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn at(tile_x: i32, tile_y: i32) -> (f32, f32) {
        (tile_x as f32 * TILE, tile_y as f32 * TILE)
    }

    #[test]
    fn open_air_is_always_visible() {
        let world = Sparse::default();
        assert!(can_hit(&world, at(10, 10), (16, 16), at(40, 30), (20, 42)));
    }

    #[test]
    fn a_solid_wall_blocks_the_line() {
        let mut world = Sparse::default();
        world.wall_column(20, 0..=40);
        assert!(!can_hit(&world, at(10, 20), (16, 16), at(30, 20), (20, 42)));
    }

    /// A one-tile hole is not enough, and that is the game's rule rather than an artefact.
    ///
    /// Each sideways step checks the tiles *above and below* the one it lands on, and blocks when
    /// both are solid. Threading a single-tile gap therefore fails, which is why enemies in the
    /// game cannot see you through an arrow slit but can through a doorway.
    #[test]
    fn a_one_tile_hole_in_a_wall_is_too_narrow_to_see_through() {
        let mut world = Sparse::default();
        world.wall_column(20, 0..=40);
        world.0.remove(&(20, 20));
        assert!(!can_hit(&world, at(10, 20), (16, 16), at(30, 20), (20, 42)));
    }

    #[test]
    fn a_two_tile_hole_lets_the_line_through() {
        let mut world = Sparse::default();
        world.wall_column(20, 0..=40);
        world.0.remove(&(20, 20));
        world.0.remove(&(20, 21));
        assert!(can_hit(&world, at(10, 20), (16, 16), at(30, 20), (20, 42)));
    }

    #[test]
    fn platforms_do_not_block_sight() {
        let mut world = Sparse::default();
        for y in 0..=40 {
            world.0.insert((20, y), Tile::framed(19, 0, 0));
        }
        assert!(
            can_hit(&world, at(10, 20), (16, 16), at(30, 20), (20, 42)),
            "tile 19 is solid to stand on but you can shoot through it"
        );
    }

    #[test]
    fn an_actuated_block_does_not_block_sight() {
        let mut world = Sparse::default();
        world.wall_column(20, 0..=40);
        for y in 0..=40 {
            let mut tile = Tile::block(1);
            tile.flags.set(TileFlags::ACTUATED, true);
            world.0.insert((20, y), tile);
        }
        assert!(can_hit(&world, at(10, 20), (16, 16), at(30, 20), (20, 42)));
    }

    #[test]
    fn a_box_in_open_air_is_not_in_anything_solid() {
        let world = Sparse::default();
        assert!(!solid_collision(&world, at(10, 10), (30, 32)));
    }

    #[test]
    fn a_box_overlapping_a_block_is_in_something_solid() {
        let mut world = Sparse::default();
        world.0.insert((10, 11), Tile::block(1));
        assert!(solid_collision(&world, at(10, 10), (30, 32)));
    }

    #[test]
    fn a_platform_is_not_something_to_be_stuck_in() {
        let mut world = Sparse::default();
        world.0.insert((10, 11), Tile::framed(19, 0, 0));
        assert!(!solid_collision(&world, at(10, 10), (30, 32)));
    }

    #[test]
    fn firing_range_is_a_screen_minus_fifty_pixels() {
        let target = (10_000.0, 10_000.0);
        assert!(within_firing_range((10_000.0 + 909.0, 10_000.0), target));
        assert!(!within_firing_range((10_000.0 + 911.0, 10_000.0), target));
        assert!(within_firing_range((10_000.0, 10_000.0 + 549.0), target));
        assert!(!within_firing_range((10_000.0, 10_000.0 + 551.0), target));
    }
}
