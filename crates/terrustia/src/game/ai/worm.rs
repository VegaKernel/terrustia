//! Style 6 — the worms.
//!
//! Ported from `AI_006_Worms`. A worm is not one NPC but a chain of them, and the routine here has
//! two halves that share nothing but a file.
//!
//! A **segment** does not steer at all. Every tick it teleports to exactly one gap behind the
//! segment ahead of it and zeroes its velocity. That is why a worm never stretches, never kinks,
//! and why cutting one in half leaves two worms rather than a worm and some debris.
//!
//! A **head** has two modes and switches between them by asking a single question: is it inside
//! solid ground? In the rock it swims, turning toward its target at a fixed rate per tick with no
//! brakes at all — which is exactly why a fast worm carves such wide circles and keeps overshooting
//! you. Out in the air it stops steering and simply falls, which is what turns a burst out of the
//! ground into an arc rather than a hover.

use terrustia_proto::npc_params::{
    WORM_ATTENTION_RANGE, WORM_DESPAWN_TICKS, worm_air_gravity, worm_always_digs,
    worm_flees_surface_target, worm_is_head, worm_motion, worm_segment_gap, worm_sink_accel,
};
use terrustia_proto::tile::TileFlags;
use terrustia_proto::tile_sets::frame_important;
use terrustia_proto::tile_solid::{solid, solid_top};

use super::{World, face};
use crate::game::npc::{Npc, TileView};

/// Liquid deep enough to swim through rather than fall through.
const SWIMMABLE_LIQUID: u8 = 64;

/// Snap a coordinate to the sixteen-pixel grid a worm aims on.
///
/// Quantising to whole tiles is what gives a worm its slightly blind pursuit: it steers at the tile
/// you are standing in, not at you.
fn snap(v: f32) -> f32 {
    ((v / 16.0) as i32 * 16) as f32
}

/// Whether a tile is something a worm can swim through rather than fall past.
fn diggable(tiles: &impl TileView, x: i32, y: i32) -> bool {
    let tile = tiles.tile(x, y);
    let active = tile.is_active() && !tile.flags.has(TileFlags::ACTUATED);
    if active && (solid(tile.block) || (solid_top(tile.block) && !frame_important(tile.block))) {
        return true;
    }
    tile.liquid > SWIMMABLE_LIQUID
}

/// Whether a worm's body is currently inside ground or deep water.
pub fn in_ground(tiles: &impl TileView, position: (f32, f32), size: (i32, i32)) -> bool {
    let left = (position.0 / 16.0) as i32 - 1;
    let right = ((position.0 + size.0 as f32) / 16.0) as i32 + 2;
    let top = (position.1 / 16.0) as i32 - 1;
    let bottom = ((position.1 + size.1 as f32) / 16.0) as i32 + 2;
    for x in left..right {
        for y in top..bottom {
            if !diggable(tiles, x, y) {
                continue;
            }
            let (tx, ty) = ((x * 16) as f32, (y * 16) as f32);
            if position.0 + size.0 as f32 > tx
                && position.0 < tx + 16.0
                && position.1 + size.1 as f32 > ty
                && position.1 < ty + 16.0
            {
                return true;
            }
        }
    }
    false
}

/// Drag one segment along behind the one in front of it.
///
/// There is no easing and no spring: the segment is placed, not moved. Its velocity is zeroed
/// every tick so nothing else in the engine tries to move it as well.
pub fn follow(npc: &mut Npc, leader_center: (f32, f32)) {
    let (cx, cy) = npc.center();
    let (dx, dy) = (leader_center.0 - cx, leader_center.1 - cy);
    let length = (dx * dx + dy * dy).sqrt();
    npc.velocity = (0.0, 0.0);
    if length == 0.0 {
        return;
    }
    let gap = worm_segment_gap(npc.npc_type, npc.stats.width);
    let reach = (length - gap) / length;
    npc.position.0 += dx * reach;
    npc.position.1 += dy * reach;
    npc.direction = if dx > 0.0 { 1 } else { -1 };
    npc.sprite_direction = npc.direction;
    npc.dirty = true;
}

