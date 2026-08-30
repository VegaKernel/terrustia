//! The moon events' flying bosses: styles 58–61.
//!
//! * **Pumpking** (58) picks a mood every five seconds and commits to it: throwing spheres,
//!   charging, or setting its blades scything. It hovers two hundred pixels above you between
//!   moods, and in its charging mood it closes *faster the further off you are*, so distance is no
//!   defence against that one.
//! * A **blade** (59) is one of two that orbit Pumpking. It dies with it.
//! * The **Ice Queen** (60) does not hover at all. It sweeps back and forth across you, turning at
//!   eight hundred pixels, and accelerates at every quarter of its health.
//! * **Santa-NK1** (61) walks, waits five seconds, and then fires — and the gap between shots
//!   shortens at every quarter, from every sixteen ticks down to every eight.
//!
//! Daylight ends all of them, and each leaves differently: Pumpking sinks, the Ice Queen
//! accelerates away upward, Santa walks off.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    ICE_QUEEN_MIST, ICE_QUEEN_MIST_DAMAGE, ICE_QUEEN_MIST_INTERVAL, ICE_QUEEN_MIST_RANGE,
    ICE_QUEEN_MIST_SPEED, ICE_QUEEN_MODE0_AT, ICE_QUEEN_MODE1_AT, ICE_QUEEN_MODE2_PACE,
    ICE_QUEEN_SHARD, ICE_QUEEN_SHARD_DAMAGE, ICE_QUEEN_SHARD_INTERVAL, PUMPKING_ABOVE,
    PUMPKING_BLADE, PUMPKING_BLADES, PUMPKING_CHARGE, PUMPKING_CHARGE_SMOOTH,
    PUMPKING_CHARGE_TICKS, PUMPKING_HOVER, PUMPKING_HOVER_SMOOTH, PUMPKING_LEASH,
    PUMPKING_MOOD_TICKS, PUMPKING_MOODS, PUMPKING_RUSH_STEPS, PUMPKING_SPHERE,
    PUMPKING_SPHERE_DAMAGE, PUMPKING_SPHERE_EVERY, PUMPKING_SPHERE_SPAN, PUMPKING_SPHERE_SPEED,
    QUEEN_ABOVE_MAX, QUEEN_ABOVE_MIN, QUEEN_CLIMB, QUEEN_CLIMB_CAP, QUEEN_PACE, QUEEN_SWEEP,
    SANTA_BULLET, SANTA_BULLET_DAMAGE, SANTA_BULLET_SPEED, SANTA_FIRE_RATE, SANTA_LEASH,
    SANTA_MUZZLE, SANTA_WAIT, SANTA_WALK,
};

use super::skeletron::Parent;
use crate::game::ai::{Shot, World, face};
use crate::game::npc::{Npc, TILE, TileView};
use crate::game::npc_ai::Spawn;

/// What one of them did this tick.
#[derive(Debug, Default)]
pub struct MoonOutcome {
    pub shots: Vec<Shot>,
    pub spawn: Vec<Spawn>,
    pub spent: bool,
}

/// Pick the value for the first threshold this health is below.
fn by_health<const N: usize, T: Copy>(health: f32, table: [(f32, T); N]) -> T {
    let mut chosen = table[0].1;
    for (threshold, value) in table {
        if health < threshold {
            chosen = value;
        }
    }
    chosen
}

