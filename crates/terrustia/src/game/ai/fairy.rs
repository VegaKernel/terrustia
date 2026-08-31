//! Style 112: the fairy critters.
//!
//! A fairy is the only NPC in the game whose whole purpose is to give you something. It drifts
//! about its home spot until you come within two hundred and fifty pixels, then follows you until
//! it can touch you, then goes looking — a hundred and fifty tiles across, fifty down — for the
//! single best thing on the ore finder's ranking. If it finds one it celebrates, then leads you
//! there, keeping ahead of you but never further than three hundred pixels, and dances over the
//! spot before it vanishes.
//!
//! If it finds nothing it hovers by you and keeps checking, because the world moves: a fairy that
//! found nothing where you were standing will find something once you have walked somewhere else.
//!
//! It gives you five minutes. After that it turns away and flies off, and no amount of following
//! brings it back.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    FAIRY_ARRIVAL, FAIRY_CELEBRATE, FAIRY_LEAD, FAIRY_NOTICE, FAIRY_PATIENCE, FAIRY_SEARCH_X,
    FAIRY_SEARCH_Y, FAIRY_VEIN, fairy_lures_to, is_ore, ore_finder_priority, valid_for_ore_finder,
};

use crate::game::ai::World;
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Target;

/// What the fairy is doing, as `ai[2]` numbers it. The order is the game's, not a sequence.
mod state {
    /// Drifting about the spot it appeared at.
    pub const IDLE: f32 = 0.0;
    /// Wandering off — a fairy that lost you, or one that never had you.
    pub const WANDER: f32 = 1.0;
    /// Coming to you.
    pub const APPROACH: f32 = 2.0;
    /// It found something: the little dance before it sets off.
    pub const CELEBRATE: f32 = 3.0;
    /// Leading you there.
    pub const LEAD: f32 = 4.0;
    /// Dancing over the spot, and then gone.
    pub const ARRIVED: f32 = 5.0;
    /// Nothing found: waiting by you and looking again.
    pub const LINGER: f32 = 6.0;
    /// Out of patience.
    pub const LEAVING: f32 = 7.0;
}

/// What it did this tick.
#[derive(Debug, Default)]
pub struct FairyOutcome {
    /// Set when its dance finishes and it should go, sparkling.
    pub spent: bool,
    /// Where it wants a treasure marker looked up, once, when it needs one.
    pub wants_treasure: bool,
}

pub fn fairy(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    treasure: Option<(i32, i32)>,
    rng: &mut SmallRng,
) -> FairyOutcome {
    let mut out = FairyOutcome::default();
    npc.dirty = true;
    let (cx, _) = npc.center();

    // Past the drifting states it is on a clock, and running out of it sends it away.
    if npc.ai[2] > state::WANDER {
        npc.local_ai[1] += 1.0;
        if npc.local_ai[1] >= FAIRY_PATIENCE {
            npc.ai[2] = state::LEAVING;
            npc.direction = match world.target {
                Some(t) if t.center.0 < cx => 1,
                _ => -1,
            };
        }
    }

    // Only a fairy that has not committed to anything can be hurt.
    npc.invulnerable = npc.ai[2] > state::WANDER;
    let mut dancing = false;

    match npc.ai[2] {
        state::IDLE => idle(npc, world, rng),
        state::WANDER => wander(npc, world),
        state::APPROACH => approach(npc, world, &mut out),
        state::CELEBRATE => dancing = celebrate(npc, world),
        state::LEAD => dancing = lead(npc, world),
        state::ARRIVED => {
            dancing = arrived(npc, world);
            if npc.ai[3] > FAIRY_ARRIVAL {
                out.spent = true;
            }
        }
        state::LINGER => linger(npc, world, &mut out),
        _ => {
            // Leaving: it climbs away and stops mattering.
            npc.no_tile_collide = true;
            npc.velocity.0 = (npc.velocity.0 + 0.05 * f32::from(npc.direction)).clamp(-10.0, 10.0);
            npc.velocity.1 = (npc.velocity.1 - 0.025).clamp(-5.0, 5.0);
            npc.time_left = npc.time_left.min(10);
        }
    }

    // A treasure the caller found is taken up here, so the search happens once rather than per
    // state: finding one starts the dance, finding none sends it to wait by you.
    if out.wants_treasure {
        match treasure {
            Some((tx, ty)) => {
                npc.ai[0] = tx as f32;
                npc.ai[1] = ty as f32;
                npc.ai[2] = state::CELEBRATE;
                npc.ai[3] = 0.0;
            }
            None if npc.ai[2] == state::APPROACH => {
                npc.ai[2] = state::LINGER;
                npc.ai[3] = 0.0;
            }
            None => {}
        }
    }

    // Fairies stack vertically rather than piling up, so a cluster spreads into a column.
    for &(_, ky, _) in world.avoid {
        if (npc.position.1 - ky).abs() < npc.width() * 1.5 {
            npc.velocity.1 += if npc.position.1 < ky { -0.05 } else { 0.05 };
        }
    }

    // Except while dancing, it faces the way it is going.
    if !dancing {
        npc.direction = if npc.velocity.0 >= 0.0 { 1 } else { -1 };
        npc.sprite_direction = -npc.direction;
    }
    out
}

