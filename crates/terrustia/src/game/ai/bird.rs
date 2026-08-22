//! Style 24 — the birds and their fellow perching critters.
//!
//! Ported from the `aiStyle == 24` block. A bird has two states: perched on the ground with
//! gravity on, and airborne with gravity off. It takes off the moment anything disturbs it — a
//! player coming within a hundred pixels, or being hit — and then flies off bouncing from anything
//! it clips.

use crate::game::npc::Npc;
use crate::game::npc_ai::Target;

/// Ordinary flight speed, and the faster one a few types use.
pub const FLIGHT_SPEED: f32 = 3.0;
pub const FLIGHT_SPEED_FAST: f32 = 4.0;

/// Acceleration toward the chosen direction.
pub const ACCEL: f32 = 0.1;

/// How close a player has to come before a perched bird takes off.
pub const STARTLE_RANGE: f32 = 100.0;

/// Flight speed by type; the gem critters are quicker.
pub fn flight_speed(npc_type: u16) -> f32 {
    match npc_type {
        671..=675 => FLIGHT_SPEED_FAST,
        _ => FLIGHT_SPEED,
    }
}

/// `ai[0]`: 0 while perched, 1 once flying.
fn flying(npc: &Npc) -> bool {
    npc.ai[0] != 0.0
}

/// Reflect off a surface, keeping at least a minimum speed so it does not stall against a wall.
fn bounce(npc: &mut Npc, speed: f32) {
    if npc.collide_x {
        npc.direction = -npc.direction;
        npc.velocity.0 = npc.old_velocity.0 * -0.5;
        let floor = speed - 1.0;
        if npc.direction == -1 && npc.velocity.0 > 0.0 && npc.velocity.0 < floor {
            npc.velocity.0 = floor;
        }
        if npc.direction == 1 && npc.velocity.0 < 0.0 && npc.velocity.0 > -floor {
            npc.velocity.0 = -floor;
        }
    }
    if npc.collide_y {
        npc.velocity.1 = npc.old_velocity.1 * -0.5;
        if npc.velocity.1 > 0.0 && npc.velocity.1 < 1.0 {
            npc.velocity.1 = 1.0;
        }
        if npc.velocity.1 < 0.0 && npc.velocity.1 > -1.0 {
            npc.velocity.1 = -1.0;
        }
    }
}

