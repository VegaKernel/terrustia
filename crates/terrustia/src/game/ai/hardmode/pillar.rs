//! The lunar pillars: style 94.
//!
//! A pillar barely moves. It bobs on a five-second sine, holds itself between ten and thirty tiles
//! above whatever is underneath, and stays sixty tiles clear of the world's edges. What makes it a
//! fight is the shield: while any of its hundred minions are unaccounted for it takes no damage at
//! all, so the pillar is not the target — its escort is.
//!
//! Two details are easy to miss. Leave one alone for a second and it starts healing itself two
//! hundred a tick, so a half-finished pillar you walked away from is a fresh one when you come
//! back. And its death is not instant: it spends three seconds sinking and fading before it counts
//! as beaten.

use terrustia_proto::npc_params::{
    TOWER_ABANDONED_RANGE, TOWER_ABANDONED_TICKS, TOWER_BOB, TOWER_BOB_TICKS, TOWER_COLLAPSE_DRIFT,
    TOWER_COLLAPSE_EASE, TOWER_COLLAPSE_FADE_AT, TOWER_COLLAPSE_TICKS, TOWER_COMFORTABLE,
    TOWER_LIFT, TOWER_MARGIN, TOWER_MARGIN_NUDGE, TOWER_REGEN, TOWER_TOO_HIGH, TOWER_TOO_LOW,
};

use super::drifters::Outcome;
use crate::game::ai::World;
use crate::game::npc::{Npc, TILE, TileView};

/// Style 94.
///
/// `shield` is how many of its minions are still unaccounted for; while that is above zero nothing
/// touches it.
pub fn pillar(npc: &mut Npc, world: &World<'_, impl TileView>, shield: i32) -> Outcome {
    let mut out = Outcome::default();
    npc.dirty = true;

    // Collapsing. It sinks, fades, and is gone three seconds later.
    if npc.ai[2] == 1.0 {
        let speed = npc.velocity.0.hypot(npc.velocity.1);
        npc.velocity = (0.0, speed);
        if npc.velocity.1 < TOWER_COLLAPSE_DRIFT {
            npc.velocity.1 += TOWER_COLLAPSE_EASE;
        }
        if npc.velocity.1 > TOWER_COLLAPSE_DRIFT {
            npc.velocity.1 -= TOWER_COLLAPSE_EASE;
        }
        npc.invulnerable = true;
        npc.ai[1] += 1.0;
        if npc.ai[1] > TOWER_COLLAPSE_FADE_AT {
            let along = (npc.ai[1] - TOWER_COLLAPSE_FADE_AT) / 60.0;
            npc.alpha = (along * 255.0) as i32;
        }
        if npc.ai[1] >= TOWER_COLLAPSE_TICKS {
            out.spent = true;
            out.died = true;
        }
        return out;
    }

    // The shield is the whole fight: while it holds, the pillar itself cannot be hurt.
    npc.invulnerable = shield > 0;

    // Left alone it heals, which is what stops a pillar being whittled down over several visits.
    let abandoned = world
        .target
        .is_none_or(|t| !t.alive || distance(npc, t.center) > TOWER_ABANDONED_RANGE);
    if abandoned {
        npc.local_ai[0] += 1.0;
        if npc.local_ai[0] >= TOWER_ABANDONED_TICKS {
            npc.local_ai[0] = 0.0;
            npc.life = (npc.life + TOWER_REGEN).min(npc.life_max);
        }
    } else {
        npc.local_ai[0] = 0.0;
    }

    // The bob.
    let phase = std::f32::consts::TAU * npc.ai[0] / TOWER_BOB_TICKS;
    npc.velocity = (0.0, phase.sin() * TOWER_BOB);

    // Height above whatever is beneath it.
    let bottom = (
        (npc.center().0 / TILE) as i32,
        ((npc.position.1 + npc.height()) / TILE) as i32,
    );
    match drop_below(world.tiles, bottom.0, bottom.1, TOWER_TOO_HIGH) {
        // Ground close underneath: rise, harder the closer it is.
        Some(depth) if depth <= TOWER_TOO_LOW => {
            let urgency = 1.0 - depth as f32 / TOWER_TOO_LOW as f32;
            npc.position.1 -= TOWER_LIFT * urgency;
        }
        // Nothing within thirty tiles: sink back down.
        None => npc.position.1 += TOWER_LIFT,
        // Between twenty and thirty tiles: sink, gently, in proportion.
        Some(depth) if depth > TOWER_COMFORTABLE => {
            let along = ((depth - TOWER_COMFORTABLE) as f32
                / (TOWER_TOO_HIGH - TOWER_COMFORTABLE) as f32)
                .clamp(0.0, 1.0);
            npc.position.1 += TOWER_LIFT * along;
        }
        // Comfortable.
        Some(_) => {}
    }

    // It will not drift out of the world.
    let (world_w, world_h) = (
        world.world_width() as f32 * TILE,
        world.world_height() as f32 * TILE,
    );
    let edge = TOWER_MARGIN as f32 * TILE;
    let (cx, cy) = npc.center();
    let clamped = (
        cx.clamp(
            edge + TOWER_MARGIN_NUDGE,
            world_w - edge - TOWER_MARGIN_NUDGE,
        ),
        cy.clamp(
            edge + TOWER_MARGIN_NUDGE,
            world_h - edge - TOWER_MARGIN_NUDGE,
        ),
    );
    if clamped != (cx, cy) {
        npc.position = (
            clamped.0 - npc.width() / 2.0,
            clamped.1 - npc.height() / 2.0,
        );
    }
    // And it holds above the surface line, its base 100px clear of it, so it never sinks into the
    // ground. PIL-2: vanilla skips this clamp outright in a For-the-Worthy world (`!Main.getGoodWorld`,
    // and `!Main.remixWorld`, which this server does not model, `NPC.cs:39574`), where the tower is
    // let sit wherever it lands. Applying it everywhere pinned the get-good towers to the surface too.
    if !world.conditions.get_good_world {
        let ceiling = world.conditions.surface_y - npc.height() - 100.0;
        if npc.position.1 + npc.height() > world.conditions.surface_y - 100.0 {
            npc.position.1 = ceiling;
        }
    }

    npc.ai[0] += 1.0;
    if npc.ai[0] >= TOWER_BOB_TICKS {
        npc.ai[0] = 0.0;
    }
    out
}

