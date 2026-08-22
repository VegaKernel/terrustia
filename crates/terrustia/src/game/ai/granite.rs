//! Style 91 — the granite flyer.
//!
//! The only routine in the pre-hardmode roster that actually *routes*. Rather than pressing into
//! whatever wall is between it and you, it looks for a corner: a point sharing your x or its own y
//! that it can reach and that can reach you. Finding one, it flies there and looks again. Finding
//! none, it drifts, and every five ticks tries the corners once more.
//!
//! Five states live in `ai[0]`: **0** planning, **1** chasing in the open, **2** phasing through
//! terrain, **3** flying to a corner, **4** drifting. `ai[1..2]` hold the corner while it is in
//! state 3, and the state's own timer otherwise.

use terrustia_proto::npc_params::{
    GRANITE_CHASE_BASE, GRANITE_CHASE_RAMP, GRANITE_CHASE_SMOOTH, GRANITE_PHASE_SMOOTH,
    GRANITE_REPLAN_EVERY, GRANITE_ROUTE_SMOOTH, GRANITE_ROUTE_SPEED, GRANITE_WANDER_LIMIT,
    GRANITE_WANDER_SMOOTH, GRANITE_WANDER_SPEED, GRANITE_WAYPOINT_MAX, GRANITE_WAYPOINT_MIN,
};

use super::{PLAYER_HEIGHT, World, sight::can_hit, sight::solid_collision};
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Target;

/// How long a stunned flyer stays down.
const STUNNED: f32 = 120.0;

/// Whether a point-sized line runs clear between two points.
fn clear(tiles: &impl TileView, from: (f32, f32), to: (f32, f32)) -> bool {
    can_hit(tiles, from, (1, 1), to, (1, 1))
}

/// Ease a velocity toward a heading, the way every state here does.
fn glide(npc: &mut Npc, toward: (f32, f32), speed: f32, smoothing: f32) {
    let length = (toward.0 * toward.0 + toward.1 * toward.1).sqrt();
    if length == 0.0 {
        return;
    }
    let wanted = (toward.0 / length * speed, toward.1 / length * speed);
    npc.velocity.0 = (npc.velocity.0 * (smoothing - 1.0) + wanted.0) / smoothing;
    npc.velocity.1 = (npc.velocity.1 * (smoothing - 1.0) + wanted.1) / smoothing;
}

/// Look for a corner it can fly to that opens a line to the target.
///
/// Two candidates, both axis-aligned: straight across to the target's column, or straight down to
/// its row. It prefers whichever also has a clear line onward to the target itself.
fn find_corner<T: TileView>(npc: &Npc, world: &World<'_, T>, target: Target) -> Option<(f32, f32)> {
    let here = npc.center();
    let across = (target.center.0, here.1);
    let down = (here.0, target.center.1);
    let far_enough = |p: (f32, f32)| {
        ((p.0 - here.0).powi(2) + (p.1 - here.1).powi(2)).sqrt() > GRANITE_WAYPOINT_MIN
    };

    if far_enough(across) && clear(world.tiles, here, across) {
        // If the other corner works end to end, prefer it: it puts the flyer on the target's level.
        if far_enough(down)
            && clear(world.tiles, here, down)
            && clear(world.tiles, down, target.center)
        {
            return Some(down);
        }
        return Some(across);
    }
    if far_enough(down) && clear(world.tiles, here, down) {
        return Some(down);
    }
    None
}

