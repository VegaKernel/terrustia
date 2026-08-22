//! Style 76: the Martian saucer's core.
//!
//! The saucer is five NPCs: a core that flies, a body drawn over it, two turrets and two cannons.
//! Only the core has a routine — everything else rides it on style 75 — so this is the whole
//! machine's behaviour in one place.
//!
//! Intact, it runs a ten-second circuit and then repeats: swing out six hundred pixels to one
//! side, strafe across you, close to three hundred, hold, come directly overhead, and hang there.
//! The circuit is a loop rather than a reaction, so a saucer is something you learn the timing of.
//!
//! Once its guns are gone it spins for two and a half seconds and comes back doing one thing
//! only: alternating a hover with a strafe, and firing a deathray at the start of every strafe.
//! That last phase runs a full minute and then starts over, so a stripped saucer is more
//! dangerous than a whole one.

use terrustia_proto::npc_params::{
    MARTIAN_SAUCER_BODY, MARTIAN_SAUCER_CANNON, MARTIAN_SAUCER_TURRET, SAUCER_BEAT, SAUCER_CLOSE,
    SAUCER_CYCLE, SAUCER_DEATHRAY, SAUCER_DEATHRAY_DAMAGE, SAUCER_GIVE_UP, SAUCER_HALF_BEAT,
    SAUCER_HIGH, SAUCER_LAST_STAND, SAUCER_LOW, SAUCER_PART_OUT, SAUCER_PHASES, SAUCER_SPIN,
    SAUCER_WIDE,
};

use crate::game::ai::{Shot, World};
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Spawn;

/// What the core is doing, as `ai[0]` numbers it.
mod state {
    /// Leaving, having lost its target. The negatives remember which state to come back to.
    pub const LEAVING_WHOLE: f32 = -1.0;
    pub const LEAVING_STRIPPED: f32 = -2.0;
    /// The circuit.
    pub const CIRCUIT: f32 = 0.0;
    /// Its guns are gone: the spin.
    pub const SPINNING: f32 = 1.0;
    /// Stripped, and dangerous.
    pub const LAST_STAND: f32 = 2.0;
    /// Finished.
    pub const DONE: f32 = 3.0;
}

/// What it did this tick.
#[derive(Debug, Default)]
pub struct SaucerOutcome {
    pub spawn: Vec<Spawn>,
    pub shots: Vec<Shot>,
    pub spent: bool,
}

pub fn core(npc: &mut Npc, world: &World<'_, impl TileView>) -> SaucerOutcome {
    let mut out = SaucerOutcome::default();
    npc.dirty = true;

    // It assembles itself on its first tick: two turrets, two cannons, and a body over the top.
    if npc.local_ai[3] == 0.0 {
        npc.local_ai[3] = 1.0;
        let (cx, cy) = npc.center();
        for npc_type in [MARTIAN_SAUCER_TURRET, MARTIAN_SAUCER_CANNON] {
            for side in 0..2 {
                out.spawn.push(Spawn {
                    npc_type,
                    position: (
                        cx + side as f32 * SAUCER_PART_OUT * 2.0 - SAUCER_PART_OUT,
                        cy,
                    ),
                    velocity: (0.0, 0.0),
                    parent: Some(Spawn::OWN_PARENT),
                });
            }
        }
        out.spawn.push(Spawn {
            npc_type: MARTIAN_SAUCER_BODY,
            position: (cx, cy),
            velocity: (0.0, 0.0),
            parent: Some(Spawn::OWN_PARENT),
        });
    }

    if npc.ai[0] == state::DONE {
        out.spent = true;
        return out;
    }

    let (cx, cy) = npc.center();
    let lost = match world.target {
        Some(t) if t.alive => (t.center.0 - cx).hypot(t.center.1 - cy) > SAUCER_GIVE_UP,
        _ => true,
    };
    if lost && npc.ai[0] != state::SPINNING {
        if npc.ai[0] == state::CIRCUIT {
            npc.ai[0] = state::LEAVING_WHOLE;
        }
        if npc.ai[0] == state::LAST_STAND {
            npc.ai[0] = state::LEAVING_STRIPPED;
        }
    }

    if npc.ai[0] == state::LEAVING_WHOLE || npc.ai[0] == state::LEAVING_STRIPPED {
        // Climbing away — but it will turn round the moment someone is worth turning round for.
        npc.velocity.1 -= 0.4;
        npc.time_left = npc.time_left.min(10);
        if !lost {
            npc.time_left = 300;
            npc.ai[0] = if npc.ai[0] == state::LEAVING_STRIPPED {
                state::LAST_STAND
            } else {
                state::CIRCUIT
            };
            npc.ai[1] = 0.0;
            npc.ai[2] = 0.0;
            npc.ai[3] = 0.0;
        }
        return out;
    }

    let Some(target) = world.target.filter(|t| t.alive) else {
        return out;
    };

    match npc.ai[0] {
        state::CIRCUIT => circuit(npc, world, target.center),
        state::SPINNING => {
            // Two and a half seconds of tumbling, tightening as it goes, and then it comes back.
            npc.invulnerable = false;
            npc.velocity.0 *= 0.96;
            npc.velocity.1 *= 0.96;
            npc.ai[1] += 1.0;
            npc.rotation = spin(npc.ai[1]);
            if npc.ai[1] >= SAUCER_SPIN {
                npc.ai[0] = state::LAST_STAND;
                npc.ai[1] = 0.0;
                npc.rotation = 0.0;
            }
        }
        state::LAST_STAND => last_stand(npc, world, target.center, &mut out),
        _ => {}
    }

    // Anything that has flown clean out of the world is gone.
    if npc.position.1 < -100.0 || npc.position.0 < -100.0 {
        out.spent = true;
    }
    out
}

