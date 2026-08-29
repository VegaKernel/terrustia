//! Spawn eligibility for the six NPCs that have to be found before they can become residents.
//!
//! These are deliberately not one shared "underground rescue" pool: vanilla gives each one its
//! own progression and location requirements. Keeping those rules named makes them testable and
//! prevents a generic lottery from producing, for example, a Wizard before Hardmode or a Mechanic
//! in an ordinary cave.

use super::{
    npc::NpcStore,
    spawn::{Biome, Depth},
};
use crate::world::World;

const SPIDER_UNSAFE_WALL: u16 = 62;
const ANGLER_SAND_REACH: i32 = 380;
const CAVERN_RESCUE_BOTTOM_MARGIN: i32 = 210;

/// Whether a rescue NPC is allowed to appear at an already-valid generic spawn candidate.
///
/// Generic room, safe-zone and water checks remain in `spawn::try_spawn`. Secret-seed layer
/// inversions are not guessed here because the project does not yet implement most Remix/Zenith
/// gameplay branches; ordinary-world rules are kept exact rather than pretending partial parity.
pub fn eligible(
    world: &World,
    bound: u16,
    x: i32,
    y: i32,
    depth: Depth,
    biome: Biome,
) -> bool {
    // Surface events replace the ordinary surface spawn pool. A rescue roll must not steal one of
    // those attempts just because the Angler or Tavernkeep happens to be otherwise eligible. The
    // same events do not own underground/cavern spawning, so only Surface is suppressed here.
    if depth == Depth::Surface && (world.eclipse || world.pumpkin_moon || world.snow_moon) {
        return false;
    }

    match bound {
        // Bound Goblin: after the Goblin Army, in Cavern, but above the bottom 210-tile margin.
        105 => {
            world.progress.downed_goblins
                && depth == Depth::Cavern
                && y < world.height() - CAVERN_RESCUE_BOTTOM_MARGIN
        }
        // Bound Wizard: same location band, Hardmode progression gate.
        106 => {
            world.progress.hard_mode
                && depth == Depth::Cavern
                && y < world.height() - CAVERN_RESCUE_BOTTOM_MARGIN
        }
        // Bound Mechanic: Skeletron first, then the Cavern-layer Dungeon.
        123 => {
            world.progress.downed_boss3 && depth == Depth::Cavern && biome == Biome::Dungeon
        }
        // Webbed Stylist: unsafe Spider Wall in Cavern.
        354 => depth == Depth::Cavern && world.tile(x, y).wall == SPIDER_UNSAFE_WALL,
        // Sleeping Angler's dry-land path: sand near a true world edge, above the surface line.
        // Vanilla also has a water-surface route; the server's generic spawn-room test rejects deep
        // water today, so that remains a disclosed generic spawn-physics gap rather than being
        // faked here.
        376 => {
            depth == Depth::Surface
                && (x < ANGLER_SAND_REACH || x > world.width() - ANGLER_SAND_REACH)
                && matches!(world.tile(x, y + 1).block, 53 | 112 | 116 | 234)
        }
        // Unconscious Man / Tavernkeep: after either evil boss, with no special depth requirement.
        579 => world.progress.downed_boss2,
        _ => false,
    }
}

