//! Plantera: styles 51, 52 and 53.
//!
//! Plantera does not fly. It *swings* — three hooks bite into the walls around you, and the body
//! is pulled toward a point five hundred pixels from the average of those three anchors in your
//! direction. That is why it moves the way it does in a corridor and the way it does in a cavern,
//! and why the fight is really about the room you fight it in.
//!
//! The two halves are different creatures. Above half health it is armoured and shoots seeds, with
//! thorn balls and spiky seeds mixed in below eighty per cent — each of which costs it a pause, so
//! the heavier shots buy you time. Below half it sheds most of that armour, hits half again as
//! hard, and grows eight tentacles that orbit it at a radius which *widens* as it dies.
//!
//! Dragged out of the jungle it becomes far faster and hits twice as hard. Like the Golem, it
//! refuses to be fought anywhere but home.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    HOOK_HURRY_ENRAGED, HOOK_HURRY_HALF, HOOK_HURRY_QUARTER, HOOK_REST, HOOK_SEARCH,
    HOOK_SEARCH_WIDEN, HOOK_SPEED, HOOK_SPEED_HALF, HOOK_SPEED_QUARTER, HOOK_STAGGER,
    PLANTERA_ACCEL, PLANTERA_ACCEL_HALF, PLANTERA_CHARGE, PLANTERA_DAMAGE, PLANTERA_DEFENSE,
    PLANTERA_ENRAGED_ACCEL, PLANTERA_ENRAGED_LEASH, PLANTERA_ENRAGED_SPEED, PLANTERA_HOOK,
    PLANTERA_HOOKS, PLANTERA_LEASH, PLANTERA_LEASH_EXPERT, PLANTERA_MIX_AT, PLANTERA_SECOND_DAMAGE,
    PLANTERA_SECOND_DEFENSE, PLANTERA_SEED, PLANTERA_SEED_DAMAGE, PLANTERA_SEED_SPEED,
    PLANTERA_SEED_SPEED_EXPERT, PLANTERA_SPEED, PLANTERA_SPEED_HALF, PLANTERA_SPEED_QUARTER,
    PLANTERA_SPIKY, PLANTERA_SPIKY_DAMAGE, PLANTERA_SPIKY_REST, PLANTERA_TENTACLE,
    PLANTERA_TENTACLES, PLANTERA_THORN_BALL, PLANTERA_THORN_BALL_DAMAGE, PLANTERA_THORN_BALL_REST,
    TENTACLE_ACCEL, TENTACLE_ACCEL_EXPERT, TENTACLE_CAP, TENTACLE_DRIFT, TENTACLE_EXPERT_RADIUS,
    TENTACLE_RADIUS, TENTACLE_RADIUS_QUARTER, TENTACLE_RADIUS_TENTH, TENTACLE_SPREAD,
};

use super::skeletron::Parent;
use crate::game::ai::{Shot, World, can_see};
use crate::game::npc::{Npc, TILE, TileView};
use crate::game::npc_ai::Spawn;

/// What Plantera's fight looks like from the outside.
#[derive(Debug, Clone, Copy)]
pub struct PlanteraState {
    /// Where its hooks have bitten, averaged. `None` before any have.
    pub hooks: Option<(f32, f32)>,
    /// Whether the player is in the jungle, underground and above the underworld — the only place
    /// Plantera fights at its ordinary pace.
    pub at_home: bool,
}

/// What a piece of Plantera did this tick.
#[derive(Debug, Default)]
pub struct PlanteraOutcome {
    pub shots: Vec<Shot>,
    pub spawn: Vec<Spawn>,
    pub spent: bool,
}

