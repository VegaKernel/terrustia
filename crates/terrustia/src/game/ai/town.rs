//! Style 7 — town residents and the critters that share their routine.
//!
//! Ported from `AI_007_TownEntities`. Bunnies, squirrels, mice, penguins, ducks, turtles and frogs
//! run the same 2,614-line routine as the Guide and the Merchant; what separates them is a handful
//! of table lookups, not a different code path.
//!
//! The shape is a two-state machine on `ai[0]`. In **state 0** it stands still and counts down; in
//! **state 1** it walks and counts down faster. Which way it walks, and what it does when the
//! ground runs out, is the whole of the routine.
//!
//! Three behaviours carry the character:
//!
//! * A resident is on a **leash**. Past twenty-five tiles from its home it will only turn further
//!   away by chance, past fifty it simply turns back, and past thirty-five its walk timer drains
//!   six times as fast when it is heading the wrong way. That is why townsfolk mill about their
//!   houses instead of wandering off.
//! * **Weather sends it indoors.** Rain, nightfall, an eclipse or a slime rain all set the same
//!   flag, and a resident then walks home and stops on its home tile. Critters ignore it.
//! * It **looks before it steps.** Every tick of walking, it probes the tile it is about to walk
//!   onto: a drop, deep water or lava turns it round, a one-, two- or three-tile step gets one of
//!   three jump impulses, and a closed door gets opened and then closed behind it.
//!
//! Not modelled here, and deliberately: shops, dialogue, the attack states, sitting and pet idle
//! animations. The user scoped this style to movement and housing.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    TOWN_FAR_FROM_HOME, TOWN_JUMP, TOWN_JUMP_LOW, TOWN_JUMP_TALL, TOWN_LEASH, TOWN_LEASH_HARD,
    TOWN_STEP_HEIGHT, town_breathes_underwater, town_hops_in_water, town_is_critter, town_is_slime,
    town_scurries, town_walk,
};
use terrustia_proto::tile::TileFlags;
use terrustia_proto::tile_solid::{solid, solid_top};

use super::{Conditions, World};
use crate::game::npc::{Npc, TILE, TileView};

/// Door and tall-gate tile types.
const DOOR: u16 = 10;
const TALL_GATE: u16 = 388;

/// How far ahead of itself a walker probes, in pixels.
const PROBE_REACH: f32 = 15.0;

/// What the routine wants done to a door it has reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorAction {
    None,
    /// Swing it open and walk through.
    Open {
        x: i32,
        y: i32,
        direction: i8,
    },
    /// Pull it shut again on the way past.
    Close {
        x: i32,
        y: i32,
    },
}

/// Where a resident lives and where the floor of that home is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Home {
    pub tile_x: i32,
    pub floor_y: i32,
}

/// Whether the weather or the hour is telling residents to go inside.
///
/// One flag covers all of it: nightfall, rain, an eclipse or a slime rain. Critters do not read it.
pub fn wants_shelter(conditions: Conditions, npc: &Npc) -> bool {
    !conditions.day
        || conditions.eclipse
        || (conditions.raining && npc.position.1 < conditions.surface_y)
}

fn blocking(tiles: &impl TileView, x: i32, y: i32) -> bool {
    let t = tiles.tile(x, y);
    t.is_active() && !t.flags.has(TileFlags::ACTUATED) && solid(t.block) && !solid_top(t.block)
}

/// Whether a tile is something to stand on: solid, or a platform.
fn footing(tiles: &impl TileView, x: i32, y: i32) -> bool {
    let t = tiles.tile(x, y);
    t.is_active()
        && !t.flags.has(TileFlags::ACTUATED)
        // Platforms are in both sets, and are footing even though they are not walls.
        && solid(t.block)
}

fn door_at(tiles: &impl TileView, x: i32, y: i32) -> Option<u16> {
    let t = tiles.tile(x, y);
    (t.is_active() && (t.block == DOOR || t.block == TALL_GATE)).then_some(t.block)
}

/// Find the floor beneath a home tile, which is what the routine actually walks to.
pub fn floor_under(tiles: &impl TileView, home_x: i32, home_y: i32, limit: i32) -> i32 {
    let mut y = home_y;
    while y < limit && !footing(tiles, home_x, y) {
        y += 1;
    }
    y
}

