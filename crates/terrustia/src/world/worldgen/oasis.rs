//! Desert oases: an elliptical basin of water sunk into the sand, with a sand-covered rim.
//!
//! Transcribed from the `Oasis` generation pass (`WorldGen.cs:16339-16364`) and `PlaceOasis`
//! (`WorldGen.cs:10728-10959`), which does essentially all of the real work — the driving pass is
//! just the retry loop that finds a candidate site. Vanilla tracks how many oases exist and where
//! in session-global `GenVars.numOasis`/`oasisPosition`/`oasisWidth`; this generator has no such
//! global; the placed set lives as a local `Vec` for the duration of one `scatter` call instead,
//! which is the same state, just not reachable from anywhere else — nothing else in vanilla reads
//! it either.
//!
//! **Two real deviations were needed to get this placing anything on a real generated world**,
//! neither in `try_place`'s own siting logic:
//!
//! - *Pipeline order.* Vanilla's `Oasis` pass runs before essentially every decorative pass —
//!   `Statues`, `PotsGraveyardsAndBoulderPiles`, and even cacti themselves
//!   (`CactusPalmTreesAndCoral`, the very end of vanilla's own pass list). `build()` in this
//!   generator now calls [`scatter`] right after liquid settling, before trees, vines, cacti, or
//!   any of the small object-placement passes get a chance to leave a non-sand tile on the desert
//!   surface — see `mod.rs`'s own comment at the call site for the full history of where this
//!   landed before that.
//! - *The window scan accepting `HARDENED_SAND`*, not just `SAND` — see that check's own comment
//!   in [`try_place`] for why.

use super::layout::Layout;
use super::rand::UnifiedRandom;
use super::tiles;
use crate::world::World;
use terrustia_proto::{Liquid, Tile, TileFlags};

/// `WorldGen.beachDistance` — a fixed margin off either edge of the world every siting pass that
/// uses it keeps clear of, oases included.
const BEACH_DISTANCE: i32 = 380;

/// `GenVars.oasisHeight` — fixed, not derived from anything.
const OASIS_HEIGHT: i32 = 20;

/// The minimum gap `PlaceOasis` enforces between two oases (`WorldGen.cs:10746`, `num = 350`).
const MIN_SPACING: f64 = 350.0;

/// The `Oasis` pass: scatter oases across the desert surface.
///
/// Returns how many were placed.
pub fn scatter(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) -> usize {
    // The search bands below are `margin..width-margin` (margin = 680) and `100..surface`. Real,
    // full-size worlds always clear both by a wide margin, but the small synthetic worlds several
    // unrelated tests build (to keep persistence/gameplay tests fast) do not — the same shape of
    // guard `traps.rs::scatter` already needed for its own search bands. Skip rather than let
    // `next_range` panic on an inverted or empty range.
    let margin = BEACH_DISTANCE + 300;
    if layout.width <= margin * 2 || layout.surface <= 100 {
        return 0;
    }

    // `Main.maxTilesX / 2100`, plus 0 or 1 more — vanilla's own count, unconditionally (the
    // `notTheBees`/`dontStarveWorldGen`/secret-seed gates this project doesn't model are skipped,
    // same standing rule as every other secret-seed branch this session has left out).
    let wanted = layout.width / 2100 + rand.next_max(2);
    let mut placed: Vec<(i32, i32, i32)> = Vec::new();

    for _ in 0..wanted {
        let mut tries = layout.width * 2;
        while tries > 0 {
            tries -= 1;
            let x = rand.next_range(margin, layout.width - margin);
            let y = rand.next_range(100, layout.surface);
            if try_place(world, layout, &mut placed, x, y, rand) {
                break;
            }
        }
    }
    placed.len()
}

