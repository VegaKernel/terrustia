//! A shared flood-fill over open cave space, used to site and shape several Tier 2/3 passes.
//!
//! Transcribed from `WorldGen.countTiles`/`nextCount` (`WorldGen.cs:9500-9589`) — the mechanism
//! `GemCaves`, `SpiderCaves`, `LivingTrees` and several other passes all use to answer "how big is
//! the open pocket at this point, and what's in it" before deciding whether to build something
//! there. Vanilla's version is recursive (`nextCount` calls itself on all four neighbours); this is
//! rewritten as an explicit-stack flood fill instead — a pocket up to `max_tiles` deep would recurse
//! that many stack frames in the literal port, which is a real stack-overflow risk this project has
//! no reason to accept for a generation-time-only utility. The traversal order and every stopping
//! rule are unchanged; only the recursion is not.
//!
//! `Spread::Gem`/`Spread::Spider` (`WorldGen.cs:3534` / `3622`) are a *different*, already-iterative
//! flood fill that decorates a pocket rather than just measuring it — see `gem_caves.rs` and
//! `spider_caves.rs`, which each carry their own transcription of those rather than sharing this
//! module, since the two do genuinely different work per tile.
//!
//! **`jungle` widened in for `wall_variety.rs`'s own second caller.** This doc comment used to say
//! "no caller passes `jungle: true` for anything built so far" — `CaveWallsInEnclosedSpaces`'
//! second loop (Tier 3) is the first that does. In vanilla's own `nextCount`, `jungle: true` skips
//! the wall-blocks-the-fill and lava-blocks-the-fill checks entirely (both live inside
//! `if (!jungle) { ... }` in source) — so with `jungle: true`, a walled or lava tile neither halts
//! the fill nor saturates it to the cap; `lava_ok` becomes moot in that mode. Every pre-existing
//! call site passes `jungle: false`, so nothing about their behaviour changes.

use terrustia_proto::tile_solid;

use crate::world::World;

/// What a flood fill found inside the pocket at the seed point.
#[derive(Debug, Clone, Copy, Default)]
pub struct CaveCount {
    pub tiles: usize,
    pub shroom: usize,
    pub lava: usize,
    pub ice: usize,
    pub sand: usize,
    pub rock: usize,
}

/// `countTiles`, `jungle = false`. `lava_ok = true` lets the fill continue through lava instead of
/// stopping dead at it (vanilla's `SpiderCaves` passes `lavaOk: true`; `GemCaves` does not).
///
/// Capped at `max_tiles` — vanilla's own `maxTileCount`, reassigned by each caller before it calls
/// this (300 for gem caves, 3500 for spider caves) — so an open pocket (an ocean, the underworld)
/// cannot make this scan the whole world looking for a bound. Hitting the cap is itself the
/// "reject this site" signal every caller reads off `.tiles`.
pub fn count(
    world: &World,
    x: i32,
    y: i32,
    max_tiles: usize,
    lava_ok: bool,
    jungle: bool,
) -> CaveCount {
    let mut found = CaveCount::default();
    let mut seen: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let mut stack = vec![(x, y)];

    while let Some((cx, cy)) = stack.pop() {
        if found.tiles >= max_tiles {
            break;
        }
        if cx <= 1 || cx >= world.width() - 1 || cy <= 1 || cy >= world.height() - 1 {
            // Vanilla sets numTileCount = maxTileCount and returns — the pocket runs off the edge
            // of the world, so it is rejected the same way an oversized one is.
            found.tiles = max_tiles;
            break;
        }
        if !seen.insert((cx, cy)) {
            continue;
        }
        let tile = world.tile(cx, cy);
        if !jungle && tile.wall != 0 {
            found.tiles = max_tiles;
            break;
        }
        let is_lava = tile.liquid > 0 && tile.liquid_kind == terrustia_proto::Liquid::Lava;
        if is_lava && !jungle {
            found.lava += 1;
            if !lava_ok {
                found.tiles = max_tiles;
                break;
            }
        }
        if tile.is_active() {
            match tile.block {
                super::tiles::MUSHROOM_GRASS => found.shroom += 1,
                super::tiles::STONE => found.rock += 1,
                super::tiles::SNOW | super::tiles::ICE => found.ice += 1,
                super::tiles::SAND | super::tiles::SANDSTONE | super::tiles::HARDENED_SAND => {
                    found.sand += 1
                }
                _ => {}
            }
        }
        if !tile_solid::solid(tile.block) || !tile.is_active() {
            found.tiles += 1;
            stack.push((cx - 1, cy));
            stack.push((cx + 1, cy));
            stack.push((cx, cy - 1));
            stack.push((cx, cy + 1));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use terrustia_proto::Tile;

    #[test]
    fn an_open_pocket_in_solid_rock_is_counted() {
        let mut world = World::empty(200, 200, "flood");
        for x in 0..200 {
            for y in 0..200 {
                world.set_tile(x, y, Tile::block(super::super::tiles::STONE));
            }
        }
        // A 5x5 hollow at the middle.
        for x in 95..100 {
            for y in 95..100 {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        let found = count(&world, 97, 97, 300, false, false);
        assert_eq!(
            found.tiles, 25,
            "a 5x5 open pocket should count all 25 tiles"
        );
    }

    #[test]
    fn a_pocket_touching_the_world_edge_saturates_to_the_cap() {
        let world = World::empty(50, 50, "flood-edge");
        // Everything open, so the fill runs straight off the edge of the world.
        let found = count(&world, 25, 25, 300, false, false);
        assert_eq!(
            found.tiles, 300,
            "a fill that reaches the world edge must hit the cap"
        );
    }

    #[test]
    fn a_walled_tile_saturates_the_fill_immediately() {
        let mut world = World::empty(200, 200, "flood-wall");
        let mut walled = Tile::AIR;
        walled.wall = 1;
        world.set_tile(100, 100, walled);
        let found = count(&world, 100, 100, 300, false, false);
        assert_eq!(
            found.tiles, 300,
            "a walled seed tile must saturate to the cap, per vanilla"
        );
    }

    #[test]
    fn lava_stops_the_fill_unless_lava_ok() {
        // A closed, solid-rock-bounded pocket, same shape as the first test, with one lava tile
        // sitting directly in the seed's path — otherwise nothing bounds the fill and it saturates
        // against the open world instead of against the lava, proving nothing about lava at all.
        let mut world = World::empty(200, 200, "flood-lava");
        for x in 0..200 {
            for y in 0..200 {
                world.set_tile(x, y, Tile::block(super::super::tiles::STONE));
            }
        }
        for x in 95..105 {
            for y in 95..105 {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        let mut lava = Tile::AIR;
        lava.liquid = 255;
        lava.liquid_kind = terrustia_proto::Liquid::Lava;
        world.set_tile(100, 100, lava);

        let blocked = count(&world, 97, 97, 300, false, false);
        assert_eq!(
            blocked.tiles, 300,
            "lava must saturate the fill when lava_ok is false"
        );

        let allowed = count(&world, 97, 97, 300, true, false);
        assert!(
            allowed.tiles < 300 && allowed.lava >= 1,
            "lava_ok should let the fill continue through lava and still count it: {allowed:?}"
        );
    }
}
