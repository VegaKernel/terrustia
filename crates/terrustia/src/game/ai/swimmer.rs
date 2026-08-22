//! Styles 18 and 44 — the two ways of moving through a medium rather than over ground.
//!
//! A **jellyfish** (18) has one trick: it does not steer toward you, it *stops*. While a wet target
//! is in view it lets its speed bleed away tick by tick, and the instant that speed drops below a
//! threshold it fires itself in a straight line at whatever it was watching. The whole pulsing
//! rhythm of a jellyfish falls out of those two numbers. Out of the water it has nothing at all and
//! simply falls.
//!
//! A **flying fish** (44) hovers: it accelerates sideways only when far enough away to bother, and
//! holds a fixed height above its target, dropping to that height when it gets close. Losing sight
//! of the target does not stop it — it keeps hunting for ninety more ticks and only then wanders.

use terrustia_proto::npc_params::{HOVER_ATTENTION, hover, jelly};

use super::{World, can_see, face, rise_out_of_water};
use crate::game::npc::{Npc, TILE, TileView};

/// Gravity on a jellyfish out of water, and its terminal speed.
const BEACHED_GRAVITY: f32 = 0.2;
const BEACHED_TERMINAL: f32 = 10.0;

/// How gently a drifting jellyfish pushes itself along, and the speed it starts shedding at.
const DRIFT_PUSH: f32 = 0.02;
const DRIFT_LIMIT: f32 = 1.0;
/// ...and how fast it bobs up and down.
const BOB: f32 = 0.01;

/// Liquid this deep in the tile above counts as being under the surface.
const SUBMERGED: u8 = 128;

/// Drive one jellyfish for a tick.
pub fn jellyfish<T: TileView>(npc: &mut Npc, world: &World<'_, T>) {
    // The expert-mode electrified phase is not modelled: it needs expert difficulty, which this
    // server does not run, and it only gates damage rather than movement.
    if npc.direction == 0
        && let Some(t) = world.target
    {
        face(npc, t);
    }

    if !world.wet {
        // Beached. It tumbles and falls, and resets its bob so it resumes upward on re-entry.
        npc.rotation += npc.velocity.0 * 0.1;
        if npc.velocity.1 == 0.0 {
            npc.velocity.0 *= 0.98;
            if npc.velocity.0 > -0.01 && npc.velocity.0 < 0.01 {
                npc.velocity.0 = 0.0;
            }
        }
        npc.velocity.1 = (npc.velocity.1 + BEACHED_GRAVITY).min(BEACHED_TERMINAL);
        npc.ai[0] = 1.0;
        npc.dirty = true;
        return;
    }

    if npc.collide_x {
        npc.velocity.0 = -npc.velocity.0;
        npc.direction = -npc.direction;
    }
    if npc.collide_y {
        if npc.velocity.1 > 0.0 {
            npc.velocity.1 = -npc.velocity.1.abs();
            npc.direction_y = -1;
            npc.ai[0] = -1.0;
        } else if npc.velocity.1 < 0.0 {
            npc.velocity.1 = npc.velocity.1.abs();
            npc.direction_y = 1;
            npc.ai[0] = 1.0;
        }
    }

    let hunting = !npc.stats.friendly
        && world
            .target
            .is_some_and(|t| world.target_wet && t.alive && can_see(world.tiles, npc, t));

    if hunting {
        let params = jelly(npc.npc_type);
        npc.rotation = npc.velocity.1.atan2(npc.velocity.0) + 1.57;
        npc.velocity.0 *= params.drag;
        npc.velocity.1 *= params.drag;
        let slow = npc.velocity.0 > -params.trigger
            && npc.velocity.0 < params.trigger
            && npc.velocity.1 > -params.trigger
            && npc.velocity.1 < params.trigger;
        if slow && let Some(t) = world.target {
            face(npc, t);
            let (cx, cy) = npc.center();
            let (dx, dy) = (t.center.0 - cx, t.center.1 - cy);
            let k = params.lunge / (dx * dx + dy * dy).sqrt();
            npc.velocity = (dx * k, dy * k);
            npc.dirty = true;
        }
        npc.dirty = true;
        return;
    }

    // Drifting. A slow horizontal push, a slow vertical bob, and a check that it is not about to
    // drift out of the water it lives in.
    npc.velocity.0 += f32::from(npc.direction) * DRIFT_PUSH;
    npc.rotation = npc.velocity.0 * 0.4;
    if npc.velocity.0 < -DRIFT_LIMIT || npc.velocity.0 > DRIFT_LIMIT {
        npc.velocity.0 *= 0.95;
    }
    if npc.ai[0] == -1.0 {
        npc.velocity.1 -= BOB;
        if npc.velocity.1 < -DRIFT_LIMIT {
            npc.ai[0] = 1.0;
        }
    } else {
        npc.velocity.1 += BOB;
        if npc.velocity.1 > DRIFT_LIMIT {
            npc.ai[0] = -1.0;
        }
    }

    // Under the surface with ground close beneath: turn back up rather than settling on it.
    let (cx, cy) = npc.center();
    let (tile_x, tile_y) = ((cx / TILE) as i32, (cy / TILE) as i32);
    if world.tiles.tile(tile_x, tile_y - 1).liquid > SUBMERGED {
        if world.tiles.tile(tile_x, tile_y + 1).is_active()
            || world.tiles.tile(tile_x, tile_y + 2).is_active()
        {
            npc.ai[0] = -1.0;
        }
    } else {
        // At or above the surface, it always heads back down.
        npc.ai[0] = 1.0;
    }
    if npc.velocity.1 > 1.2 || npc.velocity.1 < -1.2 {
        npc.velocity.1 *= 0.99;
    }
    npc.dirty = true;
}

