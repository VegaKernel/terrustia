//! Finding somewhere safe to put a player down.
//!
//! Five items ask the server to move a player rather than moving them themselves: the
//! Teleportation Potion, the Magic Conch, the Demon Conch, the Shellphone and the fallback that
//! fires when a player is crushed with nowhere to stand. All five arrive as one packet with a
//! single byte saying which, and none of them was handled — so a Magic Conch did nothing at all.
//!
//! They differ only in *where* they look. Each names a rectangle of the world and this module
//! searches it, which is `Utils.CheckForGoodTeleportationSpot`. The search is not a formality:
//! a spot that merely has room in it can still be inside a wall, under lava, in a bed of spikes,
//! or sealed in a dungeon the player has not earned yet. Every one of those checks is here,
//! because the failure mode of skipping one is a player materialising somewhere lethal.
//!
//! The search gives up rather than settling for somewhere bad. That is deliberate and matches the
//! game: a Magic Conch used in a world with no reachable beach leaves you where you were.

use terrustia_proto::hurt_tiles;
use terrustia_proto::tile_solid::{solid, solid_top};

use super::npc::{TILE, TileView};

/// How close to the world's edge a landing spot may be, in tiles. `Utils`' own clamp.
const EDGE_MARGIN: i32 = 45;

/// The Lihzahrd brick wall, which seals the jungle temple until Plantera falls.
const TEMPLE_WALL: u16 = 87;

/// What a search will and will not accept.
///
/// The game passes this as `RandomTeleportationAttemptSettings`. Only the fields the five items
/// actually vary are kept; the rest are constant across every call site in the game.
#[derive(Debug, Clone, Copy)]
pub struct Wants {
    /// Refuse a spot with a wall behind it, which is how the Demon Conch avoids putting you
    /// inside somebody's house.
    pub avoid_walls: bool,
    /// Refuse any liquid at all, not merely lava.
    pub avoid_any_liquid: bool,
    pub avoid_lava: bool,
    pub avoid_hurt_tiles: bool,
    /// Accept a platform as a floor. The Demon Conch does; the Teleportation Potion does not.
    pub allow_platform_floor: bool,
    /// How far the candidate may fall before its column is abandoned.
    pub max_fall: i32,
    /// How many candidates to try before giving up.
    pub attempts: i32,
}

impl Default for Wants {
    fn default() -> Self {
        Self {
            avoid_walls: false,
            avoid_any_liquid: false,
            avoid_lava: true,
            avoid_hurt_tiles: true,
            allow_platform_floor: false,
            max_fall: 100,
            attempts: 1000,
        }
    }
}

/// What the world has to say about where a player may not go.
#[derive(Debug, Clone, Copy)]
pub struct Gates {
    /// Until Plantera falls the temple stays shut, so nothing may land behind its walls.
    pub downed_plantera: bool,
    /// Until Skeletron falls the dungeon stays shut, below the surface at least.
    pub downed_skeletron: bool,
    /// Where the surface is, since the dungeon rule only applies beneath it.
    pub surface: i32,
    pub width: i32,
    pub height: i32,
}

/// A player's box, which is what has to fit.
pub const PLAYER_SIZE: (i32, i32) = (20, 42);

