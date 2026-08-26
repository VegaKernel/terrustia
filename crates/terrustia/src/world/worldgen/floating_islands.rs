//! Floating islands and their houses: `CloudIsland`/`CloudLake` (the driving `FloatingIslands`
//! pass, `WorldGen.cs:12988-13183`) and `IslandHouse` (the driving `FloatingIslandHouses` pass,
//! `:17986-18001`, calling `IslandHouse` itself, `:80365-80660`) — together ≥800 real lines read
//! in full.
//!
//! **What an ordinary world's own call graph actually reaches.** The driving `FloatingIslands`
//! pass has branches for `SnowCloudIsland`/`DesertCloudIsland` (drunk-world only) and several
//! `islandStyle` variants gated on remix/tenth-anniversary/"get fixed boi" world generation; none
//! of those conditions are ever true for an ordinary world (this project's own stated policy is
//! secret seeds are in scope but deprioritized — see `plan.md`'s "Secret seeds" backlog row), so
//! `islandStyle` stays `0` and only `CloudIsland` and `CloudLake` are ever called on the path this
//! module transcribes. That narrows real vanilla surface area a great deal before any deliberate
//! cut below even starts.
//!
//! **What's transcribed, faithfully:** the site-picking loop (x drawn from the middle 80% of the
//! world, excluding a ±150 band around dead centre and a ±180 band around every earlier site; y
//! found by scanning down to the real surface and then rolling well above it); the shared
//! teardrop-blob random walk both `CloudIsland` and `CloudLake` grow their main mass with
//! ([`grow_blob`]); `CloudIsland`'s second growth phase recolouring the upper mass to dirt
//! ([`cap_with_dirt`]) so real grass has something to spread onto later, exactly like vanilla's own
//! floating islands; `CloudLake`'s second growth phase carving a water-filled basin into the top
//! instead ([`carve_lake_basin`]); the Cloud-wall background fill once a mass is enclosed
//! ([`fill_walls`]); and `IslandHouse`'s real room (a solid Sunplate shell, a hollowed wall-lined
//! interior, a door on a random side, a support post on the other), its real chest — vanilla's own
//! four-item Shiny Red Balloon/Starfury/Lucky Horseshoe/Celestial Magnet cycle
//! (`WorldGen.cs:80522-80550`), not a placeholder table.
//!
//! **What's disclosed and skipped**, the same "narrower, disclosed" shape every Tier 2 item this
//! session has used:
//! * Both underside "icicle" decorator loops (`CloudIsland`/`CloudLake` each grow small hanging
//!   cloud/rain-cloud puffs below the main mass at semi-random intervals) and `CloudIsland`'s own
//!   trailing loop of 0-3 small satellite mini-clouds nearby — purely decorative, no gameplay
//!   effect.
//! * The surface-pond carving loop (`WorldGen.cs:79518-79562`/`:79943-79987`) that drops small
//!   puddles onto an island's own topside at random.
//! * `IslandHouse`'s furniture catalogue: two decorative Glass-wall "windows", a table, two chairs,
//!   and roof-corner banners (`WorldGen.cs:80568-80637`) — the same class of cut
//!   `underground_cabins.rs` already made for its own `FillRooms` furniture.
//! * `IslandHouse`'s 30-tile scan for nearby dungeon-brick *wall* (`Main.wallDungeon`, plus three
//!   more specific wall ids) is replaced with a direct check against [`Layout::dungeon_x`] — this
//!   generator already knows exactly where the dungeon is, which is strictly more reliable than
//!   vanilla's own conservative wall scan (the same kind of "use what `Layout` already knows rather
//!   than re-deriving vanilla's approximation" call `jungle_shrines.rs` made for its own siting).
//! * The shell's top-row corner chamfer (`WorldGen.cs:80447`) — two cells skipped per house,
//!   cosmetic only. The shell here is a plain rectangle.
//! * Every `islandStyle`/`remixWorldGen`/`drunkWorldGen`/`tenthAnniversaryWorldGen` branch in both
//!   the driving pass and `IslandHouse` — dead code on the path an ordinary world's own generation
//!   ever reaches, not a cut of anything a real player would see.
//!
//! **One real widening, the same bug class already found for `oasis.rs`/`pyramids.rs`/
//! `living_trees.rs`.** Vanilla scans down from row 200 to the live `Main.worldSurface` for a
//! column's own ground. This generator's [`Layout::surface`] is an *average*, not a live per-column
//! reading — `terrain::heightmap` clamps every real column to `layout.surface - 24 ..=
//! layout.surface + 20` (`terrain.rs`'s own `ROLL` constant), so scanning only up to
//! `layout.surface` missed the columns sitting in the lower half of that range and wasted retries.
//! Widened to `layout.surface + 20`, the real documented upper bound.
//!
//! **A small-world guard**, the same shape every scatter-style Tier 2 pass has needed: the ±150
//! centre-exclusion band alone is 300 tiles wide, and a world narrower than that relative to its
//! own 80%-of-width placement range makes the site-picking `while` loop spin forever redrawing `x`
//! values that can never escape the excluded band. Guarded at `layout.width < 900`.

