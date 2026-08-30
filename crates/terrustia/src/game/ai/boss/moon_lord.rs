//! The Moon Lord: styles 77–79, 81 and 82.
//!
//! The core is not the fight. It hangs a hundred and thirty pixels below you, cannot be hurt at
//! all, and waits — its two hands and its head are the fight, and only once all three are open does
//! the core become something you can attack.
//!
//! Each eye runs one of three attack scripts, fixed when it opens: five entries of "do this for
//! that long", so the three are always doing different things at once and the fight has a shape
//! rather than a rhythm. Attack nought is a pause, one is a stream of bolts, two is the heavy
//! attack — a sweeping deathray from the head, a phantasmal sphere from a hand — and three is a
//! spread of spheres.
//!
//! Breaking a socket does not remove it from the fight: the eye comes out and hunts you as a free
//! eye. And the head puts out leeches that carry life back to whichever part is most hurt, so
//! ignoring them undoes work you have already done.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    FREE_EYE_ACCEL, FREE_EYE_SPEED, LEECH_HEAL, LEECH_TICKS, MOON_LORD_ACCEL, MOON_LORD_BELOW,
    MOON_LORD_BOLT_EVERY, MOON_LORD_BOLT_SPEED, MOON_LORD_DEATH_TICKS, MOON_LORD_FIGHTING_DISTANCE,
    MOON_LORD_HAND, MOON_LORD_HAND_OUT, MOON_LORD_HAND_UP, MOON_LORD_HEAD, MOON_LORD_HEAD_UP,
    MOON_LORD_LEECH, MOON_LORD_OPENING, MOON_LORD_RAY_SWEEP, MOON_LORD_SCRIPTS, MOON_LORD_SPEED,
    PHANTASMAL_BOLT, PHANTASMAL_BOLT_DAMAGE, PHANTASMAL_DEATHRAY, PHANTASMAL_DEATHRAY_DAMAGE,
    PHANTASMAL_EYE, PHANTASMAL_EYE_DAMAGE, PHANTASMAL_SPHERE, PHANTASMAL_SPHERE_DAMAGE,
};

use super::skeletron::Parent;
use crate::game::ai::{Shot, World};
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Spawn;

/// The states shared by the core and its parts, as `ai[0]` numbers them.
mod state {
    /// Opening, and untouchable while it does.
    pub const OPENING: f32 = -1.0;
    /// Broken open: this eye is finished and its socket is empty.
    pub const BROKEN: f32 = -2.0;
    /// Waiting for the rest of the assembly.
    pub const WAITING: f32 = 0.0;
    /// The fight proper.
    pub const FIGHTING: f32 = 1.0;
    /// The death drama.
    pub const DYING: f32 = 2.0;
}

/// What a piece of it did this tick.
#[derive(Debug, Default)]
pub struct MoonLordOutcome {
    pub shots: Vec<Shot>,
    pub spawn: Vec<Spawn>,
    pub spent: bool,
    /// How much life this leech is carrying back, on the tick it arrives.
    pub healed: i32,
}

