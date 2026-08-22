//! Leapers: style 41 — the derpling, the herpling, and the chattering teeth bomb.
//!
//! A leaper does not walk toward you, it *winds up*. Grounded, it charges a counter that ticks
//! faster the closer you are, and when the counter reaches zero it launches. Two or three short
//! hops in a row are followed by one big one, then a long pause, so the approach comes in bursts
//! with a beat you can read and dodge — which is the whole character of the thing.
//!
//! Two details are easy to miss and both matter:
//!
//! * A leaper that lands in exactly the same place it took off from decides it is stuck, turns
//!   round, and waits five seconds. That is what stops one grinding against a wall forever.
//! * The chattering teeth bomb is the same routine with the ending swapped: instead of biting, it
//!   swells to a hundred and sixty pixels across and goes off, and touching water sets it off too.

use terrustia_proto::npc_params::{
    DERPLING, DERPLING_AIR_CAP, DERPLING_CHARGE, DERPLING_HOP, DERPLING_HOPS_BEFORE_LEAP,
    DERPLING_LEAP, DERPLING_URGENCY_SCALE, LEAPER_AIR_CAP, LEAPER_AIR_LEAN, LEAPER_CHARGE,
    LEAPER_HOP, LEAPER_HOPS_BEFORE_LEAP, LEAPER_LEAP, LEAPER_LONG_REST, LEAPER_REST,
    LEAPER_STUCK_WAIT, LEAPER_URGENCY, LEAPER_URGENCY_CAP, LEAPER_URGENCY_SCALE, TEETH_BLAST_SIZE,
    TEETH_BOMB, TEETH_FUSE, TEETH_TRIGGER,
};

use super::drifters::Outcome;
use crate::game::ai::{World, face};
use crate::game::npc::{Npc, TileView};

/// The numbers that differ between the derpling and the herpling.
struct Gait {
    charge: f32,
    urgency: f32,
    hop: (f32, f32),
    leap: (f32, f32),
    hops_before_leap: f32,
    air_cap: f32,
}

fn gait(npc_type: u16) -> Gait {
    if npc_type == DERPLING {
        Gait {
            charge: DERPLING_CHARGE,
            urgency: DERPLING_URGENCY_SCALE,
            hop: DERPLING_HOP,
            leap: DERPLING_LEAP,
            hops_before_leap: DERPLING_HOPS_BEFORE_LEAP,
            air_cap: DERPLING_AIR_CAP,
        }
    } else {
        Gait {
            charge: LEAPER_CHARGE,
            urgency: LEAPER_URGENCY_SCALE,
            hop: LEAPER_HOP,
            leap: LEAPER_LEAP,
            hops_before_leap: LEAPER_HOPS_BEFORE_LEAP,
            air_cap: LEAPER_AIR_CAP,
        }
    }
}