/// Drive one bird. Returns whether it wants gravity this tick.
pub fn update(npc: &mut Npc, target: Option<Target>, was_hurt: bool) -> bool {
    if !flying(npc) {
        // Perched. Anything at all sets it off.
        let startled = was_hurt
            || npc.velocity.0 != 0.0
            || npc.velocity.1 < 0.0
            || npc.velocity.1 > 0.3
            || target.is_some_and(|t| {
                let (cx, cy) = npc.center();
                (t.center.0 - cx).abs() < STARTLE_RANGE + npc.width()
                    && (t.center.1 - cy).abs() < STARTLE_RANGE + npc.height()
            });
        if startled {
            npc.ai[0] = 1.0;
            npc.direction = -npc.direction;
            npc.dirty = true;
        }
        // Perched birds keep gravity.
        return true;
    }

    let speed = flight_speed(npc.npc_type);
    bounce(npc, speed);

    // Accelerate along the way it is facing, with the small extra nudges the game applies while
    // the velocity is still on the wrong side of zero.
    if npc.direction == -1 && npc.velocity.0 > -speed {
        npc.velocity.0 -= ACCEL;
        if npc.velocity.0 > speed {
            npc.velocity.0 -= ACCEL;
        } else if npc.velocity.0 > 0.0 {
            npc.velocity.0 -= ACCEL / 2.0;
        }
        npc.velocity.0 = npc.velocity.0.max(-speed);
    } else if npc.direction == 1 && npc.velocity.0 < speed {
        npc.velocity.0 += ACCEL;
        if npc.velocity.0 < -speed {
            npc.velocity.0 += ACCEL;
        } else if npc.velocity.0 < 0.0 {
            npc.velocity.0 += ACCEL / 2.0;
        }
        npc.velocity.0 = npc.velocity.0.min(speed);
    }

    npc.sprite_direction = npc.direction;
    npc.dirty = true;
    // Airborne birds ignore gravity.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bird() -> Npc {
        Npc::new(74, (1000.0, 1000.0), 1).expect("bird")
    }

    fn player_at(x: f32, y: f32) -> Target {
        Target {
            slot: 0,
            center: (x, y),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    #[test]
    fn a_perched_bird_stays_perched_when_left_alone() {
        let mut b = bird();
        let wants_gravity = update(&mut b, Some(player_at(9000.0, 1000.0)), false);
        assert!(wants_gravity, "a perched bird is subject to gravity");
        assert_eq!(b.ai[0], 0.0, "should still be on the ground");
    }

    #[test]
    fn a_bird_takes_off_when_a_player_comes_close() {
        let mut b = bird();
        let (cx, cy) = b.center();
        update(&mut b, Some(player_at(cx + 40.0, cy)), false);
        assert_eq!(b.ai[0], 1.0, "should have taken off");
    }

    #[test]
    fn a_bird_takes_off_when_it_is_hit() {
        let mut b = bird();
        update(&mut b, None, true);
        assert_eq!(b.ai[0], 1.0);
    }

    #[test]
    fn an_airborne_bird_ignores_gravity_and_reaches_its_speed() {
        let mut b = bird();
        b.ai[0] = 1.0;
        b.direction = 1;
        let mut wants_gravity = true;
        for _ in 0..200 {
            wants_gravity = update(&mut b, None, false);
        }
        assert!(!wants_gravity, "flying birds are weightless");
        assert!(
            (b.velocity.0 - FLIGHT_SPEED).abs() < 0.001,
            "got {}",
            b.velocity.0
        );
    }

    #[test]
    fn the_gem_critters_fly_faster() {
        assert_eq!(flight_speed(672), FLIGHT_SPEED_FAST);
        assert_eq!(flight_speed(74), FLIGHT_SPEED);
    }

    #[test]
    fn a_bird_bounces_off_a_wall_and_turns_round() {
        let mut b = bird();
        b.ai[0] = 1.0;
        b.direction = 1;
        b.velocity = (3.0, 0.0);
        b.old_velocity = (3.0, 0.0);
        b.collide_x = true;
        update(&mut b, None, false);
        assert_eq!(b.direction, -1, "should turn around");
        assert!(b.velocity.0 < 0.0, "and head back, got {}", b.velocity.0);
    }

    #[test]
    fn a_bird_bouncing_off_the_ceiling_keeps_moving() {
        let mut b = bird();
        b.ai[0] = 1.0;
        b.velocity = (0.0, -2.0);
        b.old_velocity = (0.0, -2.0);
        b.collide_y = true;
        update(&mut b, None, false);
        assert!(
            b.velocity.1 >= 1.0,
            "should be pushed back down, got {}",
            b.velocity.1
        );
    }
}

/// Style 68 — ducks, seagulls and grebes: the birds that swim.
///
/// A waterfowl has the same two states a bird does, but the resting one is *on water* rather than
/// on the ground, and holding a floating bird at the surface is most of the routine. It paddles
/// along the waterline, turning at the bank, and rides the surface exactly six pixels down however
/// the water moves.
///
/// Startling one puts it up with a hard upward kick, and it flies for five seconds before looking
/// for somewhere to settle. Where it settles decides what it becomes: back onto water and it is a
/// swimming duck again, onto dry land and it turns into the walking form and rests there for
/// several seconds.
pub fn waterfowl<T: crate::game::npc::TileView>(
    npc: &mut Npc,
    world: &super::World<'_, T>,
    rng: &mut rand::rngs::SmallRng,
) -> Landing {
    use rand::Rng;
    use terrustia_proto::npc_params::{
        DUCK_CLIMB, DUCK_CLIMB_CAP, DUCK_FLIGHT_TICKS, DUCK_FLOAT_ABOVE, DUCK_FLY_ACCEL,
        DUCK_FLY_SPEED, DUCK_LANDED_REST, DUCK_LANDS_AS, DUCK_LOOK_DOWN, DUCK_PADDLE,
        DUCK_SINK_CAP, DUCK_STARTLE, DUCK_SURFACE_CLIMB, DUCK_SURFACE_CLIMB_CAP, DUCK_TAKEOFF,
        DUCK_TOO_CLOSE,
    };
    use terrustia_proto::tile_solid::solid;

    let mut landing = Landing::default();
    npc.dirty = true;
    npc.no_gravity = true;
    let tile = crate::game::npc::TILE;

    if npc.ai[0] == 0.0 {
        // On the water.
        npc.no_gravity = false;
        let was = npc.direction;
        if let Some(target) = world.target {
            super::face(npc, target);
        }
        if was != 0 {
            // Facing is for the sprite; a swimming duck keeps paddling the way it was going.
            npc.direction = was;
        }

        if world.wet {
            npc.velocity.0 =
                (npc.velocity.0 * 19.0 + DUCK_PADDLE * f32::from(npc.direction)) / 20.0;
            // The bank: solid tile ahead, or simply the end of the water.
            let (cx, cy) = npc.center();
            let ahead = ((cx + (npc.width() / 2.0 + 8.0) * f32::from(npc.direction)) / tile) as i32;
            let level = (cy / tile) as i32;
            let head = (npc.position.1 / tile) as i32;
            let feet = ((npc.position.1 + npc.height()) / tile) as i32;
            let solid_at = |x: i32, y: i32| {
                let t = world.tiles.tile(x, y);
                t.is_active() && solid(t.block)
            };
            if solid_at(ahead, level)
                || solid_at(ahead, head)
                || solid_at(ahead, feet)
                || world.tiles.tile(ahead, feet).liquid == 0
            {
                npc.direction = -npc.direction;
            }
            npc.sprite_direction = npc.direction;
            if npc.velocity.1 > 0.0 {
                npc.velocity.1 *= 0.5;
            }
            npc.no_gravity = true;

            // Ride the surface. The waterline is wherever the topmost partly-filled tile is.
            let column = (cx / tile) as i32;
            let row = (cy / tile) as i32;
            let mut surface = npc.position.1 + npc.height();
            for (offset, above) in [(-1, 0.0), (0, 1.0), (1, 2.0)] {
                let t = world.tiles.tile(column, row + offset);
                if t.liquid > 0 {
                    surface = (row as f32 + above) * tile - f32::from(t.liquid) / 16.0;
                    break;
                }
            }
            surface -= DUCK_FLOAT_ABOVE;
            if cy > surface {
                npc.velocity.1 = (npc.velocity.1 - DUCK_SURFACE_CLIMB).max(DUCK_SURFACE_CLIMB_CAP);
                if cy + npc.velocity.1 < surface {
                    npc.velocity.1 = surface - cy;
                }
            } else {
                npc.velocity.1 = surface - cy;
            }
        } else {
            // No water under it any more: up it goes.
            npc.ai[0] = 1.0;
            npc.direction = -npc.direction;
            return landing;
        }

        // Startled by company, or by being hit.
        let startled = npc.life < npc.life_max
            || world.target.is_some_and(|t| {
                let (cx, cy) = npc.center();
                (t.center.0 - cx).abs() < npc.width() / 2.0 + DUCK_STARTLE
                    && (t.center.1 - cy).abs() < npc.height() / 2.0 + DUCK_STARTLE
            });
        if startled {
            npc.ai[0] = 1.0;
            npc.velocity.1 += DUCK_TAKEOFF;
            npc.direction = -npc.direction;
        }
        return landing;
    }

    // Flying.
    npc.ai[1] += 1.0;
    if npc.ai[1] >= DUCK_FLIGHT_TICKS {
        // Coming down. It settles the moment it touches anything.
        if npc.velocity.1 == 0.0 || npc.collide_y || world.wet {
            npc.velocity = (0.0, 0.0);
            npc.ai[0] = 0.0;
            npc.ai[1] = 0.0;
            if !world.wet && DUCK_LANDS_AS.contains(&npc.npc_type) {
                landing.becomes = Some(npc.npc_type - 1);
                landing.rests_for = rng.random_range(DUCK_LANDED_REST.0..DUCK_LANDED_REST.1) as i32;
            }
        } else {
            npc.velocity.0 *= 0.98;
            npc.velocity.1 = (npc.velocity.1 + 0.1).min(2.0);
        }
        return landing;
    }

    if npc.collide_x {
        npc.direction = -npc.direction;
        npc.velocity.0 = npc.old_velocity.0 * -0.5;
        if npc.direction == -1 && (0.0..2.0).contains(&npc.velocity.0) {
            npc.velocity.0 = 2.0;
        }
        if npc.direction == 1 && (-2.0..0.0).contains(&npc.velocity.0) {
            npc.velocity.0 = -2.0;
        }
    }
    if npc.collide_y {
        npc.velocity.1 = npc.old_velocity.1 * -0.5;
        if (0.0..1.0).contains(&npc.velocity.1) {
            npc.velocity.1 = 1.0;
        }
        if (-1.0..0.0).contains(&npc.velocity.1) {
            npc.velocity.1 = -1.0;
        }
    }

    if npc.direction == -1 && npc.velocity.0 > -DUCK_FLY_SPEED {
        npc.velocity.0 -= DUCK_FLY_ACCEL;
        if npc.velocity.0 > DUCK_FLY_SPEED {
            npc.velocity.0 -= DUCK_FLY_ACCEL;
        } else if npc.velocity.0 > 0.0 {
            npc.velocity.0 -= DUCK_FLY_ACCEL / 2.0;
        }
        npc.velocity.0 = npc.velocity.0.max(-DUCK_FLY_SPEED);
    } else if npc.direction == 1 && npc.velocity.0 < DUCK_FLY_SPEED {
        npc.velocity.0 += DUCK_FLY_ACCEL;
        if npc.velocity.0 < -DUCK_FLY_SPEED {
            npc.velocity.0 += DUCK_FLY_ACCEL;
        } else if npc.velocity.0 < 0.0 {
            npc.velocity.0 += DUCK_FLY_ACCEL / 2.0;
        }
        npc.velocity.0 = npc.velocity.0.min(DUCK_FLY_SPEED);
    }

    // It flies level with whatever is ahead: nothing below means descend, something close means
    // climb, and something *very* close means climb harder.
    let ahead = ((npc.position.0 + npc.width() / 2.0) / tile) as i32 + i32::from(npc.direction);
    let feet = ((npc.position.1 + npc.height()) / tile) as i32;
    let mut open = true;
    let mut too_close = false;
    for depth in 0..DUCK_LOOK_DOWN {
        let t = world.tiles.tile(ahead, feet + depth);
        if (t.is_active() && solid(t.block)) || t.liquid > 0 {
            if depth < DUCK_TOO_CLOSE {
                too_close = true;
            }
            open = false;
            break;
        }
    }
    if open {
        npc.velocity.1 += DUCK_CLIMB;
    } else {
        npc.velocity.1 -= DUCK_CLIMB;
    }
    if too_close {
        npc.velocity.1 -= 0.2;
    }
    npc.velocity.1 = npc.velocity.1.clamp(DUCK_CLIMB_CAP, DUCK_SINK_CAP);
    landing
}

/// What a waterfowl decided when it came down.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Landing {
    /// The walking form it turned into, if it settled on dry land.
    pub becomes: Option<u16>,
    /// How long it will sit there before doing anything else.
    pub rests_for: i32,
}

