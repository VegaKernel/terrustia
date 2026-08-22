//! Styles 113 and 125 — the balloons.
//!
//! A balloon does not fly, it *floats*, and everything it does follows from that. It drifts
//! sideways on the wind, rises when there is nothing beneath it and sinks when there is, and when
//! a player comes within four hundred pixels it stops drifting and matches their height instead.
//!
//! And it pops. Touching water, hitting a wall, or bumping a ceiling on the way up all end it on
//! the spot — the routine shoves it back the way it came, turns it round, and kills it.

use terrustia_proto::npc_params::{
    BALLOON_CHASE_ACCEL, BALLOON_CHASE_RANGE, BALLOON_CHASE_SPEED, BALLOON_LOOKDOWN,
    BALLOON_TOO_LOW, balloon,
};
use terrustia_proto::tile_solid::solid;

use super::{World, can_see};
use crate::game::npc::{Npc, TILE, TileView};

/// What a balloon's tick concluded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Afloat,
    /// It burst. The NPC is gone.
    Popped,
}

/// Drive one balloon for a tick.
pub fn update<T: TileView>(npc: &mut Npc, world: &World<'_, T>) -> Outcome {
    npc.sprite_direction = npc.direction;
    npc.rotation = npc.velocity.0 * 0.05;

    // How far off the target is, and how much higher — which is how far ahead it looks for ground.
    let mut reach = f32::INFINITY;
    let mut headroom = 0;
    if let Some(t) = world.target {
        let bottom = (
            npc.position.0 + npc.width() / 2.0,
            npc.position.1 + npc.height(),
        );
        let (dx, dy) = (t.center.0 - bottom.0, t.center.1 - bottom.1);
        if dy < 0.0 {
            headroom = (dy as i32) / -16;
        }
        reach = (dx * dx + dy * dy).sqrt();
        if i32::from(npc.direction) != dx.signum() as i32 {
            headroom = 0;
        }
    }

    // Anything sharp, or wet, and it is over.
    if world.wet || npc.collide_x || (npc.collide_y && npc.old_velocity.1 < 0.0) {
        let recoil = npc.old_velocity.0 + (npc.direction as f32) * 8.0;
        npc.position.0 -= recoil;
        npc.direction = -npc.direction;
        npc.velocity.0 = f32::from(npc.direction) * 2.0;
        npc.dirty = true;
        return Outcome::Popped;
    }
    if npc.collide_y {
        npc.velocity.1 = if npc.old_velocity.1 > 0.0 { 1.0 } else { -1.0 };
    }

    // Drift, faster in a stronger wind.
    let params = balloon(npc.npc_type);
    let top = params.speed + world.conditions.wind.abs() * 2.0;
    let facing = f32::from(npc.direction);
    if npc.velocity.0.signum() as i32 != i32::from(npc.direction) || npc.velocity.0.abs() < top {
        npc.velocity.0 += facing * params.push;
        if npc.velocity.0 * facing < 0.0 {
            npc.velocity.0 += facing
                * if npc.velocity.0.abs() > top {
                    params.reverse_fast
                } else {
                    params.reverse_slow
                };
        } else if npc.velocity.0.abs() > top {
            npc.velocity.0 = facing * top;
        }
    }

    // Look down and ahead: is there ground under where it is going, and how close?
    let probe_x =
        ((npc.position.0 + (npc.stats.width / 2) as f32) / TILE) as i32 + i32::from(npc.direction);
    let probe_y = ((npc.position.1 + npc.height()) / TILE) as i32;
    let mut over_open_air = true;
    let mut too_low = false;
    for step in 0..BALLOON_LOOKDOWN + headroom {
        let tile = world.tiles.tile(probe_x, probe_y + step);
        if (tile.is_active() && solid(tile.block)) || tile.liquid > 0 {
            if step < BALLOON_TOO_LOW + headroom {
                too_low = true;
            }
            over_open_air = false;
            break;
        }
    }

    let closing =
        reach < BALLOON_CHASE_RANGE && world.target.is_some_and(|t| can_see(world.tiles, npc, t));
    if closing && let Some(t) = world.target {
        // Match the target's height rather than drifting.
        let mine = npc.center().1 + (npc.stats.height / 4) as f32;
        let theirs =
            t.center.1 - super::PLAYER_HEIGHT as f32 / 2.0 + (super::PLAYER_HEIGHT / 4) as f32;
        if mine > theirs && npc.velocity.1 > -BALLOON_CHASE_SPEED {
            npc.velocity.1 -= BALLOON_CHASE_ACCEL;
            if npc.velocity.1 > 0.0 {
                npc.velocity.1 -= BALLOON_CHASE_ACCEL;
            }
        } else if mine < theirs && npc.velocity.1 < BALLOON_CHASE_SPEED {
            npc.velocity.1 += BALLOON_CHASE_ACCEL;
            if npc.velocity.1 < 0.0 {
                npc.velocity.1 += BALLOON_CHASE_ACCEL;
            }
        }
    } else {
        // Nothing to chase: sink over solid ground, rise over a drop.
        if over_open_air {
            npc.velocity.1 += 0.05;
        } else {
            npc.velocity.1 -= 0.1;
        }
        if too_low {
            npc.velocity.1 -= 0.2;
        }
        npc.velocity.1 = npc.velocity.1.clamp(-4.0, 2.0);
    }

    npc.dirty = true;
    Outcome::Afloat
}

