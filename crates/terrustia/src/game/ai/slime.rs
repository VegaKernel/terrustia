//! Style 1 — the slimes.
//!
//! Ported from `AI_001_Slimes`. A slime does nothing at all most of the time: `ai[0]` is a timer
//! that sits far below zero and counts up, and three windows on the way back to zero each trigger
//! a hop. That is why slimes move in bursts with long pauses rather than at a steady rate.

use terrustia_proto::npc_params::{slime_hop_window, slime_timer_bonus};

use crate::game::npc::Npc;
use crate::game::npc_ai::Target;

/// Upward impulse of the long hop and the short one.
pub const BIG_HOP_Y: f32 = -8.0;
pub const HOP_Y: f32 = -6.0;

/// Sideways push added to each.
pub const BIG_HOP_X: f32 = 3.0;
pub const HOP_X: f32 = 2.0;

/// Which of the three windows the timer has reached, if any.
///
/// The windows overlap oddly because the base is negative: window 2 is `[base, base/2]` and
/// window 3 is `[base*2, base*1.5]`, so a slime cycles through all three rather than repeating one.
fn hop_kind(ai0: f32, window: f32) -> u8 {
    if ai0 >= window * 2.0 && ai0 <= window * 1.5 {
        return 3;
    }
    if ai0 >= window && ai0 <= window * 0.5 {
        return 2;
    }
    if ai0 >= 0.0 {
        return 1;
    }
    0
}

