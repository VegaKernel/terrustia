//! Style 114 — the dragonflies.
//!
//! A dragonfly keeps a perch, and everything it does is measured against it. It alternates between
//! resting and four ticks of flight — unless it has drifted more than a hundred pixels from the
//! perch, in which case it flies for two hundred instead and heads back. Approach it and it bolts,
//! setting a new perch in the direction it fled.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    DRAGONFLY_FAR, DRAGONFLY_FEAR_PLAYER, DRAGONFLY_FLIGHT, DRAGONFLY_LONG_FLIGHT, DRAGONFLY_NEAR,
    DRAGONFLY_REST, DRAGONFLY_TETHER,
};
use terrustia_proto::tile_solid::solid;

use super::World;
use crate::game::npc::{Npc, TILE, TileView};

/// The hardest a startled dragonfly can be flying.
const PANIC_SPEED: f32 = 16.0;
/// How often it checks whether anything is too close.
const FEAR_INTERVAL: f32 = 15.0;

/// A random drift, from the ellipse the game samples: a circle plus its own edge, scaled.
fn wander(rng: &mut SmallRng) -> (f32, f32) {
    let inside = {
        let angle = rng.random::<f32>() * std::f32::consts::TAU;
        let radius = rng.random::<f32>().sqrt();
        (angle.cos() * 5.0 * radius, angle.sin() * 3.0 * radius)
    };
    let edge = {
        let angle = rng.random::<f32>() * std::f32::consts::TAU;
        (angle.cos() * 5.0, angle.sin() * 3.0)
    };
    ((inside.0 + edge.0) * 0.4, (inside.1 + edge.1) * 0.4)
}