#[cfg(test)]
mod waterfowl_tests {
    use super::*;
    use crate::game::npc::{TILE, TileView};
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Pond(HashMap<(i32, i32), Tile>);

    impl TileView for Pond {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    /// A pond: stone from `bed` down, water filling the tiles above it between the banks.
    fn pond(surface: i32, bed: i32) -> Pond {
        let mut tiles = HashMap::new();
        for x in -60..60 {
            for y in bed..bed + 5 {
                tiles.insert((x, y), Tile::block(1));
            }
        }
        for x in -40..40 {
            for y in surface..bed {
                let mut t = Tile::AIR;
                t.liquid = 255;
                tiles.insert((x, y), t);
            }
        }
        // Banks either side, so a swimming duck has somewhere to turn round at.
        for x in [-41, 40] {
            for y in surface - 6..bed {
                tiles.insert((x, y), Tile::block(1));
            }
        }
        Pond(tiles)
    }

    fn world<'a>(tiles: &'a Pond, target: Option<(f32, f32)>) -> super::super::World<'a, Pond> {
        crate::game::ai::calm(
            tiles,
            target.map(|center| Target {
                slot: 0,
                center,
                velocity: (0.0, 0.0),
                alive: true,
            }),
        )
    }

    const DUCK: u16 = 363;

    fn duck(tile_x: i32, tile_y: i32) -> Npc {
        Npc::new(DUCK, (tile_x as f32 * TILE, tile_y as f32 * TILE), 1).expect("duck")
    }

    /// A duck on water paddles along it rather than sinking or flying.
    #[test]
    fn a_duck_paddles_along_the_surface() {
        let tiles = pond(20, 30);
        let mut w = world(&tiles, Some((10_000.0, 0.0)));
        w.wet = true;
        let mut d = duck(0, 20);
        let mut rng = rand::rngs::SmallRng::seed_from_u64(68);

        let start = d.position.0;
        for _ in 0..200 {
            waterfowl(&mut d, &w, &mut rng);
            crate::game::npc::step_physics(&mut d, &tiles);
        }
        assert_eq!(d.ai[0], 0.0, "it should still be on the water");
        assert!(
            (d.position.0 - start).abs() > 20.0,
            "and have paddled somewhere, moved {}",
            d.position.0 - start
        );
        // Held at the surface rather than sinking to the bed or drifting into the sky.
        let depth = d.center().1 / TILE;
        assert!(
            (18.0..24.0).contains(&depth),
            "it should be riding the waterline, got tile {depth}"
        );
    }

    /// Walking up to one puts it in the air.
    #[test]
    fn a_duck_startles_and_takes_off() {
        let tiles = pond(20, 30);
        let mut d = duck(0, 20);
        let (cx, cy) = d.center();
        let mut w = world(&tiles, Some((cx + 40.0, cy)));
        w.wet = true;
        let mut rng = rand::rngs::SmallRng::seed_from_u64(1);

        waterfowl(&mut d, &w, &mut rng);
        assert_eq!(d.ai[0], 1.0, "it should be up");
        assert!(d.velocity.1 < 0.0, "and going up, not down");
    }

    /// Five seconds later it comes down, and coming down on land turns it into the walking form.
    #[test]
    fn a_duck_that_lands_on_dry_ground_becomes_the_walking_form() {
        let tiles = pond(20, 30);
        let w = world(&tiles, Some((10_000.0, 0.0)));
        let mut rng = rand::rngs::SmallRng::seed_from_u64(3);
        let mut d = duck(50, 28);
        d.ai[0] = 1.0;
        d.ai[1] = terrustia_proto::npc_params::DUCK_FLIGHT_TICKS - 1.0;

        let mut became = None;
        for _ in 0..600 {
            let landing = waterfowl(&mut d, &w, &mut rng);
            crate::game::npc::step_physics(&mut d, &tiles);
            if landing.becomes.is_some() {
                became = landing.becomes;
                break;
            }
        }
        assert_eq!(became, Some(DUCK - 1), "it should have become the walker");
        assert_eq!(d.ai[0], 0.0, "and be settled");
    }
}
