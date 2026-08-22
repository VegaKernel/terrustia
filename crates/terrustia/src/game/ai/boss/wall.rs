//! Styles 27, 28 and 29 — the Wall of Flesh and everything hanging off it.
//!
//! The Wall is the one boss that does not chase: it *advances*. It walks in one direction along the
//! underworld at a fixed pace, faster the more it is hurt, and never turns around. The fight is
//! therefore a race, and everything else in it exists to make the race harder.
//!
//! Its **eyes** ride at a quarter and three-quarters of its height and fire lasers in volleys that
//! grow longer as the Wall weakens. Its **Hungry** hang off it on tethers and lunge at whoever comes
//! within reach, on leashes that lengthen at three-quarters and half health so the safe distance
//! keeps shrinking. And it spits **leeches** — worms that swim off after you — once it has been
//! alive long enough, sooner the more hurt it is.
//!
//! Dying is not how you escape it. If everyone in the underworld is dead it fades out over three
//! seconds and the fight is simply lost.

use rand::rngs::SmallRng;
use terrustia_proto::npc_params::{
    HUNGRY_ACCEL, HUNGRY_LEASH, HUNGRY_LEASH_DYING, HUNGRY_LEASH_WOUNDED, HUNGRY_RECOIL,
    HUNGRY_SPEED, WALL_EYE, WALL_EYE_CADENCE, WALL_EYE_CHARGE, WALL_EYE_VOLLEY, WALL_FADE_TICKS,
    WALL_HUNGRY, WALL_HUNGRY_COUNT, WALL_LASER, WALL_LASER_DAMAGE, WALL_LASER_SPEED, WALL_LEECH,
    WALL_LEECH_AFTER, WALL_LEECH_CAP, WALL_LEECH_EVERY, WALL_MIN_HEIGHT, WALL_SPEED,
    WALL_SPEED_BONUS, WALL_SPEED_RAGE, WALL_SPEED_SCALE,
};

use crate::game::ai::{Shot, World};
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Spawn;

/// What a tick of the Wall produced.
#[derive(Debug, Default)]
pub struct Advance {
    pub spawn: Vec<Spawn>,
    pub shots: Vec<Shot>,
    /// Set when the fight is over, one way or the other.
    pub gone: bool,
}

/// How many of its health thresholds it has passed, and what they add up to.
fn rage(npc: &Npc) -> f32 {
    let health = npc.life as f32 / npc.life_max as f32;
    WALL_SPEED_RAGE
        .iter()
        .filter(|(threshold, _)| health < *threshold)
        .map(|(_, extra)| extra)
        .sum()
}

