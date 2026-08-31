//! Style 9 - the caster orbs.
//!
//! Burning Sphere, Chaos Ball, Water Sphere, the Vile Spits and the Solar Pillar's flare. Terraria
//! implements a caster's "projectile" as an NPC with one hit point and no gravity, which is why the
//! pre-hardmode casters need no projectile subsystem at all.
//!
//! Ported from `NPC.cs:21459-21600`, the `aiStyle == 9` block. Two creatures share it. Six types
//! aim **once** and then fly straight forever; type 516 is launched off at an angle and homes on
//! its target until it hits something, at which point it kills itself.
//!
//! Deliberate narrowings, both `Main.getGoodWorld`-only: the four boosted flight speeds at
//! `NPC.cs:21500-21514` (a Water Sphere with a live Wall Creeper, a Burning Sphere with a live
//! Blazing Wheel, a Vile Spit of the Eater) and the `dontTakeDamage` grants at `:21528-21542`.

use super::{World, sight::solid_collision};
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Target;
use rand::Rng;
use rand::rngs::SmallRng;

/// Solar Pillar flare (`NPCID.SolarFlare`). Its whole half of this style is its own.
const SOLAR_FLARE: u16 = 516;

/// Flight speed by type, from the `num125` selection at `NPC.cs:21491-21499`.
///
/// The default is 6; the Burning Sphere is slower and the Vile Spits are faster.
pub fn speed(npc_type: u16) -> f32 {
    match npc_type {
        25 => 5.0,        // BurningSphere
        112 | 666 => 7.0, // VileSpit, VileSpitEaterOfWorlds
        _ => 6.0,
    }
}

/// Whether this orb has already chosen its heading.
///
/// The game asks `target == 255` (`NPC.cs:21489`), which works there because `NPC.target` is
/// persistent state: `NewNPC` leaves it at 255 (`NPC.cs:81574`, `:81605`) and the orb's own
/// `TargetClosest()` is the first thing to write it. This server recomputes `npc.target` from the
/// player list every tick, before any routine runs (`npc_ai.rs`'s `update_with`), so by the time an
/// orb is asked it always has one and the game's test is exactly inverted. The flag lives in
/// `local_ai` instead, which is the slot the game itself reserves for state clients never see, and
/// which nothing else in this style touches.
fn has_aimed(npc: &Npc) -> bool {
    npc.local_ai[0] != 0.0
}

/// Drive one orb. Returns whether it just killed itself.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, rng: &mut SmallRng) -> bool {
    let mut died = false;

    if npc.npc_type == SOLAR_FLARE {
        // `NPC.cs:21462-21486`. A flare is spawned carrying its parent's target (`NPC.cs:40125`,
        // `:57425` pass it to `NewNPC`), so its `target` is never 255 and it never runs the
        // aiming branch below; the launch is what gives it a heading.
        npc.alpha = (npc.alpha + 40).min(220);
        if npc.ai[0] == 0.0 {
            npc.ai[0] = 1.0;
            launch(npc, world.target, rng);
        }
        // `StrikeNPCNoInteraction(9999, ...)`: it goes off on the first thing it touches.
        let close = world
            .target
            .is_some_and(|t| distance(npc.center(), t.center) < 20.0);
        died |= npc.collide_x || npc.collide_y || close;
    } else if !has_aimed(npc) {
        // `NPC.cs:21489-21526`.
        npc.local_ai[0] = 1.0;
        if let Some(t) = world.target {
            let (cx, cy) = npc.center();
            let (dx, dy) = (t.center.0 - cx, t.center.1 - cy);
            // The game guards a zero distance by substituting 1 rather than skipping the shot.
            let d = (dx * dx + dy * dy).sqrt();
            let scale = speed(npc.npc_type) / if d <= 0.0 { 1.0 } else { d };
            npc.velocity = (dx * scale, dy * scale);
            npc.dirty = true;
        } else {
            // Nobody to aim at. The game would aim at the dummy player in slot 255; hold the
            // heading instead and take the shot on a later tick, once someone is in the world.
            npc.local_ai[0] = 0.0;
        }
    }

    if matches!(npc.npc_type, 112 | 666) {
        // `NPC.cs:21551-21573`. `ai[0]` is a spawn-frame counter that stops at 3; on frame 2 the
        // spit is nudged a whole step forward, which is what clears it of the mouth that spat it.
        npc.ai[0] = (npc.ai[0] + 1.0).min(3.0);
        if npc.ai[0] == 2.0 {
            npc.position.0 += npc.velocity.0;
            npc.position.1 += npc.velocity.1;
            npc.dirty = true;
        }
        // `NPC.cs:21575-21579`: a spit that ends up inside terrain bursts. Unlike the other five
        // it is the one orb the world can stop.
        if solid_collision(
            world.tiles,
            npc.position,
            (npc.width() as i32, npc.height() as i32),
        ) {
            died = true;
        }
    }

    // `EncourageDespawn(100)` (`NPC.cs:21581`): an orb has under two seconds of life once nobody
    // is near it, rather than an ordinary enemy's twelve and a half.
    npc.time_left = npc.time_left.min(100);

    if npc.npc_type == SOLAR_FLARE {
        home(npc, world.target);
    }

    died
}

