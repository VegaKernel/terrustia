//! Style 126 — the statue mimic.
//!
//! It waits disguised as a statue, and the interesting part is what it does *while* it waits: every
//! ten ticks, if nobody can see the tile it is standing on, it picks a hidden solid spot closer to
//! you and simply appears there. So a mimic you never noticed has been closing on you the whole
//! time, one silent teleport at a time.
//!
//! Once triggered it is a leaper rather than a walker. Its jump is sized to the drop it has to make
//! up and its run to the gap it has to close, and it drops tile collision on the way up — which is
//! how one comes at you through a ceiling.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    MIMIC_HOP, MIMIC_HOP_DELAY, MIMIC_HOP_EXTRA_CAP, MIMIC_HOP_PER_DROP, MIMIC_RELOCATE_EVERY,
    MIMIC_RUN, MIMIC_RUN_EXTRA_CAP, MIMIC_RUN_PER_GAP, MIMIC_TRIGGER,
};
use terrustia_proto::tile_solid::solid;

use super::{PLAYER_HEIGHT, World, can_see, sight::solid_collision};
use crate::game::npc::{Npc, TILE, TileView};
use crate::game::npc_ai::Target;

/// Half a screen, which is what "can anyone see this" means.
const SCREEN_HALF: (f32, f32) = (960.0, 600.0);

/// How long a dormant mimic survives if left alone.
const DORMANT_LIFE: i32 = 60;

/// Whether a tile is within any player's view.
fn seen(target: Option<Target>, tile: (i32, i32)) -> bool {
    target.is_some_and(|t| {
        let at = ((tile.0 * 16) as f32, (tile.1 * 16) as f32);
        (at.0 - t.center.0).abs() < SCREEN_HALF.0 && (at.1 - t.center.1).abs() < SCREEN_HALF.1
    })
}

fn footing(tiles: &impl TileView, x: i32, y: i32) -> bool {
    let t = tiles.tile(x, y);
    t.is_active() && solid(t.block)
}

/// Look for somewhere hidden, solid and closer to the target to appear.
fn find_hiding_place<T: TileView>(
    npc: &Npc,
    world: &World<'_, T>,
    target: Target,
    rng: &mut SmallRng,
) -> Option<(f32, f32)> {
    let here = npc.center();
    // The band to search: between the mimic and the target, or around the target when it is
    // already within a screen of them.
    let span = |mine: f32, theirs: f32, half: f32| -> (i32, i32) {
        if (theirs - mine).abs() < half {
            (
                ((theirs - half) / TILE) as i32,
                ((theirs + half) / TILE) as i32,
            )
        } else if theirs < mine {
            (((theirs + half) / TILE) as i32, (mine / TILE) as i32)
        } else {
            ((mine / TILE) as i32, ((theirs - half) / TILE) as i32)
        }
    };
    let (x0, x1) = span(here.0, target.center.0, SCREEN_HALF.0);
    let (y0, y1) = span(here.1, target.center.1, SCREEN_HALF.1);
    if x0 >= x1 || y0 >= y1 {
        return None;
    }

    let gap =
        |p: (f32, f32)| ((p.0 - target.center.0).powi(2) + (p.1 - target.center.1).powi(2)).sqrt();
    for _ in 0..10 {
        let x = rng.random_range(x0..x1);
        let mut y = rng.random_range(y0..y1);
        // Fall to the first floor beneath the chosen point.
        let mut landed = false;
        for _ in 0..10 {
            if footing(world.tiles, x, y) && footing(world.tiles, x + 1, y) {
                landed = !seen(Some(target), (x + 1, y));
                break;
            }
            y += 1;
        }
        if !landed {
            continue;
        }
        let spot = (
            (x * 16 + 16) as f32 - npc.width() / 2.0,
            (y * 16) as f32 - npc.height(),
        );
        // Not into a wall, and only if it is an improvement.
        if solid_collision(world.tiles, spot, (npc.stats.width, npc.stats.height)) {
            continue;
        }
        if gap(spot) < gap(npc.position) {
            return Some(spot);
        }
    }
    None
}

