//! Style 26 — the tumbleweed.
//!
//! It rolls. There is no chase in it: it picks a direction, accelerates to four pixels a tick, and
//! bounces along until something stops it. A sandstorm adds to the speed in the direction the wind
//! is blowing and takes from it in the other, so a storm turns a scattering of tumbleweeds into a
//! one-way stampede.
//!
//! The other half is the anti-stall. `ai[3]` counts up whenever it makes no headway — pinned
//! against a wall, or rolling against its own facing — and once it has spent half a second stuck it
//! stops re-targeting and just reverses whenever it comes to a complete stop.

use terrustia_proto::npc_params::{
    TUMBLEWEED_ACCEL, TUMBLEWEED_JUMPS, TUMBLEWEED_LEAP, TUMBLEWEED_PATIENCE,
    TUMBLEWEED_PATIENCE_CAP, TUMBLEWEED_SPEED, TUMBLEWEED_STEP, TUMBLEWEED_WIND,
};
use terrustia_proto::tile_solid::{solid, solid_top};

use super::World;
use crate::game::npc::{Npc, TILE, TileView};

/// How fast it has to be rolling before it will leap a gap.
const LEAP_SPEED: f32 = 3.0;

fn blocking(tiles: &impl TileView, x: i32, y: i32) -> bool {
    let t = tiles.tile(x, y);
    t.is_active() && solid(t.block) && !solid_top(t.block)
}

