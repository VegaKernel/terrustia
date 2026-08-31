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
use terrustia_proto::{Tile, TileFlags, tile_solid::solid};

use super::world::World;

/// `Tile.nactive()`: active, and not actuated.
///
/// An actuated tile is phased out, and every arm of the world update gates on this rather than on
/// bare `active()` (`WorldGen.cs:70140,72790,73850`). Using `active()` instead means grass creeps
/// over actuated dirt, cactus sprouts from actuated sand, and a player who actuates a block to
/// walk through it finds the world growing on it anyway.
fn nactive(tile: Tile) -> bool {
    tile.is_active() && !tile.flags.has(TileFlags::ACTUATED)
}

/// How far in from either edge the ocean reaches. `WorldGen.beachDistance`, the same fixed 380 the
/// rest of this project uses where vanilla reads that field.
const BEACH_DISTANCE: i32 = 380;

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
pub const GRASSES: [u16; 8] = [
    23,  // corrupt
    661, // corrupt jungle, which spreads exactly as ordinary corrupt grass does
    199, // crimson
    662, // crimson jungle, likewise
    109, // hallow
    60,  // jungle
    70,  // mushroom
    2,   // plain
];

/// Whether a grass spreads at all at this depth: `TileID.Sets.SpreadOverground` and
/// `SpreadUnderground` (`TileID.cs:417-419`), which are the gates the sampled tile has to pass
/// before `UpdateWorld_GrassGrowth` is reached at all (`WorldGen.cs:72937,73877`).
///
/// The difference that matters is plain grass: it is in the overground set and *not* in the
/// underground one, so a lawn does not creep through the caverns. The vanilla sets also carry the
/// two thorn blocks (32, 352), the two golf grasses (477, 492), ash grass (633) and lihzahrd brick
/// (226), none of which this project spreads.
fn spreads_at(block: u16, overground: bool) -> bool {
    match block {
        2 => overground,
        23 | 661 | 199 | 662 | 109 | 60 | 70 => true,
        _ => false,
    }
}

