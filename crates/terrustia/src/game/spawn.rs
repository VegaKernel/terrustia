//! Enemy spawning.
//!
//! The rate, cap and spawn area come from the game (`defaultSpawnRate` 600, `defaultMaxSpawns` 5,
//! an area 0.7 screens across with a 0.52-screen safe zone around the player). The pools are
//! transcribed from what `Spawner.SpawnAnNPC` can choose pre-hardmode for each depth, time and
//! biome.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::tile_solid::solid;

use super::{journey::JourneyPowers, npc::NpcStore, player::Player};
use crate::world::World;

/// Everything about the world that changes how fast things spawn.
///
/// The player-carried modifiers — water and peace candles, battle and calming potions, invisibility,
/// the sunflower, the angler set — are deliberately absent: the server does not model a player's
/// inventory or buffs, so it cannot know them. Everything here is world state the server owns.
#[derive(Debug, Clone, Copy)]
pub struct Conditions {
    pub depth: Depth,
    pub hard_mode: bool,
    pub day_time: bool,
    pub blood_moon: bool,
    pub eclipse: bool,
    /// A pumpkin or frost moon, which only matters above ground.
    pub event_moon: bool,
    /// Townsfolk living near the player.
    ///
    /// This is what makes a base safe, and it is the single most player-visible spawn rule in the
    /// game: with nobody home the wilderness comes to your door, and with three residents it stops.
    /// The game suppresses only when nothing else is going on — an invasion, a blood moon, an
    /// eclipse or a moon all overrule it, because an event that a town could turn off would not be
    /// much of an event.
    pub town_npcs: u32,
}

/// The spawn rate and cap for a set of conditions, after `NPC.GetSpawnRate`.
///
/// A flat 600/5 — the game's *surface daytime default* — was being used everywhere, so caverns
/// were about two and a half times too quiet, the underworld half as busy as it should be, and
/// neither hardmode nor a blood moon made any difference at all.
///
/// Returns `(one_in_n_per_tick, cap, spawn_friendly)`. A *lower* rate means more spawning, which
/// is the game's own convention and reads backwards until you know it. `spawn_friendly` is real
/// vanilla's own `spawnFriendly` (`NPC.cs`): when true, this attempt should draw a friendly
/// critter instead of a monster rather than being throttled by the rate at all — see the town
/// suppression block below for where it comes from. `rng` is only ever consulted there; every
/// other modifier in this function is a deterministic fact about the world.
pub fn rates(at: Conditions, rng: &mut SmallRng) -> (u32, f32, bool) {
    let mut rate = SPAWN_RATE as f32;
    let mut max = MAX_SPAWNS;

    if at.hard_mode {
        rate *= 0.9;
        max += 1.0;
    }

    match at.depth {
        Depth::Underworld => max *= 2.0,
        Depth::Cavern => {
            rate *= 0.4;
            max *= 1.9;
        }
        Depth::Underground => {
            let (r, m) = if at.hard_mode {
                (0.45, 1.8)
            } else {
                (0.5, 1.7)
            };
            rate *= r;
            max *= m;
        }
        Depth::Surface => {
            if !at.day_time {
                rate *= 0.6;
                max *= 1.3;
                if at.blood_moon {
                    rate *= 0.3;
                    max *= 1.8;
                }
                if at.event_moon {
                    rate *= 0.2;
                    max *= 2.0;
                }
            } else if at.eclipse {
                rate *= 0.2;
                max *= 1.9;
            }
        }
    }

    // Townsfolk quiet the place down, but only when nothing else is happening: an event overrules
    // them, so a blood moon still comes to a full town. Real vanilla (`NPC.cs:795-924`) is not a
    // flat multiplier here: past the event gate, every attempt is a coin flip between throttling
    // `spawnRate` and leaving it alone while shrinking `maxSpawns` and forcing the spawn to be a
    // friendly critter instead of a monster (`spawnFriendly`). Two things are deliberately not
    // modelled: the underworld's own separate, simpler fork (`NPC.cs:802-855`) — a base built in
    // the underworld is the rare exception, not the case this quiets — and the `ZoneGraveyard`
    // sub-case inside each branch below, since this project has no notion of a graveyard zone at
    // all yet (the same reason `ZonePeaceCandle` and the rest of a player's own buffs are already
    // out of `Conditions`' scope, per this struct's own doc comment).
    let event = at.blood_moon || at.eclipse || at.event_moon;
    let mut spawn_friendly = false;
    if !event {
        match at.town_npcs {
            0 => {}
            // NPC.cs:870-878, the ordinary (non-graveyard) case: a one-in-three chance forces a
            // friendly spawn and shrinks the cap; the other two-in-three simply double the rate.
            1 => {
                if rng.random_ratio(1, 3) {
                    spawn_friendly = true;
                    max *= 0.6;
                } else {
                    rate *= 2.0;
                }
            }
            // NPC.cs:893-901: the odds flip to two-in-three for the friendly fork, and the rate
            // triples on the remaining one-in-three.
            2 => {
                if !rng.random_ratio(1, 3) {
                    spawn_friendly = true;
                    max *= 0.6;
                } else {
                    rate *= 3.0;
                }
            }
            // NPC.cs:917-921: `!Main.expertMode` is unconditionally true in classic mode, so this
            // branch sets `spawnFriendly` on *every* attempt rather than rolling for it —
            // `spawnRate` is never assigned here at all. Expert mode's own further
            // `Main.rand.Next(30) != 0` (a 29-in-30 chance) is folded into the same unconditional
            // case rather than threading a whole difficulty flag through `Conditions` for a
            // 1-in-30 edge: friendly wins the overwhelming majority of the time in expert mode
            // too, and this module already accepts small, disclosed over-approximations like it
            // elsewhere.
            _ => {
                spawn_friendly = true;
                max *= 0.6;
            }
        }
    }

    // The game's own floor and ceiling, which stop a stack of modifiers running away.
    rate = rate.max(SPAWN_RATE as f32 * 0.1);
    max = max.min(MAX_SPAWNS * 3.0);
    (rate as u32, max.max(1.0), spawn_friendly)
}

#[cfg(test)]
mod rate_tests {
    use super::*;
    use rand::SeedableRng;

    fn plain() -> Conditions {
        Conditions {
            depth: Depth::Surface,
            hard_mode: false,
            day_time: true,
            blood_moon: false,
            eclipse: false,
            event_moon: false,
            town_npcs: 0,
        }
    }

    /// A fresh RNG for a call that never touches the town-suppression roll (`town_npcs: 0`, or an
    /// event overruling it) and so does not care which one it gets.
    fn any_rng() -> SmallRng {
        SmallRng::seed_from_u64(0)
    }

    /// Going down makes the world busier, which is most of what depth is for.
    #[test]
    fn caverns_are_busier_than_the_surface() {
        let (surface, surface_cap, _) = rates(plain(), &mut any_rng());
        let (cavern, cavern_cap, _) = rates(
            Conditions {
                depth: Depth::Cavern,
                ..plain()
            },
            &mut any_rng(),
        );
        assert!(
            cavern < surface,
            "a lower rate means more spawning: {cavern} vs {surface}",
        );
        assert!(cavern_cap > surface_cap);
        // The game's figure is 0.4x the rate. A flat 600 everywhere made caves this much too quiet.
        assert_eq!(cavern, (surface as f32 * 0.4) as u32);
    }

