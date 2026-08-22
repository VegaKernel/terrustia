//! Style 8 — the casters.
//!
//! A caster never walks anywhere. It stands still — its horizontal velocity is bled away at 7% a
//! tick, and there is nothing anywhere in the routine that adds to it — and every so often it
//! blinks somewhere else near you. Between blinks it conjures.
//!
//! What it conjures is the surprise. Every pre-hardmode caster summons an **NPC**, not a
//! projectile: the fire imp's burning sphere, the goblin sorcerer's chaos ball, the dark caster's
//! water sphere and Tim's are all one-hit-point, gravity-free NPCs running style 9. Only the
//! dungeon's librarian throws anything.
//!
//! The cycle is 650 ticks. It casts on ticks 100, 200 and 300, and at 650 it picks a tile near its
//! target and appears there.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    CASTER_BLINK, CASTER_BLINK_SHORT, CASTER_CADENCE, CASTER_CYCLE, CASTER_TELEFRAG_GUARD,
    CASTER_TELEPORT_LIMIT, CASTER_WINDUP, conjuring, dungeon_wall,
};
use terrustia_proto::tile_solid::solid;

use super::{Shot, World, face, sight::within_firing_range};
use crate::game::npc::{Npc, TILE, TileView};
use crate::game::npc_ai::Target;

/// What a caster's tick produced.
#[derive(Debug, Default)]
pub struct Cast {
    /// An NPC it conjured, as (type, position).
    pub summon: Option<(u16, (f32, f32))>,
    /// A projectile it threw.
    pub shot: Option<Shot>,
}

/// Whether a caster will fire regardless of how far away its target is. Only the goblin sorcerer.
fn always_in_range(npc_type: u16) -> bool {
    npc_type == 29
}

/// Find somewhere near the target to appear, as `NPC.AI_AttemptToFindTeleportSpot` does.
///
/// It wants a solid tile with three clear tiles above it, not right where the caster already is,
/// and not close enough to a player to land on top of them. A dungeon caster additionally wants a
/// dungeon wall behind it, which is what keeps dark casters inside the dungeon.
fn find_landing<T: TileView>(
    npc: &Npc,
    world: &World<'_, T>,
    target: Target,
    range: i32,
    dungeon_bound: bool,
    rng: &mut SmallRng,
) -> Option<(i32, i32)> {
    let here = (
        (npc.center().0 / TILE) as i32,
        (npc.center().1 / TILE) as i32,
    );
    let goal = (
        (target.center.0 / TILE) as i32,
        (target.center.1 / TILE) as i32,
    );
    // Too far to reach at all.
    if ((here.0 - goal.0) * 16).abs() as f32 + ((here.1 - goal.1) * 16).abs() as f32
        > CASTER_TELEPORT_LIMIT
    {
        return None;
    }

    for _ in 0..100 {
        let x = rng.random_range(goal.0 - range..=goal.0 + range);
        let start = rng.random_range(goal.1 - range..=goal.1 + range);
        for y in start..goal.1 + range {
            // Not the tile it is already standing on.
            if (y - here.1).abs() <= 1 && (x - here.0).abs() <= 1 {
                continue;
            }
            let floor = world.tiles.tile(x, y);
            if !floor.is_active() || !solid(floor.block) {
                continue;
            }
            let above = world.tiles.tile(x, y - 1);
            if dungeon_bound && !dungeon_wall(above.wall) {
                continue;
            }
            if above.liquid > 0 && above.liquid_kind == terrustia_proto::tile::Liquid::Lava {
                continue;
            }
            // Three tiles of headroom either side.
            let clear = (x - 1..=x + 1)
                .all(|cx| (y - 4..=y - 1).all(|cy| !world.tiles.tile(cx, cy).is_active()));
            if !clear {
                continue;
            }
            // And nobody standing in it.
            let guard = (CASTER_TELEFRAG_GUARD * 16) as f32;
            let landing = ((x * 16) as f32, (y * 16) as f32);
            if (target.center.0 - landing.0).abs() < guard + 16.0
                && (target.center.1 - landing.1).abs() < guard + 16.0
            {
                continue;
            }
            return Some((x, y));
        }
    }
    None
}

