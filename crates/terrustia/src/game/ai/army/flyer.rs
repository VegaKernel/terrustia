//! Style 108: the diving flyers.
//!
//! A wyvern and a kobold flyer share one routine and mean opposite things by it. Both circle above
//! you at four hundred pixels, wind up, and dive. The wyvern pulls out once it has gone past and
//! climbs back for another go. The kobold flyer does not pull out — it is carrying a bomb, and the
//! dive is the delivery.
//!
//! The circling is not a hover. It aims at a point above the target rather than the target, which
//! is what keeps it up there, and it will not commit while the angle is too shallow — so a flyer
//! sitting level with you circles rather than diving, and the fight is vertical.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    ARMY_FADE_IN, DIVING_FLYER_EXPLOSION, DIVING_FLYER_EXPLOSION_DAMAGE, DivingFlyer, diving_flyer,
};

use crate::game::ai::World;
use crate::game::npc::{Npc, TileView};

/// What the flyer is doing, as `ai[0]` numbers it.
mod state {
    /// Circling, looking for an angle.
    pub const CIRCLING: f32 = 0.0;
    /// Wound up and about to go.
    pub const WIND_UP: f32 = 1.0;
    /// Committed.
    pub const DIVING: f32 = 2.0;
    /// Gone off.
    pub const BURSTING: f32 = 3.0;
    /// Pulling out and getting its breath back.
    pub const RECOVERING: f32 = 4.0;
}

/// What it did this tick.
#[derive(Debug, Default)]
pub struct FlyerOutcome {
    /// Set on the tick it goes off — the hitbox has already been swollen to match.
    pub burst: bool,
    pub spent: bool,
}

/// The shallowest angle it will dive at, as a fraction of a half-turn either side of straight down.
const DIVE_CONE: f32 = 8.0;

/// Push away from everything of the same kind close enough to be in the way.
///
/// The game measures Manhattan distance against the NPC's own width and pushes a fixed step per
/// neighbour, so a tight knot pushes itself apart faster than a loose one.
pub(super) fn separate(npc: &mut Npc, kin: &[(f32, f32, f32)], step: f32) {
    let (cx, cy) = npc.center();
    // The third element is the reach a shimmerfly notices an entry at; this routine brings its own
    // separation distance and ignores it (`World::avoid`).
    for &(kx, ky, _) in kin {
        let (dx, dy) = (cx - kx, cy - ky);
        if (dx == 0.0 && dy == 0.0) || dx.abs() + dy.abs() >= npc.width() {
            continue;
        }
        npc.velocity.0 += if dx < 0.0 { -step } else { step };
        npc.velocity.1 += if dy < 0.0 { -step } else { step };
    }
}

