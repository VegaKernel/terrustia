//! Rollers: style 39 — giant tortoises, ice tortoises, giant shellies and the solar sroller.
//!
//! A roller has two completely different modes and the transition between them is the fight. It
//! plods about on a charge meter that fills faster the *further away* you are and faster still
//! when it has a clear line — so backing off is what winds one up. Full, it curls, pauses half a
//! second, and launches itself at you at ten pixels a tick with double damage and double armour,
//! climbing as it goes so a slope does not save you. Ninety ticks later it slows, unrolls, and
//! spends another half second standing up, which is the window to hit it.
//!
//! The variations are real rather than cosmetic. Shellies are slower everywhere and wind up at
//! half the rate. A tortoise that gets wet charges immediately, because it will not swim. And the
//! solar sroller does not roll once — it curls shorter, bounces two to four times, and only then
//! unrolls.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    ROLLER_BLIND_SPEED, ROLLER_CLIMB, ROLLER_FAR_BONUS, ROLLER_FAR_RANGE, ROLLER_LEAD,
    ROLLER_NEAR_RANGE, ROLLER_READY, ROLLER_ROLL_DAMAGE, ROLLER_ROLL_DEFENSE, ROLLER_ROLL_SPEED,
    ROLLER_ROLL_TICKS, ROLLER_SEEN_BONUS, ROLLER_SEEN_RANGE, ROLLER_SPIN, ROLLER_SPIN_DECAY,
    ROLLER_STANDUP, ROLLER_WALK_ACCEL, ROLLER_WALK_FAR, ROLLER_WALK_NEAR, ROLLER_WET_READY,
    ROLLER_WINDUP, SHELLY_FAR_BONUS, SHELLY_FIRST, SHELLY_LAST, SHELLY_ROLL_DAMAGE,
    SHELLY_ROLL_SCALE, SHELLY_SEEN_BONUS, SHELLY_WALK, SOLAR_SROLLER, SROLLER_BLIND_SPEED,
    SROLLER_BOUNCE_LIMIT, SROLLER_CURLED_HEIGHT, SROLLER_DAMAGE, SROLLER_SPEED,
    SROLLER_STANDING_HEIGHT,
};

use crate::game::ai::{World, can_see, face};
use crate::game::npc::{Npc, TILE, TileView};

/// The phases a roller moves through, kept in `ai[0]` exactly as the game numbers them.
mod phase {
    pub const WALKING: f32 = 0.0;
    pub const WINDING_UP: f32 = 1.0;
    pub const ROLLING: f32 = 3.0;
    pub const SLOWING: f32 = 4.0;
    pub const STANDING_UP: f32 = 5.0;
    pub const BOUNCING: f32 = 6.0;
}

fn is_shelly(npc_type: u16) -> bool {
    (SHELLY_FIRST..=SHELLY_LAST).contains(&npc_type)
}