/// Style 77: the core.
///
/// `parts_open` is how many of its three eyes have been broken.
pub fn core(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    parts: usize,
    parts_open: usize,
) -> MoonLordOutcome {
    let mut out = MoonLordOutcome::default();
    npc.dirty = true;

    if npc.local_ai[3] == 0.0 {
        npc.local_ai[3] = 1.0;
        npc.ai[0] = state::OPENING;
    }

    if npc.ai[0] == state::OPENING {
        // Opening. It makes its two hands and its head and cannot be touched meanwhile.
        npc.invulnerable = true;
        npc.ai[1] += 1.0;
        if npc.ai[1] >= MOON_LORD_OPENING {
            npc.ai[1] = 0.0;
            npc.ai[0] = state::WAITING;
            let (cx, cy) = npc.center();
            for side in 0..2 {
                out.spawn.push(Spawn {
                    npc_type: MOON_LORD_HAND,
                    position: (
                        cx + side as f32 * MOON_LORD_HAND_OUT * 2.0 - MOON_LORD_HAND_OUT,
                        cy - MOON_LORD_HAND_UP,
                    ),
                    velocity: (0.0, 0.0),
                    parent: Some(Spawn::OWN_PARENT),
                    // Which hand this is, seated left (0) or right (1) by ai[2] (`NPC.cs:41649`,
                    // `Main.npc[num2].ai[2] = i`). Left unset it would default to 0 for both and
                    // seat both hands on the same side.
                    ai: [None, None, Some(side as f32), None],
                });
            }
            out.spawn.push(Spawn {
                npc_type: MOON_LORD_HEAD,
                position: (cx, cy - MOON_LORD_HEAD_UP),
                velocity: (0.0, 0.0),
                parent: Some(Spawn::OWN_PARENT),
                ai: [None; 4],
            });
        }
        return out;
    }

    if npc.ai[0] == state::DYING {
        // The death drama: it drifts upward and comes apart over ten seconds.
        npc.invulnerable = true;
        npc.velocity.0 += (0.0 - npc.velocity.0) * 0.02;
        npc.velocity.1 += (-0.5 - npc.velocity.1) * 0.02;
        npc.ai[1] += 1.0;
        if npc.ai[1] >= MOON_LORD_DEATH_TICKS {
            out.spent = true;
        }
        return out;
    }

    // Its parts are gone entirely: nothing is holding it together.
    if parts == 0 {
        out.spent = true;
        return out;
    }

    let Some(target) = world.target.filter(|t| t.alive) else {
        return out;
    };
    let (cx, cy) = npc.center();
    if (target.center.0 - cx).abs() > MOON_LORD_FIGHTING_DISTANCE {
        npc.time_left = npc.time_left.min(600);
    }

    // Every eye broken: the core is open at last.
    npc.invulnerable = parts_open < 3;
    if !npc.invulnerable {
        npc.ai[0] = state::FIGHTING;
    }

    // It follows below the player, gently, with the game's odd half-and-half smoothing.
    let gap = (target.center.0 - cx, target.center.1 + MOON_LORD_BELOW - cy);
    if gap.0.hypot(gap.1) > 20.0 {
        let before = npc.velocity;
        let aim = (gap.0 - npc.velocity.0, gap.1 - npc.velocity.1);
        let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
        let wanted = (
            aim.0 / length * MOON_LORD_SPEED,
            aim.1 / length * MOON_LORD_SPEED,
        );
        super::super::hardmode::drifters::simple_fly(npc, wanted, MOON_LORD_ACCEL);
        // ...and then half of that is given back, which is what makes it drift rather than track.
        npc.velocity.0 = (npc.velocity.0 + before.0) / 2.0;
        npc.velocity.1 = (npc.velocity.1 + before.1) / 2.0;
    }
    out
}

