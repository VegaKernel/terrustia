//! Big mimics: style 87.
//!
//! A big mimic is a chest until you touch it, walk within eighty pixels, or hit it — then it stands
//! up and never stops. Its base behaviour is a hop that gets faster and further the more damage it
//! has taken, so a nearly-dead mimic is the dangerous one. Every third hop is a high short one
//! instead of a long low one, and blocked line of sight makes them all higher, which is how one
//! follows you up a shaft.
//!
//! After three and a half seconds of hopping it picks one of three specials at random:
//!
//! * **Curl up** — three seconds taking no damage at all, and in expert batting projectiles back
//!   at you. There is nothing to do about it but move.
//! * **Dive** — it climbs three hundred and fifty pixels straight up, lines up over your head,
//!   commits, and falls.
//! * **Charge** — three long bounds at twelve pixels a tick, aimed higher the further above it you
//!   are.
//!
//! And if you get more than six hundred pixels away it stops fighting and comes back to you
//! *through* the terrain, which is why running from one does not work.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    MIMIC_BIG_HOP_EVERY, MIMIC_CHARGE_ACROSS, MIMIC_CHARGE_AIR_SPEED, MIMIC_CHARGE_BOUNDS,
    MIMIC_CHARGE_REST, MIMIC_CHARGE_UP, MIMIC_CURL_TICKS, MIMIC_DIVE_AIM_SPEED,
    MIMIC_DIVE_AIM_TICKS, MIMIC_DIVE_CAP, MIMIC_DIVE_CLIMB, MIMIC_DIVE_GRAVITY, MIMIC_DIVE_HEIGHT,
    MIMIC_DIVE_LAND_TICKS, MIMIC_DIVE_LINEUP, MIMIC_HOP_ACROSS_HURT, MIMIC_HOP_ACROSS_MIN,
    MIMIC_HOP_BLIND_BONUS, MIMIC_HOP_PATIENCE, MIMIC_HOP_REST_HEALTHY, MIMIC_HOP_REST_MIN,
    MIMIC_HOP_UP, MIMIC_LOSE_RANGE, MIMIC_RETURN_RANGE, MIMIC_RETURN_SPEED, MIMIC_WAKE_RANGE,
    MIMIC_WAKE_TICKS,
};

use super::drifters::Outcome;
use crate::game::ai::{PLAYER_HEIGHT, World, can_see, face};
use crate::game::npc::{Npc, TileView};

/// The states, as `ai[0]` numbers them.
mod state {
    pub const CHEST: f32 = 0.0;
    pub const WAKING: f32 = 1.0;
    pub const HOPPING: f32 = 2.0;
    pub const CURLED: f32 = 3.0;
    pub const CLIMBING: f32 = 4.0;
    pub const DIVING: f32 = 4.1;
    pub const RETURNING: f32 = 5.0;
    pub const CHARGING: f32 = 6.0;
    pub const GIVING_UP: f32 = 7.0;
}

/// What it did this tick.
#[derive(Debug, Default)]
pub struct MimicOutcome {
    pub base: Outcome,
    /// Set while it is curled and batting projectiles back.
    pub reflecting: bool,
}