    /// Night, a blood moon and an eclipse each raise the surface's rate.
    #[test]
    fn events_make_the_surface_dangerous() {
        let (day, _, _) = rates(plain(), &mut any_rng());
        let (night, _, _) = rates(
            Conditions {
                day_time: false,
                ..plain()
            },
            &mut any_rng(),
        );
        let (blood, blood_cap, _) = rates(
            Conditions {
                day_time: false,
                blood_moon: true,
                ..plain()
            },
            &mut any_rng(),
        );
        let (eclipse, _, _) = rates(
            Conditions {
                eclipse: true,
                ..plain()
            },
            &mut any_rng(),
        );

        assert!(night < day, "night is busier than day");
        assert!(
            blood < night,
            "a blood moon is busier than an ordinary night"
        );
        assert!(eclipse < day, "an eclipse is busier than a plain day");
        assert!(blood_cap > rates(plain(), &mut any_rng()).1);
    }

    /// A town of one or two residents forks every attempt between throttling the rate and
    /// shrinking the cap while forcing a friendly spawn — real vanilla is not a flat multiplier
    /// here (`NPC.cs:870-901`), so this samples many attempts rather than asserting one.
    #[test]
    fn one_or_two_residents_fork_between_a_slower_rate_and_a_smaller_friendly_cap() {
        let (wild, wild_cap, wild_friendly) = rates(plain(), &mut any_rng());
        assert!(
            !wild_friendly,
            "no town at all never forces a friendly spawn"
        );

        let mut rng = SmallRng::seed_from_u64(1);
        for town_npcs in [1, 2] {
            let mut saw_slower_rate = false;
            let mut saw_smaller_friendly_cap = false;
            for _ in 0..200 {
                let (rate, cap, friendly) = rates(
                    Conditions {
                        town_npcs,
                        ..plain()
                    },
                    &mut rng,
                );
                if friendly {
                    assert_eq!(cap, wild_cap * 0.6, "the friendly fork shrinks the cap");
                    assert_eq!(rate, wild, "and leaves the rate exactly where it was");
                    saw_smaller_friendly_cap = true;
                } else {
                    assert!(rate > wild, "the other fork should slow the rate");
                    assert_eq!(cap, wild_cap, "and leaves the cap exactly where it was");
                    saw_slower_rate = true;
                }
            }
            assert!(
                saw_slower_rate,
                "town_npcs {town_npcs}: 200 trials never rolled the rate fork"
            );
            assert!(
                saw_smaller_friendly_cap,
                "town_npcs {town_npcs}: 200 trials never rolled the friendly fork"
            );
        }
    }

    /// Three or more residents is the one headcount classic mode makes fully deterministic:
    /// every attempt forces a friendly spawn and shrinks the cap, and the rate is never touched
    /// at all (`NPC.cs:917-921`) — not throttled further, the way a flat multiplier would.
    #[test]
    fn three_or_more_residents_always_forces_a_friendly_spawn_at_the_unchanged_rate() {
        let (wild, wild_cap, _) = rates(plain(), &mut any_rng());
        let mut rng = SmallRng::seed_from_u64(2);
        for town_npcs in 3..8 {
            let (rate, cap, friendly) = rates(
                Conditions {
                    town_npcs,
                    ..plain()
                },
                &mut rng,
            );
            assert_eq!(rate, wild, "townNPCs >= 3 never assigns spawnRate");
            assert!(friendly, "and always forces a friendly spawn");
            assert_eq!(cap, wild_cap * 0.6);
        }
    }

    /// An event overrules the town: a blood moon still comes to a full street.
    #[test]
    fn an_event_ignores_the_town() {
        let quiet_night = rates(
            Conditions {
                day_time: false,
                town_npcs: 3,
                ..plain()
            },
            &mut any_rng(),
        );
        let blood_night = rates(
            Conditions {
                day_time: false,
                blood_moon: true,
                town_npcs: 3,
                ..plain()
            },
            &mut any_rng(),
        );
        assert!(
            blood_night.0 < quiet_night.0,
            "a town that could switch off a blood moon would not be much of an event",
        );
        assert!(
            !blood_night.2,
            "and an event never forces a friendly spawn either"
        );
    }

    /// However the modifiers stack, they stay inside the game's own floor and ceiling.
    #[test]
    fn the_rate_is_bounded() {
        let worst = rates(
            Conditions {
                depth: Depth::Underworld,
                hard_mode: true,
                day_time: false,
                blood_moon: true,
                eclipse: false,
                event_moon: true,
                town_npcs: 0,
            },
            &mut any_rng(),
        );
        assert!(worst.0 >= (SPAWN_RATE as f32 * 0.1) as u32, "{worst:?}");
        assert!(worst.1 <= MAX_SPAWNS * 3.0, "{worst:?}");
    }
}

/// The baseline: `NPC.defaultSpawnRate`, before anything modifies it.
pub const SPAWN_RATE: u32 = 600;

/// Spawn slots a single player supports.
pub const MAX_SPAWNS: f32 = 5.0;

/// Spawn area around the player, in tiles: roughly 0.7 of a 1080p screen.
pub const SPAWN_RANGE_X: i32 = 84;
pub const SPAWN_RANGE_Y: i32 = 47;

/// Nothing spawns inside this box around the player.
pub const SAFE_RANGE_X: i32 = 62;
pub const SAFE_RANGE_Y: i32 = 35;

/// How deep below the world's bottom the underworld begins.
pub const UNDERWORLD_DEPTH: i32 = 200;

/// Where in the world column a spawn point sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Surface,
    Underground,
    Cavern,
    Underworld,
}

/// Which biome the surrounding tiles say we are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    Forest,
    Corruption,
    Crimson,
    Jungle,
    Snow,
    Desert,
    Ocean,
    Dungeon,
    /// Only exists after the wall falls, which is why nothing recognised it before hardmode did.
    Hallow,
}

