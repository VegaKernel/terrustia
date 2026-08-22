//! The Eternia Crystal and its lane portals: styles 105 and 106.
//!
//! The crystal is the only NPC in the game you lose by letting die. It does nothing but stand
//! there and count, and the two things that hang off it — the portals it puts at the arena's edges
//! — are pure timers: every so many ticks a gate lets one more enemy out, and the rate is the
//! wave's, not the gate's.
//!
//! Both endings are theatre. Win or lose, the crystal spends ten seconds coming apart before it
//! actually goes, which is what gives the event its ending rather than just stopping.

use terrustia_proto::npc_params::{
    CRYSTAL_DRAMA, CRYSTAL_TICK, LANE_PORTAL_CLOSING, LANE_PORTAL_INSET, LANE_PORTAL_OPENING,
};

use crate::game::npc::Npc;

/// How the crystal is doing, as `ai[1]` numbers it.
mod phase {
    /// Standing, and being defended.
    pub const STANDING: f32 = 0.0;
    /// Lost: it is coming apart and the event is over. Set from outside, by the killing blow.
    #[cfg(test)]
    pub const LOST: f32 = 1.0;
    /// Won: it is coming apart in a nicer colour.
    pub const WON: f32 = 2.0;
}

/// A gate the crystal wants raised: where in tiles, and which side of the arena it is.
pub type Gate = (i32, i32, bool);

/// What the crystal or a portal did this tick.
#[derive(Debug, Default)]
pub struct CrystalOutcome {
    /// Gates it wants placed, as (tile x, tile y, left gate).
    pub gates: Vec<Gate>,
    /// Set on the tick the drama finishes and the event should actually end.
    pub ended: Option<bool>,
    /// Set when this NPC is finished.
    pub spent: bool,
    /// Set when a gate wants an enemy let out, and from which side.
    pub release: Option<bool>,
    /// Set when the portals should be told to close.
    pub close_gates: bool,
}

/// Style 105: the Eternia Crystal.
///
/// `arena` is the arena's two ends in tiles, worked out from where the crystal stands.
pub fn crystal(npc: &mut Npc, arena: Option<((i32, i32), (i32, i32))>) -> CrystalOutcome {
    let mut out = CrystalOutcome::default();
    npc.dirty = true;

    if npc.ai[1] == phase::STANDING {
        if npc.ai[0] > 0.0 {
            npc.ai[0] -= 1.0;
        }
        if npc.ai[0] == 0.0 {
            npc.ai[0] = CRYSTAL_TICK;
            // The gates go up once, on the first count, at the arena's ends pulled two tiles in.
            if npc.local_ai[0] == 0.0
                && let Some((left, right)) = arena
            {
                npc.local_ai[0] = 1.0;
                out.gates.push((left.0 + LANE_PORTAL_INSET, left.1, true));
                out.gates
                    .push((right.0 - LANE_PORTAL_INSET, right.1, false));
            }
        }
        return out;
    }

    // Both endings: it cannot be hurt further, it is at full life, and it rises as it goes.
    npc.invulnerable = true;
    npc.life = npc.life_max;
    npc.no_gravity = true;
    let won = npc.ai[1] == phase::WON;

    if npc.ai[0] == if won { 3.0 } else { 0.0 } {
        out.close_gates = true;
    }
    npc.ai[0] += 1.0;

    if won {
        // It bobs for two seconds and then holds still.
        if npc.ai[0] <= 120.0 {
            let along = npc.ai[0] / 120.0;
            npc.velocity.1 = (along * std::f32::consts::TAU).cos() * 0.25 - 0.25;
        } else {
            npc.velocity.1 = 0.0;
        }
    } else {
        // Losing, it sinks away instead.
        let climb = 96.0;
        if npc.ai[0] < climb {
            npc.velocity.1 = -npc.ai[0] / climb;
        }
    }

    if npc.ai[0] >= CRYSTAL_DRAMA {
        npc.invulnerable = false;
        out.ended = Some(won);
        out.spent = true;
    }
    out
}

