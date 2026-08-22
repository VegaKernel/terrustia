//! Style 15 — King Slime.
//!
//! Three things drive the fight, and all three are tied to its health.
//!
//! It **shrinks**. Scale is `life/lifeMax * 0.5 + 0.75`, and its hitbox is recomputed from that
//! every tick, anchored at the bottom centre so it settles rather than sinking. A King Slime at
//! one hit point is barely half the size of a fresh one.
//!
//! It **speeds up**. The hop timer gains two a tick at full health and up to eleven at a tenth,
//! crossing five thresholds on the way, so the fight accelerates as it goes.
//!
//! It **sheds**. Every five per cent of its maximum health lost, one to three blue slimes drop out
//! of it, which is what turns the arena into a crowd.
//!
//! And it teleports. Five seconds of not being able to see you — or of being more than ten tiles
//! off your level — and it fades out, moves, and fades back in. Hold it at range long enough and it
//! stops being fussy about where it lands and simply appears on top of you.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    KING_SLIME_ANTI_CHEESE, KING_SLIME_DRIFT, KING_SLIME_DRIFT_PUSH, KING_SLIME_FADE_IN,
    KING_SLIME_FADE_OUT, KING_SLIME_GIVE_UP, KING_SLIME_HOPS, KING_SLIME_LEVEL,
    KING_SLIME_PATIENCE, KING_SLIME_RAGE, KING_SLIME_SCALE_FLOOR, KING_SLIME_SCALE_SPAN,
    KING_SLIME_SHED_STEP, KING_SLIME_SIZE, KING_SLIME_SPAWN, KING_SLIME_WIND,
};
use terrustia_proto::tile_solid::solid;

use crate::game::ai::{World, can_see, face};
use crate::game::npc::{Npc, TILE, TileView};
use crate::game::npc_ai::Target;

/// The two halves of a teleport, as `ai[1]` records them.
const FADING_OUT: f32 = 5.0;
const FADING_IN: f32 = 6.0;

/// One slime dropping out of King Slime, with the throw it was given.
pub type Shed = (u16, (f32, f32), (f32, f32));

/// What a tick of the fight produced.
#[derive(Debug, Default)]
pub struct Court {
    /// Slimes shed this tick, as (type, position, velocity).
    pub shed: Vec<Shed>,
}

/// Rebuild the hitbox from the current scale, keeping the bottom centre where it was.
fn resize(npc: &mut Npc, scale: f32) {
    let (w, h) = (
        (KING_SLIME_SIZE.0 * scale) as i32,
        (KING_SLIME_SIZE.1 * scale) as i32,
    );
    if w == npc.stats.width && h == npc.stats.height {
        return;
    }
    npc.position.0 += npc.width() / 2.0;
    npc.position.1 += npc.height();
    npc.scale = scale;
    npc.stats.width = w;
    npc.stats.height = h;
    npc.position.0 -= w as f32 / 2.0;
    npc.position.1 -= h as f32;
    npc.dirty = true;
}

/// Look for somewhere to land: a floor tile near the target with headroom over it.
///
/// Two passes, the second more forgiving than the first. Failing both — or having been kept at
/// arm's length for six seconds — it simply appears where the target is standing.
fn find_landing<T: TileView>(
    world: &World<'_, T>,
    target: Target,
    anti_cheese: bool,
    rng: &mut SmallRng,
) -> (f32, f32) {
    let on_top_of_them = (
        target.center.0,
        target.center.1 + super::super::PLAYER_HEIGHT as f32 / 2.0,
    );
    if anti_cheese {
        return on_top_of_them;
    }
    let goal = (
        (target.center.0 / TILE) as i32,
        (target.center.1 / TILE) as i32,
    );
    for (span, height) in [(10, 7), (6, 2)] {
        let mut found = Vec::new();
        for x in goal.0 - span..=goal.0 + span {
            for y in goal.1 - height..=goal.1 + height {
                let floor = world.tiles.tile(x, y);
                if !floor.is_active() || !solid(floor.block) {
                    continue;
                }
                // Six tiles of headroom, which is what a full-size King Slime needs.
                if (1..=6).any(|up| world.tiles.tile(x, y - up).is_active()) {
                    continue;
                }
                found.push((x, y));
            }
        }
        if !found.is_empty() {
            let (x, y) = found[rng.random_range(0..found.len())];
            return ((x * 16 + 8) as f32, (y * 16) as f32);
        }
    }
    on_top_of_them
}