/// The tile a town NPC is standing on.
fn standing_on(npc: &Npc) -> (i32, i32) {
    (
        ((npc.position.0 + (npc.stats.width / 2) as f32) / TILE) as i32,
        ((npc.position.1 + npc.height() + 1.0) / TILE) as i32,
    )
}

/// Whether the ground ahead is somewhere to avoid stepping.
///
/// Returns true for a drop, for lava, and for water deep enough to drown in. A critter, or a
/// resident heading home from outside its leash, ignores all of it and walks on.
fn avoid_falling<T: TileView>(
    npc: &Npc,
    tiles: &T,
    probe: (i32, i32),
    home: Option<Home>,
    drowning: bool,
) -> bool {
    let (tile_x, _) = standing_on(npc);
    let near_home = home.is_some_and(|h| (tile_x - h.tile_x).abs() <= TOWN_FAR_FROM_HOME);
    let heading_home =
        home.is_some_and(|h| i32::from(npc.direction) == (h.tile_x - tile_x).signum());
    if town_is_critter(npc.npc_type) || (!near_home && heading_home) {
        return false;
    }

    let mut liquid_depth = 0;
    let mut lava = false;
    let mut landed = false;
    for step in -1..=4 {
        let tile = tiles.tile(probe.0, probe.1 + step);
        if tile.liquid > 0 {
            liquid_depth += 1;
            if tile.liquid_kind == terrustia_proto::tile::Liquid::Lava {
                lava = true;
                break;
            }
        }
        if footing(tiles, probe.0, probe.1 + step) {
            landed = true;
            break;
        }
    }
    if lava {
        return true;
    }
    // Water as deep as the NPC is tall would put its head under.
    if liquid_depth >= (npc.height() / TILE).ceil() as i32
        && !town_breathes_underwater(npc.npc_type)
    {
        return true;
    }
    if drowning {
        return false;
    }
    !landed
}

/// Walk up a step rather than jumping it.
///
/// A town NPC steps a little higher than a fighter does — twenty pixels rather than sixteen —
/// which is what lets one climb its own doorstep without hopping.
fn step_up(npc: &mut Npc, tiles: &impl TileView) -> bool {
    let ahead = npc.direction;
    let probe_x = ((npc.position.0 + (npc.stats.width / 2) as f32 + PROBE_REACH * f32::from(ahead))
        / TILE) as i32;
    let foot_y = ((npc.position.1 + npc.height() - 1.0) / TILE) as i32;
    if !blocking(tiles, probe_x, foot_y) {
        return false;
    }
    for up in 1..=2 {
        if blocking(tiles, probe_x, foot_y - up) {
            return false;
        }
    }
    let step_top = foot_y as f32 * TILE;
    let rise = npc.position.1 + npc.height() - step_top;
    if rise <= 0.0 || rise > TOWN_STEP_HEIGHT {
        return false;
    }
    npc.position.1 = step_top - npc.height();
    npc.dirty = true;
    true
}

/// Face whoever is nearest, which is what a critter does instead of holding a course.
fn face_nearest(npc: &mut Npc, world: &World<'_, impl TileView>) {
    if let Some(t) = world.target {
        if npc.position.0 < t.center.0 {
            npc.direction = 1;
        }
        if npc.position.0 > t.center.0 {
            npc.direction = -1;
        }
        npc.sprite_direction = npc.direction;
    }
}

