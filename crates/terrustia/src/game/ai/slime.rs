//! Style 1 — the slimes.
//!
//! Ported from `AI_001_Slimes`. A slime does nothing at all most of the time: `ai[0]` is a timer
//! that sits far below zero and counts up, and three windows on the way back to zero each trigger
//! a hop. That is why slimes move in bursts with long pauses rather than at a steady rate.
//!
//! `ai[3]` is how it notices a wall: a long hop records the X it launched from, and landing back at
//! that same X means something stopped it, so it turns round (`NPC.cs:62133-62146`). `ai[2]` is the
//! two-hundred-tick cooldown that turn starts, which is what stops an active slime from immediately
//! re-facing the player it cannot reach and hopping into the wall again.
//!
//! Deliberately not modelled: the Lava Slime's own buoyancy numbers (`type == 59 && !remixWorld`,
//! `NPC.cs:62095-62106`, which pushes up 0.8 harder when `directionY < 0` and lets it rise to -10
//! rather than -4). This port has no remix-world flag, and nothing here ever sets a slime's
//! `directionY`, so a lava slime swims by the ordinary rules below.

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

/// How long a slime that just turned off a wall refuses to re-target, from `ai[2] = 200f`.
const TURN_COOLDOWN: f32 = 200.0;

/// Turn to face a target, which is all `TargetClosest()` does that matters here.
fn face(npc: &mut Npc, target: Option<Target>) {
    if let Some(t) = target {
        npc.direction = if t.center.0 > npc.center().0 { 1 } else { -1 };
        npc.sprite_direction = npc.direction;
    }
}