/// Style 58: Pumpking.
pub fn pumpking(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    rng: &mut SmallRng,
) -> MoonOutcome {
    let mut out = MoonOutcome::default();
    npc.dirty = true;
    npc.no_gravity = true;
    npc.no_tile_collide = true;

    // Its two blades go out on the first tick, on opposite sides and half a cycle out of phase so
    // they scythe opposite arcs rather than sitting on top of one another (`NPC.cs:33377-33387`):
    // the first is raised with ai[0] = -1, the second with ai[0] = 1 and its phase ai[3] started at
    // 150 of the 300-tick sweep. Left unset both would read ai[0] = signum(0) = 1 and ai[3] = 0.
    if npc.ai[0] == 0.0 {
        npc.ai[0] = 1.0;
        for side in 0..PUMPKING_BLADES {
            // The first goes left at phase 0, the second right and already 150 of its 300 ticks
            // along, so the two are half a cycle apart. Vanilla leaves the first's ai[3] at its
            // default and only sets the second's, which this mirrors.
            let (arc, phase) = if side == 0 {
                (-1.0, None)
            } else {
                (1.0, Some(150.0))
            };
            out.spawn.push(Spawn {
                npc_type: PUMPKING_BLADE,
                position: npc.center(),
                velocity: (0.0, 0.0),
                parent: Some(Spawn::OWN_PARENT),
                ai: [Some(arc), None, None, phase],
            });
        }
    }

    // The mood, which changes every five seconds whatever it is doing.
    npc.local_ai[2] += 1.0;
    if npc.local_ai[2] > PUMPKING_MOOD_TICKS {
        npc.local_ai[2] = 0.0;
        npc.ai[3] = rng.random_range(0..PUMPKING_MOODS) as f32;
    } else if npc.ai[3] == 0.0
        && npc.local_ai[2] > PUMPKING_SPHERE_EVERY
        && npc.local_ai[2] % PUMPKING_SPHERE_EVERY == 0.0
        && let Some(target) = world.target.filter(|t| t.alive)
    {
        // In its throwing mood it lobs spheres from beneath itself.
        let (cx, cy) = npc.center();
        let from = (cx, cy + 30.0);
        let mut across = target.center.0 - from.0 + rng.random_range(-50..=50) as f32;
        // Aimed low and flat: a fifth of the vertical gap, plus a downward bias.
        let mut rise = (target.center.1 - from.1 + rng.random_range(50..201) as f32) * 0.2;
        let length = across.hypot(rise).max(f32::MIN_POSITIVE);
        across = across / length * PUMPKING_SPHERE_SPEED;
        rise = rise / length * PUMPKING_SPHERE_SPEED;
        let jitter = |rng: &mut SmallRng| 1.0 + rng.random_range(-30..=30) as f32 * 0.01;
        out.shots.push(Shot {
            projectile: PUMPKING_SPHERE + rng.random_range(0..PUMPKING_SPHERE_SPAN),
            damage: PUMPKING_SPHERE_DAMAGE,
            position: from,
            velocity: (across * jitter(rng), rise * jitter(rng)),
            time_left: 600,
        });
    }

    // Lost, or daylight: it sinks away.
    let target = world.target.filter(|t| {
        t.alive
            && (npc.position.0 - t.center.0).abs() <= PUMPKING_LEASH
            && (npc.position.1 - t.center.1).abs() <= PUMPKING_LEASH
    });
    if target.is_none() || world.conditions.day {
        npc.velocity.1 += 0.3;
        npc.velocity.0 *= 0.9;
        npc.rotation = npc.velocity.0 * -0.02;
        npc.time_left = npc.time_left.min(600);
        return out;
    }
    let target = target.expect("checked just above");
    let (cx, cy) = npc.center();

    if npc.ai[1] == 0.0 {
        // Hovering above the player, and only its charging mood breaks that.
        npc.ai[2] += 1.0;
        if npc.ai[2] >= PUMPKING_MOOD_TICKS {
            npc.ai[2] = 0.0;
            if npc.ai[3] == 1.0 {
                npc.ai[1] = 1.0;
            }
        }
        let gap = (target.center.0 - cx, target.center.1 - PUMPKING_ABOVE - cy);
        let reach = gap.0.hypot(gap.1);
        // In its charging mood it closes faster the further off you are.
        let mut speed = PUMPKING_HOVER;
        if npc.ai[3] == 1.0 {
            for (at, faster) in PUMPKING_RUSH_STEPS {
                if reach > at {
                    speed = faster;
                    break;
                }
            }
        }
        if reach > 50.0 {
            let scale = speed / reach;
            npc.velocity.0 = (npc.velocity.0 * PUMPKING_HOVER_SMOOTH + gap.0 * scale)
                / (PUMPKING_HOVER_SMOOTH + 1.0);
            npc.velocity.1 = (npc.velocity.1 * PUMPKING_HOVER_SMOOTH + gap.1 * scale)
                / (PUMPKING_HOVER_SMOOTH + 1.0);
        }
    } else {
        // Charging: straight at you, very heavily smoothed, so it arcs rather than turns.
        npc.ai[2] += 1.0;
        if npc.ai[2] >= PUMPKING_CHARGE_TICKS || npc.ai[3] != 1.0 {
            npc.ai[1] = 0.0;
            npc.ai[2] = 0.0;
        }
        let gap = (target.center.0 - cx, target.center.1 - cy);
        let reach = gap.0.hypot(gap.1).max(f32::MIN_POSITIVE);
        let scale = PUMPKING_CHARGE / reach;
        npc.velocity.0 = (npc.velocity.0 * PUMPKING_CHARGE_SMOOTH + gap.0 * scale)
            / (PUMPKING_CHARGE_SMOOTH + 1.0);
        npc.velocity.1 = (npc.velocity.1 * PUMPKING_CHARGE_SMOOTH + gap.1 * scale)
            / (PUMPKING_CHARGE_SMOOTH + 1.0);
    }
    npc.rotation = npc.velocity.0 * -0.02;
    out
}

