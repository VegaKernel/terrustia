//! Swoopers: style 86 — the shadowflame apparition and the ancient cultist's squidhead.
//!
//! A swooper never turns toward you. It runs *past* you in a straight line, tracking your height as
//! it goes, and only once it is several hundred pixels beyond does it begin a wide three-legged
//! turn: up or down away from you, then back along the horizontal, then round into another run.
//! The result is a figure that keeps crossing the screen rather than one that closes, and the
//! difference between the two types is entirely in how tight that figure is.
//!
//! Two of them within fifty pixels shoulder each other apart, which is what stops a pair
//! overlapping into what looks like one enemy hitting twice.

use terrustia_proto::npc_params::{
    APPARITION_SWOOP, SQUIDHEAD, SQUIDHEAD_SWOOP, SWOOP_ENTRANCE, SWOOP_ENTRANCE_SHOVE, SWOOP_FADE,
    SWOOP_HARD_TRACK, SWOOP_PERSONAL_SPACE, SWOOP_SHOVE, Swoop,
};

use crate::game::ai::{World, face};
use crate::game::npc::{Npc, TileView};

fn table(npc_type: u16) -> Swoop {
    if npc_type == SQUIDHEAD {
        SQUIDHEAD_SWOOP
    } else {
        APPARITION_SWOOP
    }
}