/// Stand still, and decide whether it is time to move.
fn stand<T: TileView>(npc: &mut Npc, world: &World<'_, T>, home: Option<Home>, rng: &mut SmallRng) {
    let shelter = wants_shelter(world.conditions, npc) && !town_is_critter(npc.npc_type);
    let (tile_x, tile_y) = standing_on(npc);

    if shelter && let Some(h) = home {
        if tile_x == h.tile_x && tile_y == h.floor_y {
            // Home: settle to a stop.
            slow_to_a_halt(npc);
        } else {
            npc.direction = if tile_x > h.tile_x { -1 } else { 1 };
            npc.ai[0] = 1.0;
            npc.ai[1] = 200.0 + rng.random_range(0..200) as f32;
            npc.ai[2] = 0.0;
            npc.local_ai[3] = 0.0;
            npc.dirty = true;
        }
    } else {
        if town_scurries(npc.npc_type) {
            npc.velocity.0 *= 0.5;
        }
        slow_to_a_halt(npc);
        if npc.ai[1] > 0.0 {
            npc.ai[1] -= 1.0;
        }

        let probe = probe_tile(npc);
        let drowning = world.wet && !town_breathes_underwater(npc.npc_type);
        let blocked = avoid_falling(npc, world.tiles, probe, home, drowning);
        if drowning {
            start_walking(npc, rng);
        } else if npc.ai[1] <= 0.0 {
            if blocked {
                // Nowhere to go this way; turn and wait a little longer.
                npc.direction = -npc.direction;
                npc.ai[1] = 60.0 + rng.random_range(0..120) as f32;
                npc.dirty = true;
            } else {
                start_walking(npc, rng);
            }
        }
    }

    // The leash. Only applies while it is not being driven indoors.
    if !shelter && let Some(h) = home {
        let drift = tile_x - h.tile_x;
        if !(-TOWN_LEASH..=TOWN_LEASH).contains(&drift) {
            if npc.local_ai[3] == 0.0 {
                if drift < -TOWN_LEASH_HARD && npc.direction == -1 {
                    npc.direction = 1;
                    npc.dirty = true;
                } else if drift > TOWN_LEASH_HARD && npc.direction == 1 {
                    npc.direction = -1;
                    npc.dirty = true;
                }
            }
        } else if npc.local_ai[3] == 0.0 && rng.random_ratio(1, 80) {
            npc.local_ai[3] = 200.0;
            npc.direction = -npc.direction;
            npc.dirty = true;
        }
    }
}

fn slow_to_a_halt(npc: &mut Npc) {
    if npc.velocity.0 > 0.1 {
        npc.velocity.0 -= 0.1;
    } else if npc.velocity.0 < -0.1 {
        npc.velocity.0 += 0.1;
    } else {
        npc.velocity.0 = 0.0;
    }
}

fn start_walking(npc: &mut Npc, rng: &mut SmallRng) {
    npc.ai[0] = 1.0;
    npc.ai[1] = 200.0 + rng.random_range(0..300) as f32;
    npc.ai[2] = 0.0;
    if town_is_critter(npc.npc_type) {
        npc.ai[1] += rng.random_range(200..400) as f32;
    }
    npc.local_ai[3] = 0.0;
    npc.dirty = true;
}

/// The tile just ahead of the NPC's feet, which is what everything probes.
fn probe_tile(npc: &Npc) -> (i32, i32) {
    (
        ((npc.position.0 + (npc.stats.width / 2) as f32 + PROBE_REACH * f32::from(npc.direction))
            / TILE) as i32,
        ((npc.position.1 + npc.height() - 16.0) / TILE) as i32,
    )
}

