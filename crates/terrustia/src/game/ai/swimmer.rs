//! Styles 18 and 44 — the two ways of moving through a medium rather than over ground.
//!
//! A **jellyfish** (18) has one trick: it does not steer toward you, it *stops*. While a wet target
//! is in view it lets its speed bleed away tick by tick, and the instant that speed drops below a
//! threshold it fires itself in a straight line at whatever it was watching. The whole pulsing
//! rhythm of a jellyfish falls out of those two numbers. Out of the water it has nothing at all and
//! simply falls.
//!
//! A **flying fish** (44) hovers: it accelerates sideways only when far enough away to bother, and
//! holds a fixed height above its target, dropping to that height when it gets close. Losing sight
//! of the target does not stop it - it keeps hunting for ninety more ticks and only then wanders.

use terrustia_proto::npc_params::{HOVER_ATTENTION, hover, jelly};

use super::{World, can_see, face, rise_out_of_water};
use crate::game::npc::{Npc, TILE, TileView};

/// Eyeball Flying Fish (`NPCID.EyeballFlyingFish`), the one style-44 type that hunts you *into* the
/// water and behaves differently by day.
const EYEBALL_FISH: u16 = 587;

/// How far apart two of the same kind push each other, from the `velocity.X -= 0.05f` shove at
/// `NPC.cs:31191-31206`.
const SHOVE: f32 = 0.05;

/// Gravity on a jellyfish out of water, and its terminal speed.
const BEACHED_GRAVITY: f32 = 0.2;
const BEACHED_TERMINAL: f32 = 10.0;

/// How gently a drifting jellyfish pushes itself along, and the speed it starts shedding at.
const DRIFT_PUSH: f32 = 0.02;
const DRIFT_LIMIT: f32 = 1.0;
/// ...and how fast it bobs up and down.
const BOB: f32 = 0.01;

/// Liquid this deep in the tile above counts as being under the surface.
const SUBMERGED: u8 = 128;

/// Whether a type gets the expert electrified phase (`NPC.cs:24282`). The squid does not.
fn electrifies(npc_type: u16) -> bool {
    matches!(npc_type, 63 | 64 | 103 | 242)
}