use terrustia_proto::{ItemStack, Tile, tile_solid};

use super::layout::Layout;
use super::place_object::place_object;
use super::rand::UnifiedRandom;
use super::structure_map::{Rect, StructureMap};
use super::structures;
use super::tiles;
use crate::world::World;

/// What a call to [`scatter`] placed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// `CloudIsland`s grown — most of these also get a house.
    pub islands: usize,
    /// How many of those islands actually got a house built on them.
    pub houses: usize,
    /// `CloudLake`s grown — the lake-topped variant, never gets a house.
    pub lakes: usize,
}

/// The `FloatingIslands` + `FloatingIslandHouses` passes, merged into one call: for each site, grow
/// the mass and (for an ordinary island, not a lake) build its house immediately after, rather than
/// vanilla's own two separate generation passes run tens of thousands of lines apart. Nothing
/// between them in vanilla's own pass list touches the sky layer these sites live in, so nothing is
/// lost by not splitting this into two `build()` steps.
pub fn scatter(
    world: &mut World,
    layout: &Layout,
    structures: &mut StructureMap,
    rand: &mut UnifiedRandom,
) -> Report {
    // See the module doc's "small-world guard" note.
    if layout.width < 900 || layout.surface < 220 {
        return Report::default();
    }

    // `WorldGen.cs:12995`: `(int)(Main.maxTilesX * 0.0008)`.
    let island_budget = (f64::from(layout.width) * 0.0008) as i32;
    // `WorldGen.cs:11209-11217`.
    let mut sky_lakes = 1;
    if layout.width > 8000 {
        sky_lakes += 1;
    }
    if layout.width > 6000 {
        sky_lakes += 1;
    }
    let total = island_budget + sky_lakes;

    let mut site_xs: Vec<i32> = Vec::new();
    let mut house_index = 0i32;
    let mut report = Report::default();

    for _ in 0..total {
        let mut tries = layout.width;
        loop {
            if tries <= 0 {
                break;
            }
            tries -= 1;

            let lo = (f64::from(layout.width) * 0.1) as i32;
            let hi = (f64::from(layout.width) * 0.9) as i32;
            let mut x = rand.next_range(lo, hi);
            let mid = layout.width / 2;
            while x > mid - 150 && x < mid + 150 {
                x = rand.next_range(lo, hi);
            }
            if site_xs.iter().any(|&hx| x > hx - 180 && x < hx + 180) {
                continue;
            }

            // See the module doc's "one real widening" note.
            let mut surface_y = None;
            for k in 200..(layout.surface + 20) {
                if world.tile(x, k).is_active() {
                    surface_y = Some(k);
                    break;
                }
            }
            let Some(surface_y) = surface_y else {
                continue;
            };

            let site_y = rand.next_range(90, surface_y - 100);
            let is_lake = report.islands as i32 >= island_budget;

            let bounds = grow_blob(world, rand, tiles::CLOUD, x, site_y);
            if is_lake {
                carve_lake_basin(world, rand, tiles::CLOUD, x, bounds.y);
                fill_walls(world, bounds);
                report.lakes += 1;
            } else {
                cap_with_dirt(world, rand, tiles::CLOUD, x, bounds.y);
                fill_walls(world, bounds);
                report.islands += 1;
                if build_house(world, structures, rand, layout, x, site_y, &mut house_index) {
                    report.houses += 1;
                }
            }
            site_xs.push(x);
            break;
        }
    }

    report
}

