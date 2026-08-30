//! Style 54 — the Brain of Cthulhu.
//!
//! Two fights in one, and the switch between them is the creepers.
//!
//! **Shielded**, while any creeper is alive: the Brain is untouchable, drifts at one pixel a tick,
//! and blinks somewhere else every two to seven seconds. It is not trying to kill you; it is a
//! target you cannot hit, orbited by twenty that you can.
//!
//! **Exposed**, the moment the last creeper dies: it becomes vulnerable and charges at eight pixels
//! a tick, smoothed fifty to one so it sweeps through you and comes back round. It also blinks
//! four times as often, and every hit you land shortens the wait — so the more you hurt it, the
//! more it flickers.
//!
//! Both phases lead their target: the teleport offset is nudged along the player's own velocity, so
//! running in a straight line puts you exactly where it appears.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    BRAIN_BLINK_EXPOSED, BRAIN_BLINK_EXPOSED_MP_EXTRA, BRAIN_BLINK_SHIELDED, BRAIN_CHARGE,
    BRAIN_CHARGE_SMOOTH, BRAIN_CREEPERS, BRAIN_DRIFT, BRAIN_FADE_EXPOSED, BRAIN_FADE_SHIELDED,
    BRAIN_GIVE_UP, BRAIN_HOMESICK, BRAIN_LEAD, BRAIN_RANGE_EXPOSED, BRAIN_RANGE_SHIELDED,
    BRAIN_SINK_AFTER, BRAIN_SINK_RATE,
};
use terrustia_proto::tile_solid::solid;

use crate::game::ai::World;
use crate::game::npc::{Npc, TILE, TileView};
use crate::game::npc_ai::Target;

/// What the Brain's tick produced.
#[derive(Debug, Default)]
pub struct Swarm {
    /// Creepers to summon, as (position, velocity). Their type is
    /// [`terrustia_proto::npc_params::CREEPER`].
    pub creepers: Vec<((f32, f32), (f32, f32))>,
    /// Set when the Brain has left the fight.
    pub gone: bool,
}

/// Pick a tile to appear at: a random offset from the target, led along their own motion.
fn blink_to<T: TileView>(
    world: &World<'_, T>,
    target: Target,
    target_velocity: (f32, f32),
    range: (i32, i32),
    need_line_of_sight: bool,
    rng: &mut SmallRng,
) -> Option<(i32, i32)> {
    let goal = (
        (target.center.0 / TILE) as i32,
        (target.center.1 / TILE) as i32,
    );
    for attempt in 0..100 {
        let mut offset = (
            rng.random_range(range.0..=range.1) * 16,
            rng.random_range(range.0..=range.1) * 16,
        );
        if rng.random_ratio(1, 2) {
            offset.0 = -offset.0;
        }
        if rng.random_ratio(1, 2) {
            offset.1 = -offset.1;
        }
        // If the offset points the way the target is already going, throw it further that way.
        let speed = (target_velocity.0.powi(2) + target_velocity.1.powi(2)).sqrt();
        if speed > 0.0 {
            let length = ((offset.0 * offset.0 + offset.1 * offset.1) as f32).sqrt();
            if length > 0.0 {
                let dot = (target_velocity.0 / speed) * (offset.0 as f32 / length)
                    + (target_velocity.1 / speed) * (offset.1 as f32 / length);
                if dot > 0.0 {
                    offset.0 += (offset.0 as f32 / length * BRAIN_LEAD * speed) as i32;
                    offset.1 += (offset.1 as f32 / length * BRAIN_LEAD * speed) as i32;
                }
            }
        }
        let spot = (goal.0 + offset.0 / 16, goal.1 + offset.1 / 16);
        let tile = world.tiles.tile(spot.0, spot.1);
        let blocked = tile.is_active() && solid(tile.block);
        if !blocked {
            // The shielded phase also wants to be able to see you from there, until it gets
            // desperate enough to stop caring.
            if !need_line_of_sight || attempt > 75 {
                return Some(spot);
            }
            let from = ((spot.0 * 16) as f32, (spot.1 * 16) as f32);
            if crate::game::ai::sight::can_hit(
                world.tiles,
                from,
                (1, 1),
                (
                    target.center.0 - crate::game::ai::PLAYER_WIDTH as f32 / 2.0,
                    target.center.1 - crate::game::ai::PLAYER_HEIGHT as f32 / 2.0,
                ),
                (
                    crate::game::ai::PLAYER_WIDTH,
                    crate::game::ai::PLAYER_HEIGHT,
                ),
            ) {
                return Some(spot);
            }
        }
    }
    None
}

