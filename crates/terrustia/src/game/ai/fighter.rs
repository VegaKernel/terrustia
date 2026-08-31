//! Style 3 — the fighters.
//!
//! Zombies, skeletons, goblins and 59 other pre-hardmode types: everything that walks at you on
//! the ground. Ported from `AI_003_Fighters`, whose 4,511 lines are 85% per-type branches; those
//! branches are numbers and live in [`terrustia_proto::npc_params`], leaving the algorithm here.
//!
//! The three behaviours that make a fighter read correctly, and that a naive version misses:
//!
//! * It **steps up** ledges by moving its position, rather than jumping. Jumping is for walls it
//!   cannot step over.
//! * It **works at doors**: sixty ticks of pushing, then either opening one or breaking it down,
//!   depending on the type and whether a blood moon is out.
//! * It only refuses to walk off a ledge when it has no target. With a target, it walks off.
//!
//! Deliberately not modelled: `directionY`. Vanilla's `SetTargetTrackingValues` gives style 3 its
//! own rule for it (`NPC.cs:78565-78572` — a fighter looks up only when the target's *feet* are
//! above its own head, rather than the centre-to-centre test every other style uses), and one
//! branch of `AI_003` reads it: a fighter whose target is above it launches off a ledge edge at
//! -8 with 1.5x its horizontal speed instead of stepping down (`NPC.cs:60729-60734`). This port
//! has neither the rule nor the branch, so nothing here reads `direction_y` and it stays at its
//! default. The field is still synced (`systems.rs`'s packet 23), but a real client recomputes it
//! from its own `TargetClosest` every tick, so what we send it is corrected immediately.

use terrustia_proto::{
    npc_params::{
        STEP_HEIGHT, STEP_HEIGHT_TALL, fighter_movement, fighter_opens_doors, fighter_tall_step,
        fighter_wide_probe,
    },
    tile_solid::{solid, solid_top},
};

use super::Conditions;
use crate::game::npc::Npc;
use crate::game::npc::{TILE, TileView};
use crate::game::npc_ai::Target;

/// Closed door, and the closed tall gate.
pub const DOOR: u16 = 10;
pub const TALL_GATE: u16 = 388;

/// Ticks of pushing before a fighter makes any impression on a door.
pub const DOOR_PUSH_TICKS: f32 = 60.0;

/// Accumulated damage at which a door gives way.
pub const DOOR_BREAK_THRESHOLD: f32 = 10.0;

/// Damage a push does to an ordinary door, and to a tall gate.
pub const DOOR_DAMAGE: f32 = 5.0;
pub const TALL_GATE_DAMAGE: f32 = 2.0;

/// Jump impulse used to clear a wall the fighter cannot step over.
pub const JUMP_SPEED: f32 = 6.0;

/// What the fighter wants the world to do on its behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// Swing a door open. Only the types in `fighter_opens_doors` do this, and not on a blood moon.
    OpenDoor {
        x: i32,
        y: i32,
        direction: i8,
    },
    /// Smash it off its hinges.
    BreakDoor {
        x: i32,
        y: i32,
    },
}

fn blocking(tiles: &impl TileView, x: i32, y: i32) -> bool {
    let t = tiles.tile(x, y);
    t.is_active() && solid(t.block) && !solid_top(t.block)
}

fn is_door(tiles: &impl TileView, x: i32, y: i32) -> Option<u16> {
    let t = tiles.tile(x, y);
    (t.is_active() && (t.block == DOOR || t.block == TALL_GATE)).then_some(t.block)
}

