//! The world changing on its own: grass creeping over bare dirt.
//!
//! Terraria runs `WorldGen.UpdateWorld` every tick, sampling random tiles near the players and
//! growing things on them. None of it was here, which made the world permanently static: grass
//! never spread and no biome could be built by hand. Combined with the rest of that loop being
//! absent it also meant nothing was renewable — but grass is the foundation, because herbs and
//! trees both need it underneath them.
//!
//! The rule is `WorldGen.SpreadGrass`: a base tile turns to grass if it is **not sealed in**. The
//! game checks the three-by-three box around it and refuses if every tile in that box is active
//! and solid, which is what stops grass appearing in the middle of solid ground where no light
//! reaches.
//!
//! What spreads is decided by the neighbours, but *which base a grass will spread onto at all* is
//! not the same for every grass — this was the one thing an earlier version of this doc got
//! backwards, claiming "a mud pit could never become a jungle": jungle and mushroom grass grow
//! only on **mud**, never on dirt, so dirt beside jungle grass stays dirt forever. The two evils
//! are the exception that spreads onto both: dirt beside corruption becomes corrupt grass, and mud
//! beside it becomes corrupt *jungle* grass — the block that lets an infection swallow a jungle
//! rather than stopping dead at its edge. That is what lets a player make a biome, and what lets
//! one creep.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::{Tile, tile_solid::solid};

use super::world::World;

/// Plain dirt, one of the two bases grass grows on.
pub const DIRT: u16 = 0;
/// Mud, the other base — jungle and mushroom grass grow *only* here, never on dirt; the two evils
/// grow on both, taking their ordinary form on dirt and their own jungle-grass form on mud.
pub const MUD: u16 = 59;
/// Corrupt jungle grass, the block mud becomes beside ordinary corrupt grass or beside more of
/// itself — `TileID.CorruptJungleGrass`.
const CORRUPT_JUNGLE_GRASS: u16 = 661;
/// Crimson jungle grass, the same thing for the other evil — `TileID.CrimsonJungleGrass`.
const CRIMSON_JUNGLE_GRASS: u16 = 662;

/// Every grass that can spread, and so every grass a base tile might become.
///
/// Ordinary grass is last so that a tile touching both plain grass and an evil one takes the evil:
/// an infection that loses ties would never advance.
pub const GRASSES: [u16; 6] = [
    23,  // corrupt
    199, // crimson
    109, // hallow
    60,  // jungle
    70,  // mushroom
    2,   // plain
];

/// What a base tile becomes next to this grass, given which base it already is — `None` when this
/// grass does not grow on that base at all.
///
/// `WorldGen.SpreadGrass`'s own `dirt`/`grass` argument pairs, picked apart from the `TileFrame`
/// dispatcher that decides which pairs to try for a given neighbouring grass
/// (`WorldGen.cs:75225-75307`). Jungle (60) and mushroom (70) grass only ever list mud; the two
/// evils (23, 199) list both bases, taking their own jungle-grass form on mud.
fn grass_result(grass: u16, base: u16) -> Option<u16> {
    match (grass, base) {
        (23, DIRT) => Some(23),
        (23, MUD) => Some(CORRUPT_JUNGLE_GRASS),
        (199, DIRT) => Some(199),
        (199, MUD) => Some(CRIMSON_JUNGLE_GRASS),
        (109, DIRT) => Some(109),
        (60, MUD) => Some(60),
        (70, MUD) => Some(70),
        (2, DIRT) => Some(2),
        _ => None,
    }
}

/// Whether a base tile is open enough for grass to reach it.
///
/// `SpreadGrass` scans `[x-1, x+2) x [y-1, y+2)` and gives up when everything in it is active and
/// solid. One gap anywhere in that box is enough.
fn is_exposed(world: &World, x: i32, y: i32) -> bool {
    for nx in (x - 1)..=(x + 1) {
        for ny in (y - 1)..=(y + 1) {
            if !world.in_bounds(nx, ny) {
                return true;
            }
            let tile = world.tile(nx, ny);
            if !tile.is_active() || !solid(tile.block) {
                return true;
            }
        }
    }
    false
}