/// Walk, and deal with whatever is in the way.
fn walk<T: TileView>(
    npc: &mut Npc,
    world: &World<'_, T>,
    home: Option<Home>,
    rng: &mut SmallRng,
) -> DoorAction {
    let (tile_x, tile_y) = standing_on(npc);
    let shelter = wants_shelter(world.conditions, npc) && !town_is_critter(npc.npc_type);

    // Arrived home in bad weather: stop.
    if shelter
        && let Some(h) = home
        && tile_x == h.tile_x
        && tile_y == h.floor_y
    {
        npc.ai[0] = 0.0;
        npc.ai[1] = 200.0 + rng.random_range(0..200) as f32;
        npc.local_ai[3] = 60.0;
        npc.dirty = true;
        return DoorAction::None;
    }

    let drowning = world.wet && !town_breathes_underwater(npc.npc_type);
    if !drowning {
        // Walking away from home, far out: the timer drains six times as fast.
        if let Some(h) = home
            && (tile_x < h.tile_x - TOWN_FAR_FROM_HOME || tile_x > h.tile_x + TOWN_FAR_FROM_HOME)
        {
            let away = (npc.position.0 < (h.tile_x * 16) as f32 && npc.direction == -1)
                || (npc.position.0 > (h.tile_x * 16) as f32 && npc.direction == 1);
            if away {
                npc.ai[1] -= 5.0;
            }
        }
        npc.ai[1] -= 1.0;
    }
    if npc.ai[1] <= 0.0 {
        npc.ai[0] = 0.0;
        npc.ai[1] = 300.0 + rng.random_range(0..300) as f32;
        npc.ai[2] = 0.0;
        if town_is_critter(npc.npc_type) {
            npc.ai[1] -= rng.random_range(0..100) as f32;
        } else {
            npc.ai[1] += rng.random_range(0..900) as f32;
        }
        npc.local_ai[3] = 60.0;
        npc.dirty = true;
    }

    // Accelerate, or shed speed if something else pushed it past its limit.
    let speed = town_walk(npc.npc_type, world.wet);
    if town_hops_in_water(npc.npc_type) && world.wet {
        // A frog kicks once and then coasts.
        if npc.velocity.0.abs() < 0.05 && npc.velocity.1.abs() < 0.05 {
            npc.velocity.0 += speed.max * 10.0 * f32::from(npc.direction);
        } else {
            npc.velocity.0 *= 0.9;
        }
    } else if npc.velocity.0 < -speed.max || npc.velocity.0 > speed.max {
        if npc.velocity.1 == 0.0 {
            npc.velocity.0 *= 0.8;
            npc.velocity.1 *= 0.8;
        }
    } else if npc.velocity.0 < speed.max && npc.direction == 1 {
        npc.velocity.0 = (npc.velocity.0 + speed.accel).min(speed.max);
    } else if npc.velocity.0 > -speed.max && npc.direction == -1 {
        npc.velocity.0 -= speed.accel;
    }

    if npc.velocity.1 == 0.0 {
        step_up(npc, world.tiles);
    }

    npc.sprite_direction = npc.direction;
    npc.dirty = true;

    if npc.velocity.1 != 0.0 {
        // Airborne: nothing to negotiate until it lands.
        return DoorAction::None;
    }

    let probe = probe_tile(npc);
    let blocked = avoid_falling(npc, world.tiles, probe, home, drowning);

    // A door is opened rather than climbed, and a resident in bad weather never dithers about it.
    let head = (probe.0, probe.1 - 2);
    if !town_is_critter(npc.npc_type)
        && door_at(world.tiles, head.0, head.1).is_some()
        && (shelter || rng.random_ratio(1, 10))
    {
        npc.ai[1] += 80.0;
        npc.ai[2] = f32::from(npc.direction);
        npc.dirty = true;
        return DoorAction::Open {
            x: head.0,
            y: head.1,
            direction: npc.direction,
        };
    }

    let heading = (npc.velocity.0 < 0.0 && npc.direction == -1)
        || (npc.velocity.0 > 0.0 && npc.direction == 1);
    if heading {
        // Three obstacle heights, three impulses. Anything taller is turned away from.
        if blocking(world.tiles, head.0, head.1) {
            if !blocking(world.tiles, head.0, head.1 - 1) {
                npc.velocity.1 = -TOWN_JUMP_TALL;
            } else {
                npc.direction = -npc.direction;
                npc.velocity.0 = 0.0;
            }
            npc.dirty = true;
        } else if blocking(world.tiles, probe.0, probe.1 - 1) {
            npc.velocity.1 = -TOWN_JUMP;
            npc.dirty = true;
        } else if npc.position.1 + npc.height() - (probe.1 * 16) as f32 > 20.0
            && blocking(world.tiles, probe.0, probe.1)
        {
            npc.velocity.1 = -TOWN_JUMP_LOW;
            npc.dirty = true;
        } else if blocked {
            npc.direction = -npc.direction;
            npc.velocity.0 = 0.0;
            npc.dirty = true;
        }
    }

    // Pull the door shut once well past it.
    if npc.ai[2] != 0.0 {
        let door = ((npc.position.0 + (npc.stats.width / 2) as f32) / TILE) as i32;
        if (door - probe.0).abs() > 2 {
            npc.ai[2] = 0.0;
            return DoorAction::Close {
                x: probe.0,
                y: probe.1 - 2,
            };
        }
    }

    DoorAction::None
}