/// Style 106: a lane portal.
///
/// `rate` is the wave's spawn rate; `on_hold` is the gap between waves, during which nothing comes
/// through; `crystal_alive` keeps a gate from closing while there is still something to defend.
pub fn portal(npc: &mut Npc, rate: i32, on_hold: bool, crystal_alive: bool) -> CrystalOutcome {
    let mut out = CrystalOutcome::default();
    npc.dirty = true;
    let left_gate = npc.ai[2] == 0.0;

    if npc.ai[1] != 0.0 {
        // Closing: it shrinks away over ten seconds and then goes.
        npc.ai[0] += 1.0;
        npc.scale = 1.0 + (0.05 - 1.0) * ((npc.ai[0] - 500.0) / 100.0).clamp(0.0, 1.0);
        if npc.ai[0] >= LANE_PORTAL_CLOSING {
            npc.invulnerable = false;
            out.spent = true;
        }
        return out;
    }

    if !on_hold {
        npc.ai[0] += 1.0;
    }
    // The counter runs to three times the rate and wraps, letting one out on each multiple. That
    // is what makes a gate's output bursty rather than metronomic when the rate changes mid-wave.
    if npc.ai[0] >= rate as f32 {
        if npc.ai[0] >= (rate * 3) as f32 {
            npc.ai[0] = 0.0;
        }
        if npc.ai[0] as i32 % rate == 0 {
            out.release = Some(left_gate);
            if on_hold {
                npc.ai[0] += 1.0;
            }
        }
    }

    // It takes three seconds to open, and cannot be hurt while it does — or ever, while the
    // crystal it belongs to is still standing.
    npc.local_ai[0] = (npc.local_ai[0] + 1.0).min(LANE_PORTAL_OPENING);
    if npc.local_ai[0] >= LANE_PORTAL_OPENING {
        npc.invulnerable = true;
        if !crystal_alive {
            npc.ai[1] = 1.0;
            npc.ai[0] = 0.0;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::npc_params::{DD2_ETERNIA_CRYSTAL, DD2_LANE_PORTAL};

    fn piece(npc_type: u16) -> Npc {
        Npc::new(npc_type, (1000.0, 1000.0), 1).expect("a piece of the army")
    }

    /// The crystal raises two gates, once, at the arena's ends.
    #[test]
    fn the_crystal_raises_two_gates_once() {
        let mut c = piece(DD2_ETERNIA_CRYSTAL);
        let arena = Some(((100, 200), (160, 200)));
        let mut gates = Vec::new();
        for _ in 0..1000 {
            gates.extend(crystal(&mut c, arena).gates);
        }
        assert_eq!(gates.len(), 2, "two gates and no more: {gates:?}");
        assert!(gates.contains(&(102, 200, true)), "left gate pulled two in");
        assert!(gates.contains(&(158, 200, false)), "right gate too");
    }

    /// With nowhere to put them it raises none, and does not spend its one chance doing so.
    #[test]
    fn no_arena_means_no_gates() {
        let mut c = piece(DD2_ETERNIA_CRYSTAL);
        for _ in 0..1000 {
            assert!(crystal(&mut c, None).gates.is_empty());
        }
        // ...and once an arena turns up, it still raises them.
        let raised: usize = (0..1000)
            .map(|_| crystal(&mut c, Some(((10, 20), (40, 20)))).gates.len())
            .sum();
        assert_eq!(raised, 2);
    }

    /// Both endings run ten seconds and then report which one it was.
    #[test]
    fn the_endings_take_ten_seconds() {
        for (phase, won) in [(phase::LOST, false), (phase::WON, true)] {
            let mut c = piece(DD2_ETERNIA_CRYSTAL);
            c.ai[1] = phase;
            let mut ended = None;
            let mut ticks = 0;
            for _ in 0..1200 {
                ticks += 1;
                let out = crystal(&mut c, None);
                if let Some(result) = out.ended {
                    ended = Some((result, ticks));
                    break;
                }
                assert!(c.invulnerable, "nothing can touch it mid-drama");
            }
            assert_eq!(ended, Some((won, CRYSTAL_DRAMA as i32)));
        }
    }

    /// A gate lets one out every `rate` ticks, and holds everything back between waves.
    #[test]
    fn a_gate_keeps_the_wave_rate() {
        let mut p = piece(DD2_LANE_PORTAL);
        let mut released = 0;
        for _ in 0..600 {
            if portal(&mut p, 60, false, true).release.is_some() {
                released += 1;
            }
        }
        assert_eq!(released, 10, "ten in six hundred ticks at sixty apiece");

        let mut held = piece(DD2_LANE_PORTAL);
        let quiet: usize = (0..600)
            .filter(|_| portal(&mut held, 60, true, true).release.is_some())
            .count();
        assert_eq!(quiet, 0, "nothing comes through between waves");
    }

    /// A gate knows which side it is.
    #[test]
    fn a_gate_knows_its_side() {
        let mut left = piece(DD2_LANE_PORTAL);
        let mut right = piece(DD2_LANE_PORTAL);
        right.ai[2] = 1.0;
        let mut sides = std::collections::HashSet::new();
        for _ in 0..200 {
            sides.extend(portal(&mut left, 60, false, true).release);
            sides.extend(portal(&mut right, 60, false, true).release);
        }
        assert_eq!(sides.len(), 2, "one gate on each side: {sides:?}");
    }

    /// Once the crystal is gone, the gates close; while it stands, they do not.
    #[test]
    fn gates_close_when_the_crystal_does() {
        let mut standing = piece(DD2_LANE_PORTAL);
        for _ in 0..2000 {
            assert!(!portal(&mut standing, 60, false, true).spent);
        }

        let mut closing = piece(DD2_LANE_PORTAL);
        let mut gone = None;
        for tick in 1..2000 {
            if portal(&mut closing, 60, false, false).spent {
                gone = Some(tick);
                break;
            }
        }
        let gone = gone.expect("it should have closed");
        assert_eq!(
            gone,
            (LANE_PORTAL_OPENING + LANE_PORTAL_CLOSING) as i32,
            "three seconds to open and ten to shrink"
        );
        // It is killed halfway through its own shrink, so it never actually reaches the small
        // end of the lerp — it vanishes at about half size.
        assert!(
            (closing.scale - 0.525).abs() < 0.01,
            "half shrunk when it goes, not {}",
            closing.scale
        );
    }
}
