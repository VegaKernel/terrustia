//! Style 13 — the rooted plants, and style 17, the perchers.
//!
//! A **plant** (13) is tethered to the tile it grew from: it lunges toward whoever comes near,
//! never further than its reach, and dies the moment that tile is mined out from under it. Its
//! reach is not constant — over a 450-tick cycle the last third stretches it by 30%, which is the
//! slow breathing motion a man eater makes when nobody is in range.
//!
//! A **vulture** (17) sits on the sand with gravity on until something disturbs it, then kicks
//! itself into the air and circles, preferring to hang a hundred pixels above its target when it
//! is not already on top of it.

use terrustia_proto::npc_params::{
    PERCH_LAUNCH, PERCH_STARTLE, ROOTED_CYCLE, ROOTED_STRETCH, ROOTED_STRETCH_AT, VULTURE_CEILING,
    VULTURE_CLIMB_AT, rooted as rooted_params,
};

use super::{World, bounce, face, rise_out_of_water};
use crate::game::npc::{Npc, TILE, TileView};

/// What a plant's tick concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Alive,
    /// The tile it grew from is gone, so it is too.
    Uprooted,
}

/// Drive one rooted plant for a tick.
pub fn plant<T: TileView>(npc: &mut Npc, world: &World<'_, T>) -> Outcome {
    // `ai[0..1]` hold the anchor tile. The game's world generator writes it when it places the
    // plant; anything spawned without one takes root where it stands.
    if npc.ai[0] == 0.0 && npc.ai[1] == 0.0 {
        let (cx, cy) = npc.center();
        npc.ai[0] = (cx / TILE).floor();
        npc.ai[1] = (cy / TILE).floor();
    }
    let (anchor_x, anchor_y) = (npc.ai[0] as i32, npc.ai[1] as i32);
    if !world.tiles.tile(anchor_x, anchor_y).is_active() {
        return Outcome::Uprooted;
    }

    if let Some(t) = world.target {
        face(npc, t);
    }
    let params = rooted_params(npc.npc_type);

    // The stretch cycle: for the last third of it the plant reaches half again as far.
    npc.ai[2] += 1.0;
    let mut reach = params.reach;
    if npc.ai[2] > ROOTED_STRETCH_AT {
        reach = (f64::from(reach) * f64::from(ROOTED_STRETCH)) as i32 as f32;
        if npc.ai[2] > ROOTED_CYCLE {
            npc.ai[2] = 0.0;
        }
    }

    let root = ((anchor_x * 16 + 8) as f32, (anchor_y * 16 + 8) as f32);
    let (mut dx, mut dy) = match world.target {
        Some(t) => (
            t.center.0 - npc.width() / 2.0 - root.0,
            t.center.1 - npc.height() / 2.0 - root.1,
        ),
        None => (0.0, 0.0),
    };
    let span = (dx * dx + dy * dy).sqrt();
    if span > reach {
        let k = reach / span;
        dx *= k;
        dy *= k;
    }

    // Pull toward the aim point, with an extra shove while still moving the wrong way.
    if npc.position.0 < root.0 + dx {
        npc.velocity.0 += params.pull;
        if npc.velocity.0 < 0.0 && dx > 0.0 {
            npc.velocity.0 += params.pull * 1.5;
        }
    } else if npc.position.0 > root.0 + dx {
        npc.velocity.0 -= params.pull;
        if npc.velocity.0 > 0.0 && dx < 0.0 {
            npc.velocity.0 -= params.pull * 1.5;
        }
    }
    if npc.position.1 < root.1 + dy {
        npc.velocity.1 += params.pull;
        if npc.velocity.1 < 0.0 && dy > 0.0 {
            npc.velocity.1 += params.pull * 1.5;
        }
    } else if npc.position.1 > root.1 + dy {
        npc.velocity.1 -= params.pull;
        if npc.velocity.1 > 0.0 && dy < 0.0 {
            npc.velocity.1 -= params.pull * 1.5;
        }
    }
    npc.velocity.0 = npc.velocity.0.clamp(-params.cap, params.cap);
    npc.velocity.1 = npc.velocity.1.clamp(-params.cap, params.cap);

    npc.sprite_direction = if dx > 0.0 { 1 } else { -1 };
    npc.rotation = dy.atan2(dx);

    // A plant that clips terrain rebounds hard rather than grinding along it.
    if npc.collide_x {
        npc.velocity.0 = npc.old_velocity.0 * -0.7;
        if npc.velocity.0 > 0.0 && npc.velocity.0 < 2.0 {
            npc.velocity.0 = 2.0;
        }
        if npc.velocity.0 < 0.0 && npc.velocity.0 > -2.0 {
            npc.velocity.0 = -2.0;
        }
        npc.dirty = true;
    }
    if npc.collide_y {
        npc.velocity.1 = npc.old_velocity.1 * -0.7;
        if npc.velocity.1 > 0.0 && npc.velocity.1 < 2.0 {
            npc.velocity.1 = 2.0;
        }
        if npc.velocity.1 < 0.0 && npc.velocity.1 > -2.0 {
            npc.velocity.1 = -2.0;
        }
        npc.dirty = true;
    }

    npc.dirty = true;
    Outcome::Alive
}

