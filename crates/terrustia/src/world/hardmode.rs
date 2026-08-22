//! Hardmode: smashing altars, and what the world gives up for it.
//!
//! An altar is the only way the hardmode ores get into a world. The first three smashes each seed
//! one tier — cobalt or palladium, mythril or orichalcum, adamantite or titanium, and which of each
//! pair a world has is decided by the coin flip on that first smash and never changes. Every smash
//! after that adds more of the same, and each one adds less: the yield falls with the number of
//! altars already broken and again with how far round the cycle you are.
//!
//! It is not free. Every altar also puts a wraith on whoever broke it, which before Plantera is a
//! genuine fight — that is the price of the ore, and it is why smashing all of them at once is a
//! choice rather than an obvious move.

use rand::{Rng, rngs::SmallRng};

/// The three tiers, and the pair each one picks from.
pub const COBALT: u16 = 107;
pub const PALLADIUM: u16 = 221;
pub const MYTHRIL: u16 = 108;
pub const ORICHALCUM: u16 = 222;
pub const ADAMANTITE: u16 = 111;
pub const TITANIUM: u16 = 223;

/// The wraith an altar puts on whoever broke it.
pub const WRAITH: u16 = 82;

/// Which ores a world settled on. `None` means that tier has not been seeded yet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OreTiers {
    pub cobalt: Option<u16>,
    pub mythril: Option<u16>,
    pub adamantite: Option<u16>,
}

/// What smashing an altar does.
#[derive(Debug)]
pub struct Smashed {
    /// What the world should say about it.
    pub announcement: &'static str,
    /// The ore that was seeded, and where each vein wants to go.
    pub ore: u16,
    pub veins: Vec<(i32, i32, f64, i32)>,
    /// How many wraiths it put on the player.
    pub wraiths: u32,
    /// Whether this smash decided a tier, which is worth telling clients about.
    pub decided_a_tier: bool,
}

/// The shape of the world an altar's ore has to fit into.
#[derive(Debug, Clone, Copy)]
pub struct WorldShape {
    pub width: i32,
    pub height: i32,
    pub surface: i32,
    pub rock_layer: i32,
}

/// Smash an altar.
///
/// `altar_count` is how many have already gone. Returns `None` when it is not hardmode, because
/// before the wall falls an altar is only a crafting station.
pub fn smash(
    altar_count: i32,
    hard_mode: bool,
    tiers: &mut OreTiers,
    shape: WorldShape,
    rng: &mut SmallRng,
) -> Option<Smashed> {
    if !hard_mode {
        return None;
    }
    let WorldShape {
        width: world_width,
        height: world_height,
        surface,
        rock_layer,
    } = shape;
    let step = altar_count % 3;
    let round = altar_count / 3 + 1;

    // The yield: a full world's worth, less eighty-five for each step past the first, less again
    // for every full round of three already broken. So the fourth altar gives noticeably less than
    // the first and the fortieth gives almost nothing.
    let mut yield_ = f64::from(world_width) / 4200.0;
    yield_ = yield_ * 310.0 - f64::from(85 * step);
    yield_ *= 0.85;
    yield_ /= f64::from(round);

    let mut decided = false;
    let (ore, announcement) = match step {
        0 => {
            let ore = *tiers.cobalt.get_or_insert_with(|| {
                decided = true;
                if rng.random_range(0..2) == 0 {
                    PALLADIUM
                } else {
                    COBALT
                }
            });
            if ore == PALLADIUM {
                yield_ *= 0.9;
            }
            yield_ *= 1.05;
            (
                ore,
                if ore == PALLADIUM {
                    "Your world has been blessed with Palladium!"
                } else {
                    "Your world has been blessed with Cobalt!"
                },
            )
        }
        1 => {
            let ore = *tiers.mythril.get_or_insert_with(|| {
                decided = true;
                if rng.random_range(0..2) == 0 {
                    ORICHALCUM
                } else {
                    MYTHRIL
                }
            });
            if ore == ORICHALCUM {
                yield_ *= 0.9;
            }
            (
                ore,
                if ore == ORICHALCUM {
                    "Your world has been blessed with Orichalcum!"
                } else {
                    "Your world has been blessed with Mythril!"
                },
            )
        }
        _ => {
            let ore = *tiers.adamantite.get_or_insert_with(|| {
                decided = true;
                if rng.random_range(0..2) == 0 {
                    TITANIUM
                } else {
                    ADAMANTITE
                }
            });
            if ore == TITANIUM {
                yield_ *= 0.9;
            }
            (
                ore,
                if ore == TITANIUM {
                    "Your world has been blessed with Titanium!"
                } else {
                    "Your world has been blessed with Adamantite!"
                },
            )
        }
    };

    // Each tier goes deeper than the last, which is what keeps you digging further down for it.
    let top = match ore {
        MYTHRIL | ORICHALCUM => rock_layer,
        ADAMANTITE | TITANIUM => (rock_layer * 2 + world_height) / 3,
        _ => surface,
    };
    let bottom = world_height - 150;
    let mut veins = Vec::new();
    if top < bottom && world_width > 200 {
        let mut placed = 0.0;
        while placed < yield_ {
            placed += 1.0;
            let x = rng.random_range(100..world_width - 100);
            let y = rng.random_range(top..bottom);
            let strength = f64::from(rng.random_range(5..10));
            let steps = rng.random_range(5..10);
            veins.push((x, y, strength, steps));
        }
    }

    Some(Smashed {
        announcement,
        ore,
        veins,
        wraiths: rng.random_range(1..=2),
        decided_a_tier: decided,
    })
}

