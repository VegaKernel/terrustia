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
use crate::game::ai::{PLAYER_HEIGHT, World, can_see, face, unit};
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
    /// C7-07: the 10th-anniversary "stuff cannon" curl, reached only in a celebrationmk10 world by
    /// the Crimson big mimic (`NPC.cs:37956`).
    pub const FIRING: f32 = 8.0;
}

/// The Crimson big mimic (`NPCID.BigMimicCrimson`), the one type the anniversary gag applies to.
const MIMIC_BIG_CRIMSON: u16 = 476;

/// What it did this tick.
#[derive(Debug, Default)]
pub struct MimicOutcome {
    pub base: Outcome,
}

/// Style 87.
pub fn big_mimic(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    rng: &mut SmallRng,
) -> MimicOutcome {
    // Nothing writes to this any more: the curl's invulnerability is set on the NPC directly, and
    // the reflection flag that used to ride out of here modelled a projectile bounce this server
    // cannot perform. `base` was never written either, so the whole outcome is now a formality the
    // hardmode lane can drop when it next opens this file.
    let out = MimicOutcome::default();
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
                // C7-07: in a 10th-anniversary world the Crimson big mimic (type 476) turns half of
                // its curls into the gag "stuff cannon" state instead (`Main.tenthAnniversaryWorld &&
                // type == 476 && ai[0] == 3f && Main.rand.Next(2) == 0`, `NPC.cs:37956-37959`).
                if npc.ai[0] == state::CURLED
                    && world.conditions.tenth_anniversary
                    && npc.npc_type == MIMIC_BIG_CRIMSON
                    && rng.random_ratio(1, 2)
                {
                    npc.ai[0] = state::FIRING;
                }
                npc.ai[1] = 0.0;
                npc.ai[2] = 0.0;
                npc.ai[3] = 0.0;
            }
        }

        s if s == state::CURLED => {
            npc.velocity.0 *= 0.85;
            // `NPC.cs:37968`: the curl really is `dontTakeDamage = true`, in every mode. Expert
            // adds `reflectsProjectiles` on top (`NPC.cs:37977-37980`), which this server does not
            // model - a player's shots are the client's to simulate - and which would change
            // nothing here anyway, since the curl is already untouchable.
            npc.invulnerable = true;
            npc.ai[1] += 1.0;
            if npc.ai[1] >= MIMIC_CURL_TICKS {
                npc.ai[0] = state::HOPPING;
                npc.ai[1] = 0.0;
            }
        }

        s if s == state::FIRING => {
            // C7-07: the 10th-anniversary "stuff cannon" curl. It sits still for the same three
            // seconds as an ordinary curl and, in vanilla, lobs a burst of ten random junk items at
            // you every twenty ticks (`AI_87_BigMimic_FireStuffCannonBurst`, `NPC.cs:38184-38198`).
            // Unlike the ordinary curl it never sets `dontTakeDamage`, so it stays open to a hit the
            // whole time. NARROWING: the item-throwing gag is not modelled - the server has no
            // channel for an NPC to fling collectable world items - so only the state itself and its
            // vulnerability window are transcribed here.
            npc.velocity.0 *= 0.85;
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
        make_mimic(tiles, BIG_MIMIC, tile_x, tile_y)
    }

    fn crimson_mimic(tiles: &Room, tile_x: i32, tile_y: i32) -> Npc {
        make_mimic(tiles, MIMIC_BIG_CRIMSON, tile_x, tile_y)
    }

    fn make_mimic(tiles: &Room, npc_type: u16, tile_x: i32, tile_y: i32) -> Npc {
        let mut npc =
            Npc::new(npc_type, (tile_x as f32 * TILE, tile_y as f32 * TILE), 1).expect("big mimic");
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

        big_mimic(&mut m, &w, &mut rng);
        assert!(m.invulnerable, "nothing gets through");
        assert!(!m.take_damage(50, 0.0, 1), "and a hit does nothing");

        // `NPC.cs:37968` sets `dontTakeDamage` for the curl outright, so it holds in classic too.
        // Expert only adds `reflectsProjectiles` on top, which this server does not model.
        w.conditions = Conditions {
            expert: true,
            ..Conditions::default()
        };
        big_mimic(&mut m, &w, &mut rng);
        assert!(m.invulnerable, "expert changes nothing about that");
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

    /// C7-07: in a 10th-anniversary world the Crimson big mimic (type 476) can turn a curl into the
    /// gag "stuff cannon" state (`NPC.cs:37956-37959`). A plain world, or another mimic type, never
    /// does.
    #[test]
    fn a_crimson_mimic_gets_the_stuff_cannon_in_an_anniversary_world() {
        let tiles = floor(30);
        let reaches_firing = |anniversary: bool, npc_type: u16| {
            for seed in 0..120u64 {
                let mut rng = SmallRng::seed_from_u64(seed);
                let mut m = make_mimic(&tiles, npc_type, 0, 25);
                m.ai[0] = state::HOPPING;
                m.ai[2] = MIMIC_HOP_PATIENCE;
                let mut w = world(&tiles, Some((30.0 * TILE, 30.0 * TILE)));
                w.conditions.tenth_anniversary = anniversary;
                for _ in 0..400 {
                    big_mimic(&mut m, &w, &mut rng);
                    if m.ai[0] == state::FIRING {
                        return true;
                    }
                    if m.ai[0] != state::HOPPING {
                        break;
                    }
                    crate::game::npc::step_physics(&mut m, &tiles);
                }
            }
            false
        };
        assert!(
            reaches_firing(true, MIMIC_BIG_CRIMSON),
            "the anniversary Crimson mimic should reach the stuff-cannon state"
        );
        assert!(
            !reaches_firing(false, MIMIC_BIG_CRIMSON),
            "a plain world never gives it"
        );
        assert!(
            !reaches_firing(true, BIG_MIMIC),
            "and neither does the Corruption mimic, even in an anniversary world"
        );
    }

    /// C7-07: the stuff-cannon curl stays open to a hit for its whole three seconds, unlike the
    /// ordinary curl which takes nothing, and then resumes hopping.
    #[test]
    fn the_stuff_cannon_curl_is_vulnerable_and_ends() {
        let tiles = floor(30);
        let mut rng = SmallRng::seed_from_u64(1);
        let mut m = crimson_mimic(&tiles, 0, 25);
        m.ai[0] = state::FIRING;
        let w = world(&tiles, Some((300.0, 400.0)));

        big_mimic(&mut m, &w, &mut rng);
        assert!(!m.invulnerable, "the stuff-cannon curl stays open to a hit");
        let before = m.life;
        m.take_damage(10, 0.0, 1);
        assert!(m.life < before, "and a hit lands, unlike the ordinary curl");

        m.ai[0] = state::FIRING;
        m.ai[1] = 0.0;
        for _ in 0..(MIMIC_CURL_TICKS as i32 + 1) {
            big_mimic(&mut m, &w, &mut rng);
        }
        assert_eq!(m.ai[0], state::HOPPING, "then it resumes hopping");
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