/// Styles 78 and 79: a hand or the head.
///
/// `script` is which of the three it was given when it opened.
pub fn eye_socket(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    core: Option<Parent>,
    rng: &mut SmallRng,
) -> MoonLordOutcome {
    let mut out = MoonLordOutcome::default();
    npc.dirty = true;
    let head = npc.npc_type == MOON_LORD_HEAD;

    let Some(core) = core else {
        out.spent = true;
        return out;
    };
    // It rides its station on the core.
    let (bx, by) = core.center();
    let station = if head {
        (bx, by - MOON_LORD_HEAD_UP)
    } else {
        // `ai[2]` is which hand this is.
        let side = if npc.ai[2] >= 1.0 { 1.0 } else { -1.0 };
        (bx + side * MOON_LORD_HAND_OUT, by - MOON_LORD_HAND_UP)
    };
    let (cx, cy) = npc.center();
    npc.velocity = ((station.0 - cx) * 0.2, (station.1 - cy) * 0.2);

    // Broken: the socket is empty and it does nothing but hang there.
    if npc.ai[0] == state::BROKEN {
        npc.invulnerable = true;
        if core.state == state::DYING {
            out.spent = true;
        }
        return out;
    }

    // Its script is fixed when it opens.
    if npc.local_ai[3] == 0.0 {
        npc.local_ai[3] = 1.0;
        npc.ai[3] = rng.random_range(0..MOON_LORD_SCRIPTS.len()) as f32;
        npc.ai[0] = state::WAITING;
    }

    let Some(target) = world.target.filter(|t| t.alive) else {
        return out;
    };

    // `local_ai[0]` is how far through its script it is; `ai[1]` how long the current entry has run.
    let script = MOON_LORD_SCRIPTS[(npc.ai[3] as usize).min(MOON_LORD_SCRIPTS.len() - 1)];
    let step = (npc.local_ai[0] as usize) % script.len();
    let (attack, ticks) = script[step];

    npc.ai[1] += 1.0;
    match attack {
        1 => {
            // The bolts: a stream of them, aimed as they leave.
            if npc.ai[1] % MOON_LORD_BOLT_EVERY == 0.0 {
                out.shots.push(aimed(
                    npc,
                    target.center,
                    PHANTASMAL_BOLT,
                    PHANTASMAL_BOLT_DAMAGE,
                    MOON_LORD_BOLT_SPEED,
                ));
            }
        }
        2 if head => {
            // The deathray, which sweeps rather than tracks: it is fired once and turns.
            if npc.ai[1] as i32 == 1 {
                out.shots.push(Shot {
                    projectile: PHANTASMAL_DEATHRAY,
                    damage: PHANTASMAL_DEATHRAY_DAMAGE,
                    position: npc.center(),
                    velocity: (0.0, -1.0),
                    time_left: MOON_LORD_RAY_SWEEP as u16,
                });
            }
            // ...and it puts out leeches while it fires.
            if npc.ai[1] % 60.0 == 0.0 {
                out.spawn.push(Spawn {
                    npc_type: MOON_LORD_LEECH,
                    position: npc.center(),
                    velocity: (0.0, 0.0),
                    parent: Some(Spawn::OWN_PARENT),
                    ai: [None; 4],
                });
            }
        }
        2 => {
            // A hand's heavy attack is a single phantasmal eye, thrown once.
            if npc.ai[1] as i32 == 1 {
                out.shots.push(aimed(
                    npc,
                    target.center,
                    PHANTASMAL_EYE,
                    PHANTASMAL_EYE_DAMAGE,
                    6.0,
                ));
            }
        }
        3 if npc.ai[1] as i32 == 1 => {
            // The spread: spheres thrown outward on an even fan.
            {
                for i in 0..3 {
                    let angle = (i as f32 - 1.0) * 0.4;
                    let aim = (target.center.0 - cx, target.center.1 - cy);
                    let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
                    let (sin, cos) = angle.sin_cos();
                    let unit = (aim.0 / length, aim.1 / length);
                    out.shots.push(Shot {
                        projectile: PHANTASMAL_SPHERE,
                        damage: PHANTASMAL_SPHERE_DAMAGE,
                        position: (cx, cy),
                        velocity: (
                            (unit.0 * cos - unit.1 * sin) * 5.0,
                            (unit.0 * sin + unit.1 * cos) * 5.0,
                        ),
                        time_left: 900,
                    });
                }
            }
        }
        // Nought is a pause.
        _ => {}
    }

    if npc.ai[1] >= ticks as f32 {
        npc.ai[1] = 0.0;
        npc.local_ai[0] += 1.0;
    }
    out
}

/// Style 81: an eye that has come out of its broken socket.
pub fn free_eye(npc: &mut Npc, world: &World<'_, impl TileView>) -> MoonLordOutcome {
    let out = MoonLordOutcome::default();
    npc.dirty = true;
    npc.no_gravity = true;
    npc.no_tile_collide = true;

    let Some(target) = world.target.filter(|t| t.alive) else {
        return out;
    };
    let (cx, cy) = npc.center();
    let aim = (target.center.0 - cx, target.center.1 - cy);
    let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
    let wanted = (
        aim.0 / length * FREE_EYE_SPEED,
        aim.1 / length * FREE_EYE_SPEED,
    );
    super::super::hardmode::drifters::simple_fly(npc, wanted, FREE_EYE_ACCEL);
    npc.rotation = npc.velocity.1.atan2(npc.velocity.0) - std::f32::consts::FRAC_PI_2;
    out
}

/// Style 82: a leech clot, carrying life back to the Moon Lord.
///
/// It travels for a second and a half and then delivers. Left alone it undoes damage already done,
/// which is why they are worth stopping even though they never attack.
pub fn leech(npc: &mut Npc, anchor: Option<Parent>) -> MoonLordOutcome {
    let mut out = MoonLordOutcome::default();
    npc.dirty = true;
    npc.no_gravity = true;
    npc.no_tile_collide = true;

    let Some(anchor) = anchor else {
        out.spent = true;
        return out;
    };
    npc.ai[2] += 1.0;
    if npc.ai[2] >= LEECH_TICKS {
        out.spent = true;
        out.healed = LEECH_HEAL;
        return out;
    }
    // It drifts from where it was made toward its anchor, arriving as the timer runs out.
    let along = npc.ai[2] / LEECH_TICKS;
    let (ax, ay) = anchor.center();
    let (cx, cy) = npc.center();
    npc.velocity = ((ax - cx) * along * 0.2, (ay + 216.0 - cy) * along * 0.2);
    out
}