/// Style 59: one of Pumpking's blades.
///
/// It orbits its owner and dies with it. `ai[0]` is which side it is on and `ai[3]` its phase, so
/// the two sweep opposite arcs rather than sitting on top of one another.
pub fn pumpking_blade(npc: &mut Npc, owner: Option<Parent>) -> MoonOutcome {
    let mut out = MoonOutcome::default();
    npc.dirty = true;

    let Some(owner) = owner else {
        // No Pumpking, no blade.
        npc.velocity.0 *= 0.9;
        npc.velocity.1 *= 0.9;
        out.spent = true;
        return out;
    };
    npc.sprite_direction = -(npc.ai[0] as i8);

    // The arc: it swings out and back on a three-hundred-tick cycle.
    npc.ai[3] += 1.0;
    if npc.ai[3] >= 300.0 {
        npc.ai[3] = 0.0;
    }
    let along = npc.ai[3] / 300.0 * std::f32::consts::TAU;
    let radius = 150.0 + along.sin() * 100.0;
    let angle = along * npc.ai[0].signum();
    let (ox, oy) = owner.center();
    let station = (ox + angle.cos() * radius, oy + angle.sin() * radius);

    let (cx, cy) = npc.center();
    npc.velocity = ((station.0 - cx) * 0.2, (station.1 - cy) * 0.2);
    npc.rotation = npc.velocity.1.atan2(npc.velocity.0);
    out
}