/// Drive one slime. Only acts while it is standing on something.
pub fn update(npc: &mut Npc, target: Option<Target>, on_ground: bool, active: bool) {
    if !on_ground {
        return;
    }

    // Ground friction while it waits for the next hop.
    npc.velocity.0 *= 0.8;
    if npc.velocity.0.abs() < 0.1 {
        npc.velocity.0 = 0.0;
    }

    // Vanilla ticks the hop clock once a frame, and a second time when the slime is "active"
    // (`NPC.AI_001_Slimes`' own `flag3`) — so an active slime reaches its next hop in half the
    // frames, on top of whatever per-type bonus it carries.
    npc.ai[0] += 1.0
        + slime_timer_bonus(npc.npc_type, npc.life < npc.life_max)
        + f32::from(u8::from(active));
    let window = slime_hop_window(npc.npc_type);
    let kind = hop_kind(npc.ai[0], window);
    if kind == 0 {
        return;
    }

    // Vanilla only re-targets here (`TargetClosest()`, which is also what turns the slime to face
    // whatever it picks) while active (`flag3`), gated further by its own `ai[2] == 1f`: a roughly
    // two-hundred-tick cooldown after it last turned round off a wall, which this port does not
    // model (`ai[2]` is otherwise unused here). NPC.cs:62245-62248. A passive slime hopping in
    // daylight keeps whatever direction it already had instead of re-facing on every hop.
    if active && let Some(t) = target {
        npc.direction = if t.center.0 > npc.center().0 { 1 } else { -1 };
        npc.sprite_direction = npc.direction;
    }

    if kind == 3 {
        npc.velocity.1 = BIG_HOP_Y;
        npc.velocity.0 += BIG_HOP_X * f32::from(npc.direction);
        npc.ai[0] = -200.0;
        npc.ai[3] = npc.position.0;
    } else {
        npc.velocity.1 = HOP_Y;
        npc.velocity.0 += HOP_X * f32::from(npc.direction);
        npc.ai[0] = -120.0 + if kind == 1 { window } else { window * 2.0 };
    }
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slime(npc_type: u16) -> Npc {
        Npc::new(npc_type, (1000.0, 1000.0), 1).expect("slime type")
    }

    fn player(x: f32) -> Target {
        Target {
            slot: 0,
            center: (x, 1000.0),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    #[test]
    fn the_three_windows_match_the_games_arithmetic() {
        let w = -1000.0;
        assert_eq!(hop_kind(0.0, w), 1, "at zero the first window is open");
        assert_eq!(hop_kind(-750.0, w), 2, "between -1000 and -500");
        assert_eq!(hop_kind(-1750.0, w), 3, "between -2000 and -1500");
        assert_eq!(hop_kind(-300.0, w), 0, "nothing between the windows");
        assert_eq!(hop_kind(-2500.0, w), 0, "and nothing below them all");
    }

    #[test]
    fn a_slime_does_nothing_in_the_air() {
        let mut s = slime(1);
        s.ai[0] = 0.0;
        update(&mut s, Some(player(2000.0)), false, false);
        assert_eq!(s.velocity, (0.0, 0.0), "no hop while airborne");
    }

    #[test]
    fn a_slime_hops_toward_its_target_when_the_timer_comes_round() {
        let mut s = slime(1);
        s.ai[0] = -1.0; // one tick from the first window
        update(&mut s, Some(player(5000.0)), true, false);
        assert_eq!(s.velocity.1, HOP_Y, "should hop");
        assert!(s.velocity.0 > 0.0, "and lean toward the player");
        assert_eq!(s.direction, 1);
    }

    /// `TargetClosest()` (which is what turns a slime to face what it targets) only runs while the
    /// slime is active (`flag3`), not on every hop. `NPC.cs:62245-62248`.
    #[test]
    fn a_passive_slime_does_not_reface_on_every_hop() {
        let mut s = slime(1);
        s.ai[0] = -1.0; // one tick from the first window
        s.direction = 1; // already facing right
        // The player is to the left, but the slime is not active (day, full health, above the
        // surface, no slime rain), so it should keep its existing facing rather than turn to it.
        update(&mut s, Some(player(-5000.0)), true, false);
        assert_eq!(s.direction, 1, "a passive slime should not reface mid-hop");
    }

    /// The same hop, but active: now it does turn to face the player.
    #[test]
    fn an_active_slime_does_reface_on_a_hop() {
        let mut s = slime(1);
        s.ai[0] = -1.0;
        s.direction = 1;
        update(&mut s, Some(player(-5000.0)), true, true);
        assert_eq!(
            s.direction, -1,
            "an active slime should turn to face the player"
        );
    }

    #[test]
    fn the_long_hop_is_higher_and_further_than_the_short_one() {
        let mut s = slime(1);
        s.ai[0] = -1751.0; // one tick from the third window
        update(&mut s, Some(player(5000.0)), true, false);
        assert_eq!(s.velocity.1, BIG_HOP_Y);
        assert_eq!(s.ai[0], -200.0, "the long hop resets the timer to -200");
        // The long hop is the one that carries a slime across a gap.
        assert_eq!(BIG_HOP_Y, -8.0);
        assert_eq!(HOP_Y, -6.0);
    }

    #[test]
    fn the_timer_resets_deep_enough_to_pause_between_hops() {
        let mut s = slime(1);
        s.ai[0] = 0.0;
        update(&mut s, Some(player(5000.0)), true, false);
        // Window 1 resets to -120 + -1000.
        assert_eq!(s.ai[0], -1120.0);
        assert_eq!(
            hop_kind(s.ai[0], -1000.0),
            0,
            "and lands outside every window"
        );
    }

    #[test]
    fn a_twitchier_slime_fills_its_timer_faster() {
        let mut plain = slime(1);
        let mut lava = slime(59);
        plain.ai[0] = -5000.0;
        lava.ai[0] = -5000.0;
        update(&mut plain, None, true, false);
        update(&mut lava, None, true, false);
        assert!(
            lava.ai[0] > plain.ai[0],
            "LavaSlime should gain more per tick: {} vs {}",
            lava.ai[0],
            plain.ai[0]
        );
    }

    #[test]
    fn an_active_slime_fills_its_hop_clock_twice_as_fast() {
        // Type 1 has no per-type bonus, so the only difference between these two is the active flag
        // (night / hurt / below the surface / slime rain). Both start far below every hop window so
        // the tick only advances the clock — it does not fire a hop and reset it.
        let mut passive = slime(1);
        let mut active = slime(1);
        passive.ai[0] = -5000.0;
        active.ai[0] = -5000.0;
        update(&mut passive, None, true, false);
        update(&mut active, None, true, true);
        assert_eq!(
            passive.ai[0], -4999.0,
            "a passive slime ticks its clock once"
        );
        assert_eq!(
            active.ai[0], -4998.0,
            "an active slime ticks it twice, so it reaches its next hop in half the frames"
        );
    }

    #[test]
    fn a_slime_cycles_through_hops_rather_than_repeating_one() {
        // Run a long stretch and check it hops several times at a plausible cadence.
        let mut s = slime(1);
        s.ai[0] = -2100.0;
        let mut hops = 0;
        for _ in 0..3000 {
            let before = s.velocity.1;
            update(&mut s, Some(player(5000.0)), true, false);
            if s.velocity.1 < 0.0 && before >= 0.0 {
                hops += 1;
            }
            s.velocity.1 = 0.0; // simulate landing again
        }
        assert!(
            (5..=30).contains(&hops),
            "expected a handful of hops in 3000 ticks, got {hops}"
        );
    }
}