/// Drive one tumbleweed for a tick.
///
/// `in_a_sandstorm` says whether its target is standing in one, which is the only thing that makes
/// the wind count; `crowding` keeps a drift of them from stacking into one another.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, in_a_sandstorm: bool) {
    // Shoulder its neighbours aside so a drift spreads out.
    npc.velocity.0 += world.crowding.0 * 0.05;
    npc.velocity.1 += world.crowding.1 * 0.05;

    let mut stalled = false;
    if npc.velocity.1 == 0.0
        && ((npc.velocity.0 > 0.0 && npc.direction < 0)
            || (npc.velocity.0 < 0.0 && npc.direction > 0))
    {
        stalled = true;
        npc.ai[3] += 1.0;
    }

    // Hop the last stretch when it is bearing down on someone.
    if let Some(t) = world.target
        && npc.velocity.1 == 0.0
        && npc.velocity.0.abs() > LEAP_SPEED
        && ((npc.center().0 < t.center.0 && npc.velocity.0 > 0.0)
            || (npc.center().0 > t.center.0 && npc.velocity.0 < 0.0))
    {
        npc.velocity.1 -= 4.0;
    }

    let stuck = npc.position.0 == npc.old_position.0 || npc.ai[3] >= TUMBLEWEED_PATIENCE;
    if stuck || stalled {
        npc.ai[3] += 1.0;
    } else if npc.ai[3] > 0.0 {
        npc.ai[3] -= 1.0;
    }
    if npc.ai[3] > TUMBLEWEED_PATIENCE_CAP {
        npc.ai[3] = 0.0;
    }
    if npc.was_hurt {
        npc.ai[3] = 0.0;
    }
    // Close enough to somebody, and not stuck: it stops counting.
    if let Some(t) = world.target {
        let (cx, cy) = npc.center();
        let gap = ((t.center.0 - cx).powi(2)
            + (t.center.1 - super::PLAYER_HEIGHT as f32 / 2.0 - cy).powi(2))
        .sqrt();
        if gap < 200.0 && !(stuck || stalled) {
            npc.ai[3] = 0.0;
        }
    }

    // Out of the desert it simply leaves.
    if !in_a_sandstorm && world.target.is_some_and(|_| !world.conditions.desert) {
        npc.time_left = npc.time_left.min(10);
        npc.ai[3] = TUMBLEWEED_PATIENCE;
    }

    if npc.ai[3] < TUMBLEWEED_PATIENCE {
        if let Some(t) = world.target {
            npc.direction = if t.center.0 > npc.center().0 { 1 } else { -1 };
        }
    } else {
        // Given up on steering: it reverses whenever it comes to a complete stop.
        if npc.velocity.0 == 0.0 {
            if npc.velocity.1 == 0.0 {
                npc.ai[0] += 1.0;
                if npc.ai[0] >= 2.0 {
                    npc.direction = -npc.direction;
                    npc.ai[0] = 0.0;
                }
            }
        } else {
            npc.ai[0] = 0.0;
        }
        npc.direction_y = -1;
        if npc.direction == 0 {
            npc.direction = 1;
        }
    }

    // Rolling. The wind helps one way and hinders the other, which is what makes a sandstorm
    // sweep them all in one direction.
    let boost = if in_a_sandstorm {
        let strength = (0.6 + 0.4 * world.conditions.wind.abs()) * world.conditions.wind.signum();
        strength * f32::from(npc.direction) * TUMBLEWEED_WIND
    } else {
        0.0
    };
    let top = TUMBLEWEED_SPEED + boost;
    let grounded = npc.velocity.1 == 0.0
        || world.wet
        || (npc.velocity.0 <= 0.0 && npc.direction < 0)
        || (npc.velocity.0 >= 0.0 && npc.direction > 0);
    if grounded {
        if npc.velocity.0.signum() as i32 != i32::from(npc.direction) {
            npc.velocity.0 *= 0.92;
        }
        if npc.velocity.0 < -top || npc.velocity.0 > top {
            if npc.velocity.1 == 0.0 {
                npc.velocity.0 *= 0.8;
                npc.velocity.1 *= 0.8;
            }
        } else if npc.velocity.0 < top && npc.direction == 1 {
            npc.velocity.0 = (npc.velocity.0 + TUMBLEWEED_ACCEL).min(top);
        } else if npc.velocity.0 > -top && npc.direction == -1 {
            npc.velocity.0 = (npc.velocity.0 - TUMBLEWEED_ACCEL).max(-top);
        }
    }

    // Roll up a low step rather than jumping it.
    if npc.velocity.1 >= 0.0 {
        let ahead = npc.velocity.0.signum() as i32;
        let next_x = npc.position.0 + npc.velocity.0;
        let probe_x =
            ((next_x + npc.width() / 2.0 + (npc.width() / 2.0 + 1.0) * ahead as f32) / TILE) as i32;
        let foot_y = ((npc.position.1 + npc.velocity.1 + npc.height() - 1.0) / TILE) as i32;
        if blocking(world.tiles, probe_x, foot_y)
            && !blocking(world.tiles, probe_x, foot_y - 1)
            && !blocking(world.tiles, probe_x, foot_y - 2)
            && !blocking(world.tiles, probe_x, foot_y - 3)
        {
            let step_top = (foot_y * 16) as f32;
            let rise = npc.position.1 + npc.height() - step_top;
            if rise > 0.0 && rise <= TUMBLEWEED_STEP {
                npc.position.1 = step_top - npc.height();
                npc.dirty = true;
            }
        }
    }

    // On the ground with headroom: jump whatever is in the way, sized to how tall it is.
    if npc.velocity.1 == 0.0 {
        let head_y = ((npc.position.1 - 7.0) / TILE) as i32;
        let clear_above = ((npc.position.0 - 7.0) / TILE) as i32
            ..=((npc.position.0 + npc.width() + 7.0) / TILE) as i32;
        let headroom = !clear_above
            .clone()
            .any(|x| blocking(world.tiles, x, head_y));
        if headroom {
            let probe_x = ((npc.position.0
                + npc.width() / 2.0
                + (npc.width() / 2.0 + 2.0) * f32::from(npc.direction)
                + npc.velocity.0 * 5.0)
                / TILE) as i32;
            let probe_y = ((npc.position.1 + npc.height() - 15.0) / TILE) as i32;
            let heading = (npc.velocity.0 < 0.0 && npc.direction == -1)
                || (npc.velocity.0 > 0.0 && npc.direction == 1);
            if heading {
                if blocking(world.tiles, probe_x, probe_y - 2) {
                    npc.velocity.1 = if blocking(world.tiles, probe_x, probe_y - 3) {
                        TUMBLEWEED_JUMPS[0]
                    } else {
                        TUMBLEWEED_JUMPS[1]
                    };
                    npc.dirty = true;
                } else if blocking(world.tiles, probe_x, probe_y - 1) {
                    npc.velocity.1 = TUMBLEWEED_JUMPS[2];
                    npc.dirty = true;
                } else if npc.position.1 + npc.height() - (probe_y * 16) as f32 > 20.0
                    && blocking(world.tiles, probe_x, probe_y)
                {
                    npc.velocity.1 = TUMBLEWEED_JUMPS[3];
                    npc.dirty = true;
                } else if (npc.direction_y < 0 || npc.velocity.0.abs() > LEAP_SPEED)
                    && !blocking(world.tiles, probe_x, probe_y + 2)
                    && !blocking(world.tiles, probe_x + i32::from(npc.direction), probe_y + 3)
                {
                    // A gap ahead, and enough speed to clear it.
                    npc.velocity.1 = TUMBLEWEED_LEAP;
                    npc.dirty = true;
                }
            }
        }
    }

    npc.rotation += npc.velocity.0 * 0.05;
    npc.sprite_direction = -npc.direction;
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ai::Conditions;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Dunes(HashMap<(i32, i32), Tile>);

    impl TileView for Dunes {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn sand() -> Dunes {
        let mut d = Dunes::default();
        for x in 0..4000 {
            for y in 300..320 {
                d.0.insert((x, y), Tile::block(53));
            }
        }
        d
    }

    fn weed(tile_x: i32) -> Npc {
        let mut n = Npc::new(546, (0.0, 0.0), 1).expect("tumbleweed");
        n.position = (tile_x as f32 * TILE, 300.0 * TILE - n.height());
        n.old_position = (n.position.0 - 1.0, n.position.1);
        n
    }

    fn desert<'a>(tiles: &'a Dunes, target: Option<Target>) -> World<'a, Dunes> {
        World {
            conditions: Conditions {
                desert: true,
                ..Conditions::default()
            },
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
    fn a_tumbleweed_rolls_up_to_its_speed() {
        let tiles = sand();
        let mut w = weed(200);
        w.direction = 1;
        let (cx, cy) = w.center();
        let t = Some(player_at(cx + 600.0, cy));
        for _ in 0..400 {
            w.old_position = (w.position.0 - 1.0, w.position.1);
            update(&mut w, &desert(&tiles, t), true);
            w.velocity.1 = 0.0;
        }
        assert!(
            w.velocity.0 > 0.0,
            "should be rolling, got {}",
            w.velocity.0
        );
    }

    #[test]
    fn a_sandstorm_pushes_it_one_way_and_holds_it_the_other() {
        let tiles = sand();
        let mut downwind = weed(200);
        let mut upwind = weed(200);
        downwind.direction = 1;
        upwind.direction = -1;
        let mut w = desert(&tiles, None);
        w.conditions.wind = 1.0;
        for _ in 0..600 {
            downwind.old_position = (downwind.position.0 - 1.0, downwind.position.1);
            upwind.old_position = (upwind.position.0 - 1.0, upwind.position.1);
            downwind.ai[3] = TUMBLEWEED_PATIENCE;
            upwind.ai[3] = TUMBLEWEED_PATIENCE;
            update(&mut downwind, &w, true);
            update(&mut upwind, &w, true);
            downwind.velocity.1 = 0.0;
            upwind.velocity.1 = 0.0;
        }
        assert!(
            downwind.velocity.0.abs() > upwind.velocity.0.abs(),
            "downwind {} should outrun upwind {}",
            downwind.velocity.0,
            upwind.velocity.0
        );
    }

    #[test]
    fn a_tumbleweed_outside_the_desert_leaves() {
        let tiles = sand();
        let mut w = weed(200);
        let (cx, cy) = w.center();
        let mut out = desert(&tiles, Some(player_at(cx + 300.0, cy)));
        out.conditions.desert = false;
        update(&mut w, &out, false);
        assert!(w.time_left <= 10, "should be leaving, got {}", w.time_left);
    }

    #[test]
    fn a_tumbleweed_that_gets_nowhere_stops_chasing_and_reverses() {
        let tiles = sand();
        let mut w = weed(200);
        w.direction = 1;
        let (cx, cy) = w.center();
        // Someone to its right that it never makes progress toward.
        let t = Some(player_at(cx + 800.0, cy));
        for _ in 0..(TUMBLEWEED_PATIENCE as i32 + 2) {
            w.old_position = w.position;
            update(&mut w, &desert(&tiles, t), true);
            w.velocity = (0.0, 0.0);
        }
        assert!(
            w.ai[3] >= TUMBLEWEED_PATIENCE,
            "should have given up steering, got {}",
            w.ai[3]
        );
        // Standing still on the ground, it turns itself round.
        for _ in 0..3 {
            w.old_position = w.position;
            update(&mut w, &desert(&tiles, t), true);
            w.velocity = (0.0, 0.0);
        }
        assert_eq!(w.direction, -1);
    }

    /// It probes further ahead the faster it is rolling, and jumps higher the taller the thing it
    /// finds. A wall directly over its own head cancels the jump entirely, which is why the probe
    /// has to be out ahead of it for the tall jumps to be reachable at all.
    #[test]
    fn a_tumbleweed_jumps_a_wall_higher_the_taller_it_is() {
        let sample = weed(200);
        let speed = 4.0;
        let probe_x = ((sample.position.0
            + sample.width() / 2.0
            + (sample.width() / 2.0 + 2.0)
            + speed * 5.0)
            / TILE) as i32;
        let probe_y = ((sample.position.1 + sample.height() - 15.0) / TILE) as i32;

        let jump_over = |height: i32| {
            let mut w = weed(200);
            w.direction = 1;
            w.velocity = (speed, 0.0);
            let mut wall = sand();
            for up in 1..=height {
                wall.0.insert((probe_x, probe_y - up), Tile::block(1));
            }
            update(&mut w, &desert(&wall, None), true);
            w.velocity.1
        };

        let (short, tall, taller) = (jump_over(1), jump_over(2), jump_over(3));
        assert!(short < 0.0, "it should jump at all, got {short}");
        assert!(
            tall < short,
            "two tiles clears higher: {tall} against {short}"
        );
        assert!(
            taller < tall,
            "and three higher still: {taller} against {tall}"
        );
    }

    #[test]
    fn tumbleweeds_shoulder_each_other_apart() {
        let tiles = sand();
        let mut alone = weed(200);
        let mut crowded_weed = weed(200);
        alone.direction = 1;
        crowded_weed.direction = 1;
        let mut crowded = desert(&tiles, None);
        crowded.crowding = (-1.0, 0.0);
        update(&mut alone, &desert(&tiles, None), true);
        update(&mut crowded_weed, &crowded, true);
        assert!(
            crowded_weed.velocity.0 < alone.velocity.0,
            "a crowded one should be held back: {} against {}",
            crowded_weed.velocity.0,
            alone.velocity.0
        );
    }
}