/// Drive one dragonfly for a tick.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, rng: &mut SmallRng) {
    // First tick: this is its perch, and it sets off from it.
    if npc.local_ai[0] == 0.0 {
        npc.local_ai[0] = 1.0;
        let (cx, cy) = npc.center();
        npc.ai[2] = cx;
        npc.ai[3] = cy;
        npc.velocity = wander(rng);
        npc.ai[1] = 0.0;
        npc.ai[0] = 1.0;
        npc.dirty = true;
    }

    let perch = (npc.ai[2], npc.ai[3]);
    let (cx, cy) = npc.center();
    let from_perch = ((perch.0 - cx).powi(2) + (perch.1 - cy).powi(2)).sqrt();

    if npc.ai[0] == 0.0 {
        // Resting: coast to a stop and wait.
        npc.velocity.0 *= 0.94;
        npc.velocity.1 *= 0.94;
        npc.ai[1] += 1.0;
        if npc.ai[1] >= (DRAGONFLY_REST.0 + rng.random_range(0..DRAGONFLY_REST.1)) as f32 {
            // Far out: head straight back. Nearly there: amble. Home: pick a new direction.
            npc.velocity = if from_perch > DRAGONFLY_FAR {
                let k = 3.0 / from_perch;
                ((perch.0 - cx) * k, (perch.1 - cy) * k)
            } else if from_perch > DRAGONFLY_NEAR {
                let k = 1.0 / from_perch;
                let drift = wander(rng);
                (
                    (perch.0 - cx) * k + drift.0 * 0.5,
                    (perch.1 - cy) * k + drift.1 * 0.25,
                )
            } else {
                wander(rng)
            };
            npc.ai[1] = 0.0;
            npc.ai[0] = 1.0;
            npc.dirty = true;
        }
    } else {
        // Flying. A short hop, unless it has strayed and needs a long one.
        let limit = if from_perch > DRAGONFLY_TETHER {
            DRAGONFLY_LONG_FLIGHT
        } else {
            DRAGONFLY_FLIGHT
        };
        npc.ai[1] += 1.0;
        if npc.ai[1] >= limit {
            npc.ai[1] = 0.0;
            npc.ai[0] = 0.0;
            npc.dirty = true;
        }

        let (tile_x, tile_y) = ((cx / TILE) as i32, (cy / TILE) as i32);
        // About to fly into something: push up off it.
        if (tile_y..tile_y + 3).any(|y| {
            let t = world.tiles.tile(tile_x, y);
            (t.is_active() && solid(t.block)) || t.liquid > 0
        }) {
            if npc.velocity.1 > 0.0 {
                npc.velocity.1 *= 0.9;
            }
            npc.velocity.1 -= 0.2;
        }
        // Climbing with nothing below for thirty tiles: ease off, it is over open sky.
        if npc.velocity.1 < 0.0
            && !(tile_y..tile_y + 30).any(|y| {
                let t = world.tiles.tile(tile_x, y);
                t.is_active() && solid(t.block)
            })
        {
            npc.velocity.1 *= 0.9;
        }
    }

    if npc.velocity.0 != 0.0 {
        npc.direction = if npc.velocity.0 > 0.0 { 1 } else { -1 };
    }
    if world.wet {
        npc.velocity.1 = -3.0;
    }

    // Every quarter second it looks around, and bolts from anything close.
    if npc.local_ai[1] > 0.0 {
        npc.local_ai[1] -= 1.0;
        npc.dirty = true;
        return;
    }
    npc.local_ai[1] = FEAR_INTERVAL;
    if let Some(t) = world.target {
        let away = (cx - t.center.0, cy - t.center.1);
        let reach = (away.0 * away.0 + away.1 * away.1).sqrt();
        if reach <= DRAGONFLY_FEAR_PLAYER && reach > 0.0 {
            let flee = (away.0 / reach * 2.0, away.1 / reach * 2.0);
            npc.velocity.0 += flee.0;
            npc.velocity.1 += flee.1;
            let speed = (npc.velocity.0.powi(2) + npc.velocity.1.powi(2)).sqrt();
            if speed > PANIC_SPEED {
                npc.velocity.0 = npc.velocity.0 / speed * PANIC_SPEED;
                npc.velocity.1 = npc.velocity.1 / speed * PANIC_SPEED;
            }
            // And it re-perches ahead of itself, so it keeps going rather than turning back.
            npc.ai[1] = -10.0;
            npc.ai[0] = 1.0;
            npc.ai[2] = cx + flee.0 * 10.0;
            npc.ai[3] = cy + flee.1 * 10.0;
            npc.dirty = true;
        }
    }
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use terrustia_proto::tile::Tile;

    struct Air;

    impl TileView for Air {
        fn tile(&self, _x: i32, _y: i32) -> Tile {
            Tile::AIR
        }
    }

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(41)
    }

    fn fly() -> Npc {
        Npc::new(601, (10_000.0, 10_000.0), 1).expect("dragonfly")
    }

    fn world<'a>(tiles: &'a Air, target: Option<Target>) -> World<'a, Air> {
        crate::game::ai::calm(tiles, target)
    }

    #[test]
    fn a_dragonfly_remembers_where_it_started() {
        let tiles = Air;
        let mut d = fly();
        let (cx, cy) = d.center();
        update(&mut d, &world(&tiles, None), &mut rng());
        assert_eq!((d.ai[2], d.ai[3]), (cx, cy));
    }

    #[test]
    fn a_dragonfly_stays_near_its_perch() {
        let tiles = Air;
        let mut d = fly();
        let mut r = rng();
        update(&mut d, &world(&tiles, None), &mut r);
        let perch = (d.ai[2], d.ai[3]);
        let mut furthest: f32 = 0.0;
        for _ in 0..4000 {
            update(&mut d, &world(&tiles, None), &mut r);
            d.position.0 += d.velocity.0;
            d.position.1 += d.velocity.1;
            let (cx, cy) = d.center();
            furthest = furthest.max(((perch.0 - cx).powi(2) + (perch.1 - cy).powi(2)).sqrt());
        }
        assert!(
            furthest < 600.0,
            "it should not wander off, but got {furthest} away"
        );
    }

    #[test]
    fn a_dragonfly_bolts_when_you_get_close() {
        let tiles = Air;
        let mut d = fly();
        let mut r = rng();
        update(&mut d, &world(&tiles, None), &mut r);
        d.local_ai[1] = 0.0;
        d.velocity = (0.0, 0.0);
        let (cx, cy) = d.center();
        let close = Some(Target {
            slot: 0,
            center: (cx + 50.0, cy),
            velocity: (0.0, 0.0),
            alive: true,
        });
        update(&mut d, &world(&tiles, close), &mut r);
        assert!(d.velocity.0 < 0.0, "should flee left, got {}", d.velocity.0);
        assert!(d.ai[2] < cx, "and re-perch that way too");
    }

    #[test]
    fn a_dragonfly_ignores_someone_across_the_room() {
        let tiles = Air;
        let mut d = fly();
        let mut r = rng();
        update(&mut d, &world(&tiles, None), &mut r);
        let perch = (d.ai[2], d.ai[3]);
        d.local_ai[1] = 0.0;
        let (cx, cy) = d.center();
        let far = Some(Target {
            slot: 0,
            center: (cx + DRAGONFLY_FEAR_PLAYER + 100.0, cy),
            velocity: (0.0, 0.0),
            alive: true,
        });
        update(&mut d, &world(&tiles, far), &mut r);
        assert_eq!((d.ai[2], d.ai[3]), perch, "its perch should be unchanged");
    }

    #[test]
    fn a_dragonfly_in_water_heads_straight_up() {
        let tiles = Air;
        let mut d = fly();
        let mut w = world(&tiles, None);
        w.wet = true;
        update(&mut d, &w, &mut rng());
        assert_eq!(d.velocity.1, -3.0);
    }

    #[test]
    fn a_dragonfly_far_from_its_perch_flies_for_much_longer() {
        let tiles = Air;
        let mut d = fly();
        let mut r = rng();
        update(&mut d, &world(&tiles, None), &mut r);
        // Move it well past its tether and put it into flight.
        d.position.0 += DRAGONFLY_TETHER + 100.0;
        d.ai[0] = 1.0;
        d.ai[1] = DRAGONFLY_FLIGHT + 1.0;
        d.local_ai[1] = 100.0;
        update(&mut d, &world(&tiles, None), &mut r);
        assert_eq!(d.ai[0], 1.0, "should still be flying home");
    }
}