/// Drive one slime. Only hops while it is standing on something, but swims whenever it is wet.
pub fn update(npc: &mut Npc, target: Option<Target>, on_ground: bool, active: bool, wet: bool) {
    // The turn cooldown runs down to 1, never to 0: 0 is the never-initialised state below.
    if npc.ai[2] > 1.0 {
        npc.ai[2] -= 1.0;
    }

    // In liquid a slime bobs rather than hops (`NPC.cs:62080-62124`): the floor pushes it up, it
    // rises half a pixel a tick to a ceiling of -4, and the same launched-from-here X test that
    // turns it off a wall on land turns it off one underwater.
    if wet {
        if npc.collide_y {
            npc.velocity.1 = -2.0;
        }
        if npc.velocity.1 < 0.0 && npc.ai[3] == npc.position.0 {
            npc.direction = -npc.direction;
            npc.ai[2] = TURN_COOLDOWN;
        }
        if npc.velocity.1 > 0.0 {
            npc.ai[3] = npc.position.0;
        }
        if npc.velocity.1 > 2.0 {
            npc.velocity.1 *= 0.9;
        }
        npc.velocity.1 = (npc.velocity.1 - 0.5).max(-4.0);
        if npc.ai[2] == 1.0 && active {
            face(npc, target);
        }
        npc.dirty = true;
    }

    // A fresh slime has never run: vanilla settles it here, facing whatever is closest and putting
    // its first hop a hundred ticks out (`NPC.cs:62128-62132`).
    if npc.ai[2] == 0.0 {
        npc.ai[0] = -100.0;
        npc.ai[2] = 1.0;
        face(npc, target);
    }

    if !on_ground {
        return;
    }

    // Landing at the exact X a long hop launched from means a wall stopped it, so turn round and
    // hold that new direction for `TURN_COOLDOWN` ticks (`NPC.cs:62139-62146`). Without this a
    // slime facing a wall hops into it for ever: `ai[3]` was written and never read.
    if npc.ai[3] == npc.position.0 {
        npc.direction = -npc.direction;
        npc.ai[2] = TURN_COOLDOWN;
        npc.dirty = true;
    }
    npc.ai[3] = 0.0;

    // Ground friction while it waits for the next hop.
    npc.velocity.0 *= 0.8;
    if npc.velocity.0.abs() < 0.1 {
        npc.velocity.0 = 0.0;
    }

    // Vanilla ticks the hop clock once a frame, and a second time when the slime is "active"
    // (`NPC.AI_001_Slimes`' own `flag3`) — so an active slime reaches its next hop in half the
    // frames, on top of whatever per-type bonus it carries.
    npc.ai[0] += 1.0 + slime_timer_bonus(npc.npc_type) + f32::from(u8::from(active));
    let window = slime_hop_window(npc.npc_type);
    let kind = hop_kind(npc.ai[0], window);
    if kind == 0 {
        return;
    }

    // Vanilla only re-targets here (`TargetClosest()`, which is also what turns the slime to face
    // whatever it picks) while active (`flag3`), gated further by its own `ai[2] == 1f`: the
    // cooldown a turn off a wall starts, without which the slime would face the unreachable player
    // again on the very next hop and go straight back into the wall. NPC.cs:62245-62248. A passive
    // slime hopping in daylight keeps whatever direction it already had instead of re-facing.
    if active && npc.ai[2] == 1.0 {
        face(npc, target);
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

    /// A slime past its first tick: `ai[2] == 1` is the settled state vanilla's own `ai[2] == 0f`
    /// branch puts a freshly spawned one into (`NPC.cs:62128-62132`), and every test below that
    /// sets its own `ai[0]` wants a slime whose clock is not about to be reset to -100.
    fn slime(npc_type: u16) -> Npc {
        let mut npc = Npc::new(npc_type, (1000.0, 1000.0), 1).expect("slime type");
        npc.ai[2] = 1.0;
        npc
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
        update(&mut s, Some(player(2000.0)), false, false, false);
        assert_eq!(s.velocity, (0.0, 0.0), "no hop while airborne");
    }

    #[test]
    fn a_slime_hops_toward_its_target_when_the_timer_comes_round() {
        let mut s = slime(1);
        s.ai[0] = -1.0; // one tick from the first window
        update(&mut s, Some(player(5000.0)), true, false, false);
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
        update(&mut s, Some(player(-5000.0)), true, false, false);
        assert_eq!(s.direction, 1, "a passive slime should not reface mid-hop");
    }

    /// The same hop, but active: now it does turn to face the player.
    #[test]
    fn an_active_slime_does_reface_on_a_hop() {
        let mut s = slime(1);
        s.ai[0] = -1.0;
        s.direction = 1;
        update(&mut s, Some(player(-5000.0)), true, true, false);
        assert_eq!(
            s.direction, -1,
            "an active slime should turn to face the player"
        );
    }

    #[test]
    fn the_long_hop_is_higher_and_further_than_the_short_one() {
        let mut s = slime(1);
        s.ai[0] = -1751.0; // one tick from the third window
        update(&mut s, Some(player(5000.0)), true, false, false);
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
        update(&mut s, Some(player(5000.0)), true, false, false);
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
        update(&mut plain, None, true, false, false);
        update(&mut lava, None, true, false, false);
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
        update(&mut passive, None, true, false, false);
        update(&mut active, None, true, true, false);
        assert_eq!(
            passive.ai[0], -4999.0,
            "a passive slime ticks its clock once"
        );
        assert_eq!(
            active.ai[0], -4998.0,
            "an active slime ticks it twice, so it reaches its next hop in half the frames"
        );
    }

    /// BA3-02, fail-then-pass: a long hop that gets nowhere turns the slime round.
    ///
    /// `NPC.cs:62139-62146`: landing at the exact X the hop launched from means a wall stopped it,
    /// so `direction *= -1` and `ai[2] = 200f`. `ai[3]` used to be written by the long hop and read
    /// by nothing at all, so a slime facing a wall hopped into it for ever.
    #[test]
    fn a_slime_that_lands_where_it_launched_turns_around() {
        let mut s = slime(1);
        s.direction = 1;
        s.ai[0] = -1751.0; // one tick from the long hop
        update(&mut s, None, true, false, false);
        assert_eq!(s.ai[3], s.position.0, "the long hop records its launch X");

        // It went nowhere: a wall held it at the same X, and it lands there.
        update(&mut s, None, true, false, false);
        assert_eq!(
            s.direction, -1,
            "a hop that got nowhere has to turn it round"
        );
        assert_eq!(s.ai[2], TURN_COOLDOWN, "and start the re-target cooldown");
        assert_eq!(s.ai[3], 0.0, "the launch X is cleared on landing");
    }

    /// ...and a hop that actually travelled does not.
    #[test]
    fn a_slime_that_moves_keeps_going() {
        let mut s = slime(1);
        s.direction = 1;
        s.ai[0] = -1751.0;
        update(&mut s, None, true, false, false);
        s.position.0 += 40.0; // the hop carried it clear
        update(&mut s, None, true, false, false);
        assert_eq!(s.direction, 1, "nothing stopped it, so it keeps facing");
        assert_eq!(s.ai[2], 1.0, "and no cooldown is started");
    }

    /// The cooldown is what makes the turn stick: an active slime re-faces its target on every hop
    /// (`flag3 && ai[2] == 1f`), so without `ai[2]` it would turn back into the wall immediately.
    #[test]
    fn the_turn_cooldown_holds_an_active_slime_off_its_target() {
        let mut s = slime(1);
        // Facing away from the player, as it would be just after turning off a wall.
        s.direction = 1;
        s.ai[2] = TURN_COOLDOWN;
        s.ai[0] = -1.0;
        update(&mut s, Some(player(-5000.0)), true, true, false);
        assert_eq!(s.direction, 1, "still on cooldown, so it does not re-face");

        s.ai[2] = 1.0;
        s.ai[0] = -1.0;
        update(&mut s, Some(player(-5000.0)), true, true, false);
        assert_eq!(s.direction, -1, "and once it expires the target wins again");
    }

    /// A slime in water bobs upward instead of falling, and cannot exceed -4
    /// (`NPC.cs:62080-62124`). It was falling like a stone before, because `wet` never reached it.
    #[test]
    fn a_wet_slime_swims_up() {
        let mut s = slime(1);
        s.velocity.1 = 3.0;
        update(&mut s, None, false, false, true);
        // 3 is over 2, so it is damped first and then pushed up.
        assert!((s.velocity.1 - (3.0 * 0.9 - 0.5)).abs() < 1e-6, "{s:?}");

        for _ in 0..100 {
            update(&mut s, None, false, false, true);
        }
        assert_eq!(s.velocity.1, -4.0, "and it tops out rising, not falling");
    }

    /// Touching a floor underwater is a push off it, not a stop.
    #[test]
    fn a_wet_slime_pushes_off_the_bottom() {
        let mut s = slime(1);
        s.collide_y = true;
        s.velocity.1 = 4.0;
        s.ai[0] = -5000.0; // far from any hop window, so only the bob moves it
        update(&mut s, None, true, false, true);
        assert_eq!(
            s.velocity.1, -2.5,
            "the floor throws it back up (-2), then the usual half a pixel of lift"
        );
    }

    /// A freshly spawned slime settles before it does anything, which is where `ai[2]` gets its
    /// initial 1 (`NPC.cs:62128-62132`).
    #[test]
    fn a_fresh_slime_settles_before_its_first_hop() {
        let mut s = Npc::new(1, (1000.0, 1000.0), 1).expect("slime");
        assert_eq!(s.ai[2], 0.0, "a fresh slime has never run");
        s.ai[0] = -1.0; // would otherwise hop on this very tick
        update(&mut s, Some(player(5000.0)), true, false, false);
        assert_eq!(
            s.ai[0], -99.0,
            "the settle put its first hop a hundred ticks out, less this tick"
        );
        assert_eq!(s.velocity.1, 0.0, "so nothing hops yet");
        assert_eq!(s.ai[2], 1.0);
        assert_eq!(s.direction, 1, "and it faces what is closest");
    }

    #[test]
    fn a_slime_cycles_through_hops_rather_than_repeating_one() {
        // Run a long stretch and check it hops several times at a plausible cadence.
        let mut s = slime(1);
        s.ai[0] = -2100.0;
        let mut hops = 0;
        for _ in 0..3000 {
            let before = s.velocity.1;
            update(&mut s, Some(player(5000.0)), true, false, false);
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