/// Style 51: the body.
pub fn plantera(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    state: PlanteraState,
    rng: &mut SmallRng,
) -> PlanteraOutcome {
    let mut out = PlanteraOutcome::default();
    npc.dirty = true;

    // Its hooks go out on the first tick.
    if npc.local_ai[0] == 0.0 {
        npc.local_ai[0] = 1.0;
        let at = npc.center();
        for _ in 0..PLANTERA_HOOKS {
            out.spawn.push(Spawn {
                npc_type: PLANTERA_HOOK,
                position: at,
                velocity: (0.0, 0.0),
                parent: Some(Spawn::OWN_PARENT),
            });
        }
    }

    let Some(target) = world.target else {
        return out;
    };
    let dead = !target.alive;
    let enraged = !state.at_home;
    let health = npc.life as f32 / npc.life_max.max(1) as f32;
    let expert = world.conditions.expert;

    // How fast it swings. Every threshold makes it quicker; leaving the jungle makes it far
    // quicker than any of them.
    let (mut speed, mut accel) = (PLANTERA_SPEED, PLANTERA_ACCEL);
    if npc.life < npc.life_max / 2 {
        speed = PLANTERA_SPEED_HALF;
        accel = PLANTERA_ACCEL_HALF;
    }
    if npc.life < npc.life_max / 4 {
        speed = PLANTERA_SPEED_QUARTER;
    }
    if enraged {
        speed += PLANTERA_ENRAGED_SPEED;
        accel = PLANTERA_ENRAGED_ACCEL;
    }
    if expert {
        speed = (speed + 1.0) * 1.1;
        accel = (accel + 0.01) * 1.1;
    }

    // The point it is pulled toward: out from its anchors, in the player's direction, up to the
    // leash. With no hooks yet it swings from itself.
    let anchor = state.hooks.unwrap_or_else(|| npc.center());
    let mut toward = (target.center.0 - anchor.0, target.center.1 - anchor.1);
    if dead {
        // With nobody alive it swings the other way, and faster: it is leaving.
        toward = (-toward.0, -toward.1);
        speed += 8.0;
    }
    let mut leash = PLANTERA_LEASH;
    if enraged {
        leash += PLANTERA_ENRAGED_LEASH;
    }
    if expert {
        leash += PLANTERA_LEASH_EXPERT;
    }
    let reach = toward.0.hypot(toward.1);
    if reach >= leash {
        let scale = leash / reach;
        toward = (toward.0 * scale, toward.1 * scale);
    }
    let goal = (anchor.0 + toward.0, anchor.1 + toward.1);

    let (cx, cy) = npc.center();
    let gap = (goal.0 - cx, goal.1 - cy);
    let distance = gap.0.hypot(gap.1);
    let wanted = if distance < speed {
        npc.velocity
    } else {
        (gap.0 / distance * speed, gap.1 / distance * speed)
    };
    // The push doubles while it is still moving the wrong way, which is what lets it reverse a
    // swing without drifting past you first.
    for (v, w) in [
        (&mut npc.velocity.0, wanted.0),
        (&mut npc.velocity.1, wanted.1),
    ] {
        if *v < w {
            *v += accel;
            if *v < 0.0 && w > 0.0 {
                *v += accel * 2.0;
            }
        } else if *v > w {
            *v -= accel;
            if *v > 0.0 && w < 0.0 {
                *v -= accel * 2.0;
            }
        }
    }
    npc.rotation = (target.center.1 - cy).atan2(target.center.0 - cx) + std::f32::consts::FRAC_PI_2;

    if health > 0.5 {
        // The first form: armoured, and it shoots.
        npc.defense = if enraged {
            PLANTERA_DEFENSE * 2
        } else {
            PLANTERA_DEFENSE
        };
        npc.damage_bonus = (if enraged {
            PLANTERA_DAMAGE * 2
        } else {
            PLANTERA_DAMAGE
        }) as f32
            / npc.stats.damage.max(1) as f32;

        // The charge fills faster at every threshold, and faster still out of the jungle.
        npc.local_ai[1] += 1.0;
        for step in [0.9, 0.8, 0.7, 0.6] {
            if health < step {
                npc.local_ai[1] += 1.0;
            }
        }
        if enraged {
            npc.local_ai[1] += 3.0;
        }
        if expert {
            npc.local_ai[1] += 1.0;
        }
        // In expert, being hit sometimes lets the next shot through a wall.
        if expert && world.was_hurt && rng.random_range(0..2) == 0 {
            npc.local_ai[3] = 1.0;
        }
        if npc.local_ai[1] <= PLANTERA_CHARGE {
            return out;
        }
        npc.local_ai[1] = 0.0;

        let mut allowed = can_see(world.tiles, npc, target);
        if npc.local_ai[3] > 0.0 {
            allowed = true;
            npc.local_ai[3] = 0.0;
        }
        if !allowed {
            return out;
        }

        let speed = if expert {
            PLANTERA_SEED_SPEED_EXPERT
        } else {
            PLANTERA_SEED_SPEED
        };
        let aim = unit((target.center.0 - cx, target.center.1 - cy), speed);
        // Below eighty per cent it mixes in the heavier shots, each of which costs it a pause —
        // so a thorn ball is worth more to you than the seeds it replaced.
        let (mut projectile, mut damage) = (PLANTERA_SEED, PLANTERA_SEED_DAMAGE);
        let (thorn_odds, spiky_odds) = if expert { (2, 6) } else { (4, 8) };
        if health < PLANTERA_MIX_AT && rng.random_range(0..thorn_odds) == 0 {
            projectile = PLANTERA_THORN_BALL;
            damage = PLANTERA_THORN_BALL_DAMAGE;
            npc.local_ai[1] = PLANTERA_THORN_BALL_REST;
        } else if health < PLANTERA_MIX_AT && rng.random_range(0..spiky_odds) == 0 {
            projectile = PLANTERA_SPIKY;
            damage = PLANTERA_SPIKY_DAMAGE;
            npc.local_ai[1] = PLANTERA_SPIKY_REST;
        }
        if enraged {
            damage *= 2;
        }
        out.shots.push(Shot {
            projectile,
            damage,
            position: (cx + aim.0 * 3.0, cy + aim.1 * 3.0),
            velocity: aim,
            time_left: if projectile == PLANTERA_SPIKY {
                900
            } else {
                300
            },
        });
        return out;
    }

    // The second form: less armour, far more damage, and tentacles.
    npc.defense = if enraged {
        PLANTERA_SECOND_DEFENSE * 4
    } else {
        PLANTERA_SECOND_DEFENSE
    };
    npc.damage_bonus = (if enraged {
        PLANTERA_SECOND_DAMAGE * 2
    } else {
        PLANTERA_SECOND_DAMAGE
    }) as f32
        / npc.stats.damage.max(1) as f32;

    if npc.local_ai[0] == 1.0 {
        npc.local_ai[0] = 2.0;
        let at = npc.center();
        for _ in 0..PLANTERA_TENTACLES {
            out.spawn.push(Spawn {
                npc_type: PLANTERA_TENTACLE,
                position: at,
                velocity: (0.0, 0.0),
                parent: Some(Spawn::OWN_PARENT),
            });
        }
    }
    out
}