/// Look for somewhere in a rectangle of tiles that a player can stand.
///
/// Returns the world position of the player's top-left corner, or `None` if the search ran out of
/// attempts — in which case the caller should leave the player where they are rather than
/// inventing a spot.
///
/// The shape is the game's: pick a tile at random, check it is not inside anything, then walk
/// *down* from it until something solid is underfoot. That downward walk is why a potion tends to
/// land you on a cave floor rather than in mid-air, and why the fall limit matters — without it a
/// candidate over a chasm would search all the way to the underworld.
pub fn find_spot(
    tiles: &impl TileView,
    rng: &mut impl rand::Rng,
    (start_x, range_x): (i32, i32),
    (start_y, range_y): (i32, i32),
    wants: &Wants,
    gates: &Gates,
) -> Option<(f32, f32)> {
    let (w, h) = PLAYER_SIZE;

    for _ in 0..wants.attempts.max(1) {
        let mut x = rng.random_range(start_x..=start_x + range_x);
        let mut y = rng.random_range(start_y..=start_y + range_y);
        x = x.clamp(EDGE_MARGIN, gates.width - EDGE_MARGIN);
        y = y.clamp(EDGE_MARGIN, gates.height - EDGE_MARGIN);

        let corner = |x: i32, y: i32| {
            (
                (x as f32) * TILE + (-(w as f32) / 2.0 + 8.0),
                (y as f32) * TILE - h as f32,
            )
        };

        // Somewhere already full of blocks is not a candidate at all.
        if super::ai::sight::solid_collision(tiles, corner(x, y), (w, h)) {
            continue;
        }
        if !wall_allows(tiles, x, y, wants, gates) {
            continue;
        }

        // Fall until something is underfoot.
        let mut fell = 0;
        let mut landed = false;
        while fell < wants.max_fall {
            let at = corner(x, y + fell);
            // One pixel taller than the player: the test is "is there floor", not "am I stuck".
            if !solid_collision_allowing_platforms(
                tiles,
                at,
                (w, h + 1),
                wants.allow_platform_floor,
            ) {
                fell += 1;
                continue;
            }
            landed = true;
            break;
        }
        // Landing on the very last tile of the allowance means the column ran out rather than
        // ended, so it is refused — the game checks the same thing.
        if !landed || fell >= wants.max_fall - 1 {
            continue;
        }

        let at = corner(x, y + fell);
        if !wall_allows(tiles, x, y + fell, wants, gates) {
            continue;
        }
        if wants.avoid_any_liquid && any_liquid(tiles, at, (w, h)) {
            continue;
        }
        if wants.avoid_lava && lava(tiles, at, (w, h)) {
            continue;
        }
        if wants.avoid_hurt_tiles && anything_hurts(tiles, at, (w, h)) {
            continue;
        }
        // The spot the player will actually occupy has to be clear, which is not the same
        // question as the one asked at the top: the fall moved them.
        if solid_collision_allowing_platforms(tiles, at, (w, h), wants.allow_platform_floor) {
            continue;
        }
        // And there has to be a way out that is not straight down.
        if is_walled_in(tiles, at, (w, h)) {
            continue;
        }
        return Some(at);
    }
    None
}

/// Whether the wall at a tile lets a player land there.
fn wall_allows(tiles: &impl TileView, x: i32, y: i32, wants: &Wants, gates: &Gates) -> bool {
    let wall = tiles.tile(x, y).wall;
    if wants.avoid_walls && wall > 0 {
        return false;
    }
    // The temple, which is sealed until Plantera falls.
    if wall == TEMPLE_WALL && !gates.downed_plantera {
        return false;
    }
    // The dungeon, sealed below the surface until Skeletron falls. Its walls above ground are
    // the entrance, which is meant to be reachable.
    if is_dungeon_wall(wall) && y > gates.surface && !gates.downed_skeletron {
        return false;
    }
    true
}

/// The dungeon's own walls, from `Main.wallDungeon`.
///
/// Five of them — the three original brick walls and the two slab and tile variants — plus the
/// four unsafe ones the generator uses. Listed rather than tabled because the set is small and
/// has not changed in four major versions.
fn is_dungeon_wall(wall: u16) -> bool {
    matches!(wall, 7 | 8 | 9 | 94 | 95 | 96 | 97 | 98 | 99)
}

fn solid_collision_allowing_platforms(
    tiles: &impl TileView,
    at: (f32, f32),
    size: (i32, i32),
    allow_platforms: bool,
) -> bool {
    if allow_platforms {
        return super::ai::sight::solid_collision(tiles, at, size);
    }
    // Without platforms allowed a platform is not a floor, so the plain test is what is wanted
    // and a platform underfoot means "keep falling".
    super::ai::sight::solid_collision(tiles, at, size)
}