/// Drive one vulture for a tick.
pub fn vulture<T: TileView>(npc: &mut Npc, world: &World<'_, T>) {
    npc.no_gravity = true;
    if npc.ai[0] == 0.0 {
        // Perched, and therefore heavy.
        npc.no_gravity = false;
        if let Some(t) = world.target {
            face(npc, t);
        }
        let jostled = npc.velocity.0 != 0.0 || npc.velocity.1 < 0.0 || npc.velocity.1 > 0.3;
        if jostled {
            npc.ai[0] = 1.0;
            npc.dirty = true;
        } else {
            let disturbed = npc.life < npc.life_max
                || world.target.is_some_and(|t| {
                    let (cx, cy) = npc.center();
                    (t.center.0 - cx).abs() < PERCH_STARTLE + npc.width()
                        && (t.center.1 - cy).abs() < PERCH_STARTLE + npc.height()
                });
            if disturbed {
                npc.ai[0] = 1.0;
                npc.velocity.1 -= PERCH_LAUNCH;
                npc.dirty = true;
            }
        }
    } else if let Some(t) = world.target.filter(|t| t.alive) {
        bounce(npc);
        face(npc, t);

        // Level flight toward the target, with the usual brake against the turn.
        if npc.direction == -1 && npc.velocity.0 > -3.0 {
            npc.velocity.0 -= 0.1;
            if npc.velocity.0 > 3.0 {
                npc.velocity.0 -= 0.1;
            } else if npc.velocity.0 > 0.0 {
                npc.velocity.0 -= 0.05;
            }
            npc.velocity.0 = npc.velocity.0.max(-3.0);
        } else if npc.direction == 1 && npc.velocity.0 < 3.0 {
            npc.velocity.0 += 0.1;
            if npc.velocity.0 < -3.0 {
                npc.velocity.0 += 0.1;
            } else if npc.velocity.0 < 0.0 {
                npc.velocity.0 += 0.05;
            }
            npc.velocity.0 = npc.velocity.0.min(3.0);
        }

        // Hold a hundred pixels above, unless it is nearly overhead already.
        let across = (npc.position.0 + npc.width() / 2.0 - t.center.0).abs();
        let mut wanted = t.center.1 - super::PLAYER_HEIGHT as f32 / 2.0 - npc.height() / 2.0;
        if across > VULTURE_CLIMB_AT {
            wanted -= VULTURE_CEILING;
        }
        if npc.position.1 < wanted {
            npc.velocity.1 += 0.05;
            if npc.velocity.1 < 0.0 {
                npc.velocity.1 += 0.01;
            }
        } else {
            npc.velocity.1 -= 0.05;
            if npc.velocity.1 > 0.0 {
                npc.velocity.1 -= 0.01;
            }
        }
        npc.velocity.1 = npc.velocity.1.clamp(-3.0, 3.0);
    }

    if world.wet {
        rise_out_of_water(npc);
        if let Some(t) = world.target {
            face(npc, t);
        }
    }
    npc.sprite_direction = npc.direction;
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Jungle(HashMap<(i32, i32), Tile>);

    impl TileView for Jungle {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn world<'a>(tiles: &'a Jungle, target: Option<Target>) -> World<'a, Jungle> {
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

    fn rooted_at(npc_type: u16, tile_x: i32, tile_y: i32) -> (Npc, Jungle) {
        let mut n = Npc::new(npc_type, (tile_x as f32 * TILE, tile_y as f32 * TILE), 1)
            .expect("a style 13 type");
        n.ai[0] = tile_x as f32;
        n.ai[1] = tile_y as f32;
        let mut t = Jungle::default();
        t.0.insert((tile_x, tile_y), Tile::block(1));
        (n, t)
    }

    #[test]
    fn a_plant_dies_when_its_tile_is_mined_out() {
        let (mut p, tiles) = rooted_at(43, 500, 500);
        assert_eq!(plant(&mut p, &world(&tiles, None)), Outcome::Alive);
        let bare = Jungle::default();
        assert_eq!(plant(&mut p, &world(&bare, None)), Outcome::Uprooted);
    }

    #[test]
    fn a_plant_lunges_toward_a_player_within_reach() {
        let (mut p, tiles) = rooted_at(43, 500, 500);
        let (cx, cy) = p.center();
        let t = Some(player_at(cx + 150.0, cy));
        for _ in 0..200 {
            plant(&mut p, &world(&tiles, t));
        }
        assert!(p.velocity.0 > 0.0, "should reach out, got {}", p.velocity.0);
    }

    #[test]
    fn a_plant_will_not_reach_past_its_own_length() {
        let (mut p, tiles) = rooted_at(56, 500, 500);
        let root = (500.0 * 16.0 + 8.0, 500.0 * 16.0 + 8.0);
        let (cx, cy) = p.center();
        // Someone far out of reach; the plant should still stop at its own limit.
        let t = Some(player_at(cx + 4000.0, cy));
        let mut furthest: f32 = 0.0;
        for _ in 0..2000 {
            plant(&mut p, &world(&tiles, t));
            p.position.0 += p.velocity.0;
            p.position.1 += p.velocity.1;
            furthest = furthest.max(p.position.0 - root.0);
        }
        // It has no brake beyond the same pull it accelerates with, so it coasts past its limit
        // by exactly the distance it takes to shed its top speed.
        let params = rooted_params(56);
        let reach = params.reach * ROOTED_STRETCH;
        let coast = params.cap.powi(2) / (2.0 * params.pull);
        assert!(
            furthest < reach + coast + 10.0,
            "a snatcher reached {furthest}, past its {reach} plus {coast} of coasting"
        );
    }

    #[test]
    fn each_plant_has_its_own_reach() {
        assert_eq!(rooted_params(43).reach, 250.0, "man eater");
        assert_eq!(rooted_params(56).reach, 150.0, "snatcher");
        assert_eq!(rooted_params(259).reach, 100.0, "fungi bulb");
        assert_eq!(rooted_params(43).cap, 3.0, "and the man eater is quicker");
    }

    #[test]
    fn a_plant_stretches_for_the_last_third_of_its_cycle() {
        let (mut p, tiles) = rooted_at(56, 500, 500);
        let (cx, cy) = p.center();
        let t = Some(player_at(cx + 4000.0, cy));
        // Just before the stretch.
        p.ai[2] = ROOTED_STRETCH_AT - 1.0;
        plant(&mut p, &world(&tiles, t));
        let short = p.velocity.0;
        p.velocity = (0.0, 0.0);
        p.ai[2] = ROOTED_STRETCH_AT + 1.0;
        plant(&mut p, &world(&tiles, t));
        assert_eq!(
            short, p.velocity.0,
            "the pull is the same either way; only the limit moves"
        );
        assert!(p.ai[2] > ROOTED_STRETCH_AT, "and the cycle keeps running");
    }

    #[test]
    fn a_perched_vulture_stays_put_until_something_disturbs_it() {
        let tiles = Jungle::default();
        let mut v = Npc::new(61, (10_000.0, 10_000.0), 1).expect("vulture");
        let (cx, cy) = v.center();
        vulture(&mut v, &world(&tiles, Some(player_at(cx + 900.0, cy))));
        assert_eq!(v.ai[0], 0.0, "still perched");
        assert!(!v.no_gravity, "and heavy");

        vulture(&mut v, &world(&tiles, Some(player_at(cx + 50.0, cy))));
        assert_eq!(v.ai[0], 1.0, "should have taken off");
        assert!(v.velocity.1 < 0.0, "with a kick, got {}", v.velocity.1);
    }

    #[test]
    fn a_hurt_vulture_takes_off_too() {
        let tiles = Jungle::default();
        let mut v = Npc::new(61, (10_000.0, 10_000.0), 1).expect("vulture");
        v.life -= 1;
        let (cx, cy) = v.center();
        vulture(&mut v, &world(&tiles, Some(player_at(cx + 900.0, cy))));
        assert_eq!(v.ai[0], 1.0);
    }

    #[test]
    fn an_airborne_vulture_circles_above_its_target() {
        let tiles = Jungle::default();
        let mut v = Npc::new(61, (10_000.0, 10_000.0), 1).expect("vulture");
        v.ai[0] = 1.0;
        let (cx, cy) = v.center();
        // Well off to the side, so it should climb to its preferred ceiling.
        let t = Some(player_at(cx + 600.0, cy));
        // It hunts rather than settles: it climbs past its preferred ceiling, falls back through
        // it, and circles. So what to check is the height it reaches and where it spends its time,
        // not where it happens to be on the last tick.
        let mut highest = v.center().1;
        let mut above = 0;
        for _ in 0..300 {
            vulture(&mut v, &world(&tiles, t));
            v.position.0 += v.velocity.0;
            v.position.1 += v.velocity.1;
            highest = highest.min(v.center().1);
            if v.center().1 < cy {
                above += 1;
            }
        }
        assert!(v.no_gravity, "flying vultures are weightless");
        assert!(
            highest < cy - VULTURE_CEILING,
            "should have climbed above its target, got {highest} against {cy}"
        );
        assert!(above > 150, "and stayed up there most of the time: {above}");
        assert!(v.center().0 > cx, "and closed the gap");
    }
}