/// `PlaceOasis`, transcribed.
fn try_place(
    world: &mut World,
    layout: &Layout,
    placed: &mut Vec<(i32, i32, i32)>,
    x: i32,
    mut y: i32,
    rand: &mut UnifiedRandom,
) -> bool {
    if world.tile(x, y).is_active() || world.tile(x, y).wall != 0 {
        return false;
    }
    while !world.tile(x, y).is_active() && world.tile(x, y).wall == 0 && y <= layout.surface {
        y += 1;
    }
    if y as f64 > layout.surface as f64 - 10.0 {
        return false;
    }
    if world.tile(x, y).block != tiles::SAND {
        return false;
    }
    for &(ox, oy, _) in placed.iter() {
        let dist = (((x - ox).pow(2) + (y - oy).pow(2)) as f64).sqrt();
        if dist < MIN_SPACING {
            return false;
        }
    }

    let half_width = rand.next_range(45, 61);
    let outer_half = half_width + 50;
    let bank_fluff = 4;
    for k in (x - outer_half)..=(x + outer_half) {
        for l in (y - OASIS_HEIGHT)..=(y + OASIS_HEIGHT + bank_fluff) {
            let tile = world.tile(k, l);
            if tile.is_active() {
                if terrustia_proto::tile_solid::solid(tile.block) {
                    if matches!(tile.block, 151 | tiles::SANDSTONE)
                        && (k - x).abs() < half_width
                        && (l - y).abs() < OASIS_HEIGHT / 2
                    {
                        return false;
                    }
                    // Vanilla's own check here is a plain `!= SAND`. `HARDENED_SAND` is added
                    // because of *this* generator's own desert material curve
                    // (`terrain.rs::material`, `Surface::Desert` arm): sand only down to depth 6,
                    // hardened sand from 6 to 39 — well inside this window's own vertical reach
                    // (`y - OASIS_HEIGHT` to `y + OASIS_HEIGHT + bank_fluff`, depth up to 24).
                    // A literal `!= SAND` port rejects almost every real candidate not because
                    // anything is actually wrong with the site, but because the ordinary ground a
                    // few tiles under any desert surface in this generator isn't literally
                    // `tiles::SAND` — measured at 100% of candidates that reached this loop being
                    // rejected here, on a real generated world, before this fix. Sandstone (and
                    // sandstone right under the basin, above) is left rejected: unlike hardened
                    // sand, it is not the ordinary material at this depth here, and a real
                    // sandstone shelf under the site is exactly the kind of terrain vanilla's
                    // check means to steer around.
                    if tile.block != tiles::SAND && tile.block != tiles::HARDENED_SAND {
                        return false;
                    }
                }
            } else if (tile.liquid > 0 || tile.wall > 0)
                && (k - x).abs() < half_width
                && (l - y).abs() < OASIS_HEIGHT / 2
            {
                return false;
            }
        }
        if k > x - half_width / 2 && k < x - half_width / 2 {
            // Vanilla's own condition (`k > X - num2/2 && k < X - num2/2`) can never be true —
            // transcribed as written rather than "corrected", since silently changing a dead
            // vanilla branch into a live one would be inventing behaviour, not porting it.
            if world.tile(k, y - 6).is_active() {
                return false;
            }
            if !world.tile(k, y + 1).is_active() {
                return false;
            }
        }
    }

    // Settle: nudge down until both banks (at `+-half_width`) have solid, wall-free ground.
    let start_y = y;
    while !world.tile(x - half_width, y + 5).is_active()
        || world.tile(x - half_width, y + 5).wall != 0
        || !world.tile(x + half_width, y + 5).is_active()
        || world.tile(x + half_width, y + 5).wall != 0
    {
        y += 1;
        if y - start_y > 20 {
            break;
        }
    }

    carve_basin(world, x, y, half_width, rand);
    carve_banks(world, x, y, half_width, rand);

    placed.push((x, y, half_width));
    true
}

