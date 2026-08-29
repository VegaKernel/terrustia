//! Chargers: style 74 — the Martian drone and the solar corite.
//!
//! A charger does not chase. It takes up a station above and to the side of you, waits until it
//! has a clear line that is not too steep, freezes for half a second while it aims, and then
//! throws itself along that line. The freeze is the tell, and it is the only warning you get.
//!
//! What happens at the end of the dash is the difference between the two: a drone detonates,
//! swelling to a hundred and ninety-two pixels across and taking whatever is inside that with it,
//! while a corite bounces off, drifts to a stop and lines up again. A drone also goes off if you
//! simply walk into one, so killing them at range is the whole counterplay.
//!
//! One detail keeps a charger from stalling: if it cannot find a clear line for two seconds it
//! stops waiting for one and commits anyway.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    CHARGE_ARC, CHARGE_PATIENCE, CORITE_CHARGE, CORITE_REST, Charge, DRONE_BLAST_DAMAGE,
    DRONE_BLAST_SIZE, DRONE_BLAST_TICKS, DRONE_CHARGE, DRONE_TOUCH, MARTIAN_DRONE,
};

use super::drifters::Outcome;
use crate::game::ai::{World, can_see};
use crate::game::npc::{Npc, TileView};

/// The phases, numbered as the game numbers them in `ai[0]`.
mod phase {
    pub const DRIFTING: f32 = 0.0;
    pub const AIMING: f32 = 1.0;
    pub const DASHING: f32 = 2.0;
    pub const GOING_OFF: f32 = 3.0;
    pub const RECOVERING: f32 = 4.0;
}

fn table(npc_type: u16) -> Charge {
    if npc_type == MARTIAN_DRONE {
        DRONE_CHARGE
    } else {
        CORITE_CHARGE
    }
}

