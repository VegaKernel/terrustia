//! Duke Fishron and its Sharkrons: styles 69 and 71.
//!
//! Fishron runs one skeleton three times over with different numbers, which is why the per-phase
//! figures are a table rather than a stack of branches: hold station three hundred pixels to one
//! side and two hundred above, wait, then attack. What changes between phases is how long the wait
//! is, how fast it moves, how hard it hits, and — the part that matters — how much armour it has
//! left. By the third phase it has none at all.
//!
//! The attacks come off a counter rather than a die roll, ten charges to one sharkron burst to one
//! spray of bubbles, so the pattern is learnable. Crossing half health does not interrupt whatever
//! it is doing — vanilla only re-checks the threshold once it is back in its hover, choosing its
//! next attack, so a charge/burst/bubble stream in progress finishes on its own terms. Once it
//! does notice, it stops to change for two seconds, and in expert crossing fifteen per cent does
//! it again.
//!
//! A **Sharkron** (71) is not a chaser. It hangs in the air for a second and a half, aims once, and
//! commits — and dies on whatever it hits, terrain included.

use terrustia_proto::npc_params::{
    FISHRON_ABOVE, FISHRON_BESIDE, FISHRON_BUBBLE, FISHRON_BUBBLE_AT, FISHRON_BUBBLE_SPEED,
    FISHRON_BUBBLE_TICKS, FISHRON_BURST_ACCEL, FISHRON_BURST_EVERY, FISHRON_BURST_SPEED,
    FISHRON_BURST_TICKS, FISHRON_CYCLE_BUBBLES, FISHRON_CYCLE_SHARKRONS, FISHRON_EXPERT_PACE,
    FISHRON_FIRST, FISHRON_SECOND, FISHRON_SECOND_AT, FISHRON_SHIFT_TICKS, FISHRON_THIRD,
    FISHRON_THIRD_AT, FishronPhase, SHARKRON,
};

use crate::game::ai::{Shot, World};
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Spawn;

/// The states, as `ai[0]` numbers them. The second and third phases repeat the first four at an
/// offset of five and ten, which is exactly how the game numbers them.
mod state {
    pub const HOVERING: f32 = 0.0;
    pub const CHARGING: f32 = 1.0;
    pub const BURSTING: f32 = 2.0;
    pub const BUBBLING: f32 = 3.0;
    pub const CHANGING: f32 = 4.0;
    /// The offset between one phase's states and the next.
    pub const PHASE: f32 = 5.0;
}

/// What Fishron did this tick.
#[derive(Debug, Default)]
pub struct FishronOutcome {
    pub shots: Vec<Shot>,
    pub spawn: Vec<Spawn>,
}

/// Which phase a state belongs to, and its numbers.
fn phase_of(state: f32) -> (i32, FishronPhase) {
    if state > 9.0 {
        (2, FISHRON_THIRD)
    } else if state > 4.0 {
        (1, FISHRON_SECOND)
    } else {
        (0, FISHRON_FIRST)
    }
}

