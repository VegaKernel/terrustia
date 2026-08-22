//! Style 10 — the cursed skulls.
//!
//! A skull does not fly at you steadily; it stalks. Its speed is tiered by distance and every tier
//! is *slower* than the one outside it, so it rushes in from across the room and then dawdles once
//! it is close. Inside 250 pixels it stops steering entirely and jitters, and all the while a timer
//! is running. At six hundred ticks that timer flips it into a charge — eight times the
//! acceleration, four times the speed — for fifty ticks, and then resets.
//!
//! The giant one interrupts all of that to spit a skull of its own; the water bolt mimic plays dead
//! until something hits it.

use terrustia_proto::npc_params::{
    GIANT_SKULL_RANGE, GIANT_SKULL_RECOVER, GIANT_SKULL_RELEASE, GIANT_SKULL_SHOT_DAMAGE,
    GIANT_SKULL_SHOT_SPEED, GIANT_SKULL_SHOT_TYPE, GIANT_SKULL_WINDUP, SKULL_CHARGE_ACCEL,
    SKULL_CHARGE_AT, SKULL_CHARGE_OVER, SKULL_CHARGE_SPEED, SKULL_JITTER_PERIOD, SKULL_JITTER_PUSH,
    SKULL_JITTER_RANGE, SKULL_JITTER_RATE, SKULL_JITTER_TURN, skull_approach,
};

use super::{Shot, World};
use crate::game::npc::{Npc, TileView};

/// A spat skull lives five seconds like every other NPC projectile.
const SHOT_LIFETIME: u16 = 300;

/// Whether a type lies dormant until struck, rather than hunting from the start.
fn plays_dead(npc_type: u16) -> bool {
    npc_type == 694
}

/// Whether a type spits skulls of its own.
fn spits(npc_type: u16) -> bool {
    npc_type == 289
}