/// Which grass, if any, is touching this tile *and will actually grow on it* — a jungle-grass
/// neighbour is not a candidate for a dirt tile, the way a plain-grass or evil-grass one is.
fn neighbouring_grass(world: &World, x: i32, y: i32, base: u16) -> Option<u16> {
    let mut found: Option<(usize, u16)> = None;
    for nx in (x - 1)..=(x + 1) {
        for ny in (y - 1)..=(y + 1) {
            if (nx, ny) == (x, y) || !world.in_bounds(nx, ny) {
                continue;
            }
            let tile = world.tile(nx, ny);
            if !tile.is_active() {
                continue;
            }
            let Some(rank) = GRASSES.iter().position(|g| *g == tile.block) else {
                continue;
            };
            let Some(result) = grass_result(tile.block, base) else {
                continue;
            };
            // Lower index wins, so an evil grass beats plain grass on a contested tile.
            found = Some(match found {
                Some((best, best_result)) if best <= rank => (best, best_result),
                _ => (rank, result),
            });
        }
    }
    found.map(|(_, result)| result)
}

/// Try to grow grass on one tile. Returns the tile it changed, if it changed one.
pub fn spread_grass(world: &mut World, x: i32, y: i32) -> Option<(i32, i32)> {
    if !world.in_bounds(x, y) {
        return None;
    }
    let tile = world.tile(x, y);
    if !tile.is_active() || !matches!(tile.block, DIRT | MUD) {
        return None;
    }
    if !is_exposed(world, x, y) {
        return None;
    }
    let grass = neighbouring_grass(world, x, y, tile.block)?;

    let mut grown = tile;
    grown.block = grass;
    world.set_tile(x, y, grown);
    Some((x, y))
}

/// A herb just planted, which yields only seed until it grows.
pub const IMMATURE_HERB: u16 = 82;
/// A grown herb, which yields the herb itself.
pub const MATURE_HERB: u16 = 83;

/// Which herb suits this ground, as `WorldGen.PlaceSuitableHerbHere` decides.
///
/// The style is the frame column, so the same tile is all seven herbs. Ground that grows nothing
/// returns `None` — stone, wood, anything built.
fn herb_for(ground: u16) -> Option<i16> {
    Some(match ground {
        2 | 109 => 0,                         // grass, hallowed: daybloom
        60 => 1,                              // jungle: moonglow
        0 | 59 => 2,                          // dirt, mud: blinkroot
        23 | 25 | 199 | 203 | 661 | 662 => 3, // the evils: deathweed
        53 | 116 => 4,                        // sand: waterleaf
        57 | 633 => 5,                        // ash: fireblossom
        147 | 161 | 163 | 164 | 200 => 6,     // snow and ice: shiverthorn
        _ => return None,
    })
}

/// Plant a herb on suitable ground with open air above it, and ripen ones already planted.
///
/// Herbs were not renewable at all: the ones a world is generated with were every one it would
/// ever have, so potions were a finite resource. The game plants them at a low rate over the
/// whole world and thins them by refusing where several already grow nearby; this keeps the "not
/// where there are already herbs" rule, which is what stops a field turning solid green.
pub fn plant_herb(world: &mut World, x: i32, y: i32) -> Option<(i32, i32)> {
    if !world.in_bounds(x, y) || !world.in_bounds(x, y - 1) {
        return None;
    }

    // Ripen anything already growing here before considering a new one.
    let here = world.tile(x, y);
    if here.is_active() && here.block == IMMATURE_HERB {
        let mut grown = here;
        grown.block = MATURE_HERB;
        world.set_tile(x, y, grown);
        return Some((x, y));
    }

    let ground = world.tile(x, y);
    if !ground.is_active() {
        return None;
    }
    let above = world.tile(x, y - 1);
    if above.is_active() || above.liquid > 0 {
        return None;
    }
    let style = herb_for(ground.block)?;

    // Thin them out: the game counts herbs in a box around the spot and gives up if there are
    // already several.
    let mut nearby = 0;
    for nx in (x - 12)..=(x + 12) {
        for ny in (y - 12)..=(y + 12) {
            if !world.in_bounds(nx, ny) {
                continue;
            }
            let tile = world.tile(nx, ny);
            if tile.is_active() && matches!(tile.block, IMMATURE_HERB | MATURE_HERB | 84) {
                nearby += 1;
                if nearby >= 5 {
                    return None;
                }
            }
        }
    }

    world.set_tile(x, y - 1, Tile::framed(IMMATURE_HERB, style * 18, 0));
    Some((x, y - 1))
}

