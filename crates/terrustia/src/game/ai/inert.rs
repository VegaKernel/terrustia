//! Style 0 — the NPCs that do not move.
//!
//! Bound townsfolk waiting to be freed, and a few others. Ported from the `aiStyle == 0` block,
//! which after the talk handling amounts to: face the nearest player, and nothing else.

use crate::game::npc::Npc;
use crate::game::npc_ai::Target;

/// Types that ignore even the player and never turn.
pub fn always_faces_away(npc_type: u16) -> bool {
    matches!(npc_type, 376 | 579)
}

pub fn update(npc: &mut Npc, target: Option<Target>) {
    // Whatever momentum it had bleeds off; these never propel themselves.
    npc.velocity.0 *= 0.9;
    if npc.velocity.0.abs() < 0.05 {
        npc.velocity.0 = 0.0;
    }
    if always_faces_away(npc.npc_type) {
        return;
    }
    if let Some(t) = target {
        npc.direction = if t.center.0 > npc.center().0 { 1 } else { -1 };
        npc.sprite_direction = npc.direction;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound() -> Npc {
        Npc::new(105, (1000.0, 1000.0), 1).expect("bound goblin")
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
    fn a_bound_npc_faces_whoever_is_nearest_and_stays_put() {
        let mut n = bound();
        let start = n.position;
        n.velocity = (2.0, 0.0);
        for _ in 0..50 {
            update(&mut n, Some(player(5000.0)));
        }
        assert_eq!(n.direction, 1, "should turn toward the player");
        assert_eq!(n.velocity.0, 0.0, "and stop moving");
        assert_eq!(n.position, start, "the routine never moves it itself");
    }

    #[test]
    fn it_turns_the_other_way_when_the_player_walks_past() {
        let mut n = bound();
        update(&mut n, Some(player(0.0)));
        assert_eq!(n.direction, -1);
    }

    #[test]
    fn the_two_fixed_types_never_turn() {
        assert!(always_faces_away(376) && always_faces_away(579));
        let mut n = Npc::new(376, (1000.0, 1000.0), 1).unwrap();
        n.direction = -1;
        update(&mut n, Some(player(5000.0)));
        assert_eq!(n.direction, -1, "should not have turned");
    }
}