/// Drifting about the spot it was born at, until somebody comes close.
fn idle(npc: &mut Npc, world: &World<'_, impl TileView>, rng: &mut SmallRng) {
    npc.no_tile_collide = false;
    let (cx, cy) = npc.center();
    if npc.ai[0] == 0.0 && npc.ai[1] == 0.0 {
        npc.ai[0] = cx;
        npc.ai[1] = cy;
    }
    if npc.local_ai[0] == 0.0 {
        npc.local_ai[0] = 1.0;
        let side = |rng: &mut SmallRng| (rng.random_range(0..2) * 2 - 1) as f32;
        npc.velocity = (
            (2.0 + rng.random::<f32>() * 2.0) * side(rng) * 0.7,
            (1.0 + rng.random::<f32>()) * side(rng) * 0.7,
        );
    }
    // It keeps itself loosely tethered to where it started.
    let home = (npc.ai[0] - cx, npc.ai[1] - cy);
    if home.0.hypot(home.1) > 20.0 {
        npc.velocity.0 += if home.0 > 0.0 { 0.04 } else { -0.04 };
        npc.velocity.1 += if home.1 > 0.0 { 0.04 } else { -0.04 };
        if npc.velocity.1.abs() > 2.0 {
            npc.velocity.1 *= 0.95;
        }
    }
    if let Some(t) = world.target.filter(|t| t.alive) {
        let gap = (t.center.0 - cx).hypot(t.center.1 - cy);
        if gap < FAIRY_NOTICE {
            npc.ai[2] = state::APPROACH;
            npc.direction = if t.center.0 > cx { -1 } else { 1 };
            if npc.velocity.0 * f32::from(npc.direction) < 0.0 {
                npc.velocity.0 = f32::from(npc.direction) * 2.0;
            }
            npc.ai[3] = 0.0;
        }
    }
}

/// Wandering: it walks a line, bounces off what it hits, and keeps a few tiles of air below it.
fn wander(npc: &mut Npc, world: &World<'_, impl TileView>) {
    npc.no_tile_collide = false;
    if npc.collide_x {
        npc.direction *= -1;
        npc.velocity.0 = f32::from(npc.direction) * 2.0;
    }
    if npc.collide_y {
        npc.velocity.1 = if npc.old_velocity.1 > 0.0 { 1.0 } else { -1.0 };
    }
    const CRUISE: f32 = 4.5;
    let facing = f32::from(npc.direction);
    if npc.velocity.0.signum() != facing || npc.velocity.0.abs() < CRUISE {
        npc.velocity.0 += facing * 0.04;
        if npc.velocity.0 * facing < 0.0 {
            npc.velocity.0 += facing
                * if npc.velocity.0.abs() > CRUISE {
                    0.4
                } else {
                    0.2
                };
        } else if npc.velocity.0.abs() > CRUISE {
            npc.velocity.0 = facing * CRUISE;
        }
    }

    // It looks twenty tiles ahead and eight down: open air below means descend, anything close
    // below means climb, and something within five tiles means climb harder.
    let ahead = (cx_tile(npc) - if npc.direction < 0 { 20 } else { 0 }, 20);
    let below = ((npc.position.1 + npc.height()) / 16.0) as i32;
    let mut open = true;
    let mut close = false;
    'scan: for x in ahead.0..=(ahead.0 + ahead.1) {
        for y in below..below + 8 {
            let tile = world.tiles.tile(x, y);
            let blocked = (tile.is_active() && terrustia_proto::tile_solid::solid(tile.block))
                || tile.liquid > 0;
            if blocked {
                if y < below + 5 {
                    close = true;
                }
                open = false;
                break 'scan;
            }
        }
    }
    npc.velocity.1 += if open { 0.05 } else { -0.2 };
    if close {
        npc.velocity.1 -= 0.3;
    }
    npc.velocity.1 = npc.velocity.1.clamp(-5.0, 3.0);
}

