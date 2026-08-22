//! Queen Slime: style 121.
//!
//! Two fights in one. Above half health it is a hopping boss like King Slime — three hops to a
//! set, two low and one high, with the charge between them filling faster at two thirds and a
//! third of its health. Below half it stops touching the ground at all and fights from the air.
//!
//! The part that decides how the first half plays is the anti-cheese teleport. Break line of sight,
//! or get more than three hundred and twenty pixels above it, and a counter builds; at five seconds
//! it fades out and reappears next to you. Standing on a platform out of its reach does not work,
//! and that is deliberate.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    QUEEN_SLIME_CHARGE, QUEEN_SLIME_CHARGE_STEPS, QUEEN_SLIME_CHEESE_AT, QUEEN_SLIME_CHEESE_MAX,
    QUEEN_SLIME_CHEESE_RATE, QUEEN_SLIME_DIVE_RANGE, QUEEN_SLIME_FADE_IN, QUEEN_SLIME_FADE_OUT,
    QUEEN_SLIME_FLIES_AT, QUEEN_SLIME_HOPS, QUEEN_SLIME_HOVER, QUEEN_SLIME_LEASH_TILES,
    QUEEN_SLIME_REACH, QUEEN_SLIME_WAIT, QUEEN_SLIME_WAIT_FLYING,
};

use crate::game::ai::{World, can_see, face};
use crate::game::npc::{Npc, TILE, TileView};

/// The states, as `ai[0]` numbers them.
mod state {
    pub const WAITING: f32 = 0.0;
    pub const ARRIVING: f32 = 1.0;
    pub const VANISHING: f32 = 2.0;
    pub const HOPPING: f32 = 3.0;
    pub const DIVING: f32 = 4.0;
    pub const SWOOPING: f32 = 5.0;
}

/// What it did this tick.
#[derive(Debug, Default)]
pub struct QueenSlimeOutcome {
    /// Where it wants to reappear, once it has finished fading out.
    pub teleport_to: Option<(f32, f32)>,
}