/// Which blocks an ore vein is allowed to replace.
///
/// Everything else it passes straight through, which is why a vein never eats a chest, a wall of a
/// dungeon, or somebody's house.
pub fn replaceable(block: u16) -> bool {
    matches!(
        block,
        0 | 1
            | 23
            | 25
            | 40
            | 53
            | 57
            | 59
            | 60
            | 70
            | 109
            | 112
            | 116
            | 117
            | 147
            | 161
            | 163
            | 164
            | 199
            | 200
            | 203
            | 234
            | 396..=403
    ) || terrustia_proto::tile_sets::is_moss(block)
}

/// What the world needs to let a vein be dug into it.
pub trait OreWorld {
    fn tile(&self, x: i32, y: i32) -> terrustia_proto::tile::Tile;
    fn set_tile(&mut self, x: i32, y: i32, tile: terrustia_proto::tile::Tile);
    fn width(&self) -> i32;
    fn height(&self) -> i32;
}

/// Dig one vein: a blob that wanders as it goes and narrows to nothing.
///
/// Returns the tiles it changed.
pub fn run_vein(
    world: &mut impl OreWorld,
    at: (i32, i32),
    strength: f64,
    steps: i32,
    ore: u16,
    rng: &mut SmallRng,
) -> Vec<(i32, i32)> {
    let mut changed = Vec::new();
    let mut here = (f64::from(at.0), f64::from(at.1));
    // It drifts, and the drift itself wanders, which is what gives a vein its crooked shape.
    let mut drift = (
        f64::from(rng.random_range(-10..=10)) * 0.1,
        f64::from(rng.random_range(-10..=10)) * 0.1,
    );
    let mut left = f64::from(steps);
    let mut width = strength;

    while width > 0.0 && left > 0.0 {
        width = strength * (left / f64::from(steps));
        left -= 1.0;
        let x0 = ((here.0 - width * 0.5) as i32).max(0);
        let x1 = ((here.0 + width * 0.5) as i32).min(world.width());
        let y0 = ((here.1 - width * 0.5) as i32).max(0);
        let y1 = ((here.1 + width * 0.5) as i32).min(world.height());

        for x in x0..x1 {
            for y in y0..y1 {
                // A rough circle with a ragged edge, rather than a square.
                let reach = strength * 0.5 * (1.0 + f64::from(rng.random_range(-10..=10)) * 0.015);
                if (f64::from(x) - here.0).abs() + (f64::from(y) - here.1).abs() >= reach {
                    continue;
                }
                let tile = world.tile(x, y);
                if !tile.is_active() || !replaceable(tile.block) {
                    continue;
                }
                let mut ore_tile = tile;
                ore_tile.block = ore;
                world.set_tile(x, y, ore_tile);
                changed.push((x, y));
            }
        }

        here = (here.0 + drift.0, here.1 + drift.1);
        drift.0 = (drift.0 + f64::from(rng.random_range(-10..=10)) * 0.05).clamp(-1.0, 1.0);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Stone(HashMap<(i32, i32), Tile>);

    impl OreWorld for Stone {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
        fn set_tile(&mut self, x: i32, y: i32, tile: Tile) {
            self.0.insert((x, y), tile);
        }
        fn width(&self) -> i32 {
            500
        }
        fn height(&self) -> i32 {
            500
        }
    }

    fn solid_stone() -> Stone {
        let mut tiles = HashMap::new();
        for x in 0..500 {
            for y in 0..500 {
                tiles.insert((x, y), Tile::block(1));
            }
        }
        Stone(tiles)
    }

    fn smash_one(count: i32, tiers: &mut OreTiers, seed: u64) -> Option<Smashed> {
        let mut rng = SmallRng::seed_from_u64(seed);
        smash(
            count,
            true,
            tiers,
            WorldShape {
                width: 4200,
                height: 1200,
                surface: 300,
                rock_layer: 500,
            },
            &mut rng,
        )
    }

    /// Before hardmode an altar is only furniture.
    #[test]
    fn an_altar_does_nothing_before_hardmode() {
        let mut rng = SmallRng::seed_from_u64(1);
        let mut tiers = OreTiers::default();
        let shape = WorldShape {
            width: 4200,
            height: 1200,
            surface: 300,
            rock_layer: 500,
        };
        assert!(smash(0, false, &mut tiers, shape, &mut rng).is_none());
        assert_eq!(tiers, OreTiers::default(), "and settles nothing");
    }

    /// The first three smashes seed one tier each, in order.
    #[test]
    fn the_first_three_seed_the_three_tiers() {
        let mut tiers = OreTiers::default();
        let first = smash_one(0, &mut tiers, 1).unwrap();
        assert!(matches!(first.ore, COBALT | PALLADIUM));
        assert!(first.decided_a_tier);
        assert_eq!(tiers.cobalt, Some(first.ore));
        assert_eq!(tiers.mythril, None, "one at a time");

        let second = smash_one(1, &mut tiers, 2).unwrap();
        assert!(matches!(second.ore, MYTHRIL | ORICHALCUM));
        let third = smash_one(2, &mut tiers, 3).unwrap();
        assert!(matches!(third.ore, ADAMANTITE | TITANIUM));
        assert!(tiers.cobalt.is_some() && tiers.mythril.is_some() && tiers.adamantite.is_some());
    }

    /// Which ore of a pair a world gets is decided once and never revisited.
    #[test]
    fn a_world_keeps_the_ore_it_rolled() {
        let mut tiers = OreTiers::default();
        let first = smash_one(0, &mut tiers, 4).unwrap();
        for round in 1..8 {
            let again = smash_one(round * 3, &mut tiers, round as u64 + 10).unwrap();
            assert_eq!(again.ore, first.ore, "the tier changed on round {round}");
            assert!(!again.decided_a_tier, "and claimed to decide it again");
        }
    }

    /// Both ores of a pair really do come up across enough worlds.
    #[test]
    fn both_ores_of_a_pair_happen() {
        let mut seen = std::collections::HashSet::new();
        for seed in 0..40u64 {
            let mut tiers = OreTiers::default();
            seen.insert(smash_one(0, &mut tiers, seed).unwrap().ore);
        }
        assert_eq!(
            seen,
            std::collections::HashSet::from([COBALT, PALLADIUM]),
            "one of the pair never turned up"
        );
    }

    /// Each altar gives less than the last.
    #[test]
    fn the_yield_falls_with_every_altar() {
        let mut tiers = OreTiers::default();
        // Seed all three so later rounds are comparing like with like.
        for i in 0..3 {
            smash_one(i, &mut tiers, 1);
        }
        let veins = |count: i32| {
            let mut copy = tiers;
            smash_one(count, &mut copy, 7).unwrap().veins.len()
        };
        let first_round = veins(0);
        let fourth_round = veins(9);
        let tenth_round = veins(27);
        assert!(
            first_round > fourth_round,
            "{first_round} then {fourth_round}"
        );
        assert!(
            fourth_round > tenth_round,
            "{fourth_round} then {tenth_round}"
        );
        assert!(tenth_round > 0, "but never nothing at all");
    }

    /// The deeper tiers really are placed deeper.
    #[test]
    fn each_tier_goes_deeper_than_the_last() {
        let mut tiers = OreTiers::default();
        let shallowest = |smashed: &Smashed| smashed.veins.iter().map(|(_, y, _, _)| *y).min();
        let cobalt = smash_one(0, &mut tiers, 8).unwrap();
        let mythril = smash_one(1, &mut tiers, 8).unwrap();
        let adamantite = smash_one(2, &mut tiers, 8).unwrap();
        assert!(shallowest(&cobalt) < shallowest(&mythril));
        assert!(shallowest(&mythril) < shallowest(&adamantite));
    }

    /// It always costs a wraith or two.
    #[test]
    fn every_altar_costs_a_wraith() {
        let mut tiers = OreTiers::default();
        for count in 0..12 {
            let smashed = smash_one(count, &mut tiers, count as u64).unwrap();
            assert!(
                (1..=2).contains(&smashed.wraiths),
                "{} wraiths",
                smashed.wraiths
            );
        }
    }

    /// A vein replaces stone and dirt, and nothing else.
    #[test]
    fn a_vein_only_eats_what_it_should() {
        let mut world = solid_stone();
        // A chest and a dungeon brick right where the vein is going.
        world.set_tile(250, 250, Tile::framed(21, 0, 0));
        world.set_tile(251, 250, Tile::block(41));
        let mut rng = SmallRng::seed_from_u64(9);
        let changed = run_vein(&mut world, (250, 250), 12.0, 12, COBALT, &mut rng);

        assert!(!changed.is_empty(), "it should have dug something");
        assert_eq!(world.tile(250, 250).block, 21, "the chest survived");
        assert_eq!(world.tile(251, 250).block, 41, "so did the brick");
        assert!(
            changed
                .iter()
                .all(|(x, y)| world.tile(*x, *y).block == COBALT),
            "and everything it did dig is ore"
        );
    }

    /// A vein is a vein rather than a scattering: what it digs is connected and roughly round.
    #[test]
    fn a_vein_is_a_blob_not_a_scattering() {
        let mut world = solid_stone();
        let mut rng = SmallRng::seed_from_u64(10);
        let changed = run_vein(&mut world, (250, 250), 10.0, 10, MYTHRIL, &mut rng);
        assert!(
            changed.len() > 20,
            "too small to be a vein: {}",
            changed.len()
        );
        let xs: Vec<i32> = changed.iter().map(|(x, _)| *x).collect();
        let ys: Vec<i32> = changed.iter().map(|(_, y)| *y).collect();
        let span = (
            xs.iter().max().unwrap() - xs.iter().min().unwrap(),
            ys.iter().max().unwrap() - ys.iter().min().unwrap(),
        );
        assert!(span.0 < 40 && span.1 < 40, "it wandered too far: {span:?}");
    }

    /// A vein in open air digs nothing, rather than filling the air with ore.
    #[test]
    fn a_vein_in_the_air_digs_nothing() {
        let mut world = Stone(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(11);
        assert!(run_vein(&mut world, (250, 250), 10.0, 10, COBALT, &mut rng).is_empty());
    }
}