/// Coming to you: it closes on a band around your middle and stops when it can touch you.
fn approach(npc: &mut Npc, world: &World<'_, impl TileView>, out: &mut FairyOutcome) {
    npc.no_tile_collide = true;
    let Some(t) = world.target else {
        return;
    };
    if !t.alive {
        give_up(npc, t);
        return;
    }
    let (cx, cy) = npc.center();
    let reach = reach_box(t);
    if overlaps(npc, reach) {
        out.wants_treasure = true;
        return;
    }
    let point = closest_in(reach, (cx, cy));
    let toward = unit((point.0 - cx, point.1 - cy));
    let gap = (point.0 - cx).hypot(point.1 - cy);
    let hurry = if gap > 150.0 {
        4.0
    } else if gap > 80.0 {
        3.0
    } else {
        2.0
    };
    let wanted = (toward.0 * hurry, toward.1 * hurry);
    npc.velocity.0 += (wanted.0 - npc.velocity.0) * 0.07;
    npc.velocity.1 += (wanted.1 - npc.velocity.1) * 0.07;

    // `ai[3]` counts how long it has been inside the world. Past five seconds it stops trying to
    // fly around terrain and simply cuts through, which is what keeps it from getting stuck.
    if npc.ai[3] < 300.0 {
        let (down, up) = flight_advice(npc, world, 6, 3);
        if down {
            npc.velocity.1 += 0.05;
        }
        if up {
            npc.velocity.1 -= 0.02;
        }
        npc.velocity.1 = npc.velocity.1.clamp(-4.0, 2.0);
    }
    npc.ai[3] = if buried(npc, world) {
        (npc.ai[3] + 2.0).min(400.0)
    } else {
        (npc.ai[3] - 1.0).max(0.0)
    };
}

/// The little dance it does when it has found something.
fn celebrate(npc: &mut Npc, world: &World<'_, impl TileView>) -> bool {
    npc.no_tile_collide = true;
    let dancing = npc.ai[3] > 15.0;
    if !dancing {
        npc.velocity.0 *= 0.9;
        npc.velocity.1 *= 0.9;
    } else {
        face_target(npc, world);
        // Two loops, tilted opposite ways, and then a straight one.
        let along = npc.ai[3] - 15.0;
        let (tilt, height) = if along <= 65.0 {
            (std::f32::consts::FRAC_PI_8, 14.0)
        } else if along <= 130.0 {
            (-std::f32::consts::FRAC_PI_8, 18.0)
        } else {
            (0.0, 22.0)
        };
        let tilt = tilt * f32::from(npc.direction);
        let here = circle(npc, along / 65.0, tilt, height);
        let next = circle(npc, along / 65.0 + 1.0 / 65.0, tilt, height);
        npc.velocity = (next.0 - here.0, next.1 - here.1);
    }
    npc.ai[3] += 1.0;
    if npc.ai[3] >= FAIRY_CELEBRATE {
        npc.ai[2] = state::LEAD;
        npc.ai[3] = 0.0;
    }
    dancing
}

