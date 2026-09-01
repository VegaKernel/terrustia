//! Style 4 — the Eye of Cthulhu.
//!
//! One fight, twice. In both halves it hovers above you, then throws itself at you three times, and
//! goes back to hovering — and the whole difference between the halves is the numbers.
//!
//! The **first form** hovers two hundred pixels up for ten seconds and dashes at six pixels a tick,
//! spitting a Servant of Cthulhu roughly every two seconds while it hangs there. At half health it
//! **splits**: a hundred ticks of spinning faster and faster in place, then a hundred more, and it
//! comes out the other side with its shell gone.
//!
//! The **second form** hovers only a hundred and twenty pixels up, for a third as long, dashes
//! faster and recovers quicker, has no defence left at all, and stops summoning. It is the same
//! pattern played at speed, which is why the fight feels like it doubles rather than changes.
//!
//! Dawn ends it: daylight sends the Eye straight up and out of the world.
//!
//! Expert Mode changes far more than the numbers, and every one of these was missing here until
//! EYE-1..EYE-7. First form: it opens the split at 65% health rather than 50%, hovers faster and
//! far more briefly, drops the "servants only from above" rule, calls a servant much more often,
//! dashes at seven rather than six, ends each dash a third sooner and drags harder while it does.
//! The split itself is not free time: it throws a servant on a random bearing every twentieth tick
//! of the spin, ten across the transformation. Second form: the hover gains a pixel of speed and
//! 0.05 of acceleration at each of four hundred, six hundred and eight hundred pixels, so running
//! away no longer works; each dash of a set is faster than the last; and below half health the
//! whole dash-set pattern is replaced by a lunge cycle, which below 12% opens by dropping six
//! hundred pixels beneath you first, and below 4% scatters twice as wide and comes twice as fast.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    EYE_BACKOFF_ACCEL, EYE_BACKOFF_BELOW, EYE_BACKOFF_LUNGE_BONUS, EYE_BACKOFF_SPEED,
    EYE_BACKOFF_TICKS, EYE_DASH_DRAG_FIRST, EYE_DASH_DRAG_FIRST_EXPERT, EYE_DASH_DRAG_SECOND,
    EYE_DASH_DRAG_SECOND_EXPERT, EYE_DASH_DRIVE, EYE_DASH_DRIVE_SECOND_EXPERT, EYE_DASH_FIRST,
    EYE_DASH_FIRST_EXPERT, EYE_DASH_SECOND, EYE_DASH_SECOND_EXPERT_STEPS, EYE_DASH_TICKS_FIRST,
    EYE_DASH_TICKS_FIRST_EXPERT, EYE_DASH_TICKS_SECOND, EYE_DASH_TICKS_SECOND_EXPERT, EYE_DASHES,
    EYE_HOVER_FIRST, EYE_HOVER_FIRST_EXPERT, EYE_HOVER_SECOND, EYE_HOVER_SECOND_EXPERT_ACCEL_STEP,
    EYE_HOVER_SECOND_EXPERT_SPEED_STEP, EYE_HOVER_SECOND_EXPERT_STEPS, EYE_HOVER_TICKS_FIRST,
    EYE_HOVER_TICKS_FIRST_EXPERT, EYE_HOVER_TICKS_SECOND, EYE_LUNGE_AT, EYE_LUNGE_AT_HOVER,
    EYE_LUNGE_BREAK_AT, EYE_LUNGE_DRAG, EYE_LUNGE_HEAD_START, EYE_LUNGE_HOLD,
    EYE_LUNGE_HOLD_DESPERATE, EYE_LUNGE_LEAD, EYE_LUNGE_LEAD_DESPERATE, EYE_LUNGE_LEAD_FROM_BELOW,
    EYE_LUNGE_NUDGE, EYE_LUNGE_NUDGE_DESPERATE, EYE_LUNGE_RECOVER, EYE_LUNGE_SPEED,
    EYE_LUNGE_SPEED_FROM_BELOW, EYE_LUNGE_STALL_RANGE, EYE_LUNGE_STRETCH, EYE_LUNGE_SWAP_RANGE,
    EYE_LUNGES, EYE_SECOND_FORM_DAMAGE, EYE_SECOND_FORM_DAMAGE_EXPERT,
    EYE_SECOND_FORM_DAMAGE_EXPERT_LOW, EYE_SECOND_FORM_DEFENSE, EYE_SECOND_FORM_DEFENSE_LOW,
    EYE_SECOND_FORM_DEFENSE_LOW_AT, EYE_SECOND_FORM_DEFENSE_VERY_LOW,
    EYE_SECOND_FORM_DEFENSE_VERY_LOW_AT, EYE_SERVANT_EVERY, EYE_SERVANT_EVERY_EXPERT,
    EYE_SERVANT_RANGE, EYE_SERVANT_SPEED, EYE_SERVANT_SPEED_EXPERT, EYE_SERVANT_THROW,
    EYE_SPIN_MAX, EYE_SPIN_RAMP, EYE_SPLIT_AT, EYE_SPLIT_AT_EXPERT, EYE_SPLIT_SERVANT_EVERY,
    EYE_SPLIT_SERVANT_SPEED, EYE_SPLIT_SERVANT_SPREAD, EYE_SPLIT_TICKS, SERVANT_OF_CTHULHU,
};

use crate::game::ai::{PLAYER_HEIGHT, PLAYER_WIDTH, World};
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Spawn;

/// Which form it is in, as `ai[0]` records it. One and two are the two halves of the split.
const FIRST_FORM: f32 = 0.0;
const SECOND_FORM: f32 = 3.0;

/// The three steps of a dash set, as `ai[1]` records them, and the three more the Expert second
/// form's lunge cycle adds (`NPC.cs:20636`, `:20746`, `:20800`).
const HOVERING: f32 = 0.0;
const LAUNCHING: f32 = 1.0;
const DASHING: f32 = 2.0;
const AIMING: f32 = 3.0;
const LUNGING: f32 = 4.0;
const BACKING_OFF: f32 = 5.0;

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

