//! Hunters that route around terrain: styles 85 and 90.
//!
//! Style 85 — the deadly sphere, the nebula headcrab and the big stardust cell — is the only
//! routine in the game that genuinely pathfinds, and it does it with two waypoints rather than a
//! search. With a clear line it simply charges. Without one it tries the corner directly above or
//! beside you: if it can reach that corner and the corner can reach you, it goes there and then
//! charges from it. If neither corner works it drifts, bouncing off walls, and re-checks the
//! corners five times a second until one opens up. Only when you are more than eight hundred
//! pixels away does it give up on terrain altogether and fly straight through it.
//!
//! The nebula headcrab adds one thing to that: within forty pixels it stops attacking and *sits on
//! your head*, and while it is there nothing else of its kind will try, so you are dealing with one
//! at a time.
//!
//! Style 90 — a Mothron's spawn — is simpler and nastier. It circles for a second and a half, then
//! spends ten ticks lining you up and pounces, accelerating the whole way. Out of an eclipse it
//! does not fight at all: it climbs out of the world and is gone.

use terrustia_proto::npc_params::{
    CELL_CHASE, CELL_CORNER_SPEED, CELL_DRIFT_SPEED, CELL_PHASE_SPEED, CHASE_DISTANCE_GAIN,
    CHASE_SMOOTH, HEADCRAB, HEADCRAB_LATCH, PATH_CORNER_SMOOTH, PATH_CORNER_SPEED,
    PATH_DRIFT_BOUNCE, PATH_DRIFT_SMOOTH, PATH_DRIFT_SPEED, PATH_GIVE_UP, PATH_LOOK_EVERY,
    PATH_LOST_TICKS, PATH_PHASE_SMOOTH, PATH_PHASE_SPEED, PATH_RESURFACE, PATH_SHOVE,
    PATH_WAYPOINT_MIN, SPAWN_AIM_TICKS, SPAWN_CHASE, SPAWN_CHASE_GAIN, SPAWN_CHASE_SMOOTH,
    SPAWN_CIRCLE_RANGE, SPAWN_CIRCLE_TICKS, SPAWN_DESPAWN_RANGE, SPAWN_FAR, SPAWN_LEAVE_CAP,
    SPAWN_LEAVE_CLIMB, SPAWN_LOSE, SPAWN_PHASE_ACCEL, SPAWN_PHASE_GAIN, SPAWN_PHASE_SMOOTH,
    SPAWN_POUNCE, SPAWN_POUNCE_GAIN, SPAWN_POUNCE_SMOOTH, SPAWN_POUNCE_TICKS, SPAWN_REACQUIRE,
    STARDUST_CELL_BIG,
};

use super::drifters::Outcome;
use crate::game::ai::{PLAYER_HEIGHT, World, sight, unit};
use crate::game::npc::{Npc, TileView};

/// The phases of style 85, as `ai[0]` numbers them.
mod path {
    pub const DECIDING: f32 = 0.0;
    pub const CHARGING: f32 = 1.0;
    pub const PHASING: f32 = 2.0;
    pub const ROUNDING: f32 = 3.0;
    pub const LOST: f32 = 4.0;
    pub const LATCHED: f32 = 5.0;
}

/// Whether there is a clear line between two points.
fn clear(tiles: &impl TileView, from: (f32, f32), to: (f32, f32)) -> bool {
    sight::can_hit(tiles, from, (1, 1), to, (1, 1))
}