pub fn diving(npc: &mut Npc, world: &World<'_, impl TileView>, rng: &mut SmallRng) -> FlyerOutcome {
    let mut out = FlyerOutcome::default();
    npc.dirty = true;
    let it: DivingFlyer = diving_flyer(npc.npc_type);
    npc.rotation = npc.velocity.1.atan2(npc.velocity.0);

    // It comes out of its gate faded, over a second.
    if npc.local_ai[0] < ARMY_FADE_IN {
        npc.local_ai[0] += 1.0;
    }

    // Flyers of a type push each other apart, so a gate's steady output spreads into a flock
    // rather than a column. The push is a fixed nudge per neighbour, not a falloff.
    separate(npc, world.avoid, it.separation);

    if npc.velocity.0 != 0.0 {
        npc.sprite_direction = -npc.velocity.0.signum() as i8;
    }
    // The sprite is drawn nose-first either way, so the rotation is folded into a half-turn.
    if npc.rotation < -std::f32::consts::FRAC_PI_2 {
        npc.rotation += std::f32::consts::PI;
    }
    if npc.rotation > std::f32::consts::FRAC_PI_2 {
        npc.rotation -= std::f32::consts::PI;
    }

    let Some(target) = world.target.filter(|t| t.alive) else {
        return out;
    };
    let (cx, cy) = npc.center();
    let toward = (target.center.0 - cx, target.center.1 - cy);
    let range = toward.0.hypot(toward.1);
    let unit = |v: (f32, f32)| {
        let length = v.0.hypot(v.1).max(f32::MIN_POSITIVE);
        (v.0 / length, v.1 / length)
    };
    let at_target = unit(toward);
    // The point it actually flies to while circling is above its target, not on it.
    let above = unit((toward.0, toward.1 - it.hover_above));
    let accel = it.accel * it.turn;

    match npc.ai[0] {
        state::CIRCLING => {
            npc.knockback_immune = it.knockback_resist == 0.0;
            // It will only commit when it is properly above: the angle of the run must fall inside
            // a cone around straight down.
            let angle = toward.1.atan2(toward.0);
            let cone = std::f32::consts::PI / DIVE_CONE;
            let lined_up = (crate::game::ai::can_see(world.tiles, npc, target)
                || npc.ai[3] >= it.patience)
                && angle > cone
                && angle < std::f32::consts::PI - cone;

            if range > it.engage || !lined_up {
                npc.velocity.0 =
                    (npc.velocity.0 * (it.approach - 1.0) + above.0 * it.speed) / it.approach;
                npc.velocity.1 =
                    (npc.velocity.1 * (it.approach - 1.0) + above.1 * it.speed) / it.approach;
                // It never lets its target get above it.
                if target.center.1 < cy {
                    npc.velocity.1 = (npc.velocity.1 - 0.2).max(-10.0);
                }
                if lined_up {
                    npc.ai[3] = 0.0;
                } else {
                    npc.ai[3] += 1.0;
                }
            } else {
                // Committed: the run is fixed here and does not re-aim during the wind-up.
                npc.ai[0] = state::WIND_UP;
                npc.ai[2] = at_target.0 * it.speed;
                npc.ai[3] = at_target.1 * it.speed;
            }
        }
        state::WIND_UP => {
            npc.knockback_immune = true;
            npc.velocity.0 *= it.decay;
            npc.velocity.1 = npc.velocity.1 * it.decay + it.sink;
            npc.ai[1] += 1.0;
            if npc.ai[1] >= it.wind_up {
                npc.ai[0] = state::DIVING;
                npc.ai[1] = 0.0;
                let jitter = |rng: &mut SmallRng| {
                    if it.spread == 0 {
                        0.0
                    } else {
                        rng.random_range(-it.spread..=it.spread) as f32 * 0.04
                    }
                };
                let run = (npc.ai[2] + jitter(rng), npc.ai[3] + jitter(rng));
                let run = unit(run);
                npc.velocity = (run.0 * it.dive_speed, run.1 * it.dive_speed);
            }
        }
        state::DIVING => {
            npc.knockback_immune = true;
            npc.ai[1] += 1.0;
            let speed = npc.velocity.0.hypot(npc.velocity.1);
            // A dive ends when it has gone past and below, or when it has simply run out of speed.
            // A kobold flyer never satisfies the first and cannot satisfy the second, so it only
            // ever ends by hitting something.
            let past = !it.commits && range > it.break_off && cy > target.center.1;
            if (npc.ai[1] >= it.dive_ticks && past) || speed < it.min_dive_speed {
                npc.velocity = (npc.velocity.0 / 2.0, npc.velocity.1 / 2.0);
                npc.ai[1] = 45.0;
                npc.ai[0] = state::RECOVERING;
                npc.ai[2] = 0.0;
                npc.ai[3] = 0.0;
            } else {
                // Mid-dive it still steers, slowly, and gains speed as it goes.
                npc.velocity.0 =
                    (npc.velocity.0 * (it.turn - 1.0) + at_target.0 * (speed + accel)) / it.turn;
                npc.velocity.1 =
                    (npc.velocity.1 * (it.turn - 1.0) + at_target.1 * (speed + accel)) / it.turn;
            }
            if it.splat
                && crate::game::ai::sight::solid_collision(
                    world.tiles,
                    npc.position,
                    (npc.stats.width, npc.stats.height),
                )
            {
                burst(npc);
            }
        }
        state::RECOVERING => {
            npc.ai[1] -= 3.0;
            if npc.ai[1] <= 0.0 {
                npc.ai[0] = state::CIRCLING;
                npc.ai[1] = 0.0;
            }
            npc.velocity.0 *= 0.95;
            npc.velocity.1 *= 0.95;
        }
        _ => {}
    }

    // Close enough is close enough: a bomb-carrier goes off on contact wherever it is in its run.
    if it.splat && npc.ai[0] != state::BURSTING && range < 64.0 {
        burst(npc);
    }

    if npc.ai[0] == state::BURSTING {
        npc.ai[1] += 1.0;
        out.burst = npc.ai[1] >= 3.0;
        out.spent = out.burst;
    }
    out
}

