//! Meteors: where the next tier of gear comes from.
//!
//! A meteorite lands after the first evil boss falls — the orb that summoned it makes one
//! possible, and every dawn afterwards rolls for it. Mining what it leaves is how a world gets
//! meteorite bars, and with them the Space Gun and the Meteor armour that makes it free to fire.
//!
//! The crater is three passes over the same point: a ball of meteorite, a smaller ball of air
//! hollowed out of its top, and a wider sweep that clears the liquid and knocks off any meteorite
//! left hanging with nothing beside it. The result is a bowl rather than a sphere, which is what
//! makes it walkable.
//!
//! Transcribed from `WorldGen.dropMeteor` and `WorldGen.meteor` in the 1.4.5.7 build.

use rand::Rng;
use rand::rngs::SmallRng;

use super::World;
use terrustia_proto::TileFlags;

/// The tile a meteor is made of.
pub const METEORITE: u16 = 37;

/// How much meteorite a world will hold before it stops sending more, per 4200 tiles of width.
const ENOUGH: f64 = 400.0;

/// How near the spawn a meteor refuses to land, as a fraction of the world's width. Nobody wants
/// one on their house.
const CLEAR_OF_SPAWN: f64 = 0.08;

/// How solid the ground has to be under a candidate site, and how far that standard falls as the
/// search goes on.
const WANTED_SOLID: f64 = 600.0;
const GIVE_UP_BELOW: f64 = 100.0;

/// Whether this world already has as much meteorite as it will take.
pub fn world_is_full(world: &World) -> bool {
    let ceiling = (ENOUGH * f64::from(world.width()) / 4200.0) as i32;
    let mut seen = 0;
    for x in 5..world.width() - 5 {
        for y in 5..i32::from(world.surface) {
            let tile = world.tile(x, y);
            if tile.is_active() && tile.block == METEORITE {
                seen += 1;
                if seen > ceiling {
                    return true;
                }
            }
        }
    }
    false
}

/// Find somewhere for a meteor to land and put one there. Always accepts whatever
/// [`site_is_safe`] allows and never refuses for a player or NPC standing nearby — see
/// [`drop_checked`] for the version that can, which every caller that actually tracks entities
/// should be using instead.
///
/// Returns the centre it landed on. The standard for "solid enough" drops by half a tile every
/// time a site is rejected, so a world that is mostly caves still gets one eventually — but a
/// world with nowhere at all gets none rather than a meteor in the sky.
pub fn drop(world: &mut World, rng: &mut SmallRng) -> Option<(i32, i32)> {
    drop_checked(world, rng, |_, _| false)
}

/// [`drop`], but `blocked_by_entity(x, y)` gets a chance to refuse a candidate centred on `(x,
/// y)` too — vanilla's own player- and NPC-safety-rectangle check (`WorldGen.cs:6324-6345`, a
/// player's own spawn-safe zone widened by `NPC.safeRangeX`/`Y` and each active NPC's hitbox,
/// tested against a box centred on the strike) needs live entity positions this module has no
/// access to on its own.
pub fn drop_checked(
    world: &mut World,
    rng: &mut SmallRng,
    blocked_by_entity: impl Fn(i32, i32) -> bool,
) -> Option<(i32, i32)> {
    if world_is_full(world) {
        return None;
    }
    let keep_away = f64::from(world.width()) * CLEAR_OF_SPAWN;
    let start_y = (f64::from(world.surface) * 0.3) as i32;
    let mut wanted = WANTED_SOLID;

    for _ in 0..world.width() * 5 {
        if wanted < GIVE_UP_BELOW {
            return None;
        }
        // Anywhere but on top of the spawn.
        let mut x = rng.random_range(150..world.width() - 150);
        let mut tries = 0;
        while (f64::from(x) - f64::from(world.spawn_x)).abs() < keep_away && tries < 100 {
            x = rng.random_range(150..world.width() - 150);
            tries += 1;
        }

        // Fall until something solid is underfoot.
        let Some(y) = (start_y..world.height()).find(|&y| {
            let tile = world.tile(x, y);
            tile.is_active() && terrustia_proto::tile_solid::solid(tile.block)
        }) else {
            continue;
        };

        if !site_is_safe(world, x, y) || blocked_by_entity(x, y) {
            continue;
        }

        if solidity_around(world, x, y) >= wanted {
            strike(world, x, y, rng);
            return Some((x, y));
        }
        wanted -= 0.5;
    }
    None
}

