//! Style 43 — the Queen Bee.
//!
//! Three attacks, picked at random and never the same one twice running: she **charges** you along
//! a level line, she **calls bees** from directly above, and she **spits stingers** from higher
//! still. Between them she returns to a chooser state, which is the only place the next attack is
//! decided — so the fight has a rhythm you can read but not predict.
//!
//! Everything scales twice over. As her health falls her charges get faster, she strings more of
//! them together, and her stingers come three times as often. And she is furious about geography:
//! fight her above ground or drag her out of the jungle and every one of those numbers jumps
//! again. That penalty is the game telling you where the fight is supposed to happen.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    BEE, BEE_STRONG, QUEEN_BEE_SPEED, QUEEN_CHARGE, QUEEN_CHARGE_ALIGN, QUEEN_CHARGE_RAGE,
    QUEEN_CHARGES, QUEEN_CHARGING, QUEEN_CHOOSING, QUEEN_CLIMBING, QUEEN_DEFENSE_RAMP,
    QUEEN_GIVE_UP, QUEEN_HOVER, QUEEN_HOVER_ACCEL, QUEEN_LEAVING, QUEEN_STANDOFF,
    QUEEN_STING_ABOVE, QUEEN_STING_EVERY, QUEEN_STING_EVERY_ENRAGED, QUEEN_STINGING,
    QUEEN_SUMMON_ABOVE, QUEEN_SUMMON_EVERY, QUEEN_SUMMONING, QUEEN_SUMMONS, STINGER,
    STINGER_DAMAGE, STINGER_SPEED,
};

use crate::game::ai::{Shot, World};
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Spawn;

/// What a tick of the fight produced.
#[derive(Debug, Default)]
pub struct Hive {
    pub bees: Vec<Spawn>,
    pub stingers: Vec<Shot>,
}

/// How out of place she is: above ground, or outside the jungle.
///
/// Every speed and cadence in the fight reads this, which is why dragging her out of the hive turns
/// her from a boss into a blender.
fn displeasure<T: TileView>(npc: &Npc, world: &World<'_, T>) -> f32 {
    let mut n = 0.0;
    if npc.position.1 < world.conditions.surface_y {
        n += 1.0;
    }
    if !world.conditions.jungle {
        n += 1.0;
    }
    n
}

/// How many of the health thresholds she has passed.
fn wounds(npc: &Npc) -> f32 {
    let health = npc.life as f32 / npc.life_max as f32;
    QUEEN_CHARGE_RAGE
        .iter()
        .filter(|(threshold, _)| health < *threshold)
        .map(|(_, extra)| extra)
        .sum()
}

/// Edge one axis toward a wanted velocity, doubling the push while still going the wrong way.
fn close_on(velocity: &mut f32, wanted: f32, accel: f32) {
    if *velocity < wanted {
        *velocity += accel;
        if *velocity < 0.0 && wanted > 0.0 {
            *velocity += accel;
        }
    } else if *velocity > wanted {
        *velocity -= accel;
        if *velocity > 0.0 && wanted < 0.0 {
            *velocity -= accel;
        }
    }
}

