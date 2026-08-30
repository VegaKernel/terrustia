//! The Moon Lord: styles 77–79, 81 and 82.
//!
//! The core is not the fight. It hangs a hundred and thirty pixels below you, cannot be hurt at
//! all, and waits — its two hands and its head are the fight, and only once all three are open does
//! the core become something you can attack.
//!
//! Each part runs a fixed five-entry attack timeline, and which one is decided by which part it is,
//! not by chance: the left hand runs row 0, the right hand row 1, and the head row 2 of
//! `MoonLordAttacksArray` (`NPC.cs:42032`, `NPC.cs:42530`). Every tick a part steps its row by the
//! cumulative timer in `ai[1]`, writes the current attack into `ai[0]` and runs it. The three are
//! therefore always doing different things at once, and the fight has a shape rather than a rhythm.
//!
//! The same attack id means different things on a hand and on the head. Attack 0 is a pause for
//! both. Attack 1 is a hand's rapid eye-stream, but the head's charged deathray: a hundred and
//! eighty ticks of wind-up, then the beam. Attack 2 is a hand's six-sphere barrage, but the head's
//! leech attack. Attack 3 is the spread of bolts, the same for both.
//!
//! Breaking a socket does not remove it from the fight: the eye comes out and hunts you as a free
//! eye. And the head puts out leeches that carry life back to whichever part is most hurt, so
//! ignoring them undoes work you have already done.