/// Style 60: the Ice Queen.
///
/// Mode 0 sweeps back and forth, firing a mist forward while above you. Mode 1 gives up the
/// sweep for a gentler pursuit and drops ice shards straight down instead. Each runs for a while
/// before handing off to the other. Vanilla also has a third mode — a random scatter shot reached
/// the same way — that is not implemented here; see the module notes on why this routine cannot
/// draw randomness of its own.
pub fn ice_queen(npc: &mut Npc, world: &World<'_, impl TileView>) -> MoonOutcome {
    let mut out = MoonOutcome::default();
    npc.dirty = true;
    npc.no_gravity = true;
    npc.no_tile_collide = true;

    if world.conditions.day {
        // It accelerates away rather than slowing.
        npc.velocity.0 += 0.25 * if npc.velocity.0 > 0.0 { 1.0 } else { -1.0 };
        npc.velocity.1 -= 0.1;
        npc.rotation = npc.velocity.0 * 0.05;
        npc.time_left = npc.time_left.min(600);
        return out;
    }
    let Some(target) = world.target.filter(|t| t.alive) else {
        return out;
    };
    let (cx, cy) = npc.center();
    let health = npc.life as f32 / npc.life_max.max(1) as f32;

    if npc.ai[0] == 1.0 {
        // Mode 1: gentler pursuit, and ice shards dropped straight down.
        let (accel, cap) = {
            let mut chosen = (ICE_QUEEN_MODE2_PACE[0].1, ICE_QUEEN_MODE2_PACE[0].2);
            for (threshold, a, c) in ICE_QUEEN_MODE2_PACE {
                if health < threshold {
                    chosen = (a, c);
                }
            }
            chosen
        };
        if cx < target.center.0 {
            npc.velocity.0 += accel;
            if npc.velocity.0 < 0.0 {
                npc.velocity.0 *= 0.98;
            }
        }
        if cx > target.center.0 {
            npc.velocity.0 -= accel;
            if npc.velocity.0 > 0.0 {
                npc.velocity.0 *= 0.98;
            }
        }
        if npc.velocity.0 > cap || npc.velocity.0 < -cap {
            npc.velocity.0 *= 0.95;
        }
        let below = target.center.1 - (npc.position.1 + npc.height());
        if below < 180.0 {
            npc.velocity.1 -= 0.1;
        }
        if below > 200.0 {
            npc.velocity.1 += 0.1;
        }
        npc.velocity.1 = npc.velocity.1.clamp(-6.0, 6.0);
        npc.rotation = npc.velocity.0 * 0.01;

        npc.ai[3] += 1.0;
        let interval = by_health(health, ICE_QUEEN_SHARD_INTERVAL);
        if npc.ai[3] >= interval {
            npc.ai[3] = 0.0;
            let drop = (cx, npc.position.1 + npc.height() - 14.0);
            let tx = (drop.0 / TILE) as i32;
            let ty = (drop.1 / TILE) as i32;
            let tile = world.tiles.tile(tx, ty);
            let blocked = tile.is_active() && terrustia_proto::tile_solid::solid(tile.block);
            if !blocked {
                out.shots.push(Shot {
                    projectile: ICE_QUEEN_SHARD,
                    damage: ICE_QUEEN_SHARD_DAMAGE,
                    position: drop,
                    velocity: (npc.velocity.0 * 0.25, npc.velocity.1.max(0.0) + 3.0),
                    time_left: 600,
                });
            }
        }

        npc.ai[1] += 1.0;
        if npc.ai[1] > ICE_QUEEN_MODE1_AT {
            npc.ai = [0.0, 0.0, 0.0, 0.0];
        }
        return out;
    }

    // Mode 0: the sweep. `ai[2]` is which way it is going, and it only turns once it is well
    // past you.
    if npc.ai[2] == 0.0 {
        npc.ai[2] = if cx < target.center.0 { 1.0 } else { -1.0 };
    }
    let across = (cx - target.center.0).abs();
    if across > QUEEN_SWEEP
        && ((cx < target.center.0 && npc.ai[2] < 0.0) || (cx > target.center.0 && npc.ai[2] > 0.0))
    {
        npc.ai[2] = 0.0;
    }

    let (accel, cap) = {
        let mut chosen = (QUEEN_PACE[0].1, QUEEN_PACE[0].2);
        for (threshold, a, c) in QUEEN_PACE {
            if health < threshold {
                chosen = (a, c);
            }
        }
        chosen
    };
    npc.velocity.0 = (npc.velocity.0 + npc.ai[2] * accel).clamp(-cap, cap);

    // It holds a band above you rather than a height.
    let below = target.center.1 - (npc.position.1 + npc.height());
    if below < QUEEN_ABOVE_MIN {
        npc.velocity.1 -= QUEEN_CLIMB;
    }
    if below > QUEEN_ABOVE_MAX {
        npc.velocity.1 += QUEEN_CLIMB;
    }
    npc.velocity.1 = npc.velocity.1.clamp(-QUEEN_CLIMB_CAP, QUEEN_CLIMB_CAP);
    npc.rotation = npc.velocity.0 * 0.05;

    // The forward mist: only while above you, and either close or already mid-volley.
    let above_player = npc.position.1 < target.center.1;
    if (across < ICE_QUEEN_MIST_RANGE || npc.ai[3] < 0.0) && above_player {
        npc.ai[3] += 1.0;
        let interval = by_health(health, ICE_QUEEN_MIST_INTERVAL);
        if npc.ai[3] > interval {
            npc.ai[3] = -interval;
        }
        if npc.ai[3] == 0.0 {
            let speed = by_health(health, ICE_QUEEN_MIST_SPEED);
            let from = (cx + npc.velocity.0 * 7.0, cy);
            let aim = (target.center.0 - from.0, target.center.1 - from.1);
            let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
            out.shots.push(Shot {
                projectile: ICE_QUEEN_MIST,
                damage: ICE_QUEEN_MIST_DAMAGE,
                position: from,
                velocity: (aim.0 / length * speed, aim.1 / length * speed),
                time_left: 600,
            });
        }
    } else if npc.ai[3] < 0.0 {
        npc.ai[3] += 1.0;
    }

    npc.ai[1] += 1.0;
    if npc.ai[1] > ICE_QUEEN_MODE0_AT {
        npc.ai = [1.0, 0.0, 0.0, 0.0];
    }
    out
}