/// Drive the Queen Bee for a tick.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, rng: &mut SmallRng) -> Hive {
    let mut hive = Hive::default();

    // Her defence climbs as her health falls: the fight gets harder as you win it.
    npc.stats.defense +=
        (QUEEN_DEFENSE_RAMP * (1.0 - npc.life as f32 / npc.life_max as f32)) as i32;

    let Some(target) = world.target else {
        npc.ai[0] = QUEEN_LEAVING;
        npc.time_left = npc.time_left.min(10);
        return hive;
    };
    let cross = displeasure(npc, world);
    let (cx, cy) = npc.center();
    let reach = ((target.center.0 - cx).powi(2) + (target.center.1 - cy).powi(2)).sqrt();

    if npc.ai[0] != QUEEN_LEAVING {
        npc.time_left = npc.time_left.max(60);
        if reach > QUEEN_GIVE_UP {
            npc.ai[0] = 4.0;
            npc.dirty = true;
        }
    }
    if !target.alive {
        npc.ai[0] = QUEEN_LEAVING;
        npc.dirty = true;
    }

    if npc.ai[0] == QUEEN_LEAVING || npc.ai[0] == 4.0 {
        npc.velocity.1 *= 0.98;
        npc.direction = if npc.velocity.0 < 0.0 { -1 } else { 1 };
        npc.velocity.0 += 0.08 * f32::from(npc.direction);
        npc.time_left = npc.time_left.min(10);
        npc.sprite_direction = npc.direction;
        npc.dirty = true;
        return hive;
    }

    if npc.ai[0] == QUEEN_CHOOSING {
        // A new attack, and never the one she just finished.
        let last = npc.ai[1];
        let choices = [QUEEN_CHARGING, QUEEN_CLIMBING, QUEEN_STINGING];
        let mut pick = choices[rng.random_range(0..choices.len())];
        let mut guard = 0;
        while pick == last && guard < 8 {
            pick = choices[rng.random_range(0..choices.len())];
            guard += 1;
        }
        npc.ai[0] = pick;
        npc.ai[1] = 0.0;
        npc.ai[2] = 0.0;
        npc.dirty = true;
        return hive;
    }

    if npc.ai[0] == QUEEN_CHARGING {
        // Two charges, and one more for every quarter of her health gone.
        let runs = QUEEN_CHARGES + wounds(npc) as i32 + cross as i32;
        if npc.ai[1] > (2 * runs) as f32 && npc.ai[1] % 2.0 == 0.0 {
            npc.ai[0] = QUEEN_CHOOSING;
            npc.ai[1] = 0.0;
            npc.ai[2] = 0.0;
            npc.dirty = true;
            return hive;
        }

        if npc.ai[1] % 2.0 == 0.0 {
            // Lining up. Level with her target, she commits.
            let align = QUEEN_CHARGE_ALIGN + QUEEN_CHARGE_ALIGN * cross;
            if (cy - target.center.1).abs() < align {
                npc.ai[1] += 1.0;
                npc.ai[2] = 0.0;
                let speed = QUEEN_CHARGE + wounds(npc) + 7.0 * cross;
                let (dx, dy) = (target.center.0 - cx, target.center.1 - cy);
                let k = speed / (dx * dx + dy * dy).sqrt().max(0.01);
                npc.velocity = (dx * k, dy * k);
                npc.sprite_direction = npc.direction;
                npc.dirty = true;
                return hive;
            }
            // Not level yet: climb or dive to their line, holding a standoff.
            let climb = QUEEN_HOVER + wounds(npc) + 3.0 * cross;
            let accel = 0.15 + 0.05 * wounds(npc) + 0.5 * cross;
            if cy < target.center.1 {
                npc.velocity.1 += accel;
            } else {
                npc.velocity.1 -= accel;
            }
            npc.velocity.1 = npc.velocity.1.clamp(-climb, climb);

            let across = (cx - target.center.0).abs();
            if across > QUEEN_STANDOFF.1 {
                npc.velocity.0 += 0.15 * f32::from(npc.direction);
            } else if across < QUEEN_STANDOFF.0 {
                npc.velocity.0 -= 0.15 * f32::from(npc.direction);
            } else {
                npc.velocity.0 *= 0.8;
            }
            npc.velocity.0 = npc.velocity.0.clamp(-16.0, 16.0);
            npc.sprite_direction = npc.direction;
            npc.dirty = true;
            return hive;
        }

        // Mid-charge: it ends when she has overshot far enough, and then she brakes to a stop.
        npc.direction = if npc.velocity.0 < 0.0 { -1 } else { 1 };
        npc.sprite_direction = npc.direction;
        let mut limit = match npc.life as f32 / npc.life_max as f32 {
            h if h < 0.1 => 300.0,
            h if h < 0.25 => 450.0,
            h if h < 0.5 => 500.0,
            h if h < 0.75 => 550.0,
            _ => 600.0,
        };
        limit -= 100.0 * cross;
        let away = if cx < target.center.0 { -1 } else { 1 };
        if (npc.direction == away && (cx - target.center.0).abs() > limit)
            || (cy - target.center.1).abs() > limit * 1.5
        {
            npc.ai[2] = 1.0;
        }
        if npc.ai[2] == 1.0 {
            npc.velocity.0 *= 0.9;
            npc.velocity.1 *= 0.9;
            let stop = 0.1 + 0.05 * wounds(npc);
            if npc.velocity.0.abs() + npc.velocity.1.abs() < stop {
                npc.ai[2] = 0.0;
                npc.ai[1] += 1.0;
                npc.dirty = true;
            }
        }
        npc.dirty = true;
        return hive;
    }

    if npc.ai[0] == QUEEN_CLIMBING {
        // Getting into position above them to call her bees.
        let (dx, dy) = (
            target.center.0 - cx,
            target.center.1 - QUEEN_SUMMON_ABOVE - cy,
        );
        let gap = (dx * dx + dy * dy).sqrt();
        if gap < QUEEN_SUMMON_ABOVE {
            npc.ai[0] = QUEEN_SUMMONING;
            npc.ai[1] = 0.0;
            npc.dirty = true;
            return hive;
        }
        let k = QUEEN_HOVER / gap.max(0.01);
        close_on(&mut npc.velocity.0, dx * k, QUEEN_HOVER_ACCEL);
        close_on(&mut npc.velocity.1, dy * k, QUEEN_HOVER_ACCEL);
        npc.sprite_direction = npc.direction;
        npc.dirty = true;
        return hive;
    }

    if npc.ai[0] == QUEEN_SUMMONING {
        // A bee roughly every forty ticks, sooner the more hurt she is.
        npc.ai[1] += 1.0 + wounds(npc) * 0.25;
        let every = QUEEN_SUMMON_EVERY - 18.0 * cross;
        if npc.ai[1] > every.max(1.0) {
            npc.ai[1] = 0.0;
            npc.ai[2] += 1.0;
            let from = (cx, npc.position.1 + npc.height() * 0.8);
            let (dx, dy) = (target.center.0 - cx, target.center.1 - cy);
            let k = QUEEN_BEE_SPEED / (dx * dx + dy * dy).sqrt().max(0.01);
            hive.bees.push(Spawn {
                npc_type: if rng.random_ratio(1, 2) {
                    BEE
                } else {
                    BEE_STRONG
                },
                position: from,
                velocity: (dx * k, dy * k),
                parent: None,
            });
            npc.dirty = true;
        }
        npc.velocity.0 *= 0.9;
        npc.velocity.1 *= 0.9;
        if npc.ai[2] > QUEEN_SUMMONS {
            npc.ai[0] = QUEEN_CHOOSING;
            npc.ai[1] = QUEEN_SUMMONING;
            npc.dirty = true;
        }
        npc.sprite_direction = npc.direction;
        npc.dirty = true;
        return hive;
    }

    // Stinging: she hangs high and spits, and only downward.
    let (dx, dy) = (
        target.center.0 - cx,
        target.center.1 - QUEEN_STING_ABOVE - cy,
    );
    let gap = (dx * dx + dy * dy).sqrt().max(0.01);
    let speed = 4.0 + 6.0 * cross;
    let accel = 0.05 + 0.2 * cross;
    let k = speed / gap;
    close_on(&mut npc.velocity.0, dx * k, accel);
    close_on(&mut npc.velocity.1, dy * k, accel);

    npc.ai[1] += 1.0;
    let mut every = match npc.life as f32 / npc.life_max as f32 {
        h if h < 0.1 => QUEEN_STING_EVERY_ENRAGED,
        h if h < 0.33 => 25.0,
        h if h < 0.5 => 30.0,
        _ => QUEEN_STING_EVERY,
    };
    every -= 5.0 * cross;
    let every = every.max(1.0);
    if npc.ai[1] % every == every - 1.0 && npc.position.1 + npc.height() < target.center.1 {
        let muzzle = (cx, npc.position.1 + npc.height() * 0.8);
        let scatter = (80.0 - 39.0 * cross).max(1.0) as i32;
        let aim = (
            target.center.0 - muzzle.0 + rng.random_range(-scatter..=scatter) as f32,
            target.center.1 - muzzle.1 + rng.random_range(-scatter..=scatter) as f32,
        );
        let shot_speed = STINGER_SPEED + 5.0 + 7.0 * cross;
        let k = shot_speed / (aim.0 * aim.0 + aim.1 * aim.1).sqrt().max(0.01);
        hive.stingers.push(Shot {
            projectile: STINGER,
            damage: STINGER_DAMAGE,
            position: muzzle,
            velocity: (aim.0 * k, aim.1 * k),
            time_left: 300,
        });
        npc.dirty = true;
    }
    if npc.ai[1] > 300.0 {
        npc.ai[0] = QUEEN_CHOOSING;
        npc.ai[1] = QUEEN_STINGING;
        npc.dirty = true;
    }
    npc.sprite_direction = npc.direction;
    npc.dirty = true;
    hive
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ai::Conditions;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use terrustia_proto::tile::Tile;

    struct Hollow;

    impl TileView for Hollow {
        fn tile(&self, _x: i32, _y: i32) -> Tile {
            Tile::AIR
        }
    }

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(43)
    }

    fn queen() -> Npc {
        Npc::new(222, (10_000.0, 10_000.0), 1).expect("queen bee")
    }

    fn hive_world<'a>(tiles: &'a Hollow, target: Option<Target>) -> World<'a, Hollow> {
        World {
            conditions: Conditions {
                jungle: true,
                surface_y: 0.0,
                ..Conditions::default()
            },
            ..crate::game::ai::calm(tiles, target)
        }
    }

    fn player_at(x: f32, y: f32) -> Target {
        Target {
            slot: 0,
            center: (x, y),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    #[test]
    fn she_never_picks_the_same_attack_twice_running() {
        let tiles = Hollow;
        let mut q = queen();
        let t = Some(player_at(10_400.0, 10_000.0));
        let mut r = rng();
        for last in [QUEEN_CHARGING, QUEEN_CLIMBING, QUEEN_STINGING] {
            for _ in 0..40 {
                q.ai[0] = QUEEN_CHOOSING;
                q.ai[1] = last;
                update(&mut q, &hive_world(&tiles, t), &mut r);
                assert_ne!(q.ai[0], last, "she should not repeat herself");
            }
        }
    }

    #[test]
    fn she_charges_only_once_she_is_level_with_you() {
        let tiles = Hollow;
        let mut q = queen();
        q.ai[0] = QUEEN_CHARGING;
        // Well above them: she should be closing the gap rather than charging.
        let below = Some(player_at(10_400.0, 11_000.0));
        update(&mut q, &hive_world(&tiles, below), &mut rng());
        assert_eq!(q.ai[1] % 2.0, 0.0, "still lining up");

        // Level: she commits.
        let (cx, cy) = q.center();
        let level = Some(player_at(cx + 400.0, cy));
        update(&mut q, &hive_world(&tiles, level), &mut rng());
        assert_eq!(q.ai[1] % 2.0, 1.0, "should have committed");
        let speed = q.velocity.0.hypot(q.velocity.1);
        assert!(
            (speed - QUEEN_CHARGE).abs() < 0.5,
            "at her charge speed, got {speed}"
        );
    }

    #[test]
    fn a_wounded_queen_charges_faster() {
        let tiles = Hollow;
        let charge_speed = |life_fraction: f32| {
            let mut q = queen();
            q.life = (q.life_max as f32 * life_fraction) as i32;
            q.ai[0] = QUEEN_CHARGING;
            let (cx, cy) = q.center();
            let level = Some(player_at(cx + 400.0, cy));
            update(&mut q, &hive_world(&tiles, level), &mut rng());
            q.velocity.0.hypot(q.velocity.1)
        };
        assert!(
            charge_speed(0.05) > charge_speed(1.0),
            "she should speed up as she dies"
        );
    }

    #[test]
    fn she_calls_bees_from_above() {
        let tiles = Hollow;
        let mut q = queen();
        q.ai[0] = QUEEN_SUMMONING;
        let (cx, cy) = q.center();
        let t = Some(player_at(cx, cy + 300.0));
        let mut r = rng();
        let mut called = Vec::new();
        for _ in 0..200 {
            called.extend(update(&mut q, &hive_world(&tiles, t), &mut r).bees);
        }
        assert!(!called.is_empty(), "she should have called bees");
        assert!(
            called
                .iter()
                .all(|b| b.npc_type == BEE || b.npc_type == BEE_STRONG)
        );
        assert!(
            called.iter().all(|b| b.velocity.1 > 0.0),
            "and sent them down at the player"
        );
    }

    #[test]
    fn she_gives_up_calling_after_a_handful() {
        let tiles = Hollow;
        let mut q = queen();
        q.ai[0] = QUEEN_SUMMONING;
        let (cx, cy) = q.center();
        let t = Some(player_at(cx, cy + 300.0));
        let mut r = rng();
        for _ in 0..2000 {
            update(&mut q, &hive_world(&tiles, t), &mut r);
            if q.ai[0] == QUEEN_CHOOSING {
                break;
            }
        }
        assert_eq!(q.ai[0], QUEEN_CHOOSING, "and then move on");
    }

    #[test]
    fn she_spits_stingers_downward_only() {
        let tiles = Hollow;
        let mut q = queen();
        q.ai[0] = QUEEN_STINGING;
        let (cx, cy) = q.center();
        let below = Some(player_at(cx + 50.0, cy + 500.0));
        let mut r = rng();
        let mut spat = Vec::new();
        for _ in 0..200 {
            spat.extend(update(&mut q, &hive_world(&tiles, below), &mut r).stingers);
        }
        assert!(!spat.is_empty(), "she should have spat");
        assert_eq!(spat[0].projectile, STINGER);
        assert!(spat.iter().all(|s| s.velocity.1 > 0.0));

        // With the player above her, nothing leaves.
        let mut high = queen();
        high.ai[0] = QUEEN_STINGING;
        let above = Some(player_at(cx + 50.0, cy - 500.0));
        for _ in 0..200 {
            assert!(
                update(&mut high, &hive_world(&tiles, above), &mut r)
                    .stingers
                    .is_empty()
            );
        }
    }

    /// Drag her out of the jungle and every number in the fight moves against you.
    #[test]
    fn taking_her_out_of_the_jungle_enrages_her() {
        let tiles = Hollow;
        let charge = |jungle: bool| {
            let mut q = queen();
            q.ai[0] = QUEEN_CHARGING;
            let (cx, cy) = q.center();
            let level = Some(player_at(cx + 400.0, cy));
            let mut w = hive_world(&tiles, level);
            w.conditions.jungle = jungle;
            update(&mut q, &w, &mut rng());
            q.velocity.0.hypot(q.velocity.1)
        };
        assert!(
            charge(false) > charge(true) + 5.0,
            "she should be much faster outside her hive"
        );
    }

    #[test]
    fn her_defence_climbs_as_she_is_worn_down() {
        let tiles = Hollow;
        let defence = |life_fraction: f32| {
            let mut q = queen();
            q.life = (q.life_max as f32 * life_fraction) as i32;
            let t = Some(player_at(10_400.0, 10_000.0));
            update(&mut q, &hive_world(&tiles, t), &mut rng());
            q.stats.defense
        };
        assert!(defence(0.05) > defence(1.0));
    }

    #[test]
    fn a_player_who_runs_far_enough_ends_it() {
        let tiles = Hollow;
        let mut q = queen();
        let t = Some(player_at(10_000.0 + QUEEN_GIVE_UP + 100.0, 10_000.0));
        update(&mut q, &hive_world(&tiles, t), &mut rng());
        update(&mut q, &hive_world(&tiles, t), &mut rng());
        assert!(q.time_left <= 10);
    }
}