/// A sapling, waiting to become a tree.
pub const SAPLING: u16 = 20;
/// A tree.
pub const TREE: u16 = 5;

/// Ground a tree will take root in, from `WorldGen.IsTileTypeFitForTree`.
fn fit_for_tree(block: u16) -> bool {
    matches!(block, 2 | 23 | 60 | 70 | 109 | 147 | 199)
}

/// Grow a sapling into a tree.
///
/// Without this, wood is a finite resource: the trees a world is generated with are every one it
/// will ever have, and an acorn does nothing. That is not a subtle shortage — wood is the first
/// material in the game and half the crafting tree starts with it.
///
/// **Simplified deliberately, and worth knowing about.** The game picks among eight trunk styles
/// with branches and roots; this grows the plain trunk only — `frameX` 0, `frameY` one of three —
/// so a grown tree is a bare pole rather than a branching one. It is choppable, drops wood, and is
/// the right tile; it just looks plainer than one the world was generated with. Porting the branch
/// framing is a larger job and is recorded in GAPS.md rather than guessed at, because a wrong
/// frame renders as garbage.
pub fn grow_tree(world: &mut World, x: i32, y: i32, rng: &mut SmallRng) -> Option<(i32, i32)> {
    if !world.in_bounds(x, y) {
        return None;
    }
    let here = world.tile(x, y);
    if !here.is_active() || here.block != SAPLING {
        return None;
    }

    // The ground beneath, and a neighbour of the same kind: a tree needs a bank, not a ledge.
    let ground = world.tile(x, y + 1);
    if !ground.is_active() || !fit_for_tree(ground.block) {
        return None;
    }
    let flanked = [-1, 1].iter().any(|dx| {
        let side = world.tile(x + dx, y + 1);
        side.is_active() && fit_for_tree(side.block)
    });
    if !flanked {
        return None;
    }

    // Room to grow: the game rolls five to sixteen and refuses if anything is in the way.
    // The trunk stands on the sapling's own tile, so only what is *above* it has to be clear —
    // checking from the sapling itself finds the sapling and refuses every time.
    let height = rng.random_range(5..=16);
    for above in 1..height {
        if !world.in_bounds(x, y - above) {
            return None;
        }
        let space = world.tile(x, y - above);
        if space.is_active() || space.liquid > 0 {
            return None;
        }
    }

    for above in 0..height {
        let style = rng.random_range(0..3) * 22;
        world.set_tile(x, y - above, Tile::framed(TREE, 0, style));
    }
    Some((x, y))
}

/// Which vine hangs from which grass, and how far it will reach.
///
/// A vine grows down from grass one tile at a time. The pairing matters: jungle grass grows jungle
/// vines and corruption grows its own, so a vine is a piece of biome rather than decoration.
fn vine_for(grass: u16) -> Option<u16> {
    Some(match grass {
        2 => 52,    // forest
        23 => 52,   // corruption uses the plain vine too
        60 => 62,   // jungle
        109 => 115, // hallow
        199 => 205, // crimson
        _ => return None,
    })
}

/// How far a vine will hang before it stops.
const VINE_REACH: i32 = 10;

/// Hang a vine one tile further down from grass or from a vine already there.
pub fn grow_vine(world: &mut World, x: i32, y: i32) -> Option<(i32, i32)> {
    if !world.in_bounds(x, y) || !world.in_bounds(x, y + 1) {
        return None;
    }
    let here = world.tile(x, y);
    if !here.is_active() {
        return None;
    }

    // Either the grass a vine starts from, or the end of one already hanging.
    let vine = if let Some(vine) = vine_for(here.block) {
        vine
    } else if [52u16, 62, 115, 205].contains(&here.block) {
        // Do not let one trail forever: count back up to the grass it came from.
        let mut length = 0;
        while length < VINE_REACH && world.in_bounds(x, y - length - 1) {
            let above = world.tile(x, y - length - 1);
            if above.is_active() && above.block == here.block {
                length += 1;
            } else {
                break;
            }
        }
        if length >= VINE_REACH - 1 {
            return None;
        }
        here.block
    } else {
        return None;
    };

    let below = world.tile(x, y + 1);
    if below.is_active() || below.liquid > 0 {
        return None;
    }
    // `block`, not `framed(.., 0, 0)`. A vine is not frame-important, so the save format stores
    // no frames for it and a reload brings it back as -1 — the sentinel for "no frame". Writing 0
    // here made the in-memory world disagree with the one on disk about thousands of tiles, which
    // the round-trip test catches the moment a generated world contains any number of them.
    world.set_tile(x, y + 1, Tile::block(vine));
    Some((x, y + 1))
}