/// Style 74.
pub fn charger(npc: &mut Npc, world: &World<'_, impl TileView>, rng: &mut SmallRng) -> Outcome {
    let mut out = Outcome::default();
    npc.dirty = true;
    let c = table(npc.npc_type);

    // It points where it is going, but never upside down.
    npc.rotation = npc.velocity.1.atan2(npc.velocity.0);
    if npc.rotation < -std::f32::consts::FRAC_PI_2 {
        npc.rotation += std::f32::consts::PI;
    }
    if npc.rotation > std::f32::consts::FRAC_PI_2 {
        npc.rotation -= std::f32::consts::PI;
    }
    if npc.velocity.0 != 0.0 {
        let sign = npc.velocity.0.signum() as i8;
        // A drone's sprite faces the way it came from; a corite's faces the way it is going.
        npc.sprite_direction = if npc.npc_type == MARTIAN_DRONE {
            -sign
        } else {
            sign
        };
    }

    let Some(target) = world.target.filter(|t| t.alive) else {
        npc.velocity.0 *= 0.98;
        npc.velocity.1 *= 0.98;
        return out;
    };
    let (cx, cy) = npc.center();
    let to_player = (target.center.0 - cx, target.center.1 - cy);
    let reach = to_player.0.hypot(to_player.1);

    match npc.ai[0] {
        p if p == phase::DRIFTING => {
            npc.knockback_immune = false;
            // The station: above the player, and off to whichever side it is already on.
            let station = (
                to_player.0
                    + if to_player.0 < 0.0 {
                        c.beside
                    } else {
                        -c.beside
                    },
                to_player.1 - c.above,
            );
            let straight = unit(to_player, c.approach);
            let toward_station = unit(station, c.approach);

            // A clear line, and not one that is nearly vertical: a charger will not dive.
            let angle = straight.1.atan2(straight.0);
            let limit = std::f32::consts::PI / CHARGE_ARC;
            let usable = (can_see(world.tiles, npc, target) || npc.ai[3] >= CHARGE_PATIENCE)
                && angle > limit
                && angle < std::f32::consts::PI - limit;
            let too_close = reach < c.too_close;
            let too_far = reach > c.too_far;

            if too_close || too_far || !usable {
                // Not yet. Ease toward the station.
                npc.velocity.0 =
                    (npc.velocity.0 * (c.drift_smooth - 1.0) + toward_station.0) / c.drift_smooth;
                npc.velocity.1 =
                    (npc.velocity.1 * (c.drift_smooth - 1.0) + toward_station.1) / c.drift_smooth;
                // Blocked rather than merely out of position: start losing patience.
                if !usable {
                    if !too_close && !too_far {
                        npc.ai[3] += 1.0;
                    }
                } else {
                    npc.ai[3] = 0.0;
                }
            } else {
                // Committed. The line is remembered now and not recomputed.
                npc.ai[0] = phase::AIMING;
                npc.ai[2] = straight.0;
                npc.ai[3] = straight.1;
            }
        }

        p if p == phase::AIMING => {
            npc.knockback_immune = true;
            // A corite that has almost stopped drifts gently onward instead of freezing solid.
            let braking = if npc.npc_type == MARTIAN_DRONE {
                true
            } else {
                let moving = npc.velocity.0.hypot(npc.velocity.1) > 2.0;
                if !moving {
                    let nudge = unit(to_player, 0.1);
                    npc.velocity.0 += (nudge.0 - npc.velocity.0) * 0.25;
                    npc.velocity.1 += (nudge.1 - npc.velocity.1) * 0.25;
                }
                moving
            };
            if braking {
                npc.velocity.0 *= c.windup_drag;
                npc.velocity.1 *= c.windup_drag;
            }
            npc.ai[1] += 1.0;
            if npc.ai[1] >= c.windup {
                npc.ai[0] = phase::DASHING;
                npc.ai[1] = 0.0;
                // The remembered line, scattered a little, at full speed.
                let scatter = |rng: &mut SmallRng| {
                    if c.scatter == 0 {
                        0.0
                    } else {
                        rng.random_range(-c.scatter..=c.scatter) as f32 * 0.04
                    }
                };
                let aim = (npc.ai[2] + scatter(rng), npc.ai[3] + scatter(rng));
                npc.velocity = unit(aim, c.dash);
            }
        }

        p if p == phase::DASHING => {
            npc.knockback_immune = true;
            npc.ai[1] += 1.0;
            // It only gives up once it is past you *and* above you, or once it has run out of
            // speed — which is why one that misses keeps going rather than turning on the spot.
            let past = reach > c.break_off && cy > target.center.1;
            let spent = npc.velocity.0.hypot(npc.velocity.1) < c.spent_below;
            if (npc.ai[1] >= c.dash_ticks && past) || spent {
                npc.ai = [phase::DRIFTING, 0.0, 0.0, 0.0];
                npc.velocity.0 /= 2.0;
                npc.velocity.1 /= 2.0;
                if npc.npc_type != MARTIAN_DRONE {
                    npc.ai[0] = phase::RECOVERING;
                    npc.ai[1] = CORITE_REST;
                }
            } else {
                // Still steering, and gaining speed as it goes.
                let mut aim = to_player;
                let length = aim.0.hypot(aim.1);
                if length > 0.0 {
                    aim = (aim.0 / length, aim.1 / length);
                } else {
                    aim = (f32::from(npc.direction), 0.0);
                }
                let speed = npc.velocity.0.hypot(npc.velocity.1) + c.steer_gain * c.steer;
                npc.velocity.0 = (npc.velocity.0 * (c.steer - 1.0) + aim.0 * speed) / c.steer;
                npc.velocity.1 = (npc.velocity.1 * (c.steer - 1.0) + aim.1 * speed) / c.steer;
            }
            // A drone that hits terrain mid-dash goes off there.
            if c.explodes && (npc.collide_x || npc.collide_y) {
                npc.ai = [phase::GOING_OFF, 0.0, 0.0, 0.0];
            }
        }

        p if p == phase::RECOVERING => {
            // Counting down three at a time, so the rest is a third as long as the number says.
            npc.ai[1] -= 3.0;
            if npc.ai[1] <= 0.0 {
                npc.ai[0] = phase::DRIFTING;
                npc.ai[1] = 0.0;
            }
            npc.velocity.0 *= 0.95;
            npc.velocity.1 *= 0.95;
        }

        _ => {}
    }

    // Walking into a drone sets it off wherever it is in its cycle.
    if c.explodes && npc.ai[0] != phase::GOING_OFF && reach < DRONE_TOUCH {
        npc.ai = [phase::GOING_OFF, 0.0, 0.0, 0.0];
    }

    if npc.ai[0] == phase::GOING_OFF {
        // The blast *is* the hitbox: it swells, hits everything inside it, and is gone.
        npc.resize(DRONE_BLAST_SIZE, DRONE_BLAST_SIZE);
        npc.velocity = (0.0, 0.0);
        npc.damage_bonus = DRONE_BLAST_DAMAGE as f32 / npc.stats.damage.max(1) as f32;
        npc.alpha = 255;
        npc.ai[1] += 1.0;
        if npc.ai[1] >= DRONE_BLAST_TICKS {
            out.spent = true;
            out.detonated = true;
        }
    }
    out
}