/// Style 41.
pub fn leaper(npc: &mut Npc, world: &World<'_, impl TileView>) -> Outcome {
    let mut out = Outcome::default();
    npc.dirty = true;
    let g = gait(npc.npc_type);

    // A cooldown from having just turned round; it counts down to one and stops there, so that
    // "zero" can mean "start a fresh approach".
    if npc.ai[2] > 1.0 {
        npc.ai[2] -= 1.0;
    }
    if npc.ai[2] == 0.0 {
        npc.ai[0] = -100.0;
        npc.ai[2] = 1.0;
        if let Some(target) = world.target {
            face(npc, target);
        }
        npc.sprite_direction = npc.direction;
    }

    if npc.npc_type == TEETH_BOMB {
        // Already going off: swell, hold still, and detonate at the end of the fuse.
        if npc.ai[1] == 5.0 {
            npc.velocity = (0.0, 0.0);
            npc.resize(TEETH_BLAST_SIZE, TEETH_BLAST_SIZE);
            npc.invulnerable = true;
            if npc.ai[2] == 1.0 {
                out.spent = true;
                out.detonated = true;
            }
            return out;
        }
        // Water or a player within arm's reach lights it.
        let triggered = world.wet
            || world.target.is_some_and(|t| {
                let (cx, cy) = npc.center();
                (t.center.0 - cx).hypot(t.center.1 - cy) < TEETH_TRIGGER
            });
        if triggered {
            npc.ai[1] = 5.0;
            npc.ai[2] = TEETH_FUSE;
            return out;
        }
    } else if world.wet {
        // In water a leaper swims: it bounces off walls and paddles upward instead of hopping.
        if npc.collide_x {
            npc.direction = -npc.direction;
            npc.sprite_direction = npc.direction;
        }
        if npc.collide_y {
            if let Some(target) = world.target {
                face(npc, target);
            }
            if npc.old_velocity.1 < 0.0 {
                npc.velocity.1 = 5.0;
            } else {
                npc.velocity.1 -= 2.0;
            }
            npc.sprite_direction = npc.direction;
        }
        if npc.velocity.1 > 4.0 {
            npc.velocity.1 *= 0.95;
        }
        npc.velocity.1 -= 0.3;
        if npc.velocity.1 < -4.0 {
            npc.velocity.1 = -4.0;
        }
    }

    if npc.velocity.1 == 0.0 {
        // Landed exactly where it took off? Then something is in the way.
        if npc.ai[3] == npc.position.0 {
            npc.direction = -npc.direction;
            npc.ai[2] = LEAPER_STUCK_WAIT;
        }
        npc.ai[3] = 0.0;
        npc.velocity.0 *= 0.8;
        if npc.velocity.0.abs() < 0.1 {
            npc.velocity.0 = 0.0;
        }
        npc.ai[0] += g.charge;

        let (cx, cy) = npc.center();
        let (dx, dy) = world
            .target
            .map_or((0.0, 0.0), |t| (t.center.0 - cx, t.center.1 - cy));
        let reach = dx.hypot(dy).max(f32::MIN_POSITIVE);
        // The closer you are the faster it winds up, up to a cap.
        let urgency = (LEAPER_URGENCY / reach * g.urgency).min(LEAPER_URGENCY_CAP);
        npc.ai[0] += urgency as i32 as f32;

        if npc.ai[0] >= 0.0 {
            if npc.ai[2] == 1.0
                && let Some(target) = world.target
            {
                face(npc, target);
            }
            npc.ai[1] += 1.0;
            let last_of_the_set = npc.ai[1] > g.hops_before_leap;
            let (across, up) = if last_of_the_set {
                npc.ai[1] = 0.0;
                g.leap
            } else {
                g.hop
            };
            npc.velocity.1 = up;
            npc.velocity.0 += across * f32::from(npc.direction);
            // At middle distance it throws in an extra shove, which is what makes one close the
            // last few tiles in a single bound rather than two.
            if (200.0..350.0).contains(&reach) {
                npc.velocity.0 += f32::from(npc.direction);
            }
            if last_of_the_set {
                npc.ai[0] = LEAPER_LONG_REST;
                npc.ai[3] = npc.position.0;
            } else {
                npc.ai[0] = LEAPER_REST;
            }
        }
        npc.sprite_direction = npc.direction;
    } else {
        let (cx, _) = npc.center();
        // A herpling directly under you stops pushing and lets itself drop onto you.
        let dropping_on_you = npc.npc_type == DERPLING
            && world.target.is_some_and(|t| {
                npc.position.1 + npc.height() < t.center.1
                    && (t.center.0 - cx).abs() < npc.width() / 2.0
            });
        if dropping_on_you {
            npc.velocity.0 *= 0.92;
            if npc.velocity.1 < 0.0 {
                npc.velocity.1 *= 0.9;
                npc.velocity.1 += 0.1;
            }
        } else if (npc.direction == 1 && npc.velocity.0 < g.air_cap)
            || (npc.direction == -1 && npc.velocity.0 > -g.air_cap)
        {
            // Leaning into the jump, but only while it is not already going that way hard.
            if (npc.direction == -1 && npc.velocity.0 < 0.1)
                || (npc.direction == 1 && npc.velocity.0 > -0.1)
            {
                npc.velocity.0 += LEAPER_AIR_LEAN * f32::from(npc.direction);
            } else {
                npc.velocity.0 *= 0.93;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc::TILE;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Ground(HashMap<(i32, i32), Tile>);

    impl TileView for Ground {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn flat(at: i32) -> Ground {
        let mut tiles = HashMap::new();
        for x in -300..300 {
            for y in at..at + 3 {
                tiles.insert((x, y), Tile::block(1));
            }
        }
        Ground(tiles)
    }

    fn world<'a>(tiles: &'a Ground, target: Option<(f32, f32)>) -> World<'a, Ground> {
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

    /// The herpling, which uses the default gait.
    const HERPLING: u16 = 174;

    fn settled(tiles: &Ground, mut npc: Npc) -> Npc {
        for _ in 0..200 {
            crate::game::npc::step_physics(&mut npc, tiles);
            if npc.on_ground && npc.velocity.1 == 0.0 {
                break;
            }
        }
        npc
    }

    fn herpling(tiles: &Ground, tile_x: i32, tile_y: i32) -> Npc {
        settled(
            tiles,
            Npc::new(HERPLING, (tile_x as f32 * TILE, tile_y as f32 * TILE), 1).expect("herpling"),
        )
    }

    /// Short hops with one long one at the end: the pattern is the enemy, not the speed.
    #[test]
    fn a_leaper_takes_short_hops_then_a_long_one() {
        let tiles = flat(30);
        let mut d = herpling(&tiles, 0, 25);
        let w = world(&tiles, Some((60.0 * TILE, 30.0 * TILE)));

        let mut launches = Vec::new();
        let mut grounded = true;
        for _ in 0..1200 {
            leaper(&mut d, &w);
            if grounded && d.velocity.1 < 0.0 {
                launches.push(d.velocity.1);
            }
            grounded = d.velocity.1 == 0.0;
            crate::game::npc::step_physics(&mut d, &tiles);
        }
        assert!(launches.len() >= 4, "expected several hops: {launches:?}");
        assert!(
            launches.iter().any(|v| *v == LEAPER_HOP.1),
            "no short hop in {launches:?}"
        );
        assert!(
            launches.iter().any(|v| *v == LEAPER_LEAP.1),
            "no long leap in {launches:?}"
        );
    }

    /// It winds up faster when you are close: standing next to one is worse than watching from
    /// across the room.
    #[test]
    fn a_leaper_winds_up_faster_the_closer_you_are() {
        let tiles = flat(30);
        let hops_in = |player_x: f32| {
            let mut d = herpling(&tiles, 0, 25);
            let w = world(&tiles, Some((player_x, 30.0 * TILE)));
            let mut hops = 0;
            let mut grounded = true;
            for _ in 0..600 {
                leaper(&mut d, &w);
                if grounded && d.velocity.1 < 0.0 {
                    hops += 1;
                }
                grounded = d.velocity.1 == 0.0;
                crate::game::npc::step_physics(&mut d, &tiles);
            }
            hops
        };
        let close = hops_in(6.0 * TILE);
        let far = hops_in(300.0 * TILE);
        assert!(
            close > far,
            "close should mean more hops: {close} vs {far}"
        );
    }

    /// Landing where it started means something is blocking it, so it turns round and waits.
    #[test]
    fn a_leaper_that_gets_nowhere_turns_round() {
        let tiles = flat(30);
        let mut d = herpling(&tiles, 0, 25);
        d.direction = 1;
        // Pretend the last leap took off from exactly here.
        d.ai[3] = d.position.0;
        d.velocity.1 = 0.0;
        leaper(&mut d, &world(&tiles, Some((500.0, 480.0))));
        assert_eq!(d.direction, -1, "it should have turned round");
        assert_eq!(d.ai[2], LEAPER_STUCK_WAIT, "and settled in to wait");
    }

    /// The teeth bomb is a leaper that ends differently: water lights it, and it goes off.
    #[test]
    fn a_teeth_bomb_lights_in_water_and_then_detonates() {
        let tiles = flat(30);
        let mut bomb = settled(
            &tiles,
            Npc::new(TEETH_BOMB, (0.0, 25.0 * TILE), 1).expect("chattering teeth bomb"),
        );
        let mut w = world(&tiles, Some((10_000.0, 0.0)));
        w.wet = true;

        leaper(&mut bomb, &w);
        assert_eq!(bomb.ai[1], 5.0, "water should light it");
        assert_eq!(bomb.ai[2], TEETH_FUSE);

        let mut went_off = false;
        for _ in 0..(TEETH_FUSE as i32 + 5) {
            if leaper(&mut bomb, &w).detonated {
                went_off = true;
                break;
            }
        }
        assert!(went_off, "the fuse should have run out");
        assert_eq!(bomb.width(), TEETH_BLAST_SIZE, "and it swells before it does");
    }

    /// Standing next to one lights it too, without any water involved.
    #[test]
    fn a_teeth_bomb_lights_when_you_walk_into_it() {
        let tiles = flat(30);
        let mut bomb = settled(
            &tiles,
            Npc::new(TEETH_BOMB, (0.0, 25.0 * TILE), 1).expect("chattering teeth bomb"),
        );
        let (cx, cy) = bomb.center();
        leaper(&mut bomb, &world(&tiles, Some((cx + 30.0, cy))));
        assert_eq!(bomb.ai[1], 5.0, "you are close enough to set it off");
    }
}