/// The intact circuit: six phases on a ten-second loop.
fn circuit(npc: &mut Npc, world: &World<'_, impl TileView>, player: (f32, f32)) {
    let (cx, cy) = npc.center();
    let was = phase_at(npc.ai[3]);
    npc.ai[3] += 1.0;
    if npc.ai[3] >= SAUCER_CYCLE {
        npc.ai[3] = 0.0;
    }
    let now = phase_at(npc.ai[3]);

    // `ai[2]` is which side it is working from, and it is picked once, when a phase begins.
    if now != was {
        match now {
            0 | 2 => npc.ai[2] = 0.0,
            1 => npc.ai[2] = if player.0 > cx { 1.0 } else { -1.0 },
            _ => {}
        }
    }

    match now {
        0 => {
            // Swing wide, to whichever side it is already on.
            if npc.ai[2] == 0.0 {
                npc.ai[2] = -SAUCER_WIDE * (cx - player.0).signum();
            }
            let to = (player.0 + npc.ai[2] - cx, player.1 - SAUCER_HIGH - cy);
            if to.0.hypot(to.1) < 50.0 {
                // Arrived early: skip straight to the strafe rather than hanging about.
                npc.ai[3] = 19.0;
            } else {
                glide(npc, to, 16.0);
            }
        }
        1 => {
            // Strafe across, holding two hundred and fifty pixels of air below.
            hold_altitude(npc, world, SAUCER_HIGH);
            npc.velocity.0 = 3.5 * npc.ai[2];
        }
        2 => {
            if npc.ai[2] == 0.0 {
                npc.ai[2] = SAUCER_CLOSE * (cx - player.0).signum();
            }
            let mut to = (player.0 + npc.ai[2] - cx, player.1 - SAUCER_LOW - cy);
            // Ground closer than it wants pushes the whole approach upward.
            let floor = ground_gap(npc, world);
            if floor < SAUCER_LOW {
                to.1 -= SAUCER_LOW - floor;
            }
            if to.0.hypot(to.1) < 70.0 {
                npc.ai[3] = 279.0;
            } else {
                glide(npc, to, 20.0);
            }
        }
        3 => {
            // Hold: it sheds speed and keeps its height, and sheds it faster than it strafes.
            hold_altitude_at(npc, world, SAUCER_LOW, 0.85);
            npc.velocity.0 *= 0.85;
        }
        4 => {
            let to = (player.0 - cx, player.1 - SAUCER_HIGH - cy);
            if to.0.hypot(to.1) < 50.0 {
                npc.ai[3] = 439.0;
            } else {
                glide(npc, to, 16.0);
            }
        }
        _ => {
            npc.velocity.0 *= 0.85;
            npc.velocity.1 *= 0.85;
        }
    }
}

/// Stripped: hover, strafe, hover, strafe, and a deathray at the start of each strafe.
fn last_stand(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    player: (f32, f32),
    out: &mut SaucerOutcome,
) {
    let (cx, _) = npc.center();
    let strafing = |at: f32| at % SAUCER_BEAT >= SAUCER_HALF_BEAT;
    let was = strafing(npc.ai[3]);
    npc.ai[3] += 1.0;
    let now = strafing(npc.ai[3]);

    if now != was && now {
        // The ray goes off as the strafe begins, aimed along the run rather than at you.
        npc.ai[2] = if player.0 > cx { 1.0 } else { -1.0 };
        out.shots.push(Shot {
            projectile: SAUCER_DEATHRAY,
            damage: SAUCER_DEATHRAY_DAMAGE,
            position: npc.center(),
            velocity: (0.0, 0.0),
            time_left: SAUCER_HALF_BEAT as u16,
        });
    }

    if npc.ai[3] >= SAUCER_LAST_STAND {
        // A full minute, and then the whole thing again from the top.
        npc.ai[1] = 0.0;
        npc.ai[2] = 0.0;
        npc.ai[3] = 0.0;
        return;
    }

    if now {
        hold_altitude(npc, world, SAUCER_HIGH);
        npc.velocity.0 = 8.0 * npc.ai[2];
    } else {
        let (cx, cy) = npc.center();
        let to = (
            player.0 + npc.ai[2] * 350.0 - cx,
            player.1 - SAUCER_HIGH - cy,
        );
        glide(npc, to, 16.0);
    }
    npc.rotation = 0.0;
}