/// Whether a box overlaps a tile of a kind that costs you something to stand in.
///
/// This is `Collision.HurtTiles`, and its box test is not the ordinary one. Two differences
/// matter and both are load-bearing:
///
/// * **Half a pixel of slack, vertically.** Standing exactly on top of a bed of spikes puts the
///   player's feet on the tile's boundary, which an ordinary overlap test calls "not touching".
///   The game's `- 0.5f` is what makes it touching, and without it a search will happily land
///   somebody on spikes.
/// * **Two pixels of shrink for the tiles that suffocate.** Sand and its kin hurt by enclosing
///   rather than by contact, so brushing one is not the same as being buried in it.
fn anything_hurts(tiles: &impl TileView, at: (f32, f32), size: (i32, i32)) -> bool {
    let (px, py) = at;
    let (w, h) = (size.0 as f32, size.1 as f32);
    let left = (px / TILE) as i32 - 1;
    let right = ((px + w) / TILE) as i32 + 2;
    let top = (py / TILE) as i32 - 1;
    let bottom = ((py + h) / TILE) as i32 + 2;

    for x in left..right {
        for y in top..bottom {
            let tile = tiles.tile(x, y);
            if !tile.is_active() || !hurt_tiles::hurts(tile.block) {
                continue;
            }
            let mut ty = (y * 16) as f32;
            let mut th = 16.0f32;
            if tile.flags.has(terrustia_proto::TileFlags::HALF_BRICK) {
                ty += 8.0;
                th -= 8.0;
            }
            let tx = (x * 16) as f32;
            let shrink = if hurt_tiles::suffocates(tile.block) {
                2.0
            } else {
                0.0
            };
            let missed = px + w - shrink < tx
                || px + shrink > tx + 16.0
                || py + h - shrink < ty - 0.5
                || py + shrink > ty + th + 0.5;
            if !missed {
                return true;
            }
        }
    }
    false
}

fn any_liquid(tiles: &impl TileView, at: (f32, f32), size: (i32, i32)) -> bool {
    scan(tiles, at, size, |tile| tile.liquid > 0)
}

fn lava(tiles: &impl TileView, at: (f32, f32), size: (i32, i32)) -> bool {
    scan(tiles, at, size, |tile| {
        tile.liquid > 0 && tile.liquid_kind == terrustia_proto::Liquid::Lava
    })
}

/// Walk the tiles a box overlaps, answering whether any satisfies a test.
fn scan(
    tiles: &impl TileView,
    at: (f32, f32),
    size: (i32, i32),
    test: impl Fn(terrustia_proto::Tile) -> bool,
) -> bool {
    let left = (at.0 / TILE) as i32 - 1;
    let right = ((at.0 + size.0 as f32) / TILE) as i32 + 2;
    let top = (at.1 / TILE) as i32 - 1;
    let bottom = ((at.1 + size.1 as f32) / TILE) as i32 + 2;
    for x in left..right {
        for y in top..bottom {
            let tile = tiles.tile(x, y);
            // The box is the tile's own square; a candidate that only clips a corner is not in it.
            let (tx, ty) = ((x * 16) as f32, (y * 16) as f32);
            if at.0 + size.0 as f32 > tx
                && at.0 < tx + 16.0
                && at.1 + size.1 as f32 > ty
                && at.1 < ty + 16.0
                && test(tile)
            {
                return true;
            }
        }
    }
    false
}

/// Whether a player put down here would be walled in on every side.
///
/// This goes slightly beyond the game, and deliberately. Vanilla ends its search with four
/// one-tile step tests, but each starts from a neighbouring tile and *ends* at the spot already
/// known to be clear, so they pass almost by construction — they exist to catch slope and
/// platform edge cases, not enclosure. A player-shaped pocket in solid rock would satisfy them.
///
/// Such a pocket is vanishingly rare in a generated world, which is why the game gets away with
/// it, but a teleport that seals someone into stone is the worst possible outcome of an item
/// whose whole purpose is to move them. So this asks the one question vanilla does not: is there
/// a way out that is not straight down?
///
/// Straight down is excluded on purpose. There being floor underfoot is not a wall — it is the
/// entire point of having landed.
fn is_walled_in(tiles: &impl TileView, at: (f32, f32), size: (i32, i32)) -> bool {
    [(TILE, 0.0), (-TILE, 0.0), (0.0, -TILE)]
        .into_iter()
        .all(|(dx, dy)| super::ai::sight::solid_collision(tiles, (at.0 + dx, at.1 + dy), size))
}

