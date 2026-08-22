//! Style 66 — the grubs.
//!
//! Ported from the `aiStyle == 66` block. Worms, maggots and bait: they inch along the ground at a
//! fifth of a pixel a tick, stop for five to fifteen seconds, then inch somewhere else. There is no
//! target and no chase; the only thing that ever interrupts the cycle is a player getting close
//! enough to frighten a truffle worm into the ground.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    GRUB_CRAWL_TICKS, GRUB_REST_TICKS, TRUFFLE_WORM_DIGGER, TRUFFLE_WORM_RANGE,
    TRUFFLE_WORM_WINDUP, grub_speed,
};

use super::World;
use crate::game::npc::{Npc, TileView};

/// Whether a type bolts underground rather than being caught.
fn skittish(npc_type: u16) -> bool {
    npc_type == 374
}

/// Drive one grub for a tick. Returns what it turned into, if it turned into anything.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, rng: &mut SmallRng) -> Option<u16> {
    if npc.velocity.1 == 0.0 {
        if npc.ai[0] == 1.0 {
            if npc.direction == 0 {
                npc.direction = 1;
            }
            if npc.collide_x {
                npc.direction = -npc.direction;
            }
            npc.velocity.0 = grub_speed(npc.npc_type) * f32::from(npc.direction);
        } else {
            npc.velocity.0 = 0.0;
        }

        // `ai[2]` holds what the game keeps in `localAI[1]`: ticks left in the current phase.
        npc.ai[2] -= 1.0;
        if npc.ai[2] <= 0.0 {
            if npc.ai[0] == 1.0 {
                npc.ai[0] = 0.0;
                npc.ai[2] = rng.random_range(GRUB_REST_TICKS.0..GRUB_REST_TICKS.1) as f32;
            } else {
                npc.ai[0] = 1.0;
                npc.ai[2] = rng.random_range(GRUB_CRAWL_TICKS.0..GRUB_CRAWL_TICKS.1) as f32;
            }
            npc.dirty = true;
        }
    } else if npc.direction == 0 {
        npc.direction = if npc.velocity.0 < 0.0 { -1 } else { 1 };
    }

    if !skittish(npc.npc_type) {
        npc.dirty = true;
        return None;
    }

    npc.sprite_direction = npc.direction;
    let watched = world.target.is_some_and(|t| {
        let (cx, cy) = npc.center();
        ((t.center.0 - cx).powi(2) + (t.center.1 - cy).powi(2)).sqrt() <= TRUFFLE_WORM_RANGE
    });
    if watched && npc.ai[1] < TRUFFLE_WORM_WINDUP {
        npc.ai[1] += 1.0;
    }
    npc.dirty = true;
    if npc.ai[1] == TRUFFLE_WORM_WINDUP {
        // It drops a tile as it goes, which is what makes it look like it burrowed.
        npc.position.1 += 16.0;
        return Some(TRUFFLE_WORM_DIGGER);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use terrustia_proto::tile::Tile;

    struct Nowhere;

    impl TileView for Nowhere {
        fn tile(&self, _x: i32, _y: i32) -> Tile {
            Tile::AIR
        }
    }

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(5)
    }

    fn grub(npc_type: u16) -> Npc {
        Npc::new(npc_type, (1000.0, 1000.0), 1).expect("a style 66 type")
    }

    fn world<'a>(tiles: &'a Nowhere, target: Option<Target>) -> World<'a, Nowhere> {
        crate::game::ai::calm(tiles, target)
    }

    #[test]
    fn a_resting_worm_does_not_move() {
        let tiles = Nowhere;
        let mut w = grub(357);
        w.ai[2] = 100.0;
        w.velocity.0 = 5.0;
        update(&mut w, &world(&tiles, None), &mut rng());
        assert_eq!(w.velocity.0, 0.0);
    }

    #[test]
    fn a_crawling_worm_inches_along() {
        let tiles = Nowhere;
        let mut w = grub(357);
        w.ai[0] = 1.0;
        w.ai[2] = 100.0;
        w.direction = 1;
        update(&mut w, &world(&tiles, None), &mut rng());
        assert_eq!(w.velocity.0, grub_speed(357));
    }

    #[test]
    fn the_glowing_baits_crawl_faster() {
        assert_eq!(grub_speed(357), 0.2);
        assert!(grub_speed(487) > grub_speed(485));
        assert!(grub_speed(374) > grub_speed(487), "truffle worms hurry");
    }

    #[test]
    fn a_worm_turns_round_when_it_hits_something() {
        let tiles = Nowhere;
        let mut w = grub(357);
        w.ai[0] = 1.0;
        w.ai[2] = 100.0;
        w.direction = 1;
        w.collide_x = true;
        update(&mut w, &world(&tiles, None), &mut rng());
        assert_eq!(w.direction, -1);
        assert!(w.velocity.0 < 0.0);
    }

    #[test]
    fn the_phase_timer_alternates_between_resting_and_crawling() {
        let tiles = Nowhere;
        let mut w = grub(357);
        w.ai[2] = 1.0;
        update(&mut w, &world(&tiles, None), &mut rng());
        assert_eq!(w.ai[0], 1.0, "should have set off");
        assert!(
            (GRUB_CRAWL_TICKS.0 as f32..GRUB_CRAWL_TICKS.1 as f32).contains(&w.ai[2]),
            "and taken a crawl-length timer, got {}",
            w.ai[2]
        );
    }

    #[test]
    fn a_truffle_worm_burrows_when_watched_and_an_ordinary_worm_does_not() {
        let tiles = Nowhere;
        let mut truffle = grub(374);
        let (cx, cy) = truffle.center();
        let close = Some(Target {
            slot: 0,
            center: (cx + 50.0, cy),
            velocity: (0.0, 0.0),
            alive: true,
        });
        let mut became = None;
        for _ in 0..200 {
            truffle.ai[2] = 100.0;
            if let Some(t) = update(&mut truffle, &world(&tiles, close), &mut rng()) {
                became = Some(t);
                break;
            }
        }
        assert_eq!(became, Some(TRUFFLE_WORM_DIGGER));

        let mut ordinary = grub(357);
        for _ in 0..200 {
            ordinary.ai[2] = 100.0;
            assert!(update(&mut ordinary, &world(&tiles, close), &mut rng()).is_none());
        }
    }

    #[test]
    fn a_truffle_worm_left_alone_stays_put() {
        let tiles = Nowhere;
        let mut truffle = grub(374);
        let (cx, cy) = truffle.center();
        let far = Some(Target {
            slot: 0,
            center: (cx + TRUFFLE_WORM_RANGE + 10.0, cy),
            velocity: (0.0, 0.0),
            alive: true,
        });
        for _ in 0..300 {
            truffle.ai[2] = 100.0;
            assert!(update(&mut truffle, &world(&tiles, far), &mut rng()).is_none());
        }
    }
}