/// Style 121.
pub fn queen_slime(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    rng: &mut SmallRng,
) -> QueenSlimeOutcome {
    let mut out = QueenSlimeOutcome::default();
    npc.dirty = true;

    // On its first tick it holds still for a moment before it starts.
    if npc.local_ai[0] == 0.0 {
        npc.local_ai[0] = 1.0;
        npc.ai[1] = -100.0;
    }

    let health = npc.life as f32 / npc.life_max.max(1) as f32;
    let flying = health <= QUEEN_SLIME_FLIES_AT;
    npc.no_gravity = flying;
    npc.no_tile_collide = false;

    let Some(target) = world.target.filter(|t| t.alive) else {
        npc.time_left = npc.time_left.min(600);
        return out;
    };
    let (cx, cy) = npc.center();
    // Past five hundred tiles it gives up.
    if (cx - target.center.0).abs() / TILE > QUEEN_SLIME_LEASH_TILES {
        npc.time_left = npc.time_left.min(600);
        return out;
    }

    // The anti-cheese counter. It only runs in the ground phase: once it flies it can reach you
    // anywhere, so there is nothing to punish.
    if !flying {
        let out_of_reach = !can_see(world.tiles, npc, target)
            || (npc.position.1 - (target.center.1 + crate::game::ai::PLAYER_HEIGHT as f32 / 2.0))
                .abs()
                > QUEEN_SLIME_REACH;
        if out_of_reach {
            npc.ai[3] = (npc.ai[3] + QUEEN_SLIME_CHEESE_RATE).min(QUEEN_SLIME_CHEESE_MAX);
        } else {
            npc.ai[3] = (npc.ai[3] - 1.0).max(0.0);
        }
        // Full, and standing on the ground: it goes.
        if npc.ai[3] >= QUEEN_SLIME_CHEESE_AT
            && npc.ai[0] == state::WAITING
            && npc.velocity.1 == 0.0
        {
            npc.ai[0] = state::VANISHING;
            npc.ai[1] = 0.0;
        }
    }

    match npc.ai[0] {
        state::WAITING => {
            if flying {
                fly(npc, target.center);
            } else if npc.velocity.1 == 0.0 {
                npc.velocity.0 *= 0.8;
                if npc.velocity.0.abs() < 0.1 {
                    npc.velocity.0 = 0.0;
                }
            }
            // On the ground it only decides while it is standing still.
            if !flying && npc.velocity.1 != 0.0 {
                return out;
            }
            npc.ai[1] += 1.0;
            let wait = if flying {
                QUEEN_SLIME_WAIT_FLYING
            } else {
                QUEEN_SLIME_WAIT
            };
            if npc.ai[1] <= wait {
                return out;
            }
            npc.ai[1] = 0.0;
            if flying {
                // Diving needs you below it and near enough; otherwise it swoops instead.
                let below = target.center.1 > cy;
                let near = (target.center.0 - cx).abs() <= QUEEN_SLIME_DIVE_RANGE;
                npc.ai[0] = if rng.random_range(0..2) == 1 || !below || !near {
                    npc.ai[2] = 0.0;
                    state::SWOOPING
                } else {
                    npc.ai[2] = 1.0;
                    state::DIVING
                };
            } else {
                npc.ai[0] = match rng.random_range(0..3) {
                    1 => state::DIVING,
                    2 => state::SWOOPING,
                    _ => state::HOPPING,
                };
            }
        }

        state::ARRIVING => {
            // Fading back in where it landed.
            npc.rotation = 0.0;
            npc.ai[1] += 1.0;
            npc.alpha = (255.0 * (1.0 - (npc.ai[1] / QUEEN_SLIME_FADE_IN).clamp(0.0, 1.0))) as i32;
            if npc.ai[1] >= QUEEN_SLIME_FADE_IN {
                npc.ai[0] = state::WAITING;
                npc.ai[1] = 0.0;
                npc.alpha = 0;
            }
        }

        state::VANISHING => {
            // Fading out. It is untouchable while it does.
            npc.rotation = 0.0;
            npc.invulnerable = true;
            npc.velocity = (0.0, 0.0);
            npc.ai[1] += 1.0;
            npc.alpha = (255.0 * (npc.ai[1] / QUEEN_SLIME_FADE_OUT).clamp(0.0, 1.0)) as i32;
            if npc.ai[1] >= QUEEN_SLIME_FADE_OUT {
                out.teleport_to = Some(target.center);
                npc.ai[0] = state::ARRIVING;
                npc.ai[1] = 0.0;
                npc.ai[3] = 0.0;
                npc.invulnerable = false;
            }
        }

        _ if flying => {
            // In the air both attacks are the same dive at different angles.
            let aim = if npc.ai[2] == 1.0 {
                (target.center.0 - cx, target.center.1 - cy)
            } else {
                (
                    target.center.0 - cx,
                    target.center.1 - QUEEN_SLIME_HOVER - cy,
                )
            };
            let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
            let speed = if npc.ai[2] == 1.0 { 12.0 } else { 9.0 };
            npc.velocity.0 = (npc.velocity.0 * 9.0 + aim.0 / length * speed) / 10.0;
            npc.velocity.1 = (npc.velocity.1 * 9.0 + aim.1 / length * speed) / 10.0;
            npc.ai[1] += 1.0;
            if npc.ai[1] >= 90.0 {
                npc.ai[0] = state::WAITING;
                npc.ai[1] = 0.0;
                npc.ai[2] = 0.0;
            }
        }

        _ => {
            // The hop set. Only while its feet are down.
            npc.rotation = 0.0;
            if npc.velocity.1 != 0.0 {
                return out;
            }
            npc.velocity.0 *= 0.8;
            if npc.velocity.0.abs() < 0.1 {
                npc.velocity.0 = 0.0;
            }
            // The charge fills faster the more hurt it is.
            npc.ai[1] += QUEEN_SLIME_CHARGE;
            for step in QUEEN_SLIME_CHARGE_STEPS {
                if health < step {
                    npc.ai[1] += QUEEN_SLIME_CHARGE;
                }
            }
            if npc.ai[1] < 0.0 {
                return out;
            }
            face(npc, target);
            let index = (npc.ai[2] as usize).min(QUEEN_SLIME_HOPS.len() - 1);
            let (rise, drift, rest) = QUEEN_SLIME_HOPS[index];
            npc.velocity.1 = rise;
            npc.velocity.0 += drift * f32::from(npc.direction);
            npc.ai[1] = rest;
            if index + 1 >= QUEEN_SLIME_HOPS.len() {
                // The last of the set ends it.
                npc.ai[2] = 0.0;
                npc.ai[0] = state::WAITING;
            } else {
                npc.ai[2] += 1.0;
            }
        }
    }
    out
}