/// The flare's one-off launch, from `NPC.cs:21469-21485`.
///
/// It leaves at a random angle off the line to its target, never steeply downward, at somewhere
/// between 6 and 10 pixels a tick. The direction vector is deliberately not renormalised after the
/// downward clamp; the game does not renormalise it either, so a shot aimed low is also slower.
fn launch(npc: &mut Npc, target: Option<Target>, rng: &mut SmallRng) {
    let (cx, cy) = npc.center();
    let mut dir = match target.map(|t| (t.center.0 - cx, t.center.1 - cy)) {
        Some((dx, dy)) if dx * dx + dy * dy > 0.0 => {
            let d = (dx * dx + dy * dy).sqrt();
            (dx / d, dy / d)
        }
        // `vector15.HasNaNs()` - straight up.
        _ => (0.0, -1.0),
    };
    // `RotatedByRandom(PI/2)` is `RotatedBy(rand() * PI/2 - rand() * PI/2)` (`Utils.cs:1785-1788`),
    // and the game then rotates the result by another -PI/4.
    let half_pi = std::f32::consts::FRAC_PI_2;
    let angle = rng.random::<f32>() * half_pi - rng.random::<f32>() * half_pi
        + -std::f32::consts::FRAC_PI_4;
    let (s, c) = angle.sin_cos();
    dir = (dir.0 * c - dir.1 * s, dir.0 * s + dir.1 * c);
    dir.1 = dir.1.min(0.2);
    let launch_speed = 6.0 + rng.random::<f32>() * 4.0;
    npc.velocity = (dir.0 * launch_speed, dir.1 * launch_speed);
    npc.dirty = true;
}

