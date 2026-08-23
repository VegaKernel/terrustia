//! World generation.
//!
//! This builds a world that can be **played through**: every biome, a dungeon, an underworld, an
//! evil with orbs in it, altars, a temple, chests, life crystals. Not a vanilla-identical world —
//! the same seed here and in Terraria produce different maps — but a complete one.
//!
//! The distinction is the whole design and is worth being plain about. Seed-identical generation
//! means transcribing a hundred and six passes in exact order with exact random-number
//! consumption; it is measured in months and its progress is tracked in `docs/worldgen-parity.md`,
//! steered by the oracle in [`manifest`] and [`passes`]. What is here instead is the fallback
//! that plan names: every structure present, built with our own algorithms, beatable but not
//! identical. It exists because a server that generates unplayable worlds is not a working
//! server, and that is a much nearer target than parity.
//!
//! Passes run in a fixed order and each carves into what the last left:
//!
//! 1. [`layout`] decides where everything goes, before any tile is written.
//! 2. [`terrain`] lays the surface line and the layers, with biome materials over both.
//! 3. Caves are dug through the stone.
//! 4. Ore is seeded into what is left of it, in depth bands.
//! 5. The structures go in: evil chasms, the dungeon, the temple, the hive, the underworld.
//! 6. Altars, life crystals and chests fill the space that is left.
//! 7. Grass, plants and cobwebs finish it.
//!
//! Ordering is load-bearing rather than tidy. Ore seeded before the caves would be hollowed back
//! out; chests placed before the caves would have nowhere to stand.

pub mod layout;
pub mod manifest;
pub mod passes;
pub mod rand;
pub mod structures;
pub mod terrain;
pub mod tiles;

pub use passes::compare_against;

use layout::{Evil, Layout};
use rand::UnifiedRandom;

use super::World;

/// Standard "small" world dimensions.
pub const SMALL_WIDTH: i32 = 4200;
pub const SMALL_HEIGHT: i32 = 1200;

/// What a generated world came out holding.
///
/// Returned so callers — and the tests that guard this — can assert a world is playable rather
/// than merely non-empty. Every count here gates something; see [`structures`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Built {
    pub orbs: usize,
    pub altars: usize,
    pub life_crystals: usize,
    pub chests: usize,
    pub hive: bool,
}

/// Generate a world of the given size.
pub fn generate(width: i32, height: i32, name: impl Into<String>, seed: u64) -> World {
    build(width, height, name, seed).0
}