/// A random-walk teardrop-shaped blob of `material`, shared identically between `CloudIsland`'s and
/// `CloudLake`'s own main mass (`WorldGen.cs:79207-79338` and `:79657-79788` — the two are the same
/// algorithm, confirmed line for line, only what happens to the tiles *afterward* differs). Returns
/// the tight bounding box of everything it placed.
fn grow_blob(world: &mut World, rand: &mut UnifiedRandom, material: u16, cx: i32, cy: i32) -> Rect {
    let width = world.width();
    let height = world.height();

    let mut radius = f64::from(rand.next_range(100, 150));
    let mut life = rand.next_range(20, 30);
    let mut px = f64::from(cx);
    let mut py = f64::from(cy);
    let mut vx = biased_velocity(rand);
    // Always rolled below -0.2, so the very first clamp check below locks it there — see the
    // clamp's own comment.
    let mut vy = f64::from(rand.next_range(-20, -10)) * 0.02;

    let (mut min_x, mut max_x, mut min_y, mut max_y) = (cx, cx, cy, cy);

    while radius > 0.0 && life > 0 {
        radius -= f64::from(rand.next_max(4));
        life -= 1;

        let x0 = ((px - radius * 0.5) as i32).max(0);
        let x1 = ((px + radius * 0.5) as i32).min(width);
        let y0 = ((py - radius * 0.5) as i32).max(0);
        let y1 = ((py + radius * 0.5) as i32).min(height);
        let local = radius * f64::from(rand.next_range(80, 120)) * 0.01;

        let mut top = py + 1.0;
        for x in x0..x1 {
            if rand.next_bool() {
                top += f64::from(rand.next_range(-1, 2));
            }
            top = top.clamp(py, py + 2.0);
            for y in y0..y1 {
                if (y as f64) <= top {
                    continue;
                }
                let dx = (x as f64 - px).abs();
                let dy = (y as f64 - py).abs() * 3.0;
                if (dx * dx + dy * dy).sqrt() < local * 0.4 {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                    world.set_tile(x, y, Tile::block(material));
                }
            }
        }

        px += vx;
        py += vy;
        vx += f64::from(rand.next_range(-20, 21)) * 0.05;
        vx = vx.clamp(-1.0, 1.0);
        // Two separate vanilla checks (not an else-if): once `vy` strays outside ±0.2 it is forced
        // to exactly -0.2 (a bounce, not a clamp to the boundary) — transcribed as written.
        if !(-0.2..=0.2).contains(&vy) {
            vy = -0.2;
        }
    }

    Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

/// A non-zero horizontal drift, redrawn until it lands outside `(-2.0, 2.0)` — both `CloudIsland`
/// and `CloudLake` roll this the same way for every growth phase they have.
fn biased_velocity(rand: &mut UnifiedRandom) -> f64 {
    loop {
        let v = f64::from(rand.next_range(-20, 21)) * 0.2;
        if !(-2.0..2.0).contains(&v) {
            return v;
        }
    }
}

/// Recolours the upper portion of a freshly grown island from `material` to Dirt — the solid,
/// grass-growable "land" a real floating island is walked on, as distinct from the fluffy cloud
/// hanging underneath it. `CloudIsland`'s second growth phase, `WorldGen.cs:79373-79437`.
fn cap_with_dirt(world: &mut World, rand: &mut UnifiedRandom, material: u16, cx: i32, top_y: i32) {
    let width = world.width();
    let height = world.height();
    let mut radius = f64::from(rand.next_range(80, 95));
    let mut life = rand.next_range(10, 15);
    let mut px = f64::from(cx);
    let mut py = f64::from(top_y);
    let mut vx = biased_velocity(rand);
    let mut vy = f64::from(rand.next_range(-20, -10)) * 0.02;
    // Fixed for the whole phase — vanilla's own `num11 = num6 - 1` is never updated inside the
    // loop, unlike `py`, which does move.
    let y0 = (top_y - 1).max(0);

    while radius > 0.0 && life > 0 {
        radius -= f64::from(rand.next_max(4));
        life -= 1;

        let x0 = ((px - radius * 0.5) as i32).max(0);
        let x1 = ((px + radius * 0.5) as i32).min(width);
        let y1 = ((py + radius * 0.5) as i32).min(height);
        let local = radius * f64::from(rand.next_range(80, 120)) * 0.01;

        let mut top = py + 1.0;
        for x in x0..x1 {
            if rand.next_bool() {
                top += f64::from(rand.next_range(-1, 2));
            }
            top = top.clamp(py, py + 2.0);
            for y in y0..y1 {
                if (y as f64) <= top {
                    continue;
                }
                let dx = (x as f64 - px).abs();
                let dy = (y as f64 - py).abs() * 3.0;
                if (dx * dx + dy * dy).sqrt() < local * 0.4 && world.tile(x, y).block == material {
                    world.set_tile(x, y, Tile::block(tiles::DIRT));
                }
            }
        }

        px += vx;
        py += vy;
        vx += f64::from(rand.next_range(-20, 21)) * 0.05;
        vx = vx.clamp(-1.0, 1.0);
        if !(-0.2..=0.2).contains(&vy) {
            vy = -0.2;
        }
    }
}

/// Carves a lens-shaped basin into the top of a lake island's mass and fills what will actually
/// stay put with water — the feature that makes a `CloudLake` a lake rather than an ordinary
/// island. `CloudLake`'s second growth phase, `WorldGen.cs:79823-79921`.
///
/// One real simplification, disclosed: vanilla also clears the background wall on five neighbouring
/// cells around each carved tile so the rim doesn't show a wall seam later; this clears only the
/// carved tile's own wall (`Tile::AIR`'s default).
fn carve_lake_basin(
    world: &mut World,
    rand: &mut UnifiedRandom,
    material: u16,
    cx: i32,
    top_y: i32,
) {
    let width = world.width();
    let mut radius = f64::from(rand.next_range(80, 95));
    let mut life = rand.next_range(10, 15);
    let mut px = f64::from(cx);
    let mut py = f64::from(top_y);
    let mut vx = biased_velocity(rand);
    let mut vy = f64::from(rand.next_range(-20, -10)) * 0.02;
    let y0 = (top_y - 1).max(0);

    while radius > 0.0 && life > 0 {
        radius -= f64::from(rand.next_max(4));
        life -= 1;

        let x0 = ((px - radius * 0.5) as i32).max(0);
        let x1 = ((px + radius * 0.5) as i32).min(width);
        let y1 = (py + radius * 0.5) as i32;
        let local = radius * f64::from(rand.next_range(80, 120)) * 0.01;

        let mut top = py + 1.0;
        for x in x0..x1 {
            if rand.next_bool() {
                top += f64::from(rand.next_range(-1, 2));
            }
            top = top.clamp(py, py + 2.0);
            for y in y0..y1.max(y0) {
                if (y as f64) <= top - 2.0 {
                    continue;
                }
                let dx = (x as f64 - px).abs();
                let dy = (y as f64 - py).abs() * 3.0;
                if (dx * dx + dy * dy).sqrt() >= local * 0.4 {
                    continue;
                }
                if !world.in_bounds(x, y) || world.tile(x, y).block != material {
                    continue;
                }
                let mut t = Tile::AIR;
                if (y as f64) > top + 1.0 && water_will_stay(world, x, y) {
                    t.liquid = 255;
                }
                world.set_tile(x, y, t);
            }
        }

        px += vx;
        py += vy;
        vx += f64::from(rand.next_range(-20, 21)) * 0.05;
        vx = vx.clamp(-1.0, 1.0);
        // `CloudLake`'s own clamp floors at 0.0 rather than -0.2 here (`WorldGen.cs:79913-79920`),
        // biasing the basin to dig a shallower, more contained lens than the island's own dirt cap.
        if vy > 0.2 {
            vy = -0.2;
        }
        if vy < 0.0 {
            vy = 0.0;
        }
    }
}

/// `WorldGen.cs:79605-79612`: water placed here will not immediately flow away.
fn water_will_stay(world: &World, x: i32, y: i32) -> bool {
    let solid_ground = |t: Tile| t.is_active() && tile_solid::solid(t.block);
    (solid_ground(world.tile(x, y + 1)) || world.tile(x, y + 1).liquid == 255)
        && (solid_ground(world.tile(x - 1, y)) || world.tile(x - 1, y).liquid == 255)
        && (solid_ground(world.tile(x + 1, y)) || world.tile(x + 1, y).liquid == 255)
}

/// Seals a completed mass with the Cloud background wall wherever a tile's full 3x3 neighbourhood
/// is already solid — identical between `CloudIsland` and `CloudLake`, `WorldGen.cs:79496-79517`/
/// `:79922-79942`.
fn fill_walls(world: &mut World, bounds: Rect) {
    for x in (bounds.x - 20)..=(bounds.right() + 20) {
        for y in (bounds.y - 20)..=(bounds.bottom() + 20) {
            if !world.in_bounds(x, y) {
                continue;
            }
            let mut sealed = true;
            'neighbourhood: for nx in (x - 1)..=(x + 1) {
                for ny in (y - 1)..=(y + 1) {
                    if !world.in_bounds(nx, ny) {
                        sealed = false;
                        break 'neighbourhood;
                    }
                    let t = world.tile(nx, ny);
                    if !t.is_active() || (t.wall > 0 && t.wall != tiles::walls::CLOUD) {
                        sealed = false;
                        break 'neighbourhood;
                    }
                }
            }
            if sealed {
                let mut t = world.tile(x, y);
                t.wall = tiles::walls::CLOUD;
                world.set_tile(x, y, t);
            }
        }
    }
}