/// How many tiles of clear air are below `(x, y)`, or `None` if nothing solid is within `limit`.
fn drop_below(tiles: &impl TileView, x: i32, y: i32, limit: i32) -> Option<i32> {
    (0..limit).find(|&depth| {
        let tile = tiles.tile(x, y + depth);
        tile.is_active() && terrustia_proto::tile_solid::solid(tile.block)
    })
}

fn distance(npc: &Npc, to: (f32, f32)) -> f32 {
    let (cx, cy) = npc.center();
    (to.0 - cx).hypot(to.1 - cy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ai::Conditions;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Ground(HashMap<(i32, i32), Tile>);

    impl TileView for Ground {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn ground(at: i32) -> Ground {
        let mut tiles = HashMap::new();
        for x in -500..500 {
            for y in at..at + 4 {
                tiles.insert((x, y), Tile::block(1));
            }
        }
        Ground(tiles)
    }

    fn world<'a>(tiles: &'a Ground, target: Option<(f32, f32)>) -> World<'a, Ground> {
        let mut w = crate::game::ai::calm(
            tiles,
            target.map(|center| Target {
                slot: 0,
                center,
                velocity: (0.0, 0.0),
                alive: true,
            }),
        );
        // The surface has to be *below* these tests' pillars, because the rule is that a pillar
        // stays above the surface line; a surface at zero would pin every one of them to the top
        // of the world.
        w.conditions = Conditions {
            surface_y: 6000.0,
            ..Conditions::default()
        };
        w
    }

    const SOLAR_PILLAR: u16 = 517;

    fn tower(tile_x: i32, tile_y: i32) -> Npc {
        Npc::new(
            SOLAR_PILLAR,
            (tile_x as f32 * TILE, tile_y as f32 * TILE),
            1,
        )
        .expect("solar pillar")
    }

    /// While the shield holds, nothing gets through — which is why you kill its escort first.
    #[test]
    fn a_shielded_pillar_cannot_be_hurt() {
        let tiles = ground(200);
        let mut p = tower(100, 150);
        let w = world(&tiles, Some((1600.0, 2400.0)));

        pillar(&mut p, &w, 100);
        assert!(p.invulnerable, "the shield should be up");
        let before = p.life;
        assert!(!p.take_damage(500, 0.0, 1), "and a hit does nothing");
        assert_eq!(p.life, before);

        pillar(&mut p, &w, 0);
        assert!(!p.invulnerable, "shield gone, pillar open");
        assert!(p.take_damage(p.life_max, 0.0, 1) || p.life < before);
    }

    /// It holds a height above the ground rather than resting on it or floating away.
    #[test]
    fn a_pillar_holds_its_height() {
        let floor_px = 200.0 * TILE;
        let tiles = ground(200);
        let w = world(&tiles, Some((1600.0, 100_000.0)));

        // A pillar is two hundred and seventy pixels tall, so these are placed by where their
        // *bottom* ends up rather than by tile.
        let settle = |gap_tiles: f32| {
            let mut p = Npc::new(SOLAR_PILLAR, (100.0 * TILE, 0.0), 1).unwrap();
            p.position.1 = floor_px - gap_tiles * TILE - p.height();
            for _ in 0..900 {
                pillar(&mut p, &w, 100);
            }
            (floor_px - (p.position.1 + p.height())) / TILE
        };

        // Too close to the floor: it climbs.
        let from_low = settle(2.0);
        assert!(
            from_low > 2.0,
            "it should have climbed away from the floor, ended {from_low} tiles up"
        );
        // Far too high: it comes back down.
        let from_high = settle(90.0);
        assert!(
            from_high < 90.0,
            "it should have sunk toward the ground, ended {from_high} tiles up"
        );
    }

    /// Walk away and it heals; stay and it does not.
    #[test]
    fn an_abandoned_pillar_heals_itself() {
        let tiles = ground(200);
        let mut p = tower(100, 150);
        p.life = p.life_max / 2;
        let wounded = p.life;

        let far = world(&tiles, Some((100_000.0, 0.0)));
        for _ in 0..300 {
            pillar(&mut p, &far, 100);
        }
        assert!(p.life > wounded, "left alone, it should heal");

        let healed = p.life;
        let (cx, cy) = p.center();
        let near = world(&tiles, Some((cx + 100.0, cy)));
        for _ in 0..300 {
            pillar(&mut p, &near, 100);
        }
        assert_eq!(p.life, healed, "stood next to, it should not");
    }

    /// Its death takes three seconds, and it fades out over the last one.
    #[test]
    fn a_pillar_collapses_rather_than_vanishing() {
        let tiles = ground(200);
        let mut p = tower(100, 150);
        let w = world(&tiles, Some((1600.0, 2400.0)));
        p.ai[2] = 1.0;

        let mut ticks = 0;
        let done = loop {
            let out = pillar(&mut p, &w, 0);
            ticks += 1;
            if out.spent || ticks > 400 {
                break out.spent;
            }
        };
        assert!(done, "it should finish collapsing");
        assert_eq!(ticks, TOWER_COLLAPSE_TICKS as i32);
        assert!(p.alpha > 0, "and it should have faded on the way out");
    }

    /// PIL-2: a For-the-Worthy tower is left where it lands, not lifted above the surface line
    /// (`!Main.getGoodWorld`, `NPC.cs:39574`). A normal-world tower below the surface is pulled up to
    /// hover 100px clear of it; a get-good one stays put below.
    #[test]
    fn a_for_the_worthy_tower_skips_the_surface_clamp() {
        let tiles = Ground(HashMap::new()); // no ground under it, so only the surface rule can act
        let settle = |get_good: bool| {
            // Its base sits well below the surface line at y = 6000.
            let mut p = tower(100, 400);
            let mut w = world(&tiles, Some((1600.0, 6400.0)));
            w.conditions.get_good_world = get_good;
            pillar(&mut p, &w, 100);
            p.position.1
        };
        let normal = settle(false);
        let good = settle(true);
        assert!(
            normal < 6000.0,
            "a normal-world tower is lifted above the surface, got {normal}"
        );
        assert!(
            good > 6000.0,
            "a get-good tower is left below the surface, got {good}"
        );
    }

    /// It will not drift out of the world.
    #[test]
    fn a_pillar_stays_inside_the_world() {
        let tiles = ground(200);
        let w = world(&tiles, Some((1600.0, 2400.0)));
        // Right against the left edge.
        let mut p = tower(2, 150);
        pillar(&mut p, &w, 100);
        assert!(
            p.center().0 > TOWER_MARGIN as f32 * TILE,
            "it should have been pushed clear of the edge, at {}",
            p.center().0
        );
    }
}