/// Every not-yet-rescued NPC that may actually appear at this candidate tile.
///
/// Availability has three independent gates, all of which matter:
/// - the world has not already recorded the rescue;
/// - this exact candidate satisfies that rescue's progression/location rules;
/// - neither the bound form nor the resident it becomes is already alive.
///
/// The third gate fixes a subtle duplicate hole in the old `spawn::pick_bound`: it checked only
/// whether another bound copy was waiting nearby. A live freed Mechanic, Wizard, etc. therefore did
/// not stop another bound copy being manufactured by the random rescue roll.
pub fn candidates(
    world: &World,
    npcs: &NpcStore,
    x: i32,
    y: i32,
    depth: Depth,
    biome: Biome,
) -> Vec<u16> {
    crate::game::rescues::RESCUES
        .iter()
        .filter(|rescue| crate::game::rescues::still_bound(&world.progress, rescue.bound))
        .filter(|rescue| eligible(world, rescue.bound, x, y, depth, biome))
        .filter(|rescue| {
            !npcs.iter().any(|(_, npc)| {
                npc.is_alive() && (npc.npc_type == rescue.bound || npc.npc_type == rescue.freed)
            })
        })
        .map(|rescue| rescue.bound)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    fn world() -> World {
        let mut world = World::empty(1_000, 800, "bound spawn rules");
        world.surface = 200;
        world.rock_layer = 350;
        world
    }

    #[test]
    fn goblin_and_wizard_are_progression_gated_cavern_rescues() {
        let mut world = world();
        let (x, y) = (500, 400);

        assert!(!eligible(&world, 105, x, y, Depth::Cavern, Biome::Forest));
        world.progress.downed_goblins = true;
        assert!(eligible(&world, 105, x, y, Depth::Cavern, Biome::Forest));
        assert!(!eligible(
            &world,
            105,
            x,
            250,
            Depth::Underground,
            Biome::Forest
        ));

        assert!(!eligible(&world, 106, x, y, Depth::Cavern, Biome::Forest));
        world.progress.hard_mode = true;
        assert!(eligible(&world, 106, x, y, Depth::Cavern, Biome::Forest));

        let too_low = world.height() - CAVERN_RESCUE_BOTTOM_MARGIN;
        assert!(!eligible(
            &world,
            105,
            x,
            too_low,
            Depth::Cavern,
            Biome::Forest
        ));
        assert!(!eligible(
            &world,
            106,
            x,
            too_low,
            Depth::Cavern,
            Biome::Forest
        ));
    }

    #[test]
    fn mechanic_needs_skeletron_and_the_cavern_dungeon() {
        let mut world = world();
        assert!(!eligible(
            &world,
            123,
            500,
            400,
            Depth::Cavern,
            Biome::Dungeon
        ));
        world.progress.downed_boss3 = true;
        assert!(eligible(
            &world,
            123,
            500,
            400,
            Depth::Cavern,
            Biome::Dungeon
        ));
        assert!(!eligible(
            &world,
            123,
            500,
            400,
            Depth::Cavern,
            Biome::Forest
        ));
        assert!(!eligible(
            &world,
            123,
            500,
            250,
            Depth::Underground,
            Biome::Dungeon
        ));
    }

    #[test]
    fn stylist_requires_unsafe_spider_wall_in_cavern() {
        let mut world = world();
        let (x, y) = (500, 400);
        assert!(!eligible(&world, 354, x, y, Depth::Cavern, Biome::Forest));

        let mut spider = Tile::AIR;
        spider.wall = SPIDER_UNSAFE_WALL;
        assert!(world.set_tile(x, y, spider));
        assert!(eligible(&world, 354, x, y, Depth::Cavern, Biome::Forest));
        assert!(!eligible(
            &world,
            354,
            x,
            y,
            Depth::Underground,
            Biome::Forest
        ));
    }

    #[test]
    fn angler_dry_spawn_is_surface_sand_near_a_true_border() {
        let mut world = world();
        let y = 100;
        assert!(world.set_tile(100, y + 1, Tile::block(53)));
        assert!(eligible(
            &world,
            376,
            100,
            y,
            Depth::Surface,
            Biome::Ocean
        ));

        assert!(world.set_tile(500, y + 1, Tile::block(53)));
        assert!(!eligible(
            &world,
            376,
            500,
            y,
            Depth::Surface,
            Biome::Forest
        ));
        assert!(!eligible(
            &world,
            376,
            100,
            250,
            Depth::Underground,
            Biome::Ocean
        ));
    }

    #[test]
    fn surface_events_suppress_surface_rescues_only() {
        let mut world = world();
        world.progress.downed_boss2 = true;
        assert!(eligible(
            &world,
            579,
            500,
            100,
            Depth::Surface,
            Biome::Forest
        ));
        assert!(eligible(
            &world,
            579,
            500,
            400,
            Depth::Cavern,
            Biome::Forest
        ));

        world.eclipse = true;
        assert!(!eligible(
            &world,
            579,
            500,
            100,
            Depth::Surface,
            Biome::Forest
        ));
        assert!(eligible(
            &world,
            579,
            500,
            400,
            Depth::Cavern,
            Biome::Forest
        ));

        world.eclipse = false;
        world.pumpkin_moon = true;
        assert!(!eligible(
            &world,
            579,
            500,
            100,
            Depth::Surface,
            Biome::Forest
        ));

        world.pumpkin_moon = false;
        world.snow_moon = true;
        assert!(!eligible(
            &world,
            579,
            500,
            100,
            Depth::Surface,
            Biome::Forest
        ));
    }

    #[test]
    fn tavernkeep_waits_for_the_evil_boss_but_not_for_a_depth() {
        let mut world = world();
        assert!(!eligible(
            &world,
            579,
            500,
            100,
            Depth::Surface,
            Biome::Forest
        ));
        world.progress.downed_boss2 = true;
        for depth in [
            Depth::Surface,
            Depth::Underground,
            Depth::Cavern,
            Depth::Underworld,
        ] {
            assert!(eligible(&world, 579, 500, 400, depth, Biome::Forest));
        }
    }

    #[test]
    fn a_live_bound_or_freed_form_suppresses_another_copy() {
        let mut world = world();
        world.progress.downed_boss3 = true;
        let mut npcs = NpcStore::new();

        let initial = candidates(
            &world,
            &npcs,
            500,
            400,
            Depth::Cavern,
            Biome::Dungeon,
        );
        assert!(initial.contains(&123), "the Mechanic should initially be findable");

        npcs.spawn(124, (0.0, 0.0))
            .expect("the freed Mechanic should have a slot");
        assert!(
            !candidates(
                &world,
                &npcs,
                500,
                400,
                Depth::Cavern,
                Biome::Dungeon,
            )
            .contains(&123),
            "a live freed Mechanic must suppress another bound Mechanic"
        );

        let mut npcs = NpcStore::new();
        npcs.spawn(123, (0.0, 0.0))
            .expect("the bound Mechanic should have a slot");
        assert!(
            !candidates(
                &world,
                &npcs,
                500,
                400,
                Depth::Cavern,
                Biome::Dungeon,
            )
            .contains(&123),
            "a live bound Mechanic must suppress a duplicate bound Mechanic"
        );
    }

    #[test]
    fn a_recorded_rescue_is_not_offered_again() {
        let mut world = world();
        world.progress.downed_boss3 = true;
        world.progress.saved_mechanic = true;
        let npcs = NpcStore::new();

        assert!(
            !candidates(
                &world,
                &npcs,
                500,
                400,
                Depth::Cavern,
                Biome::Dungeon,
            )
            .contains(&123)
        );
    }
}
