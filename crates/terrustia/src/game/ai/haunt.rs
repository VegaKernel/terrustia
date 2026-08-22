//! Style 22 — the hauntings.
//!
//! Ghosts and dripplers do not fly at you so much as *hang over* you. The routine feels ahead and
//! down for something to float above: finding nothing, it sinks; finding something, it pushes back
//! up off it. That is the whole of the bobbing, and it is why one will follow you along a cave roof
//! and then drop through a doorway.
//!
//! The other half is the anti-stall. A ghost that has spent half a second going nowhere — pinned in
//! a corner, or grinding against a ledge — decides it is stuck, turns around, and spends three full
//! seconds deliberately walking *away* from its target before trying again.

use terrustia_proto::npc_params::{
    HAUNT_BACK_OFF, HAUNT_STUCK_AT, HAUNT_STUCK_OVER, haunt, haunt_feels_by_distance,
    haunt_flees_daylight, haunt_gives_up_at_range,
};
use terrustia_proto::tile_solid::solid;

use super::{World, face, steer_axis};
use crate::game::npc::{Npc, TILE, TileView};

/// How far from where it was it has to have got, in pixels, to count as having moved.
const STIRRED_X: f32 = 16.0;
const STIRRED_Y: f32 = 40.0;

/// Whether the hour or the target is telling it to leave.
fn leaving<T: TileView>(npc: &Npc, world: &World<'_, T>) -> bool {
    if haunt_flees_daylight(npc.npc_type) && world.conditions.day {
        return true;
    }
    if let Some(limit) = haunt_gives_up_at_range(npc.npc_type) {
        return match world.target {
            None => true,
            Some(t) => {
                let (cx, cy) = npc.center();
                !t.alive || ((t.center.0 - cx).powi(2) + (t.center.1 - cy).powi(2)).sqrt() > limit
            }
        };
    }
    false
}