/// Style 85.
///
/// `target_taken` says whether another of this type is already sitting on the player, which is
/// what stops a swarm of headcrabs all latching at once.
pub fn pathfinder(npc: &mut Npc, world: &World<'_, impl TileView>, target_taken: bool) -> Outcome {
    let out = Outcome::default();
    npc.dirty = true;
    npc.no_tile_collide = false;

    let cell = npc.npc_type == STARDUST_CELL_BIG;
    let Some(target) = world.target.filter(|t| t.alive) else {
        return out;
    };
    let (cx, cy) = npc.center();
    let to_player = (target.center.0 - cx, target.center.1 - cy);

    match npc.ai[0] {
        p if p == path::DECIDING => {
            if clear(world.tiles, npc.center(), target.center) {
                npc.ai[0] = path::CHARGING;
            } else {
                // Aim a little above the player's middle, the way the game does.
                let aim = (to_player.0, to_player.1 - PLAYER_HEIGHT as f32 / 4.0);
                if aim.0.hypot(aim.1) > PATH_GIVE_UP {
                    npc.ai[0] = path::PHASING;
                } else if let Some((wx, wy)) = corner(world.tiles, npc.center(), target.center) {
                    npc.ai[0] = path::ROUNDING;
                    npc.ai[1] = wx;
                    npc.ai[2] = wy;
                } else {
                    // Neither corner works. Nudge toward the player and start drifting.
                    npc.local_ai[0] = 0.0;
                    let nudge = unit(aim, 0.5);
                    npc.velocity.0 += nudge.0;
                    npc.velocity.1 += nudge.1;
                    npc.ai[0] = path::LOST;
                    npc.ai[1] = 0.0;
                }
            }
        }

        p if p == path::CHARGING => {
            npc.rotation += f32::from(npc.direction) * 0.3;
            // A headcrab aims at the top of your head rather than your middle.
            let aim = if npc.npc_type == HEADCRAB {
                (
                    target.center.0 - cx,
                    target.center.1 - PLAYER_HEIGHT as f32 / 2.0 - cy,
                )
            } else {
                to_player
            };
            let reach = aim.0.hypot(aim.1);
            let speed =
                (if cell { CELL_CHASE } else { SPHERE_CHASE }) + reach / CHASE_DISTANCE_GAIN;
            ease(&mut npc.velocity, unit(aim, speed), CHASE_SMOOTH);

            if !clear(world.tiles, npc.center(), target.center) {
                npc.ai[0] = path::DECIDING;
                npc.ai[1] = 0.0;
            }
            // Close enough to latch, and nobody else is already there.
            if npc.npc_type == HEADCRAB && reach < HEADCRAB_LATCH && !target_taken {
                npc.velocity = (0.0, 0.0);
                npc.ai[0] = path::LATCHED;
                npc.ai[1] = 0.0;
            }
        }

        p if p == path::PHASING => {
            npc.rotation = npc.velocity.0 * 0.1;
            npc.no_tile_collide = true;
            let reach = to_player.0.hypot(to_player.1);
            let speed = if cell {
                CELL_PHASE_SPEED
            } else {
                PATH_PHASE_SPEED
            };
            ease(&mut npc.velocity, unit(to_player, speed), PATH_PHASE_SMOOTH);
            // It only comes back out of the rock once it is close and standing in clear air.
            if reach < PATH_RESURFACE && !inside_terrain(world.tiles, npc) {
                npc.ai[0] = path::DECIDING;
            }
        }

        p if p == path::ROUNDING => {
            npc.rotation = npc.velocity.0 * 0.1;
            let waypoint = (npc.ai[1] - cx, npc.ai[2] - cy);
            let gap = waypoint.0.hypot(waypoint.1);
            let speed = if cell {
                CELL_CORNER_SPEED
            } else {
                PATH_CORNER_SPEED
            };
            ease(&mut npc.velocity, unit(waypoint, speed), PATH_CORNER_SMOOTH);
            if npc.collide_x || npc.collide_y {
                npc.ai[0] = path::LOST;
                npc.ai[1] = 0.0;
            }
            // Arrived, overshot, or the direct line opened up on the way.
            if gap < speed || gap > PATH_GIVE_UP || clear(world.tiles, npc.center(), target.center)
            {
                npc.ai[0] = path::DECIDING;
            }
        }

        p if p == path::LOST => {
            npc.rotation = npc.velocity.0 * 0.1;
            if npc.collide_x {
                npc.velocity.0 *= PATH_DRIFT_BOUNCE;
            }
            if npc.collide_y {
                npc.velocity.1 *= PATH_DRIFT_BOUNCE;
            }
            if npc.velocity == (0.0, 0.0) {
                let aim = (to_player.0, to_player.1 - PLAYER_HEIGHT as f32 / 4.0);
                npc.velocity = unit(aim, 0.1);
            }
            let speed = if cell {
                CELL_DRIFT_SPEED
            } else {
                PATH_DRIFT_SPEED
            };
            // It keeps whatever heading it has and eases its *speed* toward the drift speed.
            let heading = unit(npc.velocity, speed);
            ease(&mut npc.velocity, heading, PATH_DRIFT_SMOOTH);

            npc.ai[1] += 1.0;
            if npc.ai[1] > PATH_LOST_TICKS {
                npc.ai[0] = path::DECIDING;
                npc.ai[1] = 0.0;
            }
            if clear(world.tiles, npc.center(), target.center) {
                npc.ai[0] = path::DECIDING;
            }
            // Five times a second it looks for a corner it could round.
            npc.local_ai[0] += 1.0;
            if npc.local_ai[0] >= PATH_LOOK_EVERY && !boxed_in(world.tiles, npc) {
                npc.local_ai[0] = 0.0;
                if let Some((wx, wy)) = corner(world.tiles, npc.center(), target.center) {
                    npc.ai[0] = path::ROUNDING;
                    npc.ai[1] = wx;
                    npc.ai[2] = wy;
                }
            }
        }

        _ => {
            // Latched. It rides on the player's head and does nothing else.
            npc.position = (
                target.center.0 - npc.width() / 2.0,
                target.center.1 - PLAYER_HEIGHT as f32 / 2.0 - npc.height() / 2.0,
            );
            npc.velocity = (0.0, 0.0);
        }
    }

    // The cell and the headcrab shove their own kind apart rather than stacking into one blob.
    if cell || npc.npc_type == HEADCRAB {
        npc.rotation = if cell { 0.0 } else { npc.velocity.0 * 0.1 };
        for (kx, ky, _) in world.avoid {
            if (npc.position.0 - kx).abs() + (npc.position.1 - ky).abs() < npc.width() {
                npc.velocity.0 += if npc.position.0 < *kx {
                    -PATH_SHOVE
                } else {
                    PATH_SHOVE
                };
                npc.velocity.1 += if npc.position.1 < *ky {
                    -PATH_SHOVE
                } else {
                    PATH_SHOVE
                };
            }
        }
    }
    out
}