/// Style 52: a hook.
///
/// It bites into a wall near the player, holds for several seconds, and moves. Where it can bite
/// changes as Plantera weakens: at full health it needs solid tile, and past half it will settle
/// for a background wall, which is what lets the fight leave a corridor.
pub fn hook(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    body: Option<Parent>,
    others_moving: bool,
    rng: &mut SmallRng,
) -> PlanteraOutcome {
    let mut out = PlanteraOutcome::default();
    npc.dirty = true;

    let Some(body) = body else {
        out.spent = true;
        return out;
    };
    let Some(target) = world.target else {
        return out;
    };
    let dead = !target.alive;
    // Out of the jungle every hook re-anchors far more often, which is most of why an enraged
    // Plantera crosses ground so fast.
    let enraged = world.conditions.jungle && target.center.1 > world.conditions.surface_y;
    let enraged = !enraged;

    // The timer runs down faster the weaker Plantera is.
    if npc.ai[0] == 0.0 || npc.ai[1] == 0.0 {
        npc.local_ai[0] = 0.0;
    }
    npc.local_ai[0] -= 1.0;
    if body.health < 0.5 {
        npc.local_ai[0] -= HOOK_HURRY_HALF;
    }
    if body.health < 0.25 {
        npc.local_ai[0] -= HOOK_HURRY_QUARTER;
    }
    if enraged {
        npc.local_ai[0] -= HOOK_HURRY_ENRAGED;
    }

    // Hooks take turns: one will not let go while another is still travelling.
    if !dead && npc.local_ai[0] <= 0.0 && npc.ai[0] != 0.0 && others_moving {
        npc.local_ai[0] = rng.random_range(HOOK_STAGGER.0..HOOK_STAGGER.1) as f32;
    }

    if npc.local_ai[0] <= 0.0 {
        npc.local_ai[0] = rng.random_range(HOOK_REST.0..HOOK_REST.1) as f32;
        // Around the player, or — for a hook that has never bitten — halfway between the player
        // and Plantera, so the first three fan out rather than all landing on you.
        let around = if npc.ai[0] == 0.0 {
            (
                (target.center.0 + body.center().0) / 2.0,
                (target.center.1 + body.center().1) / 2.0,
            )
        } else if dead {
            (body.position.0, body.position.1 + 400.0)
        } else {
            target.center
        };
        if let Some((tx, ty)) = bite(world, around, body.health < 0.5, rng) {
            npc.ai[0] = tx as f32;
            npc.ai[1] = ty as f32;
        }
    }

    if npc.ai[0] <= 0.0 || npc.ai[1] <= 0.0 {
        return out;
    }
    // Travel to the anchor it chose.
    let mut speed = HOOK_SPEED;
    if body.health < 0.5 {
        speed = HOOK_SPEED_HALF;
    }
    if body.health < 0.25 {
        speed = HOOK_SPEED_QUARTER;
    }
    let anchor = (npc.ai[0] * TILE + 8.0, npc.ai[1] * TILE + 8.0);
    let (cx, cy) = npc.center();
    let gap = (anchor.0 - cx, anchor.1 - cy);
    let distance = gap.0.hypot(gap.1);
    npc.velocity = if distance < speed {
        gap
    } else {
        (gap.0 / distance * speed, gap.1 / distance * speed)
    };
    npc.rotation = npc.velocity.1.atan2(npc.velocity.0) + std::f32::consts::FRAC_PI_2;
    out
}

