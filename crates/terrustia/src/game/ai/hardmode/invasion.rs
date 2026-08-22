//! The two invasion fliers: styles 80 and 93.
//!
//! * A **probe** (80) is the only enemy in the game whose goal is to *leave*. It cruises along
//!   holding a fixed height over the ground, and the moment it sees a player below it, it stops
//!   dead for a second and then climbs out of the world — and reaching the top is what starts the
//!   invasion. Killing it before it gets there is the whole interaction.
//! * The **Flying Dutchman** (93) is a hull with four cannon bolted on. Killing the hull is not a
//!   thing you do: its death condition is that its cannon are all gone. It holds station a few
//!   hundred pixels above whatever is below it, closes only when you are far off, and drops
//!   pirates as it goes.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::{
    npc_params::{
        DUTCHMAN_ACCEL, DUTCHMAN_CANNON, DUTCHMAN_CANNON_OFFSET, DUTCHMAN_CANNON_SPACING,
        DUTCHMAN_DROP_CHANCE, DUTCHMAN_DROP_RISE, DUTCHMAN_DROPS, DUTCHMAN_GROUND_SCAN,
        DUTCHMAN_GUN, DUTCHMAN_HOVER, DUTCHMAN_HOVER_EASE, DUTCHMAN_HOVER_RATE,
        DUTCHMAN_HOVER_SLACK, DUTCHMAN_SPEED, DUTCHMAN_STANDOFF, PROBE_ALERT_TICKS, PROBE_CLIMB,
        PROBE_CLIMB_CAP, PROBE_COMFORTABLE, PROBE_CRUISE, PROBE_ESCAPE_CLIMB,
        PROBE_ESCAPE_CLIMB_CAP, PROBE_ESCAPE_DRIFT, PROBE_ESCAPE_DRIFT_CAP, PROBE_ESCAPE_TICKS,
        PROBE_SCAN, PROBE_SIGHT, PROBE_SINK_CAP, PROBE_TOO_LOW,
    },
    tile_solid::{solid, solid_top},
};

use crate::game::ai::{World, face};
use crate::game::npc::{Npc, TILE, TileView};
use crate::game::npc_ai::Spawn;

/// What a Martian concluded this tick.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Anything it put into the world.
    pub spawn: Vec<Spawn>,
    /// Set when it is finished — for a probe, because it got away.
    pub spent: bool,
    /// Set when leaving means the invasion begins.
    pub called_the_invasion: bool,
}

/// How many tiles of clear air are below `(tile_x, tile_y)`, up to `limit`.
///
/// Returns `limit` when nothing solid turns up, which is what makes an NPC over a chasm behave as
/// though it were very high rather than as though the ground were missing.
fn drop_below(tiles: &impl TileView, tile_x: i32, tile_y: i32, limit: i32) -> i32 {
    for depth in 0..limit {
        let tile = tiles.tile(tile_x, tile_y + depth);
        if tile.is_active() && solid(tile.block) && !solid_top(tile.block) {
            return depth;
        }
    }
    limit
}

/// Style 80: the probe that starts the invasion by escaping with what it saw.
pub fn probe(npc: &mut Npc, world: &World<'_, impl TileView>) -> Outcome {
    let mut out = Outcome::default();
    npc.dirty = true;

    match npc.ai[0] {
        0.0 => {
            if npc.direction == 0
                && let Some(target) = world.target
            {
                face(npc, target);
            }
            // Bouncing off a wall is how a probe turns; it never chooses to.
            if npc.collide_x {
                npc.direction = -npc.direction;
            }
            npc.velocity.0 = PROBE_CRUISE * f32::from(npc.direction);

            let (cx, cy) = npc.center();
            let depth = drop_below(
                world.tiles,
                (cx / TILE) as i32,
                (cy / TILE) as i32,
                PROBE_SCAN,
            );
            if depth < PROBE_TOO_LOW {
                npc.velocity.1 = (npc.velocity.1 - PROBE_CLIMB).max(-PROBE_CLIMB_CAP);
            } else if depth < PROBE_COMFORTABLE {
                npc.velocity.1 *= 0.95;
            } else {
                npc.velocity.1 = (npc.velocity.1 + PROBE_CLIMB).min(PROBE_SINK_CAP);
            }

            // It only counts a player it is *above*: flying under one does not trip it.
            if let Some(target) = world.target.filter(|t| t.alive) {
                let (dx, dy) = (target.center.0 - cx, target.center.1 - cy);
                if dx.hypot(dy) < PROBE_SIGHT && target.center.1 > cy {
                    npc.ai[0] = 1.0;
                    npc.ai[1] = 0.0;
                }
            }
        }
        // Seen you. It hangs still while it makes up its mind, then picks a direction away.
        1.0 => {
            npc.ai[1] += 1.0;
            npc.velocity.0 *= 0.95;
            npc.velocity.1 *= 0.95;
            if npc.ai[1] >= PROBE_ALERT_TICKS {
                npc.ai[1] = 0.0;
                npc.ai[0] = 2.0;
                npc.ai[3] = match world.target {
                    Some(t) if t.center.0 > npc.center().0 => -1.0,
                    _ => 1.0,
                };
            }
        }
        // Climbing out. Nothing stops it now except being killed.
        _ => {
            npc.no_tile_collide = true;
            npc.ai[1] += 1.0;
            npc.velocity.1 = (npc.velocity.1 - PROBE_ESCAPE_CLIMB).max(-PROBE_ESCAPE_CLIMB_CAP);
            npc.velocity.0 =
                (npc.velocity.0 + npc.ai[3] * PROBE_ESCAPE_DRIFT).min(PROBE_ESCAPE_DRIFT_CAP);
            if npc.position.1 < -npc.height() || npc.ai[1] >= PROBE_ESCAPE_TICKS {
                out.spent = true;
                out.called_the_invasion = true;
            }
        }
    }
    out
}