/// Style 61: Santa-NK1.
pub fn santa(npc: &mut Npc, world: &World<'_, impl TileView>, rng: &mut SmallRng) -> MoonOutcome {
    let mut out = MoonOutcome::default();
    npc.dirty = true;
    npc.no_gravity = true;
    npc.no_tile_collide = true;

    let health = npc.life as f32 / npc.life_max.max(1) as f32;
    let mut walk = by_health(health, SANTA_WALK);
    let mut planted = false;

    let in_reach = world
        .target
        .filter(|t| t.alive && (npc.position.0 - t.center.0).abs() <= SANTA_LEASH);
    if world.conditions.day {
        walk = 8.0;
        npc.time_left = npc.time_left.min(600);
        if npc.velocity.0 == 0.0 {
            npc.velocity.0 = 0.1;
        }
    } else if let Some(target) = in_reach {
        face(npc, target);
        if npc.ai[0] == 0.0 {
            npc.ai[1] += 1.0;
            if npc.ai[1] >= SANTA_WAIT {
                npc.ai[1] = 0.0;
                npc.ai[0] = 1.0;
            }
        } else {
            // Firing. It stands still and the gun speeds up as it is worn down.
            planted = true;
            npc.ai[1] += 1.0;
            let every = by_health(health, SANTA_FIRE_RATE);
            if npc.ai[1] % every == 0.0 {
                let (cx, cy) = npc.center();
                let muzzle = (
                    cx + f32::from(npc.direction) * SANTA_MUZZLE,
                    cy + rng.random_range(15..36) as f32,
                );
                let mut across = target.center.0 - muzzle.0 + rng.random_range(-40..=40) as f32;
                let mut rise = target.center.1 - muzzle.1 + rng.random_range(-40..=40) as f32;
                let length = across.hypot(rise).max(f32::MIN_POSITIVE);
                across = across / length * SANTA_BULLET_SPEED;
                rise = rise / length * SANTA_BULLET_SPEED;
                let jitter = |rng: &mut SmallRng| 1.0 + rng.random_range(-20..=20) as f32 * 0.015;
                out.shots.push(Shot {
                    projectile: SANTA_BULLET,
                    damage: SANTA_BULLET_DAMAGE,
                    position: muzzle,
                    velocity: (across * jitter(rng), rise * jitter(rng)),
                    time_left: 300,
                });
            }
            if npc.ai[1] >= 300.0 {
                npc.ai[1] = 0.0;
                npc.ai[0] = 0.0;
            }
        }
    }

    if planted {
        npc.velocity.0 *= 0.9;
        if npc.velocity.0.abs() < 0.1 {
            npc.velocity.0 = 0.0;
        }
    } else {
        let wanted = walk * f32::from(npc.direction);
        npc.velocity.0 = (npc.velocity.0 * 20.0 + wanted) / 21.0;
    }
    let _ = TILE;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ai::Conditions;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::npc_params::PUMPKING;
    use terrustia_proto::tile::Tile;

    struct Sky(HashMap<(i32, i32), Tile>);

    impl TileView for Sky {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn night(tiles: &Sky, target: Option<(f32, f32)>) -> World<'_, Sky> {
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
            day: false,
            ..Conditions::default()
        };
        w
    }

    fn boss(npc_type: u16, x: f32, y: f32) -> Npc {
        Npc::new(npc_type, (x, y), 1).expect("a moon boss")
    }

    /// Pumpking sends out its two blades before anything else.
    #[test]
    fn pumpking_arrives_with_its_blades() {
        let tiles = Sky(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(58);
        let mut p = boss(PUMPKING, 0.0, 0.0);
        let w = night(&tiles, Some((600.0, 400.0)));
        let out = pumpking(&mut p, &w, &mut rng);
        assert_eq!(out.spawn.len(), PUMPKING_BLADES);
        assert!(out.spawn.iter().all(|s| s.npc_type == PUMPKING_BLADE));
        assert!(pumpking(&mut p, &w, &mut rng).spawn.is_empty(), "only once");
    }

    /// PUMP-1: the two blades are raised on opposite sides and half a cycle out of phase, so they
    /// scythe opposite arcs rather than orbiting as one (`NPC.cs:33377-33387`): the first with
    /// ai[0] = -1, the second ai[0] = 1 and its phase ai[3] = 150. Left to the consumer's signum
    /// both would read ai[0] = signum(0) = 1 and ai[3] = 0, and the pair would overlap exactly.
    #[test]
    fn its_blades_seat_on_opposite_sides_out_of_phase() {
        let tiles = Sky(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(58);
        let mut p = boss(PUMPKING, 0.0, 0.0);
        let w = night(&tiles, Some((600.0, 400.0)));
        let out = pumpking(&mut p, &w, &mut rng);
        let blades: Vec<&Spawn> = out
            .spawn
            .iter()
            .filter(|s| s.npc_type == PUMPKING_BLADE)
            .collect();
        assert_eq!(blades.len(), 2);
        assert_eq!(blades[0].ai[0], Some(-1.0), "the first blade goes left");
        assert_eq!(blades[0].ai[3], None, "and starts at phase 0 (its default)");
        assert_eq!(blades[1].ai[0], Some(1.0), "the second goes right");
        assert_eq!(blades[1].ai[3], Some(150.0), "half a 300-tick cycle ahead");
    }

    /// Its moods come round, and the throwing one actually throws.
    #[test]
    fn pumpking_changes_its_mood_and_throws() {
        let tiles = Sky(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(2);
        let mut p = boss(PUMPKING, 0.0, 0.0);
        let w = night(&tiles, Some((600.0, 400.0)));

        let mut moods = std::collections::HashSet::new();
        let mut thrown = 0;
        for _ in 0..4000 {
            let out = pumpking(&mut p, &w, &mut rng);
            thrown += out.shots.len();
            moods.insert(p.ai[3] as i32);
        }
        assert!(moods.len() >= 2, "its mood should change: {moods:?}");
        assert!(thrown > 0, "and it should throw something");
    }

    /// A blade without its Pumpking does not survive.
    #[test]
    fn a_blade_dies_with_its_owner() {
        let mut b = boss(PUMPKING_BLADE, 0.0, 0.0);
        assert!(pumpking_blade(&mut b, None).spent);
    }

    /// The Ice Queen sweeps past you and turns, rather than hovering.
    #[test]
    fn the_ice_queen_sweeps_back_and_forth() {
        let tiles = Sky(HashMap::new());
        let mut q = boss(terrustia_proto::npc_params::ICE_QUEEN, 0.0, 0.0);
        let w = night(&tiles, Some((0.0, 400.0)));

        let mut crossings = 0;
        let mut side = -1.0f32;
        for _ in 0..4000 {
            ice_queen(&mut q, &w);
            q.position.0 += q.velocity.0;
            q.position.1 += q.velocity.1;
            let now = q.center().0.signum();
            if now != side && now != 0.0 {
                crossings += 1;
                side = now;
            }
        }
        assert!(
            crossings >= 2,
            "it should keep sweeping across: {crossings}"
        );
    }

    /// B12: the Ice Queen fires a mist forward while above you, in its first mode.
    #[test]
    fn the_ice_queen_fires_a_mist_while_above_you() {
        let tiles = Sky(HashMap::new());
        let mut q = boss(terrustia_proto::npc_params::ICE_QUEEN, 0.0, 0.0);
        let w = night(&tiles, Some((0.0, 400.0)));

        let mut shots = Vec::new();
        for _ in 0..800 {
            let out = ice_queen(&mut q, &w);
            shots.extend(out.shots);
            q.position.0 += q.velocity.0;
            q.position.1 += q.velocity.1;
        }
        assert!(!shots.is_empty(), "it should have fired the mist");
        assert!(
            shots
                .iter()
                .all(|s| s.projectile == ICE_QUEEN_MIST && s.damage == ICE_QUEEN_MIST_DAMAGE)
        );
    }

    /// B12: after a while it hands off to its second mode.
    #[test]
    fn the_ice_queen_switches_from_mode_zero_to_mode_one() {
        let tiles = Sky(HashMap::new());
        let mut q = boss(terrustia_proto::npc_params::ICE_QUEEN, 0.0, 0.0);
        let w = night(&tiles, Some((0.0, 400.0)));
        for _ in 0..(ICE_QUEEN_MODE0_AT as i32 + 2) {
            ice_queen(&mut q, &w);
            q.position.0 += q.velocity.0;
            q.position.1 += q.velocity.1;
        }
        assert_eq!(q.ai[0], 1.0, "it should have switched to its second mode");
    }

    /// B12: the second mode drops falling ice shards instead of firing the forward mist.
    #[test]
    fn the_ice_queen_drops_shards_in_its_second_mode() {
        let tiles = Sky(HashMap::new());
        let mut q = boss(terrustia_proto::npc_params::ICE_QUEEN, 0.0, 0.0);
        q.ai[0] = 1.0;
        let w = night(&tiles, Some((0.0, 400.0)));

        let mut shots = Vec::new();
        for _ in 0..400 {
            let out = ice_queen(&mut q, &w);
            shots.extend(out.shots);
            q.position.0 += q.velocity.0;
            q.position.1 += q.velocity.1;
        }
        assert!(!shots.is_empty(), "it should drop shards");
        assert!(
            shots
                .iter()
                .all(|s| s.projectile == ICE_QUEEN_SHARD && s.damage == ICE_QUEEN_SHARD_DAMAGE)
        );
        assert!(
            shots.iter().all(|s| s.velocity.1 > 0.0),
            "falling, not rising: {:?}",
            shots.iter().map(|s| s.velocity).collect::<Vec<_>>()
        );
    }

    /// Santa's gun speeds up as it is worn down.
    #[test]
    fn santa_fires_faster_as_it_dies() {
        let tiles = Sky(HashMap::new());
        let w = night(&tiles, Some((600.0, 0.0)));
        let shots = |health: f32| {
            let mut rng = SmallRng::seed_from_u64(61);
            let mut s = boss(terrustia_proto::npc_params::SANTA_NK1, 0.0, 0.0);
            s.life = (s.life_max as f32 * health) as i32;
            s.ai[0] = 1.0;
            (0..300)
                .map(|_| santa(&mut s, &w, &mut rng).shots.len())
                .sum::<usize>()
        };
        assert!(
            shots(0.1) > shots(1.0),
            "a hurt Santa should fire more: {} vs {}",
            shots(0.1),
            shots(1.0)
        );
    }

    /// Daylight ends all of them, each in its own way.
    #[test]
    fn daylight_ends_the_moon() {
        let tiles = Sky(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(4);
        let mut w = night(&tiles, Some((600.0, 400.0)));
        w.conditions.day = true;

        let mut p = boss(PUMPKING, 0.0, 0.0);
        p.ai[0] = 1.0;
        pumpking(&mut p, &w, &mut rng);
        assert!(p.velocity.1 > 0.0, "Pumpking sinks");

        let mut q = boss(terrustia_proto::npc_params::ICE_QUEEN, 0.0, 0.0);
        q.velocity.0 = 1.0;
        ice_queen(&mut q, &w);
        assert!(q.velocity.1 < 0.0, "the Ice Queen climbs away");

        let mut s = boss(terrustia_proto::npc_params::SANTA_NK1, 0.0, 0.0);
        let out = santa(&mut s, &w, &mut rng);
        assert!(out.shots.is_empty(), "Santa stops shooting");
    }
}