/// Whether a meteor may land here at all, beyond how solid the ground is underneath it —
/// vanilla's own tile-based refusals (`WorldGen.cs:6346-6368`), checked across the same
/// thirty-five-tile-radius box the strike itself carves: a chest, a dungeon brick, an altar
/// (26), a Lihzahrd brick (226), a display doll, a hat rack, a fallen log or a teleportation
/// pylon anywhere in it refuses the whole site, not just that one tile — an orphaned chest
/// record, a hole punched through a dungeon wall, or an early Lihzahrd-temple entrance are all
/// worse than a meteor landing a little further off.
fn site_is_safe(world: &World, x: i32, y: i32) -> bool {
    const REACH: i32 = 35;
    for at_x in x - REACH..x + REACH {
        for at_y in y - REACH..y + REACH {
            if !world.in_bounds(at_x, at_y) {
                continue;
            }
            let tile = world.tile(at_x, at_y);
            if !tile.is_active() {
                continue;
            }
            if matches!(
                tile.block,
                21 | 467 // chests (TileID.Sets.BasicChest)
                    | 41 | 43 | 44 | 677 | 678 | 679 // dungeon brick (Main.tileDungeon)
                    | 26   // altar
                    | 226  // Lihzahrd brick
                    | 470  // display doll
                    | 475  // hat rack
                    | 488  // fallen log
                    | 597 // teleportation pylon
            ) {
                return false;
            }
        }
    }
    true
}

/// How much rock there is around a point, minus what is hollow or wet.
///
/// Cloud counts heavily against: a meteor that lands on a floating island falls through it.
fn solidity_around(world: &World, x: i32, y: i32) -> f64 {
    const REACH: i32 = 15;
    let mut score = 0i32;
    for at_x in x - REACH..x + REACH {
        for at_y in y - REACH..y + REACH {
            let tile = world.tile(at_x, at_y);
            if tile.is_active() && terrustia_proto::tile_solid::solid(tile.block) {
                score += 1;
                if matches!(tile.block, 189 | 196 | 202 | 460) {
                    score -= 100;
                }
            } else if tile.liquid > 0 {
                score -= 1;
            }
        }
    }
    f64::from(score)
}

