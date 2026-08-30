//! Mothron: style 88.
//!
//! The eclipse's boss holds two hundred pixels above you and, every three seconds, picks one of
//! three things at random — and hitting it hurries that choice along, so a Mothron under pressure
//! attacks more often rather than less.
//!
//! * **Chase** — it comes straight at you, accelerating for as long as it keeps you in sight, and
//!   hits at *half* strength while doing it. Losing the line ends it.
//! * **Sweep** — it draws four hundred pixels off to one side, lines up level with you, and comes
//!   across at sixteen pixels a tick and rising, hitting a third harder. It only breaks off once it
//!   is well past you.
//! * **Lay** — it drops an egg on a floor near you. Seven of its brood at once is the limit.
//!
//! Out of an eclipse it does not fight at all: it climbs away and leaves.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    MOTHRON_ABOVE, MOTHRON_BROOD, MOTHRON_CHASE_ACCEL, MOTHRON_CHASE_ACCEL_EXPERT,
    MOTHRON_CHASE_BASE, MOTHRON_CHASE_DAMAGE, MOTHRON_CHASE_GAIN, MOTHRON_CHASE_SMOOTH,
    MOTHRON_CHASE_TICKS, MOTHRON_CROSS_GAIN, MOTHRON_CROSS_SMOOTH, MOTHRON_CROSS_SPEED,
    MOTHRON_DECIDE_TICKS, MOTHRON_EGG, MOTHRON_FAR, MOTHRON_HIT_HURRY, MOTHRON_HOVER_HOLD,
    MOTHRON_HOVER_SMOOTH, MOTHRON_HOVER_SPEED, MOTHRON_LAY_ARRIVE, MOTHRON_LAY_RANGE_X,
    MOTHRON_LAY_RANGE_Y, MOTHRON_LAY_SPEED_BASE, MOTHRON_LAY_SPEED_CAP, MOTHRON_LAY_SPEED_GAIN,
    MOTHRON_LOSE, MOTHRON_REACQUIRE, MOTHRON_RELAY_ODDS, MOTHRON_SETTLE_ARRIVE,
    MOTHRON_SETTLE_SPEED_CAP, MOTHRON_SETTLE_WAIT, MOTHRON_SETTLE_WAIT_EXPERT, MOTHRON_SWEEP_ACCEL,
    MOTHRON_SWEEP_AIM_SMOOTH, MOTHRON_SWEEP_AIM_SPEED, MOTHRON_SWEEP_AIM_TICKS,
    MOTHRON_SWEEP_DAMAGE, MOTHRON_SWEEP_DRAW_ACCEL, MOTHRON_SWEEP_DRAW_SMOOTH,
    MOTHRON_SWEEP_DRAW_SPEED, MOTHRON_SWEEP_OFFSET, MOTHRON_SWEEP_PAST, MOTHRON_SWEEP_READY_X,
    MOTHRON_SWEEP_READY_Y,
};

use super::drifters::Outcome;
use crate::game::ai::{World, can_see, face};
use crate::game::npc::{Npc, TILE, TileView};
use crate::game::npc_ai::Spawn;

/// The states, as `ai[0]` numbers them.
mod state {
    pub const LEAVING: f32 = -1.0;
    pub const HOVERING: f32 = 0.0;
    pub const CROSSING: f32 = 1.0;
    pub const CHASING: f32 = 2.0;
    pub const DRAWING_OFF: f32 = 3.0;
    pub const LINING_UP: f32 = 3.1;
    pub const SWEEPING: f32 = 3.2;
    /// Laying is not instant: it picks a spot, flies down to it, hovers there while the egg
    /// actually appears, and only then goes back to hovering over you.
    pub const LAYING: f32 = 4.0;
    pub const DESCENDING: f32 = 4.1;
    pub const SETTLING: f32 = 4.2;
}