/// Move toward the target, accelerating to the type's walking speed.
fn walk(npc: &mut Npc, target: Option<Target>) {
    if let Some(t) = target {
        npc.direction = if t.center.0 > npc.center().0 { 1 } else { -1 };
    }
    npc.sprite_direction = npc.direction;

    let m = fighter_movement(npc.npc_type);
    if npc.velocity.0 < -m.max_speed || npc.velocity.0 > m.max_speed {
        // Over top speed: bleed it off, but only while actually standing on something.
        if npc.velocity.1 == 0.0 {
            npc.velocity.0 *= m.friction;
            npc.velocity.1 *= m.friction;
        }
    } else if npc.direction > 0 {
        npc.velocity.0 = (npc.velocity.0 + m.accel).min(m.max_speed);
    } else {
        npc.velocity.0 = (npc.velocity.0 - m.accel).max(-m.max_speed);
    }
}

/// Try to step up onto the tile ahead. Returns true if the fighter climbed it.
///
/// This is what the game does instead of jumping for anything up to one tile high: it moves the
/// NPC's position directly, which is why a zombie walks up stairs smoothly rather than hopping.
fn try_step_up(npc: &mut Npc, tiles: &impl TileView) -> bool {
    let ahead = if npc.velocity.0 < 0.0 {
        -1
    } else if npc.velocity.0 > 0.0 {
        1
    } else {
        return false;
    };

    let next_x = npc.position.0 + npc.velocity.0;
    let probe_x =
        ((next_x + npc.width() / 2.0 + (npc.width() / 2.0 + 1.0) * ahead as f32) / TILE) as i32;
    let foot_y = ((npc.position.1 + npc.velocity.1.max(0.0) + npc.height() - 1.0) / TILE) as i32;

    // Something solid at foot level, and clear space for the next three tiles above it.
    if !blocking(tiles, probe_x, foot_y) {
        return false;
    }
    for up in 1..=3 {
        if blocking(tiles, probe_x, foot_y - up) {
            return false;
        }
    }
    // And clear behind the step, so it is not squeezing into a gap.
    if blocking(tiles, probe_x - ahead, foot_y - 3) {
        return false;
    }

    let step_top = foot_y as f32 * TILE;
    if step_top >= npc.position.1 + npc.height() {
        return false;
    }
    let rise = npc.position.1 + npc.height() - step_top;
    let limit = if fighter_tall_step(npc.npc_type) {
        STEP_HEIGHT_TALL
    } else {
        STEP_HEIGHT
    };
    if rise > limit {
        return false;
    }

    npc.position.1 = step_top - npc.height();
    npc.dirty = true;
    true
}

/// Outcome of looking at what is directly ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DoorState {
    /// Nothing there; the fighter is free to step up or jump.
    Clear,
    /// A door is in the way. The fighter leans on it and does not try to climb or vault it — the
    /// game handles a door instead of treating it as terrain, which is why a zombie stands and
    /// works at one rather than hopping over.
    Busy(Action),
}

