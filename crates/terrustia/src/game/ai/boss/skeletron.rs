//! Styles 11 and 12 — Skeletron and its hands.
//!
//! The head alternates between two states on a fixed clock, and the whole fight is built around
//! that clock. For thirteen seconds it **hovers**, holding two hundred and fifty pixels above you
//! and drifting sideways to stay overhead — this is the safe half, and it is when the hands come
//! for you. Then for six and a half seconds it **spins**, dropping ten points of defence and
//! grinding straight at you. That defence drop is the whole window: the fight is a matter of not
//! dying to the hands during the hover so that you can hit the head during the spin.
//!
//! Daylight does not end this fight. The head simply becomes unkillable and lethal — nine thousand
//! damage, nine thousand defence — and comes at you at eight pixels a tick. The Dungeon Guardian is
//! the same routine permanently in that state, which is why it is not a boss so much as a warning.
//!
//! A **hand** docks beside the head, and every five seconds winds up: it climbs above the head,
//! aims once, and throws itself down at eighteen pixels a tick. It gives up the moment it has
//! passed you, turned, or gone too far, and drifts back to its dock.

use terrustia_proto::npc_params::{
    HAND_DOCK_HIGH, HAND_DOCK_HIGH_DRIVE, HAND_DOCK_LOW, HAND_DOCK_LOW_DRIVE, HAND_LUNGE,
    HAND_LUNGE_LIMIT, HAND_RISE, HAND_RISE_ABOVE, HAND_RISE_CAP, HAND_SWEEP, HAND_SWEEP_CAP,
    HAND_WINDUP_AT, SKELETRON_ENRAGED_SPEED, SKELETRON_ENRAGED_STAT, SKELETRON_GIVE_UP,
    SKELETRON_HAND, SKELETRON_HOVER, SKELETRON_HOVER_ABOVE, SKELETRON_HOVER_TICKS,
    SKELETRON_SPIN_DEFENSE, SKELETRON_SPIN_RATE, SKELETRON_SPIN_SPEED, SKELETRON_SPIN_TICKS,
};

use crate::game::ai::World;
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Spawn;

/// The head's states, as `ai[1]` records them.
pub const HOVERING: f32 = 0.0;
pub const SPINNING: f32 = 1.0;
pub const ENRAGED: f32 = 2.0;
pub const LEAVING: f32 = 3.0;

/// Where a boss part's parent is, how big it is, and which state it is in.
pub type Parent = ((f32, f32), (f32, f32));

/// The Dungeon Guardian, which is Skeletron's routine with no off switch.
const DUNGEON_GUARDIAN: u16 = 68;

/// Drive one axis toward a wanted position, easing off whatever it was doing the other way.
fn drift(velocity: &mut f32, here: f32, wanted: f32, accel: f32, cap: f32) {
    if here > wanted {
        if *velocity > 0.0 {
            *velocity *= 0.98;
        }
        *velocity -= accel;
        if *velocity > cap {
            *velocity = cap;
        }
    } else if here < wanted {
        if *velocity < 0.0 {
            *velocity *= 0.98;
        }
        *velocity += accel;
        if *velocity < -cap {
            *velocity = -cap;
        }
    }
}