/// A shot aimed at the player.
fn aimed(npc: &Npc, player: (f32, f32), projectile: u16, damage: i32, speed: f32) -> Shot {
    let (cx, cy) = npc.center();
    let aim = (player.0 - cx, player.1 - cy);
    let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
    Shot {
        projectile,
        damage,
        position: (cx, cy),
        velocity: (aim.0 / length * speed, aim.1 / length * speed),
        time_left: 600,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::npc_params::{MOON_LORD_CORE, MOON_LORD_FREE_EYE};
    use terrustia_proto::tile::Tile;

    struct Sky(HashMap<(i32, i32), Tile>);

    impl TileView for Sky {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn world<'a>(tiles: &'a Sky, target: Option<(f32, f32)>) -> World<'a, Sky> {
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

    fn core_at(position: (f32, f32), state: f32) -> Parent {
        Parent {
            position,
            size: (200.0, 200.0),
            rotation: 0.0,
            scale: 1.0,
            velocity: (0.0, 0.0),
            direction: 1,
            sprite_direction: 1,
            time_left: 3600,
            state,
            health: 1.0,
        }
    }

    fn piece(npc_type: u16) -> Npc {
        Npc::new(npc_type, (0.0, 0.0), 1).expect("a piece of the Moon Lord")
    }

    /// It opens with two hands and a head, once.
    #[test]
    fn it_opens_with_three_eyes() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let mut c = piece(MOON_LORD_CORE);

        let mut spawned = Vec::new();
        for _ in 0..(MOON_LORD_OPENING as i32 + 2) {
            spawned.extend(core(&mut c, &w, 0, 0).spawn);
        }
        assert_eq!(spawned.len(), 3, "two hands and a head");
        assert_eq!(
            spawned
                .iter()
                .filter(|s| s.npc_type == MOON_LORD_HAND)
                .count(),
            2
        );
        assert_eq!(
            spawned
                .iter()
                .filter(|s| s.npc_type == MOON_LORD_HEAD)
                .count(),
            1
        );
    }

    /// ML-7: the two hands are seated by `ai[2]` (0, then 1), the index vanilla hands each one
    /// (`NPC.cs:41649`, `Main.npc[num2].ai[2] = i`). The hand routine reads that as its side; left
    /// unset both would read 0 and station on top of each other.
    #[test]
    fn its_two_hands_seat_on_opposite_sides() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let mut c = piece(MOON_LORD_CORE);

        let mut spawned = Vec::new();
        for _ in 0..(MOON_LORD_OPENING as i32 + 2) {
            spawned.extend(core(&mut c, &w, 0, 0).spawn);
        }
        let sides: Vec<f32> = spawned
            .iter()
            .filter(|s| s.npc_type == MOON_LORD_HAND)
            .map(|s| s.ai[2].expect("a hand's side is pinned in ai[2], not left to signum"))
            .collect();
        assert_eq!(sides, vec![0.0, 1.0], "one hand each side, not both at 0");

        // And the hand routine really stations them apart off that ai[2]: seat two broken hands,
        // one per side, and watch them pull toward opposite ends of the core.
        let core_part = core_at((0.0, 0.0), state::WAITING);
        let pull = |side: f32| {
            let mut hand = piece(MOON_LORD_HAND);
            hand.ai[0] = state::BROKEN;
            hand.ai[2] = side;
            eye_socket(
                &mut hand,
                &w,
                Some(core_part),
                &mut SmallRng::seed_from_u64(0),
            );
            hand.velocity.0
        };
        assert!(pull(0.0) < pull(1.0), "ai[2]=0 seats left of ai[2]=1");
    }

    /// The core cannot be hurt until every eye is broken. That is the whole structure of the fight.
    #[test]
    fn the_core_opens_only_when_every_eye_is_broken() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let mut c = piece(MOON_LORD_CORE);
        c.local_ai[3] = 1.0;
        c.ai[0] = state::WAITING;

        for open in 0..3 {
            core(&mut c, &w, 3, open);
            assert!(c.invulnerable, "{open} eyes broken is not enough");
            assert!(!c.take_damage(9999, 0.0, 1));
        }
        core(&mut c, &w, 3, 3);
        assert!(!c.invulnerable, "all three: now it is open");
    }

    /// Each eye is given one of three scripts, and they are not all the same.
    #[test]
    fn the_eyes_run_different_scripts() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let mut scripts = std::collections::HashSet::new();
        for seed in 0..40 {
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut h = piece(MOON_LORD_HAND);
            eye_socket(
                &mut h,
                &w,
                Some(core_at((0.0, 0.0), state::WAITING)),
                &mut rng,
            );
            scripts.insert(h.ai[3] as i32);
        }
        assert!(
            scripts.len() > 1,
            "they should not all get the same: {scripts:?}"
        );
        assert!(
            scripts.iter().all(|s| (0..3).contains(s)),
            "and all of them real: {scripts:?}"
        );
    }

    /// The head fires a deathray and puts out leeches; a hand does neither.
    #[test]
    fn only_the_head_fires_the_deathray() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let fired = |npc_type: u16| {
            let mut rng = SmallRng::seed_from_u64(3);
            let mut e = piece(npc_type);
            let mut projectiles = std::collections::HashSet::new();
            let mut leeches = 0;
            for _ in 0..4000 {
                let out = eye_socket(
                    &mut e,
                    &w,
                    Some(core_at((0.0, 0.0), state::WAITING)),
                    &mut rng,
                );
                for shot in out.shots {
                    projectiles.insert(shot.projectile);
                }
                leeches += out.spawn.len();
            }
            (projectiles, leeches)
        };
        let (head_shots, head_leeches) = fired(MOON_LORD_HEAD);
        assert!(
            head_shots.contains(&PHANTASMAL_DEATHRAY),
            "the head has the ray"
        );
        assert!(head_leeches > 0, "and puts out leeches");

        let (hand_shots, hand_leeches) = fired(MOON_LORD_HAND);
        assert!(
            !hand_shots.contains(&PHANTASMAL_DEATHRAY),
            "a hand does not"
        );
        assert_eq!(hand_leeches, 0, "nor leeches");
        assert!(
            hand_shots.contains(&PHANTASMAL_EYE),
            "it throws eyes instead"
        );
    }

    /// An eye with no core does not survive, and a broken one takes nothing.
    #[test]
    fn a_socket_without_a_core_is_gone() {
        let tiles = Sky(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(5);
        let w = world(&tiles, Some((0.0, 600.0)));
        let mut h = piece(MOON_LORD_HAND);
        assert!(eye_socket(&mut h, &w, None, &mut rng).spent);

        let mut broken = piece(MOON_LORD_HAND);
        broken.local_ai[3] = 1.0;
        broken.ai[0] = state::BROKEN;
        eye_socket(
            &mut broken,
            &w,
            Some(core_at((0.0, 0.0), state::WAITING)),
            &mut rng,
        );
        assert!(broken.invulnerable, "an empty socket takes nothing");
    }

    /// A free eye hunts on its own.
    #[test]
    fn a_free_eye_comes_after_you() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((2000.0, 0.0)));
        let mut e = piece(MOON_LORD_FREE_EYE);
        for _ in 0..200 {
            free_eye(&mut e, &w);
        }
        assert!(e.velocity.0 > 1.0, "it should be closing: {}", e.velocity.0);
    }

    /// A leech delivers its load and is gone; without an anchor it simply goes.
    #[test]
    fn a_leech_carries_life_home() {
        let mut l = piece(MOON_LORD_LEECH);
        assert!(leech(&mut l, None).spent, "nothing to carry it to");

        let mut l = piece(MOON_LORD_LEECH);
        let anchor = core_at((0.0, 0.0), state::WAITING);
        let mut delivered = 0;
        for _ in 0..(LEECH_TICKS as i32 + 2) {
            let out = leech(&mut l, Some(anchor));
            delivered += out.healed;
            if out.spent {
                break;
            }
        }
        assert_eq!(
            delivered, LEECH_HEAL,
            "it should have delivered exactly once"
        );
    }
}