/// Style 88.
///
/// `brood` is how many eggs and spawns are already out; it will not add to a full brood.
pub fn mothron(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    brood: usize,
    rng: &mut SmallRng,
) -> Outcome {
    let mut out = Outcome::default();
    npc.dirty = true;
    npc.no_tile_collide = false;
    npc.no_gravity = true;
    npc.knockback_immune = false;
    npc.damage_bonus = 1.0;

    // Out of an eclipse there is nothing for it here.
    if !world.conditions.eclipse {
        npc.ai[0] = state::LEAVING;
    }

    let target = world.target.filter(|t| t.alive);
    match target {
        None => npc.ai[0] = state::LEAVING,
        Some(t) => {
            let (cx, cy) = npc.center();
            let reach = (t.center.0 - cx).hypot(t.center.1 - cy);
            if reach > 3000.0 {
                npc.ai[0] = state::LEAVING;
            } else if npc.ai[0] > state::CROSSING && reach > MOTHRON_LOSE {
                // Lost you mid-attack: fall back to crossing the map.
                npc.ai[0] = state::CROSSING;
            }
        }
    }

    if npc.ai[0] == state::LEAVING {
        ease(&mut npc.velocity, (0.0, -8.0), 10.0);
        npc.no_tile_collide = true;
        npc.invulnerable = true;
        return out;
    }
    let Some(target) = target else { return out };
    let (cx, cy) = npc.center();
    let to_player = (target.center.0 - cx, target.center.1 - cy);
    let reach = to_player.0.hypot(to_player.1);
    let seen = can_see(world.tiles, npc, target);

    match npc.ai[0] {
        s if s == state::HOVERING => {
            face(npc, target);
            npc.rotation = (npc.rotation * 9.0 + npc.velocity.0 * 0.1) / 10.0;
            bounce(npc);

            // Its station is above you, and it holds it rather than closing.
            let station = (to_player.0, to_player.1 - MOTHRON_ABOVE);
            let gap = station.0.hypot(station.1);
            if gap > MOTHRON_FAR {
                npc.ai = [state::CROSSING, 0.0, 0.0, 0.0];
                return out;
            }
            if gap > MOTHRON_HOVER_HOLD {
                ease(
                    &mut npc.velocity,
                    unit(station, MOTHRON_HOVER_SPEED),
                    MOTHRON_HOVER_SMOOTH,
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
            if world.was_hurt {
                // Being hit brings the next attack forward.
                npc.ai[1] += rng.random_range(MOTHRON_HIT_HURRY.0..MOTHRON_HIT_HURRY.1) as f32;
            }
            if npc.ai[1] >= MOTHRON_DECIDE_TICKS {
                npc.ai = [state::HOVERING, 0.0, 0.0, 0.0];
                // It keeps rolling until it lands on an attack it can actually use: no chase
                // without a clear line, no laying with a full brood.
                for _ in 0..32 {
                    match rng.random_range(0..3) {
                        0 if seen => npc.ai[0] = state::CHASING,
                        1 => npc.ai[0] = state::DRAWING_OFF,
                        2 if brood < terrustia_proto::npc_params::MOTHRON_BROOD => {
                            npc.ai[0] = state::LAYING;
                        }
                        _ => continue,
                    }
                    break;
                }
            }
        }

        s if s == state::CROSSING => {
            // Coming back through the world, gaining speed with distance.
            npc.no_tile_collide = true;
            npc.knockback_immune = true;
            npc.direction = match npc.velocity.0 {
                v if v < 0.0 => -1,
                v if v > 0.0 => 1,
                _ => npc.direction,
            };
            npc.sprite_direction = npc.direction;
            npc.rotation = (npc.rotation * 9.0 + npc.velocity.0 * 0.08) / 10.0;
            if reach < MOTHRON_REACQUIRE && !boxed(world.tiles, npc) {
                npc.ai = [state::HOVERING, 0.0, 0.0, 0.0];
            }
            let speed = MOTHRON_CROSS_SPEED + reach / MOTHRON_CROSS_GAIN;
            ease(
                &mut npc.velocity,
                unit(to_player, speed),
                MOTHRON_CROSS_SMOOTH,
            );
        }

        s if s == state::CHASING => {
            // Straight at you, accelerating, at half damage.
            npc.damage_bonus = MOTHRON_CHASE_DAMAGE;
            npc.knockback_immune = true;
            npc.direction = if target.center.0 - 10.0 < cx { -1 } else { 1 };
            npc.sprite_direction = npc.direction;
            npc.rotation = (npc.rotation * 4.0 + npc.velocity.0 * 0.1) / 5.0;
            bounce(npc);

            npc.ai[2] += if world.conditions.expert {
                MOTHRON_CHASE_ACCEL + MOTHRON_CHASE_ACCEL_EXPERT
            } else {
                MOTHRON_CHASE_ACCEL
            };
            let aim = (to_player.0, to_player.1 - 20.0);
            let speed = MOTHRON_CHASE_BASE + npc.ai[2] + aim.0.hypot(aim.1) / MOTHRON_CHASE_GAIN;
            ease(&mut npc.velocity, unit(aim, speed), MOTHRON_CHASE_SMOOTH);

            npc.ai[1] += 1.0;
            if npc.ai[1] > MOTHRON_CHASE_TICKS || !seen {
                npc.ai = [state::HOVERING, 0.0, 0.0, 0.0];
            }
        }

        s if s == state::DRAWING_OFF => {
            // Backing away to one side to set up the sweep.
            npc.knockback_immune = true;
            npc.no_tile_collide = true;
            npc.direction = if npc.velocity.0 < 0.0 { -1 } else { 1 };
            npc.sprite_direction = npc.direction;
            npc.rotation = (npc.rotation * 4.0 + npc.velocity.0 * 0.07) / 5.0;

            let mut aim = (to_player.0, to_player.1 - 12.0);
            // Away from the player rather than toward: this is the run-up.
            aim.0 += if cx > target.center.0 {
                MOTHRON_SWEEP_OFFSET
            } else {
                -MOTHRON_SWEEP_OFFSET
            };
            if (cx - target.center.0).abs() > MOTHRON_SWEEP_READY_X
                && (cy - target.center.1).abs() < MOTHRON_SWEEP_READY_Y
            {
                npc.ai[0] = state::LINING_UP;
                npc.ai[1] = 0.0;
                return out;
            }
            npc.ai[1] += MOTHRON_SWEEP_DRAW_ACCEL;
            let speed = MOTHRON_SWEEP_DRAW_SPEED + npc.ai[1];
            ease(
                &mut npc.velocity,
                unit(aim, speed),
                MOTHRON_SWEEP_DRAW_SMOOTH,
            );
        }

        s if s == state::LINING_UP => {
            npc.knockback_immune = true;
            npc.no_tile_collide = true;
            npc.rotation = (npc.rotation * 4.0 + npc.velocity.0 * 0.07) / 5.0;
            let aim = (to_player.0, to_player.1 - 12.0);
            let wanted = unit(aim, MOTHRON_SWEEP_AIM_SPEED);
            ease(&mut npc.velocity, wanted, MOTHRON_SWEEP_AIM_SMOOTH);
            npc.direction = if npc.velocity.0 < 0.0 { -1 } else { 1 };
            npc.sprite_direction = npc.direction;
            npc.ai[1] += 1.0;
            if npc.ai[1] > MOTHRON_SWEEP_AIM_TICKS {
                npc.velocity = wanted;
                npc.direction = if npc.velocity.0 < 0.0 { -1 } else { 1 };
                npc.ai[0] = state::SWEEPING;
                // `ai[1]` becomes the direction of the sweep from here on.
                npc.ai[1] = f32::from(npc.direction);
            }
        }

        s if s == state::SWEEPING => {
            // A straight run across, gaining speed, at a third more damage. It does not steer.
            npc.damage_bonus = MOTHRON_SWEEP_DAMAGE;
            npc.knockback_immune = true;
            npc.no_tile_collide = true;
            npc.ai[2] += MOTHRON_SWEEP_ACCEL;
            npc.velocity.0 = (MOTHRON_SWEEP_AIM_SPEED + npc.ai[2]) * npc.ai[1];
            let past = (npc.ai[1] > 0.0 && cx > target.center.0 + MOTHRON_SWEEP_PAST)
                || (npc.ai[1] < 0.0 && cx < target.center.0 - MOTHRON_SWEEP_PAST);
            if past {
                if !boxed(world.tiles, npc) {
                    npc.ai = [state::HOVERING, 0.0, 0.0, 0.0];
                } else if (cx - target.center.0).abs() > MOTHRON_FAR {
                    npc.ai = [state::CROSSING, 0.0, 0.0, 0.0];
                }
            }
            npc.rotation = (npc.rotation * 4.0 + npc.velocity.0 * 0.07) / 5.0;
        }

        s if s == state::LAYING => {
            // Pick a spot first. `ai[1]`/`ai[2]` become the tile it is heading for, and `ai[3]`
            // starts counting the moment it actually gets there.
            face(npc, target);
            if let Some(at) = egg_spot(world, target.center, rng) {
                npc.ai = [state::DESCENDING, at.0, at.1, 0.0];
            } else {
                // Nowhere to put one: back to hovering rather than sitting here forever.
                npc.ai = [state::HOVERING, 0.0, 0.0, 0.0];
            }
        }

        s if s == state::DESCENDING => {
            // Flying down to the spot it picked — the punish window, since it is not attacking
            // and not hovering out of reach while this plays out.
            npc.no_tile_collide = true;
            npc.direction = if npc.velocity.0 < -2.0 {
                -1
            } else if npc.velocity.0 > 2.0 {
                1
            } else {
                npc.direction
            };
            npc.rotation = (npc.rotation * 9.0 + npc.velocity.0 * 0.1) / 10.0;

            let spot = lay_target(npc);
            let to_spot = (spot.0 - cx, spot.1 - cy);
            let dist = to_spot.0.hypot(to_spot.1);
            let speed =
                (MOTHRON_LAY_SPEED_BASE + dist / MOTHRON_LAY_SPEED_GAIN).min(MOTHRON_LAY_SPEED_CAP);
            if dist < MOTHRON_LAY_ARRIVE {
                npc.ai[0] = state::SETTLING;
            }
            let wanted = unit(to_spot, speed);
            npc.velocity.0 = (npc.velocity.0 * 9.0 + wanted.0) / 10.0;
            npc.velocity.1 = (npc.velocity.1 * 9.0 + wanted.1) / 10.0;
            clamp_speed(&mut npc.velocity, speed);
        }

        s if s == state::SETTLING => {
            // Hovering right over the spot while the egg actually appears, then a second, equal
            // wait before it goes back to hovering — or, with room in the brood, straight down to
            // lay another rather than always just the one.
            npc.no_tile_collide = true;
            npc.rotation = (npc.rotation * 9.0 + npc.velocity.0 * 0.1) / 10.0;

            let spot = lay_target(npc);
            let mut to_spot = (spot.0 - cx, spot.1 - cy);
            let dist = to_spot.0.hypot(to_spot.1);
            if dist < MOTHRON_SETTLE_ARRIVE {
                let wait = if world.conditions.expert {
                    MOTHRON_SETTLE_WAIT_EXPERT
                } else {
                    MOTHRON_SETTLE_WAIT
                };
                npc.ai[3] += 1.0;
                if npc.ai[3] == wait {
                    out.spawn.push(Spawn {
                        npc_type: MOTHRON_EGG,
                        position: (npc.ai[1], npc.ai[2]),
                        velocity: (0.0, 0.0),
                        parent: None,
                        ai: [None; 4],
                    });
                } else if npc.ai[3] >= wait * 2.0 {
                    let relay =
                        brood < MOTHRON_BROOD && rng.random_range(0..MOTHRON_RELAY_ODDS) != 0;
                    npc.ai = [
                        if relay {
                            state::LAYING
                        } else {
                            state::HOVERING
                        },
                        0.0,
                        0.0,
                        0.0,
                    ];
                }
            }
            if dist > MOTHRON_SETTLE_SPEED_CAP {
                let k = MOTHRON_SETTLE_SPEED_CAP / dist.max(f32::MIN_POSITIVE);
                to_spot = (to_spot.0 * k, to_spot.1 * k);
            }
            npc.velocity.0 = (npc.velocity.0 + to_spot.0) / 2.0;
            npc.velocity.1 = (npc.velocity.1 + to_spot.1) / 2.0;
            clamp_speed(&mut npc.velocity, MOTHRON_SETTLE_SPEED_CAP);
        }

        _ => {}
    }
    out
}

/// Somewhere near the player with a floor under it and open air above, out of lava.
fn egg_spot(
    world: &World<'_, impl TileView>,
    player: (f32, f32),
    rng: &mut SmallRng,
) -> Option<(f32, f32)> {
    let (px, py) = ((player.0 / TILE) as i32, (player.1 / TILE) as i32);
    for attempt in 0..1000 {
        // The search widens the longer it goes without finding anywhere.
        let spread_x = MOTHRON_LAY_RANGE_X + attempt / 50;
        let spread_y = MOTHRON_LAY_RANGE_Y + attempt / 75;
        let x = px + rng.random_range(-spread_x..=spread_x);
        let mut y = py + rng.random_range(-spread_y..=spread_y);
        if solid(world, x, y) {
            continue;
        }
        // Fall until something holds it up.
        let mut landed = false;
        for _ in 0..50 {
            let tile = world.tiles.tile(x, y);
            if tile.liquid > 0 && tile.liquid_kind == terrustia_proto::tile::Liquid::Lava {
                break;
            }
            if solid(world, x, y + 1) {
                landed = true;
                break;
            }
            y += 1;
        }
        if landed {
            return Some((x as f32 * TILE, y as f32 * TILE));
        }
    }
    None
}

fn solid(world: &World<'_, impl TileView>, x: i32, y: i32) -> bool {
    let tile = world.tiles.tile(x, y);
    tile.is_active() && terrustia_proto::tile_solid::solid(tile.block)
}

/// Where it flies to while descending or settling: a little above the spot it picked
/// (`ai[1]`/`ai[2]`, the tile's own position, which is where the egg itself ends up).
fn lay_target(npc: &Npc) -> (f32, f32) {
    (npc.ai[1] + TILE / 2.0, npc.ai[2] - 20.0)
}

/// Clamp a velocity's length to a cap, keeping its direction.
fn clamp_speed(velocity: &mut (f32, f32), cap: f32) {
    let length = velocity.0.hypot(velocity.1);
    if length > cap {
        velocity.0 = velocity.0 / length * cap;
        velocity.1 = velocity.1 / length * cap;
    }
}

/// Rebound off terrain, keeping enough speed to clear it.
fn bounce(npc: &mut Npc) {
    if npc.collide_x {
        npc.velocity.0 = (-npc.old_velocity.0 * 0.5).clamp(-4.0, 4.0);
    }
    if npc.collide_y {
        npc.velocity.1 = (-npc.old_velocity.1 * 0.5).clamp(-4.0, 4.0);
    }
}

fn ease(velocity: &mut (f32, f32), wanted: (f32, f32), smooth: f32) {
    velocity.0 = (velocity.0 * (smooth - 1.0) + wanted.0) / smooth;
    velocity.1 = (velocity.1 * (smooth - 1.0) + wanted.1) / smooth;
}

fn unit(v: (f32, f32), speed: f32) -> (f32, f32) {
    let length = v.0.hypot(v.1);
    if length <= 0.0 || !length.is_finite() {
        (0.0, 0.0)
    } else {
        (v.0 / length * speed, v.1 / length * speed)
    }
}

fn boxed(tiles: &impl TileView, npc: &Npc) -> bool {
    let x0 = (npc.position.0 / TILE).floor() as i32;
    let x1 = ((npc.position.0 + npc.width() - 1.0) / TILE).floor() as i32;
    let y0 = (npc.position.1 / TILE).floor() as i32;
    let y1 = ((npc.position.1 + npc.height() - 1.0) / TILE).floor() as i32;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ai::Conditions;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Sky(HashMap<(i32, i32), Tile>);

    impl TileView for Sky {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn ground(at: i32) -> Sky {
        let mut tiles = HashMap::new();
        for x in -600..600 {
            for y in at..at + 4 {
                tiles.insert((x, y), Tile::block(1));
            }
        }
        Sky(tiles)
    }

    fn eclipse(tiles: &Sky, target: Option<(f32, f32)>) -> World<'_, Sky> {
        let mut w = crate::game::ai::calm(
            tiles,
            target.map(|center| Target {
                slot: 0,
                center,
                velocity: (0.0, 0.0),
                alive: true,
            }),
        );
        w.conditions = Conditions {
            eclipse: true,
            day: true,
            ..Conditions::default()
        };
        w
    }

    const MOTHRON: u16 = 477;

    fn mothron_at(x: f32, y: f32) -> Npc {
        Npc::new(MOTHRON, (x, y), 1).expect("mothron")
    }

    /// No eclipse, no Mothron: it climbs away rather than fighting.
    #[test]
    fn it_leaves_when_the_eclipse_ends() {
        let tiles = ground(300);
        let mut rng = SmallRng::seed_from_u64(88);
        let mut m = mothron_at(0.0, 0.0);
        let mut w = eclipse(&tiles, Some((300.0, 0.0)));
        w.conditions.eclipse = false;

        for _ in 0..60 {
            mothron(&mut m, &w, 0, &mut rng);
        }
        assert_eq!(m.ai[0], state::LEAVING);
        assert!(m.velocity.1 < 0.0, "and it goes up");
        assert!(m.invulnerable, "nothing can stop it leaving");
    }

    /// All three attacks come round, and hovering is what it returns to.
    #[test]
    fn it_cycles_through_its_attacks() {
        let tiles = ground(300);
        let mut seen = std::collections::HashSet::new();
        for seed in 0..40 {
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut m = mothron_at(0.0, 0.0);
            m.ai[0] = state::HOVERING;
            m.ai[1] = MOTHRON_DECIDE_TICKS;
            let w = eclipse(&tiles, Some((200.0, 200.0)));
            for _ in 0..30 {
                mothron(&mut m, &w, 0, &mut rng);
                if m.ai[0] != state::HOVERING {
                    seen.insert(format!("{}", m.ai[0]));
                    break;
                }
            }
        }
        assert!(
            seen.len() >= 3,
            "chase, sweep and lay should all appear: {seen:?}"
        );
    }

    /// Hitting it brings the next attack forward.
    #[test]
    fn hitting_mothron_hurries_its_next_attack() {
        let tiles = ground(300);
        let ticks_to_attack = |harried: bool| {
            let mut rng = SmallRng::seed_from_u64(3);
            let mut m = mothron_at(0.0, 0.0);
            m.ai[0] = state::HOVERING;
            let mut w = eclipse(&tiles, Some((200.0, 200.0)));
            w.was_hurt = harried;
            for tick in 0..1200 {
                mothron(&mut m, &w, 0, &mut rng);
                if m.ai[0] != state::HOVERING {
                    return tick;
                }
            }
            i32::MAX
        };
        assert!(
            ticks_to_attack(true) < ticks_to_attack(false),
            "a harried Mothron should attack sooner"
        );
    }

    /// The chase hits at half strength; the sweep at a third more. That difference is the fight.
    #[test]
    fn the_chase_is_gentler_than_the_sweep() {
        let tiles = ground(300);
        let mut rng = SmallRng::seed_from_u64(5);
        let w = eclipse(&tiles, Some((200.0, 0.0)));

        let mut chasing = mothron_at(0.0, 0.0);
        chasing.ai[0] = state::CHASING;
        mothron(&mut chasing, &w, 0, &mut rng);
        assert_eq!(chasing.damage_bonus, MOTHRON_CHASE_DAMAGE);

        let mut sweeping = mothron_at(0.0, 0.0);
        sweeping.ai[0] = state::SWEEPING;
        sweeping.ai[1] = 1.0;
        mothron(&mut sweeping, &w, 0, &mut rng);
        assert_eq!(sweeping.damage_bonus, MOTHRON_SWEEP_DAMAGE);
        assert!(sweeping.damage_bonus > chasing.damage_bonus);
    }

    /// It lays an egg on the ground, and stops once its brood is full.
    #[test]
    fn it_lays_an_egg_on_the_floor() {
        let tiles = ground(300);
        let mut rng = SmallRng::seed_from_u64(7);
        let mut m = mothron_at(0.0, 280.0 * TILE);
        m.ai[0] = state::LAYING;
        let w = eclipse(&tiles, Some((100.0, 295.0 * TILE)));

        // Laying is not instant any more: it takes a real, multi-tick punish window — a spot to
        // pick, a flight down to it, and a hover over it before the egg actually appears — and
        // never on the very first tick.
        let first = mothron(&mut m, &w, 0, &mut rng);
        assert!(
            first.spawn.is_empty(),
            "the very first tick should not already have laid an egg"
        );
        assert_ne!(
            m.ai[0],
            state::HOVERING,
            "it should be underway, not done already"
        );

        let mut egg = None;
        let mut ticks = 0;
        for at in 0..2000 {
            ticks = at;
            let out = mothron(&mut m, &w, 0, &mut rng);
            // Nothing else moves it during the test, so the flight down has to be driven here.
            m.position.0 += m.velocity.0;
            m.position.1 += m.velocity.1;
            if let Some(spawned) = out.spawn.into_iter().next() {
                egg = Some(spawned);
                break;
            }
        }
        let egg = egg.expect("it should eventually have laid one");
        assert!(
            ticks > 10,
            "and it should have taken a real punish window: {ticks} ticks"
        );
        assert_eq!(egg.npc_type, MOTHRON_EGG);
        // On the floor rather than in the air.
        let tile_y = (egg.position.1 / TILE) as i32;
        assert!(
            (299..=302).contains(&tile_y),
            "the egg should be on the ground, at tile {tile_y}"
        );
        // It stays put, hovering over the egg, rather than leaving the instant it appears.
        assert_eq!(
            m.ai[0],
            state::SETTLING,
            "it should still be hovering there"
        );

        // A full brood means no more.
        let mut full = mothron_at(0.0, 280.0 * TILE);
        full.ai[0] = state::HOVERING;
        full.ai[1] = MOTHRON_DECIDE_TICKS;
        for _ in 0..200 {
            let out = mothron(
                &mut full,
                &w,
                terrustia_proto::npc_params::MOTHRON_BROOD,
                &mut rng,
            );
            assert!(out.spawn.is_empty(), "a full brood should lay nothing");
        }
    }

    /// The sweep draws off to one side first rather than charging from where it is.
    #[test]
    fn the_sweep_backs_off_before_it_comes_across() {
        let tiles = ground(300);
        let mut rng = SmallRng::seed_from_u64(9);
        let mut m = mothron_at(0.0, 0.0);
        m.ai[0] = state::DRAWING_OFF;
        let w = eclipse(&tiles, Some((100.0, 0.0)));

        let start = (m.center().0 - 100.0).abs();
        let mut reached = false;
        for _ in 0..600 {
            mothron(&mut m, &w, 0, &mut rng);
            m.position.0 += m.velocity.0;
            m.position.1 += m.velocity.1;
            if m.ai[0] == state::LINING_UP {
                reached = true;
                break;
            }
        }
        assert!(reached, "it should have got into position");
        assert!(
            (m.center().0 - 100.0).abs() > start,
            "and moved away from the player to do it"
        );
    }
}