/// Drive one cursed skull for a tick, returning what it spat if it spat anything.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>) -> Option<Shot> {
    let Some(target) = world.target else {
        // Nobody to stalk: drift away and let the despawn timer have it.
        npc.velocity.0 *= 0.98;
        npc.velocity.1 *= 0.98;
        npc.dirty = true;
        return None;
    };

    // States 3 and 4 belong to the mimic: dormant, then stunned after being woken.
    if plays_dead(npc.npc_type) {
        if npc.ai[3] == 3.0 {
            npc.velocity = (0.0, 0.0);
            npc.rotation = 0.0;
            if npc.was_hurt {
                npc.ai[3] = 4.0;
                npc.dirty = true;
            }
            return None;
        }
        if npc.ai[3] == 4.0 {
            npc.velocity = (0.0, 0.0);
            npc.rotation = 0.0;
            // Eighty ticks of shaking itself awake.
            if npc.ai[1] > 80.0 {
                npc.ai[1] = 0.0;
                npc.ai[3] = 0.0;
                npc.dirty = true;
            }
            return None;
        }
    }

    let (cx, cy) = npc.center();
    let (dx, dy) = (target.center.0 - cx, target.center.1 - cy);
    let reach = (dx * dx + dy * dy).sqrt();
    npc.ai[1] += 1.0;

    let charging = npc.ai[1] > SKULL_CHARGE_AT;
    let (mut speed, mut accel) = skull_approach(reach);
    if charging {
        speed = SKULL_CHARGE_SPEED;
        accel = SKULL_CHARGE_ACCEL;
        if npc.ai[1] > SKULL_CHARGE_OVER {
            npc.ai[1] = 0.0;
        }
        npc.dirty = true;
    } else if reach < SKULL_JITTER_RANGE {
        // Circling: a sawtooth on ai[0] pushes it around rather than at anything.
        npc.ai[0] += SKULL_JITTER_RATE;
        npc.velocity.1 += if npc.ai[0] > 0.0 {
            SKULL_JITTER_PUSH
        } else {
            -SKULL_JITTER_PUSH
        };
        npc.velocity.0 += if npc.ai[0] < -SKULL_JITTER_TURN || npc.ai[0] > SKULL_JITTER_TURN {
            SKULL_JITTER_PUSH
        } else {
            -SKULL_JITTER_PUSH
        };
        if npc.ai[0] > SKULL_JITTER_PERIOD {
            npc.ai[0] = -SKULL_JITTER_PERIOD;
            npc.dirty = true;
        }
    }

    // Where it wants to be going.
    let k = speed / reach.max(f32::MIN_POSITIVE);
    let wanted = if target.alive {
        (dx * k, dy * k)
    } else {
        (f32::from(npc.direction) * speed / 2.0, -speed / 2.0)
    };
    if npc.velocity.0 < wanted.0 {
        npc.velocity.0 += accel;
    } else if npc.velocity.0 > wanted.0 {
        npc.velocity.0 -= accel;
    }
    if npc.velocity.1 < wanted.1 {
        npc.velocity.1 += accel;
    } else if npc.velocity.1 > wanted.1 {
        npc.velocity.1 -= accel;
    }

    npc.sprite_direction = if dx > 0.0 { -1 } else { 1 };
    npc.rotation = (dy * k).atan2(dx * k);
    npc.dirty = true;

    if !spits(npc.npc_type) {
        return None;
    }

    // The giant one's spit: a long wind-up, then a short window with the shot in the middle.
    if npc.was_hurt {
        npc.ai[2] = 0.0;
        npc.ai[3] = 0.0;
    }
    if reach > GIANT_SKULL_RANGE {
        npc.ai[2] = 0.0;
        npc.ai[3] = 0.0;
        return None;
    }
    npc.ai[2] += 1.0;
    if npc.ai[3] == 0.0 {
        if npc.ai[2] > GIANT_SKULL_WINDUP {
            npc.ai[2] = 0.0;
            npc.ai[3] = 1.0;
            npc.dirty = true;
        }
        return None;
    }
    if npc.ai[2] > GIANT_SKULL_RECOVER {
        npc.ai[3] = 0.0;
        npc.dirty = true;
    }
    if npc.ai[2] != GIANT_SKULL_RELEASE {
        return None;
    }
    let k = GIANT_SKULL_SHOT_SPEED / reach.max(f32::MIN_POSITIVE);
    Some(Shot {
        projectile: GIANT_SKULL_SHOT_TYPE,
        damage: GIANT_SKULL_SHOT_DAMAGE,
        position: (cx, cy),
        velocity: (dx * k, dy * k),
        time_left: SHOT_LIFETIME,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use terrustia_proto::tile::Tile;

    struct Void;

    impl TileView for Void {
        fn tile(&self, _x: i32, _y: i32) -> Tile {
            Tile::AIR
        }
    }

    fn world<'a>(tiles: &'a Void, target: Option<Target>) -> World<'a, Void> {
        crate::game::ai::calm(tiles, target)
    }

    fn skull(npc_type: u16) -> Npc {
        Npc::new(npc_type, (10_000.0, 10_000.0), 1).expect("a style 10 type")
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
    fn a_skull_closes_fast_from_far_and_dawdles_up_close() {
        assert_eq!(skull_approach(400.0).0, 5.0);
        assert_eq!(skull_approach(320.0).0, 3.0);
        assert_eq!(skull_approach(280.0).0, 1.5);
        assert_eq!(skull_approach(100.0).0, 1.0, "and barely moves once there");
    }

    #[test]
    fn a_skull_charges_after_ten_seconds_of_circling() {
        let tiles = Void;
        let mut s = skull(34);
        let (cx, cy) = s.center();
        let t = Some(player_at(cx + 100.0, cy));
        s.ai[1] = SKULL_CHARGE_AT - 1.0;
        let before = s.velocity;
        update(&mut s, &world(&tiles, t));
        let jitter = (s.velocity.0 - before.0).abs();
        s.velocity = before;
        s.ai[1] = SKULL_CHARGE_AT + 1.0;
        update(&mut s, &world(&tiles, t));
        let charge = (s.velocity.0 - before.0).abs();
        assert!(
            charge > jitter * 2.0,
            "the charge should be much harder: {charge} against {jitter}"
        );
    }

    #[test]
    fn the_charge_ends_and_the_cycle_starts_again() {
        let tiles = Void;
        let mut s = skull(34);
        let (cx, cy) = s.center();
        let t = Some(player_at(cx + 100.0, cy));
        s.ai[1] = SKULL_CHARGE_OVER;
        update(&mut s, &world(&tiles, t));
        assert_eq!(s.ai[1], 0.0);
    }

    #[test]
    fn a_skull_up_close_jitters_rather_than_steering() {
        let tiles = Void;
        let mut s = skull(34);
        let (cx, cy) = s.center();
        // Directly on top of the player, so any movement can only be the jitter.
        let t = Some(player_at(cx + 20.0, cy));
        let mut turns = 0;
        let mut last = s.ai[0];
        for _ in 0..1000 {
            update(&mut s, &world(&tiles, t));
            if s.ai[0] < last {
                turns += 1;
            }
            last = s.ai[0];
        }
        assert!(turns > 0, "the sawtooth should have wrapped at least once");
    }

    #[test]
    fn a_giant_skull_spits_on_a_cycle_and_only_within_range() {
        let tiles = Void;
        let mut s = skull(289);
        let (cx, cy) = s.center();
        let close = Some(player_at(cx + 300.0, cy));
        let mut shot = None;
        for _ in 0..400 {
            if let Some(sh) = update(&mut s, &world(&tiles, close)) {
                shot = Some(sh);
                break;
            }
        }
        let shot = shot.expect("should have spat");
        assert_eq!(shot.projectile, GIANT_SKULL_SHOT_TYPE);
        assert_eq!(shot.damage, GIANT_SKULL_SHOT_DAMAGE);
        let speed = (shot.velocity.0.powi(2) + shot.velocity.1.powi(2)).sqrt();
        assert!((speed - GIANT_SKULL_SHOT_SPEED).abs() < 1e-3);

        let mut far_off = skull(289);
        let far = Some(player_at(cx + GIANT_SKULL_RANGE + 200.0, cy));
        for _ in 0..600 {
            assert!(update(&mut far_off, &world(&tiles, far)).is_none());
        }
    }

    #[test]
    fn an_ordinary_cursed_skull_spits_nothing() {
        let tiles = Void;
        let mut s = skull(34);
        let (cx, cy) = s.center();
        let t = Some(player_at(cx + 200.0, cy));
        for _ in 0..600 {
            assert!(update(&mut s, &world(&tiles, t)).is_none());
        }
    }

    #[test]
    fn a_water_bolt_mimic_plays_dead_until_it_is_hit() {
        let tiles = Void;
        let mut m = skull(694);
        m.ai[3] = 3.0;
        let (cx, cy) = m.center();
        let t = Some(player_at(cx + 100.0, cy));
        for _ in 0..100 {
            update(&mut m, &world(&tiles, t));
        }
        assert_eq!(m.velocity, (0.0, 0.0), "it should not have stirred");
        assert_eq!(m.ai[3], 3.0);

        m.was_hurt = true;
        update(&mut m, &world(&tiles, t));
        assert_eq!(m.ai[3], 4.0, "should have woken");
    }

    #[test]
    fn a_woken_mimic_shakes_itself_off_and_then_hunts() {
        let tiles = Void;
        let mut m = skull(694);
        m.ai[3] = 4.0;
        let (cx, cy) = m.center();
        let t = Some(player_at(cx + 100.0, cy));
        m.ai[1] = 81.0;
        update(&mut m, &world(&tiles, t));
        assert_eq!(m.ai[3], 0.0, "and is now a mimic like any other");
    }
}