/// Work at a door ahead.
fn work_at_door(
    npc: &mut Npc,
    tiles: &impl TileView,
    conditions: Conditions,
    target: Option<Target>,
) -> DoorState {
    // Wide types reach further ahead for the probe.
    let reach = if fighter_wide_probe(npc.npc_type) {
        npc.width() / 2.0 + 16.0
    } else {
        15.0
    };
    let x = ((npc.position.0 + npc.width() / 2.0 + reach * f32::from(npc.direction)) / TILE) as i32;
    let y = ((npc.position.1 + npc.height() - 15.0) / TILE) as i32;

    let Some(door_type) = is_door(tiles, x, y - 1) else {
        npc.ai[2] = 0.0;
        return DoorState::Clear;
    };
    if target.is_none() {
        // Without a target it has no reason to force the door, but it still will not vault it.
        return DoorState::Busy(Action::None);
    }

    npc.ai[2] += 1.0;
    npc.ai[3] = 0.0;
    if npc.ai[2] < DOOR_PUSH_TICKS {
        return DoorState::Busy(Action::None);
    }
    npc.ai[2] = 0.0;

    let is_opener = fighter_opens_doors(npc.npc_type);
    npc.velocity.0 = 0.5 * f32::from(-npc.direction);

    // Vanilla `AI_003`'s `flag28` (`NPC.cs:60601`): `((!bloodMoon || getGoodWorld) && !graveyard)
    // & isOpener`. A polite opener resets its progress every tick, so it stands and pushes at the
    // door but never gets it open. That is the whole "a closed door keeps zombies out at night,
    // except on a blood moon" mechanic — the previous code had it backwards, opening on a normal
    // night and destroying the door on a blood moon.
    //
    // `getGoodWorld` takes the blood moon back out of it: on a For-the-Worthy seed doors stay
    // polite through one, which is not the harder-world direction it reads as but is what the
    // expression says. Not modelled: the graveyard term, which this server has no biome for, and
    // vanilla's `flag27` (a target standing inside unbreakable walls), which forces every fighter
    // impolite regardless of type.
    if is_opener && (!conditions.blood_moon || conditions.get_good_world) {
        npc.ai[1] = 0.0;
        return DoorState::Busy(Action::None);
    }

    npc.ai[1] += if door_type == TALL_GATE {
        TALL_GATE_DAMAGE
    } else {
        DOOR_DAMAGE
    };
    if npc.ai[1] >= DOOR_BREAK_THRESHOLD {
        npc.ai[1] = 0.0;
        // Once it forces the door: a polite opener swings it open and the door survives (vanilla's
        // `OpenDoor` branch); a door-breaker such as the Goblin Peon destroys it (`KillTile`).
        return if is_opener {
            DoorState::Busy(Action::OpenDoor {
                x,
                y: y - 1,
                direction: npc.direction,
            })
        } else {
            DoorState::Busy(Action::BreakDoor { x, y: y - 1 })
        };
    }
    DoorState::Busy(Action::None)
}

/// Jump a wall that could not be stepped over, or turn back from a ledge.
fn jump_or_turn(npc: &mut Npc, tiles: &impl TileView, target: Option<Target>) {
    if npc.velocity.1 != 0.0 {
        return;
    }
    let ahead = npc.direction;
    let probe_x =
        ((npc.center().0 + f32::from(ahead) * (npc.width() / 2.0 + 2.0)) / TILE).floor() as i32;
    let foot_y = ((npc.position.1 + npc.height() - 1.0) / TILE).floor() as i32;

    if blocking(tiles, probe_x, foot_y) {
        npc.velocity.1 = -JUMP_SPEED;
        npc.dirty = true;
        return;
    }

    // A fighter only avoids a drop when it has nothing to chase; with a target it walks off.
    if target.is_none() {
        let ground = (1..=4).any(|d| {
            let t = tiles.tile(probe_x, foot_y + d);
            t.is_active() && solid(t.block)
        });
        if !ground {
            npc.direction = -npc.direction;
            npc.velocity.0 = 0.0;
        }
    }
}