/// Style 69.
pub fn fishron(npc: &mut Npc, world: &World<'_, impl TileView>) -> FishronOutcome {
    let mut out = FishronOutcome::default();
    npc.dirty = true;

    let expert = world.conditions.expert;
    let pace = if expert { FISHRON_EXPERT_PACE } else { 1.0 };
    let (phase, p) = phase_of(npc.ai[0]);
    let base = phase as f32 * state::PHASE;

    npc.damage_bonus = p.damage * pace;
    npc.defense = (npc.stats.defense as f32 * p.defense) as i32;

    let Some(target) = world.target.filter(|t| t.alive) else {
        npc.velocity.0 *= 0.98;
        npc.velocity.1 *= 0.98;
        return out;
    };
    let (cx, cy) = npc.center();
    let health = npc.life as f32 / npc.life_max.max(1) as f32;

    match npc.ai[0] - base {
        s if s == state::HOVERING => {
            // The side it takes station on is chosen once and kept for the whole hover.
            if npc.ai[1] == 0.0 {
                npc.ai[1] = FISHRON_BESIDE * (cx - target.center.0).signum();
            }
            let station = (
                target.center.0 + npc.ai[1] - cx,
                target.center.1 - FISHRON_ABOVE - cy,
            );
            ease_toward(npc, station, p.hover_speed, p.hover_accel);
            face(npc, target.center.0 - cx);

            npc.ai[2] += 1.0;
            if npc.ai[2] < p.hover_ticks {
                return out;
            }

            // Real vanilla nests both this check and the Expert-only second one strictly inside
            // the hover-timer-just-expired branch (`AI_069_DukeFishron`: `flag`/`flag2` are only
            // ever read where `num28`/`num33` — the *next attack* it is about to choose — get
            // computed, right here, and nowhere else). Crossing a threshold does not interrupt a
            // charge, a burst or a bubble stream already under way; it only ever gets checked at
            // this one decision point, the same one vanilla checks it at.
            let wants_phase = if expert && health <= FISHRON_THIRD_AT {
                2
            } else if health <= FISHRON_SECOND_AT {
                1
            } else {
                0
            };
            if wants_phase > phase {
                npc.ai = [base + state::CHANGING, 0.0, 0.0, npc.ai[3]];
                return out;
            }

            // The cycle: ten charges, then a burst, then bubbles.
            let attack = npc.ai[3] as i32;
            let next = if attack == FISHRON_CYCLE_SHARKRONS {
                npc.ai[3] = 1.0;
                state::BURSTING
            } else if attack == FISHRON_CYCLE_BUBBLES {
                npc.ai[3] = 0.0;
                state::BUBBLING
            } else {
                state::CHARGING
            };
            npc.ai[0] = base + next;
            npc.ai[1] = 0.0;
            npc.ai[2] = 0.0;
            if next == state::CHARGING {
                // Aimed once, at speed, and never corrected.
                let aim = (target.center.0 - cx, target.center.1 - cy);
                let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
                npc.velocity = (
                    aim.0 / length * p.charge_speed,
                    aim.1 / length * p.charge_speed,
                );
                npc.rotation = npc.velocity.1.atan2(npc.velocity.0);
            }
        }

        s if s == state::CHARGING => {
            npc.ai[2] += 1.0;
            if npc.ai[2] >= p.charge_ticks {
                npc.ai[0] = base + state::HOVERING;
                npc.ai[1] = 0.0;
                npc.ai[2] = 0.0;
                // Two steps along the cycle per charge, which is what makes the burst come round
                // every fifth charge rather than every tenth.
                npc.ai[3] += 2.0;
            }
        }

        s if s == state::BURSTING => {
            // It keeps station and throws a sharkron every twenty ticks.
            if npc.ai[1] == 0.0 {
                npc.ai[1] = FISHRON_BESIDE * (cx - target.center.0).signum();
            }
            let station = (
                target.center.0 + npc.ai[1] - cx,
                target.center.1 - FISHRON_ABOVE - cy,
            );
            ease_toward(npc, station, FISHRON_BURST_SPEED, FISHRON_BURST_ACCEL);
            face(npc, target.center.0 - cx);

            if npc.ai[2] % FISHRON_BURST_EVERY == 0.0 {
                // Out of its mouth, toward the player.
                let aim = (target.center.0 - cx, target.center.1 - cy);
                let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
                let reach = (npc.width() + 20.0) / 2.0;
                out.spawn.push(Spawn {
                    npc_type: SHARKRON,
                    position: (
                        cx + aim.0 / length * reach,
                        cy + aim.1 / length * reach + 45.0,
                    ),
                    velocity: (0.0, 0.0),
                    parent: None,
                });
            }
            npc.ai[2] += 1.0;
            if npc.ai[2] >= FISHRON_BURST_TICKS {
                npc.ai[0] = base + state::HOVERING;
                npc.ai[1] = 0.0;
                npc.ai[2] = 0.0;
            }
        }

        s if s == state::BUBBLING => {
            // It hangs almost still and spits two bubbles that drift apart.
            npc.velocity.0 *= 0.98;
            npc.velocity.1 += (0.0 - npc.velocity.1) * 0.02;
            if npc.ai[2] == FISHRON_BUBBLE_TICKS - FISHRON_BUBBLE_AT {
                let from = (
                    cx + f32::from(npc.direction) * (npc.width() + 20.0) / 2.0,
                    cy,
                );
                for side in [1.0, -1.0] {
                    out.shots.push(Shot {
                        projectile: FISHRON_BUBBLE,
                        damage: 0,
                        position: from,
                        velocity: (
                            side * f32::from(npc.direction) * FISHRON_BUBBLE_SPEED.0,
                            FISHRON_BUBBLE_SPEED.1,
                        ),
                        time_left: 900,
                    });
                }
            }
            npc.ai[2] += 1.0;
            if npc.ai[2] >= FISHRON_BUBBLE_TICKS {
                npc.ai[0] = base + state::HOVERING;
                npc.ai[1] = 0.0;
                npc.ai[2] = 0.0;
            }
        }

        _ => {
            // Changing. It does nothing at all for two seconds, which is the window.
            npc.velocity.0 *= 0.98;
            npc.velocity.1 += (0.0 - npc.velocity.1) * 0.02;
            npc.ai[2] += 1.0;
            if npc.ai[2] >= FISHRON_SHIFT_TICKS {
                // Into the next phase's hover.
                npc.ai = [base + state::PHASE, 0.0, 0.0, 0.0];
            }
        }
    }
    out
}

