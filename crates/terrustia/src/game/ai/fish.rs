//! Style 16 — the swimmers.
//!
//! Goldfish, jellyfish, piranhas, sharks. Ported from the `aiStyle == 16` block: in water they
//! steer with a per-axis acceleration and bounce off anything they hit; out of water they simply
//! fall and flop.

use terrustia_proto::npc_params::{swim_speed, swimmer_is_passive};

use crate::game::npc::Npc;
use crate::game::npc_ai::Target;

/// Gravity applied to a fish that has flopped out of the water.
pub const BEACHED_GRAVITY: f32 = 0.3;

/// Whether this fish will hunt the player at all.
///
/// Even a hunter only gives chase when its target is in the water too.
fn hunts(npc: &Npc, target: Option<Target>, target_is_wet: bool) -> bool {
    !swimmer_is_passive(npc.npc_type) && target.is_some() && target_is_wet
}

/// Drive one swimmer.
///
/// `wet` and `target_is_wet` come from the world; the routine itself never looks at tiles.
pub fn update(npc: &mut Npc, target: Option<Target>, wet: bool, target_is_wet: bool) {
    if !wet {
        // Out of water it is at the mercy of gravity, and barely steers.
        npc.velocity.1 += BEACHED_GRAVITY;
        npc.velocity.0 *= 0.98;
        return;
    }

    let chasing = hunts(npc, target, target_is_wet);

    // Bounce off whatever it swam into. A hunting fish pushes on instead.
    if !chasing {
        if npc.collide_x {
            npc.velocity.0 *= -1.0;
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
    }

    if chasing && let Some(t) = target {
        let (cx, cy) = npc.center();
        npc.direction = if t.center.0 > cx { 1 } else { -1 };
        npc.direction_y = if t.center.1 > cy { 1 } else { -1 };
    }

    let s = swim_speed(npc.npc_type);
    npc.velocity.0 = (npc.velocity.0 + f32::from(npc.direction) * s.accel).clamp(-s.max_x, s.max_x);
    npc.velocity.1 =
        (npc.velocity.1 + f32::from(npc.direction_y) * s.accel).clamp(-s.max_y, s.max_y);
    npc.sprite_direction = npc.direction;
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fish(npc_type: u16) -> Npc {
        Npc::new(npc_type, (1000.0, 1000.0), 1).expect("swimmer type")
    }

    fn player(x: f32, y: f32) -> Target {
        Target {
            slot: 0,
            center: (x, y),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    #[test]
    fn a_beached_fish_falls_and_does_not_swim() {
        let mut f = fish(55);
        update(&mut f, Some(player(5000.0, 1000.0)), false, false);
        assert_eq!(f.velocity.1, BEACHED_GRAVITY, "gravity takes over");
        assert!(f.velocity.0.abs() < 0.001, "and it cannot steer");
    }

    #[test]
    fn a_shark_hunts_a_swimmer_and_a_goldfish_never_does() {
        let mut shark = fish(65);
        shark.direction = -1;
        for _ in 0..40 {
            update(&mut shark, Some(player(5000.0, 1000.0)), true, true);
        }
        assert_eq!(shark.direction, 1, "the shark turns toward its prey");
        assert!(shark.velocity.0 > 0.0);

        let mut goldfish = fish(55);
        goldfish.direction = -1;
        for _ in 0..40 {
            update(&mut goldfish, Some(player(5000.0, 1000.0)), true, true);
        }
        assert_eq!(goldfish.direction, -1, "a goldfish is not interested");
    }

    #[test]
    fn a_hunter_ignores_a_target_that_is_out_of_the_water() {
        let mut shark = fish(65);
        shark.direction = -1;
        for _ in 0..40 {
            update(&mut shark, Some(player(5000.0, 1000.0)), true, false);
        }
        assert_eq!(shark.direction, -1, "it cannot reach a target on land");
    }

    #[test]
    fn swimmers_bounce_off_what_they_hit() {
        let mut f = fish(55);
        f.velocity = (2.0, 1.0);
        f.direction = 1;
        f.collide_x = true;
        update(&mut f, None, true, false);
        assert_eq!(f.direction, -1, "should turn around");
        assert!(f.velocity.0 < 0.0, "and swim back");

        let mut g = fish(55);
        g.velocity = (0.0, 1.5);
        g.collide_y = true;
        update(&mut g, None, true, false);
        assert_eq!(g.direction_y, -1, "should head back up");
        assert_eq!(g.ai[0], -1.0);
    }

    #[test]
    fn speeds_are_capped_at_the_types_limits() {
        let mut shark = fish(65);
        shark.direction = 1;
        shark.direction_y = 1;
        for _ in 0..400 {
            update(&mut shark, None, true, false);
        }
        assert_eq!(shark.velocity.0, 5.0);
        assert_eq!(shark.velocity.1, 3.0);

        let mut goldfish = fish(55);
        goldfish.direction = 1;
        goldfish.direction_y = 1;
        for _ in 0..400 {
            update(&mut goldfish, None, true, false);
        }
        assert_eq!(goldfish.velocity.0, 3.0);
        assert_eq!(goldfish.velocity.1, 2.0);
    }
}