/// Drive one flying fish for a tick.
///
/// `crowding` is the nudge away from others of its own kind, which the caller works out because it
/// is the only thing that can see the rest of the shoal.
pub fn flying_fish<T: TileView>(npc: &mut Npc, world: &World<'_, T>, crowding: (f32, f32)) {
    npc.no_gravity = true;

    if npc.collide_x {
        npc.direction = if npc.old_velocity.0 > 0.0 { -1 } else { 1 };
        npc.velocity.0 = f32::from(npc.direction);
    }
    if npc.collide_y {
        npc.direction_y = if npc.old_velocity.1 > 0.0 { -1 } else { 1 };
        npc.velocity.1 = f32::from(npc.direction_y);
    }

    // Attention: a target it can see refreshes the timer; anything else runs it down.
    let worth_hunting = world
        .target
        .is_some_and(|t| t.alive && !world.target_wet && can_see(world.tiles, npc, t));
    if worth_hunting {
        npc.ai[0] = HOVER_ATTENTION;
        if let Some(t) = world.target {
            face(npc, t);
        }
    } else if npc.ai[0] > 0.0 {
        npc.ai[0] -= 1.0;
        if let Some(t) = world.target {
            face(npc, t);
        }
    }

    let mut params = hover(npc.npc_type);
    if params.avoids_its_own_kind {
        npc.velocity.0 += crowding.0;
        npc.velocity.1 += crowding.1;
        npc.rotation = npc.velocity.0 * 0.1;
    }

    let (_, cy) = npc.center();
    let mut across = 0.0;
    let mut wanted_y = npc.position.1;
    if let Some(t) = world.target {
        across = (npc.position.0 + npc.width() / 2.0 - t.center.0).abs();
        wanted_y = t.center.1 - super::PLAYER_HEIGHT as f32 / 2.0 - npc.height() / 2.0;
    }

    if npc.ai[0] <= 0.0 {
        // Given up: drift, slower and lazier, toward nothing in particular.
        params.max_x *= 0.8;
        params.accel_x *= 0.7;
        wanted_y = cy + f32::from(npc.direction_y) * 1000.0;
        npc.direction = if npc.velocity.0 < 0.0 { -1 } else { 1 };
    }

    if across > params.deadband {
        if npc.direction == -1 && npc.velocity.0 > -params.max_x {
            npc.velocity.0 -= params.accel_x;
            if npc.velocity.0 > params.max_x {
                npc.velocity.0 -= params.accel_x;
            } else if npc.velocity.0 > 0.0 {
                npc.velocity.0 -= params.accel_x / 2.0;
            }
            npc.velocity.0 = npc.velocity.0.max(-params.max_x);
        } else if npc.direction == 1 && npc.velocity.0 < params.max_x {
            npc.velocity.0 += params.accel_x;
            if npc.velocity.0 < -params.max_x {
                npc.velocity.0 += params.accel_x;
            } else if npc.velocity.0 < 0.0 {
                npc.velocity.0 += params.accel_x / 2.0;
            }
            npc.velocity.0 = npc.velocity.0.min(params.max_x);
        }
    }

    // Far away it climbs; close in it drops to its target's own height.
    if across > params.climb_at {
        wanted_y -= params.climb_at / 2.0;
    }
    if npc.position.1 < wanted_y {
        npc.velocity.1 += params.accel_y;
        if npc.velocity.1 < 0.0 {
            npc.velocity.1 += params.accel_y;
        }
    } else {
        npc.velocity.1 -= params.accel_y;
        if npc.velocity.1 > 0.0 {
            npc.velocity.1 -= params.accel_y;
        }
    }
    npc.velocity.1 = npc.velocity.1.clamp(-params.max_y, params.max_y);

    if world.wet {
        rise_out_of_water(npc);
    }
    npc.sprite_direction = npc.direction;
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::{Liquid, Tile};

    #[derive(Default)]
    struct Sea(HashMap<(i32, i32), Tile>);

    impl TileView for Sea {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn ocean() -> Sea {
        let mut s = Sea::default();
        for x in 400..700 {
            for y in 400..700 {
                s.0.insert((x, y), Tile::AIR.with_liquid(Liquid::Water, 255));
            }
        }
        s
    }

    fn at(npc_type: u16, tile_x: i32, tile_y: i32) -> Npc {
        Npc::new(npc_type, (tile_x as f32 * TILE, tile_y as f32 * TILE), 1).expect("a swimmer type")
    }

    fn world<'a>(tiles: &'a Sea, target: Option<Target>) -> World<'a, Sea> {
        // These are swimmers, so the default world for them is underwater.
        World {
            wet: true,
            target_wet: true,
            ..crate::game::ai::calm(tiles, target)
        }
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
    fn a_beached_jellyfish_falls() {
        let tiles = ocean();
        let mut j = at(63, 100, 100);
        let mut w = world(&tiles, None);
        w.wet = false;
        jellyfish(&mut j, &w);
        assert_eq!(j.velocity.1, BEACHED_GRAVITY);
    }

    #[test]
    fn a_jellyfish_winds_down_and_then_lunges() {
        let tiles = ocean();
        let mut j = at(63, 500, 500);
        let (cx, cy) = j.center();
        let t = Some(player_at(cx + 200.0, cy));
        j.velocity = (5.0, 0.0);
        let mut launched = false;
        for _ in 0..400 {
            let before = (j.velocity.0.powi(2) + j.velocity.1.powi(2)).sqrt();
            jellyfish(&mut j, &world(&tiles, t));
            let after = (j.velocity.0.powi(2) + j.velocity.1.powi(2)).sqrt();
            if after > before + 1.0 {
                launched = true;
                break;
            }
        }
        assert!(launched, "should have fired itself at the player");
        let speed = (j.velocity.0.powi(2) + j.velocity.1.powi(2)).sqrt();
        assert!(
            (speed - jelly(63).lunge).abs() < 1e-3,
            "at its lunge speed, got {speed}"
        );
        assert!(j.velocity.0 > 0.0, "and toward them");
    }

    #[test]
    fn a_squid_lets_go_much_sooner_than_a_jellyfish() {
        assert!(jelly(221).trigger > jelly(63).trigger);
        assert!(jelly(221).drag > jelly(63).drag, "and coasts longer");
    }

    #[test]
    fn a_jellyfish_with_nobody_wet_to_chase_just_drifts() {
        let tiles = ocean();
        let mut j = at(63, 500, 500);
        j.direction = 1;
        let (cx, cy) = j.center();
        let mut w = world(&tiles, Some(player_at(cx + 200.0, cy)));
        // On dry land, so not worth chasing.
        w.target_wet = false;
        for _ in 0..50 {
            jellyfish(&mut j, &w);
        }
        let speed = (j.velocity.0.powi(2) + j.velocity.1.powi(2)).sqrt();
        assert!(speed < 2.0, "a drift, not a charge: {speed}");
        assert!(j.velocity.0 > 0.0, "and it should be going somewhere");
    }

    #[test]
    fn a_drifting_jellyfish_bobs_up_and_down() {
        let tiles = ocean();
        let mut j = at(63, 500, 500);
        let mut ups = 0;
        let mut downs = 0;
        for _ in 0..600 {
            jellyfish(&mut j, &world(&tiles, None));
            if j.velocity.1 < 0.0 {
                ups += 1;
            }
            if j.velocity.1 > 0.0 {
                downs += 1;
            }
        }
        assert!(ups > 50 && downs > 50, "should bob: {ups} up, {downs} down");
    }

    #[test]
    fn a_flying_fish_holds_station_above_its_target() {
        let tiles = Sea::default();
        let mut f = at(224, 500, 500);
        let (cx, cy) = f.center();
        let t = Some(player_at(cx + 600.0, cy));
        let mut w = world(&tiles, t);
        w.wet = false;
        w.target_wet = false;
        let mut highest = cy;
        for _ in 0..600 {
            flying_fish(&mut f, &w, (0.0, 0.0));
            f.position.0 += f.velocity.0;
            f.position.1 += f.velocity.1;
            highest = highest.min(f.center().1);
        }
        assert!(f.no_gravity, "it flies");
        assert!(f.center().0 > cx, "and closes the gap");
        assert!(highest < cy, "climbing above its target on the way");
    }

    #[test]
    fn a_flying_fish_keeps_hunting_for_ninety_ticks_after_losing_you() {
        let tiles = Sea::default();
        let mut f = at(224, 500, 500);
        let (cx, cy) = f.center();
        let mut w = world(&tiles, Some(player_at(cx + 200.0, cy)));
        w.wet = false;
        w.target_wet = false;
        flying_fish(&mut f, &w, (0.0, 0.0));
        assert_eq!(f.ai[0], HOVER_ATTENTION);

        // Target gone: the timer runs down rather than snapping off.
        w.target = None;
        for _ in 0..10 {
            flying_fish(&mut f, &w, (0.0, 0.0));
        }
        assert_eq!(f.ai[0], HOVER_ATTENTION - 10.0);
    }

    #[test]
    fn a_flying_antlion_is_quicker_than_a_flying_fish_and_minds_its_neighbours() {
        assert!(hover(581).max_x > hover(224).max_x);
        assert!(hover(581).avoids_its_own_kind);
        assert!(!hover(224).avoids_its_own_kind);
    }

    #[test]
    fn a_flying_fish_bounces_off_what_it_clips() {
        let tiles = Sea::default();
        let mut f = at(224, 500, 500);
        f.old_velocity = (3.0, 0.0);
        f.collide_x = true;
        let mut w = world(&tiles, None);
        w.wet = false;
        flying_fish(&mut f, &w, (0.0, 0.0));
        assert_eq!(f.direction, -1, "should turn away from the wall");
    }
}