/// Style 39.
pub fn roller(npc: &mut Npc, world: &World<'_, impl TileView>, rng: &mut SmallRng) {
    npc.dirty = true;
    let shelly = is_shelly(npc.npc_type);
    let sroller = npc.npc_type == SOLAR_SROLLER;

    // Being hit resets a tortoise's wind-up, so a fight keeps interrupting the charge. The solar
    // sroller is the exception: nothing stops it once it has started.
    if world.was_hurt && !sroller {
        npc.ai[0] = phase::WALKING;
        npc.ai[1] = 0.0;
        if let Some(target) = world.target {
            face(npc, target);
        }
    }

    let Some(target) = world.target else {
        npc.velocity.0 *= 0.8;
        return;
    };
    let (cx, cy) = npc.center();
    let (dx, dy) = (target.center.0 - cx, target.center.1 - cy);
    let reach = dx.hypot(dy);
    let seen = can_see(world.tiles, npc, target);

    match npc.ai[0] {
        p if p == phase::WALKING => {
            npc.direction = match npc.velocity.0 {
                v if v < 0.0 => -1,
                v if v > 0.0 => 1,
                _ => npc.direction,
            };
            npc.sprite_direction = npc.direction;

            // The meter. Distance and a clear line both fill it; nothing else does.
            let (seen_bonus, far_bonus) = if shelly {
                (SHELLY_SEEN_BONUS, SHELLY_FAR_BONUS)
            } else {
                (ROLLER_SEEN_BONUS, ROLLER_FAR_BONUS)
            };
            if reach > ROLLER_SEEN_RANGE && seen {
                npc.ai[1] += seen_bonus;
            }
            if reach > ROLLER_FAR_RANGE
                && (seen || npc.position.1 + npc.height() > target.center.1 - 200.0)
            {
                npc.ai[1] += far_bonus;
            }
            if world.wet && !shelly {
                npc.ai[1] = ROLLER_WET_READY;
            }
            npc.defense = npc.stats.defense;
            npc.ai[1] += 1.0;
            if npc.ai[1] >= ROLLER_READY {
                npc.ai[1] = 0.0;
                npc.ai[0] = phase::WINDING_UP;
            }

            // Walked into something: turn round.
            if !world.was_hurt && npc.velocity.0 != npc.old_velocity.0 {
                npc.direction = -npc.direction;
            }
            // Nothing under the next few tiles either: turn round rather than walk off.
            if npc.velocity.1 == 0.0 && target.center.1 < npc.position.1 + npc.height() {
                let mid = ((npc.position.0 + npc.width() * 0.5) / TILE) as i32;
                let (from, to) = if npc.direction > 0 {
                    (mid, mid + 3)
                } else {
                    (mid - 3, mid)
                };
                let top = ((npc.position.1 + npc.height() + 2.0) / TILE) as i32 - 1;
                let mut floor = false;
                for x in from..=to {
                    for y in top..=top + 4 {
                        let tile = world.tiles.tile(x, y);
                        if tile.is_active() && terrustia_proto::tile_solid::solid(tile.block) {
                            floor = true;
                        }
                    }
                }
                if !floor {
                    npc.direction = -npc.direction;
                    npc.velocity.0 = 0.1 * f32::from(npc.direction);
                }
            }

            let cap = if shelly {
                SHELLY_WALK
            } else if reach < ROLLER_NEAR_RANGE {
                ROLLER_WALK_NEAR
            } else {
                ROLLER_WALK_FAR
            };
            plod(npc, cap);
        }

        p if p == phase::WINDING_UP => {
            npc.velocity.0 *= 0.5;
            npc.ai[1] += if shelly { 0.5 } else { 1.0 };
            if npc.ai[1] >= ROLLER_WINDUP {
                face(npc, target);
                npc.ai[1] = 0.0;
                npc.ai[2] = 0.0;
                if sroller {
                    // It curls up smaller, and picks how many bounces this set will be.
                    npc.resize(npc.width(), SROLLER_CURLED_HEIGHT);
                    npc.ai[0] = phase::BOUNCING;
                    npc.ai[2] = rng.random_range(2..5) as f32;
                } else {
                    npc.ai[0] = phase::ROLLING;
                }
            }
        }

        p if p == phase::ROLLING => {
            let multiplier = if shelly {
                SHELLY_ROLL_DAMAGE
            } else {
                ROLLER_ROLL_DAMAGE
            };
            npc.damage_bonus = multiplier;
            npc.defense = npc.stats.defense * ROLLER_ROLL_DEFENSE;
            npc.ai[1] += 1.0;
            if npc.ai[1] == 1.0 {
                face(npc, target);
                npc.ai[2] += ROLLER_SPIN;
                npc.ai[1] += 1.0;
                let mut speed = if seen {
                    ROLLER_ROLL_SPEED
                } else {
                    ROLLER_BLIND_SPEED
                };
                if shelly {
                    speed *= SHELLY_ROLL_SCALE;
                }
                npc.velocity = launch(npc, target, speed, seen);
                npc.ai[3] = npc.velocity.0;
            } else {
                // Right on top of you it stops pushing, so a hit does not carry it past.
                if overlapping(npc, target) {
                    npc.velocity.0 *= 0.8;
                    npc.ai[3] = 0.0;
                    if npc.velocity.1 < 0.0 {
                        npc.velocity.1 += 0.2;
                    }
                }
                // It holds its horizontal speed and climbs, which is how it comes up a slope.
                if npc.ai[3] != 0.0 {
                    npc.velocity.0 = npc.ai[3];
                    npc.velocity.1 -= ROLLER_CLIMB;
                }
                if npc.ai[1] >= ROLLER_ROLL_TICKS {
                    npc.no_gravity = false;
                    npc.ai[1] = 0.0;
                    npc.ai[0] = phase::SLOWING;
                }
            }
            npc.rotation += npc.ai[2] * f32::from(npc.direction);
        }

        p if p == phase::SLOWING => {
            npc.velocity.0 *= 0.96;
            if npc.ai[2] > 0.0 {
                npc.ai[2] -= ROLLER_SPIN_DECAY;
                npc.rotation += npc.ai[2] * f32::from(npc.direction);
            } else if npc.velocity.1 >= 0.0 {
                npc.rotation = 0.0;
            }
            if npc.ai[2] <= 0.0 && (npc.velocity.1 == 0.0 || world.wet) {
                npc.rotation = 0.0;
                npc.ai[2] = 0.0;
                npc.ai[1] = 0.0;
                npc.ai[0] = phase::STANDING_UP;
            }
        }

        p if p == phase::BOUNCING => {
            npc.damage_bonus = SROLLER_DAMAGE;
            npc.defense = npc.stats.defense * ROLLER_ROLL_DEFENSE;
            npc.knockback_immune = true;
            npc.ai[1] += 1.0;
            if npc.ai[1] == 1.0 {
                face(npc, target);
                let speed = if seen {
                    SROLLER_SPEED
                } else {
                    SROLLER_BLIND_SPEED
                };
                npc.velocity = launch(npc, target, speed, seen);
            } else {
                if overlapping(npc, target) {
                    npc.velocity.0 *= 0.9;
                    if npc.velocity.1 < 0.0 {
                        npc.velocity.1 += 0.2;
                    }
                }
                if npc.ai[2] == 0.0 || npc.ai[1] >= SROLLER_BOUNCE_LIMIT {
                    npc.ai[1] = 0.0;
                    npc.ai[0] = phase::STANDING_UP;
                }
            }
            npc.rotation += (npc.velocity.0 / 10.0 * f32::from(npc.direction))
                .clamp(-std::f32::consts::PI / 10.0, std::f32::consts::PI / 10.0);
        }

        _ => {
            if sroller {
                npc.resize(npc.width(), SROLLER_STANDING_HEIGHT);
            }
            npc.rotation = 0.0;
            npc.velocity.0 = 0.0;
            npc.damage_bonus = 1.0;
            npc.knockback_immune = false;
            npc.ai[1] += if shelly { 0.5 } else { 1.0 };
            if npc.ai[1] >= ROLLER_STANDUP {
                face(npc, target);
                npc.ai[1] = 0.0;
                npc.ai[0] = phase::WALKING;
            }
            // Landing in water sends it straight back into a roll rather than letting it swim.
            if world.wet {
                npc.ai[0] = phase::ROLLING;
                npc.ai[1] = 0.0;
            }
        }
    }
}