/// Style 87.
pub fn big_mimic(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    rng: &mut SmallRng,
) -> MimicOutcome {
    let mut out = MimicOutcome::default();
    npc.dirty = true;
    // Every state that wants otherwise sets these itself.
    npc.invulnerable = false;
    npc.no_tile_collide = false;
    npc.no_gravity = false;
    npc.knockback_immune = false;

    // Nobody left to fight: it packs up.
    let alive = world.target.is_some_and(|t| t.alive);
    if npc.ai[0] != state::GIVING_UP && !alive {
        npc.ai = [state::GIVING_UP, 0.0, 0.0, 0.0];
    }
    let Some(target) = world.target else {
        return out;
    };
    let (cx, cy) = npc.center();
    let to_player = (target.center.0 - cx, target.center.1 - cy);
    let reach = to_player.0.hypot(to_player.1);
    let seen = can_see(world.tiles, npc, target);

    match npc.ai[0] {
        s if s == state::CHEST => {
            // Sitting there. Anything that moves it, hits it, or comes close wakes it.
            face(npc, target);
            if npc.velocity.0 != 0.0
                || npc.velocity.1 > 100.0
                || world.was_hurt
                || reach < MIMIC_WAKE_RANGE
            {
                npc.ai[0] = state::WAKING;
                npc.ai[1] = 0.0;
            }
        }

        s if s == state::WAKING => {
            npc.ai[1] += 1.0;
            if npc.ai[1] > MIMIC_WAKE_TICKS {
                npc.ai[0] = state::HOPPING;
                npc.ai[1] = 0.0;
            }
        }

        s if s == state::HOPPING => {
            if reach > MIMIC_LOSE_RANGE {
                npc.ai = [state::RETURNING, 0.0, 0.0, 0.0];
                return out;
            }
            if npc.velocity.1 == 0.0 {
                face(npc, target);
                npc.velocity.0 *= 0.85;
                npc.ai[1] += 1.0;
                // A hurt mimic waits less and jumps further: the fight speeds up as it loses.
                let health = npc.life as f32 / npc.life_max.max(1) as f32;
                let rest = MIMIC_HOP_REST_MIN + MIMIC_HOP_REST_HEALTHY * health;
                let across = MIMIC_HOP_ACROSS_MIN + MIMIC_HOP_ACROSS_HURT * (1.0 - health);
                let mut up = MIMIC_HOP_UP;
                if !seen {
                    // Higher when it cannot see you, so it clears whatever is in the way.
                    up += MIMIC_HOP_BLIND_BONUS;
                }
                if npc.ai[1] > rest {
                    npc.ai[3] += 1.0;
                    let (up, across) = if npc.ai[3] >= MIMIC_BIG_HOP_EVERY {
                        npc.ai[3] = 0.0;
                        // Every third is high and short instead of long and low.
                        (up * 2.0, across / 2.0)
                    } else {
                        (up, across)
                    };
                    npc.ai[1] = 0.0;
                    npc.velocity.1 -= up;
                    npc.velocity.0 = across * f32::from(npc.direction);
                }
            } else {
                // Airborne it keeps its heading and cannot be shoved off it.
                npc.knockback_immune = true;
                npc.velocity.0 *= 0.99;
                if npc.direction < 0 && npc.velocity.0 > -1.0 {
                    npc.velocity.0 = -1.0;
                }
                if npc.direction > 0 && npc.velocity.0 < 1.0 {
                    npc.velocity.0 = 1.0;
                }
            }
            npc.ai[2] += 1.0;
            if npc.ai[2] > MIMIC_HOP_PATIENCE && npc.velocity.1 == 0.0 {
                npc.ai[0] = match rng.random_range(0..3) {
                    0 => state::CURLED,
                    1 => {
                        npc.no_tile_collide = true;
                        npc.velocity.1 = -8.0;
                        state::CLIMBING
                    }
                    _ => state::CHARGING,
                };
                npc.ai[1] = 0.0;
                npc.ai[2] = 0.0;
                npc.ai[3] = 0.0;
            }
        }

        s if s == state::CURLED => {
            npc.velocity.0 *= 0.85;
            npc.invulnerable = true;
            if world.conditions.expert {
                out.reflecting = true;
            }
            npc.ai[1] += 1.0;
            if npc.ai[1] >= MIMIC_CURL_TICKS {
                npc.ai[0] = state::HOPPING;
                npc.ai[1] = 0.0;
            }
        }

        s if s == state::CLIMBING => {
            npc.no_tile_collide = true;
            npc.no_gravity = true;
            npc.knockback_immune = true;
            npc.direction = if npc.velocity.0 < 0.0 { -1 } else { 1 };
            npc.sprite_direction = npc.direction;

            if npc.ai[2] == 1.0 {
                // Lined up. It gathers itself for a few ticks and then commits.
                npc.ai[1] += 1.0;
                let aim = unit(to_player, MIMIC_DIVE_AIM_SPEED);
                npc.velocity = (
                    (npc.velocity.0 * 4.0 + aim.0) / 5.0,
                    (npc.velocity.1 * 4.0 + aim.1) / 5.0,
                );
                if npc.ai[1] > MIMIC_DIVE_AIM_TICKS {
                    npc.ai[1] = 0.0;
                    npc.ai[0] = state::DIVING;
                    npc.ai[2] = 0.0;
                    npc.velocity = aim;
                }
            } else if (cx - target.center.0).abs() < MIMIC_DIVE_LINEUP
                && cy < target.center.1 - 300.0
            {
                // Directly above and high enough.
                npc.ai[1] = 0.0;
                npc.ai[2] = 1.0;
            } else {
                // Climbing to the spot above the player's head.
                let above = (
                    target.center.0 - cx,
                    target.center.1 - MIMIC_DIVE_HEIGHT - cy,
                );
                let wanted = unit(above, MIMIC_DIVE_CLIMB);
                npc.velocity = (
                    (npc.velocity.0 * 5.0 + wanted.0) / 6.0,
                    (npc.velocity.1 * 5.0 + wanted.1) / 6.0,
                );
            }
        }

        s if s == state::DIVING => {
            npc.knockback_immune = true;
            // Once it has a clear line and is out of the rock it becomes solid again, so the dive
            // lands rather than passing through the floor.
            if npc.ai[2] == 0.0 && seen && !boxed(world.tiles, npc) {
                npc.ai[2] = 1.0;
            }
            if npc.position.1 + npc.height() >= target.center.1 - PLAYER_HEIGHT as f32 / 2.0
                || npc.velocity.1 <= 0.0
            {
                npc.ai[1] += 1.0;
                if npc.ai[1] > MIMIC_DIVE_LAND_TICKS {
                    npc.ai = [state::HOPPING, 0.0, 0.0, 0.0];
                    if boxed(world.tiles, npc) {
                        // Landed inside rock: dig back out toward the player instead.
                        npc.ai[0] = state::RETURNING;
                    }
                }
            } else if npc.ai[2] == 0.0 {
                npc.no_tile_collide = true;
                npc.no_gravity = true;
            }
            npc.velocity.1 = (npc.velocity.1 + MIMIC_DIVE_GRAVITY).min(MIMIC_DIVE_CAP);
        }

        s if s == state::RETURNING => {
            // Coming back through the walls.
            npc.direction = if npc.velocity.0 > 0.0 { 1 } else { -1 };
            npc.sprite_direction = npc.direction;
            npc.no_tile_collide = true;
            npc.no_gravity = true;
            npc.knockback_immune = true;
            let aim = (to_player.0, to_player.1 - 4.0);
            if aim.0.hypot(aim.1) < MIMIC_RETURN_RANGE && !boxed(world.tiles, npc) {
                npc.ai = [state::HOPPING, 0.0, 0.0, 0.0];
                return out;
            }
            let wanted = if aim.0.hypot(aim.1) > 10.0 {
                unit(aim, MIMIC_RETURN_SPEED)
            } else {
                aim
            };
            npc.velocity = (
                (npc.velocity.0 * 4.0 + wanted.0) / 5.0,
                (npc.velocity.1 * 4.0 + wanted.1) / 5.0,
            );
        }

        s if s == state::CHARGING => {
            npc.knockback_immune = true;
            if npc.velocity.1 == 0.0 {
                face(npc, target);
                npc.velocity.0 *= 0.8;
                npc.ai[1] += 1.0;
                if npc.ai[1] > MIMIC_CHARGE_REST {
                    npc.ai[1] = 0.0;
                    npc.velocity.1 -= MIMIC_CHARGE_UP;
                    // The further above it you are, the harder it launches.
                    let feet = target.center.1 + PLAYER_HEIGHT as f32 / 2.0;
                    for (height, extra) in [
                        (0.0, 1.25),
                        (40.0, 1.5),
                        (80.0, 1.75),
                        (120.0, 2.0),
                        (160.0, 2.25),
                        (200.0, 2.5),
                    ] {
                        if feet < cy - height {
                            npc.velocity.1 -= extra;
                        }
                    }
                    if !seen {
                        npc.velocity.1 -= 2.0;
                    }
                    npc.velocity.0 = MIMIC_CHARGE_ACROSS * f32::from(npc.direction);
                    npc.ai[2] += 1.0;
                }
            } else {
                npc.velocity.0 *= 0.98;
                if npc.direction < 0 && npc.velocity.0 > -MIMIC_CHARGE_AIR_SPEED {
                    npc.velocity.0 = -MIMIC_CHARGE_AIR_SPEED;
                }
                if npc.direction > 0 && npc.velocity.0 < MIMIC_CHARGE_AIR_SPEED {
                    npc.velocity.0 = MIMIC_CHARGE_AIR_SPEED;
                }
            }
            if npc.ai[2] >= MIMIC_CHARGE_BOUNDS && npc.velocity.1 == 0.0 {
                npc.ai = [state::HOPPING, 0.0, 0.0, 0.0];
            }
        }

        _ => {
            // Giving up: it becomes harmless, heals, fades out and drifts away.
            npc.damage_bonus = 0.0;
            npc.life = npc.life_max;
            npc.defense = 9999;
            npc.no_tile_collide = true;
            npc.alpha = (npc.alpha + 7).min(255);
            npc.velocity.0 *= 0.98;
        }
    }
    out
}