/// Swell it into its blast and stop it dead. The damage is the blast's, not the creature's.
fn burst(npc: &mut Npc) {
    let (cx, cy) = npc.center();
    npc.ai[0] = state::BURSTING;
    npc.ai[1] = 0.0;
    npc.ai[2] = 0.0;
    npc.ai[3] = 0.0;
    npc.size = Some((DIVING_FLYER_EXPLOSION as f32, DIVING_FLYER_EXPLOSION as f32));
    npc.position = (
        cx - DIVING_FLYER_EXPLOSION as f32 / 2.0,
        cy - DIVING_FLYER_EXPLOSION as f32 / 2.0,
    );
    npc.velocity = (0.0, 0.0);
    npc.set_contact_damage(DIVING_FLYER_EXPLOSION_DAMAGE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::npc_params::{DD2_KOBOLD_FLYER_T2, DD2_WYVERN_T1};
    use terrustia_proto::tile::Tile;

    struct Sky(HashMap<(i32, i32), Tile>);

    impl TileView for Sky {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn world<'a>(tiles: &'a Sky, target: (f32, f32)) -> World<'a, Sky> {
        crate::game::ai::calm(
            tiles,
            Some(Target {
                slot: 0,
                center: target,
                velocity: (0.0, 0.0),
                alive: true,
            }),
        )
    }

    fn flyer(npc_type: u16, at: (f32, f32)) -> Npc {
        Npc::new(npc_type, at, 1).expect("a flyer")
    }

    /// One tick: decide, then actually move. Testing the routine without the movement would test
    /// nothing — every state it has is about where it has got to.
    fn tick(npc: &mut Npc, w: &World<'_, Sky>, tiles: &Sky, rng: &mut SmallRng) -> FlyerOutcome {
        let out = diving(npc, w, rng);
        npc.no_gravity = true;
        crate::game::npc::step_physics(npc, tiles);
        out
    }

    /// A wyvern circling above its target eventually commits, dives, and comes back out of it.
    #[test]
    fn a_wyvern_dives_and_pulls_out() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, (2000.0, 2000.0));
        let mut rng = SmallRng::seed_from_u64(1);
        let mut n = flyer(DD2_WYVERN_T1, (2000.0, 1600.0));

        let mut seen = Vec::new();
        for _ in 0..2000 {
            tick(&mut n, &w, &tiles, &mut rng);
            if seen.last() != Some(&n.ai[0]) {
                seen.push(n.ai[0]);
            }
        }
        assert!(seen.contains(&state::DIVING), "it should dive: {seen:?}");
        assert!(
            seen.contains(&state::RECOVERING),
            "and pull out again: {seen:?}"
        );
        assert!(
            !seen.contains(&state::BURSTING),
            "a wyvern does not explode: {seen:?}"
        );
    }

    /// It will not dive from level: the angle has to be steep enough first.
    #[test]
    fn it_will_not_dive_from_level() {
        let tiles = Sky(HashMap::new());
        // Target exactly level with it, well inside engagement range.
        let w = world(&tiles, (2300.0, 1600.0));
        let mut rng = SmallRng::seed_from_u64(2);
        let mut n = flyer(DD2_WYVERN_T1, (2000.0, 1600.0));
        let mut climbed = false;
        for _ in 0..40 {
            tick(&mut n, &w, &tiles, &mut rng);
            climbed |= n.ai[0] != state::CIRCLING;
        }
        assert!(!climbed, "it should still be circling, not diving");
    }

    /// A kobold flyer commits: once diving it never recovers, and it goes off on contact.
    #[test]
    fn a_kobold_flyer_does_not_pull_out() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, (2000.0, 2000.0));
        let mut rng = SmallRng::seed_from_u64(3);
        let mut n = flyer(DD2_KOBOLD_FLYER_T2, (2000.0, 1600.0));

        let mut burst = false;
        let mut recovered = false;
        for _ in 0..2000 {
            let out = tick(&mut n, &w, &tiles, &mut rng);
            recovered |= n.ai[0] == state::RECOVERING;
            if out.burst {
                burst = true;
                break;
            }
        }
        assert!(burst, "it should have gone off");
        assert!(!recovered, "and never pulled out");
    }

    /// The blast is the size of the blast, not the creature.
    #[test]
    fn the_blast_is_bigger_than_the_bomber() {
        let mut n = flyer(DD2_KOBOLD_FLYER_T2, (1000.0, 1000.0));
        let before = n.center();
        let small = n.width();
        burst(&mut n);
        assert_eq!(
            (n.width(), n.height()),
            (192.0, 192.0),
            "it swells to its blast"
        );
        assert!(small < 192.0);
        let after = n.center();
        assert!(
            (after.0 - before.0).abs() < 0.01 && (after.1 - before.1).abs() < 0.01,
            "and swells around where it was, not off to one side"
        );
        assert_eq!(n.velocity, (0.0, 0.0), "and stops dead");
    }

    /// A ceiling stops a wyvern's dive being infinite: it only gives up when it has gone past.
    #[test]
    fn a_dive_that_misses_still_ends() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, (2000.0, 2000.0));
        let mut rng = SmallRng::seed_from_u64(7);
        let mut n = flyer(DD2_WYVERN_T1, (2000.0, 1600.0));
        let mut longest = 0.0f32;
        for _ in 0..4000 {
            tick(&mut n, &w, &tiles, &mut rng);
            if n.ai[0] == state::DIVING {
                longest = longest.max(n.ai[1]);
            }
        }
        assert!(longest > 0.0, "it dove at all");
        assert!(longest < 600.0, "and no dive ran ten seconds: {longest}");
    }
}