use terrustia_proto::npc_params::{
    FREE_EYE_ACCEL, FREE_EYE_SPEED, LEECH_HEAL, LEECH_TICKS, MOON_LORD_ACCEL, MOON_LORD_BELOW,
    MOON_LORD_CORE, MOON_LORD_DEATH_TICKS, MOON_LORD_FIGHTING_DISTANCE, MOON_LORD_FREE_EYE,
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

/// Vanilla `checkDead` (`NPC.cs:78864-78883`): the Moon Lord's parts do not die when their life
/// runs out. A hand or head becomes a broken, empty socket (`ai[0] = -2`) and is refilled so it
/// hangs on as a shell (its True Eye is then freed by [`eye_socket`] on the socket's next tick,
/// where the spawn plumbing lives); the core enters its ten-second death drama (`ai[0] = 2`), and
/// only the end of that drama is the actual kill. Returns true when the lethal blow was
/// intercepted, in which case the caller must not reap the NPC.
pub fn checkdead(npc: &mut Npc) -> bool {
    match npc.npc_type {
        // `PrepareForDeathAnimation` (`NPC.cs:78836-78842`): full life again, no longer takeable.
        MOON_LORD_HAND | MOON_LORD_HEAD if npc.ai[0] != state::BROKEN => {
            npc.ai[0] = state::BROKEN;
            npc.ai[1] = 0.0;
            npc.life = npc.life_max;
            npc.dirty = true;
            true
        }
        MOON_LORD_CORE if npc.ai[0] != state::DYING => {
            npc.ai[0] = state::DYING;
            npc.ai[1] = 0.0;
            npc.life = npc.life_max;
            npc.dirty = true;
            true
        }
        _ => false,
    }
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
/// `parts_open` is how many of its three eyes have been broken (counting both a socket that has
/// left the table and one still hanging on as a broken shell), worked out by the caller.
pub fn core(npc: &mut Npc, world: &World<'_, impl TileView>, parts_open: usize) -> MoonLordOutcome {
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

    // ML-1: the core is never removed just because its limbs are gone. In vanilla the sockets
    // stay on the field as broken shells (they never go inactive, so the core's own "a limb
    // vanished" guard at `NPC.cs:41697-41702` does not fire in an ordinary fight), and the only
    // way the core leaves is through its death drama (`ai[0] == 2`), reached from `checkDead` once
    // every socket is open and the exposed core has been struck down. Counting the last limb's
    // fall as a death (the old `parts == 0 -> spent`) killed the boss the instant it fell and left
    // the exposed-core finale and the death sequence as dead code.
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

/// Which of the three attack rows a part runs. It is fixed by which part it is, never random: a
/// hand takes the row for its side (`ai[2]`: left is 0, right is 1), the head takes row 2. Vanilla
/// `AI_078` picks `num6 = (ai[2]==0) ? 0 : 1` (`NPC.cs:42032`); `AI_079` fixes `num5 = 2`
/// (`NPC.cs:42530`).
fn attack_row(npc: &Npc, head: bool) -> usize {
    if head {
        2
    } else if npc.ai[2] >= 1.0 {
        1
    } else {
        0
    }
}

/// Step the fixed timeline by the cumulative timer in `ai[1]`, returning the current attack, how
/// long it has run (`num2`), and the current step's total length (`num3`). `ai[0]` is written with
/// the current attack so the wire carries it and a client plays the right animation. Transcribes
/// the walk in `AI_078` (`NPC.cs:42027-42055`) and `AI_079` (`NPC.cs:42541-42566`): find the step
/// whose cumulative end is past `ai[1]`, and wrap `ai[1]` back to zero once the last one is behind.
fn step_timeline(npc: &mut Npc, row: &[(u8, i32); 5]) -> (u8, f32, i32) {
    npc.ai[1] += 1.0;
    let mut acc = 0i32;
    let mut idx = row.len();
    for (i, &(_, dur)) in row.iter().enumerate() {
        if (dur + acc) as f32 > npc.ai[1] {
            idx = i;
            break;
        }
        acc += dur;
    }
    if idx == row.len() {
        idx = 0;
        acc = 0;
        npc.ai[1] = 0.0;
    }
    let (attack, dur) = row[idx];
    npc.ai[0] = f32::from(attack);
    (attack, npc.ai[1] - acc as f32, dur)
}

/// Styles 78 and 79: a hand or the head.
pub fn eye_socket(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    core: Option<Parent>,
) -> MoonLordOutcome {
    let mut out = MoonLordOutcome::default();
    npc.dirty = true;
    let head = npc.npc_type == MOON_LORD_HEAD;

    let Some(core) = core else {
        out.spent = true;
        return out;
    };
    // ML-6: the wire carries the core's slot in `ai[3]`, the way vanilla does
    // (`NPC.cs:42010,42522`, `Main.npc[(int)ai[3]]`), so a client running the part's own AI finds
    // the right parent. The engine tracks the link separately in `follows_boss`; this only mirrors
    // that slot into the synced `ai` array.
    if let Some(slot) = npc.follows_boss {
        npc.ai[3] = f32::from(slot);
    }

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
        // ML-2: on the tick it breaks, its eye comes out and hunts as a free True Eye of Cthulhu
        // (`NPC.cs:78873`, `MoonLord_SpawnTrueEyeOfCthulhu`). Vanilla spawns it from `checkDead`;
        // we free it here on the socket's next tick, where the spawn plumbing lives, latched by
        // `local_ai[1]` so exactly one is freed per socket.
        if npc.local_ai[1] == 0.0 {
            npc.local_ai[1] = 1.0;
            out.spawn.push(Spawn {
                npc_type: MOON_LORD_FREE_EYE,
                position: npc.center(),
                velocity: (0.0, 0.0),
                parent: None,
                ai: [None; 4],
            });
        }
        if core.state == state::DYING {
            out.spent = true;
        }
        return out;
    }

    let Some(target) = world.target.filter(|t| t.alive) else {
        return out;
    };

    // ML-3: the row is fixed by which part this is, and the timeline is stepped by `ai[1]`.
    let row = &MOON_LORD_SCRIPTS[attack_row(npc, head)];
    let (attack, within, dur) = step_timeline(npc, row);

    if head {
        run_head_attack(npc, &mut out, target.center, attack, within, dur);
    } else {
        run_hand_attack(npc, &mut out, target.center, attack, within, dur);
    }
    out
}

/// A hand's attacks. Attack 1 is the eye stream, attack 2 the six-sphere barrage, attack 3 the
/// bolt spread. It never runs the head's deathray or leech attack.
fn run_hand_attack(
    npc: &Npc,
    out: &mut MoonLordOutcome,
    target: (f32, f32),
    attack: u8,
    within: f32,
    dur: i32,
) {
    match attack {
        1 => {
            // The eye stream: proj 452 fired every four ticks through the middle third of the
            // window (`NPC.cs:42128-42159`, the `num2` in `[num8*num9, num8*num9*2)` band with
            // `num8=7, num9=4`, one shot each `num9` ticks).
            let band = 7.0 * 4.0;
            if within >= band && within < band * 2.0 && (within - band) % 4.0 == 0.0 {
                out.shots.push(aimed(
                    npc,
                    target,
                    PHANTASMAL_EYE,
                    PHANTASMAL_EYE_DAMAGE,
                    8.0,
                ));
            }
        }
        2 => {
            // ML-5: the heavy attack is SIX spheres, not one. Vanilla gathers six proj 454 over a
            // hundred and eighty ticks (`num12 % 30 == 0`, six times) and launches them together
            // toward the player at `num2 == 292` (`NPC.cs:42184-42260`). The projectile layer has no
            // hover-then-relaunch AI (the same narrowing the deathray already carries), so the six
            // are fired as one aimed fan at the launch tick rather than gathered first.
            if within as i32 == 292 {
                fire_fan(
                    npc,
                    out,
                    target,
                    6,
                    PHANTASMAL_SPHERE,
                    PHANTASMAL_SPHERE_DAMAGE,
                    12.0,
                );
            }
        }
        3 => fire_spread(npc, out, target, within, dur),
        // Nought is a pause.
        _ => {}
    }
}

/// The head's attacks. Attack 1 is the charged deathray, attack 2 the leech attack, attack 3 the
/// bolt spread. ML-4: it charges and fires the deathray, and runs neither of the hands' attacks.
fn run_head_attack(
    npc: &Npc,
    out: &mut MoonLordOutcome,
    target: (f32, f32),
    attack: u8,
    within: f32,
    dur: i32,
) {
    match attack {
        1 => {
            // ML-4: a hundred and eighty ticks of wind-up, then the beam (`NPC.cs:42606-42690`:
            // dust while `num < 180`, proj 455 at `num == 180`). A hand's id-1 attack is the eye
            // stream instead, so only the head ever fires the deathray. The projectile flies
            // straight for its lifetime (the sweep is a projectile-lane concern, not modelled here).
            if within as i32 == 180 {
                let (cx, cy) = npc.center();
                let aim = (target.0 - cx, target.1 - cy);
                let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
                out.shots.push(Shot {
                    projectile: PHANTASMAL_DEATHRAY,
                    damage: PHANTASMAL_DEATHRAY_DAMAGE,
                    position: npc.center(),
                    velocity: (aim.0 / length, aim.1 / length),
                    time_left: MOON_LORD_RAY_SWEEP as u16,
                });
            }
        }
        2 => {
            // The leech attack: the head puts out leeches that carry life back to the most-hurt
            // part (`NPC.cs:42691-42730`, proj 456 branding players and spawning NPC 401). Narrowed
            // to one leech every sixty ticks; the brand-then-blob plumbing is not modelled.
            if within % 60.0 == 0.0 {
                out.spawn.push(Spawn {
                    npc_type: MOON_LORD_LEECH,
                    position: npc.center(),
                    velocity: (0.0, 0.0),
                    parent: Some(Spawn::OWN_PARENT),
                    ai: [None; 4],
                });
            }
        }
        3 => fire_spread(npc, out, target, within, dur),
        _ => {}
    }
}

/// The bolt spread (attack 3), the same for a hand and the head: a bolt at `num2 == num3-14`,
/// `num3-7` and `num3` (`NPC.cs:42297-42302`, `NPC.cs:42760-42766`). `num2` only ever reaches
/// `num3-1` within a step, so the third (`== num3`) never fires, exactly as in vanilla.
fn fire_spread(npc: &Npc, out: &mut MoonLordOutcome, target: (f32, f32), within: f32, dur: i32) {
    let n = within as i32;
    if n == dur - 14 || n == dur - 7 || n == dur {
        out.shots.push(aimed(
            npc,
            target,
            PHANTASMAL_BOLT,
            PHANTASMAL_BOLT_DAMAGE,
            8.0,
        ));
    }
}

/// Throw `count` shots at the player in an even fan, so a barrage reads as more than one shot.
fn fire_fan(
    npc: &Npc,
    out: &mut MoonLordOutcome,
    target: (f32, f32),
    count: usize,
    projectile: u16,
    damage: i32,
    speed: f32,
) {
    let (cx, cy) = npc.center();
    let base = (target.1 - cy).atan2(target.0 - cx);
    for i in 0..count {
        let spread = (i as f32 - (count as f32 - 1.0) / 2.0) * 0.12;
        let angle = base + spread;
        out.shots.push(Shot {
            projectile,
            damage,
            position: (cx, cy),
            velocity: (angle.cos() * speed, angle.sin() * speed),
            time_left: 600,
        });
    }
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
    use std::collections::HashMap;
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
            spawned.extend(core(&mut c, &w, 0).spawn);
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
            spawned.extend(core(&mut c, &w, 0).spawn);
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
            eye_socket(&mut hand, &w, Some(core_part));
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
            core(&mut c, &w, open);
            assert!(c.invulnerable, "{open} eyes broken is not enough");
            assert!(!c.take_damage(9999, 0.0, 1));
        }
        core(&mut c, &w, 3);
        assert!(!c.invulnerable, "all three: now it is open");
    }

    /// ML-1: the core is not removed the instant its last limb falls. In vanilla the sockets stay
    /// as broken shells and the core leaves only through its death drama (`NPC.cs:41697-41722`);
    /// the old `parts == 0 -> spent` killed the boss outright and left the finale as dead code.
    #[test]
    fn losing_every_limb_is_not_a_death() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let mut c = piece(MOON_LORD_CORE);
        c.local_ai[3] = 1.0;
        c.ai[0] = state::WAITING;
        // No hands or head on the field, but all three sockets accounted as open.
        let out = core(&mut c, &w, 3);
        assert!(!out.spent, "no limbs left is not, by itself, a death");
        assert!(
            !c.invulnerable,
            "with every socket open the core is exposed, not gone"
        );
    }

    /// ML-1: struck down, the exposed core does not die on the hit. `checkdead` sends it into its
    /// ten-second drama (`NPC.cs:78878-78883`), and only the end of that drama is the kill
    /// (`NPC.cs:41869-41875`). Reverting `checkdead` to `false` fails the first assert; leaving the
    /// old `parts == 0` death or a zero `MOON_LORD_DEATH_TICKS` fails the drama.
    #[test]
    fn the_exposed_core_dies_only_after_its_death_drama() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let mut c = piece(MOON_LORD_CORE);
        c.local_ai[3] = 1.0;
        c.ai[0] = state::FIGHTING;
        c.life = 1;

        assert!(
            checkdead(&mut c),
            "a lethal blow on the exposed core is intercepted"
        );
        assert_eq!(
            c.ai[0],
            state::DYING,
            "it enters the death drama, not the grave"
        );
        assert_eq!(c.life, c.life_max, "and is not left lingering at zero life");

        let mut spent_after = None;
        for tick in 1..=(MOON_LORD_DEATH_TICKS as i32 + 2) {
            if core(&mut c, &w, 3).spent {
                spent_after = Some(tick);
                break;
            }
        }
        assert_eq!(
            spent_after,
            Some(MOON_LORD_DEATH_TICKS as i32),
            "the drama runs its full ten seconds, then the kill"
        );
    }

    /// Run a part through enough ticks to loop its row, and return the attack ids in the order they
    /// first ran (consecutive repeats of the same attack collapsed).
    fn attack_sequence(w: &World<'_, Sky>, npc_type: u16, side: f32) -> Vec<u8> {
        let mut e = piece(npc_type);
        e.ai[2] = side;
        let mut seq: Vec<u8> = Vec::new();
        for _ in 0..2400 {
            eye_socket(&mut e, w, Some(core_at((0.0, 0.0), state::WAITING)));
            let a = e.ai[0] as u8;
            if seq.last() != Some(&a) {
                seq.push(a);
            }
        }
        seq
    }

    /// ML-3: the attack row is fixed by which part it is, never random. The left hand runs row 0,
    /// the right hand row 1, the head row 2, and each steps that row in order. Restoring the random
    /// `ai[3] = rng` script (or picking the row any other way) breaks this per-part sequence.
    #[test]
    fn each_part_runs_its_own_fixed_row() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let expect =
            |row: usize| -> Vec<u8> { MOON_LORD_SCRIPTS[row].iter().map(|s| s.0).collect() };
        assert_eq!(
            attack_sequence(&w, MOON_LORD_HAND, 0.0)[..5],
            expect(0)[..],
            "the left hand runs row 0"
        );
        assert_eq!(
            attack_sequence(&w, MOON_LORD_HAND, 1.0)[..5],
            expect(1)[..],
            "the right hand runs row 1"
        );
        assert_eq!(
            attack_sequence(&w, MOON_LORD_HEAD, 0.0)[..5],
            expect(2)[..],
            "the head runs row 2, whatever its ai[2]"
        );
    }

    /// ML-6: a part carries its parent core's slot in `ai[3]` on the wire, so a client running the
    /// part's own AI can find the core it hangs from. The old model overwrote `ai[3]` with a random
    /// script index (0, 1 or 2), which a client would read as "my parent is NPC slot 0, 1 or 2".
    #[test]
    fn a_part_carries_its_parent_slot_on_the_wire() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let mut h = piece(MOON_LORD_HAND);
        h.follows_boss = Some(37);
        eye_socket(&mut h, &w, Some(core_at((0.0, 0.0), state::WAITING)));
        assert_eq!(
            h.ai[3], 37.0,
            "ai[3] carries the core's slot, not a script index"
        );
    }

    /// ML-5: a hand's heavy attack (id 2) throws six spheres, not one. The old model threw a single
    /// phantasmal eye on the attack's first tick.
    #[test]
    fn a_hands_heavy_attack_throws_six_spheres() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        // The left hand (row 0) reaches attack 2 after its idle (50) and stream (70). One full loop
        // (600 ticks) passes through the single launch.
        let mut h = piece(MOON_LORD_HAND);
        h.ai[2] = 0.0;
        let mut spheres = 0;
        for _ in 0..600 {
            let out = eye_socket(&mut h, &w, Some(core_at((0.0, 0.0), state::WAITING)));
            spheres += out
                .shots
                .iter()
                .filter(|s| s.projectile == PHANTASMAL_SPHERE)
                .count();
        }
        assert_eq!(spheres, 6, "six spheres launched together, not one");
    }

    /// ML-4: the head fires the deathray and puts out leeches, and runs neither of a hand's attacks;
    /// a hand throws eyes and spheres and never the deathray. The same attack id means different
    /// things on the two parts (id 1 is a hand's eye stream but the head's deathray; id 2 is a
    /// hand's sphere barrage but the head's leech attack), which is why the sets do not overlap.
    #[test]
    fn the_head_and_hands_run_different_attacks() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let fired = |npc_type: u16| {
            let mut e = piece(npc_type);
            let mut projectiles = std::collections::HashSet::new();
            let mut leeches = 0;
            for _ in 0..4000 {
                let out = eye_socket(&mut e, &w, Some(core_at((0.0, 0.0), state::WAITING)));
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
        assert!(
            !head_shots.contains(&PHANTASMAL_SPHERE),
            "the head does not run the hand's sphere barrage"
        );

        let (hand_shots, hand_leeches) = fired(MOON_LORD_HAND);
        assert!(
            !hand_shots.contains(&PHANTASMAL_DEATHRAY),
            "a hand never fires the deathray"
        );
        assert_eq!(hand_leeches, 0, "nor leeches");
        assert!(
            hand_shots.contains(&PHANTASMAL_EYE) && hand_shots.contains(&PHANTASMAL_SPHERE),
            "it throws eyes and spheres instead"
        );
    }

    /// ML-4: the head charges its deathray for a hundred and eighty ticks before it fires, rather
    /// than loosing it the instant the attack begins. Seat the timeline at the start of row 2's
    /// deathray step (cumulative start 180+30+435+180 = 825) and watch when the ray appears.
    #[test]
    fn the_head_charges_before_the_deathray_fires() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let mut head = piece(MOON_LORD_HEAD);
        head.follows_boss = Some(5);
        head.ai[1] = 825.0;
        let mut ray_ticks = Vec::new();
        for tick in 0..375 {
            let out = eye_socket(&mut head, &w, Some(core_at((0.0, 0.0), state::WAITING)));
            if head.ai[0] as u8 != 1 {
                break; // left the deathray step
            }
            if out
                .shots
                .iter()
                .any(|s| s.projectile == PHANTASMAL_DEATHRAY)
            {
                ray_ticks.push(tick);
            }
        }
        assert_eq!(
            ray_ticks,
            vec![179],
            "the ray fires once, on the hundred-and-eightieth tick of the charge"
        );
    }

    /// An eye with no core does not survive, and a broken one takes nothing.
    #[test]
    fn a_socket_without_a_core_is_gone() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));
        let mut h = piece(MOON_LORD_HAND);
        assert!(eye_socket(&mut h, &w, None).spent);

        let mut broken = piece(MOON_LORD_HAND);
        broken.local_ai[3] = 1.0;
        broken.ai[0] = state::BROKEN;
        eye_socket(&mut broken, &w, Some(core_at((0.0, 0.0), state::WAITING)));
        assert!(broken.invulnerable, "an empty socket takes nothing");
    }

    /// ML-2: breaking a socket does not kill the part. A struck hand or head becomes a broken,
    /// refilled shell (`checkdead`, `NPC.cs:78864-78876`) and, on its next tick, frees exactly one
    /// True Eye of Cthulhu (`MoonLord_SpawnTrueEyeOfCthulhu`, `NPC.cs:78873`). Reverting `checkdead`
    /// to `false` fails the interception asserts; dropping the `eye_socket` spawn frees no eye.
    #[test]
    fn breaking_a_socket_opens_it_and_frees_an_eye() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((0.0, 600.0)));

        let mut h = piece(MOON_LORD_HAND);
        h.local_ai[3] = 1.0;
        h.ai[0] = state::WAITING;
        h.life = 1;
        assert!(
            checkdead(&mut h),
            "a lethal blow on a socket is intercepted"
        );
        assert_eq!(h.ai[0], state::BROKEN, "it opens rather than dies");
        assert_eq!(
            h.life, h.life_max,
            "and refills, an empty shell that hangs on"
        );

        let core_part = core_at((0.0, 0.0), state::WAITING);
        let mut freed = 0;
        for _ in 0..10 {
            freed += eye_socket(&mut h, &w, Some(core_part))
                .spawn
                .iter()
                .filter(|s| s.npc_type == MOON_LORD_FREE_EYE)
                .count();
        }
        assert_eq!(freed, 1, "one True Eye of Cthulhu, freed exactly once");
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
