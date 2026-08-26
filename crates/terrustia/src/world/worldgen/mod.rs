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

pub mod cave_flood;
pub mod fallen_logs;
pub mod gem_caves;
pub mod jungle_shrines;
pub mod lakes;
pub mod layout;
pub mod liquid_settle;
pub mod manifest;
pub mod oasis;
pub mod passes;
pub mod piles;
pub mod place_object;
pub mod pots;
pub mod pyramids;
pub mod rand;
pub mod scenery;
pub mod shape_data;
pub mod smooth;
pub mod spider_caves;
pub mod statue_gen;
pub mod structure_map;
pub mod structures;
pub mod surface_plants;
pub mod terrain;
pub mod tiles;
pub mod traps;
pub mod underground_cabins;

pub use passes::compare_against;

use layout::{Evil, Layout};
use rand::UnifiedRandom;
use tiles::{COPPER, GOLD, IRON, SILVER};

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
    /// How many surface lakes were carved.
    pub lakes: usize,
    /// How many trees the forest pass grew.
    pub trees: usize,
    /// Vine tiles hung, and cacti grown.
    pub vines: usize,
    pub cacti: usize,
    pub orbs: usize,
    pub altars: usize,
    pub life_crystals: usize,
    pub chests: usize,
    pub hive: bool,
    pub pots: usize,
    pub statues: usize,
    pub piles: usize,
    pub small_piles: usize,
    pub fallen_logs: usize,
    pub flowers: usize,
    pub mushrooms: usize,
    pub herbs: usize,
    pub sunflowers: usize,
    /// Whether the lakes, oceans and underworld lava all reached a stable rest state. Always
    /// `true` outside a bug in the reused liquid simulator — see [`liquid_settle::Report`].
    pub liquids_converged: bool,
    pub dart_traps: usize,
    pub mines: usize,
    pub geysers: usize,
    pub boulder_traps: usize,
    pub sand_traps: usize,
    /// Tiles the closing smoothing pass turned into slopes, pounded half-tiles or cleared away.
    /// See [`smooth::Report`] for the breakdown; this is [`smooth::Report::total`].
    pub smoothed: usize,
    /// The first Tier 2 batch: gem-lined pockets, desert oases, cobweb-lined spider caves.
    pub gem_caves: usize,
    pub oases: usize,
    pub spider_caves: usize,
    /// Small hollow jungle-grass huts, each holding a chest and a torch.
    pub jungle_shrines: usize,
    /// Solid sandstone-brick masses buried in the desert, each with a tunnel to a treasure room.
    pub pyramids: usize,
    /// Small furnished houses found underground, in whichever material dominates the site.
    pub underground_cabins: usize,
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
    // Shared across every Tier 2 pass that sites a set-piece structure — `GenVars.structures` in
    // vanilla, one session-global instance so a jungle shrine and an underground cabin (once that
    // lands) never overlap each other, the same way they cannot in a real generated world.
    let mut structures = structure_map::StructureMap::new();
    world.id = rand.next();
    for byte in &mut world.unique_id {
        *byte = rand.next_max(256) as u8;
    }
    world.crimson = plan.evil == Evil::Crimson;
    // What `structures::ores` actually lays down. The hardmode three stay unchosen at -1 until
    // the first three altars are broken.
    world.ore_tiers = [
        COPPER as i16,
        IRON as i16,
        SILVER as i16,
        GOLD as i16,
        -1,
        -1,
        -1,
    ];
    world.seed_text = seed.to_string();
    // The sky and the treetops. Rolled here, before the terrain passes, because the tree tops are
    // derived from the backdrops and both go out in packet 7 whatever the tiles turn out like.
    scenery::choose(&mut world, &mut rand);
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

    // A forest, last of the surface passes so it grows on grass that has stopped moving.
    //
    // Generated worlds had no trees whatsoever until this — not fewer than vanilla, none — which
    // is the first thing anyone notices and the hardest to work around, since wood is the first
    // material in the game.
    //
    // Its own generator, seeded from the world's, because the tree pass draws a different number
    // of values than anything before it and threading it through `rand` would move every later
    // pass's numbers for no benefit while parity is not the goal.
    // `::rand` rather than `rand`: this module has its own `rand` submodule, holding the game's
    // generator, and it wins the name.
    let mut forest_rng = {
        use ::rand::SeedableRng;
        ::rand::rngs::SmallRng::seed_from_u64(seed ^ 0x7265_6573)
    };
    // Lakes before the greenery that would otherwise be drowned by them, and before the trees,
    // which refuse to grow where there is water.
    let lakes = lakes::carve(&mut world, &plan, &heights, &mut rand);

    // Settle every lake, ocean edge and underworld lava pool before anything checks "is this tile
    // wet" — trees refuse to grow into water, and until the water they're checking against has
    // actually come to rest, that check is answering a question about a world that no longer
    // exists a moment later. Reuses the runtime liquid simulator rather than vanilla's own
    // generation-time algorithm (see `liquid_settle`'s own doc comment for why); measured at
    // 19.5ms on a real 4200x1200 world, so it costs nothing worth noticing at generation time.
    let liquids = liquid_settle::settle(&mut world);
    debug_assert!(
        liquids.converged,
        "liquid settling did not converge; a generated world would ship with moving water"
    );

    // Desert oases, before anything below gets a chance to drop a decoration onto the desert
    // surface. Its own siting check requires every active tile in a wide scan window to be plain
    // sand, faithful to vanilla — where the same check works because vanilla's own `Oasis` pass
    // (`WorldGen.cs:16339`) runs before essentially every decorative pass, including `Statues`
    // (16962), `PotsGraveyardsAndBoulderPiles` (18123) and, notably, cacti themselves
    // (`CactusPalmTreesAndCoral`, 21488 — the very end of vanilla's own pass list). This module
    // first ran after `plant_undergrowth` (which plants cacti) and, later, after pots/statues too
    // — both left desert columns carrying a decoration the scan window's "must be plain sand"
    // check would fail on, and oases placed zero on every real world tried either way. Moving it
    // here, before any of that runs, matches vanilla's own order and is the actual fix; nothing
    // about `try_place` itself was wrong.
    let oases = oasis::scatter(&mut world, &plan, &mut rand);

    // Pyramids, for the same reason as oases just above: vanilla's own `Pyramids` pass
    // (`WorldGen.cs:15438`) runs before essentially every decoration pass too — earlier than
    // `Oasis` itself, in fact (15438 vs 16339), though the two rarely interact in practice, since
    // a pyramid's own site check only ever looks at the one point it starts digging from, not a
    // wide window the way oasis does.
    let pyramids = pyramids::scatter(&mut world, &plan, &mut rand, &mut forest_rng);

    // The jungle first: vines hang from grass, and until its mud was lined there was almost none.
    crate::world::trees::grass_the_jungle(&mut world);
    let trees = crate::world::trees::plant_forest(&mut world, &mut forest_rng);
    // Vines and cacti reuse the runtime growers, which already know the rules. A jungle without
    // vines reads as a green cave.
    let (vines, cacti) = crate::world::trees::plant_undergrowth(&mut world, &mut forest_rng);

    // Jungle shrines need real jungle grass under a candidate site, so this runs right after the
    // jungle itself is grassed — and before pots/statues/piles below get a chance to clutter a
    // floor tile a shrine's own clearance scan would then refuse.
    let jungle_shrines = jungle_shrines::scatter(&mut world, &plan, &mut structures, &mut rand);

    // Pots, statues, piles and fallen logs: the small object-placement passes built on
    // `place_object`. Ground-truth loot and decoration that makes a cave look excavated rather
    // than merely hollow.
    let pots = pots::scatter(&mut world, &plan, &mut forest_rng);
    let statues = statue_gen::scatter(&mut world, &plan, &mut forest_rng);
    let (piles, small_piles) = piles::scatter(&mut world, &plan, &mut forest_rng);
    let fallen_logs = fallen_logs::scatter(&mut world, &plan, &mut forest_rng);

    // The surface decoration passes: flowers upgrading existing plants, mushroom-cap frames near
    // that biome, alchemy herbs, and sunflowers. All four use the game's own shared generator
    // (`rand`), same as the terrain and structure passes above them, rather than `forest_rng` —
    // matching the convention `structures::greenery` already established for surface plant work.
    let flowers = surface_plants::flowers(&mut world, &mut rand);
    let mushrooms = surface_plants::mushrooms(&mut world, &mut rand);
    let herbs = surface_plants::herbs(&mut world, &mut rand);
    let sunflowers = surface_plants::sunflowers(&mut world, &mut rand);

    // Traps, after every other decoration so a trap never gets sited under a pot, statue or pile
    // that outranks it for the same floor tile — same relative order vanilla's own `Traps` pass
    // has against `Piles`.
    let traps = traps::scatter(&mut world, &plan, &mut forest_rng);

    // The rest of Tier 2's first batch: gem-lined pockets and cobweb-lined spider caves both site
    // into the caves `structures::caves()` already carved (and, since that pass's own fix, left
    // genuinely unwalled) — after every other cave decoration above so they don't overwrite a
    // pot, statue or pile that got there first, matching the same reasoning traps' own ordering
    // comment gives. Oases are placed earlier, above, alongside their own ordering rationale.
    let gem_caves = gem_caves::scatter(&mut world, &plan, &mut rand);
    let spider_caves = spider_caves::scatter(&mut world, &plan, &mut forest_rng);

    // Underground cabins, same reasoning and same relative position as gem/spider caves above:
    // sites into ground the passes above have already decorated, and needs `structures` (already
    // carrying every protected structure placed so far) to avoid overlapping one of them.
    let underground_cabins =
        underground_cabins::scatter(&mut world, &plan, &mut structures, &mut rand);

    // Spawn goes on the surface in the middle, in a pocket cleared for it.
    let spawn_y = heights[plan.spawn_x as usize];
    world.spawn_x = plan.spawn_x as i16;
    world.spawn_y = spawn_y as i16;
    terrain::clear_spawn(&mut world, plan.spawn_x, spawn_y);
    world.dungeon_y = Some(heights[plan.dungeon_x.clamp(0, width - 1) as usize]);

    let chests = chests.saturating_sub(drop_orphaned_chests(&mut world));

    // Smoothing runs last, after every fixture above is already down — see `smooth`'s own doc
    // comment for why that is a deliberate reversal of vanilla's own pass order (vanilla smooths
    // before it decorates) and how `protects_tile_below` covers the difference.
    let smoothed = smooth::smooth(&mut world, &plan, &mut rand).total();

    let built = Built {
        lakes,
        trees,
        vines,
        cacti,
        orbs,
        altars,
        life_crystals,
        chests,
        hive,
        pots,
        statues,
        piles,
        small_piles,
        fallen_logs: fallen_logs.placed,
        flowers,
        mushrooms,
        herbs,
        sunflowers,
        liquids_converged: liquids.converged,
        dart_traps: traps.dart_traps,
        mines: traps.mines,
        geysers: traps.geysers,
        boulder_traps: traps.boulder_traps,
        sand_traps: traps.sand_traps,
        smoothed,
        gem_caves,
        oases,
        spider_caves,
        jungle_shrines,
        pyramids,
        underground_cabins,
    };
    (world, built)
}