/// Fall, and lean into whichever way the target lies.
///
/// The horizontal terms look strange out of context — a worm in the air pushes *harder* the slower
/// it is going, and pushes back once it is falling fast. Together they turn what would be a lazy
/// parabola into the sharp flick a worm makes as it re-enters the ground.
fn arc_through_air(npc: &mut Npc, offset: (f32, f32), speed: f32, turn: f32) {
    let gravity = worm_air_gravity(npc.npc_type, npc.velocity.1 < 0.0);
    npc.velocity.1 += gravity;
    if npc.velocity.1 > speed {
        npc.velocity.1 = speed;
    }

    let drift = npc.velocity.0.abs() + npc.velocity.1.abs();
    if drift < speed * 0.4 {
        npc.velocity.0 += if npc.velocity.0 < 0.0 {
            -turn * 1.1
        } else {
            turn * 1.1
        };
    } else if npc.velocity.1 == speed {
        if npc.velocity.0 < offset.0 {
            npc.velocity.0 += turn;
        } else if npc.velocity.0 > offset.0 {
            npc.velocity.0 -= turn;
        }
    } else if npc.velocity.1 > 4.0 {
        npc.velocity.0 += if npc.velocity.0 < 0.0 {
            turn * 0.9
        } else {
            -turn * 0.9
        };
    }
}

/// Swim through rock toward the target.
///
/// The three branches are one decision: if the worm is already heading roughly the right way on
/// either axis it trims both, and otherwise it commits to whichever axis has further to cover and
/// keeps its speed up on the other. There is no braking term anywhere, which is the whole reason a
/// worm circles rather than homing.
///
/// `hard` is the Destroyer's second, sharper trim, which runs *before* the three branches and only
/// while both axes already agree; see [`worm_hard_turn`]. It is zero for every other worm.
fn swim_through_rock(npc: &mut Npc, offset: (f32, f32), speed: f32, turn: f32, hard: f32) {
    let reach = (offset.0 * offset.0 + offset.1 * offset.1).sqrt();
    if reach == 0.0 {
        return;
    }
    let (run, rise) = (offset.0.abs(), offset.1.abs());
    let k = speed / reach;
    let want = (offset.0 * k, offset.1 * k);
    let agrees = |v: f32, w: f32| (v > 0.0 && w > 0.0) || (v < 0.0 && w < 0.0);

    // MECH-4: `DESTROYER_TURN_HARD` was a constant nothing read. `NPC.cs:50633-50651` is a pass the
    // shared style-6 burrow does not have: a Destroyer whose velocity already agrees with the
    // wanted one on *both* axes trims at 0.15 here and then again at 0.1 below, closing at 0.25 a
    // tick. Without it the Destroyer took two and a half times as long to settle onto a line it was
    // already on, so it drifted wide of a player it had lined up.
    if hard > 0.0 && agrees(npc.velocity.0, want.0) && agrees(npc.velocity.1, want.1) {
        if npc.velocity.0 < want.0 {
            npc.velocity.0 += hard;
        } else if npc.velocity.0 > want.0 {
            npc.velocity.0 -= hard;
        }
        if npc.velocity.1 < want.1 {
            npc.velocity.1 += hard;
        } else if npc.velocity.1 > want.1 {
            npc.velocity.1 -= hard;
        }
    }

    // Read after that pass, as vanilla's own `if` at `NPC.cs:50652` reads the velocity it just
    // moved. For every worm but the Destroyer the pass above is skipped, so this is unchanged.
    let (vx, vy) = npc.velocity;
    let on_course = agrees(vx, want.0) || agrees(vy, want.1);

    if on_course {
        if npc.velocity.0 < want.0 {
            npc.velocity.0 += turn;
        } else if npc.velocity.0 > want.0 {
            npc.velocity.0 -= turn;
        }
        if npc.velocity.1 < want.1 {
            npc.velocity.1 += turn;
        } else if npc.velocity.1 > want.1 {
            npc.velocity.1 -= turn;
        }
        // Heading the wrong way along a nearly flat line: break the deadlock by climbing or
        // diving harder, so the worm loops round instead of grinding sideways.
        if want.1.abs() < speed * 0.2 && ((vx > 0.0 && want.0 < 0.0) || (vx < 0.0 && want.0 > 0.0))
        {
            npc.velocity.1 += if npc.velocity.1 > 0.0 {
                turn * 2.0
            } else {
                -turn * 2.0
            };
        }
        if want.0.abs() < speed * 0.2 && ((vy > 0.0 && want.1 < 0.0) || (vy < 0.0 && want.1 > 0.0))
        {
            npc.velocity.0 += if npc.velocity.0 > 0.0 {
                turn * 2.0
            } else {
                -turn * 2.0
            };
        }
    } else if run > rise {
        if npc.velocity.0 < want.0 {
            npc.velocity.0 += turn * 1.1;
        } else if npc.velocity.0 > want.0 {
            npc.velocity.0 -= turn * 1.1;
        }
        if npc.velocity.0.abs() + npc.velocity.1.abs() < speed * 0.5 {
            npc.velocity.1 += if npc.velocity.1 > 0.0 { turn } else { -turn };
        }
    } else {
        if npc.velocity.1 < want.1 {
            npc.velocity.1 += turn * 1.1;
        } else if npc.velocity.1 > want.1 {
            npc.velocity.1 -= turn * 1.1;
        }
        if npc.velocity.0.abs() + npc.velocity.1.abs() < speed * 0.5 {
            npc.velocity.0 += if npc.velocity.0 > 0.0 { turn } else { -turn };
        }
    }
}