/// Drive one jellyfish for a tick.
pub fn jellyfish<T: TileView>(npc: &mut Npc, world: &World<'_, T>) {
    // The expert electrified phase, `NPC.cs:24276-24345`. `ai[1]` is only ever set outside classic
    // mode, so `charged` is false there. It is not merely a damage state: a charged jellyfish stops
    // dead and cannot be hurt, and the routine returns before any movement at all.
    let charged = world.wet && npc.ai[1] == 1.0;
    npc.invulnerable = charged || npc.stats.dont_take_damage;
    // One line of sight per tick, shared by the electrified phase and the hunt below. The game asks
    // twice; a raycast is the most expensive thing in this routine and the answer cannot change
    // between the two.
    let in_view = world
        .target
        .is_some_and(|t| world.target_wet && t.alive && can_see(world.tiles, npc, t));
    if world.conditions.expert && electrifies(npc.npc_type) {
        if world.wet {
            // Sharing the water with it charges it twice as fast, and discharges it sooner.
            let sharing = world.target.is_some_and(|t| {
                in_view
                    && ((t.center.0 - npc.center().0).powi(2)
                        + (t.center.1 - npc.center().1).powi(2))
                    .sqrt()
                        < 150.0
            });
            if sharing {
                npc.ai[2] += if npc.ai[1] == 0.0 { 2.0 } else { -0.25 };
            }
            npc.ai[2] += 1.0;
            if charged {
                // Note the game does not reset `ai[2]` on the way out, so the next charge-up is
                // shorter than the first by however long this discharge ran.
                if npc.ai[2] >= 120.0 {
                    npc.ai[1] = 0.0;
                    npc.dirty = true;
                }
            } else if npc.ai[2] >= 420.0 {
                npc.ai[1] = 1.0;
                npc.ai[2] = 0.0;
                npc.dirty = true;
            }
        } else {
            npc.ai[1] = 0.0;
            npc.ai[2] = 0.0;
        }
    }

    if npc.direction == 0
        && let Some(t) = world.target
    {
        face(npc, t);
    }

    // `NPC.cs:24344`: charged, it holds absolutely still.
    if charged {
        npc.dirty = true;
        return;
    }

    if !world.wet {
        // Beached. It tumbles and falls, and resets its bob so it resumes upward on re-entry.
        npc.rotation += npc.velocity.0 * 0.1;
        if npc.velocity.1 == 0.0 {
            npc.velocity.0 *= 0.98;
            if npc.velocity.0 > -0.01 && npc.velocity.0 < 0.01 {
                npc.velocity.0 = 0.0;
            }
        }
        npc.velocity.1 = (npc.velocity.1 + BEACHED_GRAVITY).min(BEACHED_TERMINAL);
        npc.ai[0] = 1.0;
        npc.dirty = true;
        return;
    }

    if npc.collide_x {
        npc.velocity.0 = -npc.velocity.0;
        npc.direction = -npc.direction;
    }
    if npc.collide_y {
        if npc.velocity.1 > 0.0 {
            npc.velocity.1 = -npc.velocity.1.abs();
            npc.direction_y = -1;
            npc.ai[0] = -1.0;
        } else if npc.velocity.1 < 0.0 {
            npc.velocity.1 = npc.velocity.1.abs();
            npc.direction_y = 1;
            npc.ai[0] = 1.0;
        }
    }

    let hunting = !npc.stats.friendly && in_view;

    if hunting {
        let params = jelly(npc.npc_type);
        npc.rotation = npc.velocity.1.atan2(npc.velocity.0) + 1.57;
        npc.velocity.0 *= params.drag;
        npc.velocity.1 *= params.drag;
        let slow = npc.velocity.0 > -params.trigger
            && npc.velocity.0 < params.trigger
            && npc.velocity.1 > -params.trigger
            && npc.velocity.1 < params.trigger;
        if slow && let Some(t) = world.target {
            face(npc, t);
            let (cx, cy) = npc.center();
            let (dx, dy) = (t.center.0 - cx, t.center.1 - cy);
            let k = params.lunge / (dx * dx + dy * dy).sqrt();
            npc.velocity = (dx * k, dy * k);
            npc.dirty = true;
        }
        npc.dirty = true;
        return;
    }

    // Drifting. A slow horizontal push, a slow vertical bob, and a check that it is not about to
    // drift out of the water it lives in.
    npc.velocity.0 += f32::from(npc.direction) * DRIFT_PUSH;
    npc.rotation = npc.velocity.0 * 0.4;
    if npc.velocity.0 < -DRIFT_LIMIT || npc.velocity.0 > DRIFT_LIMIT {
        npc.velocity.0 *= 0.95;
    }
    if npc.ai[0] == -1.0 {
        npc.velocity.1 -= BOB;
        if npc.velocity.1 < -DRIFT_LIMIT {
            npc.ai[0] = 1.0;
        }
    } else {
        npc.velocity.1 += BOB;
        if npc.velocity.1 > DRIFT_LIMIT {
            npc.ai[0] = -1.0;
        }
    }

    // Under the surface with ground close beneath: turn back up rather than settling on it.
    let (cx, cy) = npc.center();
    let (tile_x, tile_y) = ((cx / TILE) as i32, (cy / TILE) as i32);
    if world.tiles.tile(tile_x, tile_y - 1).liquid > SUBMERGED {
        if world.tiles.tile(tile_x, tile_y + 1).is_active()
            || world.tiles.tile(tile_x, tile_y + 2).is_active()
        {
            npc.ai[0] = -1.0;
        }
    } else {
        // At or above the surface, it always heads back down.
        npc.ai[0] = 1.0;
    }
    if npc.velocity.1 > 1.2 || npc.velocity.1 < -1.2 {
        npc.velocity.1 *= 0.99;
    }
    npc.dirty = true;
}