/// Ease toward a direction at a speed. It never turns sharply; a tenth of the difference a tick.
fn glide(npc: &mut Npc, toward: (f32, f32), speed: f32) {
    let length = toward.0.hypot(toward.1).max(f32::MIN_POSITIVE);
    let wanted = (toward.0 / length * speed, toward.1 / length * speed);
    npc.velocity.0 += (wanted.0 - npc.velocity.0) * 0.1;
    npc.velocity.1 += (wanted.1 - npc.velocity.1) * 0.1;
}

/// Climb when the ground is closer than it wants; otherwise coast.
fn hold_altitude(npc: &mut Npc, world: &World<'_, impl TileView>, wanted: f32) {
    hold_altitude_at(npc, world, wanted, 0.95);
}

/// The same, with the decay the phase actually uses when it is not climbing.
fn hold_altitude_at(npc: &mut Npc, world: &World<'_, impl TileView>, wanted: f32, decay: f32) {
    let gap = ground_gap(npc, world);
    if gap < wanted {
        // It climbs at four pixels a tick, unless it is nearly on the floor, in which case it
        // climbs only as fast as it is deep — which is what keeps it from bouncing off the ground.
        let climb = (-4.0f32).max(-gap);
        npc.velocity.1 += (climb - npc.velocity.1) * 0.05;
    } else {
        npc.velocity.1 *= decay;
    }
}

/// How far below the saucer the ground is, in pixels, looking up to a hundred and fifty tiles.
fn ground_gap(npc: &Npc, world: &World<'_, impl TileView>) -> f32 {
    let x = (npc.center().0 / 16.0) as i32;
    let from = ((npc.position.1 + npc.height()) / 16.0) as i32;
    let solid = |y: i32| {
        let tile = world.tiles.tile(x, y);
        tile.is_active()
            && terrustia_proto::tile_solid::solid(tile.block)
            && !terrustia_proto::tile_solid::solid_top(tile.block)
    };
    if solid(from) {
        return 16.0;
    }
    let mut down = 0;
    while down < 150 {
        if solid(from + down) {
            down -= 1;
            break;
        }
        down += 1;
    }
    down as f32 * 16.0
}

/// The tumble: three tightening wobbles and then a real spin.
fn spin(at: f32) -> f32 {
    if at < 40.0 {
        (at / 40.0 * std::f32::consts::TAU).cos() * 0.2
    } else if at < 80.0 {
        (at / 20.0 * std::f32::consts::TAU).cos() * 0.3
    } else if at < 120.0 {
        (at / 10.0 * std::f32::consts::TAU).cos() * 0.4
    } else {
        (at - 120.0) / 30.0 * std::f32::consts::TAU
    }
}