/// Generate a world, and say what went into it.
pub fn build(width: i32, height: i32, name: impl Into<String>, seed: u64) -> (World, Built) {
    let mut world = World::empty(width, height, name);
    // The same generator the parity work uses, so a seed means the same thing in both.
    let mut rand = UnifiedRandom::new(seed as i32);

    let plan = Layout::plan(width, height, &mut rand);
    world.id = rand.next();
    for byte in &mut world.unique_id {
        *byte = rand.next_max(256) as u8;
    }
    world.crimson = plan.evil == Evil::Crimson;
    world.seed_text = seed.to_string();
    world.surface = plan.surface as i16;
    world.rock_layer = plan.rock as i16;
    world.dungeon_x = Some(plan.dungeon_x);

    let heights = terrain::heightmap(&plan, &mut rand);
    terrain::fill(&mut world, &plan, &heights, &mut rand);

    structures::caves(&mut world, &plan, &mut rand);
    structures::ores(&mut world, &plan, &mut rand);

    let orbs = structures::evil_chasms(&mut world, &plan, &heights, &mut rand);
    structures::dungeon(&mut world, &plan, &heights, &mut rand);
    structures::temple(&mut world, &plan, &mut rand);
    let hive = structures::hive(&mut world, &plan, &mut rand);
    structures::underworld(&mut world, &plan, &mut rand);

    let altars = structures::altars(&mut world, &plan, &mut rand);
    let life_crystals = structures::life_crystals(&mut world, &plan, &mut rand);
    let chests = structures::chests(&mut world, &plan, &mut rand);

    structures::greenery(&mut world, &plan, &heights, &mut rand);
    structures::cobwebs(&mut world, &plan, &mut rand);

    // Spawn goes on the surface in the middle, in a pocket cleared for it.
    let spawn_y = heights[plan.spawn_x as usize];
    world.spawn_x = plan.spawn_x as i16;
    world.spawn_y = spawn_y as i16;
    terrain::clear_spawn(&mut world, plan.spawn_x, spawn_y);
    world.dungeon_y = Some(heights[plan.dungeon_x.clamp(0, width - 1) as usize]);

    let built = Built {
        orbs,
        altars,
        life_crystals,
        chests,
        hive,
    };
    (world, built)
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    fn small() -> (World, Built) {
        build(2400, 900, "test", 1234)
    }

    /// A world has to hold every structure a playthrough needs, or the game cannot be finished
    /// in it. This is the test that makes "playable" mean something.
    #[test]
    fn a_generated_world_can_be_played_through() {
        let (world, built) = small();

        assert!(
            built.orbs >= 3,
            "only {} orbs: three must be smashable to wake the evil's boss",
            built.orbs
        );
        assert!(
            built.altars >= 6,
            "only {} altars: hardmode ore comes from nowhere else",
            built.altars
        );
        assert!(
            built.life_crystals >= 10,
            "only {} life crystals: a hundred hit points is the whole game",
            built.life_crystals
        );
        assert!(
            built.chests >= 30,
            "only {} chests: no starter gear",
            built.chests
        );

        let has = |block: u16| {
            (0..world.width()).step_by(3).any(|x| {
                (0..world.height()).step_by(3).any(|y| world.tile(x, y).block == block)
            })
        };
        assert!(has(tiles::HELLSTONE), "no hellstone: no Wall of Flesh");
        assert!(has(tiles::LIHZAHRD_BRICK), "no temple: no Golem");
        assert!(has(tiles::JUNGLE_GRASS), "no jungle");
        assert!(has(tiles::SNOW), "no snow biome");
        assert!(has(tiles::SAND), "no desert or ocean");
        assert!(
            has(tiles::BLUE_DUNGEON_BRICK)
                || has(tiles::GREEN_DUNGEON_BRICK)
                || has(tiles::PINK_DUNGEON_BRICK),
            "no dungeon: no Skeletron"
        );
        assert!(
            has(tiles::EBONSTONE) || has(tiles::CRIMSTONE),
            "no evil biome"
        );
        assert!(has(tiles::COPPER) && has(tiles::IRON), "no early ore");
        assert!(has(tiles::GOLD) && has(tiles::SILVER), "no deep ore");
    }

    /// The same seed makes the same world, twice running.
    #[test]
    fn a_seed_makes_the_same_world() {
        let (a, built_a) = build(1200, 600, "seeded", 77);
        let (b, built_b) = build(1200, 600, "seeded", 77);
        assert_eq!(built_a, built_b);
        assert_eq!(a.crimson, b.crimson);
        assert_eq!((a.spawn_x, a.spawn_y), (b.spawn_x, b.spawn_y));
        let differing = (0..a.width())
            .step_by(7)
            .flat_map(|x| (0..a.height()).step_by(7).map(move |y| (x, y)))
            .filter(|&(x, y)| a.tile(x, y) != b.tile(x, y))
            .count();
        assert_eq!(differing, 0, "the same seed produced two different worlds");
    }

    /// ...and different seeds make different worlds.
    #[test]
    fn different_seeds_make_different_worlds() {
        let (a, _) = build(1200, 600, "a", 1);
        let (b, _) = build(1200, 600, "b", 2);
        let differing = (0..a.width())
            .step_by(5)
            .flat_map(|x| (0..a.height()).step_by(5).map(move |y| (x, y)))
            .filter(|&(x, y)| a.tile(x, y).block != b.tile(x, y).block)
            .count();
        assert!(
            differing > 500,
            "two seeds differ in only {differing} sampled tiles"
        );
    }

    /// Spawn is somewhere a player can stand: air above, ground below, no water.
    #[test]
    fn spawn_is_somewhere_survivable() {
        for seed in [1u64, 2, 3, 99, 12345] {
            let (world, _) = build(1600, 700, "spawn", seed);
            let (x, y) = (i32::from(world.spawn_x), i32::from(world.spawn_y));
            for above in 1..8 {
                assert!(
                    !world.tile(x, y - above).is_active(),
                    "seed {seed}: spawn is buried at {above} above"
                );
                assert_eq!(
                    world.tile(x, y - above).liquid,
                    0,
                    "seed {seed}: spawn is underwater"
                );
            }
        }
    }

    /// A generated world saves and loads back unchanged, which is what makes it worth generating.
    #[test]
    fn a_generated_world_survives_a_save() {
        use crate::world::{wld, wld_save};
        let (world, built) = build(1200, 600, "roundtrip", 5);
        let bytes = wld_save::serialize(&world).expect("it should save");
        let back = wld::parse(&bytes).expect("it should load");

        assert_eq!(back.width(), world.width());
        assert_eq!(back.height(), world.height());
        assert_eq!(
            back.chests.iter().flatten().count(),
            world.chests.iter().flatten().count(),
            "chests were lost across a save"
        );
        assert!(
            world.chests.iter().flatten().count() >= built.chests,
            "the world should hold at least the cavern chests, plus the dungeon's"
        );
        let differing = (0..world.width())
            .flat_map(|x| (0..world.height()).map(move |y| (x, y)))
            .filter(|&(x, y)| world.tile(x, y) != back.tile(x, y))
            .count();
        assert_eq!(differing, 0, "{differing} tiles changed across a save");
    }

    /// Chests hold something. An empty chest is worse than no chest.
    #[test]
    fn chests_are_not_empty() {
        let (world, _) = small();
        let filled = world
            .chests
            .iter()
            .flatten()
            .filter(|c| c.items.iter().any(|i| !i.is_empty()))
            .count();
        let total = world.chests.iter().flatten().count();
        assert!(total > 0);
        assert_eq!(filled, total, "{} of {total} chests are empty", total - filled);
    }

    /// The world's own flags agree with what was built, since clients and saves read those
    /// rather than the tiles.
    #[test]
    fn the_world_flags_match_what_was_built() {
        let (world, _) = small();
        assert!(world.surface > 0 && world.surface < world.height() as i16);
        assert!(world.rock_layer > world.surface);
        assert!(world.dungeon_x.is_some(), "the dungeon has no recorded x");
        assert!(world.dungeon_y.is_some(), "the dungeon has no recorded y");
        assert!(!world.seed_text.is_empty(), "the seed is not recorded");
    }

    /// Nothing is written outside the world, and the top rows stay sky.
    #[test]
    fn the_sky_is_left_alone() {
        let (world, _) = small();
        for x in (0..world.width()).step_by(11) {
            assert!(
                !world.tile(x, 2).is_active(),
                "column {x} has ground at the very top of the world"
            );
        }
        assert_eq!(world.tile(-1, -1), Tile::AIR, "out of bounds reads as air");
    }
}