/// Drive one granite flyer for a tick.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, staggered: bool) {
    npc.no_gravity = true;
    npc.no_tile_collide = false;

    // A hard hit occasionally drops it out of the air entirely.
    if staggered {
        npc.ai[0] = -1.0;
        npc.ai[1] = 0.0;
        npc.dirty = true;
    }

    let Some(target) = world.target else {
        npc.velocity.0 *= 0.98;
        npc.dirty = true;
        return;
    };
    let here = npc.center();
    let visible = clear(world.tiles, here, target.center);

    match npc.ai[0] as i32 {
        -1 => {
            // Stunned: it falls, and cannot be hurt while it is down.
            npc.no_gravity = false;
            npc.velocity.0 *= 0.98;
            npc.ai[1] += 1.0;
            if npc.ai[1] >= STUNNED {
                npc.ai = [0.0; 4];
                npc.dirty = true;
            }
        }
        0 => {
            // Planning. A clear line means simply chase.
            if visible {
                npc.ai[0] = 1.0;
                npc.dirty = true;
                return;
            }
            if let Some(corner) = find_corner(npc, world, target) {
                npc.ai[0] = 3.0;
                npc.ai[1] = corner.0;
                npc.ai[2] = corner.1;
            } else {
                // Nowhere to route: nudge toward the target and drift.
                let (dx, dy) = (
                    target.center.0 - here.0,
                    target.center.1 - PLAYER_HEIGHT as f32 / 4.0 - here.1,
                );
                let length = (dx * dx + dy * dy).sqrt();
                if length > 0.0 {
                    npc.velocity.0 += dx / length * 0.5;
                    npc.velocity.1 += dy / length * 0.5;
                }
                npc.ai[0] = 4.0;
                npc.ai[1] = 0.0;
            }
            npc.dirty = true;
        }
        1 => {
            // Chasing in the open, and the further off you are the faster it comes.
            let (dx, dy) = (target.center.0 - here.0, target.center.1 - here.1);
            let reach = (dx * dx + dy * dy).sqrt();
            glide(
                npc,
                (dx, dy),
                GRANITE_CHASE_BASE + reach / GRANITE_CHASE_RAMP,
                GRANITE_CHASE_SMOOTH,
            );
            if !visible {
                npc.ai[0] = 0.0;
                npc.ai[1] = 0.0;
                npc.dirty = true;
            }
        }
        2 => {
            // Phasing straight through the rock, until it is close and back in open space.
            npc.no_tile_collide = true;
            let (dx, dy) = (target.center.0 - here.0, target.center.1 - here.1);
            let reach = (dx * dx + dy * dy).sqrt();
            glide(npc, (dx, dy), GRANITE_CHASE_BASE, GRANITE_PHASE_SMOOTH);
            if reach < 600.0
                && !solid_collision(
                    world.tiles,
                    npc.position,
                    (npc.stats.width, npc.stats.height),
                )
            {
                npc.ai[0] = 0.0;
                npc.dirty = true;
            }
        }
        3 => {
            // Flying to the corner it picked.
            let corner = (npc.ai[1], npc.ai[2]);
            let (dx, dy) = (corner.0 - here.0, corner.1 - here.1);
            let reach = (dx * dx + dy * dy).sqrt();
            glide(npc, (dx, dy), GRANITE_ROUTE_SPEED, GRANITE_ROUTE_SMOOTH);
            if npc.collide_x || npc.collide_y {
                npc.ai[0] = 4.0;
                npc.ai[1] = 0.0;
                npc.dirty = true;
            }
            // Arrived, overshot, or the line opened up anyway: plan again.
            if !(GRANITE_ROUTE_SPEED..=GRANITE_WAYPOINT_MAX).contains(&reach) || visible {
                npc.ai[0] = 0.0;
                npc.dirty = true;
            }
        }
        _ => {
            // Drifting, and bouncing off whatever it meets.
            if npc.collide_x {
                npc.velocity.0 *= -0.8;
            }
            if npc.collide_y {
                npc.velocity.1 *= -0.8;
            }
            if npc.velocity.0 == 0.0 && npc.velocity.1 == 0.0 {
                let (dx, dy) = (
                    target.center.0 - here.0,
                    target.center.1 - PLAYER_HEIGHT as f32 / 4.0 - here.1,
                );
                let length = (dx * dx + dy * dy).sqrt();
                if length > 0.0 {
                    npc.velocity = (dx / length * 0.1, dy / length * 0.1);
                }
            }
            let heading = npc.velocity;
            glide(npc, heading, GRANITE_WANDER_SPEED, GRANITE_WANDER_SMOOTH);
            npc.ai[1] += 1.0;
            if npc.ai[1] > GRANITE_WANDER_LIMIT {
                npc.ai[0] = 0.0;
                npc.ai[1] = 0.0;
                npc.dirty = true;
            }
            if visible {
                npc.ai[0] = 0.0;
                npc.dirty = true;
                return;
            }
            // Every few ticks it tries the corners again, so long as it is out in the open.
            npc.local_ai[0] += 1.0;
            if npc.local_ai[0] >= GRANITE_REPLAN_EVERY {
                npc.local_ai[0] = 0.0;
                let padded = (npc.position.0 - 10.0, npc.position.1 - 10.0);
                let embedded = solid_collision(
                    world.tiles,
                    padded,
                    (npc.stats.width + 20, npc.stats.height + 20),
                );
                if !embedded && let Some(corner) = find_corner(npc, world, target) {
                    npc.ai[0] = 3.0;
                    npc.ai[1] = corner.0;
                    npc.ai[2] = corner.1;
                    npc.dirty = true;
                }
            }
        }
    }
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc::TILE;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Cave(HashMap<(i32, i32), Tile>);

    impl TileView for Cave {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn flyer(tile_x: i32, tile_y: i32) -> Npc {
        Npc::new(483, (tile_x as f32 * TILE, tile_y as f32 * TILE), 1).expect("granite flyer")
    }

    fn world<'a>(tiles: &'a Cave, target: Option<Target>) -> World<'a, Cave> {
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
    fn a_clear_line_puts_it_straight_into_the_chase() {
        let tiles = Cave::default();
        let mut f = flyer(500, 500);
        let (cx, cy) = f.center();
        update(
            &mut f,
            &world(&tiles, Some(player_at(cx + 400.0, cy))),
            false,
        );
        assert_eq!(f.ai[0], 1.0);
    }

    #[test]
    fn it_closes_faster_the_further_away_you_are() {
        let tiles = Cave::default();
        let (near_speed, far_speed) = [200.0f32, 2000.0]
            .map(|gap| {
                let mut f = flyer(500, 500);
                f.ai[0] = 1.0;
                let (cx, cy) = f.center();
                let t = Some(player_at(cx + gap, cy));
                for _ in 0..400 {
                    update(&mut f, &world(&tiles, t), false);
                }
                (f.velocity.0.powi(2) + f.velocity.1.powi(2)).sqrt()
            })
            .into();
        assert!(
            far_speed > near_speed + 1.0,
            "distance should ramp it up: {near_speed} against {far_speed}"
        );
    }

    #[test]
    fn a_wall_makes_it_route_round_rather_than_press_into_it() {
        let mut tiles = Cave::default();
        // A wall between them, with a gap well below.
        for y in 400..520 {
            tiles.0.insert((510, y), Tile::block(1));
        }
        let mut f = flyer(500, 500);
        let (cx, cy) = f.center();
        // Player beyond the wall and below its bottom edge, so a corner exists.
        let t = Some(player_at(cx + 400.0, cy + 500.0));
        update(&mut f, &world(&tiles, t), false);
        assert_eq!(f.ai[0], 3.0, "should have picked a corner to fly to");
        assert!(f.ai[1] != 0.0 || f.ai[2] != 0.0, "and recorded where it is");
    }

    #[test]
    fn with_no_route_at_all_it_drifts() {
        let mut tiles = Cave::default();
        // Boxed in completely.
        for x in 495..510 {
            for y in 495..510 {
                if !(497..=503).contains(&x) || !(497..=503).contains(&y) {
                    tiles.0.insert((x, y), Tile::block(1));
                }
            }
        }
        let mut f = flyer(500, 500);
        let (cx, cy) = f.center();
        update(
            &mut f,
            &world(&tiles, Some(player_at(cx + 4000.0, cy + 4000.0))),
            false,
        );
        assert_eq!(f.ai[0], 4.0, "nothing to route to, so it drifts");
    }

    #[test]
    fn losing_the_line_drops_it_out_of_the_chase() {
        let mut tiles = Cave::default();
        let mut f = flyer(500, 500);
        f.ai[0] = 1.0;
        let (cx, cy) = f.center();
        let t = Some(player_at(cx + 400.0, cy));
        update(&mut f, &world(&tiles, t), false);
        assert_eq!(f.ai[0], 1.0, "still chasing");
        for y in 400..600 {
            tiles.0.insert((510, y), Tile::block(1));
        }
        update(&mut f, &world(&tiles, t), false);
        assert_eq!(f.ai[0], 0.0, "should go back to planning");
    }

    #[test]
    fn a_hard_hit_drops_it_out_of_the_air() {
        let tiles = Cave::default();
        let mut f = flyer(500, 500);
        let (cx, cy) = f.center();
        let t = Some(player_at(cx + 400.0, cy));
        update(&mut f, &world(&tiles, t), true);
        assert_eq!(f.ai[0], -1.0);
        assert!(!f.no_gravity, "and it falls");

        for _ in 0..(STUNNED as i32 + 1) {
            update(&mut f, &world(&tiles, t), false);
        }
        assert!(f.ai[0] >= 0.0, "then picks itself up, got {}", f.ai[0]);
        assert!(f.no_gravity, "and flies again");
    }

    /// A drifting flyer never settles: it re-plans on a timer and re-checks for corners every few
    /// ticks, so it always leaves state 4 rather than getting wedged in it.
    #[test]
    fn a_drifting_flyer_never_stays_drifting() {
        let mut tiles = Cave::default();
        for x in 400..700 {
            for y in 400..700 {
                if !(497..=503).contains(&x) || !(497..=503).contains(&y) {
                    tiles.0.insert((x, y), Tile::block(1));
                }
            }
        }
        let mut f = flyer(500, 500);
        f.ai[0] = 4.0;
        let (cx, cy) = f.center();
        let t = Some(player_at(cx + 4000.0, cy));
        let mut left_it = false;
        for _ in 0..(GRANITE_WANDER_LIMIT as i32 + 5) {
            update(&mut f, &world(&tiles, t), false);
            if f.ai[0] != 4.0 {
                left_it = true;
            }
        }
        assert!(left_it, "should have tried something else by now");
    }
}