/// The flare's homing, from `NPC.cs:21583-21599`.
///
/// A fifteen-tick blend towards the line to its target, gaining a twelfth of a pixel a tick, plus a
/// 5% boost whenever it is below 6. It never stops turning, so the only way out is to hit
/// something.
fn home(npc: &mut Npc, target: Option<Target>) {
    npc.rotation += 0.1 * f32::from(npc.direction);
    let Some(t) = target else {
        return;
    };
    let (cx, cy) = npc.center();
    let (dx, dy) = (t.center.0 - cx, t.center.1 - cy);
    let d = (dx * dx + dy * dy).sqrt();
    // `vector16.HasNaNs()` falls back to the way it is facing.
    let dir = if d > 0.0 {
        (dx / d, dy / d)
    } else {
        (f32::from(npc.direction), 0.0)
    };
    let speed = (npc.velocity.0 * npc.velocity.0 + npc.velocity.1 * npc.velocity.1).sqrt();
    let reach = speed + 1.0 / 12.0;
    npc.velocity = (
        (npc.velocity.0 * 14.0 + dir.0 * reach) / 15.0,
        (npc.velocity.1 * 14.0 + dir.1 * reach) / 15.0,
    );
    if (npc.velocity.0 * npc.velocity.0 + npc.velocity.1 * npc.velocity.1).sqrt() < 6.0 {
        npc.velocity = (npc.velocity.0 * 1.05, npc.velocity.1 * 1.05);
    }
    npc.dirty = true;
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ai::calm;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Cave(HashMap<(i32, i32), Tile>);

    impl TileView for Cave {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(7)
    }

    /// A freshly spawned orb, exactly as the pipeline presents one: `npc_ai::update_with` has
    /// already written `npc.target` from the player list before any routine sees it.
    fn orb(npc_type: u16, at: (f32, f32)) -> Npc {
        let mut npc = Npc::new(npc_type, at, 1).expect("orb type");
        npc.target = 2;
        npc
    }

    fn player(x: f32, y: f32) -> Target {
        Target {
            slot: 2,
            center: (x, y),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    #[test]
    fn speeds_match_the_types_the_game_singles_out() {
        assert_eq!(speed(25), 5.0, "BurningSphere");
        assert_eq!(speed(112), 7.0, "VileSpit");
        assert_eq!(speed(666), 7.0, "VileSpitEaterOfWorlds");
        assert_eq!(speed(30), 6.0, "ChaosBall takes the default");
        assert_eq!(speed(33), 6.0, "WaterSphere takes the default");
    }

    /// The regression that mattered: the server's `npc.target` is not the game's, so gating the
    /// aim on it meant an orb only ever aimed when there was nothing to aim at, and every
    /// pre-hardmode caster's one attack hung motionless at its muzzle.
    #[test]
    fn an_orb_the_pipeline_has_already_given_a_target_still_aims() {
        let tiles = Cave::default();
        let mut o = orb(30, (0.0, 0.0));
        let (cx, cy) = o.center();
        assert_ne!(
            o.target, 255,
            "the pipeline sets this before the routine runs"
        );

        update(
            &mut o,
            &calm(&tiles, Some(player(cx + 100.0, cy))),
            &mut rng(),
        );
        assert!(
            (o.velocity.0 - 6.0).abs() < 0.001,
            "should fly at its type speed, got {}",
            o.velocity.0
        );
        assert_eq!(o.velocity.1, 0.0, "no vertical component for a level shot");
    }

    #[test]
    fn the_heading_is_chosen_once_and_never_revised() {
        let tiles = Cave::default();
        let mut o = orb(30, (0.0, 0.0));
        let (cx, cy) = o.center();
        update(
            &mut o,
            &calm(&tiles, Some(player(cx + 100.0, cy))),
            &mut rng(),
        );
        let first = o.velocity;

        // The player runs the other way; the orb must not follow.
        update(
            &mut o,
            &calm(&tiles, Some(player(cx - 500.0, cy + 300.0))),
            &mut rng(),
        );
        assert_eq!(o.velocity, first, "an orb does not steer after it is fired");
    }

    #[test]
    fn a_diagonal_shot_keeps_its_total_speed() {
        let tiles = Cave::default();
        let mut o = orb(25, (0.0, 0.0));
        let (cx, cy) = o.center();
        update(
            &mut o,
            &calm(&tiles, Some(player(cx + 300.0, cy + 400.0))),
            &mut rng(),
        );
        let magnitude = (o.velocity.0.powi(2) + o.velocity.1.powi(2)).sqrt();
        assert!(
            (magnitude - 5.0).abs() < 0.001,
            "speed should be 5 regardless of direction, got {magnitude}"
        );
    }

    #[test]
    fn a_target_on_top_of_the_orb_does_not_divide_by_zero() {
        let tiles = Cave::default();
        let mut o = orb(30, (0.0, 0.0));
        let here = o.center();
        update(
            &mut o,
            &calm(&tiles, Some(player(here.0, here.1))),
            &mut rng(),
        );
        assert!(o.velocity.0.is_finite() && o.velocity.1.is_finite());
    }

    #[test]
    fn an_orb_with_nobody_to_aim_at_holds_still_and_stays_unaimed() {
        let tiles = Cave::default();
        let mut o = orb(30, (0.0, 0.0));
        update(&mut o, &calm(&tiles, None), &mut rng());
        assert_eq!(o.velocity, (0.0, 0.0));
        assert!(!has_aimed(&o), "still unaimed, so it will aim later");

        let (cx, cy) = o.center();
        update(
            &mut o,
            &calm(&tiles, Some(player(cx + 100.0, cy))),
            &mut rng(),
        );
        assert!(o.velocity.0 > 0.0, "and it does");
    }

    /// `EncourageDespawn(100)`, `NPC.cs:21581`.
    #[test]
    fn an_orb_is_reaped_far_sooner_than_an_ordinary_enemy() {
        let tiles = Cave::default();
        let mut o = orb(30, (0.0, 0.0));
        assert_eq!(o.time_left, crate::game::npc::DEFAULT_TIME_LEFT);
        update(&mut o, &calm(&tiles, None), &mut rng());
        assert_eq!(o.time_left, 100);
    }

    /// `NPC.cs:21575-21579`. Only the two spits check terrain; the other four are `noTileCollide`
    /// and pass straight through it.
    #[test]
    fn a_vile_spit_bursts_inside_a_wall_and_a_chaos_ball_does_not() {
        let mut tiles = Cave::default();
        for x in 0..8 {
            for y in 0..8 {
                tiles.0.insert((x, y), Tile::block(1));
            }
        }
        let mut spit = orb(112, (16.0, 16.0));
        assert!(update(&mut spit, &calm(&tiles, None), &mut rng()));

        let mut ball = orb(30, (16.0, 16.0));
        assert!(!update(&mut ball, &calm(&tiles, None), &mut rng()));
    }

    /// `NPC.cs:21551-21566`: the counter stops at 3 and the nudge happens exactly once.
    #[test]
    fn a_spit_is_nudged_clear_of_the_mouth_on_its_second_frame() {
        let tiles = Cave::default();
        let mut spit = orb(112, (0.0, 0.0));
        let (cx, cy) = spit.center();
        update(
            &mut spit,
            &calm(&tiles, Some(player(cx + 700.0, cy))),
            &mut rng(),
        );
        assert_eq!(spit.ai[0], 1.0);
        let before = spit.position;

        update(
            &mut spit,
            &calm(&tiles, Some(player(cx + 700.0, cy))),
            &mut rng(),
        );
        assert_eq!(spit.ai[0], 2.0);
        assert!(
            (spit.position.0 - before.0 - 7.0).abs() < 0.001,
            "one step ahead"
        );

        let after = spit.position;
        for _ in 0..5 {
            update(
                &mut spit,
                &calm(&tiles, Some(player(cx + 700.0, cy))),
                &mut rng(),
            );
        }
        assert_eq!(spit.ai[0], 3.0, "and stops there");
        assert_eq!(spit.position, after, "nudged once, never again");
    }

    /// `NPC.cs:21462-21486`: a flare leaves at a launch speed of 6 to 10, off the line to its
    /// target, and never steeply downward.
    #[test]
    fn a_solar_flare_launches_once_at_an_angle() {
        let tiles = Cave::default();
        let mut r = rng();
        for _ in 0..64 {
            let mut f = orb(SOLAR_FLARE, (5000.0, 5000.0));
            let (cx, cy) = f.center();
            update(&mut f, &calm(&tiles, Some(player(cx + 600.0, cy))), &mut r);
            assert_eq!(f.ai[0], 1.0, "launched");
            let launched = f.velocity;
            let speed = (launched.0.powi(2) + launched.1.powi(2)).sqrt();
            assert!(speed > 0.0 && speed <= 10.001, "launch speed {speed}");
            // The Y clamp is on the unit vector, so the launched Y can be at most 0.2 * 10.
            assert!(
                launched.1 <= 2.001,
                "never fired steeply downward: {launched:?}"
            );
        }
    }

    /// `NPC.cs:21583-21599`. Unlike every other orb, a flare keeps turning towards its target.
    #[test]
    fn a_solar_flare_homes_and_kills_itself_on_arrival() {
        let tiles = Cave::default();
        let mut f = orb(SOLAR_FLARE, (5000.0, 5000.0));
        let (cx, cy) = f.center();
        let mark = player(cx + 600.0, cy);
        update(&mut f, &calm(&tiles, Some(mark)), &mut rng());

        // Chase it for long enough that the blend has to have won.
        let mut closing = 0.0f32;
        for _ in 0..300 {
            update(&mut f, &calm(&tiles, Some(mark)), &mut rng());
            f.position.0 += f.velocity.0;
            f.position.1 += f.velocity.1;
            closing = distance(f.center(), mark.center);
            if closing < 20.0 {
                break;
            }
        }
        assert!(closing < 20.0, "should have homed in, still {closing} away");
        assert!(
            update(&mut f, &calm(&tiles, Some(mark)), &mut rng()),
            "and goes off when it arrives"
        );
    }

    #[test]
    fn a_flare_goes_off_on_terrain_too() {
        let tiles = Cave::default();
        let mut f = orb(SOLAR_FLARE, (5000.0, 5000.0));
        f.ai[0] = 1.0; // already launched
        f.collide_y = true;
        assert!(update(&mut f, &calm(&tiles, None), &mut rng()));
    }
}