/// Drive one fighter for a tick.
pub fn update(
    npc: &mut Npc,
    tiles: &impl TileView,
    target: Option<Target>,
    conditions: Conditions,
) -> Action {
    walk(npc, target);

    // A door is dealt with as a door, never climbed or vaulted.
    if let DoorState::Busy(action) = work_at_door(npc, tiles, conditions, target) {
        return action;
    }

    if !try_step_up(npc, tiles) {
        jump_or_turn(npc, tiles, target);
    }
    Action::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    struct Terrain<F>(F);
    impl<F: Fn(i32, i32) -> Option<u16>> TileView for Terrain<F> {
        fn tile(&self, x: i32, y: i32) -> Tile {
            match (self.0)(x, y) {
                Some(b) if terrustia_proto::tile_sets::frame_important(b) => Tile::framed(b, 0, 0),
                Some(b) => Tile::block(b),
                None => Tile::AIR,
            }
        }
    }

    const GROUND_ROW: i32 = 10;

    fn flat() -> Terrain<impl Fn(i32, i32) -> Option<u16>> {
        Terrain(|_x: i32, y: i32| (y >= GROUND_ROW).then_some(1))
    }

    /// A fighter standing on top of `GROUND_ROW`.
    fn zombie_at(tile_x: f32) -> Npc {
        let mut npc = Npc::new(3, (0.0, 0.0), 1).expect("zombie");
        npc.position = (tile_x * TILE, GROUND_ROW as f32 * TILE - npc.height());
        npc
    }

    fn player(tile_x: f32) -> Target {
        Target {
            slot: 0,
            center: (tile_x * TILE, GROUND_ROW as f32 * TILE - 20.0),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    fn peaceful() -> Conditions {
        Conditions::default()
    }

    #[test]
    fn a_fighter_accelerates_at_the_rate_the_game_uses() {
        // One tick of acceleration is exactly 0.07, and it tops out at 1.0.
        let mut z = zombie_at(100.0);
        let terrain = flat();
        update(&mut z, &terrain, Some(player(120.0)), peaceful());
        assert!((z.velocity.0 - 0.07).abs() < 1e-6, "got {}", z.velocity.0);

        for _ in 0..200 {
            update(&mut z, &terrain, Some(player(120.0)), peaceful());
        }
        assert!(
            (z.velocity.0 - 1.0).abs() < 1e-6,
            "top speed is 1.0, got {}",
            z.velocity.0
        );
    }

    #[test]
    fn a_faster_type_uses_its_own_numbers() {
        // Type 520 walks at 4.0 with an acceleration of 1.0.
        let mut n = Npc::new(520, (0.0, 0.0), 1).expect("type 520");
        n.position = (100.0 * TILE, GROUND_ROW as f32 * TILE - n.height());
        let terrain = flat();
        update(&mut n, &terrain, Some(player(140.0)), peaceful());
        assert!(
            (n.velocity.0 - 1.0).abs() < 1e-6,
            "accel 1.0, got {}",
            n.velocity.0
        );
        for _ in 0..50 {
            update(&mut n, &terrain, Some(player(140.0)), peaceful());
        }
        assert!(
            (n.velocity.0 - 4.0).abs() < 1e-6,
            "top speed 4.0, got {}",
            n.velocity.0
        );
    }

    #[test]
    fn a_fighter_steps_up_a_single_block_instead_of_jumping() {
        // One block at x = 105 on top of the ground.
        let terrain = Terrain(|x: i32, y: i32| {
            (y >= GROUND_ROW || (x == 105 && y == GROUND_ROW - 1)).then_some(1)
        });
        let mut z = zombie_at(104.0);
        z.velocity.0 = 1.0;

        let before_y = z.position.1;
        let mut jumped = false;
        for _ in 0..40 {
            update(&mut z, &terrain, Some(player(120.0)), peaceful());
            if z.velocity.1 < 0.0 {
                jumped = true;
            }
        }
        assert!(!jumped, "a one-block step should be climbed, not jumped");
        assert!(
            z.position.1 < before_y,
            "should have risen onto the step: {} -> {}",
            before_y,
            z.position.1
        );
    }

    #[test]
    fn a_fighter_jumps_a_wall_it_cannot_step_over() {
        // A three-tall wall: too high to step.
        let terrain = Terrain(|x: i32, y: i32| {
            (y >= GROUND_ROW || (x == 105 && y >= GROUND_ROW - 3)).then_some(1)
        });
        let mut z = zombie_at(104.0);
        z.velocity.0 = 1.0;

        let mut jumped = false;
        for _ in 0..40 {
            update(&mut z, &terrain, Some(player(120.0)), peaceful());
            if z.velocity.1 <= -JUMP_SPEED {
                jumped = true;
                break;
            }
        }
        assert!(jumped, "a tall wall should be jumped");
    }

    #[test]
    fn a_wandering_fighter_turns_at_a_ledge_but_a_chasing_one_walks_off() {
        // Ground runs out at x = 104, so a fighter standing there probes into open air.
        let terrain = Terrain(|x: i32, y: i32| (y >= GROUND_ROW && x <= 104).then_some(1));

        // No target: it should turn back rather than step into the void.
        let mut wanderer = zombie_at(104.0);
        wanderer.direction = 1;
        wanderer.velocity.0 = 1.0;
        update(&mut wanderer, &terrain, None, peaceful());
        assert_eq!(wanderer.direction, -1, "should refuse the drop");

        // With a target beyond it, the game walks off the edge.
        let mut chaser = zombie_at(104.0);
        chaser.direction = 1;
        chaser.velocity.0 = 1.0;
        update(&mut chaser, &terrain, Some(player(130.0)), peaceful());
        assert_eq!(
            chaser.direction, 1,
            "a chasing fighter does not stop at a ledge"
        );
    }

    /// Door directly ahead at head height.
    fn door_terrain() -> Terrain<impl Fn(i32, i32) -> Option<u16>> {
        Terrain(|x: i32, y: i32| {
            if y >= GROUND_ROW {
                Some(1)
            } else if x == 106 && y == GROUND_ROW - 2 {
                Some(DOOR)
            } else {
                None
            }
        })
    }

    #[test]
    fn a_zombie_cannot_open_a_door_on_a_normal_night() {
        // The base-defense mechanic: a closed door keeps a zombie out on an ordinary night. It
        // stands and pushes but never forces it — matching vanilla `flag28`.
        let terrain = door_terrain();
        let mut z = zombie_at(105.0);
        z.direction = 1;

        for _ in 0..400 {
            let action = update(&mut z, &terrain, Some(player(130.0)), peaceful());
            assert!(
                matches!(action, Action::None),
                "a zombie must not open or break a door on a normal night, got {action:?}"
            );
        }
    }

    #[test]
    fn on_a_blood_moon_a_zombie_opens_the_door_without_destroying_it() {
        let terrain = door_terrain();
        let mut z = zombie_at(105.0);
        z.direction = 1;
        let bloody = Conditions {
            blood_moon: true,
            ..Conditions::default()
        };

        let mut opened = false;
        for _ in 0..400 {
            match update(&mut z, &terrain, Some(player(130.0)), bloody) {
                Action::OpenDoor { .. } => {
                    opened = true;
                    break;
                }
                Action::BreakDoor { .. } => {
                    panic!("a zombie opens the door, it does not destroy it")
                }
                _ => {}
            }
        }
        assert!(opened, "on a blood moon a zombie forces the door open");
    }

    /// BA3-04, fail-then-pass: a For-the-Worthy world keeps doors shut through a blood moon.
    ///
    /// `flag28` is `((!Main.bloodMoon || Main.getGoodWorld) && !graveyard) & isOpener`
    /// (`NPC.cs:60601`), and this port dropped the `getGoodWorld` term, so a zombie on such a seed
    /// forced the door exactly as it does on any other world.
    #[test]
    fn a_for_the_worthy_seed_keeps_a_zombie_out_on_a_blood_moon() {
        let terrain = door_terrain();
        let mut z = zombie_at(105.0);
        z.direction = 1;
        let bloody_but_good = Conditions {
            blood_moon: true,
            get_good_world: true,
            ..Conditions::default()
        };

        for _ in 0..400 {
            let action = update(&mut z, &terrain, Some(player(130.0)), bloody_but_good);
            assert!(
                matches!(action, Action::None),
                "getGoodWorld takes the blood moon back out of flag28, got {action:?}"
            );
        }
    }

    #[test]
    fn door_damage_needs_two_pushes_to_break_and_a_gate_needs_five() {
        // 5 damage a push against a threshold of 10; a tall gate does 2.
        assert_eq!(DOOR_BREAK_THRESHOLD / DOOR_DAMAGE, 2.0);
        assert_eq!(DOOR_BREAK_THRESHOLD / TALL_GATE_DAMAGE, 5.0);
    }

    #[test]
    fn a_fighter_ignores_a_door_when_it_has_nobody_to_chase() {
        let terrain = Terrain(|x: i32, y: i32| {
            if y >= GROUND_ROW {
                Some(1)
            } else if x == 106 && y == GROUND_ROW - 2 {
                Some(DOOR)
            } else {
                None
            }
        });
        let mut z = zombie_at(105.0);
        z.direction = 1;
        for _ in 0..200 {
            assert_eq!(
                update(&mut z, &terrain, None, peaceful()),
                Action::None,
                "no target means no reason to work at the door"
            );
        }
    }

    #[test]
    fn the_live_server_geometry_reaches_the_door() {
        // Exactly what the integration test builds: corridor air above row 320, stone below,
        // a three-tile door at x = 405, and a zombie walking in from the right.
        let terrain = Terrain(|x: i32, y: i32| {
            if !(380..430).contains(&x) {
                return Some(1);
            }
            if (317..320).contains(&y) && x == 405 {
                return Some(DOOR);
            }
            if y >= 320 { Some(1) } else { None }
        });

        let mut z = Npc::new(3, (0.0, 0.0), 1).unwrap();
        // Standing on row 320, immediately right of the door.
        z.position = (406.0 * TILE, 320.0 * TILE - z.height());
        z.direction = -1;

        // On a blood moon, so the door is actually forced — this test is about the probe geometry
        // reaching the door, and a zombie only produces a door action once it can force it (a normal
        // night correctly leaves it holding back with no action to observe).
        let bloody = Conditions {
            blood_moon: true,
            ..Conditions::default()
        };
        let mut action = Action::None;
        for _ in 0..200 {
            action = update(
                &mut z,
                &terrain,
                Some(Target {
                    slot: 0,
                    center: (396.0 * TILE, 318.0 * TILE),
                    velocity: (0.0, 0.0),
                    alive: true,
                }),
                bloody,
            );
            if action != Action::None {
                break;
            }
        }
        assert!(
            matches!(action, Action::OpenDoor { .. }),
            "a zombie beside a door should work at it, got {action:?} (ai2={}, x={})",
            z.ai[2],
            z.position.0 / TILE
        );
    }

    #[test]
    fn a_closed_door_actually_stops_a_walking_fighter() {
        // Same corridor as the live test: air above row 320, stone below, door at x = 405.
        let terrain = Terrain(|x: i32, y: i32| {
            if (317..320).contains(&y) && x == 405 {
                return Some(DOOR);
            }
            if y >= 320 { Some(1) } else { None }
        });

        let mut z = Npc::new(3, (0.0, 0.0), 1).unwrap();
        z.position = (412.0 * TILE, 320.0 * TILE - z.height());
        z.direction = -1;

        let target = Target {
            slot: 0,
            center: (396.0 * TILE, 318.0 * TILE),
            velocity: (0.0, 0.0),
            alive: true,
        };
        for _ in 0..600 {
            update(&mut z, &terrain, Some(target), peaceful());
            crate::game::npc::step_physics(&mut z, &terrain);
        }
        assert!(
            z.position.0 / TILE > 405.0,
            "the door should have stopped it, but it reached tile {}",
            z.position.0 / TILE
        );
    }

    #[test]
    fn friction_only_applies_while_on_the_ground() {
        let mut z = zombie_at(100.0);
        z.velocity = (5.0, 0.0); // well over top speed, standing still vertically
        let terrain = flat();
        update(&mut z, &terrain, Some(player(120.0)), peaceful());
        assert!(z.velocity.0 < 5.0, "should be braking on the ground");

        let mut airborne = zombie_at(100.0);
        airborne.velocity = (5.0, -3.0); // mid-jump
        update(&mut airborne, &terrain, Some(player(120.0)), peaceful());
        assert_eq!(airborne.velocity.0, 5.0, "no braking in the air");
    }
}