/// Carve the crater.
pub fn strike(world: &mut World, x: i32, y: i32, rng: &mut SmallRng) {
    // A ball of meteorite, flat-topped: nothing above the impact line is filled.
    let radius = rng.random_range(17..23);
    for at_x in x - radius..x + radius {
        for at_y in y - radius..y + radius {
            if at_y <= y + rng.random_range(-2..3) - 5 {
                continue;
            }
            let reach = f64::from(radius) * 0.9 + f64::from(rng.random_range(-4..5));
            if distance(x, y, at_x, at_y) >= reach {
                continue;
            }
            let mut tile = world.tile(at_x, at_y);
            // Vanilla never activates a tile here (`WorldGen.cs:6383-6390`) — it only ever
            // *keeps* one active, by leaving `active` untouched, and only when what was already
            // there was solid. Anything that was not solid (air included: an inactive tile reads
            // as `solid(0) == false`) is explicitly deactivated instead. The net effect: existing
            // solid ground becomes meteorite ore; open air and non-solid decorations stay empty,
            // never filled in — a meteor over a cave carves a hollow ball, not a solid one.
            if !terrustia_proto::tile_solid::solid(tile.block) {
                tile.flags.set(TileFlags::ACTIVE, false);
            }
            tile.flags.set(TileFlags::HALF_BRICK, false);
            tile.slope = 0;
            tile.frame_x = -1;
            tile.frame_y = -1;
            tile.block = METEORITE;
            world.set_tile(at_x, at_y, tile);
        }
    }

    // Then hollow the top out, which is what makes it a bowl you can stand in.
    let radius = rng.random_range(8..14);
    for at_x in x - radius..x + radius {
        for at_y in y - radius..y + radius {
            if at_y <= y + rng.random_range(-2..3) - 4 {
                continue;
            }
            let reach = f64::from(radius) * 0.8 + f64::from(rng.random_range(-3..4));
            if distance(x, y, at_x, at_y) < reach {
                let mut tile = world.tile(at_x, at_y);
                tile.flags.set(TileFlags::ACTIVE, false);
                tile.block = 0;
                world.set_tile(at_x, at_y, tile);
            }
        }
    }

    // A wider sweep: drain the liquid, and knock off any meteorite left hanging in the air.
    let radius = rng.random_range(25..35);
    for at_x in x - radius..x + radius {
        for at_y in y - radius..y + radius {
            let mut tile = world.tile(at_x, at_y);
            let mut touched = false;
            if distance(x, y, at_x, at_y) < f64::from(radius) * 0.7 && tile.liquid > 0 {
                tile.liquid = 0;
                touched = true;
            }
            if tile.is_active() && tile.block == METEORITE {
                let anchored = [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| {
                    let neighbour = world.tile(at_x + dx, at_y + dy);
                    neighbour.is_active() && terrustia_proto::tile_solid::solid(neighbour.block)
                });
                if !anchored {
                    tile.flags.set(TileFlags::ACTIVE, false);
                    tile.block = 0;
                    touched = true;
                }
            }
            if touched {
                world.set_tile(at_x, at_y, tile);
            }
        }
    }
}