/// Drive King Slime for a tick.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>, rng: &mut SmallRng) -> Court {
    let mut court = Court::default();
    if npc.life <= 0 {
        return court;
    }
    // `ai[3]` remembers the health at which it last shed; `local_ai[3]` marks the first tick.
    if npc.ai[3] == 0.0 {
        npc.ai[3] = npc.life_max as f32;
    }
    let first_tick = npc.local_ai[3] == 0.0;
    if first_tick {
        npc.local_ai[3] = 1.0;
        npc.ai[0] = -100.0;
        if let Some(t) = world.target {
            face(npc, t);
        }
    }

    // Nobody left to fight, or they have run right out of the arena.
    let abandoned = match world.target {
        None => true,
        Some(t) => {
            let (cx, cy) = npc.center();
            !t.alive
                || ((t.center.0 - cx).powi(2) + (t.center.1 - cy).powi(2)).sqrt()
                    > KING_SLIME_GIVE_UP
        }
    };
    if abandoned {
        npc.time_left = npc.time_left.min(10);
        if let Some(t) = world.target {
            npc.direction = if t.center.0 < npc.center().0 { 1 } else { -1 };
        }
    }

    // Patience. Losing sight of the target, or being well off its level, fills the timer.
    let in_reach = world.target.is_some_and(|t| {
        can_see(world.tiles, npc, t)
            && (npc.position.1 - (t.center.1 + super::super::PLAYER_HEIGHT as f32 / 2.0)).abs()
                <= KING_SLIME_LEVEL
    });
    if in_reach {
        npc.local_ai[0] = (npc.local_ai[0] - 1.0).max(0.0);
    } else {
        npc.ai[2] += 1.0;
        npc.local_ai[0] += 1.0;
    }

    // Out of patience and standing still: begin a teleport.
    if !abandoned
        && npc.ai[2] >= KING_SLIME_PATIENCE
        && npc.ai[1] < FADING_OUT
        && npc.velocity.1 == 0.0
        && let Some(t) = world.target
    {
        npc.ai[2] = 0.0;
        npc.ai[0] = 0.0;
        npc.ai[1] = FADING_OUT;
        let cornered = npc.local_ai[0] >= KING_SLIME_ANTI_CHEESE;
        if cornered {
            npc.local_ai[0] = KING_SLIME_ANTI_CHEESE;
        }
        let landing = find_landing(world, t, cornered, rng);
        npc.local_ai[1] = landing.0;
        npc.local_ai[2] = landing.1;
        npc.dirty = true;
    }

    // The teleport itself: a second of fading out, the move, then half a second fading back in.
    let mut fade = 1.0;
    let mut teleporting = false;
    let mut gone = false;
    if npc.ai[1] == FADING_OUT {
        teleporting = true;
        npc.ai[0] += 1.0;
        fade =
            0.5 + ((KING_SLIME_FADE_OUT - npc.ai[0]) / KING_SLIME_FADE_OUT).clamp(0.0, 1.0) * 0.5;
        if npc.ai[0] >= KING_SLIME_FADE_OUT {
            gone = true;
            npc.position.0 = npc.local_ai[1] - npc.width() / 2.0;
            npc.position.1 = npc.local_ai[2] - npc.height();
            npc.ai[1] = FADING_IN;
            npc.ai[0] = 0.0;
            npc.dirty = true;
        }
    } else if npc.ai[1] == FADING_IN {
        teleporting = true;
        npc.ai[0] += 1.0;
        fade = 0.5 + (npc.ai[0] / KING_SLIME_FADE_IN).clamp(0.0, 1.0) * 0.5;
        if npc.ai[0] >= KING_SLIME_FADE_IN {
            npc.ai[1] = 0.0;
            npc.ai[0] = 0.0;
            if let Some(t) = world.target {
                face(npc, t);
            }
            npc.dirty = true;
        }
    }
    // Mid-teleport it cannot be hurt, which is the window the fight gives you to reposition.
    npc.stats.dont_take_damage = gone;

    if npc.velocity.1 == 0.0 {
        npc.velocity.0 *= 0.8;
        if npc.velocity.0 > -0.1 && npc.velocity.0 < 0.1 {
            npc.velocity.0 = 0.0;
        }
        if !teleporting {
            // The hop timer, which fills faster the more hurt it is.
            npc.ai[0] += KING_SLIME_WIND;
            let health = npc.life as f32 / npc.life_max as f32;
            for (threshold, extra) in KING_SLIME_RAGE {
                if health < threshold {
                    npc.ai[0] += extra;
                }
            }
            if npc.ai[0] >= 0.0 {
                if let Some(t) = world.target {
                    face(npc, t);
                }
                let step = (npc.ai[1] as usize).min(KING_SLIME_HOPS.len() - 1);
                let (rise, push, recover) = KING_SLIME_HOPS[step];
                npc.velocity.1 = rise;
                npc.velocity.0 += push * f32::from(npc.direction);
                npc.ai[0] = recover;
                // Three hops, then the leap, then back to the start.
                npc.ai[1] = if step + 1 >= KING_SLIME_HOPS.len() {
                    0.0
                } else {
                    (step + 1) as f32
                };
                npc.dirty = true;
            }
        }
    } else if world.target.is_some() {
        // Airborne, steering toward whoever it is chasing.
        let top = KING_SLIME_DRIFT;
        if (npc.direction == 1 && npc.velocity.0 < top)
            || (npc.direction == -1 && npc.velocity.0 > -top)
        {
            if (npc.direction == -1 && npc.velocity.0 < 0.1)
                || (npc.direction == 1 && npc.velocity.0 > -0.1)
            {
                npc.velocity.0 += KING_SLIME_DRIFT_PUSH * f32::from(npc.direction);
            } else {
                npc.velocity.0 *= 0.93;
            }
        }
    }

    // Size follows health, and the teleport fade squeezes it further.
    let scale = (npc.life as f32 / npc.life_max as f32 * KING_SLIME_SCALE_SPAN
        + KING_SLIME_SCALE_FLOOR)
        * fade;
    if scale != npc.scale || first_tick {
        resize(npc, scale);
    }

    // Shedding: one to three slimes for every twentieth of its health.
    let step = npc.life_max as f32 * KING_SLIME_SHED_STEP;
    if (npc.life as f32 + step) < npc.ai[3] {
        npc.ai[3] = npc.life as f32;
        for _ in 0..rng.random_range(1..4) {
            let at = (
                npc.position.0 + rng.random_range(0..(npc.stats.width - 32).max(1)) as f32,
                npc.position.1 + rng.random_range(0..(npc.stats.height - 32).max(1)) as f32,
            );
            court.shed.push((
                KING_SLIME_SPAWN,
                at,
                (
                    rng.random_range(-15..16) as f32 * 0.1,
                    rng.random_range(-30..1) as f32 * 0.1,
                ),
            ));
        }
        npc.dirty = true;
    }

    npc.sprite_direction = npc.direction;
    npc.dirty = true;
    court
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Arena(HashMap<(i32, i32), Tile>);

    impl TileView for Arena {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn arena() -> Arena {
        let mut a = Arena::default();
        for x in 0..4000 {
            for y in 300..320 {
                a.0.insert((x, y), Tile::block(1));
            }
        }
        a
    }

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(15)
    }

    fn king() -> Npc {
        let mut n = Npc::new(50, (0.0, 0.0), 1).expect("king slime");
        n.position = (200.0 * TILE, 300.0 * TILE - n.height());
        n
    }

    fn world<'a>(tiles: &'a Arena, target: Option<Target>) -> World<'a, Arena> {
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

    /// The whole shape of the fight in one test: it shrinks as it dies.
    #[test]
    fn king_slime_shrinks_as_it_is_worn_down() {
        let tiles = arena();
        let mut k = king();
        let (cx, cy) = k.center();
        let t = Some(player_at(cx + 200.0, cy));
        update(&mut k, &world(&tiles, t), &mut rng());
        let full = (k.stats.width, k.stats.height);

        k.life = k.life_max / 10;
        update(&mut k, &world(&tiles, t), &mut rng());
        assert!(
            k.stats.width < full.0 && k.stats.height < full.1,
            "should have shrunk from {full:?} to {:?}",
            (k.stats.width, k.stats.height)
        );
        assert!(k.scale < 1.0);
    }

    #[test]
    fn shrinking_keeps_it_standing_on_the_floor() {
        let tiles = arena();
        let mut k = king();
        let (cx, cy) = k.center();
        let t = Some(player_at(cx + 200.0, cy));
        update(&mut k, &world(&tiles, t), &mut rng());
        let floor = k.position.1 + k.height();
        k.life = k.life_max / 4;
        update(&mut k, &world(&tiles, t), &mut rng());
        assert!(
            (k.position.1 + k.height() - floor).abs() < 1.0,
            "it should settle, not sink or jump"
        );
    }

    #[test]
    fn it_hops_three_times_and_then_leaps() {
        let tiles = arena();
        let mut k = king();
        let (cx, cy) = k.center();
        let t = Some(player_at(cx + 200.0, cy));
        let mut jumps = Vec::new();
        for _ in 0..2000 {
            k.velocity.1 = 0.0;
            update(&mut k, &world(&tiles, t), &mut rng());
            if k.velocity.1 < 0.0 {
                jumps.push(k.velocity.1);
            }
            if jumps.len() == 4 {
                break;
            }
        }
        assert_eq!(jumps.len(), 4);
        let leap = *jumps.last().unwrap();
        assert!(
            jumps[..3].iter().all(|&j| j > leap),
            "the fourth should be the big one: {jumps:?}"
        );
    }

    #[test]
    fn a_wounded_king_slime_hops_more_often() {
        let tiles = arena();
        let count_hops = |life: i32| {
            let mut k = king();
            k.life = life;
            let (cx, cy) = k.center();
            let t = Some(player_at(cx + 200.0, cy));
            let mut hops = 0;
            for _ in 0..1200 {
                k.velocity.1 = 0.0;
                update(&mut k, &world(&tiles, t), &mut rng());
                if k.velocity.1 < 0.0 {
                    hops += 1;
                }
            }
            hops
        };
        let healthy = count_hops(2000);
        let dying = count_hops(2000 / 20);
        assert!(
            dying > healthy,
            "a dying king slime should be quicker: {dying} against {healthy}"
        );
    }

    #[test]
    fn it_sheds_slimes_as_it_loses_health() {
        let tiles = arena();
        let mut k = king();
        let (cx, cy) = k.center();
        let t = Some(player_at(cx + 200.0, cy));
        update(&mut k, &world(&tiles, t), &mut rng());
        // Take off a tenth: two five-per-cent steps.
        k.life -= k.life_max / 10;
        let court = update(&mut k, &world(&tiles, t), &mut rng());
        assert!(!court.shed.is_empty(), "should have shed something");
        assert!(court.shed.len() <= 3);
        assert!(court.shed.iter().all(|(t, _, _)| *t == KING_SLIME_SPAWN));
        assert!(
            court.shed.iter().any(|(_, _, v)| v.1 <= 0.0),
            "and thrown them upward"
        );
    }

    #[test]
    fn shedding_does_not_repeat_without_further_damage() {
        let tiles = arena();
        let mut k = king();
        let (cx, cy) = k.center();
        let t = Some(player_at(cx + 200.0, cy));
        update(&mut k, &world(&tiles, t), &mut rng());
        k.life -= k.life_max / 10;
        update(&mut k, &world(&tiles, t), &mut rng());
        for _ in 0..100 {
            let court = update(&mut k, &world(&tiles, t), &mut rng());
            assert!(court.shed.is_empty(), "no more damage, no more slimes");
        }
    }

    #[test]
    fn losing_sight_of_you_makes_it_teleport() {
        let mut tiles = arena();
        // A wall between them.
        for y in 200..300 {
            tiles.0.insert((210, y), Tile::block(1));
        }
        let mut k = king();
        let (cx, cy) = k.center();
        let t = Some(player_at(cx + 400.0, cy));
        let start = k.position.0;
        let mut r = rng();
        for _ in 0..(KING_SLIME_PATIENCE as i32 + KING_SLIME_FADE_OUT as i32 + 5) {
            k.velocity.1 = 0.0;
            update(&mut k, &world(&tiles, t), &mut r);
        }
        assert!(
            (k.position.0 - start).abs() > 100.0,
            "should have moved, from {start} to {}",
            k.position.0
        );
    }

    #[test]
    fn it_cannot_be_hurt_in_the_moment_it_vanishes() {
        let tiles = arena();
        let mut k = king();
        let (cx, cy) = k.center();
        let t = Some(player_at(cx + 200.0, cy));
        // Past its first tick, which is where it sets its own timers up.
        k.local_ai[3] = 1.0;
        k.ai[3] = k.life_max as f32;
        k.ai[1] = FADING_OUT;
        k.ai[0] = KING_SLIME_FADE_OUT - 1.0;
        update(&mut k, &world(&tiles, t), &mut rng());
        assert!(k.stats.dont_take_damage, "untouchable as it goes");
        // And vulnerable again once it is back.
        k.ai[1] = 0.0;
        update(&mut k, &world(&tiles, t), &mut rng());
        assert!(!k.stats.dont_take_damage);
    }

    #[test]
    fn holding_it_at_range_makes_it_land_on_top_of_you() {
        let tiles = arena();
        let t = player_at(200.0 * TILE + 4000.0, 299.0 * TILE);
        let cornered = find_landing(&world(&tiles, Some(t)), t, true, &mut rng());
        assert_eq!(cornered.0, t.center.0, "no more being fussy about it");
    }

    #[test]
    fn a_player_who_runs_right_away_is_given_up_on() {
        let tiles = arena();
        let mut k = king();
        let (cx, cy) = k.center();
        update(
            &mut k,
            &world(&tiles, Some(player_at(cx + KING_SLIME_GIVE_UP + 500.0, cy))),
            &mut rng(),
        );
        assert!(k.time_left <= 10);
    }
}