/// Somewhere near `around` for a hook to bite into.
///
/// Solid tile always works. A background wall only works once Plantera is past half health, or
/// after five hundred failed attempts — which is what stops the fight stalling in an open cavern.
fn bite(
    world: &World<'_, impl TileView>,
    around: (f32, f32),
    wounded: bool,
    rng: &mut SmallRng,
) -> Option<(i32, i32)> {
    let (ax, ay) = ((around.0 / TILE) as i32, (around.1 / TILE) as i32);
    for attempt in 0..1000 {
        // The search widens the longer it goes unanswered.
        let spread = HOOK_SEARCH + (HOOK_SEARCH_WIDEN * (attempt as f32 / 1000.0)) as i32;
        let x = ax + rng.random_range(-spread..=spread);
        let y = ay + rng.random_range(-spread..=spread);
        let tile = world.tiles.tile(x, y);
        let solid = tile.is_active() && terrustia_proto::tile_solid::solid(tile.block);
        let walled = tile.wall > 0 && (attempt > 500 || wounded);
        if solid || walled {
            return Some((x, y));
        }
    }
    None
}

/// Style 53: a tentacle.
///
/// It orbits Plantera at a radius that *widens* as Plantera dies, so the second half of the fight
/// gets harder to approach rather than easier.
pub fn tentacle(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    body: Option<Parent>,
    rng: &mut SmallRng,
) -> PlanteraOutcome {
    let mut out = PlanteraOutcome::default();
    npc.dirty = true;

    let Some(body) = body else {
        out.spent = true;
        return out;
    };

    // A new offset within the orbit every couple of seconds.
    npc.local_ai[0] -= 1.0;
    if npc.local_ai[0] <= 0.0 {
        npc.local_ai[0] = rng.random_range(TENTACLE_DRIFT.0..TENTACLE_DRIFT.1) as f32;
        npc.ai[0] = rng.random_range(-TENTACLE_SPREAD..=TENTACLE_SPREAD) as f32;
        npc.ai[1] = rng.random_range(-TENTACLE_SPREAD..=TENTACLE_SPREAD) as f32;
    }

    let mut accel = TENTACLE_ACCEL;
    let mut radius = TENTACLE_RADIUS;
    if body.health < 0.25 {
        radius += TENTACLE_RADIUS_QUARTER;
    }
    if body.health < 0.1 {
        radius += TENTACLE_RADIUS_TENTH;
    }
    if world.conditions.expert {
        // In expert a wounded Plantera's tentacles reach much further out.
        let hurt = 1.0 - npc.life as f32 / npc.life_max.max(1) as f32;
        radius += hurt * TENTACLE_EXPERT_RADIUS;
        accel += TENTACLE_ACCEL_EXPERT;
    }

    let (bx, by) = body.center();
    // The offset it drifts around, normalised to the orbit's radius.
    let offset = (npc.ai[0], npc.ai[1]);
    let length = offset.0.hypot(offset.1).max(f32::MIN_POSITIVE);
    let station = (
        bx + offset.0 / length * radius,
        by + offset.1 / length * radius,
    );

    let (cx, cy) = npc.center();
    for (v, here, wanted) in [
        (&mut npc.velocity.0, cx, station.0),
        (&mut npc.velocity.1, cy, station.1),
    ] {
        if here > wanted {
            if *v > 0.0 {
                *v *= 0.9;
            }
            *v -= accel;
        } else if here < wanted {
            if *v < 0.0 {
                *v *= 0.9;
            }
            *v += accel;
        }
        *v = v.clamp(-TENTACLE_CAP, TENTACLE_CAP);
    }
    let (dx, dy) = (station.0 - cx, station.1 - cy);
    npc.rotation = dy.atan2(dx);
    npc.sprite_direction = if dx > 0.0 { 1 } else { -1 };
    if dx < 0.0 {
        npc.rotation += std::f32::consts::PI;
    }
    out
}

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
    use crate::game::ai::Conditions;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::npc_params::PLANTERA;
    use terrustia_proto::tile::Tile;

    struct Jungle(HashMap<(i32, i32), Tile>);

    impl TileView for Jungle {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    /// A cavern with mud all round it, so a hook always has somewhere to bite.
    fn cavern() -> Jungle {
        let mut tiles = HashMap::new();
        for x in -300..300 {
            for y in -300..300 {
                if !(-40..40).contains(&x) || !(-40..40).contains(&y) {
                    tiles.insert((x, y), Tile::block(59));
                }
            }
        }
        Jungle(tiles)
    }

    fn world<'a>(tiles: &'a Jungle, target: Option<(f32, f32)>) -> World<'a, Jungle> {
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
            jungle: true,
            surface_y: -10_000.0,
            ..Conditions::default()
        };
        w
    }

    fn home(hooks: Option<(f32, f32)>) -> PlanteraState {
        PlanteraState {
            hooks,
            at_home: true,
        }
    }

    fn body_at(position: (f32, f32), health: f32) -> Parent {
        Parent {
            position,
            size: (100.0, 100.0),
            rotation: 0.0,
            scale: 1.0,
            velocity: (0.0, 0.0),
            direction: 1,
            sprite_direction: 1,
            time_left: 3600,
            state: 0.0,
            health,
        }
    }

    fn plant(x: f32, y: f32) -> Npc {
        Npc::new(PLANTERA, (x, y), 1).expect("plantera")
    }

    /// It puts out three hooks before it does anything else.
    #[test]
    fn plantera_throws_three_hooks() {
        let tiles = cavern();
        let mut rng = SmallRng::seed_from_u64(51);
        let mut p = plant(0.0, 0.0);
        let w = world(&tiles, Some((200.0, 0.0)));
        let out = plantera(&mut p, &w, home(None), &mut rng);
        assert_eq!(out.spawn.len(), PLANTERA_HOOKS);
        assert!(out.spawn.iter().all(|s| s.npc_type == PLANTERA_HOOK));
        assert!(plantera(&mut p, &w, home(None), &mut rng).spawn.is_empty());
    }

    /// It swings from its hooks, not from itself: move the anchors and it follows.
    #[test]
    fn it_swings_from_wherever_its_hooks_are() {
        let tiles = cavern();
        let settle = |anchor: (f32, f32)| {
            let mut rng = SmallRng::seed_from_u64(2);
            let mut p = plant(0.0, 0.0);
            p.local_ai[0] = 1.0;
            let w = world(&tiles, Some((0.0, 0.0)));
            for _ in 0..1200 {
                plantera(&mut p, &w, home(Some(anchor)), &mut rng);
                p.position.0 += p.velocity.0;
                p.position.1 += p.velocity.1;
            }
            p.center()
        };
        let left = settle((-2000.0, 0.0));
        let right = settle((2000.0, 0.0));
        assert!(
            left.0 < right.0 - 1000.0,
            "the anchors should decide where it ends up: {left:?} vs {right:?}"
        );
    }

    /// The first half shoots and the second half grows tentacles.
    #[test]
    fn the_two_halves_fight_differently() {
        let tiles = cavern();
        let mut rng = SmallRng::seed_from_u64(3);
        let w = world(&tiles, Some((200.0, 0.0)));

        let mut fresh = plant(0.0, 0.0);
        fresh.local_ai[0] = 1.0;
        let mut shots = 0;
        for _ in 0..600 {
            shots += plantera(&mut fresh, &w, home(Some((0.0, 0.0))), &mut rng)
                .shots
                .len();
        }
        assert!(shots > 0, "the first form should shoot");
        assert_eq!(fresh.defense, PLANTERA_DEFENSE, "and be armoured");

        let mut hurt = plant(0.0, 0.0);
        hurt.local_ai[0] = 1.0;
        hurt.life = hurt.life_max / 4;
        let out = plantera(&mut hurt, &w, home(Some((0.0, 0.0))), &mut rng);
        assert_eq!(out.spawn.len(), PLANTERA_TENTACLES, "tentacles come out");
        assert_eq!(hurt.defense, PLANTERA_SECOND_DEFENSE, "armour drops");
        assert!(
            hurt.damage_bonus > 1.0,
            "and it hits far harder: {}",
            hurt.damage_bonus
        );
    }

    /// Dragged out of the jungle it is faster and hits twice as hard.
    #[test]
    fn leaving_the_jungle_enrages_it() {
        let tiles = cavern();
        let w = world(&tiles, Some((3000.0, 0.0)));
        let travel = |at_home: bool| {
            let mut rng = SmallRng::seed_from_u64(4);
            let mut p = plant(0.0, 0.0);
            p.local_ai[0] = 1.0;
            let state = PlanteraState {
                hooks: Some((0.0, 0.0)),
                at_home,
            };
            for _ in 0..300 {
                plantera(&mut p, &w, state, &mut rng);
                p.position.0 += p.velocity.0;
            }
            (p.position.0, p.defense, p.damage_bonus)
        };
        let (calm_x, calm_def, calm_dmg) = travel(true);
        let (angry_x, angry_def, angry_dmg) = travel(false);
        assert!(
            angry_x > calm_x,
            "it should cross ground faster: {angry_x} vs {calm_x}"
        );
        assert!(angry_def > calm_def, "and be tougher");
        assert!(angry_dmg > calm_dmg, "and hit harder");
    }

    /// A hook bites into terrain rather than hanging in the air.
    #[test]
    fn a_hook_bites_into_something_solid() {
        let tiles = cavern();
        let mut rng = SmallRng::seed_from_u64(5);
        let mut h = Npc::new(PLANTERA_HOOK, (0.0, 0.0), 1).expect("hook");
        let w = world(&tiles, Some((0.0, 0.0)));
        hook(&mut h, &w, Some(body_at((0.0, 0.0), 1.0)), false, &mut rng);
        assert!(
            h.ai[0] != 0.0 && h.ai[1] != 0.0,
            "it should have chosen an anchor"
        );
        let tile = tiles.tile(h.ai[0] as i32, h.ai[1] as i32);
        assert!(
            tile.is_active() && terrustia_proto::tile_solid::solid(tile.block),
            "and something solid to bite: {tile:?}"
        );
    }

    /// A hook or tentacle with no Plantera does not survive.
    #[test]
    fn the_parts_die_with_plantera() {
        let tiles = cavern();
        let mut rng = SmallRng::seed_from_u64(6);
        let w = world(&tiles, Some((0.0, 0.0)));
        let mut h = Npc::new(PLANTERA_HOOK, (0.0, 0.0), 1).unwrap();
        assert!(hook(&mut h, &w, None, false, &mut rng).spent);
        let mut t = Npc::new(PLANTERA_TENTACLE, (0.0, 0.0), 1).unwrap();
        assert!(tentacle(&mut t, &w, None, &mut rng).spent);
    }

    /// The tentacles' orbit widens as Plantera dies, so the fight closes in on you rather than
    /// opening up.
    #[test]
    fn the_tentacles_reach_further_as_plantera_dies() {
        let tiles = cavern();
        let w = world(&tiles, Some((0.0, 0.0)));
        let orbit = |health: f32| {
            let mut rng = SmallRng::seed_from_u64(7);
            let mut t = Npc::new(PLANTERA_TENTACLE, (0.0, 0.0), 1).unwrap();
            let body = body_at((0.0, 0.0), health);
            let mut furthest: f32 = 0.0;
            for _ in 0..1200 {
                tentacle(&mut t, &w, Some(body), &mut rng);
                t.position.0 += t.velocity.0;
                t.position.1 += t.velocity.1;
                let (cx, cy) = t.center();
                furthest = furthest.max((cx - 50.0).hypot(cy - 50.0));
            }
            furthest
        };
        let healthy = orbit(1.0);
        let dying = orbit(0.05);
        assert!(
            dying > healthy,
            "a dying Plantera's tentacles should reach further: {dying} vs {healthy}"
        );
    }
}
