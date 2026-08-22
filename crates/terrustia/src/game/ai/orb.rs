//! Style 9 — the caster orbs.
//!
//! Burning Sphere, Chaos Ball, Water Sphere and the Vile Spit. Terraria implements a caster's
//! "projectile" as an NPC with one hit point and no gravity, which is why the pre-hardmode casters
//! need no projectile subsystem at all.
//!
//! Ported from the `aiStyle == 9` block. The behaviour that matters: an orb aims **once**, on the
//! tick it has no target yet, and then flies straight forever. It does not steer after that.

use crate::game::npc::Npc;
use crate::game::npc_ai::Target;

/// Flight speed by type, from the `num125` selection at the top of the block.
///
/// The default is 6; the Burning Sphere is slower and the Vile Spits are faster.
pub fn speed(npc_type: u16) -> f32 {
    match npc_type {
        25 => 5.0,        // BurningSphere
        112 | 666 => 7.0, // VileSpit, VileSpitEaterOfWorlds
        _ => 6.0,
    }
}

/// Whether this orb has already chosen its heading.
///
/// The game uses `target == 255` for this: an orb spawns with no target, aims on its first tick,
/// and from then on has one, so the aiming branch never runs again.
fn has_aimed(npc: &Npc) -> bool {
    npc.target != 255
}

/// Drive one orb.
pub fn update(npc: &mut Npc, target: Option<Target>) {
    if has_aimed(npc) {
        // Already committed to a heading; nothing steers it.
        return;
    }
    let Some(t) = target else {
        return;
    };

    npc.target = u16::from(t.slot);

    let (cx, cy) = npc.center();
    let (dx, dy) = (t.center.0 - cx, t.center.1 - cy);
    // The game guards a zero distance by substituting 1 rather than skipping the shot.
    let distance = {
        let d = (dx * dx + dy * dy).sqrt();
        if d <= 0.0 { 1.0 } else { d }
    };
    let scale = speed(npc.npc_type) / distance;
    npc.velocity = (dx * scale, dy * scale);
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orb(npc_type: u16, at: (f32, f32)) -> Npc {
        let mut npc = Npc::new(npc_type, at, 1).expect("orb type");
        npc.target = 255; // freshly spawned, not yet aimed
        npc
    }

    fn player(x: f32, y: f32) -> Target {
        Target {
            slot: 2,
            center: (x, y),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    #[test]
    fn speeds_match_the_types_the_game_singles_out() {
        assert_eq!(speed(25), 5.0, "BurningSphere");
        assert_eq!(speed(112), 7.0, "VileSpit");
        assert_eq!(speed(666), 7.0, "VileSpitEaterOfWorlds");
        assert_eq!(speed(30), 6.0, "ChaosBall takes the default");
        assert_eq!(speed(33), 6.0, "WaterSphere takes the default");
    }

    #[test]
    fn an_orb_aims_at_its_target_at_the_right_speed() {
        // Player directly to the right, 100 pixels away.
        let mut o = orb(30, (0.0, 0.0));
        let (cx, cy) = o.center();
        update(&mut o, Some(player(cx + 100.0, cy)));

        assert_eq!(o.velocity.1, 0.0, "no vertical component for a level shot");
        assert!(
            (o.velocity.0 - 6.0).abs() < 0.001,
            "should fly at its type speed, got {}",
            o.velocity.0
        );
    }

    #[test]
    fn the_heading_is_chosen_once_and_never_revised() {
        let mut o = orb(30, (0.0, 0.0));
        let (cx, cy) = o.center();
        update(&mut o, Some(player(cx + 100.0, cy)));
        let first = o.velocity;

        // The player runs the other way; the orb must not follow.
        update(&mut o, Some(player(cx - 500.0, cy + 300.0)));
        assert_eq!(o.velocity, first, "an orb does not steer after it is fired");
    }

    #[test]
    fn a_diagonal_shot_keeps_its_total_speed() {
        let mut o = orb(25, (0.0, 0.0));
        let (cx, cy) = o.center();
        update(&mut o, Some(player(cx + 300.0, cy + 400.0)));
        let magnitude = (o.velocity.0.powi(2) + o.velocity.1.powi(2)).sqrt();
        assert!(
            (magnitude - 5.0).abs() < 0.001,
            "speed should be 5 regardless of direction, got {magnitude}"
        );
    }

    #[test]
    fn a_target_on_top_of_the_orb_does_not_divide_by_zero() {
        // The game substitutes a distance of 1 rather than skipping.
        let mut o = orb(30, (0.0, 0.0));
        let here = o.center();
        update(&mut o, Some(player(here.0, here.1)));
        assert!(o.velocity.0.is_finite() && o.velocity.1.is_finite());
    }

    #[test]
    fn an_orb_with_nobody_to_aim_at_holds_still() {
        let mut o = orb(30, (0.0, 0.0));
        update(&mut o, None);
        assert_eq!(o.velocity, (0.0, 0.0));
        assert_eq!(o.target, 255, "still unaimed, so it will aim later");
    }
}