/// Which phase of the circuit a point in the cycle falls in.
fn phase_at(at: f32) -> u8 {
    SAUCER_PHASES
        .iter()
        .find(|(from, _)| at >= *from)
        .map_or(0, |(_, phase)| *phase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::npc_params::MARTIAN_SAUCER_CORE;
    use terrustia_proto::tile::Tile;

    struct Sky(HashMap<(i32, i32), Tile>);

    impl TileView for Sky {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn ground() -> Sky {
        let mut tiles = HashMap::new();
        for x in 0..400 {
            for y in 200..210 {
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

    fn saucer() -> Npc {
        Npc::new(MARTIAN_SAUCER_CORE, (2000.0, 3000.0), 1).expect("a saucer core")
    }

    fn tick(npc: &mut Npc, w: &World<'_, Sky>, tiles: &Sky) -> SaucerOutcome {
        let out = core(npc, w);
        npc.no_gravity = true;
        npc.no_tile_collide = true;
        crate::game::npc::step_physics(npc, tiles);
        out
    }

    /// It builds itself out of five pieces, once.
    #[test]
    fn it_assembles_itself() {
        let tiles = ground();
        let w = world(&tiles, Some((2000.0, 3100.0)));
        let mut n = saucer();
        let mut parts = Vec::new();
        for _ in 0..600 {
            parts.extend(tick(&mut n, &w, &tiles).spawn);
        }
        assert_eq!(parts.len(), 5, "a body, two turrets and two cannons");
        let count = |ty| parts.iter().filter(|p| p.npc_type == ty).count();
        assert_eq!(count(MARTIAN_SAUCER_TURRET), 2);
        assert_eq!(count(MARTIAN_SAUCER_CANNON), 2);
        assert_eq!(count(MARTIAN_SAUCER_BODY), 1);
        // The guns go out to both sides, not on top of each other.
        let xs: Vec<f32> = parts
            .iter()
            .filter(|p| p.npc_type == MARTIAN_SAUCER_TURRET)
            .map(|p| p.position.0)
            .collect();
        assert!(
            (xs[0] - xs[1]).abs() > SAUCER_PART_OUT,
            "one either side: {xs:?}"
        );
    }

    /// The circuit really is a circuit: it visits every phase and comes back round.
    #[test]
    fn the_circuit_runs_all_six_phases() {
        let tiles = ground();
        let w = world(&tiles, Some((2000.0, 3100.0)));
        let mut n = saucer();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..(SAUCER_CYCLE as i32 * 3) {
            tick(&mut n, &w, &tiles);
            seen.insert(phase_at(n.ai[3]));
        }
        assert_eq!(seen.len(), 6, "all six: {seen:?}");
    }

    /// It only fires once its guns are gone, and then on the beat.
    #[test]
    fn only_a_stripped_saucer_fires() {
        let tiles = ground();
        let w = world(&tiles, Some((2000.0, 3100.0)));

        let mut whole = saucer();
        let mut shots = 0;
        for _ in 0..2000 {
            shots += tick(&mut whole, &w, &tiles).shots.len();
        }
        assert_eq!(shots, 0, "an intact saucer has no ray");

        let mut stripped = saucer();
        stripped.ai[0] = state::LAST_STAND;
        let mut fired = Vec::new();
        for at in 0..1200 {
            if !tick(&mut stripped, &w, &tiles).shots.is_empty() {
                fired.push(at);
            }
        }
        assert!(fired.len() >= 9, "one every two seconds: {fired:?}");
        for pair in fired.windows(2) {
            assert_eq!(
                pair[1] - pair[0],
                SAUCER_BEAT as i32,
                "the rays keep the beat"
            );
        }
    }

    /// Losing its guns puts it through the spin before the last phase, not straight into it.
    #[test]
    fn it_spins_before_the_last_stand() {
        let tiles = ground();
        let w = world(&tiles, Some((2000.0, 3100.0)));
        let mut n = saucer();
        n.ai[0] = state::SPINNING;
        let mut rotations = Vec::new();
        let mut arrived = None;
        for at in 0..400 {
            tick(&mut n, &w, &tiles);
            rotations.push(n.rotation);
            if n.ai[0] == state::LAST_STAND {
                arrived = Some(at);
                break;
            }
        }
        assert_eq!(
            arrived,
            Some(SAUCER_SPIN as i32 - 1),
            "two and a half seconds"
        );
        let widest = rotations
            .iter()
            .cloned()
            .fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(widest > 1.0, "and it really tumbles: {widest}");
    }

    /// It will not fly into the ground: strafing over a floor it keeps its height.
    #[test]
    fn it_keeps_its_height_over_the_ground() {
        let tiles = ground();
        // The player standing on the floor, so the saucer is asked to come low.
        let w = world(&tiles, Some((2000.0, 199.0 * 16.0)));
        let mut n = saucer();
        n.position = (2000.0, 195.0 * 16.0);
        let floor = 200.0 * 16.0;
        let mut lowest = f32::MIN;
        for _ in 0..(SAUCER_CYCLE as i32 * 2) {
            tick(&mut n, &w, &tiles);
            lowest = lowest.max(n.position.1 + n.height());
        }
        // It passes through terrain, so the test is that it holds station rather than sinking:
        // it may graze the floor but must never go through it.
        assert!(
            lowest < floor + 16.0,
            "it should hold above the floor: {lowest} vs {floor}"
        );
    }

    /// With nobody to chase it climbs away, and comes back if somebody turns up.
    #[test]
    fn it_leaves_when_there_is_nobody_left() {
        let tiles = ground();
        let mut n = saucer();
        let empty = world(&tiles, None);
        for _ in 0..60 {
            tick(&mut n, &empty, &tiles);
        }
        assert_eq!(n.ai[0], state::LEAVING_WHOLE);
        assert!(n.velocity.1 < -1.0, "climbing away: {:?}", n.velocity);

        let back = world(&tiles, Some((2000.0, 3100.0)));
        tick(&mut n, &back, &tiles);
        assert_eq!(n.ai[0], state::CIRCUIT, "and it turns right round");
        assert_eq!(n.time_left, 300);
    }
}