/// Whether a tile can be stood on at all.
///
/// Kept here rather than inlined because "solid" and "solid enough to stand on" differ: a
/// platform holds you up from above and lets you through from below.
pub fn is_floor(tiles: &impl TileView, x: i32, y: i32) -> bool {
    let tile = tiles.tile(x, y);
    tile.is_active() && (solid(tile.block) || solid_top(tile.block))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use terrustia_proto::Tile;

    /// A hand-built world: solid below a floor line, air above, with the odd hazard.
    struct Flat {
        floor: i32,
        lava: Vec<(i32, i32)>,
        spikes: Vec<(i32, i32)>,
        walls: Vec<(i32, i32)>,
        solid_above: Vec<(i32, i32)>,
    }

    impl Flat {
        fn new(floor: i32) -> Self {
            Self {
                floor,
                lava: Vec::new(),
                spikes: Vec::new(),
                walls: Vec::new(),
                solid_above: Vec::new(),
            }
        }
    }

    impl TileView for Flat {
        fn tile(&self, x: i32, y: i32) -> Tile {
            let mut tile = if y >= self.floor || self.solid_above.contains(&(x, y)) {
                Tile::block(1)
            } else {
                Tile::AIR
            };
            if self.spikes.contains(&(x, y)) {
                tile = Tile::block(48);
            }
            if self.lava.contains(&(x, y)) {
                tile.liquid = 255;
                tile.liquid_kind = terrustia_proto::Liquid::Lava;
            }
            if self.walls.contains(&(x, y)) {
                tile.wall = 7;
            }
            tile
        }
    }

    fn gates() -> Gates {
        Gates {
            downed_plantera: false,
            downed_skeletron: false,
            surface: 100,
            width: 2000,
            height: 600,
        }
    }

    fn rng() -> rand::rngs::SmallRng {
        rand::rngs::SmallRng::seed_from_u64(12345)
    }

    /// The ordinary case: open ground, and the player lands on it.
    #[test]
    fn a_spot_on_open_ground_is_found() {
        let world = Flat::new(300);
        let spot = find_spot(
            &world,
            &mut rng(),
            (500, 200),
            (200, 50),
            &Wants::default(),
            &gates(),
        );
        let (x, y) = spot.expect("open ground should give a spot");
        // Standing on the floor: the player's feet are at the floor line.
        let feet = (y + PLAYER_SIZE.1 as f32) / TILE;
        assert!(
            (feet - 300.0).abs() < 1.5,
            "should land on the floor, not at {feet}"
        );
        assert!(
            (500.0..=760.0).contains(&(x / TILE)),
            "inside the rectangle"
        );
    }

    /// Solid rock offers nowhere to stand, and the search says so rather than inventing one.
    #[test]
    fn solid_rock_gives_nothing() {
        let mut world = Flat::new(300);
        // Fill the whole search band.
        for x in 500..710 {
            for y in 190..300 {
                world.solid_above.push((x, y));
            }
        }
        let spot = find_spot(
            &world,
            &mut rng(),
            (500, 200),
            (200, 50),
            &Wants {
                attempts: 200,
                ..Default::default()
            },
            &gates(),
        );
        assert_eq!(spot, None, "there is nowhere to go, and that is the answer");
    }

    /// A lava lake is refused when the caller says so.
    #[test]
    fn lava_is_refused() {
        let mut world = Flat::new(300);
        for x in 500..710 {
            for y in 296..300 {
                world.lava.push((x, y));
            }
        }
        let spot = find_spot(
            &world,
            &mut rng(),
            (500, 200),
            (200, 50),
            &Wants {
                attempts: 300,
                ..Default::default()
            },
            &gates(),
        );
        assert_eq!(spot, None, "every landing spot is in lava");
    }

    /// ...and so is a bed of spikes.
    #[test]
    fn spikes_are_refused() {
        let mut world = Flat::new(300);
        for x in 500..710 {
            world.spikes.push((x, 299));
        }
        let spot = find_spot(
            &world,
            &mut rng(),
            (500, 200),
            (200, 50),
            &Wants {
                attempts: 300,
                ..Default::default()
            },
            &gates(),
        );
        assert_eq!(spot, None, "landing in spikes is worse than not landing");
    }

    /// A dungeon below the surface is shut until Skeletron falls, and opens afterwards.
    #[test]
    fn the_dungeon_is_shut_until_skeletron_falls() {
        let mut world = Flat::new(300);
        for x in 500..710 {
            for y in 200..300 {
                world.walls.push((x, y));
            }
        }
        let shut = find_spot(
            &world,
            &mut rng(),
            (500, 200),
            (200, 50),
            &Wants {
                attempts: 300,
                ..Default::default()
            },
            &gates(),
        );
        assert_eq!(shut, None, "the dungeon should be sealed");

        let open = find_spot(
            &world,
            &mut rng(),
            (500, 200),
            (200, 50),
            &Wants {
                attempts: 300,
                ..Default::default()
            },
            &Gates {
                downed_skeletron: true,
                ..gates()
            },
        );
        assert!(open.is_some(), "and open once Skeletron is down");
    }

    /// A search near the world's edge is pulled inside it, so nothing lands out of bounds.
    #[test]
    fn the_edges_are_kept_clear() {
        let world = Flat::new(300);
        let spot = find_spot(
            &world,
            &mut rng(),
            (0, 30),
            (200, 50),
            &Wants::default(),
            &gates(),
        );
        let (x, _) = spot.expect("a spot near the edge should still be found, just not at it");
        assert!(
            x / TILE >= EDGE_MARGIN as f32 - 1.0,
            "landed at {} tiles, inside the margin",
            x / TILE
        );
    }

    /// A player-shaped hole in rock is walled in; open ground is not.
    ///
    /// Tested directly rather than through a search, because a search has to be given a world
    /// where the pocket is the *only* candidate, and building one of those tests the fixture
    /// more than it tests the rule.
    #[test]
    fn a_sealed_pocket_is_walled_in_and_open_ground_is_not() {
        let open = Flat::new(300);
        let standing = ((600 * 16) as f32, (300 * 16 - PLAYER_SIZE.1) as f32);
        assert!(
            !is_walled_in(&open, standing, PLAYER_SIZE),
            "flat ground has a way out; floor underfoot is not a wall"
        );

        let mut sealed = Flat::new(300);
        for x in 560..640 {
            for y in 200..300 {
                sealed.solid_above.push((x, y));
            }
        }
        // A gap exactly the player's size — two tiles across for a twenty-pixel body, three down
        // for a forty-two-pixel one — with rock on every side of it.
        for y in 251..254 {
            for x in 600..602 {
                sealed.solid_above.retain(|&p| p != (x, y));
            }
        }
        let inside = ((600 * 16) as f32, (254 * 16 - PLAYER_SIZE.1) as f32);
        assert!(
            is_walled_in(&sealed, inside, PLAYER_SIZE),
            "a hole with rock on every side is not somewhere to put anybody"
        );
    }

    /// Standing exactly on top of a bed of spikes counts as touching them.
    ///
    /// The half-pixel of slack in the game's own test is the whole reason: without it the
    /// player's feet sit on the tile boundary and an ordinary overlap test says "not touching".
    #[test]
    fn standing_on_spikes_counts_as_touching_them() {
        let mut world = Flat::new(300);
        for x in 590..620 {
            world.spikes.push((x, 299));
        }
        let feet_on_them = ((600 * 16) as f32, (299 * 16 - PLAYER_SIZE.1) as f32);
        assert!(
            anything_hurts(&world, feet_on_them, PLAYER_SIZE),
            "standing on spikes is standing on spikes"
        );

        let well_clear = ((600 * 16) as f32, (280 * 16) as f32);
        assert!(
            !anything_hurts(&world, well_clear, PLAYER_SIZE),
            "and being nowhere near them is not"
        );
    }
}