/// Leading: it makes for the treasure, but will not get more than three hundred pixels ahead.
fn lead(npc: &mut Npc, world: &World<'_, impl TileView>) -> bool {
    npc.no_tile_collide = true;
    let Some(t) = world.target else {
        return false;
    };
    if !t.alive {
        give_up(npc, t);
        return false;
    }
    let (cx, cy) = npc.center();
    let spot = (npc.ai[0] * 16.0 + 8.0, npc.ai[1] * 16.0 + 8.0);
    if (cx - spot.0).abs() < npc.width() / 2.0 + 3.0
        && (cy - spot.1).abs() < npc.height() / 2.0 + 3.0
    {
        npc.ai[2] = state::ARRIVED;
        npc.ai[3] = 0.0;
        return false;
    }

    let behind = (spot.0 - cx).hypot(spot.1 - cy);
    let lag = (t.center.0 - cx).hypot(t.center.1 - cy);
    if lag > FAIRY_LEAD {
        // Too far ahead: it waits, drifting back toward you and hovering in place.
        face_target(npc, world);
        let away = unit((cx - t.center.0, cy - t.center.1));
        if lag > FAIRY_LEAD + 60.0 {
            npc.velocity.0 -= away.0 * 0.1;
            npc.velocity.1 -= away.1 * 0.1;
        } else if lag < FAIRY_LEAD + 30.0 {
            let on = unit((spot.0 - cx, spot.1 - cy));
            npc.velocity.0 += on.0 * 0.1;
            npc.velocity.1 += on.1 * 0.1;
        }
        let speed = npc.velocity.0.hypot(npc.velocity.1);
        if speed > 1.0 {
            npc.velocity.0 /= speed;
            npc.velocity.1 /= speed;
        }
        return true;
    }

    let toward = unit((spot.0 - cx, spot.1 - cy));
    let hurry = if behind > 150.0 {
        3.0
    } else if behind > 80.0 {
        2.0
    } else {
        1.0
    };
    let wanted = (toward.0 * hurry, toward.1 * hurry);
    npc.velocity.0 += (wanted.0 - npc.velocity.0) * 0.07;
    npc.velocity.1 += (wanted.1 - npc.velocity.1) * 0.07;
    if npc.ai[3] < 300.0 {
        let (down, up) = flight_advice(npc, world, 4, 2);
        if down {
            npc.velocity.1 += 0.05;
        }
        if up {
            npc.velocity.1 -= 0.05;
        }
        npc.velocity.1 = npc.velocity.1.clamp(-1.0, 1.0);
    }
    npc.ai[3] = if buried(npc, world) {
        (npc.ai[3] + 2.0).min(400.0)
    } else {
        (npc.ai[3] - 1.0).max(0.0)
    };
    false
}

/// Over the spot at last: a wobblier dance, and then it goes.
fn arrived(npc: &mut Npc, world: &World<'_, impl TileView>) -> bool {
    npc.local_ai[1] = 0.0;
    npc.no_tile_collide = true;
    let dancing = npc.ai[3] > 15.0;
    if !dancing {
        npc.velocity.0 *= 0.9;
        npc.velocity.1 *= 0.9;
    } else {
        let along = npc.ai[3] - 15.0;
        // The tilt and the height both wander with the loop count, so no two loops match.
        let loops = (along / 50.0) as i32 as f32;
        let tilt = loops.cos() * std::f32::consts::TAU / 16.0 * f32::from(npc.direction);
        let height = (loops * 2.0).cos() * 10.0 + 8.0;
        let here = circle(npc, along / 50.0, tilt, height);
        let next = circle(npc, along / 50.0 + 0.02, tilt, height);
        npc.velocity = (next.0 - here.0, next.1 - here.1);
        face_target(npc, world);
    }
    npc.ai[3] += 1.0;
    dancing
}

/// Nothing found: it stays by you and keeps looking, because the world moves under you.
fn linger(npc: &mut Npc, world: &World<'_, impl TileView>, out: &mut FairyOutcome) {
    npc.no_tile_collide = true;
    let Some(t) = world.target else {
        return;
    };
    let (cx, cy) = npc.center();
    let toward = (t.center.0 - cx, t.center.1 - cy);
    if toward.0.hypot(toward.1) > 100.0 {
        npc.ai[2] = state::APPROACH;
        npc.ai[3] = 0.0;
        return;
    }
    if !crate::game::ai::sight::solid_collision(
        world.tiles,
        npc.position,
        (npc.stats.width, npc.stats.height),
    ) {
        npc.no_tile_collide = false;
        if npc.collide_x {
            npc.velocity.0 *= -1.0;
        }
        if npc.collide_y {
            npc.velocity.1 *= -1.0;
        }
    }
    if toward.0.hypot(toward.1) > 20.0 {
        npc.velocity.0 += if toward.0 > 0.0 { 0.04 } else { -0.04 };
        npc.velocity.1 += if toward.1 > 0.0 { 0.04 } else { -0.04 };
        if npc.velocity.1.abs() > 2.0 {
            npc.velocity.1 *= 0.95;
        }
    }
    out.wants_treasure = true;
}