/// A cactus.
pub const CACTUS: u16 = 80;

/// Sand a cactus will root in — the four colours, as `TileID.Sets.Conversion.Sand` has them.
const SANDS: [u16; 4] = [53, 112, 116, 234];

/// How much water nearby stops one growing, in whole tiles' worth.
const CACTUS_WATER_LIMIT: i32 = 4;

/// Grow a cactus upward from sand, or add a segment to one already standing.
///
/// **Simplified the same way trees are**: the plain trunk, not the game's arms and branches,
/// because tile 80 is frame-important and a wrong frame renders as garbage. It is choppable and
/// drops cactus.
pub fn grow_cactus(world: &mut World, x: i32, y: i32) -> Option<(i32, i32)> {
    if !world.in_bounds(x, y) || !world.in_bounds(x, y - 1) {
        return None;
    }
    let here = world.tile(x, y);
    if !here.is_active() || !(SANDS.contains(&here.block) || here.block == CACTUS) {
        return None;
    }

    // Clear above, and clear either side of that, so a cactus does not grow into a wall.
    for dx in -1..=1 {
        let above = world.tile(x + dx, y - 1);
        if above.is_active() || above.liquid > 0 {
            return None;
        }
    }

    // A desert, not an oasis: too much water nearby and nothing grows.
    let mut water = 0i32;
    for nx in (x - 3)..=(x + 3) {
        for ny in (y - 3)..=(y + 3) {
            if world.in_bounds(nx, ny) {
                water += i32::from(world.tile(nx, ny).liquid);
            }
        }
    }
    if water / 255 > CACTUS_WATER_LIMIT {
        return None;
    }

    // Thin them, and cap the height, by counting what is already about.
    let mut nearby = 0;
    for nx in (x - 6)..=(x + 6) {
        for ny in (y - 8)..=(y + 1) {
            if world.in_bounds(nx, ny) {
                let tile = world.tile(nx, ny);
                if tile.is_active() && tile.block == CACTUS {
                    nearby += 1;
                    if nearby >= 8 {
                        return None;
                    }
                }
            }
        }
    }

    // Likewise: a cactus carries no frames in the save format.
    world.set_tile(x, y - 1, Tile::block(CACTUS));
    Some((x, y - 1))
}

/// Sand, in all four colours, and the muds that fall with it.
const FALLS: [u16; 6] = [53, 112, 116, 123, 224, 234];

/// Let unsupported sand fall one tile.
///
/// The game throws a falling-sand entity and lets it land; the result is the same and the server
/// only has to agree about where the tile ends up. Without this, sand simply hangs in the air —
/// dig under a desert and the ceiling stays put, which is one of the first things anybody notices.
pub fn fall_sand(world: &mut World, x: i32, y: i32) -> Option<(i32, i32)> {
    if !world.in_bounds(x, y) || !world.in_bounds(x, y + 1) {
        return None;
    }
    let here = world.tile(x, y);
    if !here.is_active() || !FALLS.contains(&here.block) {
        return None;
    }
    let below = world.tile(x, y + 1);
    if below.is_active() {
        return None;
    }

    world.set_tile(x, y + 1, here);
    world.set_tile(x, y, Tile::AIR);
    Some((x, y))
}