/// Whether the roller's box overlaps the player's, which is what "already on you" means here.
fn overlapping(npc: &Npc, target: crate::game::npc_ai::Target) -> bool {
    let (bx, by) = (
        target.center.0 - crate::game::ai::PLAYER_WIDTH as f32 / 2.0,
        target.center.1 - crate::game::ai::PLAYER_HEIGHT as f32 / 2.0,
    );
    npc.position.0 + npc.width() > bx
        && npc.position.0 < bx + crate::game::ai::PLAYER_WIDTH as f32
        && npc.position.1 < by + crate::game::ai::PLAYER_HEIGHT as f32
}

/// The launch vector for a roll: aimed slightly above the player, or simply upward when the shot
/// is blocked, which is what makes a blind tortoise pop over the lip of a ledge.
fn launch(npc: &Npc, target: crate::game::npc_ai::Target, speed: f32, seen: bool) -> (f32, f32) {
    let (cx, cy) = npc.center();
    let across = target.center.0 - cx;
    let lead = if npc.direction_y > 0 {
        0.0
    } else {
        across.abs() * ROLLER_LEAD
    };
    let up = target.center.1 - crate::game::ai::PLAYER_HEIGHT as f32 / 2.0 - cy - lead;
    let length = across.hypot(up).max(f32::MIN_POSITIVE);
    let scale = speed / length;
    if seen {
        (across * scale, up * scale)
    } else {
        (across * scale, -10.0)
    }
}