/// The elliptical dig-and-fill: water in the middle, sand rim tapering out to either side.
fn carve_basin(world: &mut World, x: i32, y: i32, half_width: i32, rand: &mut UnifiedRandom) {
    let quarter_width = half_width / 2;
    let x0 = (x - half_width * 3).max(0);
    let x1 = (x + half_width * 3).min(world.width());
    let y0 = (y - OASIS_HEIGHT * 4).max(0);
    let y1 = (y + OASIS_HEIGHT * 3).min(world.height());

    for m in x0..x1 {
        for n in y0..y1 {
            let dx = f64::from((m - x).abs()) * 0.7;
            let dy = f64::from((n - y).abs()) * 1.35;
            let dist = (dx * dx + dy * dy).sqrt();
            let radius = f64::from(quarter_width) * (0.53 + rand.next_double() * 0.04);
            let mut edge_taper = f64::from((m - x).abs()) / f64::from((x1 - x).max(1));
            edge_taper = 1.0 - edge_taper;
            edge_taper *= 2.3;
            edge_taper *= edge_taper;
            edge_taper *= edge_taper;

            if dist < radius {
                let mut t = world.tile(m, n);
                if n == y + 1 {
                    t.liquid = 127;
                } else if n > y + 1 {
                    t.liquid = 255;
                }
                if n > y {
                    t.liquid_kind = Liquid::Water;
                }
                t.flags.set(TileFlags::ACTIVE, false);
                world.set_tile(m, n, t);
            } else if n < y
                && dx < radius + f64::from((n - y).abs() * 3) * edge_taper
                && world.tile(m, n).block == tiles::SAND
            {
                let mut t = world.tile(m, n);
                t.flags.set(TileFlags::ACTIVE, false);
                world.set_tile(m, n, t);
            } else if n >= y
                && dx < radius + f64::from((n - y).abs()) * edge_taper
                && world.tile(m, n).wall == 0
            {
                let t = world.tile(m, n);
                if t.is_active()
                    && terrustia_proto::tile_solid::solid(t.block)
                    && !terrustia_proto::tile_solid::solid_top(t.block)
                {
                    let mut flat = t;
                    flat.slope = 0;
                    flat.flags
                        .set(terrustia_proto::TileFlags::HALF_BRICK, false);
                    world.set_tile(m, n, flat);
                    continue;
                }
                let mut sand = Tile::block(tiles::SAND);
                sand.slope = 0;
                world.set_tile(m, n, sand);
            }
        }
    }
}

