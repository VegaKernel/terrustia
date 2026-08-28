//! The two-beat hopper: style 25.
//!
//! A hopper sits still until something disturbs it — a player stepping inside a box a hundred
//! pixels wider than its own, or any damage at all — and from then on it never stops. What makes
//! it read as a *hop* rather than a walk is that the two jumps differ: a long flat one, then a
//! high short one, alternating, with a pause on the ground between them that is shorter before the
//! first of each pair. Landing is what advances it; a hopper knocked into the air simply waits
//! until it comes down.

use terrustia_proto::npc_params::{
    HOP_DRAG, HOP_FIRST_REST, HOP_HIGH, HOP_LEAN, HOP_LEAN_CAP, HOP_LONG, HOP_REST, HOP_WAKE_MARGIN,
};

use crate::game::ai::{PLAYER_HEIGHT, PLAYER_WIDTH, World, face};
use crate::game::npc::{Npc, TileView};

/// Style 25.
///
/// `sleeps_through_targeting` is the one type that ignores players entirely outside its event — it
/// still hops, but never turns to face anybody.
pub fn hopper(npc: &mut Npc, world: &World<'_, impl TileView>, sleeps_through_targeting: bool) {
    npc.dirty = true;
    let look = |npc: &mut Npc| {
        if !sleeps_through_targeting && let Some(target) = world.target {
            face(npc, target);
        }
    };

    if npc.ai[0] == 0.0 {
        look(npc);
        // Anything that has already moved it counts as disturbed, which is how a hopper pushed off
        // a ledge wakes up on the way down.
        if npc.velocity.0 != 0.0 || npc.velocity.1 < 0.0 || npc.velocity.1 > 0.3 {
            npc.ai[0] = 1.0;
            return;
        }
        // Vanilla intersects two real rectangles — the wake box against the player's own hitbox —
        // not the wake box against a bare point, so the box has to be widened by the player's own
        // half-extents too, or it only wakes once you are already ten to twenty-one pixels inside.
        let woken = npc.life < npc.life_max
            || world.target.is_some_and(|t| {
                let (cx, cy) = npc.center();
                (t.center.0 - cx).abs()
                    < npc.width() / 2.0 + HOP_WAKE_MARGIN + PLAYER_WIDTH as f32 / 2.0
                    && (t.center.1 - cy).abs()
                        < npc.height() / 2.0 + HOP_WAKE_MARGIN + PLAYER_HEIGHT as f32 / 2.0
            });
        if woken {
            npc.ai[0] = 1.0;
        }
        return;
    }

    if npc.velocity.1 == 0.0 {
        npc.ai[2] += 1.0;
        // The rest before the *first* hop of a pair is shorter, so the pair reads as one movement
        // rather than two separate ones.
        let rest = if npc.ai[1] == 0.0 {
            HOP_FIRST_REST
        } else {
            HOP_REST
        };
        if npc.ai[2] < rest {
            npc.velocity.0 *= HOP_DRAG;
            return;
        }
        npc.ai[2] = 0.0;
        look(npc);
        if npc.direction == 0 {
            npc.direction = -1;
        }
        npc.sprite_direction = npc.direction;
        npc.ai[1] += 1.0;
        let (across, up) = if npc.ai[1] == 2.0 {
            npc.ai[1] = 0.0;
            HOP_HIGH
        } else {
            HOP_LONG
        };
        npc.velocity = (f32::from(npc.direction) * across, up);
    } else if npc.direction == 1 && npc.velocity.0 < HOP_LEAN_CAP {
        npc.velocity.0 += HOP_LEAN;
    } else if npc.direction == -1 && npc.velocity.0 > -HOP_LEAN_CAP {
        npc.velocity.0 -= HOP_LEAN;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Room(HashMap<(i32, i32), Tile>);

    impl TileView for Room {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn floor(at: i32) -> Room {
        let mut tiles = HashMap::new();
        for x in -200..200 {
            for y in at..at + 3 {
                tiles.insert((x, y), Tile::block(1));
            }
        }
        Room(tiles)
    }

    fn world<'a>(tiles: &'a Room, target: Option<(f32, f32)>) -> World<'a, Room> {
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

    /// The Snowman Gangsta, a style-25 type.
    const SNOWMAN: u16 = 341;

    fn snowman(tile_x: i32, tile_y: i32) -> Npc {
        Npc::new(
            SNOWMAN,
            (
                tile_x as f32 * crate::game::npc::TILE,
                tile_y as f32 * crate::game::npc::TILE,
            ),
            1,
        )
        .expect("snowman gangsta")
    }

    /// Drop an NPC onto the floor, the way one that has been sitting there already is.
    ///
    /// A hopper counts its own falling as a disturbance, so a test that spawns one in mid-air is
    /// testing the wrong thing.
    fn settled(tiles: &Room, mut npc: Npc) -> Npc {
        for _ in 0..120 {
            crate::game::npc::step_physics(&mut npc, tiles);
            if npc.velocity.1 == 0.0 && npc.on_ground {
                break;
            }
        }
        npc.velocity = (0.0, 0.0);
        npc
    }

    /// Undisturbed, a hopper does nothing at all — that is what makes walking into one a surprise.
    #[test]
    fn a_hopper_left_alone_stays_put() {
        let tiles = floor(20);
        let w = world(&tiles, Some((10_000.0, 0.0)));
        let mut s = settled(&tiles, snowman(0, 17));
        let resting = s.position.0;
        for _ in 0..300 {
            hopper(&mut s, &w, false);
            crate::game::npc::step_physics(&mut s, &tiles);
        }
        assert_eq!(s.ai[0], 0.0, "nothing disturbed it");
        assert_eq!(s.position.0, resting, "so it should not have moved");
    }

    /// Getting close wakes it, and being hurt wakes it even from across the room.
    #[test]
    fn a_hopper_wakes_when_you_come_near_or_hit_it() {
        let tiles = floor(20);
        let mut near = snowman(0, 17);
        let (cx, cy) = near.center();
        hopper(&mut near, &world(&tiles, Some((cx + 60.0, cy))), false);
        assert_eq!(near.ai[0], 1.0, "a player right there should wake it");

        let mut hurt = snowman(0, 17);
        hurt.life -= 1;
        hopper(&mut hurt, &world(&tiles, Some((10_000.0, 0.0))), false);
        assert_eq!(hurt.ai[0], 1.0, "so should being hit");
    }

    /// The wake box is a real rectangle intersected against the player's own hitbox, not the wake
    /// box against a bare point — so it has to wake up to twenty-one pixels sooner than the box's
    /// own edge, where the player's own width and height still overlap it.
    #[test]
    fn a_hopper_wakes_at_the_edge_of_the_players_own_hitbox() {
        let tiles = floor(20);
        let mut s = snowman(0, 17);
        let (cx, cy) = s.center();
        // Fifteen pixels past the box's own edge: outside the old, point-only test, but still
        // within reach once the player's own eighty-four-pixel-tall hitbox is accounted for.
        let just_past = cy - (s.height() / 2.0 + HOP_WAKE_MARGIN + 15.0);
        hopper(&mut s, &world(&tiles, Some((cx, just_past))), false);
        assert_eq!(
            s.ai[0], 1.0,
            "the player's own hitbox should have reached it"
        );
    }

    /// The jumps alternate: one long and low, one short and high. A hopper that only ever did one
    /// of them would still travel, so the test is that both shapes occur.
    #[test]
    fn a_hopper_alternates_a_long_hop_and_a_high_one() {
        let tiles = floor(20);
        let mut s = snowman(0, 17);
        s.ai[0] = 1.0;
        let w = world(&tiles, Some((5000.0, 0.0)));

        let mut launches = Vec::new();
        let mut was_grounded = true;
        for _ in 0..600 {
            hopper(&mut s, &w, false);
            if was_grounded && s.velocity.1 < 0.0 {
                launches.push(s.velocity);
            }
            was_grounded = s.velocity.1 == 0.0;
            crate::game::npc::step_physics(&mut s, &tiles);
        }
        assert!(
            launches.len() >= 4,
            "expected several hops, got {launches:?}"
        );
        assert!(
            launches.iter().any(|v| v.1 == HOP_LONG.1),
            "no long hop in {launches:?}"
        );
        assert!(
            launches.iter().any(|v| v.1 == HOP_HIGH.1),
            "no high hop in {launches:?}"
        );
    }

    /// It travels toward the player rather than away.
    #[test]
    fn a_hopper_hops_toward_you() {
        let tiles = floor(20);
        let mut s = snowman(0, 17);
        s.ai[0] = 1.0;
        let start = s.position.0;
        let w = world(&tiles, Some((start + 2000.0, s.center().1)));
        for _ in 0..600 {
            hopper(&mut s, &w, false);
            crate::game::npc::step_physics(&mut s, &tiles);
        }
        assert!(
            s.position.0 > start + 50.0,
            "it should have closed the distance, got {}",
            s.position.0 - start
        );
    }
}