/// Drive the Wall of Flesh for a tick.
///
/// `leeches` is how many of its worms are still swimming, which the caller counts.
pub fn wall<T: TileView>(
    npc: &mut Npc,
    world: &World<'_, T>,
    leeches: usize,
    rng: &mut SmallRng,
) -> Advance {
    let mut out = Advance::default();

    // The band it occupies. The game measures this against the terrain each tick; what matters for
    // the fight is that it is a tall slab centred on the Wall, at least ten tiles high.
    let top = npc.position.1 - WALL_MIN_HEIGHT / 2.0;
    let bottom = npc.position.1 + npc.height() + WALL_MIN_HEIGHT / 2.0;

    // First tick: two eyes and a row of Hungry come with it.
    if npc.local_ai[0] == 0.0 {
        npc.local_ai[0] = 1.0;
        for (side, at) in [
            (1.0, (npc.center().1 + top) / 2.0),
            (-1.0, (npc.center().1 + bottom) / 2.0),
        ] {
            out.spawn.push(Spawn {
                npc_type: WALL_EYE,
                position: (npc.position.0, at),
                // The side rides in the velocity, as it does for Skeletron's hands.
                velocity: (side, 0.0),
                parent: Some(Spawn::OWN_PARENT),
            });
        }
        for n in 0..WALL_HUNGRY_COUNT {
            out.spawn.push(Spawn {
                npc_type: WALL_HUNGRY,
                position: (npc.position.0, (npc.center().1 + bottom) / 2.0),
                // Its height along the wall, spread evenly.
                velocity: (n as f32 * 0.1 - 0.05, 0.0),
                parent: Some(Spawn::OWN_PARENT),
            });
        }
        npc.dirty = true;
    }

    // Leeches, once it has been alive long enough — and sooner the more it is hurt.
    npc.ai[1] += 1.0;
    if npc.ai[2] == 0.0 {
        let health = npc.life as f32 / npc.life_max as f32;
        if health < 0.5 {
            npc.ai[1] += 1.0;
        }
        if health < 0.2 {
            npc.ai[1] += 1.0;
        }
        if npc.ai[1] > WALL_LEECH_AFTER {
            npc.ai[2] = 1.0;
        }
    }
    if npc.ai[2] > 0.0 && npc.ai[1] > WALL_LEECH_EVERY {
        let volley = 3.0
            + if (npc.life as f32) < npc.life_max as f32 * 0.3 {
                1.0
            } else {
                0.0
            };
        npc.ai[2] += 1.0;
        npc.ai[1] = 0.0;
        if npc.ai[2] > volley {
            npc.ai[2] = 0.0;
        }
        if leeches < WALL_LEECH_CAP {
            out.spawn.push(Spawn {
                npc_type: WALL_LEECH,
                position: (
                    npc.position.0 + npc.width() / 2.0,
                    npc.position.1 + npc.height() / 2.0 + 20.0,
                ),
                velocity: (f32::from(npc.direction) * 8.0, 0.0),
                parent: None,
            });
        }
        npc.dirty = true;
    }

    // It walks, and only ever the way it is already going.
    let speed = (WALL_SPEED + rage(npc)) * WALL_SPEED_SCALE + WALL_SPEED_BONUS;
    if npc.velocity.0 == 0.0 {
        // Its opening direction is toward whoever woke it.
        if let Some(t) = world.target {
            npc.direction = if npc.center().0 < t.center.0 { 1 } else { -1 };
        }
        npc.velocity.0 = f32::from(npc.direction);
    }
    if npc.velocity.0 < 0.0 {
        npc.velocity.0 = -speed;
        npc.direction = -1;
    } else {
        npc.velocity.0 = speed;
        npc.direction = 1;
    }
    npc.velocity.1 = 0.0;
    npc.sprite_direction = npc.direction;

    // Nobody alive down here: it fades out over three seconds and the fight is lost.
    let everyone_dead = world.target.is_none_or(|t| !t.alive);
    if everyone_dead {
        npc.local_ai[1] += 1.0 / WALL_FADE_TICKS;
        if npc.local_ai[1] >= 1.0 {
            out.gone = true;
        }
    } else {
        npc.local_ai[1] = (npc.local_ai[1] - 1.0 / 30.0).max(0.0);
    }

    let _ = rng;
    npc.dirty = true;
    out
}