/// The outer sand banks tapering into the surrounding terrain, so the oasis has a shore rather
/// than stopping at a hard edge.
fn carve_banks(world: &mut World, x: i32, y: i32, half_width: i32, rand: &mut UnifiedRandom) {
    let reach = 50;
    let x0 = x - half_width * 2;
    let x1 = x + half_width * 2;
    let y1 = y + OASIS_HEIGHT * 2;

    for m in x0..x1 {
        for n in (y..=y1).rev() {
            let dx = f64::from((m - x).abs()) * 0.7;
            let dy = f64::from((n - y).abs()) * 1.35;
            let dist = (dx * dx + dy * dy).sqrt();
            let inner_radius = f64::from(half_width / 2) * 0.57;
            if dist <= inner_radius {
                continue;
            }
            if world.tile(m, n).is_active() || world.tile(m, n).wall != 0 {
                continue;
            }
            // Vanilla tracks a `flag`/`found_sand`-shaped "did either side see sand" bool through
            // both scans below, then overwrites it to `true` unconditionally right before the one
            // place it's read (`WorldGen.cs:10926`, `flag = true;`) — genuinely dead code there
            // too, not a transcription slip. Not carried here; only `left`/`right` gate anything.
            let mut right = -1;
            let mut kr = m;
            while kr <= m + reach
                && world.tile(kr, n + 1).is_active()
                && terrustia_proto::tile_solid::solid(world.tile(kr, n + 1).block)
                && world.tile(kr, n).wall == 0
            {
                let t = world.tile(kr, n);
                if t.is_active() && terrustia_proto::tile_solid::solid(t.block) {
                    right = kr;
                    break;
                }
                kr += 1;
            }
            let mut left = -1;
            let mut kl = m;
            while kl >= m - reach
                && world.tile(kl, n + 1).is_active()
                && terrustia_proto::tile_solid::solid(world.tile(kl, n + 1).block)
                && world.tile(kl, n).wall == 0
            {
                let t = world.tile(kl, n);
                if t.is_active() && terrustia_proto::tile_solid::solid(t.block) {
                    left = kl;
                    break;
                }
                kl -= 1;
            }
            if left > -1 && right > -1 {
                let mut dip = 0;
                for fill_x in (left + 1)..right {
                    if right - left > 5 && rand.next_max(5) == 0 {
                        dip = rand.next_range(5, 10);
                    }
                    let mut sand = world.tile(fill_x, n);
                    sand.block = tiles::SAND;
                    sand.flags.set(TileFlags::ACTIVE, true);
                    world.set_tile(fill_x, n, sand);
                    if dip > 0 {
                        dip -= 1;
                        let mut sand_above = world.tile(fill_x, n - 1);
                        sand_above.block = tiles::SAND;
                        sand_above.flags.set(TileFlags::ACTIVE, true);
                        world.set_tile(fill_x, n - 1, sand_above);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    /// A wide, flat sand surface, well above `layout.surface` — `layout.surface` is the
    /// underground-threshold reference line `PlaceOasis` refuses to site within 10 tiles of, not
    /// the ground itself, so a realistic desert's actual surface sits well above (a smaller `y`
    /// than) it, same as vanilla's own `Main.worldSurface`.
    fn desert(width: i32, height: i32, ground_y: i32, surface_threshold: i32) -> (World, Layout) {
        let mut world = World::empty(width, height, "oasis");
        for x in 0..width {
            for y in ground_y..(ground_y + 60) {
                world.set_tile(x, y, Tile::block(tiles::SAND));
            }
        }
        let mut rand = UnifiedRandom::new(1);
        let mut layout = Layout::plan(width, height, &mut rand);
        layout.surface = surface_threshold;
        (world, layout)
    }

    #[test]
    fn an_oasis_holds_real_water_on_a_flat_sand_surface() {
        let (mut world, layout) = desert(3000, 500, 120, 300);
        let mut rand = UnifiedRandom::new(11);
        let made = scatter(&mut world, &layout, &mut rand);
        assert!(
            made > 0,
            "a wide flat sand surface should take at least one oasis"
        );

        let mut wet = 0;
        for x in 0..world.width() {
            // Water sits below wherever the site's ground line settled — near the 120 the desert
            // was carved at, not `layout.surface` (300), which is only a siting *threshold*.
            for y in 100..200 {
                if world.tile(x, y).liquid > 0 {
                    wet += 1;
                }
            }
        }
        assert!(
            wet > 20,
            "a placed oasis should hold real water, got {wet} tiles"
        );
    }

    /// A desert with a *realistic* depth profile: loose sand for the first 6 tiles, hardened sand
    /// below that — matching this generator's own `terrain.rs::material` curve for
    /// `Surface::Desert` (`depth < 6` sand, `depth < 40` hardened sand), rather than the flat
    /// all-`SAND` block [`desert`] builds. The oasis window scans down to `y + OASIS_HEIGHT +
    /// bank_fluff` (depth 24) below the candidate surface, which on this profile is deep inside
    /// the hardened-sand band — reproducing the actual defect this module shipped with, where a
    /// literal `!= SAND` port in the window scan rejected essentially every real candidate.
    fn layered_desert(
        width: i32,
        height: i32,
        ground_y: i32,
        surface_threshold: i32,
    ) -> (World, Layout) {
        let mut world = World::empty(width, height, "oasis-layered");
        for x in 0..width {
            for y in ground_y..(ground_y + 6) {
                world.set_tile(x, y, Tile::block(tiles::SAND));
            }
            for y in (ground_y + 6)..(ground_y + 60) {
                world.set_tile(x, y, Tile::block(tiles::HARDENED_SAND));
            }
        }
        let mut rand = UnifiedRandom::new(1);
        let mut layout = Layout::plan(width, height, &mut rand);
        layout.surface = surface_threshold;
        (world, layout)
    }

    /// Fails on the pre-fix code (restoring a bare `tile.block != tiles::SAND` in the window scan
    /// makes `scatter` return `0`, same as on every real generated world before this fix).
    #[test]
    fn an_oasis_still_forms_over_the_hardened_sand_a_real_desert_actually_has() {
        let (mut world, layout) = layered_desert(3000, 500, 120, 300);
        let mut rand = UnifiedRandom::new(11);
        let made = scatter(&mut world, &layout, &mut rand);
        assert!(
            made > 0,
            "a realistic desert, sand over hardened sand, should still take an oasis — the \
             hardened sand a few tiles under any real desert surface here must not be treated as \
             disqualifying terrain the way actual out-of-place material (stone, sandstone) is"
        );
    }

    #[test]
    fn no_oasis_forms_where_the_surface_is_not_sand() {
        // Stone instead of sand — PlaceOasis's own `type != 53` check should refuse every site.
        let mut world = World::empty(2000, 500, "not-desert");
        for x in 0..2000 {
            for y in 200..260 {
                world.set_tile(x, y, Tile::block(tiles::STONE));
            }
        }
        let mut rand = UnifiedRandom::new(1);
        let mut layout = Layout::plan(2000, 500, &mut rand);
        layout.surface = 200;
        let mut rand2 = UnifiedRandom::new(5);
        assert_eq!(scatter(&mut world, &layout, &mut rand2), 0);
    }
}
