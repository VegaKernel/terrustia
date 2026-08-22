//! Style 2 — the demon eyes.
//!
//! Ported from `AI_002_FloatingEye`. Structurally this is the bat's cousin: a weightless flier
//! that steers on both axes and bounces off terrain. Two things make it its own routine.
//!
//! The first is dawn. A demon eye does not fight the sunrise — above the surface, in daylight, and
//! with nobody standing in a graveyard, it turns away from whatever it was chasing and its despawn
//! timer is cut to ten ticks. That is why the sky empties at first light rather than filling with
//! corpses.
//!
//! The second is phasing. The hardmode eyes in this style give up on a target they cannot see
//! after five seconds and sink through the terrain toward it, going translucent and dropping tile
//! collision until they break through.

use terrustia_proto::npc_params::{
    DESPAWN_ENCOURAGED_TICKS, EYE_PHASE_DELAY, eye_enraged_steering, eye_flees_daylight,
    eye_phases_through_walls, eye_rises_in_water, eye_steering,
};

use super::{
    Conditions, World, bounce, can_see, face, rise_out_of_water, sight::solid_collision, steer,
    steer_axis_gated,
};
use crate::game::npc::Npc;
use crate::game::npc_ai::Target;

/// Whether daylight is currently driving this type away.
///
/// The graveyard exemption is the player's, not the NPC's: standing among enough tombstones keeps
/// the eyes out in the sun.
pub fn discouraged(npc: &Npc, conditions: Conditions, target_in_graveyard: bool) -> bool {
    eye_flees_daylight(npc.npc_type)
        && !target_in_graveyard
        && conditions.day
        && npc.position.1 <= conditions.surface_y
}

/// Turn away from whatever was being chased and leave.
///
/// The game writes the horizontal turn twice here, the first assignment dead; what survives is
/// simply "keep going the way you are already drifting".
fn flee(npc: &mut Npc) {
    npc.time_left = npc.time_left.min(DESPAWN_ENCOURAGED_TICKS);
    npc.direction_y = -1;
    npc.direction = if npc.velocity.0 > 0.0 { 1 } else { -1 };
}

/// Run the phasing eyes' sink-through-walls timer, returning whether it is currently phasing.
///
/// `ai[0]` counts ticks without a line of sight and `ai[1]` is the phase flag. Breaking back out
/// needs both a clear line *and* room to be solid — an eye halfway through a wall stays in it.
fn phase_timer<T: crate::game::npc::TileView>(
    npc: &mut Npc,
    world: &World<'_, T>,
    target: Option<Target>,
) -> bool {
    let visible = target.is_some_and(|t| can_see(world.tiles, npc, t));
    if visible {
        let embedded = solid_collision(
            world.tiles,
            npc.position,
            (npc.stats.width, npc.stats.height),
        );
        if npc.ai[1] > 0.0 && !embedded {
            npc.ai[1] = 0.0;
            npc.ai[0] = 0.0;
            npc.dirty = true;
        }
    } else if npc.ai[1] == 0.0 {
        npc.ai[0] += 1.0;
    }
    if npc.ai[0] >= EYE_PHASE_DELAY {
        npc.ai[1] = 1.0;
        npc.ai[0] = 0.0;
        npc.dirty = true;
    }
    npc.ai[1] != 0.0
}