/// `IslandHouse` (`WorldGen.cs:80365-80660`), narrowed to what an ordinary world's own call reaches
/// — see the module doc for exactly what that trims. `(x, y)` matches vanilla's own `(i, j)`: `y`
/// is near the house's *floor*, not its centre — the room is carved upward from it.
#[allow(clippy::too_many_arguments)]
fn build_house(
    world: &mut World,
    structures: &mut StructureMap,
    rand: &mut UnifiedRandom,
    layout: &Layout,
    x: i32,
    y: i32,
    house_index: &mut i32,
) -> bool {
    // Replaces vanilla's own 30-tile dungeon-*wall* scan — see the module doc.
    if (x - layout.dungeon_x).abs() < 250 {
        return false;
    }

    let dir = if rand.next_bool() { 1 } else { -1 };
    let half_w = rand.next_range(7, 12);
    let half_h = rand.next_range(5, 7);

    // The real write footprint: `WorldGen.cs:80443-80457`'s shell bounds.
    let shell = Rect::new(x - half_w - 1, y - half_h - 2, half_w * 2 + 3, half_h + 5);
    if !structures.can_place(world, shell, 2) {
        return false;
    }

    // The outer shell, one tile thick and solid. Vanilla chamfers the top row's two corners; not
    // reproduced here, see the module doc.
    for n in (x - half_w - 1)..=(x + half_w + 1) {
        for m in (y - half_h - 2)..=(y + 2) {
            world.set_tile(n, m, Tile::block(tiles::SUNPLATE));
        }
    }
    // Hollowed and lined with wall — `WorldGen.cs:80478-80488`. Note the room is *not* centred on
    // `y`: it runs from `y - half_h` up to `y` itself, with the floor starting at `y + 1`.
    for n in (x - half_w)..=(x + half_w) {
        for m in (y - half_h)..=y {
            let mut t = Tile::AIR;
            t.wall = tiles::walls::SUNPLATE;
            world.set_tile(n, m, t);
        }
    }

    // The door, on whichever side `dir` picked, opening upward from the floor.
    let door_x = x + (half_w + 1) * dir;
    for dx in -2..=2 {
        world.set_tile(door_x + dx, y, Tile::AIR);
        world.set_tile(door_x + dx, y - 1, Tile::AIR);
        world.set_tile(door_x + dx, y - 2, Tile::AIR);
    }
    place_object(world, door_x, y, 10, 9, -1);

    // A solid support post on the opposite wall, running from the ceiling to below the floor —
    // `WorldGen.cs:80512-80521`.
    let post_x = x - dir * (half_w + 2);
    for m in (y - half_h)..=(y + 2) {
        world.set_tile(post_x, m, Tile::block(tiles::SUNPLATE));
    }

    // The chest: vanilla's real four-item cycle (`WorldGen.cs:80522-80550`) — Shiny Red Balloon,
    // Starfury, Lucky Horseshoe, Celestial Magnet, in that order for the first four houses in a
    // world, then rolled randomly among the same four past that.
    const SIGNATURES: [i32; 4] = [159, 65, 158, 2219];
    let signature = if *house_index < 4 {
        SIGNATURES[*house_index as usize]
    } else {
        SIGNATURES[rand.next_max(4) as usize]
    };
    *house_index += 1;
    let items = vec![
        ItemStack::new(signature, 1, 0),
        ItemStack::new(8, rand.next_range(10, 30) as i16, 0),
        ItemStack::new(71, rand.next_range(10, 99) as i16, 0),
    ];
    structures::add_chest(world, x, y, items, rand);

    structures.add_protected_structure(shell, 2);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real 4200x1200 world, generated far enough to have a real surface line — everything
    /// [`scatter`] needs to find sites against.
    fn surfaced_world(seed: i32) -> (World, Layout) {
        let mut world = World::empty(4200, 1200, "floating-islands");
        let mut rand = UnifiedRandom::new(seed);
        let layout = Layout::plan(4200, 1200, &mut rand);
        let heights = super::super::terrain::heightmap(&layout, &mut rand);
        super::super::terrain::fill(&mut world, &layout, &heights, &mut rand);
        (world, layout)
    }

    #[test]
    fn islands_and_a_lake_are_placed_on_a_real_world() {
        let (mut world, layout) = surfaced_world(1);
        let mut structures = StructureMap::new();
        let mut rand = UnifiedRandom::new(2);
        let report = scatter(&mut world, &layout, &mut structures, &mut rand);

        assert!(report.islands > 0, "no floating islands were placed");
        assert!(report.lakes > 0, "no cloud lake was placed");

        let mut cloud_tiles = 0;
        let mut dirt_in_sky = 0;
        for x in (0..world.width()).step_by(5) {
            for y in 0..(layout.surface as usize) {
                let t = world.tile(x, y as i32);
                if t.is_active() && t.block == tiles::CLOUD {
                    cloud_tiles += 1;
                }
                if t.is_active() && t.block == tiles::DIRT {
                    dirt_in_sky += 1;
                }
            }
        }
        assert!(cloud_tiles > 0, "no cloud material found above the surface");
        assert!(
            dirt_in_sky > 0,
            "no dirt cap found above the surface — islands should have a walkable top"
        );
    }

    #[test]
    fn most_islands_get_a_house_with_a_chest() {
        let (mut world, layout) = surfaced_world(3);
        let mut structures = StructureMap::new();
        let mut rand = UnifiedRandom::new(4);
        let report = scatter(&mut world, &layout, &mut structures, &mut rand);

        assert!(report.houses > 0, "no floating island house was built");
        assert!(
            world.chests.iter().flatten().count() > 0,
            "no chest was placed in any floating island house"
        );

        let mut sunplate = 0;
        for x in (0..world.width()).step_by(3) {
            for y in 0..(layout.surface as usize) {
                if world.tile(x, y as i32).block == tiles::SUNPLATE {
                    sunplate += 1;
                }
            }
        }
        assert!(sunplate > 0, "no Sunplate house material found");
    }

    #[test]
    fn a_small_world_returns_zero_rather_than_looping_forever() {
        let mut world = World::empty(300, 200, "tiny");
        let mut rand = UnifiedRandom::new(1);
        let layout = Layout::plan(300, 200, &mut rand);
        let mut structures = StructureMap::new();
        let report = scatter(&mut world, &layout, &mut structures, &mut rand);
        assert_eq!(report, Report::default());
    }

    #[test]
    fn a_seed_places_the_same_islands_twice() {
        let (mut world_a, layout_a) = surfaced_world(5);
        let mut rand_a = UnifiedRandom::new(6);
        let report_a = scatter(
            &mut world_a,
            &layout_a,
            &mut StructureMap::new(),
            &mut rand_a,
        );

        let (mut world_b, layout_b) = surfaced_world(5);
        let mut rand_b = UnifiedRandom::new(6);
        let report_b = scatter(
            &mut world_b,
            &layout_b,
            &mut StructureMap::new(),
            &mut rand_b,
        );

        assert_eq!(report_a, report_b);
    }

    /// `cargo test -p terrustia --lib floating_islands::tests::measure_on_real_worlds --
    /// --ignored --nocapture`.
    ///
    /// Measured seeds 999/4242/12345 on a real 4200x1200 world: 3 islands (all 3 with a house)
    /// and 1 cloud lake, every seed — matching the hand-derived budget (`(4200*0.0008) as i32 = 3`
    /// islands, `sky_lakes = 1` since `4200 <= 6000`) exactly.
    #[test]
    #[ignore]
    fn measure_on_real_worlds() {
        for seed in [999u64, 4242, 12345] {
            let (_world, built) = super::super::build(4200, 1200, "measure", seed);
            eprintln!(
                "seed {seed}: islands={} houses={} lakes={}",
                built.floating_islands, built.floating_island_houses, built.cloud_lakes
            );
        }
    }
}