/// What a base tile becomes next to this grass, given which base it already is — `None` when this
/// grass does not grow on that base at all.
///
/// `WorldGen.SpreadGrass`'s own `dirt`/`grass` argument pairs, picked apart from the `TileFrame`
/// dispatcher that decides which pairs to try for a given neighbouring grass
/// (`WorldGen.cs:75225-75307`). Jungle (60) and mushroom (70) grass only ever list mud; the two
/// evils (23, 199) list both bases, taking their own jungle-grass form on mud.
fn grass_result(grass: u16, base: u16) -> Option<u16> {
    match (grass, base) {
        // `num10 == 23 || num10 == 661` collapses to the same `grass`/`num18` pair
        // (`WorldGen.cs:75227-75236`), so corrupt jungle grass spreads as corruption does. Without
        // this an infection that swallows a jungle stops dead at the far side of it.
        (23 | CORRUPT_JUNGLE_GRASS, DIRT) => Some(23),
        (23 | CORRUPT_JUNGLE_GRASS, MUD) => Some(CORRUPT_JUNGLE_GRASS),
        (199 | CRIMSON_JUNGLE_GRASS, DIRT) => Some(199),
        (199 | CRIMSON_JUNGLE_GRASS, MUD) => Some(CRIMSON_JUNGLE_GRASS),
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

/// Spread grass *from* a sampled tile onto everything around it that will take it.
///
/// This is the direction vanilla runs in, and it was the wrong way round here. `UpdateWorld` only
/// reaches `UpdateWorld_GrassGrowth` when the sampled tile is **already** a spreading grass
/// (`WorldGen.cs:72937` overground, `:73877` underground, both behind
/// `TileID.Sets.SpreadOverground`/`SpreadUnderground`), and that function then walks the 3x3 box
/// and converts **every** qualifying neighbour (`WorldGen.cs:75238-75260`): up to eight tiles from
/// one sample. Running it the other way, converting only the sampled tile when a grass happens to
/// be beside it, means a sample is wasted unless it lands exactly on the one-tile-wide frontier,
/// and even then it advances the edge by a single tile. On a 4200x1200 world that is 151 overground
/// samples a tick against roughly five million tiles, so the frontier is hit rarely enough that
/// grass and infections crawled.
///
/// One narrowing: which grass a converted tile takes is decided by [`spread_grass`] from all of
/// *its* neighbours by the priority in [`GRASSES`], rather than from the sampled tile's own type.
/// The two only disagree where a tile touches two different grasses at once, and there the priority
/// order (evil first) is what the existing behaviour and its tests already pin.
pub fn spread_grass_from(
    world: &mut World,
    x: i32,
    y: i32,
    overground: bool,
    out: &mut Vec<(i32, i32)>,
) {
    if !world.in_bounds(x, y) {
        return;
    }
    let here = world.tile(x, y);
    if !nactive(here) || !spreads_at(here.block, overground) {
        return;
    }
    for nx in (x - 1)..=(x + 1) {
        for ny in (y - 1)..=(y + 1) {
            if (nx, ny) == (x, y) {
                continue;
            }
            if let Some(at) = spread_grass(world, nx, ny) {
                out.push(at);
            }
        }
    }
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

/// Ripen an immature herb already growing on this tile.
///
/// Vanilla runs `GrowAlch` whenever a sampled tile is an alchemy plant (`WorldGen.cs:72659`);
/// this is the ripening half of that, kept apart from planting so the two can carry vanilla's very
/// different rates - ripening on any sample that lands on a herb, planting only one sample in
/// tens of thousands.
pub fn ripen_herb(world: &mut World, x: i32, y: i32) -> Option<(i32, i32)> {
    if !world.in_bounds(x, y) {
        return None;
    }
    let here = world.tile(x, y);
    if here.is_active() && here.block == IMMATURE_HERB {
        let mut grown = here;
        grown.block = MATURE_HERB;
        world.set_tile(x, y, grown);
        return Some((x, y));
    }
    None
}

/// Plant a herb on suitable ground with open air above it.
///
/// Herbs were not renewable at all: the ones a world is generated with were every one it would
/// ever have, so potions were a finite resource. Vanilla's `PlantAlch` plants them at a low rate
/// over the whole world and thins them by refusing where several already grow nearby
/// (`WorldGen.cs:46308`); this keeps the "not where there are already herbs" rule, which is what
/// stops a field turning solid green. The ripening half is [`ripen_herb`].
pub fn plant_herb(world: &mut World, x: i32, y: i32) -> Option<(i32, i32)> {
    if !world.in_bounds(x, y) || !world.in_bounds(x, y - 1) {
        return None;
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
    // already five. The box is world-scaled, `num3 = (int)(15 * maxTilesX / 4200.0)`, clamped to
    // `[4, maxTilesX - 4]` on each axis (`WorldGen.cs:46319-46326`), so 15 on a small world and 22
    // on a large one rather than the flat 12 this used.
    let reach = (15 * world.width()) / 4200;
    let mut nearby = 0;
    for nx in (x - reach).max(4)..=(x + reach).min(world.width() - 4) {
        for ny in (y - reach).max(4)..=(y + reach).min(world.height() - 4) {
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

/// `WorldGen.PlantAlch` (`WorldGen.cs:46308-46345`): plant one herb somewhere in the world.
///
/// The site is **not** the sampled tile. Vanilla rolls this once per overground sample and then
/// picks its own column anywhere in the world, its own starting row from a three-way weighted
/// draw, and scans *down* from there to the first solid ground:
///
/// ```text
/// x = Next(20, maxTilesX - 20)
/// y = Next(40) == 0 ? Next((rockLayer + maxTilesY) / 2, maxTilesY - 20)   // 1 in 40, deep
///   : Next(10) != 0 ? Next(worldSurface, maxTilesY - 20)                  // 9 in 10, underground
///   : Next(20, maxTilesY - 20)                                            // the rest, anywhere
/// while (y < maxTilesY - 20 && !tile[x, y].active()) y++;
/// ```
///
/// Nailing it to the sampled overground tile instead, as this used to, has two consequences. Most
/// overground samples land in open sky, where the scan never happens and the attempt is simply
/// wasted; and no attempt can ever reach the underground, so Moonglow, Deathweed, Blinkroot,
/// Fireblossom and Shiverthorn (every herb whose ground is not surface grass) never regrew at all.
/// About 90% of vanilla's attempts are underground.
pub fn plant_alch(world: &mut World, rng: &mut SmallRng) -> Option<(i32, i32)> {
    let (w, h) = (world.width(), world.height());
    // Every band below needs room inside the 20-tile margins; the unit-test worlds do not have it.
    if w <= 40 || h <= 40 {
        return None;
    }
    let x = rng.random_range(20..w - 20);
    let surface = i32::from(world.surface).clamp(20, h - 21);
    let rock = i32::from(world.rock_layer).clamp(20, h - 21);
    let deep = ((rock + h) / 2).clamp(20, h - 21);
    let mut y = if rng.random_range(0..40) == 0 {
        rng.random_range(deep..h - 20)
    } else if rng.random_range(0..10) != 0 {
        rng.random_range(surface..h - 20)
    } else {
        rng.random_range(20..h - 20)
    };
    while y < h - 20 && !world.tile(x, y).is_active() {
        y += 1;
    }
    if !nactive(world.tile(x, y)) {
        return None;
    }
    plant_herb(world, x, y)
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

/// `WorldGen.GrowMoreVines` (`WorldGen.cs:45990-46024`): whether the area is thin enough on vines
/// to take another.
///
/// This is the real rate limiter on vines. The roll in front of it is `genRand.Next(1)` overground,
/// which always succeeds (vanilla computes 60 or 20 and then overwrites it with `num24 = 1` two
/// lines later, `WorldGen.cs:73090-73098`), so overground vine growth is gated by density alone.
/// Vines in `[x-4, x+4] x [y-6, y+10]` are counted, and one below the source counts for
/// `1 + (j - y) * 2` rather than 1, so a long vine already hanging is worth much more than a short
/// one. Sixty is the ceiling.
///
/// One narrowing: vanilla also requires `Collision.CanHitLine` between the two tiles before
/// applying the depth weighting, so a vine behind a wall of rock counts as 1. This has no
/// line-of-sight test, so it weights every vine in the box, which makes it slightly stricter than
/// vanilla rather than looser.
fn grow_more_vines(world: &World, x: i32, y: i32) -> bool {
    const VINES: [u16; 6] = [52, 62, 115, 205, 382, 528];
    let mut count = 0i32;
    for nx in (x - 4)..=(x + 4) {
        for ny in (y - 6)..=(y + 10) {
            if !world.in_bounds(nx, ny) {
                continue;
            }
            let tile = world.tile(nx, ny);
            if !tile.is_active() || !VINES.contains(&tile.block) {
                continue;
            }
            count += 1;
            if ny > y {
                count += (ny - y) * 2;
            }
            if count > 60 {
                return false;
            }
        }
    }
    true
}

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
    } else if [52u16, 62, 115, 205, 382].contains(&here.block) {
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

/// Let an unsupported column of sand fall, all the way down.
///
/// The game throws a falling-sand projectile and lets it land, so a block that loses its support
/// arrives at the bottom in one go rather than descending a tile at a time; and because
/// `SpawnFallingBlockProjectile` runs from `TileFrame` on the tile above as well
/// (`WorldGen.cs:82665`), the whole stack comes down together. So does this: it finds the landing
/// row once and moves the contiguous run of falling blocks above `(x, y)` down as a unit.
///
/// **This is only half of vanilla's behaviour, and the missing half is the trigger.** Vanilla drops
/// sand from `TileFrame`, which runs on every tile edit, so a ceiling collapses the instant the
/// block under it is mined. Here the only trigger is a world-update sample landing on the tile: on
/// a 4200x1200 world that is 152 overground samples a tick spread over roughly 1.4 million
/// candidate cells, and 76 underground ones over roughly 3.5 million, so a given tile waits about
/// two and a half minutes on the surface and about thirteen underground before its collapse even
/// starts. Falling the whole way once it does start is what can be fixed from inside the world
/// module; wiring it to the tile-edit path needs the mine and place handlers, which live elsewhere.
pub fn fall_sand(world: &mut World, x: i32, y: i32, out: &mut Vec<(i32, i32)>) -> bool {
    if !world.in_bounds(x, y) || !world.in_bounds(x, y + 1) {
        return false;
    }
    if !nactive(world.tile(x, y)) || !FALLS.contains(&world.tile(x, y).block) {
        return false;
    }
    // How far there is to fall: down to the last empty tile in the world below this one.
    let mut floor = y;
    while world.in_bounds(x, floor + 1) && !world.tile(x, floor + 1).is_active() {
        floor += 1;
    }
    let drop = floor - y;
    if drop == 0 {
        return false;
    }
    // The contiguous run of falling blocks resting on this one comes with it.
    let mut top = y;
    while world.in_bounds(x, top - 1)
        && nactive(world.tile(x, top - 1))
        && FALLS.contains(&world.tile(x, top - 1).block)
    {
        top -= 1;
    }
    // Bottom upward, so a tile is only ever written after the one it is moving into has been read.
    for sy in (top..=y).rev() {
        let block = world.tile(x, sy);
        world.set_tile(x, sy + drop, block);
        world.set_tile(x, sy, Tile::AIR);
        out.push((x, sy));
        out.push((x, sy + drop));
    }
    true
}

/// Grow whatever will grow on one sampled tile, the way vanilla's `UpdateWorld_OvergroundTile` and
/// `UpdateWorld_UndergroundTile` do (`WorldGen.cs:72612,73812`). Every tile it changes is pushed
/// onto `out`, so the caller can reuse one buffer across the whole per-tick sweep and this stays
/// allocation-free on the common path where nothing grows.
///
/// `herb_plant_odds` is vanilla's `num7 * 100` PlantAlch rate (`WorldGen.cs:72129,72657`), which
/// widens with the world; a herb is planted somewhere in the world on one overground sample in
/// `herb_plant_odds`. `overground` selects between the two update functions, which share the herb
/// ripening and the grass spread but little else.
pub fn grow_at(
    world: &mut World,
    x: i32,
    y: i32,
    overground: bool,
    herb_plant_odds: u32,
    rng: &mut SmallRng,
    out: &mut Vec<(i32, i32)>,
) {
    if !world.in_bounds(x, y) {
        return;
    }
    // `PlantAlch`'s roll sits in the sampling loop, not in the tile update, and picks its own site
    // anywhere in the world (`WorldGen.cs:72118,72129`). It is not a growth on the sampled tile.
    if overground
        && rng.random_range(0..herb_plant_odds.max(1)) == 0
        && let Some(at) = plant_alch(world, rng)
    {
        out.push(at);
    }

    let here = world.tile(x, y);
    // `if (Main.tileAlch[type]) GrowAlch(i, j) else if ...`: a herb tile ripens and nothing else
    // about that tile is considered (`WorldGen.cs:72649-72651` overground, `:73846-73848`
    // underground). Both arms of the chain below are the `else`.
    if let Some(at) = ripen_herb(world, x, y) {
        out.push(at);
        return;
    }
    if here.is_active() && matches!(here.block, IMMATURE_HERB | MATURE_HERB | 84) {
        return;
    }

    // Overground, a submerged tile takes the water branch (lily pads, cattails, plants dying) and
    // reaches none of the growth below (`WorldGen.cs:72763`). The one thing it still does is spread
    // jungle grass, which is what lets a flooded jungle keep creeping. Underground has no such
    // gate.
    if overground && here.liquid > 32 {
        if here.block == 60 {
            spread_grass_from(world, x, y, overground, out);
        }
        return;
    }
    // Everything from here on is inside vanilla's `nactive()` gate.
    if !nactive(here) {
        return;
    }

    if overground {
        // A cactus already standing grows a segment one sample in fifteen (`WorldGen.cs:72810`),
        // but a *new* one from bare sand is one in three hundred and only well inland:
        // `i > beachDistance + 20 && i < maxTilesX - beachDistance - 20` (`WorldGen.cs:72865`).
        // Both used the 1-in-15 rate and neither had the inland gate, so cactus sprouted twenty
        // times too fast and all over the beaches, which vanilla keeps clear for sea oats and
        // coral. Those two rolls (1 in 25 for an oasis plant, 1 in 20 for a sea oat) take
        // precedence over the cactus in vanilla and are not modelled here, so the sand arm is
        // reached slightly more often than it should be.
        let inland = x > BEACH_DISTANCE + 20 && x < world.width() - BEACH_DISTANCE - 20;
        let cactus_odds = if here.block == CACTUS {
            15
        } else if inland {
            300
        } else {
            0
        };
        if cactus_odds > 0
            && rng.random_range(0..cactus_odds) == 0
            && let Some(at) = grow_cactus(world, x, y)
        {
            out.push(at);
        }
        // A forest sapling grows one sample in twenty (`WorldGen.cs:73017`, `genRand.Next(20)`).
        if rng.random_range(0..20) == 0
            && let Some(at) = grow_tree(world, x, y, rng)
        {
            out.push(at);
        }
        // Overground vines hang from plain grass and from vines already hanging, and from nothing
        // else: `type == 2 || type == 52 || type == 382` (`WorldGen.cs:73088`). The roll in front of
        // it is `genRand.Next(num24)` with `num24` computed as 60 or 20 and then overwritten with
        // `num24 = 1` (`:73090-73098`), so it always succeeds and the real limiter is
        // `GrowMoreVines`' density count. This used `Next(12)` for both depths, twelve times too
        // slow here. Vanilla also grows the jungle-wall vine (382) instead of 52 where the wall
        // behind is a jungle one; this only ever grows the plain 52 overground.
        if matches!(here.block, 2 | 52 | 382)
            && grow_more_vines(world, x, y)
            && let Some(at) = grow_vine(world, x, y)
        {
            out.push(at);
        }
    } else {
        // Underground vines are jungle only, and one sample in five:
        // `(type == 60 || type == 62) && genRand.Next(5) == 0 && GrowMoreVines(i, j)`
        // (`WorldGen.cs:73909`). This used `Next(12)`, 2.4 times too slow.
        if matches!(here.block, 60 | 62)
            && rng.random_range(0..5) == 0
            && grow_more_vines(world, x, y)
            && let Some(at) = grow_vine(world, x, y)
        {
            out.push(at);
        }
    }

    // Grass spreads *from* the sampled tile onto its neighbours; see `spread_grass_from`.
    spread_grass_from(world, x, y, overground, out);

    // Sand is physics rather than growth, so it is tried on every sample. See `fall_sand` for the
    // trigger this cannot reach from here.
    fall_sand(world, x, y, out);
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
        assert_eq!(ripen_herb(&mut world, 10, 19), Some((10, 19)));
        assert_eq!(world.tile(10, 19).block, MATURE_HERB);
    }

    /// Herbs thin themselves out rather than carpeting a field.
    ///
    /// A full-width world on purpose: the thinning box is `15 * maxTilesX / 4200`
    /// (`WorldGen.cs:46319`), so on the 40-wide worlds the rest of these tests use it rounds to
    /// zero, in vanilla exactly as here.
    #[test]
    fn herbs_keep_their_distance() {
        let mut world = World::empty(4200, 40, "growth");
        for x in 0..4200 {
            for y in 20..40 {
                world.set_tile(x, y, Tile::block(DIRT));
            }
        }
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

    /// Unsupported sand falls all the way to the floor in one go, and the whole stack above it
    /// comes down with it.
    ///
    /// Fails before the fix: one call moved one tile, so a ten-tile ceiling needed ten separate
    /// world-update samples to land on it, each a couple of minutes apart on the surface and about
    /// thirteen underground.
    #[test]
    fn sand_falls_until_it_lands() {
        let mut world = World::empty(40, 40, "sand");
        world.set_tile(10, 30, Tile::block(1)); // stone floor
        for y in 18..=20 {
            world.set_tile(10, y, Tile::block(53)); // a three-tile column in mid-air
        }

        let mut out = Vec::new();
        assert!(fall_sand(&mut world, 10, 20, &mut out));
        for y in 27..=29 {
            assert_eq!(
                world.tile(10, y).block,
                53,
                "the column should rest at y={y}"
            );
        }
        for y in 18..=20 {
            assert!(!world.tile(10, y).is_active(), "and nothing left at y={y}");
        }
    }

    /// Sand already resting on something stays put.
    #[test]
    fn supported_sand_stays() {
        let mut world = World::empty(40, 40, "sand");
        world.set_tile(10, 30, Tile::block(1));
        world.set_tile(10, 29, Tile::block(53));
        let mut out = Vec::new();
        assert!(!fall_sand(&mut world, 10, 29, &mut out));
        assert!(out.is_empty());
    }

    /// Actuated sand is phased out, and vanilla's world update skips it: `nactive()`, not
    /// `active()` (`WorldGen.cs:72790`).
    ///
    /// Fails before the fix: every gate in this module used bare `active()`, so an actuated block
    /// a player had phased out to walk through still fell, still grew grass, and still sprouted
    /// cactus.
    #[test]
    fn actuated_sand_does_not_fall() {
        let mut world = World::empty(40, 40, "sand");
        world.set_tile(10, 30, Tile::block(1));
        let mut sand = Tile::block(53);
        sand.flags.set(TileFlags::ACTUATED, true);
        world.set_tile(10, 20, sand);
        let mut out = Vec::new();
        assert!(!fall_sand(&mut world, 10, 20, &mut out));
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

    /// A sample that lands on grass converts every qualifying tile in the 3x3 box around it, not
    /// one tile per sample, and a sample that lands on bare dirt does nothing at all
    /// (`WorldGen.cs:75238-75260`).
    ///
    /// Fails before the fix: `grow_at` ran the spread the other way round, converting the sampled
    /// tile if a grass happened to be beside it. That turned one tile per sample at best, and only
    /// when the sample landed exactly on the frontier.
    #[test]
    fn a_grass_sample_converts_its_whole_neighbourhood() {
        let mut world = dirt_field(Some((20, 20)), 2);
        let mut rng = SmallRng::seed_from_u64(1);
        let mut changed = Vec::new();
        grow_at(&mut world, 20, 20, true, u32::MAX, &mut rng, &mut changed);

        // Both dirt tiles of the 3x3 box that are open to the air, from the one sample. The three
        // at y=21 are sealed inside the field, which `SpreadGrass`'s own box test refuses.
        for at in [(19, 20), (21, 20)] {
            assert_eq!(
                world.tile(at.0, at.1).block,
                2,
                "the box tile {at:?} should have turned to grass"
            );
        }
        assert!(changed.len() >= 2, "one sample, several tiles: {changed:?}");

        // And a sample on bare dirt, even right beside the grass, changes nothing: the spread runs
        // from the grass outward, which is the direction that was backwards.
        let mut elsewhere = Vec::new();
        grow_at(&mut world, 22, 20, true, u32::MAX, &mut rng, &mut elsewhere);
        assert!(
            world.tile(22, 20).block == DIRT && elsewhere.is_empty(),
            "a sample landing on dirt should grow nothing: {elsewhere:?}"
        );
    }

    /// Plain grass creeps overground and not underground: `TileID.Sets.SpreadOverground` carries
    /// type 2 and `SpreadUnderground` does not (`TileID.cs:417-419`).
    #[test]
    fn plain_grass_does_not_creep_underground() {
        let mut rng = SmallRng::seed_from_u64(2);
        for (overground, expected) in [(true, 2u16), (false, DIRT)] {
            let mut world = dirt_field(Some((20, 20)), 2);
            let mut changed = Vec::new();
            grow_at(
                &mut world,
                20,
                20,
                overground,
                u32::MAX,
                &mut rng,
                &mut changed,
            );
            assert_eq!(
                world.tile(21, 20).block,
                expected,
                "overground={overground}"
            );
        }
    }

    /// `PlantAlch` picks its own site anywhere in the world and scans down to real ground, so a
    /// herb can be planted underground from an overground sample (`WorldGen.cs:46308-46318`).
    ///
    /// Fails before the fix: planting was nailed to the sampled overground tile, so no attempt
    /// could ever reach below the surface and the five herbs whose ground is not surface grass
    /// (moonglow, deathweed, blinkroot, fireblossom, shiverthorn) never regrew at all.
    #[test]
    fn herbs_are_planted_underground_not_only_where_the_sample_landed() {
        // Jungle grass floor well below the surface line, and nothing at all above it.
        let mut world = World::empty(200, 400, "alch");
        world.surface = 100;
        world.rock_layer = 150;
        for x in 0..200 {
            world.set_tile(x, 300, Tile::block(60));
        }
        let mut rng = SmallRng::seed_from_u64(9);
        let mut planted = 0;
        for _ in 0..200 {
            if plant_alch(&mut world, &mut rng).is_some() {
                planted += 1;
            }
        }
        assert!(
            planted > 0,
            "nothing was planted underground in 200 attempts"
        );
        // Moonglow, which only grows on jungle grass and so could never have been planted from an
        // overground sample.
        let moonglow = (0..200)
            .filter(|x| world.tile(*x, 299).block == IMMATURE_HERB)
            .filter(|x| world.tile(*x, 299).frame_x == 18)
            .count();
        assert!(moonglow > 0, "no moonglow: {planted} herbs planted");
    }

    /// A new cactus grows from bare sand one sample in three hundred, and only well inland
    /// (`WorldGen.cs:72865`), while one already standing extends one sample in fifteen
    /// (`WorldGen.cs:72810`).
    ///
    /// Fails before the fix: both used the 1-in-15 rate and neither had the inland gate, so cactus
    /// sprouted twenty times too fast and all over the beaches vanilla keeps clear.
    #[test]
    fn new_cactus_is_rare_and_inland() {
        let sprouted = |x: i32, seed: u64| {
            let mut world = World::empty(4200, 60, "cactus");
            for cx in 0..4200 {
                world.set_tile(cx, 30, Tile::block(53));
                // Bedrock under the sand, or it simply falls to the bottom of the world.
                world.set_tile(cx, 31, Tile::block(1));
            }
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut changed = Vec::new();
            for _ in 0..600 {
                grow_at(&mut world, x, 30, true, u32::MAX, &mut rng, &mut changed);
            }
            world.tile(x, 29).block == CACTUS
        };
        // On the beach, inside `beachDistance + 20`, nothing at all however long it is sampled.
        assert!(
            !(0..8).any(|seed| sprouted(200, seed)),
            "a cactus grew on the beach"
        );
        // Well inland it does, eventually.
        assert!(
            (0..8).any(|seed| sprouted(2100, seed)),
            "no cactus grew inland in eight runs of six hundred samples"
        );
    }

    /// Underground vines hang one sample in five, not one in twelve (`WorldGen.cs:73909`), and
    /// only from jungle grass and jungle vine.
    #[test]
    fn underground_vines_hang_from_jungle_grass() {
        let mut world = World::empty(60, 60, "vine");
        world.set_tile(20, 20, Tile::block(60));
        let mut rng = SmallRng::seed_from_u64(4);
        let mut changed = Vec::new();
        for _ in 0..60 {
            grow_at(&mut world, 20, 20, false, u32::MAX, &mut rng, &mut changed);
        }
        assert_eq!(world.tile(20, 21).block, 62, "a jungle vine should hang");
    }

    /// A submerged overground tile takes vanilla's water branch and grows nothing
    /// (`WorldGen.cs:72763`), except that jungle grass still spreads.
    #[test]
    fn a_submerged_overground_tile_does_not_grow() {
        let mut world = dirt_field(Some((20, 20)), 2);
        let mut wet = world.tile(20, 20);
        wet.liquid = 100;
        world.set_tile(20, 20, wet);
        let mut rng = SmallRng::seed_from_u64(5);
        let mut changed = Vec::new();
        grow_at(&mut world, 20, 20, true, u32::MAX, &mut rng, &mut changed);
        assert!(changed.is_empty(), "a drowned tile grew: {changed:?}");
    }

    /// The sampler changes something, given a field it can work on.
    #[test]
    fn sampling_grows_something() {
        let mut world = dirt_field(Some((20, 20)), 2);
        let mut rng = SmallRng::seed_from_u64(7);
        let mut changed = Vec::new();
        for _ in 0..400 {
            // Tight around the one patch of grass: the spread now runs *from* a grass tile, so a
            // sample that lands on bare dirt correctly does nothing at all.
            let x = 20 + rng.random_range(-6..=6);
            let y = 20 + rng.random_range(-1..=1);
            grow_at(&mut world, x, y, true, 40, &mut rng, &mut changed);
        }
        assert!(!changed.is_empty(), "nothing grew in four hundred tries");
    }
}
