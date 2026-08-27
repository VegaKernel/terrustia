//! Style 67 — the snails.
//!
//! A snail lives on surfaces. It has no gravity while it is stuck to one, and it turns every corner
//! it meets, so one placed on the outside of a block will trace the whole perimeter and come back
//! round. Two things pull it off: a one-in-7,200 chance per tick of simply letting go, and touching
//! nothing at all for five ticks. Once it is off, gravity comes back and it crawls along the floor
//! until it finds a wall to climb again.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{SNAIL_LOST_GRIP, SNAIL_SLIP_CHANCE, snail_speed};

use super::World;
use crate::game::npc::{Npc, TILE, TileView};

/// Drive one snail for a tick.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, rng: &mut SmallRng) {
    // `ai[3]` is its size, rolled once. A snail is between four fifths and eleven tenths of one.
    if npc.npc_type == 359 {
        if npc.ai[3] == 0.0 {
            npc.ai[3] = rng.random_range(80..111) as f32 * 0.01;
            npc.dirty = true;
        }
        npc.scale = npc.ai[3];
    }
    let speed = snail_speed(npc.npc_type);

    if npc.ai[0] == 0.0 {
        npc.direction_y = 1;
        npc.ai[0] = 1.0;
        if npc.direction > 0 {
            npc.sprite_direction = 1;
        }
    }

    // Letting go, either on a whim or because there is nothing left to hold.
    if npc.ai[2] == 0.0 && rng.random_ratio(1, SNAIL_SLIP_CHANCE) {
        npc.ai[2] = 2.0;
        npc.dirty = true;
    }
    // Note the two counters: `ai[1]` says which way round it is crawling, and this one counts how
    // long it has been touching nothing. They are different things and the game keeps them apart.
    if !npc.collide_x && !npc.collide_y {
        npc.local_ai[3] += 1.0;
        if npc.local_ai[3] > SNAIL_LOST_GRIP {
            npc.ai[2] = 2.0;
            npc.dirty = true;
        }
    } else {
        npc.local_ai[3] = 0.0;
    }

    if npc.ai[2] > 0.0 {
        // Fallen off: heavy, and crawling along the ground looking for a wall.
        npc.ai[0] = 1.0;
        npc.direction_y = 1;
        if npc.velocity.1 > speed {
            npc.rotation += f32::from(npc.direction) * 0.1;
        } else {
            npc.rotation = 0.0;
        }
        npc.sprite_direction = npc.direction;
        npc.velocity.0 = speed * f32::from(npc.direction);
        npc.no_gravity = false;

        // Landing counts down its confusion; two landings and it is back on the wall.
        let behind_x = ((npc.center().0 + (npc.stats.width / 2 * i32::from(-npc.direction)) as f32)
            / TILE) as i32;
        let below_y = ((npc.position.1 + npc.height() + 8.0) / TILE) as i32;
        if npc.collide_y && world.tiles.tile(behind_x, below_y).slope == 0 {
            npc.ai[2] -= 1.0;
            npc.dirty = true;
        }
        // Walking into a wall while on the ground is exactly the handhold it wants.
        if npc.collide_x && npc.velocity.1 == 0.0 {
            npc.ai[2] = 0.0;
            npc.direction_y = -1;
            npc.ai[1] = 1.0;
            npc.dirty = true;
        }
        npc.dirty = true;
        return;
    }

    // Stuck to a surface. The corner-turning is the blazing wheel's, at a fifth of the speed.
    npc.no_gravity = true;
    if npc.ai[1] == 0.0 {
        if npc.collide_y {
            npc.ai[0] = 2.0;
        }
        if !npc.collide_y && npc.ai[0] == 2.0 {
            npc.direction = -npc.direction;
            npc.ai[1] = 1.0;
            npc.ai[0] = 1.0;
        }
        if npc.collide_x {
            npc.direction_y = -npc.direction_y;
            npc.ai[1] = 1.0;
        }
    } else {
        if npc.collide_x {
            npc.ai[0] = 2.0;
        }
        if !npc.collide_x && npc.ai[0] == 2.0 {
            npc.direction_y = -npc.direction_y;
            npc.ai[1] = 0.0;
            npc.ai[0] = 1.0;
        }
        if npc.collide_y {
            npc.direction = -npc.direction;
            npc.ai[1] = 0.0;
        }
    }

    npc.velocity.0 = speed * f32::from(npc.direction);
    npc.velocity.1 = speed * f32::from(npc.direction_y);
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ai::Conditions;
    use rand::SeedableRng;
    use terrustia_proto::tile::Tile;

    struct Bare;

    impl TileView for Bare {
        fn tile(&self, _x: i32, _y: i32) -> Tile {
            Tile::AIR
        }
    }

    fn world<'a>(tiles: &'a Bare) -> World<'a, Bare> {
        World {
            tiles,
            target: None,
            wet: false,
            target_wet: false,
            conditions: Conditions::default(),
            was_hurt: false,
            target_velocity: (0.0, 0.0),
            hostile: None,
            census: &[],
            parent: None,
            parent_state: 0.0,
            parent_health: 1.0,
            crowding: (0.0, 0.0),
            avoid: &[],
            target_taken: false,
            hooks: None,
            kin_moving: false,
            sockets_open: 0,
            army: crate::game::ai::ArmyView::default(),
            treasure: None,
            mage: Default::default(),
            slot: 0,
        }
    }

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(31)
    }

    fn snail() -> Npc {
        Npc::new(359, (10_000.0, 10_000.0), 1).expect("snail")
    }

    #[test]
    fn a_snail_rolls_its_size_once() {
        let tiles = Bare;
        let mut s = snail();
        update(&mut s, &world(&tiles), &mut rng());
        let size = s.ai[3];
        assert!((0.8..1.11).contains(&size), "got {size}");
        assert_eq!(s.scale, size);
        for _ in 0..20 {
            s.collide_y = true;
            update(&mut s, &world(&tiles), &mut rng());
        }
        assert_eq!(s.ai[3], size, "it should not reroll");
    }

    #[test]
    fn a_snail_on_a_wall_is_weightless() {
        let tiles = Bare;
        let mut s = snail();
        s.collide_y = true;
        update(&mut s, &world(&tiles), &mut rng());
        assert!(s.no_gravity);
        let speed = snail_speed(359);
        assert!(s.velocity.0.abs() == speed || s.velocity.1.abs() == speed);
    }

    #[test]
    fn a_snail_turns_the_corner_when_its_surface_ends() {
        let tiles = Bare;
        let mut s = snail();
        s.direction = 1;
        s.direction_y = 1;
        s.collide_y = true;
        update(&mut s, &world(&tiles), &mut rng());
        assert_eq!(s.ai[0], 2.0, "should have found the surface");
        s.collide_y = false;
        update(&mut s, &world(&tiles), &mut rng());
        assert_eq!(s.direction, -1, "and turned when it ran out");
    }

    #[test]
    fn a_snail_touching_nothing_falls_off() {
        let tiles = Bare;
        let mut s = snail();
        s.ai[0] = 1.0;
        for _ in 0..(SNAIL_LOST_GRIP as i32 + 2) {
            update(&mut s, &world(&tiles), &mut rng());
        }
        assert!(s.ai[2] > 0.0, "should have let go");
        assert!(!s.no_gravity, "and become heavy again");
    }

    #[test]
    fn a_fallen_snail_climbs_the_first_wall_it_walks_into() {
        let tiles = Bare;
        let mut s = snail();
        s.ai[2] = 2.0;
        s.collide_x = true;
        s.velocity.1 = 0.0;
        update(&mut s, &world(&tiles), &mut rng());
        assert_eq!(s.ai[2], 0.0, "back on a surface");
        assert_eq!(s.direction_y, -1, "and heading up it");
    }

    #[test]
    fn a_gastropod_is_twice_as_quick_as_a_snail() {
        assert_eq!(snail_speed(359), 0.3);
        assert_eq!(snail_speed(360), 0.6);
    }
}