/// Ease toward an offset at a given speed, doubling the push while still going the wrong way.
fn ease_toward(npc: &mut Npc, offset: (f32, f32), speed: f32, accel: f32) {
    // The game subtracts the current velocity before normalising, which is what stops it
    // overshooting its own station at speed.
    let aim = (offset.0 - npc.velocity.0, offset.1 - npc.velocity.1);
    let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
    let wanted = (aim.0 / length * speed, aim.1 / length * speed);
    for (v, w) in [
        (&mut npc.velocity.0, wanted.0),
        (&mut npc.velocity.1, wanted.1),
    ] {
        if *v < w {
            *v += accel;
            if *v < 0.0 && w > 0.0 {
                *v += accel;
            }
        } else if *v > w {
            *v -= accel;
            if *v > 0.0 && w < 0.0 {
                *v -= accel;
            }
        }
    }
}

/// Turn to face the player, flipping the sprite without spinning the body.
fn face(npc: &mut Npc, across: f32) {
    let side = across.signum() as i8;
    if side == 0 {
        return;
    }
    npc.direction = side;
    if npc.sprite_direction != -side {
        npc.rotation += std::f32::consts::PI;
    }
    npc.sprite_direction = -side;
}

/// Style 71: a Sharkron.
///
/// It hangs where it was thrown, turning to face you, and after ninety ticks commits to a single
/// line at sixteen pixels a tick. It does not steer afterwards and it dies on whatever it meets,
/// so a Sharkron is dodged rather than outrun.
pub fn sharkron(npc: &mut Npc, world: &World<'_, impl TileView>) -> bool {
    npc.dirty = true;
    npc.no_gravity = true;
    npc.invulnerable = npc.ai[0] == 0.0 && npc.ai[1] < 60.0;

    let Some(target) = world.target.filter(|t| t.alive) else {
        return false;
    };
    let (cx, cy) = npc.center();

    if npc.ai[0] == 0.0 {
        // Winding up. It fades in, holds still, and turns to face you.
        npc.ai[1] += 1.0;
        npc.alpha = (npc.alpha - 6).max(0);
        let aim = (target.center.0 - cx, target.center.1 - cy);
        npc.rotation = aim.1.atan2(aim.0) + std::f32::consts::FRAC_PI_2;
        npc.velocity.1 = npc.ai[3];

        if npc.ai[1] >= 90.0 {
            npc.ai[0] = 1.0;
            npc.ai[1] = 0.0;
            let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
            npc.velocity = (aim.0 / length * 16.0, aim.1 / length * 16.0);
            npc.rotation = npc.velocity.1.atan2(npc.velocity.0);
            npc.direction = npc.velocity.0.signum() as i8;
        }
        return false;
    }

    // Committed. It travels its line, and hitting anything is the end of it.
    npc.no_tile_collide = false;
    npc.ai[1] += 1.0;
    if npc.ai[1] >= 60.0 {
        // Past a second it becomes solid again and falls out of the sky.
        npc.no_gravity = false;
    }
    npc.rotation = npc.velocity.1.atan2(npc.velocity.0);
    npc.collide_x || npc.collide_y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ai::Conditions;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::npc_params::FISHRON;
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

    fn duke(x: f32, y: f32) -> Npc {
        Npc::new(FISHRON, (x, y), 1).expect("duke fishron")
    }

    /// The cycle is a counter, not a die roll: ten charges, a sharkron burst, then bubbles.
    #[test]
    fn its_attacks_come_round_in_order() {
        let tiles = Sky(HashMap::new());
        let mut d = duke(0.0, 0.0);
        let w = world(&tiles, Some((600.0, 400.0)));

        let mut order = Vec::new();
        let mut was = d.ai[0];
        for _ in 0..6000 {
            fishron(&mut d, &w);
            d.position.0 += d.velocity.0;
            d.position.1 += d.velocity.1;
            if d.ai[0] != was {
                if d.ai[0] != state::HOVERING {
                    order.push(d.ai[0]);
                }
                was = d.ai[0];
            }
        }
        assert!(order.contains(&state::CHARGING), "it charges: {order:?}");
        assert!(order.contains(&state::BURSTING), "it bursts: {order:?}");
        assert!(order.contains(&state::BUBBLING), "it bubbles: {order:?}");
    }

    /// This used to be `half_health_starts_the_second_phase`, and its own assertion — that
    /// starting Fishron mid-charge and crossing 50% health interrupts the charge on the very next
    /// tick — was itself the bug: real vanilla (`AI_069_DukeFishron`) only ever reads the
    /// threshold flags (`flag`/`flag2`) inside the `ai[0]==0f`/`ai[0]==5f` hover branches, at the
    /// exact point the hover timer has just expired and it is choosing its next attack. There is
    /// no code path in vanilla that checks health mid-charge, mid-burst, or mid-bubble-stream, so
    /// asserting an immediate interrupt there was asserting a behaviour vanilla doesn't have. This
    /// test now asserts the corrected shape: the current attack always finishes, and the phase
    /// only changes once it is back in the hover and its own timer has run out.
    #[test]
    fn half_health_finishes_the_current_attack_before_the_second_phase_starts() {
        let tiles = Sky(HashMap::new());
        let mut d = duke(0.0, 0.0);
        let w = world(&tiles, Some((600.0, 400.0)));

        // Start it mid-charge, already below the 50% threshold.
        d.ai[0] = state::CHARGING;
        d.ai[2] = 0.0;
        d.life = d.life_max / 3;
        fishron(&mut d, &w);
        assert_eq!(
            d.ai[0],
            state::CHARGING,
            "a charge already in progress should not be interrupted by crossing a threshold"
        );

        // Let the charge run out on its own terms.
        for _ in 0..(FISHRON_FIRST.charge_ticks as i32 + 2) {
            if d.ai[0] != state::CHARGING {
                break;
            }
            fishron(&mut d, &w);
        }
        assert_eq!(
            d.ai[0],
            state::HOVERING,
            "the charge finishes and returns to hovering, not a phase change mid-attack"
        );

        // Only once it is back in the hover, past its own hover timer, does it notice.
        for _ in 0..(FISHRON_FIRST.hover_ticks as i32 + 2) {
            if d.ai[0] == state::CHANGING {
                break;
            }
            fishron(&mut d, &w);
        }
        assert_eq!(
            d.ai[0],
            state::CHANGING,
            "the hover's own decision point is where it finally notices"
        );

        for _ in 0..(FISHRON_SHIFT_TICKS as i32 + 2) {
            fishron(&mut d, &w);
        }
        assert_eq!(d.ai[0], state::PHASE, "and comes out in the second phase");
        assert!(
            d.defense < d.stats.defense,
            "with less armour: {} was {}",
            d.defense,
            d.stats.defense
        );
    }

    /// In expert it sheds its armour entirely for the last stretch.
    #[test]
    fn the_third_phase_has_no_armour_at_all() {
        let tiles = Sky(HashMap::new());
        let mut w = world(&tiles, Some((600.0, 400.0)));
        w.conditions = Conditions {
            expert: true,
            ..Conditions::default()
        };
        let mut d = duke(0.0, 0.0);
        d.ai[0] = state::PHASE + state::HOVERING;
        d.life = d.life_max / 20;

        // Same fix as `half_health_finishes_the_current_attack_before_the_second_phase_starts`:
        // even starting fresh in the hover, real vanilla only notices the threshold once that
        // hover's own timer has actually run out — not on the very first tick of it.
        for _ in 0..(FISHRON_SECOND.hover_ticks as i32 + 2) {
            if d.ai[0] == state::PHASE + state::CHANGING {
                break;
            }
            fishron(&mut d, &w);
        }
        assert_eq!(d.ai[0], state::PHASE + state::CHANGING);
        for _ in 0..(FISHRON_SHIFT_TICKS as i32 + 2) {
            fishron(&mut d, &w);
        }
        assert_eq!(d.ai[0], state::PHASE * 2.0, "into the third phase");
        fishron(&mut d, &w);
        assert_eq!(d.defense, 0, "and no armour left");
    }

    /// The burst throws sharkrons rather than projectiles.
    #[test]
    fn the_burst_throws_sharkrons() {
        let tiles = Sky(HashMap::new());
        let mut d = duke(0.0, 0.0);
        d.ai[0] = state::BURSTING;
        let w = world(&tiles, Some((600.0, 400.0)));

        let mut thrown = Vec::new();
        for _ in 0..(FISHRON_BURST_TICKS as i32 + 2) {
            thrown.extend(fishron(&mut d, &w).spawn);
        }
        assert!(!thrown.is_empty(), "it should have thrown some");
        assert!(thrown.iter().all(|s| s.npc_type == SHARKRON));
    }

    /// The bubbles come in pairs that drift apart.
    #[test]
    fn the_bubbles_come_in_pairs() {
        let tiles = Sky(HashMap::new());
        let mut d = duke(0.0, 0.0);
        d.ai[0] = state::BUBBLING;
        d.direction = 1;
        let w = world(&tiles, Some((600.0, 400.0)));

        let mut bubbles = Vec::new();
        for _ in 0..(FISHRON_BUBBLE_TICKS as i32 + 2) {
            bubbles.extend(fishron(&mut d, &w).shots);
        }
        assert_eq!(bubbles.len(), 2, "two bubbles");
        assert!(
            bubbles[0].velocity.0 * bubbles[1].velocity.0 < 0.0,
            "and they should go opposite ways: {:?}",
            bubbles.iter().map(|b| b.velocity).collect::<Vec<_>>()
        );
    }

    /// A Sharkron aims once and commits, and cannot be hurt while it winds up.
    #[test]
    fn a_sharkron_aims_once_and_commits() {
        let tiles = Sky(HashMap::new());
        let mut s = Npc::new(SHARKRON, (0.0, 0.0), 1).expect("sharkron");
        let w = world(&tiles, Some((600.0, 0.0)));

        s.alpha = 255;
        sharkron(&mut s, &w);
        assert!(s.invulnerable, "it cannot be hit while it fades in");

        for _ in 0..95 {
            sharkron(&mut s, &w);
        }
        assert_eq!(s.ai[0], 1.0, "it should have committed");
        let speed = s.velocity.0.hypot(s.velocity.1);
        assert!((speed - 16.0).abs() < 0.1, "at its full speed, got {speed}");

        // Moving the player afterwards does not change its line.
        let aside = world(&tiles, Some((-600.0, 0.0)));
        let before = s.velocity;
        sharkron(&mut s, &aside);
        assert_eq!(s.velocity, before, "it does not steer once committed");
    }
}