/// A vector of length `speed` in the direction of `v`, or nothing when `v` has no direction.
fn unit(v: (f32, f32), speed: f32) -> (f32, f32) {
    let length = v.0.hypot(v.1);
    if length <= 0.0 || !length.is_finite() {
        (0.0, 0.0)
    } else {
        (v.0 / length * speed, v.1 / length * speed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Open(HashMap<(i32, i32), Tile>);

    impl TileView for Open {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn open() -> Open {
        Open(HashMap::new())
    }

    fn world<'a>(tiles: &'a Open, target: Option<(f32, f32)>) -> World<'a, Open> {
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

    const SOLAR_CORITE_TYPE: u16 = 418;

    fn drone(x: f32, y: f32) -> Npc {
        Npc::new(MARTIAN_DRONE, (x, y), 1).expect("martian drone")
    }

    /// The cycle: drift, freeze, dash. The freeze is what makes one readable.
    #[test]
    fn a_charger_lines_up_before_it_commits() {
        let tiles = open();
        let mut rng = SmallRng::seed_from_u64(74);
        let mut d = drone(0.0, 0.0);
        // Off to the side and above, which is where it wants to be.
        let w = world(&tiles, Some((400.0, 300.0)));

        // A drone that already has a clear line commits on its first tick, so the starting phase
        // has to be recorded before any of them run.
        let mut phases = vec![d.ai[0]];
        for _ in 0..600 {
            charger(&mut d, &w, &mut rng);
            if phases.last() != Some(&d.ai[0]) {
                phases.push(d.ai[0]);
            }
            d.position.0 += d.velocity.0;
            d.position.1 += d.velocity.1;
        }
        assert!(
            phases
                .windows(2)
                .any(|w| w == [phase::DRIFTING, phase::AIMING]),
            "it should have lined up: {phases:?}"
        );
        assert!(
            phases
                .windows(2)
                .any(|w| w == [phase::AIMING, phase::DASHING]),
            "and then committed: {phases:?}"
        );
    }

    /// While aiming it is nearly still, and while dashing it is fast. Those are the two speeds
    /// that matter, and they have to be different enough to read.
    #[test]
    fn aiming_is_slow_and_dashing_is_fast() {
        let tiles = open();
        let mut rng = SmallRng::seed_from_u64(3);
        let mut d = drone(0.0, 0.0);
        let w = world(&tiles, Some((400.0, 300.0)));

        d.ai[0] = phase::AIMING;
        d.ai[2] = 1.0;
        d.ai[3] = 0.0;
        d.velocity = (6.0, 0.0);
        let mut slowest = f32::MAX;
        for _ in 0..(DRONE_CHARGE.windup as i32) {
            charger(&mut d, &w, &mut rng);
            if d.ai[0] == phase::AIMING {
                slowest = slowest.min(d.velocity.0.hypot(d.velocity.1));
            }
        }
        assert!(
            slowest < 2.0,
            "the wind-up should nearly stop it: {slowest}"
        );
        assert_eq!(d.ai[0], phase::DASHING, "and then it goes");
        let launched = d.velocity.0.hypot(d.velocity.1);
        assert!(
            (launched - DRONE_CHARGE.dash).abs() < 1.0,
            "at dash speed, got {launched}"
        );
    }

    /// Walking into a drone sets it off, and the blast is its hitbox.
    #[test]
    fn a_drone_goes_off_when_you_touch_it() {
        let tiles = open();
        let mut rng = SmallRng::seed_from_u64(1);
        let mut d = drone(0.0, 0.0);
        let (cx, cy) = d.center();
        let w = world(&tiles, Some((cx + 30.0, cy)));

        charger(&mut d, &w, &mut rng);
        assert_eq!(d.ai[0], phase::GOING_OFF, "that is close enough");
        assert_eq!(d.width(), DRONE_BLAST_SIZE, "the blast is the hitbox");

        let mut went_off = false;
        for _ in 0..10 {
            if charger(&mut d, &w, &mut rng).detonated {
                went_off = true;
                break;
            }
        }
        assert!(went_off, "and it should not linger");
    }

    /// A drone's sprite faces the way it came from (mirrored); a corite's faces the way it is
    /// actually going — the two are not the same sign.
    #[test]
    fn a_corite_faces_the_way_it_is_going_unlike_a_drone() {
        let tiles = open();
        let mut rng = SmallRng::seed_from_u64(2);
        let w = world(&tiles, None);

        let mut d = drone(0.0, 0.0);
        d.velocity.0 = 5.0;
        charger(&mut d, &w, &mut rng);
        assert_eq!(d.sprite_direction, -1, "a drone mirrors its heading");

        let mut c = Npc::new(SOLAR_CORITE_TYPE, (0.0, 0.0), 1).expect("solar corite");
        c.velocity.0 = 5.0;
        charger(&mut c, &w, &mut rng);
        assert_eq!(c.sprite_direction, 1, "a corite does not");
    }

    /// A corite does not detonate; it recovers and lines up again.
    #[test]
    fn a_corite_bounces_off_and_tries_again() {
        let tiles = open();
        let mut rng = SmallRng::seed_from_u64(418);
        let mut c = Npc::new(SOLAR_CORITE_TYPE, (0.0, 0.0), 1).expect("solar corite");
        let (cx, cy) = c.center();
        let w = world(&tiles, Some((cx + 30.0, cy)));

        // Close enough to set a drone off; a corite should carry on regardless.
        for _ in 0..30 {
            let out = charger(&mut c, &w, &mut rng);
            assert!(!out.detonated, "a corite never goes off");
        }
        assert_ne!(c.ai[0], phase::GOING_OFF);

        // Ending a dash puts it into its rest rather than killing it.
        c.ai[0] = phase::DASHING;
        c.ai[1] = CORITE_CHARGE.dash_ticks;
        c.velocity = (0.0, 0.0);
        charger(&mut c, &w, &mut rng);
        assert_eq!(c.ai[0], phase::RECOVERING, "it should be recovering");
    }

    /// A charger that cannot find a clear line eventually commits anyway rather than circling for
    /// ever behind a wall.
    #[test]
    fn a_blocked_charger_runs_out_of_patience() {
        // A wall right between them.
        let mut tiles = HashMap::new();
        for y in -40..40 {
            tiles.insert((12, y), Tile::block(1));
        }
        let tiles = Open(tiles);
        let mut rng = SmallRng::seed_from_u64(9);
        let mut d = drone(0.0, 0.0);
        let w = world(&tiles, Some((400.0, 300.0)));

        let mut committed = false;
        for _ in 0..900 {
            charger(&mut d, &w, &mut rng);
            if d.ai[0] != phase::DRIFTING {
                committed = true;
                break;
            }
        }
        assert!(committed, "it should give up waiting and go anyway");
    }
}