/// Drive one worm head for a tick.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, expert: bool) {
    // `Player.ZoneSandstorm` is `ZoneDesert && SurfaceAtmospherics && Sandstorm.Happening`
    // (`SceneMetrics.cs:706`); the two flags the conditions carry are the same pair every other
    // sandstorm-aware routine here reads.
    let target_in_sandstorm = world.target.is_some_and(|t| t.alive)
        && world.conditions.sandstorm
        && world.conditions.desert;
    let mut motion = worm_motion(npc.npc_type, expert, target_in_sandstorm);
    // The Destroyer's second turn rate, and the get-fixed-boi fifth on top of both of them
    // (`NPC.cs:50509-50515`). Style 6's own get-good bumps already live in `worm_motion`'s table.
    let mut hard = terrustia_proto::npc_params::worm_hard_turn(npc.npc_type);
    if hard > 0.0 && world.conditions.get_good_world {
        motion.turn *= terrustia_proto::npc_params::DESTROYER_TURN_GET_GOOD;
        hard *= terrustia_proto::npc_params::DESTROYER_TURN_GET_GOOD;
    }

    if let Some(t) = world.target {
        face(npc, t);
    }

    // A target who has climbed out of reach, or died, is one to give up on.
    let abandoned = match world.target {
        None => true,
        Some(t) => {
            worm_flees_surface_target(npc.npc_type)
                && t.center.1 - super::PLAYER_HEIGHT as f32 / 2.0 < world.conditions.surface_y
        }
    };
    if abandoned {
        npc.time_left = npc.time_left.min(WORM_DESPAWN_TICKS);
        if worm_flees_surface_target(npc.npc_type) {
            npc.velocity.1 += worm_sink_accel(npc.npc_type);
        }
    }

    let size = (npc.stats.width, npc.stats.height);
    let mut digging = worm_always_digs(npc.npc_type) || in_ground(world.tiles, npc.position, size);
    if !digging && worm_is_head(npc.npc_type) {
        let anyone_near = world.target.is_some_and(|t| {
            (t.center.0 - npc.center().0).abs() < WORM_ATTENTION_RANGE
                && (t.center.1 - npc.center().1).abs() < WORM_ATTENTION_RANGE
        });
        // With nobody within a thousand pixels it burrows away rather than hanging in the air.
        digging = !anyone_near;
    }

    let Some(target) = world.target else {
        // Nothing to steer at; it keeps whatever course it was on and runs its timer down.
        npc.dirty = true;
        return;
    };

    // A worm aims at the tile its target is standing in, not at the target.
    let (cx, cy) = npc.center();
    let offset = (
        snap(target.center.0) - snap(cx),
        snap(target.center.1) - snap(cy),
    );

    if digging {
        swim_through_rock(npc, offset, motion.speed, motion.turn, hard);
    } else {
        arc_through_air(npc, offset, motion.speed, motion.turn);
    }
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ai::Conditions;
    use crate::game::npc::TILE;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Rock(HashMap<(i32, i32), Tile>);

    impl TileView for Rock {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn solid_block(from: (i32, i32), to: (i32, i32)) -> Rock {
        let mut r = Rock::default();
        for x in from.0..to.0 {
            for y in from.1..to.1 {
                r.0.insert((x, y), Tile::block(1));
            }
        }
        r
    }

    use terrustia_proto::npc_params::SOLAR_CRAWLTIPEDE_HEAD;

    fn worm(npc_type: u16, tile: (i32, i32)) -> Npc {
        Npc::new(npc_type, (tile.0 as f32 * TILE, tile.1 as f32 * TILE), 1).expect("a style 6 type")
    }

    fn player_at(x: f32, y: f32) -> Target {
        Target {
            slot: 0,
            center: (x, y),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    fn underground<'a>(tiles: &'a Rock, target: Option<Target>) -> World<'a, Rock> {
        World {
            conditions: Conditions {
                surface_y: 100.0 * TILE,
                ..Conditions::default()
            },
            ..crate::game::ai::calm(tiles, target)
        }
    }

    #[test]
    fn a_worm_in_rock_knows_it_is_in_rock() {
        let tiles = solid_block((0, 0), (800, 800));
        let w = worm(10, (400, 400));
        assert!(in_ground(
            &tiles,
            w.position,
            (w.stats.width, w.stats.height)
        ));
    }

    #[test]
    fn a_worm_in_open_air_knows_it_is_not() {
        let tiles = Rock::default();
        let w = worm(10, (400, 400));
        assert!(!in_ground(
            &tiles,
            w.position,
            (w.stats.width, w.stats.height)
        ));
    }

    #[test]
    fn deep_water_counts_as_something_to_swim_through() {
        let mut tiles = Rock::default();
        for x in 395..410 {
            for y in 395..410 {
                tiles
                    .0
                    .insert((x, y), Tile::AIR.with_liquid(Default::default(), 200));
            }
        }
        let w = worm(10, (400, 400));
        assert!(in_ground(
            &tiles,
            w.position,
            (w.stats.width, w.stats.height)
        ));
    }

    #[test]
    fn a_worm_in_the_air_falls_rather_than_steering() {
        let tiles = Rock::default();
        let mut w = worm(10, (400, 400));
        let (cx, cy) = w.center();
        // Someone close by, so it does not decide to burrow off instead.
        let t = Some(player_at(cx + 200.0, cy));
        update(&mut w, &underground(&tiles, t), false);
        assert!(
            w.velocity.1 > 0.0,
            "should be falling, got {}",
            w.velocity.1
        );
    }

    #[test]
    fn a_bone_serpent_rising_gets_a_gentler_gravity() {
        assert_eq!(worm_air_gravity(39, true), 0.08);
        assert_eq!(worm_air_gravity(39, false), 0.11);
        assert_eq!(worm_air_gravity(10, true), 0.11, "only the serpent leaps");
    }

    #[test]
    fn a_worm_in_rock_swims_toward_its_target() {
        let tiles = solid_block((0, 0), (800, 800));
        let mut w = worm(10, (400, 400));
        let (cx, cy) = w.center();
        let t = Some(player_at(cx + 2000.0, cy + 2000.0));
        for _ in 0..400 {
            update(&mut w, &underground(&tiles, t), false);
            w.position.0 += w.velocity.0;
            w.position.1 += w.velocity.1;
        }
        assert!(w.position.0 > 400.0 * TILE, "should have moved right");
        assert!(w.position.1 > 400.0 * TILE, "and down");
    }

    #[test]
    fn a_worm_overshoots_rather_than_stopping_on_you() {
        // No braking term anywhere in the routine, so a worm that reaches you keeps going.
        let tiles = solid_block((0, 0), (2000, 2000));
        let mut w = worm(7, (400, 400));
        let fixed = player_at(400.0 * TILE + 600.0, 400.0 * TILE);
        let mut passed = false;
        for _ in 0..600 {
            update(&mut w, &underground(&tiles, Some(fixed)), false);
            w.position.0 += w.velocity.0;
            w.position.1 += w.velocity.1;
            if w.center().0 > fixed.center.0 + 100.0 {
                passed = true;
            }
        }
        assert!(passed, "a devourer should sail straight past its target");
    }

    #[test]
    fn each_worm_swims_at_its_own_pace() {
        assert_eq!(
            worm_motion(10, false, false).speed,
            6.0,
            "a giant worm is slow"
        );
        assert_eq!(worm_motion(7, false, false).speed, 9.0, "a devourer is not");
        assert_eq!(worm_motion(13, false, false).turn, 0.07);
        assert_eq!(
            worm_motion(13, true, false).turn,
            0.15,
            "expert sharpens it"
        );
    }

    /// B3: two of three vanilla branches that were missing from the table.
    ///
    /// The Solar Crawltipede sets its own 10.0/0.30 after the whole chain (`NPC.cs:52325-52328`)
    /// and was falling to the 8.0/0.07 default: four-fifths of the speed and less than a quarter
    /// of the turn rate. The tomb crawler doubles down on a target caught in a sandstorm
    /// (`NPC.cs:52255-52267`) and was pinned at its calm-weather numbers.
    #[test]
    fn the_crawltipede_and_the_tomb_crawler_read_their_own_branches() {
        let default_worm = worm_motion(1000, false, false);
        let crawltipede = worm_motion(SOLAR_CRAWLTIPEDE_HEAD, false, false);
        assert_eq!(crawltipede.speed, 10.0);
        assert_eq!(crawltipede.turn, 0.3);
        assert!(
            crawltipede.turn > default_worm.turn * 4.0,
            "it was taking the default's 0.07"
        );

        let calm = worm_motion(510, false, false);
        assert_eq!((calm.speed, calm.turn), (10.0, 0.25));
        let storm = worm_motion(510, false, true);
        assert_eq!((storm.speed, storm.turn), (16.0, 0.35));
        assert_eq!(
            worm_motion(10, false, true),
            worm_motion(10, false, false),
            "no other worm reads the weather"
        );
    }

    #[test]
    fn a_segment_sits_exactly_one_gap_behind_its_leader() {
        let mut body = worm(11, (400, 400));
        let leader = (400.0 * TILE + 200.0, 400.0 * TILE);
        follow(&mut body, leader);
        let gap = (leader.0 - body.center().0).hypot(leader.1 - body.center().1);
        let want = worm_segment_gap(11, body.stats.width);
        assert!((gap - want).abs() < 0.01, "gap {gap}, wanted {want}");
        assert_eq!(body.velocity, (0.0, 0.0), "and it carries no velocity");
    }

    #[test]
    fn a_segment_on_top_of_its_leader_stays_put_rather_than_going_nowhere_fast() {
        let mut body = worm(11, (400, 400));
        let here = body.center();
        follow(&mut body, here);
        assert!(body.position.0.is_finite() && body.position.1.is_finite());
    }

    #[test]
    fn a_giant_worm_gives_up_on_a_target_who_climbs_to_the_surface() {
        let tiles = solid_block((0, 0), (800, 800));
        let mut w = worm(10, (400, 400));
        let above = Some(player_at(400.0 * TILE, 50.0 * TILE));
        update(&mut w, &underground(&tiles, above), false);
        assert!(
            w.time_left <= WORM_DESPAWN_TICKS,
            "should be leaving, got {}",
            w.time_left
        );
        assert!(w.velocity.1 > 0.0, "and diving away");
    }

    #[test]
    fn a_devourer_follows_you_into_the_daylight() {
        assert!(!worm_flees_surface_target(7));
        assert!(!worm_flees_surface_target(13));
        assert!(worm_flees_surface_target(10));
        assert!(worm_flees_surface_target(39));
        assert!(worm_flees_surface_target(117));
        assert!(worm_flees_surface_target(513));
    }

    /// The same worm in the same empty air behaves differently depending only on how far away the
    /// nearest player is: close, it falls; far, it swims off as though the air were rock.
    #[test]
    fn a_head_with_nobody_near_burrows_off_instead_of_falling() {
        let tiles = Rock::default();
        let mut near = worm(10, (400, 400));
        let mut far = worm(10, (400, 400));
        let (cx, cy) = near.center();

        update(
            &mut near,
            &underground(&tiles, Some(player_at(cx + 200.0, cy))),
            false,
        );
        update(
            &mut far,
            &underground(&tiles, Some(player_at(cx + 5000.0, cy))),
            false,
        );

        assert_eq!(
            near.velocity.1,
            worm_air_gravity(10, false),
            "with someone watching it should be falling"
        );
        assert!(
            far.velocity.1 < 0.0,
            "with nobody near it should swim off, got {}",
            far.velocity.1
        );
    }
}