/// Drop chest records whose tiles are no longer there, and say how many went.
///
/// Chests are placed part-way through generation and several later passes — greenery, cobwebs and
/// the spawn pocket — write tiles over the top of them. A record left pointing at cleared ground
/// is not harmless: Terraria loads it, then deletes it and its contents on its own first save, so
/// the loot disappears some time after the world was handed over rather than here.
///
/// This is the same footprint check the game runs when it saves (`WorldFile.cs:1620`): all four
/// tiles in bounds, active, and a container.
fn drop_orphaned_chests(world: &mut World) -> usize {
    let mut dropped = 0;
    for slot in 0..world.chests.len() {
        let Some(chest) = world.chests[slot].as_ref() else {
            continue;
        };
        let (x, y) = (i32::from(chest.x), i32::from(chest.y));
        let whole = (0..2).all(|dx| {
            (0..2).all(|dy| {
                world.in_bounds(x + dx, y + dy) && {
                    let tile = world.tile(x + dx, y + dy);
                    tile.is_active() && tile.block == tiles::CHEST
                }
            })
        });
        if !whole {
            world.chests[slot] = None;
            dropped += 1;
        }
    }
    dropped
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
                (0..world.height())
                    .step_by(3)
                    .any(|y| world.tile(x, y).block == block)
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

    /// Every header field survives a save.
    ///
    /// The tile comparison above cannot see a header field the writer drops, and several were
    /// being dropped in silence — the dungeon's y, the seed text, the hardmode ore tiers — because
    /// nothing ever compared them. Each piece was already in place: the generator recorded them,
    /// the parser read them back, and only the writer threw them away.
    ///
    /// This compares the whole header at once and reports every difference in one run, rather than
    /// stopping at the first, so the next field to go missing fails a test instead of a
    /// playthrough.
    #[test]
    fn every_header_field_survives_a_save() {
        for &(width, height) in &[(1200i32, 600i32), (4200, 1200)] {
            check_header_round_trip(width, height);
        }
    }

    /// One world's worth of the check above.
    ///
    /// Split out and run at more than one size deliberately: the header holds variable-length runs
    /// whose lengths depend on the world, so a writer and reader that disagree about one of them
    /// can still agree at one size and not another.
    fn check_header_round_trip(width: i32, height: i32) {
        use crate::world::{wld, wld_save};
        let (world, _) = build(width, height, "header roundtrip", 9);
        let bytes = wld_save::serialize(&world).expect("it should save");
        let back = wld::parse(&bytes).expect("it should load");

        let mut wrong: Vec<String> = Vec::new();
        macro_rules! same {
            ($($field:ident),+ $(,)?) => {$(
                if world.$field != back.$field {
                    wrong.push(format!(
                        "{}: wrote {:?}, read back {:?}",
                        stringify!($field),
                        world.$field,
                        back.$field,
                    ));
                }
            )+};
        }

        same!(
            spawn_x,
            spawn_y,
            surface,
            rock_layer,
            name,
            id,
            unique_id,
            time,
            day_time,
            blood_moon,
            eclipse,
            moon_phase,
            raining,
            rain_time,
            max_rain,
            sandstorm,
            sandstorm_time,
            sandstorm_severity,
            sandstorm_intended_severity,
            dungeon_x,
            dungeon_y,
            pumpkin_moon,
            snow_moon,
            wind,
            crimson,
            ore_tiers,
            progress,
            game_mode,
            world_gen_version,
            seed_text,
            moon_type,
            tree_x,
            tree_style,
            cave_back_x,
            cave_back_style,
            ice_back_style,
            jungle_back_style,
            hell_back_style,
            backgrounds,
            tree_tops,
            num_clouds,
        );

        assert!(
            wrong.is_empty(),
            "{width}x{height}: {} header field(s) changed across a save:\n  {}",
            wrong.len(),
            wrong.join("\n  "),
        );
    }

    /// Every chest record has a chest under it.
    ///
    /// A record whose tiles were carved away by a later pass survives generation, a save and a
    /// load, and is only deleted when Terraria next saves the world — taking the loot with it,
    /// long after the world looked fine.
    ///
    /// This sweeps several sizes and seeds deliberately. A single world is not enough: of the
    /// twenty-one checked when this was written, seventeen had between one and four orphans and
    /// four had none, so one unlucky choice of seed makes the check pass while the bug is
    /// untouched.
    #[test]
    fn every_chest_record_has_tiles_under_it() {
        for &(width, height) in &[(1200i32, 600i32), (4200, 1200)] {
            for seed in 1..6u64 {
                let (world, _) = build(width, height, "chest footprints", seed);
                let orphans: Vec<_> = world
                    .chests
                    .iter()
                    .flatten()
                    .filter(|chest| {
                        let (x, y) = (i32::from(chest.x), i32::from(chest.y));
                        !(0..2).all(|dx| {
                            (0..2).all(|dy| {
                                world.in_bounds(x + dx, y + dy) && {
                                    let tile = world.tile(x + dx, y + dy);
                                    tile.is_active() && tile.block == tiles::CHEST
                                }
                            })
                        })
                    })
                    .map(|chest| (chest.x, chest.y))
                    .collect();
                assert!(
                    orphans.is_empty(),
                    "{width}x{height} seed {seed}: {} chest record(s) point at ground with no \
                     chest on it: {orphans:?}",
                    orphans.len(),
                );
                assert!(
                    world.chests.iter().flatten().count() > 0,
                    "{width}x{height} seed {seed}: the sweep took every chest",
                );
            }
        }
    }

    /// The townsfolk survive a save, with their names and their houses.
    ///
    /// The world file's NPC section was carried through as an opaque blob in both directions, so a
    /// resident was a session-long guest: nobody who moved in was ever written down, and a real
    /// Terraria world's existing residents were invisible to the server entirely.
    #[test]
    fn the_townsfolk_survive_a_save() {
        use crate::world::{objects::TownNpc, wld, wld_save};

        let (mut world, _) = build(1200, 600, "townsfolk", 3);
        world.town_npcs = vec![
            TownNpc {
                net_id: 22,
                name: "Andrew".into(),
                position: (1234.0, 567.0),
                homeless: false,
                home: (77, 88),
                variation: 1,
                homeless_despawn: false,
            },
            TownNpc {
                net_id: 17,
                name: "Wilhelmina".into(),
                position: (99.5, 12.25),
                homeless: true,
                home: (0, 0),
                variation: 0,
                homeless_despawn: true,
            },
        ];
        world.shimmered_town_npcs = vec![22];

        let bytes = wld_save::serialize(&world).expect("it should save");
        let back = wld::parse(&bytes).expect("it should load");

        assert_eq!(back.town_npcs, world.town_npcs, "the residents changed");
        assert_eq!(back.shimmered_town_npcs, world.shimmered_town_npcs);
    }

    /// Banner kill counts survive a save.
    ///
    /// Nothing counted kills at all before this, and the save wrote two zeroes with a comment
    /// saying so — meaning a hundred zombies killed before a restart counted for nothing after it.
    #[test]
    fn banner_kills_survive_a_save() {
        use crate::world::{wld, wld_save};

        let (mut world, _) = build(1200, 600, "banners", 4);
        world.banner_kills.insert(7, 49);
        world.banner_kills.insert(120, 1);
        world.banner_kills.insert(292, 1234);

        let bytes = wld_save::serialize(&world).expect("it should save");
        let back = wld::parse(&bytes).expect("it should load");

        assert_eq!(back.banner_kills.get(&7), Some(&49));
        assert_eq!(back.banner_kills.get(&120), Some(&1));
        assert_eq!(back.banner_kills.get(&292), Some(&1234));
        assert_eq!(
            back.banner_kills.get(&8),
            None,
            "untouched banners stay absent"
        );
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
        assert_eq!(
            filled,
            total,
            "{} of {total} chests are empty",
            total - filled
        );
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