/// What hardmode adds where, on top of whatever the place had before.
///
/// Every entry was read out of `NPC.Spawner` with its real condition rather than from memory: the
/// zone it needs, the depth, and whether it wants day or night. A pool that named the wrong biome
/// would be worse than an empty one, because it would look right.
///
/// These *add* to the ordinary pool rather than replacing it — a hardmode forest still has
/// zombies — except in the hallow, which has no pre-hardmode life of its own.
pub fn hardmode_pool(depth: Depth, biome: Biome, day: bool) -> &'static [u16] {
    use Biome::*;
    use Depth::*;

    match (depth, biome) {
        // The two evils, which are the same shape with different names.
        (Surface, Corruption) => &[
            81,  // CorruptSlime
            121, // Slimer
            94,  // Corruptor
        ],
        (Underground | Cavern, Corruption) => &[
            81,  // CorruptSlime
            83,  // CursedHammer
            94,  // Corruptor
            98,  // SeekerHead — a world feeder
            170, // PigronCorruption
            473, // BigMimicCorruption
        ],
        (Surface, Crimson) => &[
            183, // Crimslime
            241, // BloodFeeder
            242, // BloodJelly
        ],
        (Underground | Cavern, Crimson) => &[
            174, // Herpling
            179, // CrimsonAxe
            180, // PigronCrimson
            182, // FloatyGross
            183, // Crimslime
            268, // IchorSticker
            474, // BigMimicCrimson
        ],
        // The hallow, which has nothing but hardmode life.
        (Surface, Hallow) => {
            if day {
                &[
                    75,  // Pixie
                    122, // Gastropod
                ]
            } else {
                &[
                    75,  // Pixie
                    122, // Gastropod
                    137, // IlluminantBat
                    138, // IlluminantSlime
                ]
            }
        }
        (Underground | Cavern, Hallow) => &[
            75,  // Pixie
            84,  // EnchantedSword
            120, // ChaosElemental
            137, // IlluminantBat
            138, // IlluminantSlime
            171, // PigronHallow
            475, // BigMimicHallow
        ],
        // The snow, which gets a great deal.
        (Surface, Snow) => &[
            197, // ArmoredViking
            243, // IceGolem
            250, // AngryNimbus
        ],
        (Underground | Cavern, Snow) => &[
            95,  // DiggerHead
            150, // IceBat
            154, // IceTortoise
            184, // SpikedIceSlime
            197, // ArmoredViking
            206, // IcyMerman
            629, // IceMimic
        ],
        // The jungle.
        (Surface, Jungle) => {
            if day {
                &[
                    177, // Derpling
                    153, // GiantTortoise
                ]
            } else {
                &[
                    152, // GiantFlyingFox
                    153, // GiantTortoise
                ]
            }
        }
        (Underground | Cavern, Jungle) => &[
            157, // Arapaima
            176, // MossHornet
            205, // Moth
            236, // JungleCreeper
            476, // BigMimicJungle
        ],
        // The desert, whose hardmode life is the underground half of it.
        (Underground | Cavern, Desert) => &[
            78,  // Mummy
            79,  // DarkMummy
            80,  // LightMummy
            510, // DuneSplicerHead
        ],
        // The underworld, which only opens up once a mechanical boss is down; the caller holds
        // that gate because it is progression rather than place.
        (Underworld, _) => &[
            151, // Lavabat
            156, // RedDevil
        ],
        // An ordinary forest surface at night.
        (Surface, _) => {
            if day {
                &[]
            } else {
                &[
                    82,  // Wraith
                    93,  // GiantBat
                    133, // WanderingEye
                    140, // PossessedArmor
                ]
            }
        }
        // ...and everything under it.
        (Underground | Cavern, _) => &[
            77,  // ArmoredSkeleton
            85,  // Mimic
            93,  // GiantBat
            110, // SkeletonArcher
            141, // ToxicSludge
            163, // BlackRecluse
            172, // RuneWizard
        ],
    }
}

/// What a blood moon adds to the surface at night.
///
/// It does not replace the night's pool — a blood moon night still has zombies — it widens it, and
/// the widening is what makes one worth fighting through rather than sleeping past. The Clown only
/// comes in hardmode, which is why he is not simply on the list.
pub fn blood_moon_pool(depth: Depth, hard_mode: bool) -> &'static [u16] {
    const EARLY: [u16; 2] = [
        489, // BloodZombie
        490, // Drippler
    ];
    const LATE: [u16; 3] = [
        489, // BloodZombie
        490, // Drippler
        109, // Clown
    ];
    if depth != Depth::Surface {
        return &[];
    }
    if hard_mode { &LATE } else { &EARLY }
}

/// Classify a spawn point by how far down it is.
pub fn depth_at(world: &World, y: i32) -> Depth {
    if y >= world.height() - UNDERWORLD_DEPTH {
        Depth::Underworld
    } else if y >= i32::from(world.rock_layer) {
        Depth::Cavern
    } else if y >= i32::from(world.surface) {
        Depth::Underground
    } else {
        Depth::Surface
    }
}

/// Work out the biome from the tiles around a point, the way the game counts zone tiles.
pub fn biome_at(world: &World, x: i32, y: i32) -> Biome {
    // The ocean is defined by position rather than tiles.
    if x < 250 || x > world.width() - 250 {
        return Biome::Ocean;
    }

    let (mut corrupt, mut crimson, mut jungle, mut snow, mut desert, mut dungeon, mut hallow) =
        (0, 0, 0, 0, 0, 0, 0);
    let radius = 20;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let tile = world.tile(x + dx, y + dy);
            if !tile.is_active() {
                continue;
            }
            match tile.block {
                // Ebonstone, corrupt grass, ebonsand, corrupt ice, corrupt sandstone.
                23 | 25 | 112 | 163 | 400 | 398 => corrupt += 1,
                // Crimstone, crimson grass, crimsand, crimson ice, crimson sandstone.
                199 | 203 | 234 | 200 | 401 | 399 => crimson += 1,
                // Jungle grass and mud.
                59 | 60 => jungle += 1,
                // Snow, ice, slush.
                147 | 161 | 224 => snow += 1,
                // Sand, sandstone, hardened sand.
                53 | 396 | 397 => desert += 1,
                // Dungeon bricks in all three colours.
                41 | 43 | 44 | 481 | 482 | 483 => dungeon += 1,
                // Pearlstone, hallowed grass, pearlsand, hallowed ice, hallowed sandstone.
                109 | 116 | 117 | 164 | 402 | 403 => hallow += 1,
                _ => {}
            }
        }
    }

    // Dungeon wins outright; the rest go to whichever is most represented, with a threshold so a
    // handful of stray blocks does not rename the biome.
    let threshold = 60;
    if dungeon > 20 {
        return Biome::Dungeon;
    }
    let candidates = [
        (corrupt, Biome::Corruption),
        (crimson, Biome::Crimson),
        (jungle, Biome::Jungle),
        (snow, Biome::Snow),
        (desert, Biome::Desert),
        (hallow, Biome::Hallow),
    ];
    candidates
        .iter()
        .filter(|(count, _)| *count >= threshold)
        .max_by_key(|(count, _)| *count)
        .map(|(_, biome)| *biome)
        .unwrap_or(Biome::Forest)
}