/// Drive one haunting for a tick.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, drift: f32) {
    let params = haunt(npc.npc_type);
    let mut stuck = false;

    if leaving(npc, world) {
        // On its way out: it keeps whatever course it has, and picks one if it has none.
        if npc.velocity.0 == 0.0 {
            npc.velocity.0 = drift;
            npc.dirty = true;
        }
        npc.time_left = npc.time_left.min(10);
    } else if npc.ai[2] >= 0.0 {
        // `ai[0..1]` remember where it was; if it is still about there, the counter climbs.
        let held_x = (npc.position.0 > npc.ai[0] - STIRRED_X
            && npc.position.0 < npc.ai[0] + STIRRED_X)
            || (npc.velocity.0 < 0.0 && npc.direction > 0)
            || (npc.velocity.0 > 0.0 && npc.direction < 0);
        let held_y =
            npc.position.1 > npc.ai[1] - STIRRED_Y && npc.position.1 < npc.ai[1] + STIRRED_Y;
        if held_x && held_y {
            npc.ai[2] += 1.0;
            if npc.ai[2] >= HAUNT_STUCK_AT {
                stuck = true;
            }
            if npc.ai[2] >= HAUNT_STUCK_OVER {
                npc.ai[2] = -HAUNT_BACK_OFF;
                npc.direction = -npc.direction;
                npc.velocity.0 = -npc.velocity.0;
                npc.collide_x = false;
                npc.dirty = true;
            }
        } else {
            npc.ai[0] = npc.position.0;
            npc.ai[1] = npc.position.1;
            npc.ai[2] = 0.0;
            npc.dirty = true;
        }
        if let Some(t) = world.target {
            face(npc, t);
        }
    } else {
        // Backing off: it deliberately faces away from whoever it was chasing.
        npc.ai[2] += 1.0;
        if let Some(t) = world.target {
            npc.direction = if t.center.0 > npc.center().0 { -1 } else { 1 };
        }
    }

    // How far ahead and down to feel. A drippler reaches further the further off its target is.
    let mut feel = params.feel;
    if haunt_feels_by_distance(npc.npc_type)
        && let Some(t) = world.target
    {
        let (cx, cy) = npc.center();
        let reach = ((t.center.0 - cx).powi(2) + (t.center.1 - cy).powi(2)).sqrt() / 70.0;
        feel += reach.min(8.0) as i32;
    }

    let probe_x = (npc.center().0 / TILE) as i32 + i32::from(npc.direction) * 2;
    let probe_y = ((npc.position.1 + npc.height()) / TILE) as i32;
    let mut nothing_below = true;
    let mut right_on_it = false;
    // It only bothers looking while it is below its target's head; above that it just floats.
    let below_target = world.target.is_some_and(|t| {
        npc.position.1 + npc.height() > t.center.1 - super::PLAYER_HEIGHT as f32 / 2.0
    });
    if below_target {
        for step in 0..feel {
            let tile = world.tiles.tile(probe_x, probe_y + step);
            if (tile.is_active() && solid(tile.block)) || tile.liquid > 0 {
                if step <= 1 {
                    right_on_it = true;
                }
                nothing_below = false;
                break;
            }
        }
    }
    if stuck {
        // Being stuck overrides the terrain reading: sink out of wherever it is wedged.
        right_on_it = false;
        nothing_below = true;
    }

    if nothing_below {
        npc.velocity.1 += params.sink;
        if npc.velocity.1 > params.sink_cap {
            npc.velocity.1 = params.sink_cap;
        }
    } else {
        if (npc.direction_y < 0 && npc.velocity.1 > 0.0) || right_on_it {
            npc.velocity.1 -= params.lift;
        }
        if let Some(cap) = params.lift_cap
            && npc.velocity.1 < -cap
        {
            npc.velocity.1 = -cap;
        }
        if npc.velocity.1 < -4.0 {
            npc.velocity.1 = -4.0;
        }
    }

    // A soft rebound: it drifts off terrain rather than bouncing away from it.
    if npc.collide_x {
        npc.velocity.0 = npc.old_velocity.0 * -0.4;
        if npc.direction == -1 && npc.velocity.0 > 0.0 && npc.velocity.0 < 1.0 {
            npc.velocity.0 = 1.0;
        }
        if npc.direction == 1 && npc.velocity.0 < 0.0 && npc.velocity.0 > -1.0 {
            npc.velocity.0 = -1.0;
        }
    }
    if npc.collide_y {
        npc.velocity.1 = npc.old_velocity.1 * -0.25;
        if npc.velocity.1 > 0.0 && npc.velocity.1 < 1.0 {
            npc.velocity.1 = 1.0;
        }
        if npc.velocity.1 < 0.0 && npc.velocity.1 > -1.0 {
            npc.velocity.1 = -1.0;
        }
    }

    steer_axis(&mut npc.velocity.0, npc.direction, params.steering.x);
    steer_axis(&mut npc.velocity.1, npc.direction_y, params.steering.y);
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Crypt(HashMap<(i32, i32), Tile>);

    impl TileView for Crypt {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn floor(top: i32) -> Crypt {
        let mut c = Crypt::default();
        for x in 0..2000 {
            for y in top..top + 10 {
                c.0.insert((x, y), Tile::block(1));
            }
        }
        c
    }

    fn ghost(npc_type: u16, tile_x: i32, tile_y: i32) -> Npc {
        Npc::new(npc_type, (tile_x as f32 * TILE, tile_y as f32 * TILE), 1)
            .expect("a style 22 type")
    }

    fn night<'a>(tiles: &'a Crypt, target: Option<Target>) -> World<'a, Crypt> {
        crate::game::ai::calm(tiles, target)
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
    fn a_ghost_sinks_over_a_drop_and_lifts_over_ground() {
        let empty = Crypt::default();
        let mut falling = ghost(316, 100, 100);
        falling.direction = 1;
        falling.direction_y = -1;
        let (cx, cy) = falling.center();
        // Level with it: the ground probe only runs while the haunting is at or below its
        // target's head, which is what lets one drift over your head and then drop on you.
        let t = Some(player_at(cx + 100.0, cy));
        update(&mut falling, &night(&empty, t), 1.5);
        assert!(falling.velocity.1 > 0.0, "should sink into the drop");

        let solid = floor(102);
        let mut hovering = ghost(316, 100, 100);
        hovering.direction = 1;
        hovering.direction_y = -1;
        hovering.velocity.1 = 1.0;
        update(&mut hovering, &night(&solid, t), 1.5);
        assert!(
            hovering.velocity.1 < 1.0,
            "should push off it, got {}",
            hovering.velocity.1
        );
    }

    #[test]
    fn a_drippler_hangs_lower_and_slower_than_a_ghost() {
        assert!(haunt(490).steering.x.max < haunt(316).steering.x.max);
        assert!(haunt(490).sink < haunt(316).sink);
        assert!(haunt_feels_by_distance(490));
        assert!(!haunt_feels_by_distance(316));
    }

    #[test]
    fn a_drippler_leaves_at_dawn_and_a_ghost_does_not() {
        assert!(haunt_flees_daylight(490));
        assert!(!haunt_flees_daylight(316));

        let tiles = floor(200);
        let mut d = ghost(490, 100, 100);
        let (cx, cy) = d.center();
        let mut w = night(&tiles, Some(player_at(cx + 100.0, cy)));
        w.conditions.day = true;
        update(&mut d, &w, 1.5);
        assert!(d.time_left <= 10, "should be leaving, got {}", d.time_left);
    }

    #[test]
    fn a_ghost_gives_up_on_a_target_across_the_world() {
        let tiles = floor(200);
        let mut g = ghost(316, 100, 100);
        let (cx, cy) = g.center();
        update(
            &mut g,
            &night(&tiles, Some(player_at(cx + 5000.0, cy))),
            1.5,
        );
        assert!(g.time_left <= 10, "should be leaving");
    }

    #[test]
    fn a_ghost_going_nowhere_backs_away_and_then_comes_back() {
        let tiles = floor(200);
        let mut g = ghost(316, 100, 100);
        g.direction = 1;
        g.ai[0] = g.position.0;
        g.ai[1] = g.position.1;
        let (cx, cy) = g.center();
        // A target off to the right, which it will not be making progress toward.
        let t = Some(player_at(cx + 400.0, cy));

        for _ in 0..(HAUNT_STUCK_OVER as i32 + 1) {
            update(&mut g, &night(&tiles, t), 1.5);
        }
        assert!(g.ai[2] < 0.0, "should have decided it is stuck");
        assert_eq!(g.direction, -1, "and turned away");

        // Facing away from the target while it backs off, rather than turning straight back.
        update(&mut g, &night(&tiles, t), 1.5);
        assert_eq!(g.direction, -1);

        for _ in 0..(HAUNT_BACK_OFF as i32 + 2) {
            update(&mut g, &night(&tiles, t), 1.5);
        }
        assert!(g.ai[2] >= 0.0, "should be hunting again");
    }

    #[test]
    fn moving_along_resets_the_stuck_counter() {
        let tiles = floor(200);
        let mut g = ghost(316, 100, 100);
        g.direction = 1;
        let (cx, cy) = g.center();
        let t = Some(player_at(cx + 400.0, cy));
        for _ in 0..40 {
            update(&mut g, &night(&tiles, t), 1.5);
            // Actually getting somewhere.
            g.position.0 += 40.0;
        }
        assert_eq!(g.ai[2], 0.0, "it is not stuck if it is moving");
    }

    #[test]
    fn a_haunting_rebounds_softly_rather_than_bouncing() {
        let tiles = floor(200);
        let mut g = ghost(316, 100, 100);
        g.direction = 1;
        g.velocity = (2.0, 0.0);
        g.old_velocity = (2.0, 0.0);
        g.collide_x = true;
        let (cx, cy) = g.center();
        update(&mut g, &night(&tiles, Some(player_at(cx + 400.0, cy))), 1.5);
        assert!(
            g.velocity.0 < 0.0 && g.velocity.0 > -2.0,
            "should ease back, got {}",
            g.velocity.0
        );
    }
}