/// The corner to round: the point level with this NPC and above the player, or vice versa.
///
/// Both have to be reachable from here *and* have a clear line onward to the player, which is what
/// makes this a two-leg route rather than a guess.
fn corner(tiles: &impl TileView, from: (f32, f32), player: (f32, f32)) -> Option<(f32, f32)> {
    // Straight across, then down.
    let across = (player.0, from.1);
    if (across.0 - from.0).abs() > PATH_WAYPOINT_MIN
        && clear(tiles, from, across)
        && clear(tiles, across, player)
    {
        return Some(across);
    }
    // Straight down, then across.
    let down = (from.0, player.1);
    if (down.1 - from.1).abs() > PATH_WAYPOINT_MIN
        && clear(tiles, from, down)
        && clear(tiles, down, player)
    {
        return Some(down);
    }
    None
}

/// Whether the NPC's own box overlaps solid tiles, which is how it knows it is still inside rock.
fn inside_terrain(tiles: &impl TileView, npc: &Npc) -> bool {
    boxed(tiles, npc.position, (npc.width(), npc.height()))
}

/// Whether the NPC has ten pixels of clearance all round; used before looking for a corner, so it
/// does not try to route while it is wedged.
fn boxed_in(tiles: &impl TileView, npc: &Npc) -> bool {
    boxed(
        tiles,
        (npc.position.0 - 10.0, npc.position.1 - 10.0),
        (npc.width() + 20.0, npc.height() + 20.0),
    )
}

fn boxed(tiles: &impl TileView, position: (f32, f32), size: (f32, f32)) -> bool {
    let tile = crate::game::npc::TILE;
    let x0 = (position.0 / tile).floor() as i32;
    let x1 = ((position.0 + size.0 - 1.0) / tile).floor() as i32;
    let y0 = (position.1 / tile).floor() as i32;
    let y1 = ((position.1 + size.1 - 1.0) / tile).floor() as i32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let t = tiles.tile(x, y);
            if t.is_active() && terrustia_proto::tile_solid::solid(t.block) {
                return true;
            }
        }
    }
    false
}