/// Where a carrying balloon wants its passenger to hang.
pub fn carry_point(npc: &Npc) -> (f32, f32) {
    (
        npc.position.0 + npc.width() / 2.0,
        npc.position.1 + npc.height() - 8.0 + 56.0 * npc.scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::{Liquid, Tile};

    #[derive(Default)]
    struct Sky(HashMap<(i32, i32), Tile>);

    impl TileView for Sky {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn ground(from: i32, to: i32, top: i32) -> Sky {
        let mut s = Sky::default();
        for x in from..to {
            for y in top..top + 20 {
                s.0.insert((x, y), Tile::block(1));
            }
        }
        s
    }

    fn balloon_at(npc_type: u16, tile_x: i32, tile_y: i32) -> Npc {
        Npc::new(npc_type, (tile_x as f32 * TILE, tile_y as f32 * TILE), 1).expect("a balloon type")
    }

    fn world<'a>(tiles: &'a Sky, target: Option<Target>) -> World<'a, Sky> {
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
    fn a_balloon_pops_in_water() {
        let tiles = Sky::default();
        let mut b = balloon_at(594, 100, 100);
        let mut w = world(&tiles, None);
        w.wet = true;
        assert_eq!(update(&mut b, &w), Outcome::Popped);
    }

    #[test]
    fn a_balloon_pops_on_a_wall() {
        let tiles = Sky::default();
        let mut b = balloon_at(594, 100, 100);
        b.collide_x = true;
        b.old_velocity = (2.0, 0.0);
        assert_eq!(update(&mut b, &world(&tiles, None)), Outcome::Popped);
    }

    #[test]
    fn a_balloon_pops_on_a_ceiling_but_not_on_the_ground() {
        let tiles = Sky::default();
        let mut rising = balloon_at(594, 100, 100);
        rising.collide_y = true;
        rising.old_velocity = (0.0, -2.0);
        assert_eq!(update(&mut rising, &world(&tiles, None)), Outcome::Popped);

        let mut settling = balloon_at(594, 100, 100);
        settling.collide_y = true;
        settling.old_velocity = (0.0, 2.0);
        assert_eq!(update(&mut settling, &world(&tiles, None)), Outcome::Afloat);
    }

    #[test]
    fn a_balloon_over_a_drop_climbs_and_over_ground_sinks() {
        // Ground far below, well past its lookdown.
        let empty = Sky::default();
        let mut high = balloon_at(594, 100, 100);
        high.direction = 1;
        update(&mut high, &world(&empty, None));
        assert!(high.velocity.1 > 0.0, "should sink toward the drop");

        let close = ground(0, 400, 102);
        let mut low = balloon_at(594, 100, 100);
        low.direction = 1;
        update(&mut low, &world(&close, None));
        assert!(low.velocity.1 < 0.0, "should climb away from the ground");
    }

    #[test]
    fn a_balloon_near_a_player_matches_their_height() {
        let tiles = Sky::default();
        let mut b = balloon_at(594, 100, 100);
        b.direction = 1;
        let (cx, cy) = b.center();
        // Player a little above and well within range.
        let t = Some(player_at(cx + 100.0, cy - 200.0));
        for _ in 0..30 {
            update(&mut b, &world(&tiles, t));
        }
        assert!(
            b.velocity.1 < 0.0,
            "should rise to them, got {}",
            b.velocity.1
        );
    }

    #[test]
    fn a_stronger_wind_carries_a_balloon_faster() {
        let tiles = Sky::default();
        let mut still = balloon_at(594, 100, 100);
        let mut blown = balloon_at(594, 100, 100);
        still.direction = 1;
        blown.direction = 1;
        let calm = world(&tiles, None);
        let mut gale = world(&tiles, None);
        gale.conditions.wind = 1.0;
        for _ in 0..2000 {
            update(&mut still, &calm);
            update(&mut blown, &gale);
        }
        assert!(
            blown.velocity.0 > still.velocity.0 + 1.0,
            "wind should help: {} against {}",
            blown.velocity.0,
            still.velocity.0
        );
    }

    #[test]
    fn a_clumsy_slime_balloon_is_brisker_than_a_windy_one() {
        assert!(balloon(686).speed > balloon(594).speed);
        assert!(balloon(686).push > balloon(594).push);
    }

    #[test]
    fn a_pop_shoves_the_balloon_back_the_way_it_came() {
        let tiles = Sky::default();
        let mut b = balloon_at(594, 100, 100);
        b.direction = 1;
        b.old_velocity = (2.0, 0.0);
        b.collide_x = true;
        let before = b.position.0;
        update(&mut b, &world(&tiles, None));
        assert!(b.position.0 < before, "should recoil");
        assert_eq!(b.direction, -1);
    }

    #[test]
    fn the_carry_point_hangs_below_the_balloon() {
        let b = balloon_at(594, 100, 100);
        let at = carry_point(&b);
        assert!(
            at.1 > b.position.1 + b.height(),
            "the passenger hangs below"
        );
    }

    #[test]
    fn deep_water_ahead_counts_as_ground_to_stay_above() {
        let mut tiles = Sky::default();
        for x in 90..120 {
            for y in 102..110 {
                tiles
                    .0
                    .insert((x, y), Tile::AIR.with_liquid(Liquid::Water, 255));
            }
        }
        let mut b = balloon_at(594, 100, 100);
        b.direction = 1;
        update(&mut b, &world(&tiles, None));
        assert!(b.velocity.1 < 0.0, "should climb away from the water");
    }
}