/// Drive one of the Wall's eyes for a tick.
///
/// `wall_at` is the Wall's position and size; `wall_health` is the fraction it has left, which is
/// what sets the eye's cadence.
pub fn eye<T: TileView>(
    npc: &mut Npc,
    world: &World<'_, T>,
    wall_at: Option<super::skeletron::Parent>,
    wall_health: f32,
) -> Option<Shot> {
    let (wall_position, wall_size) = wall_at?;
    // It rides the Wall's column, at the top or bottom of the band depending on its side.
    npc.position.0 = wall_position.0;
    npc.direction = if wall_position.0 < npc.position.0 {
        -1
    } else {
        npc.direction
    };
    npc.sprite_direction = npc.direction;

    let middle = wall_position.1 + wall_size.1 / 2.0;
    let want = if npc.ai[0] > 0.0 {
        middle - WALL_MIN_HEIGHT / 4.0
    } else {
        middle + WALL_MIN_HEIGHT / 4.0
    } - npc.height() / 2.0;
    if npc.position.1 > want + 1.0 {
        npc.velocity.1 = -1.0;
    } else if npc.position.1 < want - 1.0 {
        npc.velocity.1 = 1.0;
    } else {
        npc.velocity.1 = 0.0;
        npc.position.1 = want;
    }
    npc.velocity.1 = npc.velocity.1.clamp(-5.0, 5.0);

    // Its volley gets longer and its charge shorter as the Wall dies.
    let mut charge = 1.0;
    let mut volley = WALL_EYE_VOLLEY;
    for (threshold, extra_charge, extra_shots) in
        [(0.75, 1.0, 1), (0.5, 1.0, 1), (0.25, 1.0, 2), (0.1, 2.0, 3)]
    {
        if wall_health < threshold {
            charge += extra_charge;
            volley += extra_shots;
        }
    }
    npc.local_ai[1] += charge;

    let target = world.target?;
    // Only the eye facing the way the Wall is going can see anything to shoot at.
    let looking = (npc.direction > 0 && target.center.0 > npc.center().0)
        || (npc.direction < 0 && target.center.0 < npc.center().0);

    if npc.local_ai[2] == 0.0 {
        if npc.local_ai[1] > WALL_EYE_CHARGE {
            npc.local_ai[2] = 1.0;
            npc.local_ai[1] = 0.0;
            npc.dirty = true;
        }
        return None;
    }
    if npc.local_ai[1] <= WALL_EYE_CADENCE || !crate::game::ai::can_see(world.tiles, npc, target) {
        return None;
    }
    npc.local_ai[1] = 0.0;
    npc.local_ai[2] += 1.0;
    if npc.local_ai[2] >= volley as f32 {
        npc.local_ai[2] = 0.0;
    }
    if !looking {
        return None;
    }

    // The laser gets faster and harder as the Wall weakens, like everything else here.
    let mut speed = WALL_LASER_SPEED;
    let mut damage = WALL_LASER_DAMAGE;
    for (threshold, extra) in [(0.5, 1), (0.25, 1), (0.1, 2)] {
        if wall_health < threshold {
            damage += extra;
            speed += extra as f32;
        }
    }
    let muzzle = (
        npc.position.0 + npc.width() * 0.5,
        npc.position.1 + npc.height() * 0.5,
    );
    let (dx, dy) = (target.center.0 - muzzle.0, target.center.1 - muzzle.1);
    let k = speed / (dx * dx + dy * dy).sqrt().max(0.01);
    npc.dirty = true;
    Some(Shot {
        projectile: WALL_LASER,
        damage,
        position: (muzzle.0 + dx * k, muzzle.1 + dy * k),
        velocity: (dx * k, dy * k),
        time_left: 300,
    })
}