/// Drive the Eye of Cthulhu for a tick.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, rng: &mut SmallRng) -> Vec<Spawn> {
    let mut summoned = Vec::new();
    let expert = world.conditions.expert;
    let Some(target) = world.target else {
        npc.velocity.1 -= 0.04;
        npc.time_left = npc.time_left.min(10);
        return summoned;
    };

    // Dawn, or nobody left standing, and it simply leaves.
    if world.conditions.day || !target.alive {
        npc.velocity.1 -= 0.04;
        npc.time_left = npc.time_left.min(10);
        npc.dirty = true;
        return summoned;
    }

    let centre = (
        npc.position.0 + npc.width() * 0.5,
        npc.position.1 + npc.height() * 0.5,
    );
    let their_middle = (target.center.0, target.center.1);

    let health = npc.life as f32 / npc.life_max.max(1) as f32;

    // The two halves of the split: it spins in place, faster and faster, and comes out changed.
    if npc.ai[0] == 1.0 || npc.ai[0] == 2.0 {
        npc.ai[2] = (npc.ai[2] + EYE_SPIN_RAMP).min(EYE_SPIN_MAX);
        npc.rotation += npc.ai[2];
        npc.ai[1] += 1.0;
        // EYE-4: an Expert split is not two hundred free ticks. Every twentieth tick throws a
        // servant out on a random bearing (`NPC.cs:20363-20400`), ten across the transformation.
        if expert && npc.ai[1] % EYE_SPLIT_SERVANT_EVERY == 0.0 {
            let spread = EYE_SPLIT_SERVANT_SPREAD;
            let mut throw = (
                rng.random_range(-spread..spread) as f32,
                rng.random_range(-spread..spread) as f32,
            );
            let length = throw.0.hypot(throw.1).max(f32::MIN_POSITIVE);
            throw = (
                throw.0 / length * EYE_SPLIT_SERVANT_SPEED,
                throw.1 / length * EYE_SPLIT_SERVANT_SPEED,
            );
            summoned.push(Spawn {
                npc_type: SERVANT_OF_CTHULHU,
                position: (
                    centre.0 + throw.0 * EYE_SERVANT_THROW,
                    centre.1 + throw.1 * EYE_SERVANT_THROW,
                ),
                velocity: throw,
                parent: None,
                ai: [None; 4],
            });
        }
        if npc.ai[1] >= EYE_SPLIT_TICKS {
            npc.ai[0] += 1.0;
            npc.ai[1] = 0.0;
            if npc.ai[0] == SECOND_FORM {
                npc.ai[2] = 0.0;
            }
            npc.dirty = true;
        }
        npc.velocity.0 *= 0.98;
        npc.velocity.1 *= 0.98;
        if npc.velocity.0.abs() < 0.1 {
            npc.velocity.0 = 0.0;
        }
        if npc.velocity.1.abs() < 0.1 {
            npc.velocity.1 = 0.0;
        }
        npc.dirty = true;
        return summoned;
    }

    let second = npc.ai[0] == SECOND_FORM;
    // Vanilla's `flag2` and `flag3` (`NPC.cs:20012-20021`): the last two health bands, which only
    // exist in Expert and which only the second form can reach.
    let frenzied = second && expert && health < EYE_SECOND_FORM_DEFENSE_LOW_AT;
    let desperate = second && expert && health < EYE_SECOND_FORM_DEFENSE_VERY_LOW_AT;

    // Expert Mode hovers faster and much more briefly in the first form. The second form's own
    // hover keeps its numbers here and gains a distance term inside the hover itself (EYE-5).
    let (lift, hover_speed, hover_accel) = if second {
        EYE_HOVER_SECOND
    } else if expert {
        (
            EYE_HOVER_FIRST.0,
            EYE_HOVER_FIRST_EXPERT.0,
            EYE_HOVER_FIRST_EXPERT.1,
        )
    } else {
        EYE_HOVER_FIRST
    };
    let hover_for = if second {
        EYE_HOVER_TICKS_SECOND
    } else if expert {
        EYE_HOVER_TICKS_FIRST_EXPERT
    } else {
        EYE_HOVER_TICKS_FIRST
    };
    // EYE-1: the first form's dash is 7 in Expert, not 6 (`NPC.cs:20252-20256`). The second form's
    // own Expert multipliers are per-dash and applied at the launch below.
    let dash_speed = if second {
        EYE_DASH_SECOND
    } else if expert {
        EYE_DASH_FIRST_EXPERT
    } else {
        EYE_DASH_FIRST
    };
    // EYE-2/EYE-6: both forms end a dash sooner in Expert (`NPC.cs:20298-20302`, `:20608-20612`).
    let dash_for = match (second, expert) {
        (true, true) => EYE_DASH_TICKS_SECOND_EXPERT,
        (true, false) => EYE_DASH_TICKS_SECOND,
        (false, true) => EYE_DASH_TICKS_FIRST_EXPERT,
        (false, false) => EYE_DASH_TICKS_FIRST,
    };
    // ...and the second form drives 25% longer before it starts braking (`NPC.cs:20582-20587`).
    let drive = if second && expert {
        EYE_DASH_DRIVE_SECOND_EXPERT
    } else {
        EYE_DASH_DRIVE
    };
    // EYE-3: Expert multiplies the drag *on top of* the classic figure rather than replacing it
    // (`NPC.cs:20276-20280`, `:20590-20594`), so a spent Expert dash bleeds off faster.
    let mut drag = if second {
        EYE_DASH_DRAG_SECOND
    } else {
        EYE_DASH_DRAG_FIRST
    };
    if expert {
        drag *= if second {
            EYE_DASH_DRAG_SECOND_EXPERT
        } else {
            EYE_DASH_DRAG_FIRST_EXPERT
        };
    }

    // Its shell is gone, so nothing softens a hit any more — and Expert Mode strips even more
    // armour once it is nearly dead. Both write the live `defense`/`damage_bonus` fields combat
    // actually reads, not the type's own baseline stats.
    if second {
        npc.defense = EYE_SECOND_FORM_DEFENSE;
        // `NPC.cs:20446-20461`: the classic figure is 23, but Expert lerps it *down* to 18 before
        // the difficulty multiplier doubles it, and only the near-dead `flag3` band pushes it back
        // up to 20. `GetAttackDamage_LerpBetweenFinalValues` clamps outside classic..expert, so
        // master reads the same 18 as expert and journey the same 23 as classic; the following
        // `GetAttackDamage_CappedAtMaster` is then the ordinary difficulty scaling for every mode a
        // world can actually be in.
        let mut normal = EYE_SECOND_FORM_DAMAGE;
        if expert {
            normal = EYE_SECOND_FORM_DAMAGE_EXPERT;
            if desperate {
                npc.defense = EYE_SECOND_FORM_DEFENSE_VERY_LOW;
                normal = EYE_SECOND_FORM_DAMAGE_EXPERT_LOW;
            } else if frenzied {
                npc.defense = EYE_SECOND_FORM_DEFENSE_LOW;
            }
        }
        npc.set_contact_damage(normal);
        // EYE-7: in the frenzy band the hover is not entered at all, it is swapped for the
        // back-off (`NPC.cs:20464-20467`).
        //
        // Disclosed narrowing: vanilla's hover branch carries a second escape for the `flag3` band
        // (`NPC.cs:20544-20551`, `ai[1] = 3; ai[3] -= 1000`), and it can never run. `flag3` implies
        // `flag2`, so this line has already moved `ai[1]` off the hover before that branch is
        // reached. It is left out here rather than transcribed as code nothing can enter.
        if frenzied && npc.ai[1] == HOVERING {
            npc.ai[1] = BACKING_OFF;
        }
    }

    if npc.ai[1] == HOVERING {
        // Hanging above them, easing into position.
        let (dx, dy) = (their_middle.0 - centre.0, their_middle.1 - lift - centre.1);
        let reach = (dx * dx + dy * dy).sqrt().max(f32::MIN_POSITIVE);
        // EYE-5: an Expert second form closes the gap rather than letting you open it. One pixel a
        // tick and 0.05 of acceleration at each of four hundred, six hundred and eight hundred
        // pixels, cumulative (`NPC.cs:20476-20490`). Classic has no such term.
        let (mut speed, mut accel) = (hover_speed, hover_accel);
        if second && expert {
            for step in EYE_HOVER_SECOND_EXPERT_STEPS {
                if reach > step {
                    speed += EYE_HOVER_SECOND_EXPERT_SPEED_STEP;
                    accel += EYE_HOVER_SECOND_EXPERT_ACCEL_STEP;
                }
            }
        }
        let k = speed / reach;
        let wanted = (dx * k, dy * k);
        close_on(&mut npc.velocity.0, wanted.0, accel);
        close_on(&mut npc.velocity.1, wanted.1, accel);

        npc.ai[2] += 1.0;
        if npc.ai[2] >= hover_for {
            npc.ai[1] = LAUNCHING;
            npc.ai[2] = 0.0;
            npc.ai[3] = 0.0;
            // EYE-7: below 35% an Expert second form leaves the hover into a lunge, not a dash set
            // (`NPC.cs:20537-20540`).
            if second && expert && health < EYE_LUNGE_AT_HOVER {
                npc.ai[1] = AIMING;
            }
            npc.dirty = true;
        } else if !second {
            // The first form spits servants while it hovers, close enough and — in Normal mode
            // only — from above; Expert Mode drops the "from above" requirement and calls one
            // much more often, a little faster.
            let (dx, dy) = (their_middle.0 - centre.0, their_middle.1 - centre.1);
            let reach = (dx * dx + dy * dy).sqrt();
            let above = npc.position.1 + npc.height() < target.center.1;
            if (above || expert) && reach < EYE_SERVANT_RANGE {
                npc.ai[3] += 1.0;
                let every = if expert {
                    EYE_SERVANT_EVERY_EXPERT
                } else {
                    EYE_SERVANT_EVERY
                };
                if npc.ai[3] >= every {
                    npc.ai[3] = 0.0;
                    let speed = if expert {
                        EYE_SERVANT_SPEED_EXPERT
                    } else {
                        EYE_SERVANT_SPEED
                    };
                    let k = speed / reach.max(f32::MIN_POSITIVE);
                    let throw = (dx * k, dy * k);
                    summoned.push(Spawn {
                        npc_type: SERVANT_OF_CTHULHU,
                        // Thrown out ahead of itself rather than dropped.
                        position: (
                            centre.0 + throw.0 * EYE_SERVANT_THROW,
                            centre.1 + throw.1 * EYE_SERVANT_THROW,
                        ),
                        velocity: throw,
                        parent: None,
                        ai: [None; 4],
                    });
                    npc.dirty = true;
                }
            }
        }
    } else if npc.ai[1] == LAUNCHING {
        // One tick to commit: the whole dash is aimed here and never corrected.
        let (dx, dy) = (their_middle.0 - centre.0, their_middle.1 - centre.1);
        let reach = (dx * dx + dy * dy).sqrt().max(f32::MIN_POSITIVE);
        // EYE-6: the second form's dashes get faster within a set in Expert (`NPC.cs:20558-20565`).
        let mut speed = dash_speed;
        if second && expert {
            if npc.ai[3] == 1.0 {
                speed *= EYE_DASH_SECOND_EXPERT_STEPS[0];
            } else if npc.ai[3] == 2.0 {
                speed *= EYE_DASH_SECOND_EXPERT_STEPS[1];
            }
        }
        let k = speed / reach;
        npc.velocity = (dx * k, dy * k);
        npc.ai[1] = DASHING;
        npc.dirty = true;
    } else if npc.ai[1] == DASHING {
        npc.ai[2] += 1.0;
        if npc.ai[2] >= drive {
            npc.velocity.0 *= drag;
            npc.velocity.1 *= drag;
            if npc.velocity.0.abs() < 0.1 {
                npc.velocity.0 = 0.0;
            }
            if npc.velocity.1.abs() < 0.1 {
                npc.velocity.1 = 0.0;
            }
        } else {
            npc.rotation = npc.velocity.1.atan2(npc.velocity.0) - 1.57;
        }
        if npc.ai[2] >= dash_for {
            npc.ai[3] += 1.0;
            npc.ai[2] = 0.0;
            // Three to a set, then back to hovering.
            npc.ai[1] = if npc.ai[3] >= EYE_DASHES {
                npc.ai[3] = 0.0;
                // EYE-7: below half health an Expert second form lunges instead, with a random
                // head start on the lunge count so the run is shorter (`NPC.cs:20623-20627`).
                if second && expert && health < EYE_LUNGE_AT {
                    npc.ai[3] += rng.random_range(EYE_LUNGE_HEAD_START) as f32;
                    AIMING
                } else {
                    HOVERING
                }
            } else {
                LAUNCHING
            };
            npc.dirty = true;
        }
    } else if npc.ai[1] == AIMING {
        // EYE-7: the lunge's one aiming tick (`NPC.cs:20636-20745`). It leads your own velocity,
        // scatters the line, and then, unless it is nearly dead, turns the whole thing ninety
        // degrees so the lunge sweeps past you rather than landing on you.
        if frenzied && npc.ai[3] == EYE_LUNGE_BREAK_AT && centre.1 > their_middle.1 {
            // The one escape from the cycle.
            npc.ai[1] = HOVERING;
            npc.ai[2] = 0.0;
            npc.ai[3] = 0.0;
        } else {
            let mut speed = EYE_LUNGE_SPEED;
            let mut lead = EYE_LUNGE_LEAD;
            // Straight off the back-off it leads much further and travels faster.
            if npc.ai[2] == -1.0 && !desperate {
                lead *= EYE_LUNGE_LEAD_FROM_BELOW;
                speed *= EYE_LUNGE_SPEED_FROM_BELOW;
            }
            if desperate {
                lead *= EYE_LUNGE_LEAD_DESPERATE;
            }
            let stretch = |rng: &mut SmallRng| {
                1.0 + rng.random_range(-EYE_LUNGE_STRETCH..=EYE_LUNGE_STRETCH) as f32 * 0.01
            };
            let nudge = |rng: &mut SmallRng, n: i32| rng.random_range(-n..=n) as f32 * 0.1;
            let mut aim = (
                their_middle.0 - centre.0 - target.velocity.0 * lead,
                their_middle.1 - centre.1 - target.velocity.1 * lead / 4.0,
            );
            aim = (aim.0 * stretch(rng), aim.1 * stretch(rng));
            if desperate {
                aim = (aim.0 * stretch(rng), aim.1 * stretch(rng));
            }
            let reach = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
            let k = speed / reach;
            npc.velocity = (aim.0 * k, aim.1 * k);
            npc.velocity.0 += nudge(rng, EYE_LUNGE_NUDGE);
            npc.velocity.1 += nudge(rng, EYE_LUNGE_NUDGE);
            // The perpendicular swap: whichever branch runs, the two components are exchanged and
            // signed away from the player, which is what turns a lunge into a pass.
            let signed = |npc: &Npc| {
                let (mut across, mut down) = (npc.velocity.0.abs(), npc.velocity.1.abs());
                if centre.0 > their_middle.0 {
                    down = -down;
                }
                if centre.1 > their_middle.1 {
                    across = -across;
                }
                (across, down)
            };
            if desperate {
                npc.velocity.0 += nudge(rng, EYE_LUNGE_NUDGE_DESPERATE);
                npc.velocity.1 += nudge(rng, EYE_LUNGE_NUDGE_DESPERATE);
                let (across, down) = signed(npc);
                npc.velocity = (down + npc.velocity.0, across + npc.velocity.1);
                let length = npc.velocity.0.hypot(npc.velocity.1).max(f32::MIN_POSITIVE);
                npc.velocity = (
                    npc.velocity.0 / length * speed,
                    npc.velocity.1 / length * speed,
                );
                npc.velocity.0 += nudge(rng, EYE_LUNGE_NUDGE);
                npc.velocity.1 += nudge(rng, EYE_LUNGE_NUDGE);
            } else if reach < EYE_LUNGE_SWAP_RANGE {
                if npc.velocity.0.abs() > npc.velocity.1.abs() {
                    let (across, down) = signed(npc);
                    npc.velocity = (down, across);
                }
            } else if npc.velocity.0.abs() > npc.velocity.1.abs() {
                let mean = (npc.velocity.0.abs() + npc.velocity.1.abs()) / 2.0;
                let (mut across, mut down) = (mean, mean);
                if centre.0 > their_middle.0 {
                    across = -across;
                }
                if centre.1 > their_middle.1 {
                    down = -down;
                }
                npc.velocity = (across, down);
            }
            npc.ai[1] = LUNGING;
        }
        npc.dirty = true;
    } else if npc.ai[1] == LUNGING {
        // EYE-7: holding the line, then braking (`NPC.cs:20746-20799`).
        let hold = if desperate {
            EYE_LUNGE_HOLD_DESPERATE
        } else {
            EYE_LUNGE_HOLD
        };
        npc.ai[2] += 1.0;
        // Right on top of you it refuses to start braking, so the lunge carries through rather
        // than stalling in your face (`NPC.cs:20754-20757`, a `position`-to-`position` distance).
        let their_corner = (
            their_middle.0 - PLAYER_WIDTH as f32 / 2.0,
            their_middle.1 - PLAYER_HEIGHT as f32 / 2.0,
        );
        let apart = (npc.position.0 - their_corner.0).hypot(npc.position.1 - their_corner.1);
        if npc.ai[2] == hold && apart < EYE_LUNGE_STALL_RANGE {
            npc.ai[2] -= 1.0;
        }
        if npc.ai[2] >= hold {
            npc.velocity.0 *= EYE_LUNGE_DRAG;
            npc.velocity.1 *= EYE_LUNGE_DRAG;
            if npc.velocity.0.abs() < 0.1 {
                npc.velocity.0 = 0.0;
            }
            if npc.velocity.1.abs() < 0.1 {
                npc.velocity.1 = 0.0;
            }
        } else {
            npc.rotation = npc.velocity.1.atan2(npc.velocity.0) - 1.57;
        }
        if npc.ai[2] >= hold + EYE_LUNGE_RECOVER {
            npc.ai[3] += 1.0;
            npc.ai[2] = 0.0;
            if npc.ai[3] >= EYE_LUNGES {
                npc.ai[1] = HOVERING;
                npc.ai[3] = 0.0;
            } else {
                npc.ai[1] = AIMING;
            }
            npc.dirty = true;
        }
    } else if npc.ai[1] == BACKING_OFF {
        // EYE-7: six hundred pixels *below* you, and then up through you (`NPC.cs:20800-20852`).
        let (dx, dy) = (
            their_middle.0 - centre.0,
            their_middle.1 + EYE_BACKOFF_BELOW - centre.1,
        );
        let reach = dx.hypot(dy).max(f32::MIN_POSITIVE);
        let k = EYE_BACKOFF_SPEED / reach;
        close_on(&mut npc.velocity.0, dx * k, EYE_BACKOFF_ACCEL);
        close_on(&mut npc.velocity.1, dy * k, EYE_BACKOFF_ACCEL);
        npc.ai[2] += 1.0;
        if npc.ai[2] >= EYE_BACKOFF_TICKS {
            npc.ai[1] = AIMING;
            // The marker the aim reads to know this is the fast lunge.
            npc.ai[2] = -1.0;
            npc.ai[3] = rng.random_range(EYE_BACKOFF_LUNGE_BONUS) as f32;
            npc.dirty = true;
        }
    }

    // EYE-7: nearly dead it does not back off at all, it just keeps lunging (`NPC.cs:20854-20857`).
    if desperate && npc.ai[1] == BACKING_OFF {
        npc.ai[1] = AIMING;
    }

    // Half health opens it up — 65% in Expert Mode. Checked only in the first form, so it happens
    // once.
    let split_at = if expert {
        EYE_SPLIT_AT_EXPERT
    } else {
        EYE_SPLIT_AT
    };
    if !second && npc.ai[0] == FIRST_FORM && (npc.life as f32) < npc.life_max as f32 * split_at {
        npc.ai = [1.0, 0.0, 0.0, 0.0];
        npc.dirty = true;
    }

    npc.dirty = true;
    summoned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use terrustia_proto::tile::Tile;

    struct Night;

    impl TileView for Night {
        fn tile(&self, _x: i32, _y: i32) -> Tile {
            Tile::AIR
        }
    }

    fn eye() -> Npc {
        Npc::new(4, (10_000.0, 9_500.0), 1).expect("eye of cthulhu")
    }

    fn world<'a>(tiles: &'a Night, target: Option<Target>) -> World<'a, Night> {
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

    fn seeded() -> SmallRng {
        use rand::SeedableRng;
        SmallRng::seed_from_u64(4)
    }

    /// One tick for the tests that do not care about the random branches: each call gets its own
    /// fixed generator, so they are as deterministic as they were before the Expert lunge cycle
    /// needed one.
    fn tick<T: TileView>(npc: &mut Npc, world: &World<'_, T>) -> Vec<Spawn> {
        update(npc, world, &mut seeded())
    }

    #[test]
    fn it_hovers_above_you_rather_than_on_you() {
        let tiles = Night;
        let mut e = eye();
        let t = Some(player_at(10_000.0, 10_000.0));
        for _ in 0..300 {
            tick(&mut e, &world(&tiles, t));
            e.position.0 += e.velocity.0;
            e.position.1 += e.velocity.1;
        }
        let above = 10_000.0 - e.center().1;
        assert!(
            above > 100.0,
            "it should be hanging above, got {above} pixels up"
        );
    }

    #[test]
    fn it_dashes_three_times_and_then_hovers_again() {
        let tiles = Night;
        let mut e = eye();
        let t = Some(player_at(10_000.0, 10_000.0));
        // Skip to the end of the hover.
        e.ai[2] = EYE_HOVER_TICKS_FIRST;
        let mut dashes = 0;
        let mut back_to_hovering = false;
        for _ in 0..2000 {
            let before = e.ai[1];
            tick(&mut e, &world(&tiles, t));
            if before == LAUNCHING && e.ai[1] == DASHING {
                dashes += 1;
            }
            if dashes == EYE_DASHES as i32 && e.ai[1] == HOVERING {
                back_to_hovering = true;
                break;
            }
        }
        assert_eq!(dashes, EYE_DASHES as i32);
        assert!(back_to_hovering, "and then it should settle again");
    }

    #[test]
    fn a_dash_is_aimed_once_and_never_corrected() {
        let tiles = Night;
        let mut e = eye();
        let t = Some(player_at(10_000.0, 10_000.0));
        e.ai[1] = LAUNCHING;
        tick(&mut e, &world(&tiles, t));
        let launched = e.velocity;
        assert!(
            (launched.0.hypot(launched.1) - EYE_DASH_FIRST).abs() < 1e-3,
            "should leave at its dash speed, got {launched:?}"
        );

        // Move the player: the dash keeps its original heading.
        let moved = Some(player_at(4_000.0, 10_000.0));
        tick(&mut e, &world(&tiles, moved));
        assert_eq!(e.velocity, launched, "a committed dash does not steer");
    }

    #[test]
    fn it_throws_out_servants_while_it_hovers() {
        let tiles = Night;
        let mut e = eye();
        // Above the player and well within range.
        e.position = (10_000.0, 10_000.0 - 250.0);
        let t = Some(player_at(10_000.0, 10_000.0));
        let mut spawned = Vec::new();
        for _ in 0..(EYE_SERVANT_EVERY as i32 + 5) {
            spawned.extend(tick(&mut e, &world(&tiles, t)));
        }
        assert!(!spawned.is_empty(), "should have summoned");
        let servant = spawned[0];
        assert_eq!(servant.npc_type, SERVANT_OF_CTHULHU);
        assert!(
            servant.velocity.1 > 0.0,
            "and thrown it down at the player, got {:?}",
            servant.velocity
        );
    }

    #[test]
    fn the_second_form_summons_nothing() {
        let tiles = Night;
        let mut e = eye();
        e.position = (10_000.0, 10_000.0 - 250.0);
        e.ai[0] = SECOND_FORM;
        let t = Some(player_at(10_000.0, 10_000.0));
        for _ in 0..600 {
            assert!(tick(&mut e, &world(&tiles, t)).is_empty());
        }
    }

    #[test]
    fn half_health_splits_it_open() {
        let tiles = Night;
        let mut e = eye();
        let t = Some(player_at(10_000.0, 10_000.0));
        tick(&mut e, &world(&tiles, t));
        assert_eq!(e.ai[0], FIRST_FORM);

        e.life = (e.life_max as f32 * EYE_SPLIT_AT) as i32 - 1;
        tick(&mut e, &world(&tiles, t));
        assert_eq!(e.ai[0], 1.0, "should have started to split");

        // Two hundred ticks of spinning and it is through.
        let before = e.rotation;
        for _ in 0..(EYE_SPLIT_TICKS as i32 * 2 + 2) {
            tick(&mut e, &world(&tiles, t));
        }
        assert_eq!(e.ai[0], SECOND_FORM);
        assert!(e.rotation != before, "and it should have been spinning");
    }

    #[test]
    fn the_second_form_has_no_defence_left() {
        let tiles = Night;
        let mut e = eye();
        e.ai[0] = SECOND_FORM;
        let t = Some(player_at(10_000.0, 10_000.0));
        tick(&mut e, &world(&tiles, t));
        // The live fields combat actually reads (`server.rs`'s "live armour, not the type's"),
        // not the type's own baseline stats — writing those left the shell's defence in place.
        assert_eq!(e.defense, EYE_SECOND_FORM_DEFENSE);
        assert!(
            (e.damage_bonus - EYE_SECOND_FORM_DAMAGE as f32 / e.stats.damage as f32).abs() < 1e-6
        );
    }

    /// Real vanilla (`NPC.cs`, `aiStyle==4`): `flag2`/`flag3` drop defence to -15 below 12% health
    /// and -30 below 4%, Expert Mode only. On the unfixed code neither `world.conditions.expert`
    /// nor these thresholds were read at all, so this fails on that code with every case at 0.
    #[test]
    fn expert_mode_strips_more_armour_as_the_second_form_nears_death() {
        let tiles = Night;
        let t = Some(player_at(10_000.0, 10_000.0));
        let defense_at = |life_fraction: f32, expert: bool| {
            let mut e = eye();
            e.ai[0] = SECOND_FORM;
            e.life = (e.life_max as f32 * life_fraction) as i32;
            let mut w = world(&tiles, t);
            w.conditions.expert = expert;
            tick(&mut e, &w);
            e.defense
        };
        assert_eq!(
            defense_at(0.5, true),
            EYE_SECOND_FORM_DEFENSE,
            "still just shell-less at half health"
        );
        assert_eq!(defense_at(0.1, true), EYE_SECOND_FORM_DEFENSE_LOW);
        assert_eq!(defense_at(0.03, true), EYE_SECOND_FORM_DEFENSE_VERY_LOW);
        assert_eq!(
            defense_at(0.03, false),
            EYE_SECOND_FORM_DEFENSE,
            "Normal mode's defence never drops further"
        );
    }

    /// Real vanilla splits the first form open at 50% health, 65% in Expert Mode (`flag`-less
    /// `num28` check). Fails on the unfixed code, which splits at 50% regardless of Expert Mode.
    #[test]
    fn expert_mode_splits_it_open_at_a_higher_health_fraction() {
        let tiles = Night;
        let t = Some(player_at(10_000.0, 10_000.0));
        // Above Normal's 50% threshold but below Expert's 65%.
        let splits = |expert: bool| {
            let mut e = eye();
            e.life = (e.life_max as f32 * 0.6) as i32;
            let mut w = world(&tiles, t);
            w.conditions.expert = expert;
            tick(&mut e, &w);
            e.ai[0]
        };
        assert_eq!(
            splits(false),
            FIRST_FORM,
            "Normal mode should not have split yet"
        );
        assert_eq!(splits(true), 1.0, "Expert Mode should already be splitting");
    }

    /// Real vanilla: the first form's hover is 5px/s at 0.04 accel and 600 ticks long in Normal
    /// mode, 7px/s at 0.15 accel and 210 ticks in Expert. Fails on the unfixed code, which hovers
    /// identically either way.
    #[test]
    fn expert_mode_hovers_faster_and_far_more_briefly() {
        let tiles = Night;
        let t = Some(player_at(10_000.0, 10_000.0));

        let hover_ticks = |expert: bool| {
            let mut e = eye();
            let mut w = world(&tiles, t);
            w.conditions.expert = expert;
            let mut ticks = 0;
            while e.ai[1] == HOVERING && ticks < 2000 {
                tick(&mut e, &w);
                ticks += 1;
            }
            ticks
        };
        assert_eq!(hover_ticks(false), EYE_HOVER_TICKS_FIRST as i32);
        assert_eq!(hover_ticks(true), EYE_HOVER_TICKS_FIRST_EXPERT as i32);

        let velocity_after_one_tick = |expert: bool| {
            let mut e = eye();
            let mut w = world(&tiles, t);
            w.conditions.expert = expert;
            tick(&mut e, &w);
            e.velocity.0.hypot(e.velocity.1)
        };
        assert!(
            velocity_after_one_tick(true) > velocity_after_one_tick(false) * 2.0,
            "Expert's much sharper acceleration should show up after a single tick"
        );
    }

    /// Real vanilla: Expert Mode calls a servant every 44 ticks instead of every 110, and does not
    /// require being above the player first. Fails on the unfixed code, which never varies by
    /// Expert Mode and never spawns when not above the player.
    #[test]
    fn expert_mode_calls_servants_more_often_and_even_when_not_above_you() {
        let tiles = Night;
        let t = Some(player_at(10_000.0, 10_000.0));

        // Safely above the player: satisfies both modes' gate, isolating the cadence difference.
        let spawned_above = |expert: bool| {
            let mut e = eye();
            e.position = (10_000.0, 10_000.0 - 250.0);
            let mut w = world(&tiles, t);
            w.conditions.expert = expert;
            let mut spawned = 0;
            for _ in 0..(EYE_SERVANT_EVERY as i32 + 5) {
                spawned += tick(&mut e, &w).len();
            }
            spawned
        };
        assert!(
            spawned_above(true) > spawned_above(false),
            "Expert's shorter cadence should call more of them in the same window"
        );

        // Level with the player rather than above: Normal mode's gate should block it outright.
        let spawned_level = |expert: bool| {
            let mut e = eye();
            e.position = (10_000.0, 10_000.0);
            let mut w = world(&tiles, t);
            w.conditions.expert = expert;
            let mut spawned = 0;
            for _ in 0..(EYE_SERVANT_EVERY_EXPERT as i32 + 5) {
                spawned += tick(&mut e, &w).len();
            }
            spawned
        };
        assert_eq!(
            spawned_level(false),
            0,
            "Normal mode needs to be above the player"
        );
        assert!(spawned_level(true) > 0, "Expert Mode does not");
    }

    #[test]
    fn the_second_form_is_the_same_fight_but_faster() {
        const {
            assert!(EYE_HOVER_TICKS_SECOND < EYE_HOVER_TICKS_FIRST);
            assert!(EYE_DASH_SECOND > EYE_DASH_FIRST);
            assert!(EYE_DASH_TICKS_SECOND < EYE_DASH_TICKS_FIRST);
            assert!(EYE_HOVER_SECOND.0 < EYE_HOVER_FIRST.0);
        }
    }

    #[test]
    fn daylight_ends_the_fight() {
        let tiles = Night;
        let mut e = eye();
        let t = Some(player_at(10_000.0, 10_000.0));
        let mut w = world(&tiles, t);
        w.conditions.day = true;
        tick(&mut e, &w);
        assert!(e.velocity.1 < 0.0, "it should climb away");
        assert!(e.time_left <= 10);
    }

    #[test]
    fn a_dead_player_ends_it_too() {
        let tiles = Night;
        let mut e = eye();
        let dead = Some(Target {
            slot: 0,
            center: (10_000.0, 10_000.0),
            velocity: (0.0, 0.0),
            alive: false,
        });
        tick(&mut e, &world(&tiles, dead));
        assert!(e.time_left <= 10);
    }

    /// A world at a chosen difficulty, with the player somewhere below.
    fn fight<'a>(tiles: &'a Night, expert: bool, player: (f32, f32)) -> World<'a, Night> {
        let mut w = world(tiles, Some(player_at(player.0, player.1)));
        w.conditions.expert = expert;
        w
    }

    /// EYE-1: the first form dashes at seven in Expert, not six (`NPC.cs:20252-20256`).
    #[test]
    fn expert_mode_speeds_up_the_first_forms_dash() {
        let tiles = Night;
        let launched = |expert: bool| {
            let mut e = eye();
            e.ai[1] = LAUNCHING;
            tick(&mut e, &fight(&tiles, expert, (10_000.0, 10_400.0)));
            e.velocity.0.hypot(e.velocity.1)
        };
        assert!((launched(false) - EYE_DASH_FIRST).abs() < 1e-4);
        assert!((launched(true) - EYE_DASH_FIRST_EXPERT).abs() < 1e-4);
    }

    /// EYE-2: and ends it a third sooner (`NPC.cs:20298-20302`), so an Expert set comes round again
    /// far faster.
    #[test]
    fn expert_mode_shortens_the_first_forms_dash() {
        let tiles = Night;
        let dash_ticks = |expert: bool| {
            let mut e = eye();
            e.ai[1] = DASHING;
            let w = fight(&tiles, expert, (10_000.0, 10_400.0));
            let mut ticks = 0;
            while e.ai[1] == DASHING && ticks < 1_000 {
                tick(&mut e, &w);
                ticks += 1;
            }
            ticks as f32
        };
        assert_eq!(dash_ticks(false), EYE_DASH_TICKS_FIRST);
        assert_eq!(dash_ticks(true), EYE_DASH_TICKS_FIRST_EXPERT);
    }

    /// EYE-3: the Expert drag multiplies the classic one rather than replacing it
    /// (`NPC.cs:20276-20280`), so a spent Expert dash bleeds off faster.
    #[test]
    fn expert_mode_drags_a_spent_dash_harder() {
        let tiles = Night;
        let after = |expert: bool| {
            let mut e = eye();
            e.ai[1] = DASHING;
            e.ai[2] = EYE_DASH_DRIVE; // already braking
            e.velocity = (10.0, 0.0);
            tick(&mut e, &fight(&tiles, expert, (10_000.0, 10_400.0)));
            e.velocity.0
        };
        assert!((after(false) - 10.0 * EYE_DASH_DRAG_FIRST).abs() < 1e-4);
        assert!(
            (after(true) - 10.0 * EYE_DASH_DRAG_FIRST * EYE_DASH_DRAG_FIRST_EXPERT).abs() < 1e-4
        );
    }

    /// EYE-4: the Expert split is not two hundred free ticks. It throws a servant every twentieth
    /// tick of the spin (`NPC.cs:20363-20400`), ten across the whole transformation, on bearings
    /// that go every way rather than at you.
    #[test]
    fn the_expert_split_throws_servants_while_it_spins() {
        let tiles = Night;
        let through_the_split = |expert: bool| {
            let mut rng = seeded();
            let mut e = eye();
            e.ai[0] = 1.0; // mid-transformation
            let w = fight(&tiles, expert, (10_000.0, 10_400.0));
            let mut thrown = Vec::new();
            for _ in 0..(EYE_SPLIT_TICKS as i32 * 2) {
                thrown.extend(update(&mut e, &w, &mut rng));
            }
            thrown
        };
        assert!(through_the_split(false).is_empty(), "classic throws none");
        let expert = through_the_split(true);
        assert_eq!(
            expert.len(),
            (EYE_SPLIT_TICKS as i32 * 2 / EYE_SPLIT_SERVANT_EVERY as i32) as usize,
            "ten across the transformation"
        );
        assert!(expert.iter().all(|s| s.npc_type == SERVANT_OF_CTHULHU));
        assert!(
            expert
                .iter()
                .all(|s| (s.velocity.0.hypot(s.velocity.1) - EYE_SPLIT_SERVANT_SPEED).abs() < 1e-3)
        );
        // Random bearings, not aimed: they go both ways.
        assert!(expert.iter().any(|s| s.velocity.0 > 0.0));
        assert!(expert.iter().any(|s| s.velocity.0 < 0.0));
    }

    /// EYE-5: an Expert second form closes the gap rather than letting you open it
    /// (`NPC.cs:20476-20490`): a pixel of speed and 0.05 of acceleration at each of four hundred,
    /// six hundred and eight hundred pixels. Classic has no such term, which is why kiting works
    /// there and not here.
    #[test]
    fn the_expert_second_form_chases_harder_the_further_off_you_are() {
        let tiles = Night;
        let push = |expert: bool, across: f32| {
            let mut e = eye();
            e.ai[0] = SECOND_FORM;
            e.velocity = (0.0, 0.0);
            // Sat exactly at the hover offset, so the whole gap is the horizontal one and the
            // acceleration lands entirely on `velocity.0`.
            let (cx, cy) = e.center();
            let at = (cx + across, cy + EYE_HOVER_SECOND.0);
            tick(&mut e, &fight(&tiles, expert, at));
            e.velocity.0
        };
        let base = EYE_HOVER_SECOND.2;
        assert!((push(false, 100.0) - base).abs() < 1e-4, "classic, close");
        assert!(
            (push(false, 900.0) - base).abs() < 1e-4,
            "classic does not change with distance"
        );
        assert!((push(true, 100.0) - base).abs() < 1e-4, "expert, close");
        assert!(
            (push(true, 900.0) - (base + 3.0 * EYE_HOVER_SECOND_EXPERT_ACCEL_STEP)).abs() < 1e-4,
            "expert past eight hundred takes all three steps, got {}",
            push(true, 900.0)
        );
    }

    /// EYE-6: each dash of an Expert second form's set is faster than the last
    /// (`NPC.cs:20558-20565`), and the dash both drives longer before braking and ends sooner
    /// (`:20582-20587`, `:20608-20612`).
    #[test]
    fn the_expert_second_forms_dashes_escalate_within_a_set() {
        let tiles = Night;
        let launched = |expert: bool, index: f32| {
            let mut e = eye();
            e.ai[0] = SECOND_FORM;
            e.ai[1] = LAUNCHING;
            e.ai[3] = index;
            tick(&mut e, &fight(&tiles, expert, (10_000.0, 10_400.0)));
            e.velocity.0.hypot(e.velocity.1)
        };
        for index in [0.0, 1.0, 2.0] {
            assert!(
                (launched(false, index) - EYE_DASH_SECOND).abs() < 1e-4,
                "classic runs all three flat"
            );
        }
        assert!((launched(true, 0.0) - EYE_DASH_SECOND).abs() < 1e-4);
        assert!(
            (launched(true, 1.0) - EYE_DASH_SECOND * EYE_DASH_SECOND_EXPERT_STEPS[0]).abs() < 1e-3
        );
        assert!(
            (launched(true, 2.0) - EYE_DASH_SECOND * EYE_DASH_SECOND_EXPERT_STEPS[1]).abs() < 1e-3
        );

        // The brake and the end of the dash both move.
        let braked_at = |expert: bool| {
            let mut e = eye();
            e.ai[0] = SECOND_FORM;
            e.ai[1] = DASHING;
            e.velocity = (10.0, 0.0);
            let w = fight(&tiles, expert, (10_000.0, 10_400.0));
            let mut ticks = 0;
            // The first tick on which the drag actually bites.
            while e.velocity.0 == 10.0 && ticks < 1_000 {
                tick(&mut e, &w);
                ticks += 1;
            }
            ticks as f32
        };
        assert_eq!(braked_at(false), EYE_DASH_DRIVE);
        assert_eq!(braked_at(true), EYE_DASH_DRIVE_SECOND_EXPERT);

        let dash_ticks = |expert: bool| {
            let mut e = eye();
            e.ai[0] = SECOND_FORM;
            e.ai[1] = DASHING;
            let w = fight(&tiles, expert, (10_000.0, 10_400.0));
            let mut ticks = 0;
            while e.ai[1] == DASHING && ticks < 1_000 {
                tick(&mut e, &w);
                ticks += 1;
            }
            ticks as f32
        };
        assert_eq!(dash_ticks(false), EYE_DASH_TICKS_SECOND);
        assert_eq!(dash_ticks(true), EYE_DASH_TICKS_SECOND_EXPERT);
    }

    /// EYE-7: below half health an Expert second form stops running dash sets and starts lunging
    /// (`NPC.cs:20623-20627`). Classic never does, at any health.
    #[test]
    fn a_wounded_expert_second_form_lunges_instead_of_dashing() {
        let tiles = Night;
        let after_a_set = |expert: bool, health: f32| {
            let mut e = eye();
            e.ai[0] = SECOND_FORM;
            e.ai[1] = DASHING;
            e.ai[3] = EYE_DASHES - 1.0; // the last dash of the set
            e.ai[2] = if expert {
                EYE_DASH_TICKS_SECOND_EXPERT
            } else {
                EYE_DASH_TICKS_SECOND
            } - 1.0;
            e.life = (e.life_max as f32 * health) as i32;
            tick(&mut e, &fight(&tiles, expert, (10_000.0, 10_400.0)));
            (e.ai[1], e.ai[3])
        };
        assert_eq!(after_a_set(false, 0.45).0, HOVERING, "classic hovers again");
        assert_eq!(
            after_a_set(true, 0.8).0,
            HOVERING,
            "and so does a healthy expert one"
        );
        let (state, count) = after_a_set(true, 0.45);
        assert_eq!(state, AIMING, "below half, expert lunges");
        assert!(
            (1.0..4.0).contains(&count),
            "with a random head start on the lunge count, got {count}"
        );
    }

    /// EYE-7: and below 35% it leaves the hover straight into a lunge too
    /// (`NPC.cs:20537-20540`).
    #[test]
    fn a_badly_hurt_expert_second_form_lunges_out_of_the_hover() {
        let tiles = Night;
        let after_a_hover = |health: f32| {
            let mut e = eye();
            e.ai[0] = SECOND_FORM;
            e.ai[2] = EYE_HOVER_TICKS_SECOND - 1.0;
            e.life = (e.life_max as f32 * health) as i32;
            tick(&mut e, &fight(&tiles, true, (10_000.0, 10_400.0)));
            e.ai[1]
        };
        assert_eq!(after_a_hover(0.45), LAUNCHING, "above 35% it still dashes");
        assert_eq!(after_a_hover(0.3), AIMING, "below it lunges");
    }

    /// EYE-7: the lunge itself. One aiming tick at twenty pixels a tick, held and then braked, five
    /// times over before it hovers again (`NPC.cs:20649`, `:20746-20798`).
    #[test]
    fn the_lunge_cycle_runs_five_times_and_then_hovers() {
        let tiles = Night;
        let mut rng = seeded();
        let mut e = eye();
        e.ai[0] = SECOND_FORM;
        e.ai[1] = AIMING;
        e.life = (e.life_max as f32 * 0.45) as i32;
        let w = fight(&tiles, true, (10_000.0, 10_800.0));

        update(&mut e, &w, &mut rng);
        assert_eq!(e.ai[1], LUNGING, "one tick to aim, then it commits");
        assert!(
            (e.velocity.0.hypot(e.velocity.1) - EYE_LUNGE_SPEED).abs() < 3.0,
            "roughly twenty a tick before the scatter, got {}",
            e.velocity.0.hypot(e.velocity.1)
        );

        let mut lunges = 1;
        for _ in 0..2_000 {
            update(&mut e, &w, &mut rng);
            if e.ai[1] == AIMING {
                lunges += 1;
            }
            if e.ai[1] == HOVERING {
                break;
            }
        }
        assert_eq!(e.ai[1], HOVERING, "the cycle ends");
        assert_eq!(lunges as f32, EYE_LUNGES, "five lunges to a cycle");
    }

    /// EYE-7: below 12% every cycle opens by dropping six hundred pixels *beneath* you first, and
    /// it comes back up marked for the faster lunge (`NPC.cs:20464-20467`, `:20800-20852`).
    #[test]
    fn a_frenzied_expert_second_form_backs_off_underneath_you() {
        let tiles = Night;
        let mut rng = seeded();
        let mut e = eye();
        e.ai[0] = SECOND_FORM;
        e.life = (e.life_max as f32 * 0.1) as i32;
        let w = fight(&tiles, true, (10_000.0, 9_000.0));

        update(&mut e, &w, &mut rng);
        assert_eq!(e.ai[1], BACKING_OFF, "the hover is replaced outright");
        assert!(e.velocity.1 > 0.0, "and it heads down, below the player");

        let mut ticks = 1;
        while e.ai[1] == BACKING_OFF && ticks < 500 {
            update(&mut e, &w, &mut rng);
            ticks += 1;
        }
        assert_eq!(ticks as f32, EYE_BACKOFF_TICKS, "seventy ticks under you");
        assert_eq!(e.ai[1], AIMING);
        assert_eq!(e.ai[2], -1.0, "marked as the fast lunge");
        assert!(
            EYE_BACKOFF_LUNGE_BONUS.contains(&(e.ai[3] as i32)),
            "with extra lunges to come, got {}",
            e.ai[3]
        );
    }

    /// EYE-7: and below 4% it never backs off and never stops lunging (`NPC.cs:20544-20551`,
    /// `:20854-20857`).
    #[test]
    fn a_desperate_expert_second_form_only_lunges() {
        let tiles = Night;
        let w = fight(&tiles, true, (10_000.0, 9_000.0));
        // Both routes out of the lunge cycle are closed in this band, and both close in the same
        // tick: the hover is swapped for the back-off going in, and the back-off is turned straight
        // back into an aim coming out.
        let from = |state: f32| {
            let mut rng = seeded();
            let mut e = eye();
            e.ai[0] = SECOND_FORM;
            e.ai[1] = state;
            e.life = (e.life_max as f32 * 0.02) as i32;
            update(&mut e, &w, &mut rng);
            e.ai[1]
        };
        assert_eq!(
            from(BACKING_OFF),
            AIMING,
            "the back-off is turned straight back round"
        );
        assert_eq!(from(HOVERING), AIMING, "and the hover is never entered");
        assert_eq!(from(LUNGING), LUNGING, "the lunge itself carries on");
    }
}