/// Drive Skeletron's head for a tick, returning the hands it wants raised.
pub fn head<T: TileView>(npc: &mut Npc, world: &World<'_, T>) -> Vec<Spawn> {
    let mut hands = Vec::new();
    let guardian = npc.npc_type == DUNGEON_GUARDIAN;

    // First tick: a head raises two hands, one to each side, out of step with each other.
    if npc.ai[0] == 0.0 {
        npc.ai[0] = 1.0;
        if !guardian {
            for side in [-1.0, 1.0] {
                hands.push(Spawn {
                    npc_type: SKELETRON_HAND,
                    position: (
                        npc.position.0 + npc.width() / 2.0,
                        npc.position.1 + npc.height() / 2.0,
                    ),
                    velocity: (side, 0.0),
                    parent: Some(Spawn::OWN_PARENT),
                });
            }
        }
        npc.dirty = true;
    }

    npc.stats.defense = npc.stats.defense.max(0);

    // Nobody within two thousand pixels on either axis, or nobody left alive.
    let abandoned = match world.target {
        None => true,
        Some(t) => {
            !t.alive
                || (npc.position.0 - t.center.0).abs() > SKELETRON_GIVE_UP
                || (npc.position.1 - t.center.1).abs() > SKELETRON_GIVE_UP
        }
    };
    if abandoned {
        npc.ai[1] = LEAVING;
    } else if (guardian || world.conditions.day) && npc.ai[1] != LEAVING && npc.ai[1] != ENRAGED {
        // Dawn does not save you here.
        npc.ai[1] = ENRAGED;
    }

    let Some(target) = world.target else {
        npc.velocity.1 += 0.1;
        npc.time_left = npc.time_left.min(50);
        return hands;
    };
    let (cx, cy) = npc.center();

    if npc.ai[1] == HOVERING {
        npc.ai[2] += 1.0;
        if npc.ai[2] >= SKELETRON_HOVER_TICKS {
            npc.ai[2] = 0.0;
            npc.ai[1] = SPINNING;
            npc.dirty = true;
        }
        npc.rotation = npc.velocity.0 / 15.0;
        let (up_accel, up_cap, across_accel, across_cap) = SKELETRON_HOVER;
        drift(
            &mut npc.velocity.1,
            npc.position.1,
            target.center.1 - crate::game::ai::PLAYER_HEIGHT as f32 / 2.0 - SKELETRON_HOVER_ABOVE,
            up_accel,
            up_cap,
        );
        drift(
            &mut npc.velocity.0,
            cx,
            target.center.0,
            across_accel,
            across_cap,
        );
    } else if npc.ai[1] == SPINNING {
        // The window: ten points off its defence while it grinds at you.
        npc.stats.defense -= SKELETRON_SPIN_DEFENSE;
        npc.ai[2] += 1.0;
        if npc.ai[2] >= SKELETRON_SPIN_TICKS {
            npc.ai[2] = 0.0;
            npc.ai[1] = HOVERING;
            npc.dirty = true;
        }
        npc.rotation += f32::from(npc.direction) * SKELETRON_SPIN_RATE;
        let (dx, dy) = (target.center.0 - cx, target.center.1 - cy);
        let reach = (dx * dx + dy * dy).sqrt().max(0.01);
        let k = SKELETRON_SPIN_SPEED / reach;
        npc.velocity = (dx * k, dy * k);
    } else if npc.ai[1] == ENRAGED {
        // Untouchable and fatal.
        npc.stats.damage = SKELETRON_ENRAGED_STAT;
        npc.stats.defense = SKELETRON_ENRAGED_STAT;
        npc.rotation += f32::from(npc.direction) * SKELETRON_SPIN_RATE;
        let (dx, dy) = (target.center.0 - cx, target.center.1 - cy);
        let reach = (dx * dx + dy * dy).sqrt().max(0.01);
        let k = SKELETRON_ENRAGED_SPEED / reach;
        npc.velocity = (dx * k, dy * k);
    } else {
        npc.velocity.1 += 0.1;
        if npc.velocity.1 < 0.0 {
            npc.velocity.1 *= 0.95;
        }
        npc.velocity.0 *= 0.95;
        npc.time_left = npc.time_left.min(50);
    }

    npc.dirty = true;
    hands
}

/// What a hand's tick concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandOutcome {
    Attached,
    /// Its head is gone, so it is too.
    Orphaned,
}