/// A vector of length `speed` along `v`.
fn unit(v: (f32, f32), speed: f32) -> (f32, f32) {
    let length = v.0.hypot(v.1);
    if length <= 0.0 || !length.is_finite() {
        (0.0, 0.0)
    } else {
        (v.0 / length * speed, v.1 / length * speed)
    }
}

/// Whether the NPC's box overlaps solid tiles.
fn boxed(tiles: &impl TileView, npc: &Npc) -> bool {
    let tile = crate::game::npc::TILE;
    let x0 = (npc.position.0 / tile).floor() as i32;
    let x1 = ((npc.position.0 + npc.width() - 1.0) / tile).floor() as i32;
    let y0 = (npc.position.1 / tile).floor() as i32;
    let y1 = ((npc.position.1 + npc.height() - 1.0) / tile).floor() as i32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let t = tiles.tile(x, y);
            if t.is_active() && terrustia_proto::tile_solid::solid(t.block) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ai::Conditions;
    use crate::game::npc::TILE;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    struct Room(HashMap<(i32, i32), Tile>);

    impl TileView for Room {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn floor(at: i32) -> Room {
        let mut tiles = HashMap::new();
        for x in -400..400 {
            for y in at..at + 4 {
                tiles.insert((x, y), Tile::block(1));
            }
        }
        Room(tiles)
    }

    fn world<'a>(tiles: &'a Room, target: Option<(f32, f32)>) -> World<'a, Room> {
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

    /// The corruption big mimic.
    const BIG_MIMIC: u16 = 475;

    fn mimic(tiles: &Room, tile_x: i32, tile_y: i32) -> Npc {
        let mut npc = Npc::new(BIG_MIMIC, (tile_x as f32 * TILE, tile_y as f32 * TILE), 1)
            .expect("big mimic");
        for _ in 0..200 {
            crate::game::npc::step_physics(&mut npc, tiles);
            if npc.on_ground && npc.velocity.1 == 0.0 {
                break;
            }
        }
        npc.velocity = (0.0, 0.0);
        npc
    }

    /// It is furniture until you come near it.
    #[test]
    fn a_mimic_sits_still_until_you_approach() {
        let tiles = floor(30);
        let mut rng = SmallRng::seed_from_u64(87);
        let mut m = mimic(&tiles, 0, 25);
        let far = world(&tiles, Some((10_000.0, 0.0)));
        for _ in 0..600 {
            big_mimic(&mut m, &far, &mut rng);
        }
        assert_eq!(m.ai[0], state::CHEST, "nothing has disturbed it");

        let (cx, cy) = m.center();
        let near = world(&tiles, Some((cx + 40.0, cy)));
        big_mimic(&mut m, &near, &mut rng);
        assert_eq!(m.ai[0], state::WAKING, "that is close enough");
    }

    /// Hitting one from across the room wakes it too.
    #[test]
    fn hitting_a_mimic_wakes_it() {
        let tiles = floor(30);
        let mut rng = SmallRng::seed_from_u64(2);
        let mut m = mimic(&tiles, 0, 25);
        let mut w = world(&tiles, Some((10_000.0, 0.0)));
        w.was_hurt = true;
        big_mimic(&mut m, &w, &mut rng);
        assert_eq!(m.ai[0], state::WAKING);
    }

    /// A hurt mimic hops more often, which is what makes the end of the fight the hard part.
    #[test]
    fn a_hurt_mimic_comes_at_you_faster() {
        let tiles = floor(30);
        let hops = |health: f32| {
            let mut rng = SmallRng::seed_from_u64(5);
            let mut m = mimic(&tiles, 0, 25);
            m.ai[0] = state::HOPPING;
            m.life = (m.life_max as f32 * health) as i32;
            // Inside its six-hundred-pixel leash, or it stops hopping and comes back through
            // the walls instead.
            let w = world(&tiles, Some((20.0 * TILE, 30.0 * TILE)));
            let mut count = 0;
            let mut grounded = true;
            for _ in 0..600 {
                big_mimic(&mut m, &w, &mut rng);
                if grounded && m.velocity.1 < 0.0 {
                    count += 1;
                }
                grounded = m.velocity.1 == 0.0;
                crate::game::npc::step_physics(&mut m, &tiles);
            }
            count
        };
        assert!(
            hops(0.1) > hops(1.0),
            "a hurt mimic should hop more: {} vs {}",
            hops(0.1),
            hops(1.0)
        );
    }

    /// Curled up it takes nothing, and in expert it bats shots back.
    #[test]
    fn a_curled_mimic_is_untouchable() {
        let tiles = floor(30);
        let mut rng = SmallRng::seed_from_u64(3);
        let mut m = mimic(&tiles, 0, 25);
        m.ai[0] = state::CURLED;
        let mut w = world(&tiles, Some((300.0, 400.0)));

        let out = big_mimic(&mut m, &w, &mut rng);
        assert!(m.invulnerable, "nothing gets through");
        assert!(!out.reflecting, "but it only reflects in expert");
        assert!(!m.take_damage(50, 0.0, 1), "and a hit does nothing");

        w.conditions = Conditions {
            expert: true,
            ..Conditions::default()
        };
        assert!(
            big_mimic(&mut m, &w, &mut rng).reflecting,
            "expert: it does"
        );
    }

    /// All three specials come round.
    #[test]
    fn a_mimic_uses_all_three_specials() {
        let tiles = floor(30);
        let mut seen = std::collections::HashSet::new();
        for seed in 0..30 {
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut m = mimic(&tiles, 0, 25);
            m.ai[0] = state::HOPPING;
            m.ai[2] = MIMIC_HOP_PATIENCE;
            let w = world(&tiles, Some((30.0 * TILE, 30.0 * TILE)));
            for _ in 0..400 {
                big_mimic(&mut m, &w, &mut rng);
                if m.ai[0] != state::HOPPING {
                    seen.insert(format!("{}", m.ai[0]));
                    break;
                }
                crate::game::npc::step_physics(&mut m, &tiles);
            }
        }
        assert!(
            seen.len() >= 3,
            "all three specials should appear: {seen:?}"
        );
    }

    /// Running away does not work: it comes back through the walls.
    #[test]
    fn a_mimic_follows_you_through_the_rock() {
        let tiles = floor(30);
        let mut rng = SmallRng::seed_from_u64(9);
        let mut m = mimic(&tiles, 0, 25);
        m.ai[0] = state::HOPPING;
        let w = world(&tiles, Some((80.0 * TILE, 30.0 * TILE)));

        big_mimic(&mut m, &w, &mut rng);
        assert_eq!(m.ai[0], state::RETURNING, "too far: it comes back");

        let start = m.position.0;
        for _ in 0..120 {
            big_mimic(&mut m, &w, &mut rng);
            m.position.0 += m.velocity.0;
            m.position.1 += m.velocity.1;
        }
        assert!(m.no_tile_collide, "through anything in the way");
        assert!(m.position.0 > start, "and toward you");
    }
}