/// Drive one floating eye for a tick.
pub fn update<T: crate::game::npc::TileView>(
    npc: &mut Npc,
    world: &World<'_, T>,
    target_in_graveyard: bool,
) {
    npc.no_gravity = true;
    // An eye sinking through a wall is not touching it, so it does not bounce off one either.
    if !npc.no_tile_collide {
        bounce(npc);
    }

    let mut target = world.target;
    if discouraged(npc, world.conditions, target_in_graveyard) {
        flee(npc);
        // Nothing to chase any more; the steering below runs on the fleeing direction.
        target = None;
    } else if let Some(t) = target {
        face(npc, t);
    }

    if eye_phases_through_walls(npc.npc_type) {
        let phasing = phase_timer(npc, world, target);
        npc.no_tile_collide = phasing;
        if let Some(t) = target {
            face(npc, t);
        }
        let steering = eye_steering(npc.npc_type);
        // A phasing eye only accelerates toward a target it has not already flown past, so each
        // arm carries a position test as well as a speed one.
        let (toward_left, toward_right, toward_up, toward_down) = match target {
            Some(t) => {
                let (bx, by) = (
                    t.center.0 - super::PLAYER_WIDTH as f32 / 2.0,
                    t.center.1 - super::PLAYER_HEIGHT as f32 / 2.0,
                );
                (
                    npc.position.0 > bx + super::PLAYER_WIDTH as f32,
                    npc.position.0 + npc.width() < bx,
                    npc.position.1 > by + super::PLAYER_HEIGHT as f32,
                    npc.position.1 + npc.height() < by,
                )
            }
            None => (false, false, false, false),
        };
        steer_axis_gated(
            &mut npc.velocity.0,
            npc.direction,
            steering.x,
            toward_left,
            toward_right,
        );
        steer_axis_gated(
            &mut npc.velocity.1,
            npc.direction_y,
            steering.y,
            toward_up,
            toward_down,
        );
    } else {
        // Some types find a second wind below half health.
        let enraged = eye_enraged_steering(npc.npc_type)
            .filter(|_| f64::from(npc.life) < f64::from(npc.life_max) * 0.5);
        steer(npc, enraged.unwrap_or_else(|| eye_steering(npc.npc_type)));
    }

    if eye_rises_in_water(npc.npc_type) && world.wet {
        rise_out_of_water(npc);
        if let Some(t) = target {
            face(npc, t);
        }
    }

    npc.sprite_direction = npc.direction;
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc::TILE;
    use std::collections::HashMap;
    use terrustia_proto::npc_params::eye_steering;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Sky(HashMap<(i32, i32), Tile>);

    impl crate::game::npc::TileView for Sky {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn eye(npc_type: u16) -> Npc {
        Npc::new(npc_type, (10_000.0, 10_000.0), 1).expect("a style 2 type")
    }

    fn player_at(x: f32, y: f32) -> Target {
        Target {
            slot: 0,
            center: (x, y),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    fn night<'a>(tiles: &'a Sky, target: Option<Target>) -> World<'a, Sky> {
        World {
            conditions: Conditions {
                day: false,
                surface_y: 100.0 * TILE,
                ..Conditions::default()
            },
            ..crate::game::ai::calm(tiles, target)
        }
    }

    #[test]
    fn a_demon_eye_chases_at_night() {
        let tiles = Sky::default();
        let mut e = eye(2);
        let (cx, cy) = e.center();
        let t = Some(player_at(cx + 5000.0, cy + 5000.0));
        for _ in 0..300 {
            update(&mut e, &night(&tiles, t), false);
        }
        let s = eye_steering(2);
        assert_eq!(e.velocity.0, s.x.max);
        assert_eq!(e.velocity.1, s.y.max);
    }

    #[test]
    fn dawn_above_the_surface_sends_a_demon_eye_away() {
        let tiles = Sky::default();
        let mut e = eye(2);
        e.position = (10_000.0, 50.0 * TILE);
        e.velocity = (2.0, 0.0);
        let (cx, cy) = e.center();
        let mut w = night(&tiles, Some(player_at(cx - 400.0, cy)));
        w.conditions.day = true;
        update(&mut e, &w, false);
        assert_eq!(e.direction_y, -1, "should climb away");
        assert_eq!(e.direction, 1, "and keep drifting the way it was going");
        assert!(
            e.time_left <= DESPAWN_ENCOURAGED_TICKS,
            "and be about to vanish, got {}",
            e.time_left
        );
    }

    #[test]
    fn daylight_underground_is_no_reason_to_leave() {
        let tiles = Sky::default();
        let mut e = eye(2);
        e.position = (10_000.0, 400.0 * TILE);
        let (cx, cy) = e.center();
        let mut w = night(&tiles, Some(player_at(cx - 400.0, cy)));
        w.conditions.day = true;
        update(&mut e, &w, false);
        assert_eq!(e.direction, -1, "should still be chasing");
        assert!(e.time_left > DESPAWN_ENCOURAGED_TICKS);
    }

    #[test]
    fn a_graveyard_keeps_the_eyes_out_in_the_sun() {
        let tiles = Sky::default();
        let mut e = eye(2);
        e.position = (10_000.0, 50.0 * TILE);
        let (cx, cy) = e.center();
        let mut w = night(&tiles, Some(player_at(cx - 400.0, cy)));
        w.conditions.day = true;
        update(&mut e, &w, true);
        assert_eq!(e.direction, -1, "should still be chasing");
    }

    #[test]
    fn every_coloured_eye_flees_the_sun() {
        for npc_type in [2, 190, 191, 192, 193, 194] {
            assert!(
                eye_flees_daylight(npc_type),
                "type {npc_type} should fear daylight"
            );
        }
    }

    #[test]
    fn an_eye_in_water_swims_up() {
        let tiles = Sky::default();
        let mut e = eye(2);
        e.velocity = (0.0, 2.0);
        let mut w = night(&tiles, None);
        w.wet = true;
        for _ in 0..10 {
            update(&mut e, &w, false);
        }
        assert!(e.velocity.1 < 0.0, "should rise, got {}", e.velocity.1);
    }

    #[test]
    fn an_eye_bounces_off_a_wall() {
        let tiles = Sky::default();
        let mut e = eye(2);
        e.direction = 1;
        e.velocity = (4.0, 0.0);
        e.old_velocity = (4.0, 0.0);
        e.collide_x = true;
        update(&mut e, &night(&tiles, None), false);
        assert!(e.velocity.0 < 0.0, "should rebound, got {}", e.velocity.0);
    }

    #[test]
    fn a_wandering_eye_sinks_through_walls_after_five_seconds_blind() {
        let mut tiles = Sky::default();
        for y in 0..2000 {
            for x in 630..640 {
                tiles.0.insert((x, y), Tile::block(1));
            }
        }
        let mut e = eye(170);
        e.position = (620.0 * TILE, 620.0 * TILE);
        let (cx, cy) = e.center();
        let t = Some(player_at(cx + 500.0, cy));
        for _ in 0..(EYE_PHASE_DELAY as i32) {
            update(&mut e, &night(&tiles, t), false);
        }
        assert!(e.no_tile_collide, "should have started phasing");
    }

    #[test]
    fn a_wandering_eye_stops_phasing_once_it_is_out_in_the_open_again() {
        let tiles = Sky::default();
        let mut e = eye(170);
        e.ai[1] = 1.0;
        e.no_tile_collide = true;
        let (cx, cy) = e.center();
        update(
            &mut e,
            &night(&tiles, Some(player_at(cx + 200.0, cy))),
            false,
        );
        assert!(
            !e.no_tile_collide,
            "clear air and a clear line ends the dive"
        );
    }

    #[test]
    fn a_wandering_eye_buried_in_rock_keeps_phasing_even_with_a_clear_line() {
        let mut tiles = Sky::default();
        for x in 615..630 {
            for y in 615..630 {
                tiles.0.insert((x, y), Tile::block(1));
            }
        }
        let mut e = eye(170);
        e.position = (620.0 * TILE, 620.0 * TILE);
        e.ai[1] = 1.0;
        e.no_tile_collide = true;
        let (cx, cy) = e.center();
        update(
            &mut e,
            &night(&tiles, Some(player_at(cx + 20.0, cy))),
            false,
        );
        assert!(
            e.no_tile_collide,
            "surfacing inside a wall would leave it stuck"
        );
    }
}