/// Give up on a dead player and go back to wandering.
fn give_up(npc: &mut Npc, t: Target) {
    let (cx, _) = npc.center();
    npc.ai[2] = state::WANDER;
    npc.direction = if t.center.0 > cx { -1 } else { 1 };
    if npc.velocity.0 * f32::from(npc.direction) < 0.0 {
        npc.velocity.0 = f32::from(npc.direction) * 2.0;
    }
    npc.ai[3] = 0.0;
}

/// Where a fairy is on its little loop, at a point in the loop.
fn circle(npc: &Npc, elapsed: f32, rotation: f32, height: f32) -> (f32, f32) {
    let angle = std::f32::consts::TAU * elapsed + std::f32::consts::FRAC_PI_2;
    let point = (
        (angle.cos()) * (6.0 * -f32::from(npc.direction)),
        (angle.sin() - 1.0) * height,
    );
    let (sin, cos) = rotation.sin_cos();
    (point.0 * cos - point.1 * sin, point.0 * sin + point.1 * cos)
}

/// Whether there is floor a few tiles ahead, and whether it is close enough to want climbing over.
fn flight_advice(
    npc: &Npc,
    world: &World<'_, impl TileView>,
    down_scan: i32,
    up_range: i32,
) -> (bool, bool) {
    let (cx, cy) = npc.center();
    let x = (cx / 16.0) as i32 + i32::from(npc.direction);
    let top = (cy / 16.0) as i32;
    for y in top..top + down_scan {
        let tile = world.tiles.tile(x, y);
        if (tile.is_active() && terrustia_proto::tile_solid::solid(tile.block)) || tile.liquid > 0 {
            return (false, y < top + up_range);
        }
    }
    (true, false)
}

fn buried(npc: &Npc, world: &World<'_, impl TileView>) -> bool {
    let (cx, cy) = npc.center();
    let tile = world.tiles.tile((cx / 16.0) as i32, (cy / 16.0) as i32);
    tile.is_active() && terrustia_proto::tile_solid::solid(tile.block)
}

fn face_target(npc: &mut Npc, world: &World<'_, impl TileView>) {
    if let Some(t) = world.target {
        npc.sprite_direction = if t.center.0 > npc.center().0 { -1 } else { 1 };
    }
}

fn cx_tile(npc: &Npc) -> i32 {
    ((npc.position.0 + npc.width() / 2.0) / 16.0) as i32
}

fn unit(v: (f32, f32)) -> (f32, f32) {
    let length = v.0.hypot(v.1).max(f32::MIN_POSITIVE);
    (v.0 / length, v.1 / length)
}

/// The band around a player's middle a fairy aims for: their width plus sixty, half their height.
fn reach_box(t: Target) -> (f32, f32, f32, f32) {
    let (w, h) = (
        crate::game::ai::PLAYER_WIDTH as f32 + 60.0,
        crate::game::ai::PLAYER_HEIGHT as f32 / 2.0,
    );
    (
        t.center.0 - w / 2.0,
        t.center.1 - h / 2.0,
        t.center.0 + w / 2.0,
        t.center.1 + h / 2.0,
    )
}

fn closest_in(r: (f32, f32, f32, f32), p: (f32, f32)) -> (f32, f32) {
    (p.0.clamp(r.0, r.2), p.1.clamp(r.1, r.3))
}

fn overlaps(npc: &Npc, r: (f32, f32, f32, f32)) -> bool {
    npc.position.0 < r.2
        && npc.position.0 + npc.width() > r.0
        && npc.position.1 < r.3
        && npc.position.1 + npc.height() > r.1
}