/// Drive one statue mimic for a tick.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, rng: &mut SmallRng) {
    if npc.ai[0] == 0.0 {
        // Dormant. It expires quickly if left alone, which is what stops the world filling with
        // statues nobody ever walked past.
        npc.time_left = npc.time_left.min(DORMANT_LIFE);
        let Some(target) = world.target else {
            return;
        };
        npc.local_ai[0] += 1.0;

        let here = npc.center();
        let gap = ((here.0 - target.center.0).powi(2) + (here.1 - target.center.1).powi(2)).sqrt();
        if target.alive && gap < MIMIC_TRIGGER && can_see(world.tiles, npc, target) {
            npc.ai[0] = 1.0;
            npc.dirty = true;
            return;
        }

        if npc.local_ai[0] < MIMIC_RELOCATE_EVERY {
            return;
        }
        npc.local_ai[0] = 0.0;
        let my_tile = (
            (here.0 / TILE) as i32,
            ((npc.position.1 + npc.height()) / TILE) as i32,
        );
        if seen(Some(target), my_tile) {
            return;
        }
        if let Some(spot) = find_hiding_place(npc, world, target, rng) {
            npc.direction = if spot.0 < npc.position.0 { -1 } else { 1 };
            npc.position = spot;
            npc.dirty = true;
        }
        return;
    }

    // Awake. It stands, then leaps, and repeats.
    let Some(target) = world.target else {
        return;
    };
    let mut leap = false;
    let bottom = npc.position.1 + npc.height();
    let their_bottom = target.center.1 + PLAYER_HEIGHT as f32 / 2.0;
    let their_top = target.center.1 - PLAYER_HEIGHT as f32 / 2.0;

    if npc.velocity.1 > 0.0
        && npc.position.1 > their_bottom
        && solid_collision(
            world.tiles,
            npc.position,
            (npc.stats.width, npc.stats.height),
        )
    {
        // It has fallen below its target and buried itself: leap straight back out.
        leap = true;
    } else if npc.velocity.1 == 0.0 {
        npc.velocity.0 = 0.0;
        npc.ai[2] -= 1.0;
        if npc.ai[2] <= 0.0 {
            npc.ai[2] = MIMIC_HOP_DELAY;
            leap = true;
            npc.dirty = true;
        }
    }

    // Directly under them: it stops pushing sideways and lets itself rise.
    let under_them = npc.position.0 + npc.width() >= target.center.0 - 10.0
        && npc.position.0 <= target.center.0 + 10.0;
    if under_them && bottom < their_top {
        npc.velocity.0 *= 0.75;
        if npc.velocity.1 < 0.0 {
            npc.velocity.1 *= 0.75;
        }
    }

    if leap {
        npc.no_tile_collide = true;
        npc.direction = if target.center.0 < npc.center().0 {
            -1
        } else {
            1
        };
        let drop = ((bottom - their_bottom) / MIMIC_HOP_PER_DROP).clamp(0.0, MIMIC_HOP_EXTRA_CAP);
        npc.velocity.1 = MIMIC_HOP - drop;
        let gap =
            ((npc.center().0 - target.center.0).abs() / MIMIC_RUN_PER_GAP).min(MIMIC_RUN_EXTRA_CAP);
        npc.velocity.0 = (MIMIC_RUN + gap) * f32::from(npc.direction);
        npc.dirty = true;
    } else if npc.velocity.1 != 0.0 {
        // Airborne. It drops through the floor to reach someone below, and holds its ground
        // otherwise.
        if under_them && bottom < their_top {
            npc.velocity.1 = 16.0;
        } else {
            if npc.velocity.1 > 0.0 {
                npc.velocity.1 += crate::game::npc::GRAVITY;
            }
            if target.alive {
                if npc.direction > 0 && npc.center().0 > target.center.0 {
                    npc.velocity.0 *= 0.96;
                    npc.velocity.1 *= 0.96;
                }
                if npc.direction < 0 && npc.center().0 < target.center.0 {
                    npc.velocity.0 *= 0.96;
                    npc.velocity.1 *= 0.96;
                }
            }
        }
        npc.no_tile_collide = npc.velocity.1 < 0.0
            || (under_them && bottom < their_top)
            || solid_collision(
                world.tiles,
                npc.position,
                (npc.stats.width, npc.stats.height),
            );
    }
    npc.sprite_direction = npc.direction;
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Cavern(HashMap<(i32, i32), Tile>);

    impl TileView for Cavern {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn floored() -> Cavern {
        let mut c = Cavern::default();
        for x in 0..4000 {
            for y in 500..510 {
                c.0.insert((x, y), Tile::block(1));
            }
        }
        c
    }

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(77)
    }

    fn mimic(tile_x: i32) -> Npc {
        let mut n = Npc::new(690, (0.0, 0.0), 1).expect("statue mimic");
        n.position = (tile_x as f32 * TILE, 500.0 * TILE - n.height());
        n
    }

    fn world<'a>(tiles: &'a Cavern, target: Option<Target>) -> World<'a, Cavern> {
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
    fn a_statue_mimic_wakes_when_you_get_within_arms_reach() {
        let tiles = floored();
        let mut m = mimic(200);
        let (cx, cy) = m.center();
        update(
            &mut m,
            &world(&tiles, Some(player_at(cx + MIMIC_TRIGGER + 50.0, cy))),
            &mut rng(),
        );
        assert_eq!(m.ai[0], 0.0, "still a statue");

        update(
            &mut m,
            &world(&tiles, Some(player_at(cx + 40.0, cy))),
            &mut rng(),
        );
        assert_eq!(m.ai[0], 1.0, "should have woken");
    }

    #[test]
    fn a_dormant_mimic_does_not_hang_around() {
        let tiles = floored();
        let mut m = mimic(200);
        let (cx, cy) = m.center();
        update(
            &mut m,
            &world(&tiles, Some(player_at(cx + 5000.0, cy))),
            &mut rng(),
        );
        assert!(m.time_left <= DORMANT_LIFE);
    }

    #[test]
    fn a_dormant_mimic_creeps_closer_while_you_are_not_looking() {
        let tiles = floored();
        let mut m = mimic(200);
        let (cx, cy) = m.center();
        // Far enough that it cannot see the mimic's tile.
        let t = Some(player_at(cx + 4000.0, cy));
        let start = m.position.0;
        let mut r = rng();
        for _ in 0..200 {
            update(&mut m, &world(&tiles, t), &mut r);
        }
        assert!(
            m.position.0 > start,
            "should have closed the gap, from {start} to {}",
            m.position.0
        );
    }

    #[test]
    fn a_mimic_you_are_watching_stays_where_it_is() {
        let tiles = floored();
        let mut m = mimic(200);
        let (cx, cy) = m.center();
        // Close enough to see it, far enough not to trigger it.
        let t = Some(player_at(cx + 400.0, cy));
        let start = m.position.0;
        let mut r = rng();
        for _ in 0..200 {
            update(&mut m, &world(&tiles, t), &mut r);
        }
        assert_eq!(m.position.0, start, "it will not move while watched");
    }

    #[test]
    fn a_woken_mimic_leaps_after_a_pause() {
        let tiles = floored();
        let mut m = mimic(200);
        m.ai[0] = 1.0;
        m.ai[2] = MIMIC_HOP_DELAY;
        let (cx, cy) = m.center();
        let t = Some(player_at(cx + 300.0, cy));
        let mut r = rng();
        for _ in 0..(MIMIC_HOP_DELAY as i32) {
            m.velocity.1 = 0.0;
            update(&mut m, &world(&tiles, t), &mut r);
        }
        assert!(m.velocity.1 < 0.0, "should leap, got {}", m.velocity.1);
        assert!(m.velocity.0 > MIMIC_RUN, "and toward the player");
        assert_eq!(m.direction, 1);
    }

    #[test]
    fn a_mimic_leaps_higher_to_reach_someone_far_above_it() {
        let tiles = floored();
        let (cx, cy) = mimic(200).center();
        let heights: Vec<f32> = [0.0f32, 400.0]
            .iter()
            .map(|up| {
                let mut m = mimic(200);
                m.ai[0] = 1.0;
                m.ai[2] = 1.0;
                m.velocity.1 = 0.0;
                let t = Some(player_at(cx + 300.0, cy - up));
                update(&mut m, &world(&tiles, t), &mut rng());
                m.velocity.1
            })
            .collect();
        assert!(
            heights[1] < heights[0],
            "the higher target should get the bigger leap: {heights:?}"
        );
    }

    #[test]
    fn a_mimic_runs_faster_the_further_it_has_to_go() {
        let tiles = floored();
        let (cx, cy) = mimic(200).center();
        let speeds: Vec<f32> = [100.0f32, 2000.0]
            .iter()
            .map(|gap| {
                let mut m = mimic(200);
                m.ai[0] = 1.0;
                m.ai[2] = 1.0;
                m.velocity.1 = 0.0;
                let t = Some(player_at(cx + gap, cy));
                update(&mut m, &world(&tiles, t), &mut rng());
                m.velocity.0
            })
            .collect();
        assert!(speeds[1] > speeds[0], "got {speeds:?}");
        assert!(
            speeds[1] <= MIMIC_RUN + MIMIC_RUN_EXTRA_CAP,
            "but capped: {speeds:?}"
        );
    }

    #[test]
    fn a_leaping_mimic_passes_through_terrain_on_the_way_up() {
        let tiles = floored();
        let mut m = mimic(200);
        m.ai[0] = 1.0;
        m.ai[2] = 1.0;
        m.velocity.1 = 0.0;
        let (cx, cy) = m.center();
        update(
            &mut m,
            &world(&tiles, Some(player_at(cx + 300.0, cy - 200.0))),
            &mut rng(),
        );
        assert!(m.no_tile_collide, "it comes up through the floor");
    }
}