/// The enemies that can appear at a given place and time, pre-hardmode.
///
/// Every id here was resolved from `NPCID` by name and checked against the stats table, because
/// the numbers are not guessable: Undead Miner is 44 rather than 52, Sand Slime is 537, and Blood
/// Crawler is 239. The coloured slimes (Green, Purple, Jungle and the rest) are *negative* net ids
/// — variants of Blue Slime — so they are not in these pools; see `docs/protocol-notes.md`.
pub fn pool(depth: Depth, biome: Biome, day: bool) -> &'static [u16] {
    use Biome::*;
    use Depth::*;

    match (depth, biome) {
        // The hallow does not exist before the wall falls, so nothing pre-hardmode lives there of
        // its own. It borrows the forest's pool so a hallowed forest is not silently empty of the
        // ordinary things; what is *only* there in hardmode is in `hardmode_pool`.
        (depth, Hallow) => pool(depth, Forest, day),
        (Underworld, _) => &[
            24, // FireImp
            59, // LavaSlime
            60, // Hellbat
            62, // Demon
            66, // VoodooDemon
            39, // BoneSerpentHead
        ],
        (_, Dungeon) => &[
            31, // AngryBones
            32, // DarkCaster
            34, // CursedSkull
            71, // DungeonSlime
        ],
        (_, Ocean) => &[
            67,  // Crab
            63,  // BlueJellyfish
            64,  // PinkJellyfish
            65,  // Shark
            221, // Squid
            55,  // Goldfish
        ],
        (Surface, Corruption) => &[
            6,  // EaterofSouls
            7,  // DevourerHead
            47, // CorruptBunny
        ],
        (_, Corruption) => &[
            6,  // EaterofSouls
            7,  // DevourerHead
            81, // CorruptSlime
        ],
        (Surface, Crimson) => &[
            173, // Crimera
            181, // FaceMonster
        ],
        (_, Crimson) => &[
            173, // Crimera
            181, // FaceMonster
            239, // BloodCrawler
        ],
        (Surface, Jungle) => &[
            42, // Hornet
            51, // JungleBat
        ],
        (_, Jungle) => &[
            42, // Hornet
            43, // ManEater
            56, // Snatcher
            51, // JungleBat
        ],
        (Surface, Snow) => {
            if day {
                &[
                    147, // IceSlime
                    185, // SnowFlinx
                ]
            } else {
                &[
                    161, // ZombieEskimo
                    167, // UndeadViking
                    147, // IceSlime
                ]
            }
        }
        (_, Snow) => &[
            147, // IceSlime
            185, // SnowFlinx
            167, // UndeadViking
            150, // IceBat
        ],
        (Surface, Desert) => {
            if day {
                &[
                    61,  // Vulture
                    537, // SandSlime
                ]
            } else {
                &[
                    61,  // Vulture
                    537, // SandSlime
                    3,   // Zombie
                ]
            }
        }
        (_, Desert) => &[
            537, // SandSlime
            69,  // Antlion
            580, // WalkingAntlion
        ],
        (Surface, Forest) => {
            if day {
                &[
                    1,   // BlueSlime
                    46,  // Bunny
                    74,  // Bird
                    299, // Squirrel
                    361, // Frog
                ]
            } else {
                &[
                    3, // Zombie
                    2, // DemonEye
                ]
            }
        }
        (Underground, _) => &[
            1,   // BlueSlime
            16,  // MotherSlime
            10,  // GiantWormHead
            44,  // UndeadMiner
            498, // Salamander
        ],
        (Cavern, _) => &[
            21,  // Skeleton
            49,  // CaveBat
            44,  // UndeadMiner
            16,  // MotherSlime
            10,  // GiantWormHead
            93,  // GiantBat
            498, // Salamander
        ],
    }
}

/// How far down the game looks for ground from a chosen point (`FindGroundTile`).
pub const GROUND_SCAN: i32 = 30;

/// Scan downward for the first solid tile, returning its row.
///
/// The game does this rather than requiring a random point to land exactly on the surface: at any
/// column there is usually one standable row in a 90-tile band, so picking blind would almost
/// never find it.
pub fn find_ground(world: &World, x: i32, from_y: i32) -> Option<i32> {
    (from_y..from_y + GROUND_SCAN).find(|&y| {
        let tile = world.tile(x, y);
        tile.is_active() && solid(tile.block)
    })
}

/// Whether an NPC can stand at this tile: open space with something solid underneath.
fn has_room(world: &World, x: i32, y: i32) -> bool {
    for dy in 0..3 {
        let tile = world.tile(x, y - dy);
        if tile.is_active() && solid(tile.block) {
            return false;
        }
        if tile.liquid > 200 {
            return false; // deep water is not a walking spot
        }
    }
    let floor = world.tile(x, y + 1);
    floor.is_active() && solid(floor.block)
}

/// Pick spawns for this tick.
///
/// Returns the types and pixel positions to create; the caller owns the NPC table, so this stays a
/// pure decision and is straightforward to test.
/// What the events running right now do to the spawn pool.
///
/// A moon or an eclipse does not add to the ordinary pool — it replaces it on the surface, which
/// is why standing outside during one is a different game and standing in a cave is not.
pub struct EventSpawns<'a> {
    /// Which moon is up, and which wave it is on.
    pub moon: Option<(crate::game::moons::Moon, i32)>,
    /// Whether a solar eclipse is happening.
    pub eclipse: bool,
    pub downed_plantera: bool,
    pub downed_all_mechs: bool,
    /// Whether the field already holds as many event bosses as it will take.
    pub boss_cap: bool,
    /// Whether the wall has fallen, which is what opens the hardmode half of every pool.
    pub hard_mode: bool,
    /// ...and whether a mechanical boss is down, which is what opens the underworld's.
    pub downed_mech_any: bool,
    /// How many of a type are alive, for the tables that cap their heavies.
    pub census: &'a dyn Fn(u16) -> usize,
    /// The six cavern enemies this particular world has.
    ///
    /// Not part of the pool tables because they are not the same in every world: each world draws
    /// six of the thirteen from its own id. Two worlds therefore feel different underground, and
    /// a player who knows theirs has Salamanders and no Crawdads is right about that permanently.
    pub cavern_monsters: crate::game::cavern_monsters::CavernMonsters,
}

impl EventSpawns<'_> {
    /// Whether anything is running that overrides the surface pool.
    fn running(&self) -> bool {
        self.moon.is_some() || self.eclipse
    }
}

/// How many townsfolk are close enough to a point to quiet it down.
///
/// The game counts them through `SceneMetrics`, which is roughly what is on screen. This uses a
/// radius in the same neighbourhood: far enough that a house nearby counts, close enough that a
/// town on the other side of the world does not make the whole map safe.
fn town_npcs_near(npcs: &NpcStore, at: (f32, f32)) -> u32 {
    /// Tiles, converted to pixels below.
    const REACH: f32 = 100.0 * 16.0;

    npcs.iter()
        .filter(|(_, npc)| npc.stats.town_npc && npc.is_alive())
        .filter(|(_, npc)| {
            (npc.position.0 - at.0).abs() < REACH && (npc.position.1 - at.1).abs() < REACH
        })
        .count() as u32
}

/// One spawn attempt in this many considers a bound townsperson instead of an enemy.
///
/// Deliberately steep. A handful of them exist in a world's whole lifetime and each is a resident
/// you cannot otherwise have, so they want to be a find rather than a fixture.
const BOUND_RARITY: u32 = 120;

/// Whether a bound townsperson may be found at this depth, biome and spot.
///
/// Each gate is the real `NPC.Spawner.SpawnNPC` condition for that bound NPC rather than "anywhere
/// underground": without them the Wizard, Mechanic and Goblin Tinkerer were all findable on day
/// one, skipping the hardmode / Skeletron / goblin-army progression the game puts in front of
/// them. The gates key on world progress the server already tracks, plus the depth and biome of
/// the candidate spot. `spawn_y` is the tile row, used for the Mechanic's exact depth threshold.
///
/// Two deliberate narrowings, both because the server does not model the tile the real gate reads:
/// the Stylist's real gate is a spider-nest wall (wall 62, `NPC.cs:1662-1671`), approximated here
/// as "the caverns"; and the Angler's is the ocean surface/water (`NPC.cs:1778-1928`), so he is
/// gated to the ocean biome and is therefore never *mis*-found in a cave, even though this
/// underground-only bound path never reaches the ocean to place him (a disclosed gap, not a fake).
pub fn bound_gate(bound: u16, world: &World, depth: Depth, biome: Biome, spawn_y: i32) -> bool {
    let p = &world.progress;
    match bound {
        // Goblin Tinkerer: the goblin army beaten, deeper than the rock layer but above the
        // underworld (`NPC.cs:2087`: downedGoblins && deeperThanRockLayer && spawnTileY < maxTilesY-210).
        105 => p.downed_goblins && depth == Depth::Cavern,
        // Wizard: hardmode, same caverns band (`NPC.cs:2091`).
        106 => p.hard_mode && depth == Depth::Cavern,
        // Mechanic: Skeletron beaten, below (worldSurface*4 + rockLayer)/5 (`NPC.cs:2656`).
        123 => {
            let threshold = (f64::from(world.surface) * 4.0 + f64::from(world.rock_layer)) / 5.0;
            p.downed_boss3 && f64::from(spawn_y) > threshold
        }
        // Stylist: the spider nest, approximated as the caverns (`NPC.cs:1662-1671`; see above).
        354 => depth == Depth::Cavern,
        // Angler: the ocean (`NPC.cs:1778-1928`; see above).
        376 => biome == Biome::Ocean,
        // Bartender: the Old One's Army becomes available once the Eater of Worlds / Brain of
        // Cthulhu is down (`NPC.cs:1658`, `DD2Event.ReadyToFindBartender => NPC.downedBoss2`).
        579 => p.downed_boss2,
        // Golfer: the underground desert (`NPC.cs:1682-1697`).
        589 => biome == Biome::Desert && matches!(depth, Depth::Underground | Depth::Cavern),
        _ => false,
    }
}