/// Style 90: a Mothron's spawn.
pub fn mothron_spawn(npc: &mut Npc, world: &World<'_, impl TileView>) -> Outcome {
    let out = Outcome::default();
    npc.dirty = true;
    npc.no_tile_collide = false;
    npc.no_gravity = true;
    npc.rotation = (npc.rotation * 9.0 + npc.velocity.0 * 0.1) / 10.0;

    // These exist only during an eclipse. Without one there is nothing to do but leave.
    if !world.conditions.eclipse {
        npc.velocity.1 = (npc.velocity.1 + SPAWN_LEAVE_CLIMB).max(SPAWN_LEAVE_CAP);
        npc.no_tile_collide = true;
        npc.time_left = npc.time_left.min(60);
        return out;
    }

    // While circling or crossing, they keep out of each other's way.
    if npc.ai[0] == 0.0 || npc.ai[0] == 1.0 {
        let (cx, cy) = npc.center();
        for (kx, ky, _) in world.avoid {
            let (dx, dy) = (kx - cx, ky - cy);
            let gap = dx.hypot(dy);
            if gap > 0.0 && gap < npc.width() + npc.height() {
                npc.velocity.0 -= dx / gap * 0.1;
                npc.velocity.1 -= dy / gap * 0.1;
            }
        }
    }

    let Some(target) = world.target.filter(|t| t.alive) else {
        npc.ai[0] = -1.0;
        return out;
    };
    let (cx, cy) = npc.center();
    let to_player = (target.center.0 - cx, target.center.1 - cy);
    let reach = to_player.0.hypot(to_player.1);
    if reach > SPAWN_DESPAWN_RANGE {
        npc.ai[0] = -1.0;
    } else if npc.ai[0] > 1.0 && reach > SPAWN_LOSE {
        // Lost you mid-pounce: back to crossing the map.
        npc.ai[0] = 1.0;
    }

    match npc.ai[0] {
        p if p < 0.0 => {
            // Leaving.
            ease(&mut npc.velocity, (0.0, -8.0), 10.0);
            npc.no_tile_collide = true;
            npc.invulnerable = true;
        }

        0.0 => {
            // Circling. It closes to two hundred pixels and then holds station, winding up.
            npc.sprite_direction = npc.direction;
            if npc.collide_x {
                npc.velocity.0 = (-npc.old_velocity.0 * 0.5).clamp(-4.0, 4.0);
            }
            if npc.collide_y {
                npc.velocity.1 = (-npc.old_velocity.1 * 0.5).clamp(-4.0, 4.0);
            }
            if reach > SPAWN_FAR {
                npc.ai = [1.0, 0.0, 0.0, 0.0];
            } else if reach > SPAWN_CIRCLE_RANGE {
                // The longer it circles the faster it moves, which is what makes the pounce feel
                // like the end of a build-up rather than a surprise.
                let speed = SPAWN_CHASE + reach / SPAWN_CHASE_GAIN + npc.ai[1] / 15.0;
                ease(
                    &mut npc.velocity,
                    unit(to_player, speed),
                    SPAWN_CHASE_SMOOTH,
                );
            } else {
                let speed = npc.velocity.0.hypot(npc.velocity.1);
                let scale = if speed > 2.0 {
                    0.95
                } else if speed < 1.0 {
                    1.05
                } else {
                    1.0
                };
                npc.velocity.0 *= scale;
                npc.velocity.1 *= scale;
            }
            npc.ai[1] += 1.0;
            if npc.ai[1] >= SPAWN_CIRCLE_TICKS {
                npc.ai[1] = 0.0;
                npc.ai[0] = 2.0;
            }
        }

        1.0 => {
            // Crossing the map, through anything in the way, gaining speed as it comes.
            npc.no_tile_collide = true;
            npc.knockback_immune = true;
            npc.direction = match npc.velocity.0 {
                v if v < 0.0 => -1,
                v if v > 0.0 => 1,
                _ => npc.direction,
            };
            npc.sprite_direction = npc.direction;
            npc.rotation = (npc.rotation * 9.0 + npc.velocity.0 * 0.08) / 10.0;
            if reach < SPAWN_REACQUIRE && !inside_terrain(world.tiles, npc) {
                npc.ai = [0.0, 0.0, 0.0, 0.0];
            }
            npc.ai[2] += SPAWN_PHASE_ACCEL;
            let speed = SPAWN_CHASE + npc.ai[2] + reach / SPAWN_PHASE_GAIN;
            ease(
                &mut npc.velocity,
                unit(to_player, speed),
                SPAWN_PHASE_SMOOTH,
            );
        }

        2.0 => {
            // Lining up. Ten ticks, then it commits to whatever line it has.
            npc.knockback_immune = true;
            npc.no_tile_collide = true;
            let aim = (to_player.0, to_player.1 - 8.0);
            let wanted = unit(aim, SPAWN_POUNCE);
            ease(&mut npc.velocity, wanted, SPAWN_POUNCE_SMOOTH);
            npc.direction = if npc.velocity.0 < 0.0 { -1 } else { 1 };
            npc.sprite_direction = npc.direction;
            npc.rotation = (npc.rotation * 7.0 + npc.velocity.0 * 0.1) / 8.0;
            npc.ai[1] += 1.0;
            if npc.ai[1] > SPAWN_AIM_TICKS {
                npc.velocity = wanted;
                npc.direction = if npc.velocity.0 < 0.0 { -1 } else { 1 };
                npc.ai[0] = 2.1;
                npc.ai[1] = 0.0;
            }
        }

        _ => {
            // Pouncing. It accelerates the whole way and does not steer.
            npc.knockback_immune = true;
            npc.no_tile_collide = true;
            npc.direction = match npc.velocity.0 {
                v if v < 0.0 => -1,
                v if v > 0.0 => 1,
                _ => npc.direction,
            };
            npc.sprite_direction = npc.direction;
            npc.velocity.0 *= SPAWN_POUNCE_GAIN;
            npc.velocity.1 *= SPAWN_POUNCE_GAIN;
            npc.ai[1] += 1.0;
            if npc.ai[1] > SPAWN_POUNCE_TICKS {
                if !inside_terrain(world.tiles, npc) {
                    npc.ai = [0.0, 0.0, 0.0, npc.ai[3]];
                } else if npc.ai[1] > SPAWN_POUNCE_TICKS * 2.0 {
                    // Buried. Cross the map again rather than thrash inside the rock.
                    npc.ai = [1.0, 0.0, 0.0, npc.ai[3]];
                }
            }
        }
    }
    out
}