/// Drive one of Skeletron's hands for a tick.
///
/// `head_at` is the head's position and size, and `head_hovering` says whether the head is in the
/// half of its cycle during which the hands come at you.
pub fn hand(
    npc: &mut Npc,
    head_at: Option<Parent>,
    head_hovering: bool,
    head_leaving: bool,
    target: Option<crate::game::npc_ai::Target>,
) -> HandOutcome {
    let Some((head_position, head_size)) = head_at else {
        return HandOutcome::Orphaned;
    };
    // `ai[0]` is which side of the head it belongs to.
    let side = if npc.ai[0] < 0.0 { -1.0 } else { 1.0 };
    npc.sprite_direction = -(side as i8);
    if head_leaving {
        npc.time_left = npc.time_left.min(10);
    }

    let dock = |offset: (f32, f32)| {
        (
            head_position.0 + head_size.0 / 2.0 - offset.0 * side,
            head_position.1 + offset.1,
        )
    };
    let (cx, cy) = (
        npc.position.0 + npc.width() * 0.5,
        npc.position.1 + npc.height() * 0.5,
    );
    let half_width = npc.width() / 2.0;

    match npc.ai[2] as i32 {
        // Docked, either close in beside the head or hanging low beneath it.
        0 | 3 => {
            if head_hovering {
                let at = dock(HAND_DOCK_HIGH);
                let (uy, cyv, ux, cxv) = HAND_DOCK_HIGH_DRIVE;
                drift(&mut npc.velocity.1, npc.position.1, at.1, uy, cyv);
                let here = npc.position.0 + half_width;
                drift(&mut npc.velocity.0, here, at.0, ux, cxv);
            } else {
                let at = dock(HAND_DOCK_LOW);
                let (uy, cyv, ux, cxv) = HAND_DOCK_LOW_DRIVE;
                drift(&mut npc.velocity.1, npc.position.1, at.1, uy, cyv);
                let here = npc.position.0 + half_width;
                drift(&mut npc.velocity.0, here, at.0, ux, cxv);
                // Only from the low dock does it wind itself up.
                npc.ai[3] += 1.0;
                if npc.ai[3] >= HAND_WINDUP_AT {
                    npc.ai[2] += 1.0;
                    npc.ai[3] = 0.0;
                    npc.dirty = true;
                }
            }
            let at = dock(HAND_DOCK_LOW);
            npc.rotation = (at.1 - cy).atan2(at.0 - cx) + 1.57;
        }
        // Winding up: it climbs above the head and then commits.
        1 => {
            npc.velocity.0 *= 0.95;
            npc.velocity.1 -= HAND_RISE;
            if npc.velocity.1 < -HAND_RISE_CAP {
                npc.velocity.1 = -HAND_RISE_CAP;
            }
            if npc.position.1 < head_position.1 - HAND_RISE_ABOVE
                && let Some(t) = target
            {
                let (dx, dy) = (t.center.0 - cx, t.center.1 - cy);
                let reach = (dx * dx + dy * dy).sqrt().max(0.01);
                let k = HAND_LUNGE / reach;
                npc.velocity = (dx * k, dy * k);
                npc.ai[2] = 2.0;
                npc.dirty = true;
            }
        }
        // The lunge, which is over the moment it has passed you or turned.
        2 => {
            let done = match target {
                None => true,
                Some(t) => {
                    let toward = (t.center.0 - cx, t.center.1 - cy);
                    npc.position.1 > t.center.1
                        || npc.velocity.0 * toward.0 + npc.velocity.1 * toward.1 <= 0.0
                        || (toward.0 * toward.0 + toward.1 * toward.1).sqrt() > HAND_LUNGE_LIMIT
                        || npc.velocity.1 < 0.0
                }
            };
            if done {
                npc.ai[2] = 3.0;
                npc.dirty = true;
            }
        }
        // The sideways sweep, and its end.
        4 => {
            npc.velocity.1 *= 0.95;
            npc.velocity.0 += HAND_SWEEP * -side;
            npc.velocity.0 = npc.velocity.0.clamp(-HAND_SWEEP_CAP, HAND_SWEEP_CAP);
            if let Some(t) = target
                && ((npc.velocity.0 > 0.0 && cx > t.center.0)
                    || (npc.velocity.0 < 0.0 && cx < t.center.0))
            {
                npc.ai[2] = 5.0;
                npc.dirty = true;
            }
        }
        _ => {
            npc.ai[2] = 0.0;
            npc.dirty = true;
        }
    }

    npc.dirty = true;
    HandOutcome::Attached
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use terrustia_proto::tile::Tile;

    struct Dungeon;

    impl TileView for Dungeon {
        fn tile(&self, _x: i32, _y: i32) -> Tile {
            Tile::AIR
        }
    }

    fn world<'a>(tiles: &'a Dungeon, target: Option<Target>) -> World<'a, Dungeon> {
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

    fn skeletron() -> Npc {
        Npc::new(35, (10_000.0, 9_600.0), 1).expect("skeletron")
    }

    #[test]
    fn it_raises_two_hands_on_its_first_tick_and_no_more() {
        let tiles = Dungeon;
        let mut s = skeletron();
        let t = Some(player_at(10_000.0, 10_000.0));
        let raised = head(&mut s, &world(&tiles, t));
        assert_eq!(raised.len(), 2);
        assert!(raised.iter().all(|h| h.npc_type == SKELETRON_HAND));
        assert!(
            raised[0].velocity.0 != raised[1].velocity.0,
            "one to each side"
        );
        assert!(head(&mut s, &world(&tiles, t)).is_empty());
    }

    #[test]
    fn the_dungeon_guardian_has_no_hands_and_is_always_enraged() {
        let tiles = Dungeon;
        let mut g = Npc::new(68, (10_000.0, 9_600.0), 1).expect("dungeon guardian");
        let t = Some(player_at(10_000.0, 10_000.0));
        let raised = head(&mut g, &world(&tiles, t));
        assert!(raised.is_empty());
        assert_eq!(g.ai[1], ENRAGED);
        assert_eq!(g.stats.defense, SKELETRON_ENRAGED_STAT);
    }

    #[test]
    fn it_hovers_above_you_then_spins_at_you() {
        let tiles = Dungeon;
        let mut s = skeletron();
        let t = Some(player_at(10_000.0, 10_000.0));
        for _ in 0..(SKELETRON_HOVER_TICKS as i32 + 1) {
            head(&mut s, &world(&tiles, t));
        }
        assert_eq!(s.ai[1], SPINNING, "should have started spinning");
        let before = s.rotation;
        head(&mut s, &world(&tiles, t));
        assert!(s.rotation != before, "and be turning");

        for _ in 0..(SKELETRON_SPIN_TICKS as i32 + 1) {
            head(&mut s, &world(&tiles, t));
        }
        assert_eq!(s.ai[1], HOVERING, "and settle again");
    }

    /// The whole fight in one number: the spin is when the head can actually be hurt.
    #[test]
    fn spinning_is_when_its_guard_drops() {
        let tiles = Dungeon;
        let mut s = skeletron();
        let t = Some(player_at(10_000.0, 10_000.0));
        head(&mut s, &world(&tiles, t));
        let guarded = s.stats.defense;
        s.ai[1] = SPINNING;
        head(&mut s, &world(&tiles, t));
        assert_eq!(s.stats.defense, guarded - SKELETRON_SPIN_DEFENSE);
    }

    #[test]
    fn daylight_makes_it_lethal_rather_than_ending_it() {
        let tiles = Dungeon;
        let mut s = skeletron();
        let t = Some(player_at(10_000.0, 10_000.0));
        let mut day = world(&tiles, t);
        day.conditions.day = true;
        head(&mut s, &day);
        head(&mut s, &day);
        assert_eq!(s.ai[1], ENRAGED);
        assert_eq!(s.stats.damage, SKELETRON_ENRAGED_STAT);
        assert!(s.time_left > 50, "it does not leave, it kills you");
    }

    #[test]
    fn a_player_who_runs_far_enough_ends_it() {
        let tiles = Dungeon;
        let mut s = skeletron();
        let t = Some(player_at(10_000.0 + SKELETRON_GIVE_UP + 100.0, 10_000.0));
        head(&mut s, &world(&tiles, t));
        head(&mut s, &world(&tiles, t));
        assert_eq!(s.ai[1], LEAVING);
        assert!(s.time_left <= 50);
    }

    fn a_hand() -> Npc {
        let mut n = Npc::new(36, (10_000.0, 9_800.0), 1).expect("skeletron hand");
        n.ai[0] = 1.0;
        n
    }

    #[test]
    fn a_hand_without_a_head_is_finished() {
        let mut h = a_hand();
        assert_eq!(hand(&mut h, None, true, false, None), HandOutcome::Orphaned);
    }

    #[test]
    fn a_hand_winds_up_from_its_low_dock_and_lunges() {
        let mut h = a_hand();
        let head_at = Some(((10_000.0, 9_600.0), (100.0, 100.0)));
        let t = Some(player_at(10_000.0, 10_400.0));
        // Docked low, because the head is spinning.
        for _ in 0..(HAND_WINDUP_AT as i32 + 1) {
            hand(&mut h, head_at, false, false, t);
        }
        assert_eq!(h.ai[2], 1.0, "should be winding up");

        // It climbs, and once above the head it commits.
        for _ in 0..400 {
            hand(&mut h, head_at, false, false, t);
            h.position.1 += h.velocity.1;
            if h.ai[2] == 2.0 {
                break;
            }
        }
        assert_eq!(h.ai[2], 2.0, "should have let go");
        let speed = h.velocity.0.hypot(h.velocity.1);
        assert!((speed - HAND_LUNGE).abs() < 1e-3, "at speed, got {speed}");
        assert!(h.velocity.1 > 0.0, "and downward at the player");
    }

    #[test]
    fn a_hand_gives_up_its_lunge_once_it_has_passed_you() {
        let mut h = a_hand();
        let head_at = Some(((10_000.0, 9_600.0), (100.0, 100.0)));
        h.ai[2] = 2.0;
        h.velocity = (0.0, 18.0);
        // Already below the player.
        h.position.1 = 11_000.0;
        hand(
            &mut h,
            head_at,
            false,
            false,
            Some(player_at(10_000.0, 10_400.0)),
        );
        assert_eq!(h.ai[2], 3.0, "the lunge is over");
    }

    #[test]
    fn a_hand_docks_closer_while_the_head_hovers() {
        assert!(HAND_DOCK_HIGH.1 < HAND_DOCK_LOW.1, "high dock is above");
        let mut close = a_hand();
        let mut low = a_hand();
        let head_at = Some(((10_000.0, 9_600.0), (100.0, 100.0)));
        for _ in 0..200 {
            hand(&mut close, head_at, true, false, None);
            close.position.1 += close.velocity.1;
            hand(&mut low, head_at, false, false, None);
            low.position.1 += low.velocity.1;
        }
        assert!(
            close.position.1 < low.position.1,
            "the hovering dock should be the higher one: {} against {}",
            close.position.1,
            low.position.1
        );
    }
}