/// Style 86.
pub fn swooper(npc: &mut Npc, world: &World<'_, impl TileView>) {
    npc.dirty = true;
    let s = table(npc.npc_type);
    npc.no_gravity = true;
    npc.no_tile_collide = true;
    npc.knockback_immune = true;

    // They arrive faded out and solidify over four ticks.
    if npc.alpha > 0 {
        npc.alpha = (npc.alpha - SWOOP_FADE).max(0);
    }

    // Two too close shove apart. Vanilla writes this push twice over: once from this NPC's own
    // pass, which pushes itself, and once more from the *neighbour's* own pass, which reaches
    // across and pushes this one directly. A routine here only ever has its own NPC to move, so
    // it cannot reach across the way vanilla's second write does — doubling its own push is what
    // makes up the difference and keeps the pair separating at the true, full rate.
    let (cx, cy) = npc.center();
    for (kx, ky, _) in world.avoid {
        let (dx, dy) = (kx - cx, ky - cy);
        let gap = dx.hypot(dy);
        if gap < SWOOP_PERSONAL_SPACE {
            let push = if gap > 0.0 {
                (dx / gap * SWOOP_SHOVE * 2.0, dy / gap * SWOOP_SHOVE * 2.0)
            } else {
                // Exactly on top of one another: pick a side rather than divide by zero.
                (SWOOP_SHOVE * 2.0, 0.0)
            };
            npc.velocity.0 -= push.0;
            npc.velocity.1 -= push.1;
        }
    }

    // The entrance: a shove in whichever direction it is facing, once.
    if npc.local_ai[0] < SWOOP_ENTRANCE {
        if npc.local_ai[0] == 0.0 {
            if let Some(target) = world.target {
                face(npc, target);
            }
            npc.velocity.0 += SWOOP_ENTRANCE_SHOVE * f32::from(npc.direction);
        }
        npc.local_ai[0] += 1.0;
    }

    let Some(target) = world.target.filter(|t| t.alive) else {
        return;
    };

    match npc.ai[0] {
        0.0 => {
            // Choosing a run.
            face(npc, target);
            npc.ai[0] = 1.0;
            npc.ai[1] = f32::from(npc.direction);
        }

        1.0 => {
            // The run. It accelerates along `ai[1]` and holds your height as it passes.
            face(npc, target);
            npc.velocity.0 =
                (npc.velocity.0 + npc.ai[1] * s.run_accel).clamp(-s.run_cap, s.run_cap);

            let mut rise = target.center.1 - cy;
            // Far off your height it corrects hard; near it, gently — so a run arrives level.
            let smooth = if rise.abs() > s.track_band {
                SWOOP_HARD_TRACK
            } else {
                s.track_smooth
            };
            rise = rise.clamp(-s.track_band, s.track_band);
            npc.velocity.1 = (npc.velocity.1 * (smooth - 1.0) + rise) / smooth;

            // Past you by enough: start the turn, upward if it is below you and down if above.
            let across = target.center.0 - cx;
            let past = (npc.ai[1] > 0.0 && across < -s.overshoot)
                || (npc.ai[1] < 0.0 && across > s.overshoot);
            if past {
                npc.ai[0] = 2.0;
                npc.ai[1] = if cy + 20.0 > target.center.1 {
                    -1.0
                } else {
                    1.0
                };
            }
        }

        2.0 => {
            // The vertical leg, until the horizontal speed has bled off.
            npc.velocity.1 += npc.ai[1] * s.climb_accel;
            if npc.velocity.0.hypot(npc.velocity.1) > s.climb_cap {
                npc.velocity.0 *= s.climb_drag;
                npc.velocity.1 *= s.climb_drag;
            }
            if npc.velocity.0.abs() < 1.0 {
                face(npc, target);
                npc.ai[0] = 3.0;
                npc.ai[1] = f32::from(npc.direction);
            }
        }

        _ => {
            // The return leg, until the vertical speed has bled off and it can run again.
            npc.velocity.0 += npc.ai[1] * s.return_accel;
            if cy > target.center.1 {
                npc.velocity.1 -= s.return_pull;
            } else {
                npc.velocity.1 += s.return_pull;
            }
            if npc.velocity.0.hypot(npc.velocity.1) > s.return_cap {
                npc.velocity.0 *= s.return_drag;
                npc.velocity.1 *= s.return_drag;
            }
            if npc.velocity.1.abs() < 1.0 {
                face(npc, target);
                npc.ai[0] = 0.0;
                npc.ai[1] = f32::from(npc.direction);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Sky(HashMap<(i32, i32), Tile>);

    impl TileView for Sky {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn world<'a>(tiles: &'a Sky, target: Option<(f32, f32)>) -> World<'a, Sky> {
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

    const APPARITION: u16 = 472;

    fn apparition(x: f32, y: f32) -> Npc {
        Npc::new(APPARITION, (x, y), 1).expect("shadowflame apparition")
    }

    /// It runs past you rather than into you, and keeps doing it.
    #[test]
    fn a_swooper_crosses_you_again_and_again() {
        let tiles = Sky(HashMap::new());
        let mut a = apparition(0.0, 0.0);
        let w = world(&tiles, Some((0.0, 0.0)));

        let mut crossings = 0;
        let mut side = a.center().0 - 0.0;
        for _ in 0..4000 {
            swooper(&mut a, &w);
            a.position.0 += a.velocity.0;
            a.position.1 += a.velocity.1;
            let now = a.center().0;
            if side.signum() != now.signum() && now != 0.0 {
                crossings += 1;
            }
            side = now;
        }
        assert!(
            crossings >= 3,
            "it should keep crossing back and forth, got {crossings}"
        );
    }

    /// The turn is a real three-legged figure, not a stop and a reverse.
    #[test]
    fn the_turn_goes_through_all_three_legs() {
        let tiles = Sky(HashMap::new());
        let mut a = apparition(0.0, 0.0);
        let w = world(&tiles, Some((0.0, 0.0)));

        let mut phases = vec![a.ai[0]];
        for _ in 0..4000 {
            swooper(&mut a, &w);
            a.position.0 += a.velocity.0;
            a.position.1 += a.velocity.1;
            if phases.last() != Some(&a.ai[0]) {
                phases.push(a.ai[0]);
            }
        }
        assert!(phases.contains(&1.0), "a run: {phases:?}");
        assert!(phases.contains(&2.0), "a climb: {phases:?}");
        assert!(phases.contains(&3.0), "and a return: {phases:?}");
    }

    /// While running it holds your height, so a swoop arrives level rather than diagonally.
    #[test]
    fn a_run_comes_in_level_with_you() {
        let tiles = Sky(HashMap::new());
        let mut a = apparition(0.0, 0.0);
        // Player well below where it starts.
        let w = world(&tiles, Some((2000.0, 600.0)));
        a.ai[0] = 1.0;
        a.ai[1] = 1.0;

        // Measured while it is still running: once it overshoots it starts its turn, and the
        // turn is meant to take it away from your height again.
        let mut closest = f32::MAX;
        for _ in 0..400 {
            swooper(&mut a, &w);
            a.position.0 += a.velocity.0;
            a.position.1 += a.velocity.1;
            if a.ai[0] == 1.0 {
                closest = closest.min((a.center().1 - 600.0).abs());
            }
        }
        assert!(
            closest < 20.0,
            "it should have levelled off with the player, off by {closest}"
        );
    }

    /// The squidhead is the faster of the two, which is most of what makes it different.
    #[test]
    fn a_squidhead_is_quicker_than_an_apparition() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((3000.0, 0.0)));
        let top_speed = |npc_type: u16| {
            let mut n = Npc::new(npc_type, (0.0, 0.0), 1).unwrap();
            n.ai[0] = 1.0;
            n.ai[1] = 1.0;
            let mut fastest: f32 = 0.0;
            for _ in 0..200 {
                swooper(&mut n, &w);
                fastest = fastest.max(n.velocity.0);
            }
            fastest
        };
        assert!(
            top_speed(SQUIDHEAD) > top_speed(APPARITION),
            "the squidhead should be faster"
        );
    }

    /// Two on the same spot separate rather than overlapping.
    #[test]
    fn two_swoopers_shoulder_each_other_apart() {
        let tiles = Sky(HashMap::new());
        // Past the entrance shove, which would otherwise swamp a push this small.
        let run = |neighbour: &[(f32, f32, f32)]| {
            let mut a = apparition(0.0, 0.0);
            a.local_ai[0] = SWOOP_ENTRANCE;
            let mut w = world(&tiles, Some((5000.0, 0.0)));
            w.avoid = neighbour;
            swooper(&mut a, &w);
            a.velocity.0
        };
        let (cx, cy) = apparition(0.0, 0.0).center();
        let alone = run(&[]);
        let crowded = run(&[(cx + 10.0, cy, 0.0)]);
        assert!(
            crowded < alone,
            "a neighbour on the right should push it left: {crowded} vs {alone}"
        );
    }

    /// Vanilla writes the shove into both NPCs from both of their passes; this routine only ever
    /// moves its own NPC, so its lone push has to be doubled to stand in for the write it cannot
    /// make into the neighbour, and end up at the true, full separation rate.
    #[test]
    fn the_shove_makes_up_for_not_being_able_to_push_the_neighbour_too() {
        let tiles = Sky(HashMap::new());
        let mut a = apparition(0.0, 0.0);
        a.local_ai[0] = SWOOP_ENTRANCE;
        let mut w = world(&tiles, Some((5000.0, 0.0)));
        let (cx, cy) = a.center();
        let neighbour = [(cx + 10.0, cy, 0.0)];
        w.avoid = &neighbour;

        swooper(&mut a, &w);
        assert!(
            (a.velocity.0.abs() - SWOOP_SHOVE * 2.0).abs() < 1e-4,
            "the self-push should double up for the write vanilla makes into the neighbour \
             directly: got {}, wanted {}",
            a.velocity.0.abs(),
            SWOOP_SHOVE * 2.0
        );
    }
}
