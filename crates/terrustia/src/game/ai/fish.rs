//! Style 16 — the swimmers.
//!
//! Goldfish, jellyfish, piranhas, sharks. Ported from the `aiStyle == 16` block: in water they
//! steer with a per-axis acceleration and bounce off anything they hit; out of water they simply
//! fall and flop.

use terrustia_proto::npc_params::{
    ARAPAIMA, ARAPAIMA_REVERSE_DAMPING, swim_speed, swimmer_is_passive,
};

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

    // An arapaima that is still carrying speed the wrong way sheds it before it accelerates
    // (`NPC.cs:23886-23891`), which is why it turns as sharply as it does at seven pixels a tick.
    if npc.npc_type == ARAPAIMA
        && ((npc.velocity.0 > 0.0 && npc.direction < 0)
            || (npc.velocity.0 < 0.0 && npc.direction > 0))
    {
        npc.velocity.0 *= ARAPAIMA_REVERSE_DAMPING;
    }

    let s = swim_speed(npc.npc_type);
    npc.velocity.0 += f32::from(npc.direction) * s.accel.0;
    npc.velocity.1 += f32::from(npc.direction_y) * s.accel.1;
    // Vanilla tests and assigns two separate numbers per axis rather than clamping; they are equal
    // for everything but the arapaima. See `SwimSpeed::max_x`.
    if npc.velocity.0 > s.max_x.0 {
        npc.velocity.0 = s.max_x.1;
    } else if npc.velocity.0 < -s.max_x.0 {
        npc.velocity.0 = -s.max_x.1;
    }
    if npc.velocity.1 > s.max_y.0 {
        npc.velocity.1 = s.max_y.1;
    } else if npc.velocity.1 < -s.max_y.0 {
        npc.velocity.1 = -s.max_y.1;
    }
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

    /// B2: `NPC.cs:23892-23919` gives the arapaima a branch ahead of the shark's, and this port
    /// had neither, so the fastest swimmer in the game fell to the 0.1/3.0 default and was
    /// trivially outswum. Its cap is also written differently from everything else: it is *tested*
    /// against 8 and knocked back to 7, so it surges between the two rather than sitting on a
    /// ceiling.
    #[test]
    fn an_arapaima_outswims_a_shark_and_surges_rather_than_riding_its_cap() {
        let mut it = fish(ARAPAIMA);
        it.direction = 1;
        it.direction_y = 1;
        let mut seen: Vec<f32> = Vec::new();
        for _ in 0..400 {
            update(&mut it, None, true, false);
            seen.push(it.velocity.0);
        }
        // Settled behaviour, past the run-up.
        let tail = &seen[seen.len() - 40..];
        let top = tail.iter().copied().fold(f32::MIN, f32::max);
        let low = tail.iter().copied().fold(f32::MAX, f32::min);
        assert_eq!(top, 8.0, "it climbs to eight");
        assert_eq!(low, 7.0, "and is knocked back to seven, not held at eight");

        // A shark, by contrast, sits exactly on its cap because its two numbers are the same.
        let mut shark = fish(65);
        shark.direction = 1;
        shark.direction_y = 1;
        let mut shark_tail: Vec<f32> = Vec::new();
        for _ in 0..400 {
            update(&mut shark, None, true, false);
            shark_tail.push(shark.velocity.0);
        }
        assert!(
            shark_tail[shark_tail.len() - 40..]
                .iter()
                .all(|v| *v == 5.0),
            "a shark rides a flat ceiling"
        );
        assert!(
            low > shark.velocity.0,
            "and the arapaima is faster even at its slowest"
        );
    }

    /// The 0.95 shed on a reversal (`NPC.cs:23886-23891`) is the arapaima's alone, and it is what
    /// lets something moving at seven pixels a tick turn at all.
    #[test]
    fn only_the_arapaima_sheds_speed_when_it_turns() {
        let mut it = fish(ARAPAIMA);
        it.velocity.0 = 8.0;
        it.direction = -1;
        it.direction_y = 0;
        update(&mut it, None, true, false);
        // 8 * 0.95 = 7.6, then the -1 facing takes 0.25 off.
        assert!((it.velocity.0 - 7.35).abs() < 1e-4, "got {}", it.velocity.0);

        let mut shark = fish(65);
        shark.velocity.0 = 5.0;
        shark.direction = -1;
        shark.direction_y = 0;
        update(&mut shark, None, true, false);
        assert!(
            (shark.velocity.0 - 4.85).abs() < 1e-4,
            "a shark only decelerates, got {}",
            shark.velocity.0
        );
    }
}