/// Drive one town NPC or critter for a tick.
pub fn update<T: TileView>(
    npc: &mut Npc,
    world: &World<'_, T>,
    home: Option<Home>,
    rng: &mut SmallRng,
) -> DoorAction {
    npc.direction_y = -1;
    if npc.direction == 0 {
        npc.direction = 1;
    }

    // A critter always faces whoever is nearest; a resident holds its course.
    if town_is_critter(npc.npc_type) {
        face_nearest(npc, world);
    }

    if npc.local_ai[3] > 0.0 {
        npc.local_ai[3] -= 1.0;
    }

    // A town slime bobs rather than sinks.
    if town_is_slime(npc.npc_type) && world.wet && npc.velocity.1 > 0.0 {
        npc.velocity.1 *= 0.5;
    }

    if npc.ai[0] == 1.0 {
        walk(npc, world, home, rng)
    } else {
        // Every other state is a rest, an attack or an animation; the ones this port does not
        // model fall back to standing, which is what the game does between them anyway.
        npc.ai[0] = 0.0;
        stand(npc, world, home, rng);
        DoorAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Ground(HashMap<(i32, i32), Tile>);

    impl TileView for Ground {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    /// Flat ground from tile y = 100 down, across the given span.
    fn flat(from: i32, to: i32) -> Ground {
        let mut g = Ground::default();
        for x in from..to {
            for y in 100..110 {
                g.0.insert((x, y), Tile::block(1));
            }
        }
        g
    }

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(3)
    }

    fn stand_on(npc_type: u16, tile_x: i32) -> Npc {
        let mut n = Npc::new(npc_type, (0.0, 0.0), 1).expect("a style 7 type");
        n.position = (tile_x as f32 * TILE, 100.0 * TILE - n.height());
        n
    }

    fn day<'a>(tiles: &'a Ground) -> World<'a, Ground> {
        World {
            tiles,
            target: None,
            wet: false,
            target_wet: false,
            conditions: Conditions {
                day: true,
                surface_y: 90.0 * TILE,
                ..Conditions::default()
            },
            was_hurt: false,
            target_velocity: (0.0, 0.0),
            census: &[],
            parent: None,
            parent_state: 0.0,
            parent_health: 1.0,
            crowding: (0.0, 0.0),
            avoid: &[],
            target_taken: false,
            hooks: None,
            kin_moving: false,
        }
    }

    #[test]
    fn a_bunny_is_a_critter_and_the_guide_is_not() {
        assert!(town_is_critter(46), "bunny");
        assert!(town_is_critter(299), "squirrel");
        assert!(town_is_critter(616), "turtle");
        assert!(!town_is_critter(22), "the guide is a resident");
    }

    #[test]
    fn a_critter_faces_whoever_is_nearest() {
        let tiles = flat(0, 400);
        let mut bunny = stand_on(46, 200);
        bunny.direction = -1;
        let mut w = day(&tiles);
        w.target = Some(Target {
            slot: 0,
            center: (300.0 * TILE, 100.0 * TILE),
            velocity: (0.0, 0.0),
            alive: true,
        });
        update(&mut bunny, &w, None, &mut rng());
        assert_eq!(bunny.direction, 1, "should turn toward the player");
    }

    #[test]
    fn a_resident_walks_home_when_the_weather_turns() {
        let tiles = flat(0, 400);
        let mut guide = stand_on(22, 250);
        let home = Some(Home {
            tile_x: 200,
            floor_y: 100,
        });
        let mut w = day(&tiles);
        w.conditions.day = false;
        let mut r = rng();
        update(&mut guide, &w, home, &mut r);
        assert_eq!(guide.ai[0], 1.0, "should set off");
        assert_eq!(guide.direction, -1, "and head toward home");
    }

    #[test]
    fn a_resident_at_home_in_bad_weather_stops() {
        let tiles = flat(0, 400);
        let mut guide = stand_on(22, 200);
        guide.velocity = (0.05, 0.0);
        let home = Some(Home {
            tile_x: standing_on(&guide).0,
            floor_y: standing_on(&guide).1,
        });
        let mut w = day(&tiles);
        w.conditions.day = false;
        update(&mut guide, &w, home, &mut rng());
        assert_eq!(guide.velocity.0, 0.0, "should have settled");
        assert_eq!(guide.ai[0], 0.0, "and stayed put");
    }

    /// Given the same house and the same weather, a resident heads for it and a critter does not.
    #[test]
    fn a_critter_ignores_the_weather_a_resident_obeys_it() {
        let tiles = flat(0, 400);
        let home = Some(Home {
            tile_x: 200,
            floor_y: 100,
        });
        let mut w = day(&tiles);
        w.conditions.day = false;

        let mut guide = stand_on(22, 250);
        guide.direction = 1;
        update(&mut guide, &w, home, &mut rng());
        assert_eq!(guide.direction, -1, "a resident turns for home");

        let mut bunny = stand_on(46, 250);
        bunny.direction = 1;
        update(&mut bunny, &w, home, &mut rng());
        assert_eq!(bunny.direction, 1, "a bunny keeps hopping");
    }

    #[test]
    fn a_walker_accelerates_to_its_own_speed() {
        let tiles = flat(0, 400);
        for (npc_type, want) in [(22u16, 1.0f32), (299, 1.5), (300, 2.0)] {
            let mut n = stand_on(npc_type, 200);
            n.ai[0] = 1.0;
            n.ai[1] = 5000.0;
            n.direction = 1;
            for _ in 0..200 {
                update(&mut n, &day(&tiles), None, &mut rng());
                n.velocity.1 = 0.0;
            }
            assert!(
                (n.velocity.0 - want).abs() < 0.01,
                "type {npc_type} should walk at {want}, got {}",
                n.velocity.0
            );
        }
    }

    #[test]
    fn a_turtle_is_slow_on_land_and_quick_in_water() {
        assert_eq!(town_walk(616, false).max, 0.5);
        assert_eq!(town_walk(616, true).max, 2.0);
        assert_eq!(town_walk(625, true).max, 2.5, "a sea turtle more so");
    }

    #[test]
    fn a_resident_turns_back_at_the_edge_of_its_leash() {
        let tiles = flat(0, 400);
        let mut guide = stand_on(22, 200 + TOWN_LEASH_HARD + 5);
        guide.direction = 1;
        guide.ai[1] = 1000.0;
        let home = Some(Home {
            tile_x: 200,
            floor_y: 100,
        });
        update(&mut guide, &day(&tiles), home, &mut rng());
        assert_eq!(guide.direction, -1, "should turn for home");
    }

    /// Set an NPC walking right, and put the ground's edge exactly where it is about to probe.
    fn walking_toward_the_edge(npc_type: u16) -> (Npc, Ground) {
        let mut n = stand_on(npc_type, 208);
        n.ai[0] = 1.0;
        n.ai[1] = 5000.0;
        n.direction = 1;
        n.velocity.0 = 1.0;
        let edge = probe_tile(&n).0;
        (n, flat(0, edge))
    }

    #[test]
    fn a_resident_stops_at_a_cliff_and_a_critter_does_not() {
        let home = Some(Home {
            tile_x: 200,
            floor_y: 100,
        });
        let (mut guide, tiles) = walking_toward_the_edge(22);
        update(&mut guide, &day(&tiles), home, &mut rng());
        assert_eq!(guide.direction, -1, "a resident looks before it steps");

        let (mut bunny, tiles) = walking_toward_the_edge(46);
        update(&mut bunny, &day(&tiles), None, &mut rng());
        assert_eq!(bunny.direction, 1, "a bunny does not");
    }

    #[test]
    fn a_resident_opens_a_door_rather_than_climbing_it() {
        let mut guide = stand_on(22, 208);
        guide.ai[0] = 1.0;
        guide.ai[1] = 5000.0;
        guide.direction = 1;
        guide.velocity.0 = 1.0;
        let probe = probe_tile(&guide);
        let mut tiles = flat(0, 400);
        // A door filling the three tiles above the floor just ahead.
        for y in (probe.1 - 2)..=probe.1 {
            tiles.0.insert((probe.0, y), Tile::framed(DOOR, 0, 0));
        }
        let mut w = day(&tiles);
        // Bad weather removes the one-in-ten dithering, so the door is tried every tick.
        w.conditions.day = false;
        let action = update(
            &mut guide,
            &w,
            Some(Home {
                tile_x: 300,
                floor_y: 100,
            }),
            &mut rng(),
        );
        assert!(
            matches!(action, DoorAction::Open { .. }),
            "expected a door to be opened, got {action:?}"
        );
    }

    #[test]
    fn a_critter_walks_into_a_door_rather_than_opening_it() {
        let mut bunny = stand_on(46, 208);
        bunny.ai[0] = 1.0;
        bunny.ai[1] = 5000.0;
        bunny.direction = 1;
        bunny.velocity.0 = 1.0;
        let probe = probe_tile(&bunny);
        let mut tiles = flat(0, 400);
        for y in (probe.1 - 2)..=probe.1 {
            tiles.0.insert((probe.0, y), Tile::framed(DOOR, 0, 0));
        }
        let action = update(&mut bunny, &day(&tiles), None, &mut rng());
        assert_eq!(action, DoorAction::None, "a bunny has no hands");
    }

    #[test]
    fn a_walker_jumps_a_low_wall_and_turns_from_a_tall_one() {
        let mut low = stand_on(22, 208);
        low.ai[0] = 1.0;
        low.ai[1] = 5000.0;
        low.direction = 1;
        low.velocity.0 = 1.0;
        let probe = probe_tile(&low);
        let mut tiles = flat(0, 400);
        // Two tiles of step, which is higher than it can walk up but well within a hop.
        tiles.0.insert((probe.0, probe.1), Tile::block(1));
        tiles.0.insert((probe.0, probe.1 - 1), Tile::block(1));
        update(&mut low, &day(&tiles), None, &mut rng());
        assert!(
            low.velocity.1 < 0.0,
            "should hop it, got {}",
            low.velocity.1
        );

        // A wall taller than anything it can clear.
        let mut tall = stand_on(22, 208);
        tall.ai[0] = 1.0;
        tall.ai[1] = 5000.0;
        tall.direction = 1;
        tall.velocity.0 = 1.0;
        let mut wall = flat(0, 400);
        for y in (probe.1 - 9)..=probe.1 {
            wall.0.insert((probe.0, y), Tile::block(1));
        }
        update(&mut tall, &day(&wall), None, &mut rng());
        assert_eq!(tall.direction, -1, "should give up and turn round");
    }

    #[test]
    fn a_frog_kicks_off_in_water_rather_than_swimming() {
        let tiles = flat(0, 400);
        let mut frog = stand_on(361, 200);
        frog.ai[0] = 1.0;
        frog.ai[1] = 5000.0;
        frog.direction = 1;
        let mut w = day(&tiles);
        w.wet = true;
        update(&mut frog, &w, None, &mut rng());
        assert!(
            frog.velocity.0 > 5.0,
            "a frog should shove off hard, got {}",
            frog.velocity.0
        );
    }

    #[test]
    fn the_walk_timer_drains_faster_when_heading_away_from_home() {
        let tiles = flat(0, 800);
        let home = Some(Home {
            tile_x: 200,
            floor_y: 100,
        });
        let mut away = stand_on(22, 200 + TOWN_FAR_FROM_HOME + 10);
        away.ai[0] = 1.0;
        away.ai[1] = 1000.0;
        away.direction = 1;

        let mut back = stand_on(22, 200 + TOWN_FAR_FROM_HOME + 10);
        back.ai[0] = 1.0;
        back.ai[1] = 1000.0;
        back.direction = -1;

        update(&mut away, &day(&tiles), home, &mut rng());
        update(&mut back, &day(&tiles), home, &mut rng());
        assert_eq!(away.ai[1], 994.0, "six ticks off for walking away");
        assert_eq!(back.ai[1], 999.0, "one for walking back");
    }
}