/// Drive one flying fish for a tick.
///
/// The shoal it keeps out of comes in on [`World::avoid`], which only the two types that jostle
/// ever ask for.
pub fn flying_fish<T: TileView>(npc: &mut Npc, world: &World<'_, T>) {
    npc.no_gravity = true;

    if npc.collide_x {
        npc.direction = if npc.old_velocity.0 > 0.0 { -1 } else { 1 };
        npc.velocity.0 = f32::from(npc.direction);
    }
    if npc.collide_y {
        npc.direction_y = if npc.old_velocity.1 > 0.0 { -1 } else { 1 };
        npc.velocity.1 = f32::from(npc.direction_y);
    }

    // Attention, `NPC.cs:31155-31164`. The timer is refreshed by everything *except* a live target
    // it cannot reach: no target at all, a target in the water (which only type 587 will follow), a
    // dead one, or one it can hit all reset it to ninety. It counts down only while there is
    // somebody alive, out of the water, and out of reach. Both arms re-target, and a fish whose
    // attention has already run out does neither, so it keeps whatever heading it had.
    let hopeless = world.target.is_some_and(|t| {
        !(npc.npc_type != EYEBALL_FISH && world.target_wet)
            && t.alive
            && !can_see(world.tiles, npc, t)
    });
    if !hopeless || npc.ai[0] > 0.0 {
        if hopeless {
            npc.ai[0] -= 1.0;
        } else {
            npc.ai[0] = HOVER_ATTENTION;
        }
        if let Some(t) = world.target {
            face(npc, t);
        }
    }

    let mut params = hover(npc.npc_type);
    if params.avoids_its_own_kind {
        // `NPC.cs:31191-31228`: a nudge away from each of its own kind close enough to touch, and
        // the roll that goes with it. Only the two lunar swarmers do this.
        let (cx, cy) = npc.center();
        // The reach an entry carries is ignored here, as in every other avoid consumer: this one
        // has its own threshold, vanilla's "close enough to touch" test against the hitbox width.
        for &(kx, ky, _) in world.avoid {
            let (dx, dy) = (cx - kx, cy - ky);
            // Its own entry in the list, which the game skips by slot.
            if dx == 0.0 && dy == 0.0 {
                continue;
            }
            if dx.abs() + dy.abs() < npc.width() {
                npc.velocity.0 += if dx < 0.0 { -SHOVE } else { SHOVE };
                npc.velocity.1 += if dy < 0.0 { -SHOVE } else { SHOVE };
            }
        }
        npc.rotation = npc.velocity.0 * 0.1;
    }

    let (_, cy) = npc.center();
    let mut across = 0.0;
    let mut wanted_y = npc.position.1;
    if let Some(t) = world.target {
        across = (npc.position.0 + npc.width() / 2.0 - t.center.0).abs();
        // `num717`, which is measured from a different point for each of the three groups
        // (`NPC.cs:31176`, `:31186`, `:31213`, `:31245`).
        let top = t.center.1 - super::PLAYER_HEIGHT as f32 / 2.0;
        wanted_y = match npc.npc_type {
            509 | 581 => t.center.1 - npc.height() / 2.0,
            EYEBALL_FISH => top,
            _ => top - npc.height() / 2.0,
        };
        // `NPC.cs:31245-31249`: a merman caught in daylight climbs for the top of the world and
        // turns round, which is how one gets out of the way once the night is over.
        if npc.npc_type == EYEBALL_FISH && world.conditions.day {
            wanted_y = 0.0;
            npc.direction = -npc.direction;
        }
    }

    if npc.ai[0] <= 0.0 {
        // Given up: drift, slower and lazier, toward nothing in particular.
        params.max_x *= 0.8;
        params.accel_x *= 0.7;
        wanted_y = cy + f32::from(npc.direction_y) * 1000.0;
        npc.direction = if npc.velocity.0 < 0.0 { -1 } else { 1 };
    }

    if across > params.deadband {
        if npc.direction == -1 && npc.velocity.0 > -params.max_x {
            npc.velocity.0 -= params.accel_x;
            if npc.velocity.0 > params.max_x {
                npc.velocity.0 -= params.accel_x;
            } else if npc.velocity.0 > 0.0 {
                npc.velocity.0 -= params.accel_x / 2.0;
            }
            npc.velocity.0 = npc.velocity.0.max(-params.max_x);
        } else if npc.direction == 1 && npc.velocity.0 < params.max_x {
            npc.velocity.0 += params.accel_x;
            if npc.velocity.0 < -params.max_x {
                npc.velocity.0 += params.accel_x;
            } else if npc.velocity.0 < 0.0 {
                npc.velocity.0 += params.accel_x / 2.0;
            }
            npc.velocity.0 = npc.velocity.0.min(params.max_x);
        }
    }

    // Far away it climbs; close in it drops to its target's own height.
    if across > params.climb_at {
        wanted_y -= params.climb_at / 2.0;
    }
    if npc.position.1 < wanted_y {
        npc.velocity.1 += params.accel_y;
        if npc.velocity.1 < 0.0 {
            npc.velocity.1 += params.accel_y;
        }
    } else {
        npc.velocity.1 -= params.accel_y;
        if npc.velocity.1 > 0.0 {
            npc.velocity.1 -= params.accel_y;
        }
    }
    npc.velocity.1 = npc.velocity.1.clamp(-params.max_y, params.max_y);

    // `NPC.cs:31333`: everything but the Zombie Merman claws its way back out of water. The merman
    // hunts you into it, which is the whole point of the type.
    if world.wet && npc.npc_type != EYEBALL_FISH {
        rise_out_of_water(npc);
    }
    npc.sprite_direction = npc.direction;
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::{Liquid, Tile};

    #[derive(Default)]
    struct Sea(HashMap<(i32, i32), Tile>);

    impl TileView for Sea {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn ocean() -> Sea {
        let mut s = Sea::default();
        for x in 400..700 {
            for y in 400..700 {
                s.0.insert((x, y), Tile::AIR.with_liquid(Liquid::Water, 255));
            }
        }
        s
    }

    fn at(npc_type: u16, tile_x: i32, tile_y: i32) -> Npc {
        Npc::new(npc_type, (tile_x as f32 * TILE, tile_y as f32 * TILE), 1).expect("a swimmer type")
    }

    fn world<'a>(tiles: &'a Sea, target: Option<Target>) -> World<'a, Sea> {
        // These are swimmers, so the default world for them is underwater.
        World {
            wet: true,
            target_wet: true,
            ..crate::game::ai::calm(tiles, target)
        }
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
    fn a_beached_jellyfish_falls() {
        let tiles = ocean();
        let mut j = at(63, 100, 100);
        let mut w = world(&tiles, None);
        w.wet = false;
        jellyfish(&mut j, &w);
        assert_eq!(j.velocity.1, BEACHED_GRAVITY);
    }

    #[test]
    fn a_jellyfish_winds_down_and_then_lunges() {
        let tiles = ocean();
        let mut j = at(63, 500, 500);
        let (cx, cy) = j.center();
        let t = Some(player_at(cx + 200.0, cy));
        j.velocity = (5.0, 0.0);
        let mut launched = false;
        for _ in 0..400 {
            let before = (j.velocity.0.powi(2) + j.velocity.1.powi(2)).sqrt();
            jellyfish(&mut j, &world(&tiles, t));
            let after = (j.velocity.0.powi(2) + j.velocity.1.powi(2)).sqrt();
            if after > before + 1.0 {
                launched = true;
                break;
            }
        }
        assert!(launched, "should have fired itself at the player");
        let speed = (j.velocity.0.powi(2) + j.velocity.1.powi(2)).sqrt();
        assert!(
            (speed - jelly(63).lunge).abs() < 1e-3,
            "at its lunge speed, got {speed}"
        );
        assert!(j.velocity.0 > 0.0, "and toward them");
    }

    /// `NPC.cs:24551-24567`. The game multiplies every jellyfish's velocity by 0.98 and only then
    /// applies the per-type extra, so a squid's real drag is 0.98 * 0.99 and a blood jelly's is
    /// 0.98 * 0.995. Reading the extra as the whole drag made a squid coast to its trigger in
    /// about 194 ticks instead of 64.
    #[test]
    fn a_squid_lets_go_much_sooner_than_a_jellyfish() {
        assert!(jelly(221).trigger > jelly(63).trigger);
        assert!(
            jelly(221).drag < jelly(63).drag,
            "and sheds speed faster, not slower: {} against {}",
            jelly(221).drag,
            jelly(63).drag
        );
        assert!((jelly(221).drag - 0.9702).abs() < 1e-5);
        assert!((jelly(242).drag - 0.9751).abs() < 1e-5);
        assert!((jelly(103).drag - 0.9604).abs() < 1e-5);
        assert!((jelly(63).drag - 0.98).abs() < 1e-5);

        // The whole point of the number: how long the wind-up actually lasts.
        let ticks_to_trigger = |ty: u16| {
            let p = jelly(ty);
            let mut speed = p.lunge;
            let mut n = 0;
            while speed >= p.trigger && n < 10_000 {
                speed *= p.drag;
                n += 1;
            }
            n
        };
        assert_eq!(ticks_to_trigger(221), 65, "a squid");
        assert_eq!(ticks_to_trigger(242), 34, "a blood jelly");
        // Against 194 and 169 with the inner factor read as the whole drag.
        assert_eq!(
            [0.99f32, 0.995].map(|wrong| {
                let p = jelly(221);
                let mut speed = p.lunge;
                let mut n = 0;
                while speed >= p.trigger && n < 10_000 {
                    speed *= wrong;
                    n += 1;
                }
                n
            })[0],
            194,
            "what the old number gave"
        );
    }

    /// `NPC.cs:24276-24345`: in expert a jellyfish charges for 420 ticks, then freezes solid and
    /// invulnerable for 120. This is not a damage flag: the routine returns before it moves.
    #[test]
    fn an_expert_jellyfish_freezes_solid_while_it_is_electrified() {
        let tiles = ocean();
        let mut j = at(63, 500, 500);
        j.velocity = (3.0, 1.0);
        let mut w = world(&tiles, None);
        w.conditions.expert = true;

        for _ in 0..420 {
            jellyfish(&mut j, &w);
        }
        assert_eq!(j.ai[1], 1.0, "charged");

        // The game reads its own flag at the top of the tick, so the freeze starts on the next one.
        let held = (j.velocity, j.position);
        let mut frozen = 0;
        while j.ai[1] == 1.0 && frozen < 500 {
            jellyfish(&mut j, &w);
            frozen += 1;
            assert!(j.invulnerable, "untouchable while charged");
            assert_eq!(j.velocity, held.0, "and it does not move at all");
            assert_eq!(j.position, held.1);
        }
        assert_eq!(frozen, 120, "two seconds of it");

        jellyfish(&mut j, &w);
        assert!(!j.invulnerable, "and then it is a jellyfish again");
        assert_ne!(j.velocity, held.0, "moving again");
    }

    /// Classic mode never sets `ai[1]`, so the phase cannot start, and neither can a squid's.
    #[test]
    fn nothing_electrifies_in_classic_and_a_squid_never_does() {
        let tiles = ocean();
        for (kind, expert) in [(63u16, false), (221, true)] {
            let mut j = at(kind, 500, 500);
            let mut w = world(&tiles, None);
            w.conditions.expert = expert;
            for _ in 0..600 {
                jellyfish(&mut j, &w);
            }
            assert_eq!(j.ai[1], 0.0, "type {kind} expert={expert}");
            assert!(!j.invulnerable);
        }
    }

    #[test]
    fn a_jellyfish_with_nobody_wet_to_chase_just_drifts() {
        let tiles = ocean();
        let mut j = at(63, 500, 500);
        j.direction = 1;
        let (cx, cy) = j.center();
        let mut w = world(&tiles, Some(player_at(cx + 200.0, cy)));
        // On dry land, so not worth chasing.
        w.target_wet = false;
        for _ in 0..50 {
            jellyfish(&mut j, &w);
        }
        let speed = (j.velocity.0.powi(2) + j.velocity.1.powi(2)).sqrt();
        assert!(speed < 2.0, "a drift, not a charge: {speed}");
        assert!(j.velocity.0 > 0.0, "and it should be going somewhere");
    }

    #[test]
    fn a_drifting_jellyfish_bobs_up_and_down() {
        let tiles = ocean();
        let mut j = at(63, 500, 500);
        let mut ups = 0;
        let mut downs = 0;
        for _ in 0..600 {
            jellyfish(&mut j, &world(&tiles, None));
            if j.velocity.1 < 0.0 {
                ups += 1;
            }
            if j.velocity.1 > 0.0 {
                downs += 1;
            }
        }
        assert!(ups > 50 && downs > 50, "should bob: {ups} up, {downs} down");
    }

    #[test]
    fn a_flying_fish_holds_station_above_its_target() {
        let tiles = Sea::default();
        let mut f = at(224, 500, 500);
        let (cx, cy) = f.center();
        let t = Some(player_at(cx + 600.0, cy));
        let mut w = world(&tiles, t);
        w.wet = false;
        w.target_wet = false;
        let mut highest = cy;
        for _ in 0..600 {
            flying_fish(&mut f, &w);
            f.position.0 += f.velocity.0;
            f.position.1 += f.velocity.1;
            highest = highest.min(f.center().1);
        }
        assert!(f.no_gravity, "it flies");
        assert!(f.center().0 > cx, "and closes the gap");
        assert!(highest < cy, "climbing above its target on the way");
    }

    /// `NPC.cs:31155-31164`. The timer counts down only for a live, dry, unreachable target, and is
    /// refreshed by everything else. The negation was inverted for three of the four arms, so a
    /// flying fish gave up the moment its target stepped into water, died, or logged out.
    #[test]
    fn a_flying_fish_only_gives_up_on_a_target_it_cannot_reach() {
        let (cx, cy) = at(224, 500, 500).center();

        // Blocked line of sight to a live, dry player: this is the one case that counts down.
        let mut walled = Sea::default();
        for y in 480..520 {
            walled.0.insert((510, y), Tile::block(1));
        }
        let mut f = at(224, 500, 500);
        let mut w = world(&walled, Some(player_at(cx + 600.0, cy)));
        w.wet = false;
        w.target_wet = false;
        f.ai[0] = HOVER_ATTENTION;
        for _ in 0..10 {
            flying_fish(&mut f, &w);
        }
        assert_eq!(f.ai[0], HOVER_ATTENTION - 10.0, "out of reach, so it fades");

        // The same target, but standing in water: the game refreshes rather than giving up.
        let refreshes = |set: &dyn Fn(&mut World<'_, Sea>)| {
            let mut f = at(224, 500, 500);
            f.ai[0] = 10.0;
            let mut w = world(&walled, Some(player_at(cx + 600.0, cy)));
            w.wet = false;
            w.target_wet = false;
            set(&mut w);
            flying_fish(&mut f, &w);
            f.ai[0]
        };
        assert_eq!(
            refreshes(&|w| w.target_wet = true),
            HOVER_ATTENTION,
            "a target in the water"
        );
        assert_eq!(
            refreshes(&|w| w.target = w.target.map(|mut t| {
                t.alive = false;
                t
            })),
            HOVER_ATTENTION,
            "a dead target"
        );
        assert_eq!(
            refreshes(&|w| w.target = None),
            HOVER_ATTENTION,
            "no target"
        );

        // ...but an Eyeball Flying Fish follows you in.
        let mut eyeball = at(EYEBALL_FISH, 500, 500);
        eyeball.ai[0] = 10.0;
        let mut w = world(&walled, Some(player_at(cx + 600.0, cy)));
        w.wet = false;
        w.target_wet = true;
        flying_fish(&mut eyeball, &w);
        assert_eq!(eyeball.ai[0], 9.0, "it keeps hunting into the water");
    }

    /// `NPC.cs:31333`: everything but type 587 claws its way back out of water, with a half-pixel
    /// kick a tick that dwarfs the hover's own acceleration.
    #[test]
    fn an_eyeball_fish_stays_under_and_a_flying_fish_does_not() {
        let tiles = ocean();
        let mut fish = at(224, 500, 500);
        let mut eyeball = at(EYEBALL_FISH, 500, 500);
        let w = world(&tiles, None);
        flying_fish(&mut fish, &w);
        flying_fish(&mut eyeball, &w);
        assert!(fish.velocity.1 <= -0.5, "a flying fish surfaces hard");
        assert!(
            eyeball.velocity.1 > -0.5,
            "type 587 does not: {}",
            eyeball.velocity.1
        );
    }

    /// `NPC.cs:31245-31249`: daylight sends type 587 for the top of the world, turning as it goes.
    #[test]
    fn daylight_turns_an_eyeball_fish_round_and_sends_it_up() {
        let tiles = Sea::default();
        let mut m = at(EYEBALL_FISH, 500, 500);
        m.direction = 1;
        let (cx, cy) = m.center();
        let mut w = world(&tiles, Some(player_at(cx + 100.0, cy)));
        w.wet = false;
        w.target_wet = false;
        w.conditions.day = true;
        flying_fish(&mut m, &w);
        assert_eq!(m.direction, -1, "it turns");
        assert!(m.velocity.1 < 0.0, "and climbs");
    }

    #[test]
    fn a_flying_antlion_is_quicker_than_a_flying_fish_and_minds_its_neighbours() {
        assert!(hover(581).max_x > hover(224).max_x);
        assert!(hover(581).avoids_its_own_kind);
        assert!(!hover(224).avoids_its_own_kind);
    }

    /// The flag was set but nothing fed it: the caller passed a literal zero and `reads_crowding`
    /// did not list style 44, so the shove and the roll it carries were both dead.
    #[test]
    fn a_flying_antlion_shoves_its_own_kind_aside() {
        let tiles = Sea::default();
        let (cx, cy) = at(581, 500, 500).center();
        // Third element is the entry's own reach, which this consumer ignores in favour of the
        // hitbox-width test, so any value does.
        let kin = [(cx + 4.0, cy, 0.0)];

        let mut alone = at(581, 500, 500);
        let mut crowded = at(581, 500, 500);
        let mut plain = world(&tiles, None);
        plain.wet = false;
        let mut packed = world(&tiles, None);
        packed.wet = false;
        packed.avoid = &kin;

        flying_fish(&mut alone, &plain);
        flying_fish(&mut crowded, &packed);
        assert!(
            crowded.velocity.0 < alone.velocity.0,
            "should be pushed off its neighbour: {} against {}",
            crowded.velocity.0,
            alone.velocity.0
        );

        // A flying fish is given the same list and ignores it.
        let mut fish = at(224, 500, 500);
        let mut fish_alone = at(224, 500, 500);
        let mut packed_fish = world(&tiles, None);
        packed_fish.wet = false;
        packed_fish.avoid = &kin;
        flying_fish(&mut fish, &packed_fish);
        flying_fish(&mut fish_alone, &plain);
        assert_eq!(fish.velocity, fish_alone.velocity);
    }

    #[test]
    fn a_flying_fish_bounces_off_what_it_clips() {
        let tiles = Sea::default();
        let mut f = at(224, 500, 500);
        f.old_velocity = (3.0, 0.0);
        f.collide_x = true;
        let mut w = world(&tiles, None);
        w.wet = false;
        flying_fish(&mut f, &w);
        assert_eq!(f.direction, -1, "should turn away from the wall");
    }
}