/// Sample tiles around a point and grow whatever will grow, the way `UpdateWorld` does.
///
/// The game scales its sample count with the world's area; this takes the same shape but is driven
/// from where the players actually are, because that is the only part of the world anybody can
/// see changing and the only part worth spending a tick on.
pub fn tick_growth(
    world: &mut World,
    around: &[(i32, i32)],
    samples: usize,
    reach: i32,
    rng: &mut SmallRng,
) -> Vec<(i32, i32)> {
    let mut changed = Vec::new();
    for &(cx, cy) in around {
        for _ in 0..samples {
            let x = cx + rng.random_range(-reach..=reach);
            let y = cy + rng.random_range(-reach..=reach);
            if let Some(at) = spread_grass(world, x, y) {
                changed.push(at);
            }
            // Herbs are rarer than grass by a wide margin, so they get their own, much longer,
            // odds rather than a try per sample.
            if rng.random_range(0..40) == 0
                && let Some(at) = plant_herb(world, x, y)
            {
                changed.push(at);
            }
            // Saplings are rarer still, and a tree changes a lot of tiles at once.
            if rng.random_range(0..60) == 0
                && let Some(at) = grow_tree(world, x, y, rng)
            {
                changed.push(at);
            }
            if rng.random_range(0..12) == 0
                && let Some(at) = grow_vine(world, x, y)
            {
                changed.push(at);
            }
            if rng.random_range(0..30) == 0
                && let Some(at) = grow_cactus(world, x, y)
            {
                changed.push(at);
            }
            // Sand is physics rather than growth, so it is tried every sample: a ceiling that
            // takes a minute to come down has already been walked under.
            if let Some(at) = fall_sand(world, x, y) {
                changed.push(at);
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    /// A world of dirt with one patch of grass in it, open to the air above.
    fn dirt_field(grass_at: Option<(i32, i32)>, grass: u16) -> World {
        let mut world = World::empty(40, 40, "growth");
        for x in 0..40 {
            for y in 20..40 {
                world.set_tile(x, y, Tile::block(DIRT));
            }
        }
        if let Some((x, y)) = grass_at {
            world.set_tile(x, y, Tile::block(grass));
        }
        world
    }

    /// Exposed dirt next to grass becomes grass.
    #[test]
    fn grass_creeps_onto_bare_dirt() {
        let mut world = dirt_field(Some((10, 20)), 2);
        assert_eq!(spread_grass(&mut world, 11, 20), Some((11, 20)));
        assert_eq!(world.tile(11, 20).block, 2);
    }

    /// Dirt with no grass anywhere near it stays dirt.
    #[test]
    fn dirt_alone_stays_dirt() {
        let mut world = dirt_field(None, 2);
        assert_eq!(spread_grass(&mut world, 11, 20), None);
        assert_eq!(world.tile(11, 20).block, DIRT);
    }

    /// Dirt sealed inside solid ground gets no light and stays dirt, however close the grass is.
    #[test]
    fn buried_dirt_stays_dirt() {
        let mut world = dirt_field(Some((10, 30)), 2);
        // (11, 30) is surrounded on all sides by the dirt field.
        assert_eq!(spread_grass(&mut world, 11, 30), None);
        assert_eq!(world.tile(11, 30).block, DIRT);
    }

    /// An evil grass wins a tile that plain grass also touches, so an infection can advance.
    #[test]
    fn corruption_beats_plain_grass() {
        let mut world = dirt_field(Some((10, 20)), 2);
        world.set_tile(12, 20, Tile::block(23));
        assert_eq!(spread_grass(&mut world, 11, 20), Some((11, 20)));
        assert_eq!(world.tile(11, 20).block, 23, "corruption should take it");
    }

    /// Jungle grass spreads onto mud, never onto dirt — the reverse of what this file's own doc
    /// used to claim ("a mud pit could never become a jungle").
    ///
    /// Fails before the fix: `spread_grass` only ever accepted `DIRT` as the base, so mud beside
    /// jungle grass stayed mud forever.
    #[test]
    fn jungle_grass_spreads_onto_mud_not_dirt() {
        let mut world = World::empty(40, 40, "growth");
        for x in 0..40 {
            for y in 20..40 {
                world.set_tile(x, y, Tile::block(MUD));
            }
        }
        world.set_tile(10, 20, Tile::block(60)); // jungle grass

        assert_eq!(spread_grass(&mut world, 11, 20), Some((11, 20)));
        assert_eq!(
            world.tile(11, 20).block,
            60,
            "mud should have turned to jungle grass"
        );
    }

    /// Dirt beside jungle grass does not turn to jungle grass at all — vanilla never spreads
    /// jungle grass onto dirt, only onto mud.
    ///
    /// Fails before the fix: the old code adopted whatever grass touched a dirt tile, jungle
    /// included, which is exactly the bug this file's own doc used to assert was impossible in
    /// the other direction.
    #[test]
    fn dirt_beside_jungle_grass_stays_dirt() {
        let mut world = dirt_field(Some((10, 20)), 60);
        assert_eq!(spread_grass(&mut world, 11, 20), None);
        assert_eq!(world.tile(11, 20).block, DIRT);
    }

    /// Corrupt grass spreads onto dirt as itself *and* onto mud as corrupt jungle grass — the two
    /// evils are the only grasses that take root on both bases at once.
    #[test]
    fn corrupt_grass_spreads_onto_both_dirt_and_mud() {
        let mut world = World::empty(40, 40, "growth");
        for x in 0..40 {
            for y in 20..40 {
                world.set_tile(x, y, Tile::block(if x < 20 { DIRT } else { MUD }));
            }
        }
        world.set_tile(10, 20, Tile::block(23)); // corrupt grass beside the dirt half
        world.set_tile(30, 20, Tile::block(23)); // and beside the mud half

        assert_eq!(spread_grass(&mut world, 11, 20), Some((11, 20)));
        assert_eq!(
            world.tile(11, 20).block,
            23,
            "dirt takes ordinary corrupt grass"
        );

        assert_eq!(spread_grass(&mut world, 31, 20), Some((31, 20)));
        assert_eq!(
            world.tile(31, 20).block,
            CORRUPT_JUNGLE_GRASS,
            "mud takes corrupt jungle grass instead"
        );
    }

    /// A herb appears on grass with open air above it.
    #[test]
    fn herbs_grow_on_grass() {
        let mut world = dirt_field(None, 2);
        world.set_tile(10, 20, Tile::block(2));
        assert_eq!(plant_herb(&mut world, 10, 20), Some((10, 19)));
        let herb = world.tile(10, 19);
        assert_eq!(herb.block, IMMATURE_HERB);
        assert_eq!(herb.frame_x, 0, "grass grows daybloom, which is style zero");
    }

    /// Jungle grass grows a different herb from ordinary grass.
    #[test]
    fn the_ground_decides_the_herb() {
        let mut world = dirt_field(None, 2);
        world.set_tile(10, 20, Tile::block(60));
        plant_herb(&mut world, 10, 20).expect("a herb");
        assert_eq!(world.tile(10, 19).frame_x, 18, "jungle grows moonglow");
    }

    /// A planted herb ripens into one worth picking.
    #[test]
    fn herbs_ripen() {
        let mut world = dirt_field(None, 2);
        world.set_tile(10, 20, Tile::block(2));
        plant_herb(&mut world, 10, 20).expect("a herb");
        assert_eq!(plant_herb(&mut world, 10, 19), Some((10, 19)));
        assert_eq!(world.tile(10, 19).block, MATURE_HERB);
    }

    /// Herbs thin themselves out rather than carpeting a field.
    #[test]
    fn herbs_keep_their_distance() {
        let mut world = dirt_field(None, 2);
        for x in 5..15 {
            world.set_tile(x, 20, Tile::block(2));
        }
        let mut planted = 0;
        for x in 5..15 {
            if plant_herb(&mut world, x, 20).is_some() {
                planted += 1;
            }
        }
        assert!(
            (1..=5).contains(&planted),
            "ten adjacent patches of grass gave {planted} herbs; they should thin out",
        );
    }

    /// Bare stone grows nothing.
    #[test]
    fn stone_grows_no_herbs() {
        let mut world = dirt_field(None, 2);
        world.set_tile(10, 20, Tile::block(1));
        assert_eq!(plant_herb(&mut world, 10, 20), None);
    }

    /// A sapling on grass with room above becomes a tree.
    #[test]
    fn saplings_become_trees() {
        let mut world = dirt_field(None, 2);
        for x in 9..12 {
            world.set_tile(x, 20, Tile::block(2));
        }
        world.set_tile(10, 19, Tile::framed(SAPLING, 0, 0));

        let mut rng = SmallRng::seed_from_u64(3);
        assert_eq!(grow_tree(&mut world, 10, 19, &mut rng), Some((10, 19)));
        assert_eq!(world.tile(10, 19).block, TREE);
        assert_eq!(world.tile(10, 18).block, TREE, "and it has some height");
        assert_eq!(world.tile(10, 19).frame_x, 0, "the plain trunk");
    }

    /// A sapling with a ceiling over it stays a sapling rather than growing into the rock.
    #[test]
    fn a_sapling_needs_headroom() {
        let mut world = dirt_field(None, 2);
        for x in 9..12 {
            world.set_tile(x, 20, Tile::block(2));
        }
        world.set_tile(10, 19, Tile::framed(SAPLING, 0, 0));
        world.set_tile(10, 16, Tile::block(1));

        let mut rng = SmallRng::seed_from_u64(3);
        assert_eq!(grow_tree(&mut world, 10, 19, &mut rng), None);
        assert_eq!(world.tile(10, 19).block, SAPLING);
    }

    /// Nothing grows out of bare stone.
    #[test]
    fn a_sapling_needs_soil() {
        let mut world = dirt_field(None, 2);
        for x in 9..12 {
            world.set_tile(x, 20, Tile::block(1));
        }
        world.set_tile(10, 19, Tile::framed(SAPLING, 0, 0));
        let mut rng = SmallRng::seed_from_u64(3);
        assert_eq!(grow_tree(&mut world, 10, 19, &mut rng), None);
    }

    /// Unsupported sand falls, and stops when it lands.
    #[test]
    fn sand_falls_until_it_lands() {
        let mut world = World::empty(40, 40, "sand");
        world.set_tile(10, 30, Tile::block(1)); // stone floor
        world.set_tile(10, 20, Tile::block(53)); // sand in mid-air

        // One call moves it one tile, so follow it down.
        let mut at = 20;
        while fall_sand(&mut world, 10, at).is_some() {
            at += 1;
        }
        assert_eq!(world.tile(10, 29).block, 53, "it should rest on the stone");
        assert!(!world.tile(10, 20).is_active(), "and not still be up there");
    }

    /// Sand already resting on something stays put.
    #[test]
    fn supported_sand_stays() {
        let mut world = World::empty(40, 40, "sand");
        world.set_tile(10, 30, Tile::block(1));
        world.set_tile(10, 29, Tile::block(53));
        assert_eq!(fall_sand(&mut world, 10, 29), None);
    }

    /// A vine hangs down from grass, and stops before it reaches the world's floor.
    #[test]
    fn vines_hang_from_grass() {
        let mut world = World::empty(40, 40, "vine");
        world.set_tile(10, 10, Tile::block(60)); // jungle grass ceiling

        assert_eq!(grow_vine(&mut world, 10, 10), Some((10, 11)));
        assert_eq!(
            world.tile(10, 11).block,
            62,
            "jungle grass grows jungle vine"
        );

        // It keeps going, one tile at a time, and then stops.
        let mut at = 11;
        let mut grown = 1;
        while grow_vine(&mut world, 10, at).is_some() {
            at += 1;
            grown += 1;
        }
        assert!(
            (2..=VINE_REACH).contains(&grown),
            "a vine should stop rather than trail forever: {grown} tiles",
        );
    }

    /// Stone grows no vines.
    #[test]
    fn stone_hangs_nothing() {
        let mut world = World::empty(40, 40, "vine");
        world.set_tile(10, 10, Tile::block(1));
        assert_eq!(grow_vine(&mut world, 10, 10), None);
    }

    /// A cactus grows up out of dry sand.
    #[test]
    fn cacti_grow_on_sand() {
        let mut world = World::empty(40, 40, "cactus");
        for x in 5..15 {
            world.set_tile(x, 20, Tile::block(53));
        }
        assert_eq!(grow_cactus(&mut world, 10, 20), Some((10, 19)));
        assert_eq!(world.tile(10, 19).block, CACTUS);
        // And it stacks on itself.
        assert_eq!(grow_cactus(&mut world, 10, 19), Some((10, 18)));
    }

    /// An oasis grows no cactus.
    #[test]
    fn wet_sand_grows_nothing() {
        let mut world = World::empty(40, 40, "cactus");
        for x in 5..15 {
            world.set_tile(x, 20, Tile::block(53));
        }
        for x in 8..13 {
            for y in 17..20 {
                let mut wet = world.tile(x, y);
                wet.liquid = 255;
                world.set_tile(x, y, wet);
            }
        }
        assert_eq!(grow_cactus(&mut world, 10, 20), None);
    }

    /// Stone is not a desert.
    #[test]
    fn stone_grows_no_cactus() {
        let mut world = World::empty(40, 40, "cactus");
        world.set_tile(10, 20, Tile::block(1));
        assert_eq!(grow_cactus(&mut world, 10, 20), None);
    }

    /// The sampler changes something, given a field it can work on.
    #[test]
    fn sampling_grows_something() {
        let mut world = dirt_field(Some((20, 20)), 2);
        let mut rng = SmallRng::seed_from_u64(7);
        let changed = tick_growth(&mut world, &[(20, 20)], 400, 6, &mut rng);
        assert!(!changed.is_empty(), "nothing grew in four hundred tries");
    }
}