/// The best thing worth showing, within a hundred and fifty tiles across and a hundred down.
///
/// Ties on priority are broken by distance — but by *greatest* distance, not least, which is the
/// game's own oddity and makes a fairy walk you further than it strictly has to.
pub fn treasure(tiles: &impl TileView, from: (f32, f32), bounds: (i32, i32)) -> Option<(i32, i32)> {
    let at = ((from.0 / 16.0) as i32, (from.1 / 16.0) as i32);
    const EDGE: i32 = 40;
    let left = (at.0 - FAIRY_SEARCH_X).max(EDGE);
    let right = (at.0 + FAIRY_SEARCH_X).min(bounds.0 - EDGE);
    let top = (at.1 - FAIRY_SEARCH_Y).max(EDGE);
    let bottom = (at.1 + FAIRY_SEARCH_Y).min(bounds.1 - EDGE);

    let mut best: Option<(i16, f32, (i32, i32))> = None;
    for x in left..=right {
        for y in top..=bottom {
            let tile = tiles.tile(x, y);
            if !tile.is_active()
                || !fairy_lures_to(tile.block)
                || !valid_for_ore_finder(tile.block, tile.frame_x)
            {
                continue;
            }
            let mut priority = ore_finder_priority(tile.block);
            // A single stray block of ore is not worth a walk: it has to be a real vein.
            if is_ore(tile.block) {
                let mut seam = 0;
                for kx in x - 3..=x + 3 {
                    for ky in y - 3..=y + 3 {
                        let near = tiles.tile(kx, ky);
                        if near.is_active() && near.block == tile.block {
                            seam += 1;
                        }
                    }
                }
                if seam < FAIRY_VEIN {
                    priority = -1;
                }
            }
            if priority < 0 {
                continue;
            }
            let away = (x as f32 * 16.0 + 8.0 - from.0).hypot(y as f32 * 16.0 + 8.0 - from.1);
            let better = match best {
                None => true,
                Some((top_priority, _, _)) if priority > top_priority => true,
                Some((top_priority, far, _)) => priority == top_priority && away >= far,
            };
            if better {
                best = Some((priority, away, (x, y)));
            }
        }
    }
    best.map(|(_, _, at)| at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::npc_params::FAIRY_CRITTER_PINK;
    use terrustia_proto::tile::Tile;

    struct Cave(HashMap<(i32, i32), Tile>);

    impl TileView for Cave {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn world<'a>(tiles: &'a Cave, target: Option<(f32, f32)>) -> World<'a, Cave> {
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

    fn one() -> Npc {
        Npc::new(FAIRY_CRITTER_PINK, (1600.0, 1600.0), 1).expect("a fairy")
    }

    fn tick(
        npc: &mut Npc,
        w: &World<'_, Cave>,
        tiles: &Cave,
        found: Option<(i32, i32)>,
        rng: &mut SmallRng,
    ) -> FairyOutcome {
        let out = fairy(npc, w, found, rng);
        npc.no_gravity = true;
        crate::game::npc::step_physics(npc, tiles);
        out
    }

    /// It notices you at two hundred and fifty pixels and not before.
    #[test]
    fn it_notices_you_when_you_come_close() {
        let tiles = Cave(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(1);

        let far = world(&tiles, Some((1600.0 + 400.0, 1600.0)));
        let mut n = one();
        for _ in 0..200 {
            tick(&mut n, &far, &tiles, None, &mut rng);
        }
        assert_eq!(n.ai[2], state::IDLE, "four hundred pixels is too far");

        let near = world(&tiles, Some((1600.0 + 100.0, 1600.0)));
        let mut n = one();
        tick(&mut n, &near, &tiles, None, &mut rng);
        assert_eq!(n.ai[2], state::APPROACH, "a hundred is close enough");
    }

    /// Touched, it asks for a treasure; given one it celebrates, given none it waits by you.
    #[test]
    fn touching_you_sends_it_looking() {
        let tiles = Cave(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(2);
        let w = world(&tiles, Some((1610.0, 1610.0)));

        let mut found = one();
        found.ai[2] = state::APPROACH;
        for _ in 0..200 {
            tick(&mut found, &w, &tiles, Some((90, 120)), &mut rng);
            if found.ai[2] == state::CELEBRATE {
                break;
            }
        }
        assert_eq!(found.ai[2], state::CELEBRATE, "it should have set off");
        assert_eq!(
            (found.ai[0], found.ai[1]),
            (90.0, 120.0),
            "and remembered where"
        );

        let mut empty = one();
        empty.ai[2] = state::APPROACH;
        for _ in 0..200 {
            tick(&mut empty, &w, &tiles, None, &mut rng);
            if empty.ai[2] == state::LINGER {
                break;
            }
        }
        assert_eq!(empty.ai[2], state::LINGER, "nothing found: it waits");
    }

    /// The whole sequence runs: celebrate, lead, arrive, gone.
    #[test]
    fn it_leads_you_there_and_then_goes() {
        let tiles = Cave(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(3);
        let mut n = one();
        n.ai[2] = state::CELEBRATE;
        // The treasure a little way off, and a player who follows.
        let spot = (105, 100);
        let mut player = (1600.0, 1600.0);
        let mut seen = Vec::new();
        let mut gone = false;
        for _ in 0..4000 {
            let w = world(&tiles, Some(player));
            n.ai[0] = spot.0 as f32;
            n.ai[1] = spot.1 as f32;
            let out = tick(&mut n, &w, &tiles, Some(spot), &mut rng);
            if seen.last() != Some(&n.ai[2]) {
                seen.push(n.ai[2]);
            }
            if out.spent {
                gone = true;
                break;
            }
            // The player walks toward the fairy, slowly, the way one does.
            let (cx, cy) = n.center();
            player.0 += (cx - player.0).clamp(-3.0, 3.0);
            player.1 += (cy - player.1).clamp(-3.0, 3.0);
        }
        assert!(gone, "it should have finished: {seen:?}");
        assert_eq!(
            seen,
            vec![state::CELEBRATE, state::LEAD, state::ARRIVED],
            "in that order"
        );
    }

    /// Out of patience, it turns away and leaves.
    #[test]
    fn it_gives_you_five_minutes() {
        let tiles = Cave(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(4);
        let w = world(&tiles, Some((1610.0, 1610.0)));
        let mut n = one();
        n.ai[2] = state::LINGER;
        n.local_ai[1] = FAIRY_PATIENCE - 2.0;
        for _ in 0..5 {
            tick(&mut n, &w, &tiles, None, &mut rng);
        }
        assert_eq!(n.ai[2], state::LEAVING);
        assert!(n.velocity.1 < 0.0, "and climbing away");
        assert!(n.time_left <= 10, "and no longer sticking around");
    }

    /// It can only be hurt while it is still drifting: once it has taken you on, it is safe.
    #[test]
    fn a_committed_fairy_cannot_be_hurt() {
        let tiles = Cave(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(5);
        let w = world(&tiles, Some((1610.0, 1610.0)));
        let mut n = one();
        tick(&mut n, &w, &tiles, None, &mut rng);
        assert!(!n.invulnerable, "an idle one is fair game");
        n.ai[2] = state::LEAD;
        tick(&mut n, &w, &tiles, None, &mut rng);
        assert!(n.invulnerable, "a leading one is not");
    }

    /// It picks the best thing on the ranking, not the nearest.
    #[test]
    fn it_leads_to_the_best_thing_not_the_closest() {
        let mut cave = HashMap::new();
        // A chest (500) two tiles away, and a life crystal (12, 550) forty tiles away.
        cave.insert((102, 100), Tile::framed(21, 0, 0));
        cave.insert((140, 100), Tile::framed(12, 0, 0));
        let tiles = Cave(cave);
        let found = treasure(&tiles, (100.0 * 16.0, 100.0 * 16.0), (4200, 1200));
        assert_eq!(found, Some((140, 100)), "the crystal outranks the chest");
    }

    /// A single stray block of ore is not a vein, and it will not walk you to one.
    #[test]
    fn one_block_of_ore_is_not_a_vein() {
        let mut cave = HashMap::new();
        cave.insert((110, 100), Tile::block(107));
        let tiles = Cave(cave);
        assert_eq!(
            treasure(&tiles, (100.0 * 16.0, 100.0 * 16.0), (4200, 1200)),
            None,
            "one cobalt block is not worth the trip"
        );

        // A proper seam is.
        let mut cave = HashMap::new();
        for x in 108..115 {
            for y in 98..105 {
                cave.insert((x, y), Tile::block(107));
            }
        }
        let tiles = Cave(cave);
        assert!(
            treasure(&tiles, (100.0 * 16.0, 100.0 * 16.0), (4200, 1200)).is_some(),
            "forty-nine blocks is a vein"
        );
    }

    /// It ignores what the ore finder ranks but a fairy does not care about.
    #[test]
    fn it_ignores_what_it_does_not_lure_to() {
        let mut cave = HashMap::new();
        // Copper ore: ranked at 200, but not on the fairy's list.
        for x in 105..112 {
            for y in 98..105 {
                cave.insert((x, y), Tile::block(7));
            }
        }
        let tiles = Cave(cave);
        assert_eq!(
            treasure(&tiles, (100.0 * 16.0, 100.0 * 16.0), (4200, 1200)),
            None
        );
    }
}