/// Somebody still tied up somewhere in this world, if any are left to find here.
///
/// Refuses anyone already rescued, anyone already standing about waiting to be talked to (so a
/// world cannot end up with two Mechanics or a corridor full of bound wizards), and anyone whose
/// real progression / biome / depth gate this spot does not satisfy.
fn pick_bound(
    world: &World,
    npcs: &NpcStore,
    depth: Depth,
    biome: Biome,
    spawn_y: i32,
    rng: &mut SmallRng,
) -> Option<u16> {
    let waiting: Vec<u16> = crate::game::rescues::RESCUES
        .iter()
        .map(|r| r.bound)
        .filter(|bound| crate::game::rescues::still_bound(&world.progress, *bound))
        .filter(|bound| bound_gate(*bound, world, depth, biome, spawn_y))
        .filter(|bound| {
            !npcs
                .iter()
                .any(|(_, n)| n.npc_type == *bound && n.is_alive())
        })
        .collect();
    if waiting.is_empty() {
        return None;
    }
    Some(waiting[rng.random_range(0..waiting.len())])
}

pub fn try_spawn(
    world: &World,
    npcs: &NpcStore,
    players: &[Option<Player>],
    events: &EventSpawns<'_>,
    journey: &JourneyPowers,
    rng: &mut SmallRng,
    _ticks: u64,
) -> Vec<(u16, (f32, f32))> {
    let active: Vec<&Player> = players
        .iter()
        .flatten()
        .filter(|p| p.is_playing() && p.life > 0)
        .collect();
    if active.is_empty() {
        return Vec::new();
    }

    // The cap grows with the number of players, as it does in the game, and with wherever the
    // deepest of them is standing: a cavern holds far more at once than a forest.
    let deepest = active
        .iter()
        .map(|p| depth_at(world, (p.position.1 / 16.0) as i32))
        .max_by_key(|d| match d {
            Depth::Surface => 0,
            Depth::Underground => 1,
            Depth::Cavern => 2,
            Depth::Underworld => 3,
        })
        .unwrap_or(Depth::Surface);
    // The cap uses the *least* protected player, so one person out in the wild is not sheltered
    // by everybody else standing in town.
    let loneliest = active
        .iter()
        .map(|p| town_npcs_near(npcs, p.position))
        .min()
        .unwrap_or(0);
    let (_, band, _) = rates(
        Conditions {
            depth: deepest,
            hard_mode: world.progress.hard_mode,
            day_time: world.day_time,
            blood_moon: world.blood_moon,
            eclipse: world.eclipse,
            event_moon: world.pumpkin_moon || world.snow_moon,
            town_npcs: loneliest,
        },
        rng,
    );
    let cap = band * (1.0 + 0.3 * active.len() as f32);
    if npcs.used_slots() >= cap {
        return Vec::new();
    }

    let mut out = Vec::new();
    for player in active {
        let (px, py) = (
            (player.position.0 / 16.0) as i32,
            (player.position.1 / 16.0) as i32,
        );

        // Journey mode's `SpawnRate`, gated on the world's own difficulty being literally
        // Journey (`Main.IsJourneyMode` — every one of its five real vanilla call sites checks
        // this before reading the power at all; the power itself has no effect outside a Journey
        // world, even for a player who somehow has it set). Both real effects — a hard "spawns
        // off" at the slider's exact floor, and the ordinary rate scaling otherwise — are checked
        // here; only the *rate*, not the shared cap above, is adjusted per player — this
        // function's own cap is already one number shared across every active player rather than
        // vanilla's fully independent per-player `maxSpawns`, an existing simplification predating
        // this power, not something worth restructuring just to extend one Journey slider into.
        let journey_world = world.game_mode == 3;
        if journey_world && journey.spawns_disabled(player.slot) {
            continue;
        }

        // The rate is the player's own, not one number for the world: two people in the same
        // world can be standing in a quiet forest and a busy cavern at the same moment.
        let (mut rate, _, spawn_friendly) = rates(
            Conditions {
                depth: depth_at(world, py),
                hard_mode: world.progress.hard_mode,
                day_time: world.day_time,
                blood_moon: world.blood_moon,
                eclipse: world.eclipse,
                event_moon: world.pumpkin_moon || world.snow_moon,
                town_npcs: town_npcs_near(npcs, player.position),
            },
            rng,
        );
        if journey_world {
            let multiplier = journey.spawn_rate_multiplier(player.slot);
            rate = ((rate as f32) / multiplier).max(1.0) as u32;
        }
        if rng.random_range(0..rate.max(1)) != 0 {
            continue;
        }
        // `spawnFriendly` (`NPC.cs:795-924`, see `rates`'s own doc): this attempt should draw a
        // friendly critter instead of a monster. This project has no friendly-critter table to
        // draw from yet — a disclosed, separate gap, not this one's to close — so the honest thing
        // is to skip the attempt rather than spawn a monster real vanilla would not have here.
        // What the town suppression this item exists to fix is actually for — fewer hostile
        // spawns near a populated base — still holds either way.
        if spawn_friendly {
            continue;
        }

        // Try a handful of candidate tiles rather than scanning the whole area.
        for _ in 0..20 {
            let x = px + rng.random_range(-SPAWN_RANGE_X..=SPAWN_RANGE_X);
            let from_y = py + rng.random_range(-SPAWN_RANGE_Y..=SPAWN_RANGE_Y);
            if x < 10 || from_y < 10 || x >= world.width() - 10 || from_y >= world.height() - 40 {
                continue;
            }

            // Drop to whatever ground is under the chosen point, then stand on top of it.
            let Some(ground) = find_ground(world, x, from_y) else {
                continue;
            };
            let y = ground - 1;

            // Never spawn on top of somebody.
            if (x - px).abs() < SAFE_RANGE_X && (y - py).abs() < SAFE_RANGE_Y {
                continue;
            }
            if !has_room(world, x, y) {
                continue;
            }

            let depth = depth_at(world, y);
            // An event owns the surface while it runs, and nothing below it.
            let event_type = if events.running() && depth == Depth::Surface {
                match (events.moon, events.eclipse) {
                    (Some((moon, wave)), _) if !world.day_time => crate::game::moons::moon_spawn(
                        moon,
                        wave,
                        events.census,
                        events.boss_cap,
                        rng,
                    ),
                    (_, true) if world.day_time => Some(crate::game::moons::eclipse_spawn(
                        events.downed_plantera,
                        events.downed_all_mechs,
                        events.census,
                        rng,
                    )),
                    _ => None,
                }
            } else {
                None
            };
            // Somebody tied up, once in a long while, deep enough down to be worth finding.
            //
            // Rare and unique on purpose: these are the *only* way their residents ever arrive, so
            // one of them failing to appear is a whole townsperson missing — the Mechanic, and with
            // her every piece of wire in the game. Each one is gated on its real vanilla condition
            // (`bound_gate`), so the Wizard, Mechanic and Goblin Tinkerer are no longer findable
            // day one and the Golfer wants the underground desert.
            if matches!(depth, Depth::Underground | Depth::Cavern)
                && rng.random_range(0..BOUND_RARITY) == 0
                && let Some(bound) = pick_bound(world, npcs, depth, biome_at(world, x, y), y, rng)
            {
                out.push((bound, (x as f32 * 16.0, y as f32 * 16.0)));
                break;
            }

            let npc_type = match event_type {
                Some(npc_type) => npc_type,
                None => {
                    let biome = biome_at(world, x, y);
                    let ordinary = pool(depth, biome, world.day_time);
                    // Hardmode adds to what a place had rather than replacing it, so a hardmode
                    // forest still has zombies in it. The underworld's additions wait for a
                    // mechanical boss, which is progression rather than place and so is held here.
                    let extra = if events.hard_mode
                        && (depth != Depth::Underworld || events.downed_mech_any)
                    {
                        hardmode_pool(depth, biome, world.day_time)
                    } else {
                        &[]
                    };
                    // ...and a blood moon widens the night on top of both.
                    let bloody = if world.blood_moon && !world.day_time {
                        blood_moon_pool(depth, events.hard_mode)
                    } else {
                        &[]
                    };
                    // The caverns also draw from the six this world happens to have, which is a
                    // world-specific list rather than a table. It counts as one entry in the
                    // draw, as the game counts it — not six — so a world's own monsters are a
                    // seasoning on the cavern pool rather than most of it.
                    let world_specific = depth == Depth::Cavern && biome == Biome::Forest;
                    let total =
                        ordinary.len() + extra.len() + bloody.len() + usize::from(world_specific);
                    if total == 0 {
                        continue;
                    }
                    let at = rng.random_range(0..total);
                    if at < ordinary.len() {
                        ordinary[at]
                    } else if at < ordinary.len() + extra.len() {
                        extra[at - ordinary.len()]
                    } else if at < ordinary.len() + extra.len() + bloody.len() {
                        bloody[at - ordinary.len() - extra.len()]
                    } else {
                        events.cavern_monsters.pick(rng)
                    }
                }
            };

            // Position is the NPC's top-left, so it stands on the tile below.
            out.push((npc_type, (x as f32 * 16.0, y as f32 * 16.0)));
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing running: what the ordinary world looks like to `try_spawn`.
    fn quiet() -> EventSpawns<'static> {
        EventSpawns {
            moon: None,
            eclipse: false,
            downed_plantera: false,
            downed_all_mechs: false,
            boss_cap: false,
            hard_mode: false,
            downed_mech_any: false,
            census: &|_| 0,
            cavern_monsters: crate::game::cavern_monsters::CavernMonsters::for_world(7),
        }
    }
    use crate::world::worldgen;
    use rand::SeedableRng;

    fn test_world() -> World {
        worldgen::generate(800, 600, "spawn", 7)
    }

    #[test]
    fn depth_bands_follow_the_world_layers() {
        let mut world = test_world();
        world.surface = 200;
        world.rock_layer = 300;
        assert_eq!(depth_at(&world, 100), Depth::Surface);
        assert_eq!(depth_at(&world, 250), Depth::Underground);
        assert_eq!(depth_at(&world, 350), Depth::Cavern);
        assert_eq!(depth_at(&world, 599 - 1), Depth::Underworld);
    }

    /// Every hardmode pool names real, hostile types, and each biome's are its own.
    #[test]
    fn the_hardmode_pools_are_real_and_placed_right() {
        use terrustia_proto::npc_data::npc_stats;
        let mut anywhere = std::collections::HashSet::new();
        for depth in [
            Depth::Surface,
            Depth::Underground,
            Depth::Cavern,
            Depth::Underworld,
        ] {
            for biome in [
                Biome::Forest,
                Biome::Corruption,
                Biome::Crimson,
                Biome::Jungle,
                Biome::Snow,
                Biome::Desert,
                Biome::Ocean,
                Biome::Dungeon,
                Biome::Hallow,
            ] {
                for day in [true, false] {
                    for npc_type in hardmode_pool(depth, biome, day) {
                        let stats = npc_stats(*npc_type)
                            .unwrap_or_else(|| panic!("{npc_type} in {biome:?} is not a type"));
                        assert!(
                            !stats.friendly && !stats.town_npc,
                            "{} is friendly and should not be spawned at anyone",
                            stats.name
                        );
                        anywhere.insert(*npc_type);
                    }
                }
            }
        }
        assert!(
            anywhere.len() > 40,
            "only {} hardmode types",
            anywhere.len()
        );
    }

    /// The hallow is empty before hardmode and full after it.
    #[test]
    fn the_hallow_only_lives_in_hardmode() {
        // Before: it borrows the forest's ordinary life rather than being barren.
        assert_eq!(
            pool(Depth::Surface, Biome::Hallow, true),
            pool(Depth::Surface, Biome::Forest, true),
        );
        // After: it has its own, and nothing it has is the forest's.
        let hallow = hardmode_pool(Depth::Surface, Biome::Hallow, true);
        assert!(!hallow.is_empty());
        let forest = hardmode_pool(Depth::Surface, Biome::Forest, true);
        assert!(
            hallow.iter().all(|t| !forest.contains(t)),
            "the hallow is sharing the forest's hardmode life"
        );
    }

    /// A blood moon widens the night rather than replacing it, and only on the surface.
    #[test]
    fn a_blood_moon_widens_the_night() {
        use terrustia_proto::npc_data::npc_stats;
        let early = blood_moon_pool(Depth::Surface, false);
        let late = blood_moon_pool(Depth::Surface, true);
        assert!(!early.is_empty());
        assert!(late.len() > early.len(), "hardmode adds the Clown");
        assert!(late.contains(&109), "the Clown");
        assert!(!early.contains(&109), "but not before hardmode");
        for npc_type in late {
            assert!(npc_stats(*npc_type).is_some(), "{npc_type} is not a type");
        }
        // Underground is untouched: a blood moon is a thing that happens to the sky.
        for depth in [Depth::Underground, Depth::Cavern, Depth::Underworld] {
            assert!(blood_moon_pool(depth, true).is_empty(), "{depth:?}");
        }
    }

    /// The two evils get different creatures, not the same list twice.
    #[test]
    fn the_evils_are_not_the_same_list() {
        for depth in [Depth::Surface, Depth::Cavern] {
            let corrupt = hardmode_pool(depth, Biome::Corruption, false);
            let crimson = hardmode_pool(depth, Biome::Crimson, false);
            assert!(!corrupt.is_empty() && !crimson.is_empty());
            assert!(
                corrupt.iter().all(|t| !crimson.contains(t)),
                "the two evils share {depth:?} spawns"
            );
        }
    }

    #[test]
    fn every_pool_is_non_empty_and_known() {
        // A pool that names an NPC this build does not define would spawn nothing at all.
        for depth in [
            Depth::Surface,
            Depth::Underground,
            Depth::Cavern,
            Depth::Underworld,
        ] {
            for biome in [
                Biome::Forest,
                Biome::Corruption,
                Biome::Crimson,
                Biome::Jungle,
                Biome::Snow,
                Biome::Desert,
                Biome::Ocean,
                Biome::Dungeon,
            ] {
                for day in [true, false] {
                    let types = pool(depth, biome, day);
                    assert!(!types.is_empty(), "{depth:?}/{biome:?} day={day} is empty");
                    for t in types {
                        assert!(
                            terrustia_proto::npc_data::npc_stats(*t).is_some(),
                            "{depth:?}/{biome:?} names unknown NPC {t}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn surface_forest_swaps_between_day_and_night() {
        let day = pool(Depth::Surface, Biome::Forest, true);
        let night = pool(Depth::Surface, Biome::Forest, false);
        assert!(day.contains(&1), "slimes by day");
        assert!(night.contains(&3), "zombies by night");
        assert!(!night.contains(&46), "no bunnies at night");
    }

    #[test]
    fn the_ocean_is_decided_by_position_not_tiles() {
        let world = test_world();
        assert_eq!(biome_at(&world, 10, 100), Biome::Ocean);
        assert_eq!(biome_at(&world, world.width() - 10, 100), Biome::Ocean);
    }

    #[test]
    fn a_generated_forest_reads_as_forest() {
        let world = test_world();
        let x = world.width() / 2;
        assert_eq!(
            biome_at(&world, x, i32::from(world.surface) + 30),
            Biome::Forest
        );
    }

    #[test]
    fn nothing_spawns_without_players() {
        let world = test_world();
        let npcs = NpcStore::new();
        let mut rng = SmallRng::seed_from_u64(1);
        assert!(
            try_spawn(
                &world,
                &npcs,
                &[],
                &quiet(),
                &JourneyPowers::default(),
                &mut rng,
                0
            )
            .is_empty()
        );
    }

    /// A base with three or more residents stops producing monsters entirely (C1-b item 8):
    /// `NPC.cs:917-921`'s own classic-mode `spawnFriendly = true` on every attempt, which this
    /// project's own `try_spawn` skips outright rather than fabricate a friendly critter it has no
    /// table for (see `try_spawn`'s own comment on the fork). Fails before the fix, when town
    /// suppression was a flat `3.0x` rate multiplier that still let monsters through.
    #[test]
    fn a_populated_base_produces_no_monsters_at_all() {
        const GUIDE: u16 = 22;
        let world = test_world();
        let mut npcs = NpcStore::new();
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (
            f32::from(world.spawn_x) * 16.0,
            f32::from(world.spawn_y) * 16.0,
        );
        // Three townsfolk standing right where the player is, well inside town_npcs_near's reach.
        for _ in 0..3 {
            npcs.spawn(GUIDE, player.position);
        }
        let players = vec![Some(player)];

        let mut rng = SmallRng::seed_from_u64(13);
        let mut spawned = 0;
        for _ in 0..3600 {
            spawned += try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut rng,
                0,
            )
            .len();
        }
        assert_eq!(
            spawned, 0,
            "a minute of ticks in a populated base should produce no monsters at all"
        );
    }

    #[test]
    fn spawns_appear_outside_the_safe_zone_and_on_solid_ground() {
        let world = test_world();
        let npcs = NpcStore::new();
        let mut rng = SmallRng::seed_from_u64(9);

        let (tx, ty) = (world.spawn_x as i32, world.spawn_y as i32);
        let (tx_px, ty_px) = (tx as f32 * 16.0, ty as f32 * 16.0);
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (tx_px, ty_px);
        let players = vec![Some(player)];

        // Run many ticks so the one-in-600 roll fires repeatedly.
        let mut seen = 0;
        for _ in 0..20_000 {
            for (npc_type, (px, py)) in try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut rng,
                0,
            ) {
                seen += 1;
                assert!(
                    terrustia_proto::npc_data::npc_stats(npc_type).is_some(),
                    "spawned an unknown type {npc_type}"
                );
                let (x, y) = ((px / 16.0) as i32, (py / 16.0) as i32);
                assert!(
                    (x - tx).abs() >= SAFE_RANGE_X || (y - ty).abs() >= SAFE_RANGE_Y,
                    "spawned inside the safe zone at ({x}, {y}) vs player ({tx}, {ty})"
                );
                assert!(has_room(&world, x, y), "spawned somewhere with no room");
            }
            if seen > 20 {
                break;
            }
        }
        assert!(seen > 0, "nothing ever spawned");
    }

    #[test]
    fn ground_is_found_by_scanning_down() {
        let world = test_world();
        let x = world.width() / 2;
        // Start well above the terrain; the scan should land on the first solid row.
        let surface = (0..world.height())
            .find(|y| world.tile(x, *y).is_active())
            .expect("the column has ground");
        assert_eq!(find_ground(&world, x, surface - 10), Some(surface));
        // And starting on the ground finds it immediately.
        assert_eq!(find_ground(&world, x, surface), Some(surface));
    }

    #[test]
    fn scanning_gives_up_rather_than_falling_through_the_world() {
        let world = test_world();
        // Deep sky above the terrain, more than the scan depth.
        assert_eq!(find_ground(&world, world.width() / 2, 0), None);
    }

    #[test]
    fn spawning_is_frequent_enough_to_matter() {
        // Picking a blind point and demanding it be the surface almost never works; scanning down
        // is what makes the spawn rate real. A minute of ticks should produce several spawns.
        let world = test_world();
        let npcs = NpcStore::new();
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (
            f32::from(world.spawn_x) * 16.0,
            f32::from(world.spawn_y) * 16.0,
        );
        let players = vec![Some(player)];

        let mut rng = SmallRng::seed_from_u64(11);
        let mut spawned = 0;
        for _ in 0..3600 {
            spawned += try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut rng,
                0,
            )
            .len();
        }
        assert!(
            spawned >= 3,
            "only {spawned} spawns in a minute of ticks; the spawn point search is too fussy"
        );
    }

    #[test]
    fn the_cap_stops_further_spawns() {
        let world = test_world();
        let mut npcs = NpcStore::new();
        // Fill well past the single-player cap.
        for _ in 0..40 {
            npcs.spawn(3, (0.0, 0.0));
        }

        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (100.0, 100.0);
        let players = vec![Some(player)];

        let mut rng = SmallRng::seed_from_u64(3);
        for _ in 0..5_000 {
            assert!(
                try_spawn(
                    &world,
                    &npcs,
                    &players,
                    &quiet(),
                    &JourneyPowers::default(),
                    &mut rng,
                    0
                )
                .is_empty(),
                "spawned past the cap"
            );
        }
    }

    /// A single test player, `is_playing` and clear of the safe zone, with a real channel behind
    /// it — the same construction `spawns_appear_outside_the_safe_zone_and_on_solid_ground` above
    /// already uses.
    fn one_player(world: &World) -> Vec<Option<Player>> {
        let (tx, ty) = (world.spawn_x as i32, world.spawn_y as i32);
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (tx as f32 * 16.0, ty as f32 * 16.0);
        vec![Some(player)]
    }

    /// Journey mode's `SpawnRate` at its exact floor (`0.0`) disables spawns outright for that
    /// player — `GetShouldDisableSpawnsFor`'s own hard condition, not merely the 0.1× remap floor
    /// `spawn_rate_multiplier` alone would give.
    #[test]
    fn spawn_rate_at_the_floor_disables_spawns_in_a_journey_world() {
        let mut world = test_world();
        world.game_mode = 3; // Journey
        let npcs = NpcStore::new();
        let players = one_player(&world);
        let mut journey = JourneyPowers::default();
        journey.set_spawn_rate_slider(0, 0.0);

        let mut rng = SmallRng::seed_from_u64(11);
        let mut seen = 0;
        for _ in 0..20_000 {
            seen += try_spawn(&world, &npcs, &players, &quiet(), &journey, &mut rng, 0).len();
        }
        assert_eq!(seen, 0, "spawns should be disabled outright at the floor");
    }

    /// The same slider at its floor has no effect at all outside a Journey world — every one of
    /// `SpawnRateSliderPerPlayerPower`'s five real vanilla call sites gates on `Main.IsJourneyMode`
    /// before reading the power, and this is the one behaviour among the three per-player powers
    /// where getting that gate wrong is easy to miss testing, since an ungated implementation would
    /// otherwise look identical to a correct one on an ordinary difficulty.
    #[test]
    fn spawn_rate_has_no_effect_outside_a_journey_world() {
        let world = test_world(); // game_mode 0: ordinary
        let npcs = NpcStore::new();
        let players = one_player(&world);
        let mut journey = JourneyPowers::default();
        journey.set_spawn_rate_slider(0, 0.0); // would disable spawns entirely, in a Journey world

        let mut rng = SmallRng::seed_from_u64(11);
        let mut seen = 0;
        for _ in 0..20_000 {
            seen += try_spawn(&world, &npcs, &players, &quiet(), &journey, &mut rng, 0).len();
        }
        assert!(
            seen > 0,
            "an ordinary-difficulty world should spawn normally regardless of the slider"
        );
    }

    /// Above the floor, the slider scales how often spawns roll — pinned as a real measured ratio
    /// across many ticks, not just "some vs none": a 10× player should see roughly ten times as
    /// many spawn events as a 1× player over the same window.
    #[test]
    fn spawn_rate_at_its_top_spawns_far_more_often_than_the_default_in_a_journey_world() {
        let mut world = test_world();
        world.game_mode = 3;
        let npcs = NpcStore::new();
        let players = one_player(&world);

        let ordinary = JourneyPowers::default(); // 0.5 -> 1x, the default
        let mut boosted = JourneyPowers::default();
        boosted.set_spawn_rate_slider(0, 1.0); // the top of the slider -> 10x

        const TICKS: usize = 200_000;
        let mut ordinary_seen = 0;
        let mut rng = SmallRng::seed_from_u64(21);
        for _ in 0..TICKS {
            ordinary_seen +=
                try_spawn(&world, &npcs, &players, &quiet(), &ordinary, &mut rng, 0).len();
        }
        let mut boosted_seen = 0;
        let mut rng = SmallRng::seed_from_u64(21);
        for _ in 0..TICKS {
            boosted_seen +=
                try_spawn(&world, &npcs, &players, &quiet(), &boosted, &mut rng, 0).len();
        }
        assert!(
            boosted_seen > ordinary_seen * 5,
            "10x should spawn noticeably more often than 1x over {TICKS} ticks: \
             {boosted_seen} boosted vs {ordinary_seen} ordinary"
        );
    }

    /// The bound townsfolk are gated on their real progression, biome and depth, so the Wizard,
    /// Mechanic and Goblin Tinkerer are not findable on day one (`NPC.cs:2087-2091,2656`). Fails
    /// before the fix, when every still-bound resident was offered the moment a player reached a
    /// cavern, with no hardmode / Skeletron / goblin-army gate at all.
    #[test]
    fn bound_townsfolk_are_gated_on_real_progression() {
        let mut world = test_world();
        world.progress.hard_mode = false;
        world.progress.downed_boss3 = false;
        world.progress.downed_goblins = false;
        let y = i32::from(world.rock_layer) + 5; // squarely in the caverns
        assert_eq!(depth_at(&world, y), Depth::Cavern);

        // Day one, deep in a plain cavern: none of the three progression-gated finds are eligible.
        assert!(
            !bound_gate(106, &world, Depth::Cavern, Biome::Forest, y),
            "the Wizard needs hardmode"
        );
        assert!(
            !bound_gate(123, &world, Depth::Cavern, Biome::Forest, y),
            "the Mechanic needs Skeletron down"
        );
        assert!(
            !bound_gate(105, &world, Depth::Cavern, Biome::Forest, y),
            "the Goblin Tinkerer needs the army beaten"
        );

        // The Stylist has no progression gate (a spider nest is a day-one find), so she is eligible.
        assert!(bound_gate(354, &world, Depth::Cavern, Biome::Forest, y));

        // Hardmode opens the Wizard; Skeletron opens the Mechanic; the beaten army the Goblin.
        world.progress.hard_mode = true;
        world.progress.downed_boss3 = true;
        world.progress.downed_goblins = true;
        assert!(bound_gate(106, &world, Depth::Cavern, Biome::Forest, y));
        assert!(bound_gate(123, &world, Depth::Cavern, Biome::Forest, y));
        assert!(bound_gate(105, &world, Depth::Cavern, Biome::Forest, y));

        // The Golfer wants the underground desert, not a forest cavern.
        assert!(!bound_gate(589, &world, Depth::Cavern, Biome::Forest, y));
        assert!(bound_gate(
            589,
            &world,
            Depth::Underground,
            Biome::Desert,
            y
        ));

        // And a fresh cavern only ever offers the Stylist, never a progression-gated resident.
        let mut rng = SmallRng::seed_from_u64(5);
        world.progress.hard_mode = false;
        world.progress.downed_boss3 = false;
        world.progress.downed_goblins = false;
        for _ in 0..500 {
            if let Some(bound) = pick_bound(
                &world,
                &NpcStore::new(),
                Depth::Cavern,
                Biome::Forest,
                y,
                &mut rng,
            ) {
                assert_eq!(bound, 354, "only the Stylist is a day-one cavern find");
            }
        }
    }
}

/// Whether an NPC type counts against an invasion's remaining size.
///
/// Only the invasion's own members count. A goblin army is not shortened by killing the bats that
/// happened to be in the way, and the game keeps these rosters as flat lists for exactly that
/// reason.
pub fn belongs_to(kind: crate::game::event::Invasion, npc_type: u16) -> bool {
    use crate::game::event::Invasion;
    match kind {
        Invasion::Goblin => matches!(npc_type, 26 | 27 | 28 | 29 | 111 | 471),
        Invasion::FrostLegion => matches!(npc_type, 143..=145),
        Invasion::Pirate => matches!(npc_type, 212 | 213 | 214 | 215 | 216 | 252 | 491),
        Invasion::Martian => matches!(
            npc_type,
            381 | 382 | 383 | 385 | 386 | 388 | 389 | 390 | 395 | 520
        ),
    }
}

#[cfg(test)]
mod invasion_tests {
    use super::belongs_to;
    use crate::game::event::Invasion;

    /// Only an invasion's own members shorten it.
    #[test]
    fn bystanders_do_not_count_against_an_invasion() {
        assert!(belongs_to(Invasion::Goblin, 28), "a goblin peon does");
        assert!(!belongs_to(Invasion::Goblin, 1), "a blue slime does not");
        assert!(
            !belongs_to(Invasion::Goblin, 143),
            "nor does a member of a different invasion"
        );
        assert!(belongs_to(Invasion::Pirate, 491), "the Dutchman counts");
        assert!(belongs_to(Invasion::Martian, 395), "so does the saucer");
    }
}