/// Drive the Brain of Cthulhu for a tick.
///
/// `creepers_alive` is the count the caller reads off the NPC table; `at_home` says whether the
/// target is still in the crimson.
pub fn update<T: TileView>(
    npc: &mut Npc,
    world: &World<'_, T>,
    creepers_alive: usize,
    at_home: bool,
    target_velocity: (f32, f32),
    rng: &mut SmallRng,
) -> Swarm {
    let mut swarm = Swarm::default();

    // First tick: it arrives wrapped in its creepers.
    if npc.local_ai[0] == 0.0 {
        npc.local_ai[0] = 1.0;
        for _ in 0..BRAIN_CREEPERS {
            let at = (
                npc.center().0 + rng.random_range(-npc.stats.width..npc.stats.width) as f32,
                npc.center().1 + rng.random_range(-npc.stats.height..npc.stats.height) as f32,
            );
            swarm.creepers.push((
                at,
                (
                    rng.random_range(-30..31) as f32 * 0.1,
                    rng.random_range(-30..31) as f32 * 0.1,
                ),
            ));
        }
        npc.dirty = true;
    }

    let Some(target) = world.target else {
        swarm.gone = true;
        return swarm;
    };
    let (cx, cy) = npc.center();
    if (cx - target.center.0).abs() + (cy - target.center.1).abs() > BRAIN_GIVE_UP {
        swarm.gone = true;
        return swarm;
    }

    let exposed = npc.ai[0] < 0.0;
    // The creepers are the armour. While one lives, nothing you do lands.
    npc.stats.dont_take_damage = !exposed;

    let (dx, dy) = (target.center.0 - cx, target.center.1 - cy);
    let reach = (dx * dx + dy * dy).sqrt().max(f32::MIN_POSITIVE);

    if exposed {
        // Charging, and turning like a barge.
        let k = BRAIN_CHARGE / reach;
        npc.velocity.0 =
            (npc.velocity.0 * BRAIN_CHARGE_SMOOTH + dx * k) / (BRAIN_CHARGE_SMOOTH + 1.0);
        npc.velocity.1 =
            (npc.velocity.1 * BRAIN_CHARGE_SMOOTH + dy * k) / (BRAIN_CHARGE_SMOOTH + 1.0);

        match npc.ai[0] as i32 {
            -1 => {
                npc.local_ai[1] += 1.0;
                // Every hit it takes brings the next blink forward.
                if world.was_hurt {
                    npc.local_ai[1] -= rng.random_range(0..5) as f32;
                }
                // BRN-1: a dedicated server is netMode 2, where the exposed blink wait gains a
                // further 30-89 ticks (`num859 += Main.rand.Next(30, 90)`, `NPC.cs:32690-32693`).
                // This ran the single-player formula, which omits it, so it blinked too eagerly.
                let wait = (BRAIN_BLINK_EXPOSED.0
                    + rng.random_range(0..BRAIN_BLINK_EXPOSED.1)
                    + rng.random_range(
                        BRAIN_BLINK_EXPOSED_MP_EXTRA.0..BRAIN_BLINK_EXPOSED_MP_EXTRA.1,
                    )) as f32;
                if npc.local_ai[1] >= wait
                    && let Some(spot) = blink_to(
                        world,
                        target,
                        target_velocity,
                        BRAIN_RANGE_EXPOSED,
                        false,
                        rng,
                    )
                {
                    npc.local_ai[1] = 0.0;
                    npc.ai[3] = 0.0;
                    npc.ai[0] = -2.0;
                    npc.ai[1] = spot.0 as f32;
                    npc.ai[2] = spot.1 as f32;
                    npc.dirty = true;
                }
            }
            -2 => {
                // Fading out, and slowing as it goes.
                npc.velocity.0 *= 0.9;
                npc.velocity.1 *= 0.9;
                npc.ai[3] += BRAIN_FADE_EXPOSED;
                if npc.ai[3] >= 255.0 {
                    npc.ai[3] = 255.0;
                    npc.position.0 = npc.ai[1] * 16.0 - npc.width() / 2.0;
                    npc.position.1 = npc.ai[2] * 16.0 - npc.height() / 2.0;
                    npc.ai[0] = -3.0;
                    npc.dirty = true;
                }
            }
            _ => {
                npc.ai[3] -= BRAIN_FADE_EXPOSED;
                if npc.ai[3] <= 0.0 {
                    npc.ai[3] = 0.0;
                    npc.ai[0] = -1.0;
                    npc.dirty = true;
                }
            }
        }
    } else {
        // Shielded: a slow drift, and a blink on a long timer.
        if reach < BRAIN_DRIFT {
            npc.velocity = (dx, dy);
        } else {
            let k = BRAIN_DRIFT / reach;
            npc.velocity = (dx * k, dy * k);
        }

        match npc.ai[0] as i32 {
            0 => {
                if creepers_alive == 0 {
                    // The armour is gone.
                    npc.ai[0] = -1.0;
                    npc.local_ai[1] = 0.0;
                    npc.ai[3] = 0.0;
                    npc.dirty = true;
                    return swarm;
                }
                npc.local_ai[1] += 1.0;
                let wait =
                    (BRAIN_BLINK_SHIELDED.0 + rng.random_range(0..BRAIN_BLINK_SHIELDED.1)) as f32;
                if npc.local_ai[1] >= wait
                    && let Some(spot) = blink_to(
                        world,
                        target,
                        target_velocity,
                        BRAIN_RANGE_SHIELDED,
                        true,
                        rng,
                    )
                {
                    npc.local_ai[1] = 0.0;
                    npc.ai[0] = 1.0;
                    npc.ai[1] = spot.0 as f32;
                    npc.ai[2] = spot.1 as f32;
                    npc.dirty = true;
                }
            }
            1 => {
                npc.ai[3] += BRAIN_FADE_SHIELDED;
                if npc.ai[3] >= 255.0 {
                    npc.ai[3] = 255.0;
                    npc.position.0 = npc.ai[1] * 16.0 - npc.width() / 2.0;
                    npc.position.1 = npc.ai[2] * 16.0 - npc.height() / 2.0;
                    npc.ai[0] = 2.0;
                    npc.dirty = true;
                }
            }
            _ => {
                npc.ai[3] -= BRAIN_FADE_SHIELDED;
                if npc.ai[3] <= 0.0 {
                    npc.ai[3] = 0.0;
                    npc.ai[0] = 0.0;
                    npc.dirty = true;
                }
            }
        }
    }

    // Out of the crimson, or with nobody left alive: it sinks out of the world.
    if !at_home || !target.alive {
        if npc.local_ai[3] < BRAIN_HOMESICK {
            npc.local_ai[3] += 1.0;
        }
        if npc.local_ai[3] > BRAIN_SINK_AFTER {
            npc.velocity.1 += (npc.local_ai[3] - BRAIN_SINK_AFTER) * BRAIN_SINK_RATE;
        }
        npc.ai[0] = 2.0;
        npc.dirty = true;
    } else if npc.local_ai[3] > 0.0 {
        npc.local_ai[3] -= 1.0;
    }

    npc.dirty = true;
    swarm
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Crimson(HashMap<(i32, i32), Tile>);

    impl TileView for Crimson {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(54)
    }

    fn brain() -> Npc {
        Npc::new(266, (10_000.0, 10_000.0), 1).expect("brain of cthulhu")
    }

    fn world<'a>(tiles: &'a Crimson, target: Option<Target>) -> World<'a, Crimson> {
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

    #[test]
    fn it_arrives_wrapped_in_twenty_creepers() {
        let tiles = Crimson::default();
        let mut b = brain();
        let (cx, cy) = b.center();
        let swarm = update(
            &mut b,
            &world(&tiles, Some(player_at(cx + 300.0, cy))),
            0,
            true,
            (0.0, 0.0),
            &mut rng(),
        );
        assert_eq!(swarm.creepers.len(), BRAIN_CREEPERS);
        assert_eq!(terrustia_proto::npc_params::CREEPER, 267);

        // And only once.
        let again = update(
            &mut b,
            &world(&tiles, Some(player_at(cx + 300.0, cy))),
            20,
            true,
            (0.0, 0.0),
            &mut rng(),
        );
        assert!(again.creepers.is_empty());
    }

    #[test]
    fn the_creepers_are_its_armour() {
        let tiles = Crimson::default();
        let mut b = brain();
        b.local_ai[0] = 1.0;
        let (cx, cy) = b.center();
        let t = Some(player_at(cx + 300.0, cy));
        update(&mut b, &world(&tiles, t), 20, true, (0.0, 0.0), &mut rng());
        assert!(b.stats.dont_take_damage, "untouchable while shielded");

        // Kill them all: it drops into the second phase and becomes vulnerable.
        update(&mut b, &world(&tiles, t), 0, true, (0.0, 0.0), &mut rng());
        assert_eq!(b.ai[0], -1.0);
        update(&mut b, &world(&tiles, t), 0, true, (0.0, 0.0), &mut rng());
        assert!(!b.stats.dont_take_damage, "now you can hurt it");
    }

    #[test]
    fn it_drifts_while_shielded_and_charges_once_exposed() {
        let tiles = Crimson::default();
        // Sample the peak speed over the run rather than the speed at one arbitrary tick: a blink
        // damps the velocity mid-fade, so the instantaneous final speed depends on where the blink
        // cadence happens to land, while the peak captures the charge itself.
        let speed_of = |exposed: bool| {
            let mut b = brain();
            b.local_ai[0] = 1.0;
            if exposed {
                b.ai[0] = -1.0;
            }
            let (cx, cy) = b.center();
            let t = Some(player_at(cx + 3000.0, cy));
            let mut peak = 0.0_f32;
            for _ in 0..600 {
                update(
                    &mut b,
                    &world(&tiles, t),
                    if exposed { 0 } else { 20 },
                    true,
                    (0.0, 0.0),
                    &mut rng(),
                );
                peak = peak.max((b.velocity.0.powi(2) + b.velocity.1.powi(2)).sqrt());
            }
            peak
        };
        let drift = speed_of(false);
        let charge = speed_of(true);
        assert!(
            (drift - BRAIN_DRIFT).abs() < 0.1,
            "shielded it should crawl, got {drift}"
        );
        assert!(
            charge > drift * 4.0,
            "exposed it should charge, got {charge} against {drift}"
        );
    }

    #[test]
    fn it_blinks_much_more_often_once_exposed() {
        assert!(BRAIN_BLINK_EXPOSED.0 < BRAIN_BLINK_SHIELDED.0);
        let tiles = Crimson::default();
        let mut b = brain();
        b.local_ai[0] = 1.0;
        b.ai[0] = -1.0;
        let (cx, cy) = b.center();
        let t = Some(player_at(cx + 400.0, cy));
        let start = b.position;
        let mut r = rng();
        for _ in 0..600 {
            update(&mut b, &world(&tiles, t), 0, true, (0.0, 0.0), &mut r);
        }
        assert!(
            (b.position.0 - start.0).abs() > 100.0 || (b.position.1 - start.1).abs() > 100.0,
            "it should have blinked somewhere"
        );
    }

    #[test]
    fn hitting_it_brings_the_next_blink_forward() {
        let tiles = Crimson::default();
        let ticks_to_blink = |hit: bool| {
            let mut b = brain();
            b.local_ai[0] = 1.0;
            b.ai[0] = -1.0;
            let (cx, cy) = b.center();
            let t = Some(player_at(cx + 400.0, cy));
            let mut r = rng();
            let mut w = world(&tiles, t);
            w.was_hurt = hit;
            let mut n = 0;
            for tick in 0..2000 {
                update(&mut b, &w, 0, true, (0.0, 0.0), &mut r);
                if b.ai[0] != -1.0 {
                    n = tick;
                    break;
                }
            }
            n
        };
        // Being hit only ever subtracts from the timer, so it can never take longer.
        assert!(ticks_to_blink(true) <= ticks_to_blink(false) + 1);
    }

    #[test]
    fn it_leads_a_running_player() {
        let tiles = Crimson::default();
        let t = player_at(10_000.0, 10_000.0);
        let mut r = rng();
        let mut ahead = 0;
        for _ in 0..40 {
            if let Some(spot) = blink_to(
                &world(&tiles, Some(t)),
                t,
                (8.0, 0.0),
                BRAIN_RANGE_EXPOSED,
                false,
                &mut r,
            ) && (spot.0 * 16) as f32 > t.center.0
            {
                ahead += 1;
            }
        }
        assert!(
            ahead > 0,
            "some of its blinks should land ahead of a running player"
        );
    }

    #[test]
    fn leaving_the_crimson_makes_it_sink_away() {
        let tiles = Crimson::default();
        let mut b = brain();
        b.local_ai[0] = 1.0;
        let (cx, cy) = b.center();
        let t = Some(player_at(cx + 300.0, cy));
        let mut r = rng();
        for _ in 0..(BRAIN_SINK_AFTER as i32 + 20) {
            update(&mut b, &world(&tiles, t), 20, false, (0.0, 0.0), &mut r);
        }
        assert!(
            b.velocity.1 > 0.0,
            "should be sinking, got {}",
            b.velocity.1
        );
    }

    #[test]
    fn a_player_who_runs_right_away_ends_the_fight() {
        let tiles = Crimson::default();
        let mut b = brain();
        b.local_ai[0] = 1.0;
        let (cx, cy) = b.center();
        let swarm = update(
            &mut b,
            &world(
                &tiles,
                Some(player_at(cx + BRAIN_GIVE_UP, cy + BRAIN_GIVE_UP)),
            ),
            20,
            true,
            (0.0, 0.0),
            &mut rng(),
        );
        assert!(swarm.gone);
    }

    /// BRN-1: a dedicated server is netMode 2, where the exposed teleport fades at 15 a tick, not
    /// the single-player 25 (`NPC.cs:32744` against `32748`). `ai[3]` climbs 0 -> 255 through the
    /// fade-out (`ai[0] == -2`), so the slower rate takes noticeably longer.
    #[test]
    fn its_exposed_fade_uses_the_slower_multiplayer_rate() {
        let tiles = Crimson::default();
        let mut b = brain();
        b.local_ai[0] = 1.0;
        b.ai[0] = -2.0; // fading out toward a chosen tile
        b.ai[1] = 100.0;
        b.ai[2] = 100.0;
        b.ai[3] = 0.0;
        let (cx, cy) = b.center();
        let t = Some(player_at(cx + 300.0, cy));
        let mut r = rng();
        let mut ticks = 0;
        for _ in 0..40 {
            update(&mut b, &world(&tiles, t), 0, true, (0.0, 0.0), &mut r);
            ticks += 1;
            if b.ai[0] == -3.0 {
                break;
            }
        }
        // 255 / 15 = 17 ticks (dedicated server); 255 / 25 = 11 (single-player).
        assert!(
            ticks >= 15,
            "the fade should take the slower multiplayer count, got {ticks}"
        );
    }

    /// BRN-1: on a dedicated server the exposed blink wait gains another 30-89 ticks
    /// (`num859 += Main.rand.Next(30, 90)`, `NPC.cs:32690-32693`), so the minimum wait is 90, not
    /// the single-player 60. Across seeds it therefore never blinks before tick 90.
    #[test]
    fn the_exposed_blink_never_fires_before_the_multiplayer_minimum() {
        let tiles = Crimson::default();
        let earliest = (0..200u64)
            .map(|seed| {
                let mut b = brain();
                b.local_ai[0] = 1.0;
                b.ai[0] = -1.0;
                let (cx, cy) = b.center();
                let t = Some(player_at(cx + 400.0, cy));
                let mut r = SmallRng::seed_from_u64(seed);
                for tick in 0..400 {
                    update(&mut b, &world(&tiles, t), 0, true, (0.0, 0.0), &mut r);
                    if b.ai[0] != -1.0 {
                        return tick;
                    }
                }
                400
            })
            .min()
            .unwrap();
        assert!(
            earliest >= 90,
            "the earliest exposed blink should be the multiplayer 90, got {earliest}"
        );
    }
}