/// Where the Dutchman's cannon start out, relative to its centre.
pub fn cannon_stations(npc: &Npc) -> Vec<(f32, f32)> {
    let (cx, cy) = npc.center();
    (0..DUTCHMAN_CANNON)
        .map(|i| {
            (
                cx + i as f32 * DUTCHMAN_CANNON_SPACING - DUTCHMAN_CANNON_OFFSET,
                cy,
            )
        })
        .collect()
}

/// Style 93: the Flying Dutchman's hull.
///
/// `cannon_alive` is how many of the four it launched with are still up; the hull gives out when
/// that reaches zero, which is why shooting the guns off is the way to sink one.
pub fn dutchman(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    rng: &mut SmallRng,
    cannon_alive: usize,
) -> Outcome {
    let mut out = Outcome::default();
    npc.dirty = true;

    // First tick: bolt the guns on.
    if npc.local_ai[0] == 0.0 {
        npc.local_ai[0] = 1.0;
        for at in cannon_stations(npc) {
            out.spawn.push(Spawn {
                npc_type: DUTCHMAN_GUN,
                position: at,
                velocity: (0.0, 0.0),
                parent: Some(Spawn::OWN_PARENT),
            });
        }
        return out;
    }
    if cannon_alive == 0 {
        out.spent = true;
        return out;
    }

    // Now and then it drops a soldier, thrown upward so it arcs clear of the hull.
    if rng.random_range(0..DUTCHMAN_DROP_CHANCE) == 0 {
        let (cx, cy) = npc.center();
        let across = (rng.random::<f32>() - 0.5) * (npc.width() - 70.0);
        let above = (rng.random::<f32>() - 0.5) * 20.0 - npc.height() / 2.0 - 20.0;
        out.spawn.push(Spawn {
            npc_type: DUTCHMAN_DROPS[rng.random_range(0..DUTCHMAN_DROPS.len())],
            position: (cx + across, cy + above),
            velocity: (
                (rng.random::<f32>() - 0.5) * 5.0 + npc.velocity.0,
                DUTCHMAN_DROP_RISE + npc.velocity.1,
            ),
            parent: None,
        });
    }

    if let Some(target) = world.target {
        face(npc, target);
    }

    // Hold station over whatever is underneath, however far down that turns out to be.
    let (cx, _) = npc.center();
    let ahead = (cx / TILE) as i32 + npc.velocity.0.signum() as i32 * 10;
    let feet = ((npc.position.1 + npc.height()) / TILE) as i32;
    let below = drop_below(world.tiles, ahead, feet, DUTCHMAN_GROUND_SCAN) as f32 * TILE;

    if below < DUTCHMAN_HOVER {
        // Too low: climb, and the closer to the floor the harder.
        let wanted = (below - DUTCHMAN_HOVER).max(-DUTCHMAN_HOVER_RATE);
        npc.velocity.1 += (wanted - npc.velocity.1) * DUTCHMAN_HOVER_EASE;
    } else if below > DUTCHMAN_HOVER + DUTCHMAN_HOVER_SLACK {
        let wanted = (below - DUTCHMAN_HOVER).min(DUTCHMAN_HOVER_RATE);
        npc.velocity.1 += (wanted - npc.velocity.1) * DUTCHMAN_HOVER_EASE;
    } else {
        npc.velocity.1 *= 0.95;
    }

    // It closes only from a distance, and stops pushing once it is up to speed the right way.
    if let Some(target) = world.target {
        let across = target.center.0 - npc.center().0;
        if across.abs() >= DUTCHMAN_STANDOFF
            && (npc.velocity.0.abs() < DUTCHMAN_SPEED
                || npc.velocity.0.signum() as i8 != npc.direction)
        {
            npc.velocity.0 += f32::from(npc.direction) * DUTCHMAN_ACCEL;
        }
    }
    npc.rotation = npc.velocity.0 * 0.025;
    npc.sprite_direction = -(npc.velocity.0.signum() as i8);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Sky(HashMap<(i32, i32), Tile>);

    impl TileView for Sky {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    /// Ground at `floor`, stone, across the whole width these tests use.
    fn ground(floor: i32) -> Sky {
        let mut tiles = HashMap::new();
        for x in -400..400 {
            for y in floor..floor + 4 {
                tiles.insert((x, y), Tile::block(1));
            }
        }
        Sky(tiles)
    }

    fn world<'a>(tiles: &'a Sky, target: Option<(f32, f32)>) -> World<'a, Sky> {
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

    const MARTIAN_PROBE: u16 = 399;
    const FLYING_DUTCHMAN: u16 = 491;

    fn probe_at(x: f32, y: f32) -> Npc {
        Npc::new(MARTIAN_PROBE, (x, y), 1).expect("martian probe")
    }

    /// A probe cruising over flat ground stays over it rather than drifting into the sky or the
    /// floor: that is the whole of its patrol.
    #[test]
    fn a_probe_holds_its_height() {
        let floor = 60;
        let tiles = ground(floor);
        let mut p = probe_at(0.0, (floor - 25) as f32 * TILE);
        let w = world(&tiles, None);
        let mut lowest: f32 = f32::MIN;
        let mut highest: f32 = f32::MAX;
        for _ in 0..600 {
            probe(&mut p, &w);
            crate::game::npc::step_physics(&mut p, &tiles);
            let gap = (floor as f32 * TILE) - (p.position.1 + p.height());
            lowest = lowest.max(gap);
            highest = highest.min(gap);
        }
        // Between the two bands it steers for, with a little overshoot either way.
        assert!(
            (PROBE_TOO_LOW as f32 * TILE - 64.0..=PROBE_SCAN as f32 * TILE + 64.0)
                .contains(&highest),
            "dropped to {highest} above the floor"
        );
        assert!(highest > 0.0, "it flew into the ground");
    }

    /// Seeing a player below is what sends a probe home, and getting home is what starts the
    /// invasion.
    #[test]
    fn a_probe_that_sees_you_leaves_and_calls_it_in() {
        let tiles = ground(60);
        let mut p = probe_at(0.0, 40.0 * TILE);
        let (cx, cy) = p.center();
        let w = world(&tiles, Some((cx, cy + 200.0)));

        probe(&mut p, &w);
        assert_eq!(p.ai[0], 1.0, "it should have spotted the player below");

        let mut called = false;
        for _ in 0..1000 {
            let out = probe(&mut p, &w);
            crate::game::npc::step_physics(&mut p, &tiles);
            if out.spent {
                called = out.called_the_invasion;
                break;
            }
        }
        assert!(called, "escaping is what starts the invasion");
        assert!(p.velocity.1 < 0.0, "and it leaves upward");
    }

    /// A player *above* a probe is not something it can report.
    #[test]
    fn a_probe_ignores_a_player_it_is_not_above() {
        let tiles = ground(60);
        let mut p = probe_at(0.0, 40.0 * TILE);
        let (cx, cy) = p.center();
        let w = world(&tiles, Some((cx, cy - 200.0)));
        for _ in 0..60 {
            probe(&mut p, &w);
        }
        assert_eq!(p.ai[0], 0.0, "it should still be patrolling");
    }

    /// The hull launches four turrets and then depends on them entirely.
    #[test]
    fn the_dutchman_is_its_cannon() {
        let tiles = ground(120);
        let w = world(&tiles, Some((500.0, 500.0)));
        let mut rng = SmallRng::seed_from_u64(93);
        let mut s = Npc::new(FLYING_DUTCHMAN, (0.0, 40.0 * TILE), 1).unwrap();

        let first = dutchman(&mut s, &w, &mut rng, 0);
        assert_eq!(first.spawn.len(), DUTCHMAN_CANNON, "four guns");
        assert!(
            first.spawn.iter().all(|s| s.npc_type == DUTCHMAN_GUN),
            "and they should all be turrets"
        );
        assert!(!first.spent, "it does not die on the tick it arms itself");

        // Guns still up: it flies.
        assert!(!dutchman(&mut s, &w, &mut rng, 4).spent);
        // Guns gone: it does not.
        assert!(
            dutchman(&mut s, &w, &mut rng, 0).spent,
            "no guns, no saucer"
        );
    }

    /// The Dutchman keeps its distance rather than flying into you.
    #[test]
    fn the_dutchman_closes_only_from_a_distance() {
        let tiles = ground(120);
        let mut rng = SmallRng::seed_from_u64(1);
        let mut s = Npc::new(FLYING_DUTCHMAN, (0.0, 40.0 * TILE), 1).unwrap();
        s.local_ai[0] = 1.0;

        let (cx, cy) = s.center();
        let near = world(&tiles, Some((cx + 50.0, cy)));
        let before = s.velocity.0;
        dutchman(&mut s, &near, &mut rng, 4);
        assert_eq!(
            s.velocity.0, before,
            "already on top of you, no need to push"
        );

        let far = world(&tiles, Some((cx + 900.0, cy)));
        dutchman(&mut s, &far, &mut rng, 4);
        assert!(s.velocity.0 > before, "from far off it should close");
    }
}