/// Drive one Hungry for a tick.
///
/// Each hangs at a fixed fraction `ai[0]` down the Wall and strays no further than its leash.
pub fn hungry<T: TileView>(
    npc: &mut Npc,
    world: &World<'_, T>,
    wall_at: Option<super::skeletron::Parent>,
    wall_health: f32,
) -> bool {
    let Some((wall_position, wall_size)) = wall_at else {
        return false;
    };
    // Being hit knocks it out of its lunge for a moment.
    if world.was_hurt {
        npc.ai[1] = HUNGRY_RECOIL;
    }

    // Its leash lengthens as the Wall dies, so the safe distance keeps shrinking.
    let mut leash = HUNGRY_LEASH;
    if wall_health < 0.5 {
        leash = HUNGRY_LEASH_DYING;
        npc.stats.defense = 30;
    } else if wall_health < 0.75 {
        leash = HUNGRY_LEASH_WOUNDED;
        npc.stats.defense = 20;
    }

    // Where its tether is anchored: the Wall's column, at its own height along it.
    let anchor = (
        wall_position.0 + wall_size.0 / 2.0,
        wall_position.1 - WALL_MIN_HEIGHT / 2.0 + (wall_size.1 + WALL_MIN_HEIGHT) * npc.ai[0],
    );

    // Every hundred ticks it strains a little further than it should.
    npc.ai[2] += 1.0;
    if npc.ai[2] > 100.0 {
        leash *= 1.3;
        if npc.ai[2] > 200.0 {
            npc.ai[2] = 0.0;
        }
    }

    let Some(target) = world.target else {
        return true;
    };
    let (mut dx, mut dy) = (
        target.center.0 - npc.width() / 2.0 - anchor.0,
        target.center.1 - npc.height() / 2.0 - anchor.1,
    );
    let reach = (dx * dx + dy * dy).sqrt();

    if npc.ai[1] == 0.0 {
        // Within the leash it goes for the player; beyond it, only as far as the leash allows.
        if reach > leash {
            let k = leash / reach.max(0.01);
            dx *= k;
            dy *= k;
        }
        // Accelerating toward that point, and much harder while still moving the wrong way.
        if npc.position.0 < anchor.0 + dx {
            npc.velocity.0 += HUNGRY_ACCEL;
            if npc.velocity.0 < 0.0 && dx > 0.0 {
                npc.velocity.0 += HUNGRY_ACCEL * 2.5;
            }
        } else if npc.position.0 > anchor.0 + dx {
            npc.velocity.0 -= HUNGRY_ACCEL;
            if npc.velocity.0 > 0.0 && dx < 0.0 {
                npc.velocity.0 -= HUNGRY_ACCEL * 2.5;
            }
        }
        if npc.position.1 < anchor.1 + dy {
            npc.velocity.1 += HUNGRY_ACCEL;
            if npc.velocity.1 < 0.0 && dy > 0.0 {
                npc.velocity.1 += HUNGRY_ACCEL * 2.5;
            }
        } else if npc.position.1 > anchor.1 + dy {
            npc.velocity.1 -= HUNGRY_ACCEL;
            if npc.velocity.1 > 0.0 && dy < 0.0 {
                npc.velocity.1 -= HUNGRY_ACCEL * 2.5;
            }
        }
        npc.velocity.0 = npc.velocity.0.clamp(-HUNGRY_SPEED, HUNGRY_SPEED);
        npc.velocity.1 = npc.velocity.1.clamp(-HUNGRY_SPEED, HUNGRY_SPEED);
    } else {
        npc.ai[1] -= 1.0;
    }

    npc.sprite_direction = if dx > 0.0 { 1 } else { -1 };
    npc.rotation = dy.atan2(dx) + if dx > 0.0 { 0.0 } else { std::f32::consts::PI };
    npc.dirty = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use terrustia_proto::tile::Tile;

    struct Hell;

    impl TileView for Hell {
        fn tile(&self, _x: i32, _y: i32) -> Tile {
            Tile::AIR
        }
    }

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(27)
    }

    fn hell<'a>(tiles: &'a Hell, target: Option<Target>) -> World<'a, Hell> {
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

    fn the_wall() -> Npc {
        Npc::new(113, (10_000.0, 20_000.0), 1).expect("wall of flesh")
    }

    #[test]
    fn it_arrives_with_its_eyes_and_its_hungry() {
        let tiles = Hell;
        let mut w = the_wall();
        let t = Some(player_at(11_000.0, 20_000.0));
        let out = wall(&mut w, &hell(&tiles, t), 0, &mut rng());
        assert_eq!(
            out.spawn.iter().filter(|s| s.npc_type == WALL_EYE).count(),
            2
        );
        assert_eq!(
            out.spawn
                .iter()
                .filter(|s| s.npc_type == WALL_HUNGRY)
                .count(),
            WALL_HUNGRY_COUNT
        );
        assert!(
            out.spawn
                .iter()
                .all(|s| s.parent == Some(Spawn::OWN_PARENT)),
            "and they all belong to it"
        );
    }

    /// The Wall never turns round. That is the whole fight.
    #[test]
    fn it_advances_in_one_direction_and_never_turns() {
        let tiles = Hell;
        let mut w = the_wall();
        let ahead = Some(player_at(11_000.0, 20_000.0));
        wall(&mut w, &hell(&tiles, ahead), 0, &mut rng());
        assert_eq!(w.direction, 1, "it sets off toward you");

        // Run behind it: it keeps going.
        let behind = Some(player_at(5_000.0, 20_000.0));
        for _ in 0..200 {
            wall(&mut w, &hell(&tiles, behind), 0, &mut rng());
        }
        assert_eq!(w.direction, 1, "and does not follow");
        assert!(w.velocity.0 > 0.0);
    }

    #[test]
    fn it_advances_faster_the_more_it_is_hurt() {
        let tiles = Hell;
        let pace = |life_fraction: f32| {
            let mut w = the_wall();
            w.life = (w.life_max as f32 * life_fraction) as i32;
            let t = Some(player_at(11_000.0, 20_000.0));
            wall(&mut w, &hell(&tiles, t), 0, &mut rng());
            w.velocity.0.abs()
        };
        assert!(pace(0.05) > pace(1.0) * 1.5, "the race gets harder");
    }

    #[test]
    fn it_spits_leeches_once_it_has_been_alive_long_enough() {
        let tiles = Hell;
        let mut w = the_wall();
        let t = Some(player_at(11_000.0, 20_000.0));
        let mut r = rng();
        let mut spat = 0;
        for _ in 0..500 {
            spat += wall(&mut w, &hell(&tiles, t), 0, &mut r)
                .spawn
                .iter()
                .filter(|s| s.npc_type == WALL_LEECH)
                .count();
        }
        assert_eq!(spat, 0, "not straight away");

        w.ai[1] = WALL_LEECH_AFTER;
        for _ in 0..500 {
            spat += wall(&mut w, &hell(&tiles, t), 0, &mut r)
                .spawn
                .iter()
                .filter(|s| s.npc_type == WALL_LEECH)
                .count();
        }
        assert!(spat > 0, "but eventually, yes");
    }

    #[test]
    fn it_stops_spitting_once_its_leeches_fill_the_room() {
        let tiles = Hell;
        let mut w = the_wall();
        w.ai[1] = WALL_LEECH_AFTER;
        w.ai[2] = 1.0;
        let t = Some(player_at(11_000.0, 20_000.0));
        let mut r = rng();
        for _ in 0..500 {
            let out = wall(&mut w, &hell(&tiles, t), WALL_LEECH_CAP, &mut r);
            assert!(out.spawn.iter().all(|s| s.npc_type != WALL_LEECH));
        }
    }

    #[test]
    fn everyone_dying_ends_the_fight() {
        let tiles = Hell;
        let mut w = the_wall();
        let dead = Some(Target {
            slot: 0,
            center: (11_000.0, 20_000.0),
            velocity: (0.0, 0.0),
            alive: false,
        });
        let mut r = rng();
        let mut over = false;
        for _ in 0..(WALL_FADE_TICKS as i32 + 5) {
            over |= wall(&mut w, &hell(&tiles, dead), 0, &mut r).gone;
        }
        assert!(over, "and the fight is simply lost");
    }

    fn an_eye(side: f32) -> Npc {
        let mut n = Npc::new(114, (10_000.0, 20_000.0), 1).expect("wall eye");
        n.ai[0] = side;
        n.direction = 1;
        n
    }

    #[test]
    fn an_eye_rides_the_wall() {
        let tiles = Hell;
        let mut e = an_eye(1.0);
        e.position.0 = 9_000.0;
        let at = Some(((10_000.0, 20_000.0), (16.0, 200.0)));
        let t = Some(player_at(11_000.0, 20_000.0));
        eye(&mut e, &hell(&tiles, t), at, 1.0);
        assert_eq!(e.position.0, 10_000.0, "it is carried, not steered");
    }

    #[test]
    fn the_two_eyes_ride_at_different_heights() {
        let tiles = Hell;
        let at = Some(((10_000.0, 20_000.0), (16.0, 200.0)));
        let t = Some(player_at(11_000.0, 20_000.0));
        let mut top = an_eye(1.0);
        let mut bottom = an_eye(-1.0);
        for _ in 0..400 {
            eye(&mut top, &hell(&tiles, t), at, 1.0);
            top.position.1 += top.velocity.1;
            eye(&mut bottom, &hell(&tiles, t), at, 1.0);
            bottom.position.1 += bottom.velocity.1;
        }
        assert!(
            top.position.1 < bottom.position.1,
            "one high, one low: {} against {}",
            top.position.1,
            bottom.position.1
        );
    }

    #[test]
    fn an_eye_charges_and_then_fires_a_volley() {
        let tiles = Hell;
        let mut e = an_eye(1.0);
        let at = Some(((10_000.0, 20_000.0), (16.0, 200.0)));
        let t = Some(player_at(11_000.0, 20_000.0));
        let mut fired = Vec::new();
        for _ in 0..2000 {
            if let Some(shot) = eye(&mut e, &hell(&tiles, t), at, 1.0) {
                fired.push(shot);
            }
        }
        assert!(!fired.is_empty(), "it should have fired");
        assert_eq!(fired[0].projectile, WALL_LASER);
        assert!(fired[0].velocity.0 > 0.0, "and toward the player");
    }

    #[test]
    fn a_dying_wall_makes_its_eyes_fire_harder() {
        let tiles = Hell;
        let at = Some(((10_000.0, 20_000.0), (16.0, 200.0)));
        let t = Some(player_at(11_000.0, 20_000.0));
        let volley = |health: f32| {
            let mut e = an_eye(1.0);
            let mut n = 0;
            let mut damage = 0;
            for _ in 0..3000 {
                if let Some(shot) = eye(&mut e, &hell(&tiles, t), at, health) {
                    n += 1;
                    damage = shot.damage;
                }
            }
            (n, damage)
        };
        let (healthy, weak_damage) = volley(1.0);
        let (dying, hard_damage) = volley(0.05);
        assert!(dying > healthy, "more shots: {dying} against {healthy}");
        assert!(hard_damage > weak_damage, "and harder ones");
    }

    fn a_hungry() -> Npc {
        let mut n = Npc::new(115, (10_000.0, 20_000.0), 1).expect("the hungry");
        n.ai[0] = 0.5;
        n
    }

    #[test]
    fn a_hungry_without_a_wall_is_finished() {
        let tiles = Hell;
        let mut h = a_hungry();
        assert!(!hungry(&mut h, &hell(&tiles, None), None, 1.0));
    }

    #[test]
    fn a_hungry_lunges_at_you_but_only_so_far() {
        let tiles = Hell;
        let mut h = a_hungry();
        let at = Some(((10_000.0, 20_000.0), (16.0, 200.0)));
        // Someone way beyond its leash.
        let t = Some(player_at(10_000.0 + HUNGRY_LEASH * 6.0, 20_000.0));
        for _ in 0..600 {
            hungry(&mut h, &hell(&tiles, t), at, 1.0);
            h.position.0 += h.velocity.0;
            h.position.1 += h.velocity.1;
        }
        let strayed = h.position.0 - 10_000.0;
        assert!(strayed > 0.0, "it should come at you");
        assert!(
            strayed < HUNGRY_LEASH * 2.0,
            "but stay on its tether, got {strayed}"
        );
    }

    #[test]
    fn its_leash_lengthens_as_the_wall_dies() {
        let tiles = Hell;
        let strays = |wall_health: f32| {
            let mut h = a_hungry();
            let at = Some(((10_000.0, 20_000.0), (16.0, 200.0)));
            let t = Some(player_at(10_000.0 + HUNGRY_LEASH * 6.0, 20_000.0));
            let mut furthest: f32 = 0.0;
            for _ in 0..900 {
                hungry(&mut h, &hell(&tiles, t), at, wall_health);
                h.position.0 += h.velocity.0;
                furthest = furthest.max(h.position.0 - 10_000.0);
            }
            furthest
        };
        assert!(
            strays(0.3) > strays(1.0),
            "the safe distance keeps shrinking"
        );
    }

    #[test]
    fn hitting_a_hungry_knocks_it_out_of_its_lunge() {
        let tiles = Hell;
        let mut h = a_hungry();
        let at = Some(((10_000.0, 20_000.0), (16.0, 200.0)));
        let t = Some(player_at(10_500.0, 20_000.0));
        let mut hit = hell(&tiles, t);
        hit.was_hurt = true;
        hungry(&mut h, &hit, at, 1.0);
        // Set on the hit and immediately begins counting down, so it reads one short.
        assert_eq!(h.ai[1], HUNGRY_RECOIL - 1.0);
        let held = h.velocity;
        hungry(&mut h, &hell(&tiles, t), at, 1.0);
        assert_eq!(h.velocity, held, "it coasts while it recovers");
    }
}