/// The flying phase's idle: it holds station above the player.
fn fly(npc: &mut Npc, player: (f32, f32)) {
    let (cx, cy) = npc.center();
    let gap = (player.0 - cx, player.1 - QUEEN_SLIME_HOVER - cy);
    let reach = gap.0.hypot(gap.1).max(f32::MIN_POSITIVE);
    let wanted = (gap.0 / reach * 8.0, gap.1 / reach * 8.0);
    npc.velocity.0 = (npc.velocity.0 * 19.0 + wanted.0) / 20.0;
    npc.velocity.1 = (npc.velocity.1 * 19.0 + wanted.1) / 20.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::npc_params::QUEEN_SLIME;
    use terrustia_proto::tile::Tile;

    struct Cave(HashMap<(i32, i32), Tile>);

    impl TileView for Cave {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn open() -> Cave {
        Cave(HashMap::new())
    }

    /// A wall between the boss and the player.
    fn walled() -> Cave {
        let mut tiles = HashMap::new();
        for y in -100..100 {
            tiles.insert((5, y), Tile::block(1));
        }
        Cave(tiles)
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

    fn queen(x: f32, y: f32) -> Npc {
        Npc::new(QUEEN_SLIME, (x, y), 1).expect("queen slime")
    }

    /// Above half health it hops; below, it flies.
    #[test]
    fn half_health_puts_it_in_the_air() {
        let tiles = open();
        let mut rng = SmallRng::seed_from_u64(121);
        let w = world(&tiles, Some((300.0, 0.0)));

        let mut ground = queen(0.0, 0.0);
        queen_slime(&mut ground, &w, &mut rng);
        assert!(!ground.no_gravity, "a healthy queen falls");

        let mut air = queen(0.0, 0.0);
        air.life = air.life_max / 4;
        queen_slime(&mut air, &w, &mut rng);
        assert!(air.no_gravity, "a hurt one flies");
    }

    /// Hiding behind a wall builds the teleport, and it goes.
    #[test]
    fn hiding_from_it_makes_it_teleport() {
        let tiles = walled();
        let mut rng = SmallRng::seed_from_u64(2);
        let mut q = queen(0.0, 0.0);
        // On the ground, so it is allowed to go.
        q.velocity.1 = 0.0;
        let w = world(&tiles, Some((600.0, 0.0)));

        let mut went = None;
        for tick in 0..900 {
            // Standing on the ground the whole time: the teleport only fires with its feet down,
            // and nothing here runs physics to put them there.
            q.velocity.1 = 0.0;
            let out = queen_slime(&mut q, &w, &mut rng);
            if let Some(to) = out.teleport_to {
                went = Some((tick, to));
                break;
            }
        }
        let (tick, to) = went.expect("it should have teleported");
        assert!(tick > 200, "not instantly, at {tick}");
        assert_eq!(to, (600.0, 0.0), "and to the player");
        assert_eq!(q.ai[3], 0.0, "with its patience reset");
    }

    /// In plain sight it never teleports, however long the fight runs.
    #[test]
    fn it_stays_put_while_you_face_it() {
        let tiles = open();
        let mut rng = SmallRng::seed_from_u64(3);
        let mut q = queen(0.0, 0.0);
        let w = world(&tiles, Some((200.0, 0.0)));
        for _ in 0..1200 {
            assert!(
                queen_slime(&mut q, &w, &mut rng).teleport_to.is_none(),
                "it should have no reason to leave"
            );
        }
    }

    /// The hop set is two low hops and one high one that ends it.
    #[test]
    fn it_hops_twice_low_and_once_high() {
        let tiles = open();
        let mut rng = SmallRng::seed_from_u64(5);
        let mut q = queen(0.0, 0.0);
        q.ai[0] = state::HOPPING;
        let w = world(&tiles, Some((300.0, 0.0)));

        let mut rises = Vec::new();
        for _ in 0..600 {
            let before = q.velocity.1;
            queen_slime(&mut q, &w, &mut rng);
            if q.velocity.1 < 0.0 && before == 0.0 {
                rises.push(q.velocity.1);
            }
            // Land again straight away, so the set runs to its end.
            q.velocity.1 = 0.0;
            if q.ai[0] != state::HOPPING {
                break;
            }
        }
        assert_eq!(rises.len(), 3, "three hops to a set: {rises:?}");
        assert!(
            rises[2] < rises[0] && rises[2] < rises[1],
            "and the last is the highest: {rises:?}"
        );
    }

    /// It cannot be hurt while it is fading out.
    #[test]
    fn it_is_untouchable_mid_teleport() {
        let tiles = open();
        let mut rng = SmallRng::seed_from_u64(7);
        let mut q = queen(0.0, 0.0);
        q.ai[0] = state::VANISHING;
        let w = world(&tiles, Some((300.0, 0.0)));

        queen_slime(&mut q, &w, &mut rng);
        assert!(q.invulnerable, "nothing lands while it is going");
        assert!(!q.take_damage(1000, 0.0, 1));
    }
}