/// The plodding walk: it either brakes or eases up to `cap`, never both in one tick.
fn plod(npc: &mut Npc, cap: f32) {
    if npc.velocity.0 < -cap || npc.velocity.0 > cap {
        if npc.velocity.1 == 0.0 {
            npc.velocity.0 *= 0.8;
            npc.velocity.1 *= 0.8;
        }
    } else if npc.velocity.0 < cap && npc.direction == 1 {
        npc.velocity.0 = (npc.velocity.0 + ROLLER_WALK_ACCEL).min(cap);
    } else if npc.velocity.0 > -cap && npc.direction == -1 {
        npc.velocity.0 = (npc.velocity.0 - ROLLER_WALK_ACCEL).max(-cap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Cave(HashMap<(i32, i32), Tile>);

    impl TileView for Cave {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn floor(at: i32) -> Cave {
        let mut tiles = HashMap::new();
        for x in -400..400 {
            for y in at..at + 3 {
                tiles.insert((x, y), Tile::block(1));
            }
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

    const GIANT_TORTOISE: u16 = 153;

    fn tortoise(tiles: &Cave, tile_x: i32, tile_y: i32) -> Npc {
        let mut npc = Npc::new(
            GIANT_TORTOISE,
            (tile_x as f32 * TILE, tile_y as f32 * TILE),
            1,
        )
        .expect("giant tortoise");
        for _ in 0..200 {
            crate::game::npc::step_physics(&mut npc, tiles);
            if npc.on_ground && npc.velocity.1 == 0.0 {
                break;
            }
        }
        npc
    }

    /// Backing off is what winds one up. Standing next to it is not.
    #[test]
    fn distance_is_what_charges_a_tortoise() {
        let tiles = floor(30);
        let mut rng = SmallRng::seed_from_u64(39);
        let ticks_to_roll = |player_x: f32, rng: &mut SmallRng| {
            let mut t = tortoise(&tiles, 0, 25);
            let w = world(&tiles, Some((player_x, 29.0 * TILE)));
            for tick in 0..3000 {
                roller(&mut t, &w, rng);
                if t.ai[0] != phase::WALKING {
                    return tick;
                }
            }
            i32::MAX
        };
        let close = ticks_to_roll(2.0 * TILE, &mut rng);
        let far = ticks_to_roll(60.0 * TILE, &mut rng);
        assert!(
            far < close,
            "backing off should wind it up faster: {far} vs {close}"
        );
    }

    /// Rolling is the dangerous state: it hits harder, soaks more, and moves at speed.
    #[test]
    fn a_rolling_tortoise_hits_harder_and_soaks_more() {
        let tiles = floor(30);
        let mut rng = SmallRng::seed_from_u64(2);
        let mut t = tortoise(&tiles, 0, 25);
        let w = world(&tiles, Some((20.0 * TILE, 29.0 * TILE)));

        // Straight into the roll rather than waiting out the meter.
        t.ai[0] = phase::WINDING_UP;
        for _ in 0..(ROLLER_WINDUP as i32 + 2) {
            roller(&mut t, &w, &mut rng);
        }
        assert_eq!(t.ai[0], phase::ROLLING, "it should have committed");

        roller(&mut t, &w, &mut rng);
        assert_eq!(t.defense, t.stats.defense * ROLLER_ROLL_DEFENSE);
        assert_eq!(t.damage_bonus, ROLLER_ROLL_DAMAGE);
        let speed = t.velocity.0.hypot(t.velocity.1);
        assert!(
            (speed - ROLLER_ROLL_SPEED).abs() < 0.5,
            "it should launch at rolling speed, got {speed}"
        );
        assert!(t.velocity.0 > 0.0, "and toward the player");
    }

    /// The roll ends, and standing back up is the window where it is vulnerable again.
    #[test]
    fn a_roll_ends_and_the_tortoise_stands_back_up() {
        let tiles = floor(30);
        let mut rng = SmallRng::seed_from_u64(5);
        let mut t = tortoise(&tiles, 0, 25);
        let w = world(&tiles, Some((20.0 * TILE, 29.0 * TILE)));
        t.ai[0] = phase::ROLLING;

        let mut reached_walking = false;
        for _ in 0..600 {
            roller(&mut t, &w, &mut rng);
            crate::game::npc::step_physics(&mut t, &tiles);
            if t.ai[0] == phase::WALKING {
                reached_walking = true;
                break;
            }
        }
        assert!(reached_walking, "it should come back to walking");
        assert_eq!(t.damage_bonus, 1.0, "and stop hitting like a boulder");
        // Armour is restored by the walking branch, so it takes one more tick to show up — which
        // is exactly the beat where a rolling tortoise is briefly still armoured but no longer
        // moving.
        roller(&mut t, &w, &mut rng);
        assert_eq!(t.defense, t.stats.defense, "and be soft again");
    }

    /// Being hit interrupts the wind-up, which is why hitting one keeps it from ever rolling.
    #[test]
    fn hitting_a_tortoise_resets_its_charge() {
        let tiles = floor(30);
        let mut rng = SmallRng::seed_from_u64(9);
        let mut t = tortoise(&tiles, 0, 25);
        let mut w = world(&tiles, Some((60.0 * TILE, 29.0 * TILE)));
        for _ in 0..50 {
            roller(&mut t, &w, &mut rng);
        }
        assert!(t.ai[1] > 0.0, "it should have some charge");
        let charged = t.ai[1];
        w.was_hurt = true;
        roller(&mut t, &w, &mut rng);
        // The hit empties the meter, and then the same tick's walking branch starts filling it
        // again — so the meter goes *backwards*, which it otherwise never does, rather than to
        // exactly zero.
        assert!(
            t.ai[1] < charged,
            "a hit should have dumped the charge: {} was {charged}",
            t.ai[1]
        );
        // And it is back to one tick's worth, not to most of what it had.
        let one_tick = ROLLER_SEEN_BONUS + ROLLER_FAR_BONUS + 1.0;
        assert_eq!(t.ai[1], one_tick, "back to a single tick of charge");
    }

    /// A tortoise that ends up in water charges at once rather than swimming.
    #[test]
    fn water_makes_a_tortoise_charge_immediately() {
        let tiles = floor(30);
        let mut rng = SmallRng::seed_from_u64(1);
        let mut t = tortoise(&tiles, 0, 25);
        let mut w = world(&tiles, Some((4.0 * TILE, 29.0 * TILE)));
        w.wet = true;
        roller(&mut t, &w, &mut rng);
        assert_eq!(t.ai[0], phase::WINDING_UP, "it should curl up at once");
    }

    /// The solar sroller curls shorter and bounces several times instead of rolling once.
    #[test]
    fn a_solar_sroller_curls_up_and_bounces() {
        let tiles = floor(30);
        let mut rng = SmallRng::seed_from_u64(417);
        let mut s = Npc::new(SOLAR_SROLLER, (0.0, 25.0 * TILE), 1).expect("solar sroller");
        let tall = s.height();
        let w = world(&tiles, Some((20.0 * TILE, 29.0 * TILE)));
        s.ai[0] = phase::WINDING_UP;
        for _ in 0..(ROLLER_WINDUP as i32 + 2) {
            roller(&mut s, &w, &mut rng);
        }
        assert_eq!(s.ai[0], phase::BOUNCING, "it bounces rather than rolls");
        assert_eq!(s.height(), SROLLER_CURLED_HEIGHT, "curled up it is shorter");
        assert!(tall > SROLLER_CURLED_HEIGHT);
        assert!(
            (2.0..5.0).contains(&s.ai[2]),
            "two to four bounces, got {}",
            s.ai[2]
        );
    }
}
