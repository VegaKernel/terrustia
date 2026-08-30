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
//! Expert Mode changes more than the numbers: it opens the split at 65% health rather than 50%,
//! hovers faster and far more briefly in the first form, drops that form's "servants only from
//! above" rule, calls a servant much more often, and — once split — strips still more armour as
//! the second form nears death.

use terrustia_proto::npc_params::{
    EYE_DASH_DRAG_FIRST, EYE_DASH_DRAG_SECOND, EYE_DASH_DRIVE, EYE_DASH_FIRST, EYE_DASH_SECOND,
    EYE_DASH_TICKS_FIRST, EYE_DASH_TICKS_SECOND, EYE_DASHES, EYE_HOVER_FIRST,
    EYE_HOVER_FIRST_EXPERT, EYE_HOVER_SECOND, EYE_HOVER_TICKS_FIRST, EYE_HOVER_TICKS_FIRST_EXPERT,
    EYE_HOVER_TICKS_SECOND, EYE_SECOND_FORM_DAMAGE, EYE_SECOND_FORM_DEFENSE,
    EYE_SECOND_FORM_DEFENSE_LOW, EYE_SECOND_FORM_DEFENSE_LOW_AT, EYE_SECOND_FORM_DEFENSE_VERY_LOW,
    EYE_SECOND_FORM_DEFENSE_VERY_LOW_AT, EYE_SERVANT_EVERY, EYE_SERVANT_EVERY_EXPERT,
    EYE_SERVANT_RANGE, EYE_SERVANT_SPEED, EYE_SERVANT_SPEED_EXPERT, EYE_SPIN_MAX, EYE_SPIN_RAMP,
    EYE_SPLIT_AT, EYE_SPLIT_AT_EXPERT, EYE_SPLIT_TICKS, SERVANT_OF_CTHULHU,
};

use crate::game::ai::World;
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Spawn;

/// Which form it is in, as `ai[0]` records it. One and two are the two halves of the split.
const FIRST_FORM: f32 = 0.0;
const SECOND_FORM: f32 = 3.0;

/// The three steps of a dash set, as `ai[1]` records them.
const HOVERING: f32 = 0.0;
const LAUNCHING: f32 = 1.0;
const DASHING: f32 = 2.0;

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
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>) -> Vec<Spawn> {
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

    // The two halves of the split: it spins in place, faster and faster, and comes out changed.
    if npc.ai[0] == 1.0 || npc.ai[0] == 2.0 {
        npc.ai[2] = (npc.ai[2] + EYE_SPIN_RAMP).min(EYE_SPIN_MAX);
        npc.rotation += npc.ai[2];
        npc.ai[1] += 1.0;
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
    // Expert Mode hovers faster and much more briefly in the first form — the second form's own
    // hover (already faster and briefer than the first's) does not change again in Expert Mode.
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
    let dash_speed = if second {
        EYE_DASH_SECOND
    } else {
        EYE_DASH_FIRST
    };
    let dash_for = if second {
        EYE_DASH_TICKS_SECOND
    } else {
        EYE_DASH_TICKS_FIRST
    };
    let drag = if second {
        EYE_DASH_DRAG_SECOND
    } else {
        EYE_DASH_DRAG_FIRST
    };

    // Its shell is gone, so nothing softens a hit any more — and Expert Mode strips even more
    // armour once it is nearly dead. Both write the live `defense`/`damage_bonus` fields combat
    // actually reads, not the type's own baseline stats.
    if second {
        npc.defense = EYE_SECOND_FORM_DEFENSE;
        if expert {
            let health = npc.life as f32 / npc.life_max as f32;
            if health < EYE_SECOND_FORM_DEFENSE_VERY_LOW_AT {
                npc.defense = EYE_SECOND_FORM_DEFENSE_VERY_LOW;
            } else if health < EYE_SECOND_FORM_DEFENSE_LOW_AT {
                npc.defense = EYE_SECOND_FORM_DEFENSE_LOW;
            }
        }
        npc.damage_bonus = EYE_SECOND_FORM_DAMAGE as f32 / npc.stats.damage.max(1) as f32;
    }

    if npc.ai[1] == HOVERING {
        // Hanging above them, easing into position.
        let wanted = {
            let (dx, dy) = (their_middle.0 - centre.0, their_middle.1 - lift - centre.1);
            let reach = (dx * dx + dy * dy).sqrt().max(f32::MIN_POSITIVE);
            let k = hover_speed / reach;
            (dx * k, dy * k)
        };
        close_on(&mut npc.velocity.0, wanted.0, hover_accel);
        close_on(&mut npc.velocity.1, wanted.1, hover_accel);

        npc.ai[2] += 1.0;
        if npc.ai[2] >= hover_for {
            npc.ai[1] = LAUNCHING;
            npc.ai[2] = 0.0;
            npc.ai[3] = 0.0;
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
                        position: (centre.0 + throw.0 * 10.0, centre.1 + throw.1 * 10.0),
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
        let k = dash_speed / reach;
        npc.velocity = (dx * k, dy * k);
        npc.ai[1] = DASHING;
        npc.dirty = true;
    } else if npc.ai[1] == DASHING {
        npc.ai[2] += 1.0;
        if npc.ai[2] >= EYE_DASH_DRIVE {
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
                HOVERING
            } else {
                LAUNCHING
            };
            npc.dirty = true;
        }
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

    #[test]
    fn it_hovers_above_you_rather_than_on_you() {
        let tiles = Night;
        let mut e = eye();
        let t = Some(player_at(10_000.0, 10_000.0));
        for _ in 0..300 {
            update(&mut e, &world(&tiles, t));
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
            update(&mut e, &world(&tiles, t));
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
        update(&mut e, &world(&tiles, t));
        let launched = e.velocity;
        assert!(
            (launched.0.hypot(launched.1) - EYE_DASH_FIRST).abs() < 1e-3,
            "should leave at its dash speed, got {launched:?}"
        );

        // Move the player: the dash keeps its original heading.
        let moved = Some(player_at(4_000.0, 10_000.0));
        update(&mut e, &world(&tiles, moved));
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
            spawned.extend(update(&mut e, &world(&tiles, t)));
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
            assert!(update(&mut e, &world(&tiles, t)).is_empty());
        }
    }

    #[test]
    fn half_health_splits_it_open() {
        let tiles = Night;
        let mut e = eye();
        let t = Some(player_at(10_000.0, 10_000.0));
        update(&mut e, &world(&tiles, t));
        assert_eq!(e.ai[0], FIRST_FORM);

        e.life = (e.life_max as f32 * EYE_SPLIT_AT) as i32 - 1;
        update(&mut e, &world(&tiles, t));
        assert_eq!(e.ai[0], 1.0, "should have started to split");

        // Two hundred ticks of spinning and it is through.
        let before = e.rotation;
        for _ in 0..(EYE_SPLIT_TICKS as i32 * 2 + 2) {
            update(&mut e, &world(&tiles, t));
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
        update(&mut e, &world(&tiles, t));
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
            update(&mut e, &w);
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
            update(&mut e, &w);
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
                update(&mut e, &w);
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
            update(&mut e, &w);
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
                spawned += update(&mut e, &w).len();
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
                spawned += update(&mut e, &w).len();
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
        update(&mut e, &w);
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
        update(&mut e, &world(&tiles, dead));
        assert!(e.time_left <= 10);
    }
}