/// Ease a velocity toward a wanted one over `smooth` ticks.
fn ease(velocity: &mut (f32, f32), wanted: (f32, f32), smooth: f32) {
    velocity.0 = (velocity.0 * (smooth - 1.0) + wanted.0) / smooth;
    velocity.1 = (velocity.1 * (smooth - 1.0) + wanted.1) / smooth;
}

/// The deadly sphere's chase speed, which is the base the other two vary from.
const SPHERE_CHASE: f32 = terrustia_proto::npc_params::SPHERE_CHASE;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ai::Conditions;
    use crate::game::npc::TILE;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Cave(HashMap<(i32, i32), Tile>);

    impl TileView for Cave {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn open() -> Cave {
        Cave(HashMap::new())
    }

    /// A wall from the ceiling down to `gap_top`, leaving a way round underneath.
    fn wall_with_gap(column: i32, gap_top: i32) -> Cave {
        let mut tiles = HashMap::new();
        for y in -60..gap_top {
            tiles.insert((column, y), Tile::block(1));
        }
        Cave(tiles)
    }

    fn world<'a>(tiles: &'a Cave, target: Option<(f32, f32)>) -> World<'a, Cave> {
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

    const DEADLY_SPHERE: u16 = 522;
    /// Taken from the shared table rather than typed again here. It was typed again here, as 491,
    /// and 491 is `PirateShip`: the Flying Dutchman's hull, a different creature with different
    /// stats and a different AI style. Both Mothron-spawn tests below were driving this routine
    /// with a pirate ship and asserting on what came out, so neither could ever have caught a
    /// Mothron regression. This is the same mistake `npc_params.rs:3352` records against itself,
    /// where two ids were 470/471 and an eclipse had Mothron laying Crimson Penguins.
    use terrustia_proto::npc_params::MOTHRON_SPAWN;

    fn sphere(x: f32, y: f32) -> Npc {
        Npc::new(DEADLY_SPHERE, (x, y), 1).expect("deadly sphere")
    }

    /// With a clear line it simply charges, and faster the further away you are.
    #[test]
    fn a_clear_line_means_a_straight_charge() {
        let tiles = open();
        let mut near = sphere(0.0, 0.0);
        let mut far = sphere(0.0, 0.0);
        let close = world(&tiles, Some((200.0, 0.0)));
        let distant = world(&tiles, Some((700.0, 0.0)));

        for _ in 0..60 {
            pathfinder(&mut near, &close, false);
            pathfinder(&mut far, &distant, false);
        }
        assert_eq!(near.ai[0], path::CHARGING);
        assert!(
            far.velocity.0 > near.velocity.0,
            "distance should make it faster: {} vs {}",
            far.velocity.0,
            near.velocity.0
        );
    }

    /// A wall makes it route round rather than grind into it.
    #[test]
    fn a_blocked_hunter_routes_round_the_corner() {
        // A wall between them with a gap low down.
        let tiles = wall_with_gap(10, 20);
        let mut s = sphere(0.0, 0.0);
        let w = world(&tiles, Some((40.0 * TILE, 30.0 * TILE)));

        let mut rounded = false;
        for _ in 0..200 {
            pathfinder(&mut s, &w, false);
            if s.ai[0] == path::ROUNDING {
                rounded = true;
                break;
            }
        }
        assert!(rounded, "it should have found a way round");
        // The waypoint is a real place, not the player's position.
        assert!(s.ai[1] != 0.0 || s.ai[2] != 0.0);
    }

    /// From a long way off it gives up on terrain and comes straight through it.
    #[test]
    fn a_distant_hunter_flies_through_the_rock() {
        let tiles = wall_with_gap(10, 60);
        let mut s = sphere(0.0, 0.0);
        let w = world(&tiles, Some((900.0, 0.0)));
        pathfinder(&mut s, &w, false);
        assert_eq!(s.ai[0], path::PHASING, "too far to bother going round");
        pathfinder(&mut s, &w, false);
        assert!(s.no_tile_collide, "and it passes through the rock");
    }

    /// A headcrab sits on your head, but only one at a time.
    #[test]
    fn only_one_headcrab_latches_on() {
        let tiles = open();
        let mut first = Npc::new(HEADCRAB, (0.0, 0.0), 1).expect("nebula headcrab");
        let (cx, cy) = first.center();
        let w = world(&tiles, Some((cx + 10.0, cy + 10.0)));

        first.ai[0] = path::CHARGING;
        pathfinder(&mut first, &w, false);
        assert_eq!(first.ai[0], path::LATCHED, "it should have latched on");

        let mut second = Npc::new(HEADCRAB, (0.0, 0.0), 1).unwrap();
        second.ai[0] = path::CHARGING;
        pathfinder(&mut second, &w, true);
        assert_ne!(second.ai[0], path::LATCHED, "your head is taken");
    }

    /// Out of an eclipse a Mothron spawn does not fight; it climbs away.
    #[test]
    fn a_mothron_spawn_leaves_when_the_eclipse_ends() {
        let tiles = open();
        let mut s = Npc::new(MOTHRON_SPAWN, (0.0, 0.0), 1).expect("mothron spawn");
        let w = world(&tiles, Some((200.0, 0.0)));
        for _ in 0..60 {
            mothron_spawn(&mut s, &w);
        }
        assert!(s.velocity.1 < 0.0, "it should be climbing out");
        assert!(s.time_left <= 60, "and on its way out");
    }

    /// During an eclipse it circles, lines up, and pounces.
    #[test]
    fn a_mothron_spawn_circles_then_pounces() {
        let tiles = open();
        let mut s = Npc::new(MOTHRON_SPAWN, (0.0, 0.0), 1).expect("mothron spawn");
        let mut w = world(&tiles, Some((300.0, 0.0)));
        w.conditions = Conditions {
            eclipse: true,
            ..Conditions::default()
        };

        let mut phases = vec![s.ai[0]];
        for _ in 0..400 {
            mothron_spawn(&mut s, &w);
            if phases.last() != Some(&s.ai[0]) {
                phases.push(s.ai[0]);
            }
        }
        assert!(phases.contains(&2.0), "it should line up: {phases:?}");
        assert!(phases.contains(&2.1), "and then pounce: {phases:?}");
    }
}