/// Drive one caster for a tick.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, rng: &mut SmallRng) -> Cast {
    let mut cast = Cast::default();
    if let Some(t) = world.target {
        face(npc, t);
    }

    // It never propels itself; whatever knocked it about is bled off.
    npc.velocity.0 *= 0.93;
    if npc.velocity.0 > -0.1 && npc.velocity.0 < 0.1 {
        npc.velocity.0 = 0.0;
    }
    if npc.ai[0] == 0.0 {
        npc.ai[0] = 500.0;
    }

    // A pending blink lands at the top of the tick after it was chosen.
    if npc.ai[2] != 0.0 && npc.ai[3] != 0.0 {
        npc.position.0 = npc.ai[2] * 16.0 - (npc.stats.width / 2) as f32 + 8.0;
        npc.position.1 = npc.ai[3] * 16.0 - npc.height();
        npc.velocity = (0.0, 0.0);
        npc.ai[2] = 0.0;
        npc.ai[3] = 0.0;
        npc.dirty = true;
    }

    npc.ai[0] += 1.0;
    let Some(spell) = conjuring(npc.npc_type) else {
        npc.dirty = true;
        return cast;
    };

    let in_range = always_in_range(npc.npc_type)
        || world
            .target
            .is_some_and(|t| within_firing_range(npc.center(), t.center));
    if in_range && CASTER_CADENCE.contains(&npc.ai[0]) {
        npc.ai[1] = CASTER_WINDUP;
        npc.dirty = true;
    }

    if npc.ai[0] >= CASTER_CYCLE {
        npc.ai[0] = 1.0;
        if let Some(t) = world.target
            && let Some((x, y)) = find_landing(
                npc,
                world,
                t,
                spell.teleport_range,
                spell.dungeon_bound,
                rng,
            )
        {
            npc.ai[1] = if spell.teleport_range <= 5 {
                CASTER_BLINK_SHORT
            } else {
                CASTER_BLINK
            };
            npc.ai[2] = x as f32;
            npc.ai[3] = y as f32;
        }
        npc.dirty = true;
    }

    if npc.ai[1] > 0.0 {
        npc.ai[1] -= 1.0;
        if npc.ai[1] == spell.release_at {
            let x = npc.position.0
                + npc.width() / 2.0
                + if spell.offset_follows_facing {
                    spell.offset.0 * f32::from(npc.direction)
                } else {
                    spell.offset.0
                };
            let y = npc.position.1 + spell.offset.1;
            if let Some(summon) = spell.summons {
                cast.summon = Some((summon, (x, y)));
            }
            if let Some((projectile, damage)) = spell.throws {
                cast.shot = Some(Shot {
                    projectile,
                    damage,
                    position: (x, y),
                    velocity: (0.0, 0.0),
                    time_left: 300,
                });
            }
            npc.dirty = true;
        }
    }

    npc.sprite_direction = npc.direction;
    cast
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Cave(HashMap<(i32, i32), Tile>);

    impl TileView for Cave {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    /// A floor at tile y = 500 across a wide span, optionally walled as a dungeon.
    fn cavern(dungeon: bool) -> Cave {
        let mut c = Cave::default();
        for x in 0..2000 {
            for y in 500..510 {
                c.0.insert((x, y), Tile::block(1));
            }
            if dungeon {
                for y in 480..500 {
                    c.0.insert((x, y), Tile::AIR.with_wall(7));
                }
            }
        }
        c
    }

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(21)
    }

    fn caster(npc_type: u16, tile_x: i32) -> Npc {
        let mut n = Npc::new(npc_type, (0.0, 0.0), 1).expect("a style 8 type");
        n.position = (tile_x as f32 * TILE, 500.0 * TILE - n.height());
        n
    }

    fn player_at(x: f32, y: f32) -> Target {
        Target {
            slot: 0,
            center: (x, y),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    fn world<'a>(tiles: &'a Cave, target: Option<Target>) -> World<'a, Cave> {
        crate::game::ai::calm(tiles, target)
    }

    #[test]
    fn a_caster_never_walks() {
        let tiles = cavern(false);
        let mut c = caster(29, 100);
        c.velocity.0 = 5.0;
        let (cx, cy) = c.center();
        let t = Some(player_at(cx + 200.0, cy));
        for _ in 0..200 {
            update(&mut c, &world(&tiles, t), &mut rng());
        }
        assert_eq!(c.velocity.0, 0.0, "it should have come to a complete stop");
    }

    #[test]
    fn every_pre_hardmode_caster_summons_an_npc_rather_than_firing() {
        for (npc_type, summon) in [(24u16, 25u16), (29, 30), (32, 33), (45, 665)] {
            let spell = conjuring(npc_type).expect("a caster");
            assert_eq!(spell.summons, Some(summon), "type {npc_type}");
            assert!(spell.throws.is_none(), "type {npc_type} throws nothing");
        }
        let librarian = conjuring(693).expect("librarian");
        assert!(librarian.summons.is_none());
        assert_eq!(librarian.throws, Some((1092, 13)));
    }

    #[test]
    fn a_goblin_sorcerer_conjures_a_chaos_ball_on_its_cadence() {
        let tiles = cavern(false);
        let mut c = caster(29, 100);
        let (cx, cy) = c.center();
        let t = Some(player_at(cx + 200.0, cy));
        // A fresh caster starts its timer at 500, so it blinks first and only then settles into
        // the cadence. Wind it to the start of a cycle to watch a whole one.
        c.ai[0] = 1.0;
        let mut summons = Vec::new();
        for _ in 0..350 {
            if let Some(s) = update(&mut c, &world(&tiles, t), &mut rng()).summon {
                summons.push(s.0);
            }
        }
        assert_eq!(summons, vec![30, 30, 30], "three casts in one cycle");
    }

    #[test]
    fn a_fire_imp_conjures_to_the_side_and_sooner() {
        let tiles = cavern(false);
        let mut c = caster(24, 100);
        c.direction = 1;
        let (cx, cy) = c.center();
        let t = Some(player_at(cx + 200.0, cy));
        c.ai[0] = 1.0;
        let mut where_at = None;
        for _ in 0..350 {
            if let Some(s) = update(&mut c, &world(&tiles, t), &mut rng()).summon {
                where_at = Some(s.1);
                break;
            }
        }
        let at = where_at.expect("should have conjured");
        assert!(
            at.0 > c.position.0 + c.width() / 2.0,
            "should be out to its right, got {at:?}"
        );
        assert_eq!(conjuring(24).unwrap().release_at, 10.0, "and cast sooner");
    }

    #[test]
    fn a_caster_out_of_range_holds_its_fire_and_a_sorcerer_does_not() {
        let tiles = cavern(false);
        let (cx, cy) = (100.0 * TILE, 500.0 * TILE);
        let far = Some(player_at(cx + 40_000.0, cy));

        let mut imp = caster(24, 100);
        imp.ai[0] = 1.0;
        for _ in 0..350 {
            assert!(
                update(&mut imp, &world(&tiles, far), &mut rng())
                    .summon
                    .is_none()
            );
        }

        let mut sorcerer = caster(29, 100);
        sorcerer.ai[0] = 1.0;
        let mut cast = false;
        for _ in 0..350 {
            if update(&mut sorcerer, &world(&tiles, far), &mut rng())
                .summon
                .is_some()
            {
                cast = true;
                break;
            }
        }
        assert!(cast, "a goblin sorcerer fires whatever the range");
    }

    #[test]
    fn a_caster_blinks_at_the_end_of_its_cycle() {
        let tiles = cavern(false);
        let mut c = caster(29, 100);
        let start = c.position.0;
        let t = Some(player_at(140.0 * TILE, 499.0 * TILE));
        let mut r = rng();
        for _ in 0..(CASTER_CYCLE as i32 + 5) {
            update(&mut c, &world(&tiles, t), &mut r);
        }
        assert!(
            (c.position.0 - start).abs() > 100.0,
            "should have moved, from {start} to {}",
            c.position.0
        );
    }

    #[test]
    fn a_dark_caster_will_not_blink_out_of_the_dungeon() {
        // No dungeon walls anywhere, so there is nowhere it will accept.
        let plain = cavern(false);
        let mut c = caster(32, 100);
        let start = c.position.0;
        let t = Some(player_at(140.0 * TILE, 499.0 * TILE));
        let mut r = rng();
        for _ in 0..(CASTER_CYCLE as i32 + 5) {
            update(&mut c, &world(&plain, t), &mut r);
        }
        assert_eq!(c.position.0, start, "it should have stayed put");

        // With the walls in place it moves.
        let dungeon = cavern(true);
        let mut c = caster(32, 100);
        for _ in 0..(CASTER_CYCLE as i32 + 5) {
            update(&mut c, &world(&dungeon, t), &mut r);
        }
        assert!((c.position.0 - start).abs() > 100.0, "should have blinked");
    }

    #[test]
    fn a_caster_will_not_land_on_top_of_its_target() {
        let tiles = cavern(false);
        let mut c = caster(29, 100);
        let t = player_at(140.0 * TILE, 499.0 * TILE);
        let mut r = rng();
        for _ in 0..40 {
            if let Some((x, y)) = find_landing(&c, &world(&tiles, Some(t)), t, 20, false, &mut r) {
                let guard = (CASTER_TELEFRAG_GUARD * 16) as f32;
                let landing = ((x * 16) as f32, (y * 16) as f32);
                assert!(
                    (t.center.0 - landing.0).abs() >= guard
                        || (t.center.1 - landing.1).abs() >= guard,
                    "landed on the player at {landing:?}"
                );
            }
            c.ai[0] = 0.0;
        }
    }

    #[test]
    fn a_caster_a_world_away_does_not_try_to_blink() {
        let tiles = cavern(false);
        let c = caster(29, 100);
        let t = player_at(100.0 * TILE + CASTER_TELEPORT_LIMIT * 2.0, 499.0 * TILE);
        assert!(find_landing(&c, &world(&tiles, Some(t)), t, 20, false, &mut rng()).is_none());
    }
}