fn distance(x: i32, y: i32, at_x: i32, at_y: i32) -> f64 {
    let (dx, dy) = (f64::from(x - at_x), f64::from(y - at_y));
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use terrustia_proto::Tile;

    fn stone_world() -> World {
        let mut world = crate::world::worldgen::generate(1200, 600, "meteor", 3);
        for x in 0..1200 {
            for y in 260..400 {
                world.set_tile(x, y, Tile::block(1));
            }
        }
        world
    }

    /// A meteor lands, and what it leaves is meteorite.
    #[test]
    fn a_meteor_leaves_meteorite() {
        let mut world = stone_world();
        let mut rng = SmallRng::seed_from_u64(4);
        let at = drop(&mut world, &mut rng).expect("nowhere in solid stone to land a meteor");

        let mut meteorite = 0;
        for x in at.0 - 30..at.0 + 30 {
            for y in at.1 - 30..at.1 + 30 {
                if world.tile(x, y).block == METEORITE && world.tile(x, y).is_active() {
                    meteorite += 1;
                }
            }
        }
        assert!(meteorite > 200, "only {meteorite} meteorite tiles");
    }

    /// A meteor never fills existing open space with solid ore — vanilla only ever *keeps* a
    /// tile active if it was already solid (`WorldGen.cs:6383-6390`); open air stays open air.
    ///
    /// The pocket sits 15 tiles straight down from the impact centre: outside the *hollowing*
    /// pass's own largest possible reach (radius 8..13, so at most `13*0.8+3 = 13.4`), which
    /// would otherwise clear it regardless of what the first pass did and make this test
    /// meaningless either way, but within the *filling* pass's radius (17..22) for this seed.
    ///
    /// Fails before the fix: `strike()` set `ACTIVE` on every tile in the ball unconditionally,
    /// so this pocket came out solid ore instead of staying the hollow it started as.
    #[test]
    fn strike_does_not_fill_an_existing_cave_with_ore() {
        let mut world = stone_world();
        let (cx, cy) = (600, 300);
        for x in cx - 1..=cx + 1 {
            for y in cy + 14..=cy + 16 {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        let mut rng = SmallRng::seed_from_u64(11);
        strike(&mut world, cx, cy, &mut rng);

        assert!(
            !world.tile(cx, cy + 15).is_active(),
            "the cave pocket should not have been filled solid by the meteor"
        );
    }

    /// It is a bowl, not a ball: the middle is open so a player can get to the ore.
    #[test]
    fn the_crater_is_hollow_on_top() {
        let mut world = stone_world();
        let mut rng = SmallRng::seed_from_u64(5);
        let (x, y) = drop(&mut world, &mut rng).expect("a meteor");
        let open = (y - 6..y)
            .filter(|&at| !world.tile(x, at).is_active())
            .count();
        assert!(open >= 4, "the crater was sealed over: {open} open tiles");
    }

    /// A world already full of meteorite gets no more.
    #[test]
    fn a_world_stops_taking_meteors() {
        let mut world = stone_world();
        for x in 5..1000 {
            for y in 100..102 {
                world.set_tile(x, y, Tile::block(METEORITE));
            }
        }
        assert!(world_is_full(&world));
        let mut rng = SmallRng::seed_from_u64(6);
        assert!(drop(&mut world, &mut rng).is_none());
    }

    /// A site near a chest, an altar, a dungeon brick or a Lihzahrd brick is refused; one far
    /// enough away is fine — vanilla's own tile-based refusals (`WorldGen.cs:6346-6368`).
    ///
    /// Fails before the fix: `site_is_safe` did not exist, and nothing stopped a meteor cratering
    /// straight through a chest, a dungeon wall, or the temple.
    #[test]
    fn a_site_near_protected_furniture_is_unsafe() {
        let world = stone_world();
        assert!(site_is_safe(&world, 600, 300), "plain stone should be fine");

        for (block, name) in [
            (21u16, "chest"),
            (26, "altar"),
            (41, "dungeon brick"),
            (226, "Lihzahrd brick"),
        ] {
            let mut guarded = stone_world();
            guarded.set_tile(620, 300, Tile::framed(block, 0, 0));
            assert!(
                !site_is_safe(&guarded, 600, 300),
                "a {name} within reach should refuse the site"
            );
            assert!(
                site_is_safe(&guarded, 700, 300),
                "but a {name} far enough away should not"
            );
        }
    }

    /// `drop_checked` also refuses wherever `blocked_by_entity` says a player or NPC is standing
    /// — the piece `drop` itself has no way to check.
    ///
    /// Fails before the fix: `drop_checked` did not exist, and `drop` had no entity-awareness at
    /// all — a meteor could land directly on a player.
    #[test]
    fn drop_checked_refuses_every_entity_blocked_site() {
        let mut world = stone_world();
        let mut rng = SmallRng::seed_from_u64(4);
        assert!(
            drop_checked(&mut world, &mut rng, |_, _| true).is_none(),
            "an entity blocking every candidate should stop it landing anywhere"
        );
    }

    /// ...but with nothing blocking, `drop_checked` behaves exactly like `drop`.
    #[test]
    fn drop_checked_with_nothing_blocking_still_lands() {
        let mut world = stone_world();
        let mut rng = SmallRng::seed_from_u64(4);
        assert!(drop_checked(&mut world, &mut rng, |_, _| false).is_some());
    }

    /// It lands away from the spawn, because nobody wants one on their house.
    #[test]
    fn a_meteor_keeps_away_from_spawn() {
        let mut world = stone_world();
        let mut rng = SmallRng::seed_from_u64(9);
        for _ in 0..6 {
            let Some((x, _)) = drop(&mut world, &mut rng) else {
                break;
            };
            let gap = (x - i32::from(world.spawn_x)).abs();
            assert!(gap > 90, "landed {gap} tiles from spawn");
        }
    }
}
