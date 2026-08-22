//! The NPCs that hold still: styles 72, 73, 92, 122, 124 and 127.
//!
//! None of these hunt. They are furniture, escorts and attendants, and what makes each one
//! interesting is the single condition it dies on:
//!
//! * A **slime chest** (124) only falls.
//! * A **pinned part** (72) has no motion of its own at all — it *is* wherever its parent is, and
//!   the moment the parent is gone it is gone too.
//! * A **training dummy** (92) is held up by the tile it was placed on, and it stops existing when
//!   that tile does, or when every player is more than three hundred tiles away.
//! * A **pirate ghost** (122) drifts at you and shoulders other ghosts aside so a pack spreads out
//!   instead of stacking. With nobody to chase it fades out, and finishing the fade kills it.
//! * A **stationary caster** (73) stands still and lobs something every sixty ticks; being hit
//!   costs it half a reload, so hitting one is how you stop it casting.
//! * A **pal** (127) waits for the escort that spawned with it to die, then for you to come within
//!   a hundred pixels, then pays out and leaves.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::npc_params::{
    CASTER_ARRIVAL, CASTER_COOLDOWN, CASTER_DRAG, CASTER_FADE_IN, CASTER_FLINCH, CASTER_RELOAD,
    CASTER_SHOT, CASTER_SHOT_DAMAGE, CASTER_SHOT_SPEED, CASTER_SPREAD, CHEST_GRAVITY, DUMMY_RANGE,
    DUMMY_TILE, GHOST_EASE, GHOST_FADE, GHOST_PERSONAL_SPACE, GHOST_SHOVE, GHOST_SPEED,
    PAL_APPROACH, PAL_DRAG, PAL_PAYOUT_TICKS,
};

use super::drifters::Outcome;
use crate::game::ai::{Shot, World, face};
use crate::game::npc::{Npc, TILE, TileView};

/// Style 124: falls, and that is the whole routine.
pub fn slime_chest(npc: &mut Npc) {
    npc.velocity.1 += CHEST_GRAVITY;
    npc.dirty = true;
}

/// Style 72: a part that sits exactly on its parent and dies with it.
///
/// `parent` is the parent's centre. Passing `None` means the parent is gone.
pub fn pinned(npc: &mut Npc, parent: Option<(f32, f32)>) -> Outcome {
    let mut out = Outcome::default();
    let Some((cx, cy)) = parent else {
        out.spent = true;
        return out;
    };
    npc.velocity = (0.0, 0.0);
    npc.position = (cx - npc.width() / 2.0, cy - npc.height() / 2.0);
    npc.dirty = true;
    out
}

/// Style 92: a training dummy, which exists only while its tile and a nearby player do.
pub fn training_dummy(npc: &mut Npc, world: &World<'_, impl TileView>) -> Outcome {
    let mut out = Outcome::default();
    // A dummy has no armour: the point of it is to show what a hit really does.
    npc.defense = 0;

    let anchor = (npc.ai[0] as i32, npc.ai[1] as i32);
    let still_placed = world.tiles.tile(anchor.0, anchor.1).is_active()
        && world.tiles.tile(anchor.0, anchor.1).block == DUMMY_TILE;
    let watched = world.target.is_some_and(|t| {
        t.alive && {
            let (cx, cy) = npc.center();
            let (dx, dy) = (t.center.0 - cx, t.center.1 - cy);
            dx.hypot(dy) <= DUMMY_RANGE
        }
    });
    if !still_placed || !watched {
        out.spent = true;
    }
    out
}

/// Style 122: a pirate ghost.
pub fn pirate_ghost(npc: &mut Npc, world: &World<'_, impl TileView>) -> Outcome {
    let mut out = Outcome::default();
    let Some(target) = world.target.filter(|t| t.alive) else {
        // Nothing to haunt: coast to a stop, fade, and go when the fade is done.
        npc.velocity.0 *= 0.9;
        npc.velocity.1 *= 0.9;
        npc.alpha = (npc.alpha + GHOST_FADE).clamp(0, 255);
        if npc.alpha >= 255 {
            out.spent = true;
        }
        npc.dirty = true;
        return out;
    };
    npc.alpha = (npc.alpha - GHOST_FADE).clamp(0, 255);

    let (cx, cy) = npc.center();
    // `MoveTowards` on the offset: the wanted velocity is four pixels along the line to the
    // player, or the whole remaining distance when that is shorter than four pixels.
    let (dx, dy) = (target.center.0 - cx, target.center.1 - cy);
    let reach = dx.hypot(dy);
    let wanted = if reach <= GHOST_SPEED || reach == 0.0 {
        (dx, dy)
    } else {
        (dx / reach * GHOST_SPEED, dy / reach * GHOST_SPEED)
    };
    move_towards(&mut npc.velocity, wanted, GHOST_EASE);

    // Ghosts stacked on one another shove apart, and harder sideways than up: a pack fans out into
    // a line rather than a column.
    for (kx, ky) in world.kin {
        let (ox, oy) = (kx - cx, ky - cy);
        let gap = ox.hypot(oy);
        if gap > 0.0 && gap < GHOST_PERSONAL_SPACE {
            let push = (ox / gap * GHOST_SHOVE, oy / gap * GHOST_SHOVE);
            npc.velocity.0 -= push.0 * 2.0;
            npc.velocity.1 -= push.1;
        }
    }
    npc.dirty = true;
    out
}

/// Step `velocity` toward `wanted` by at most `step`, in a straight line rather than per-axis.
fn move_towards(velocity: &mut (f32, f32), wanted: (f32, f32), step: f32) {
    let (dx, dy) = (wanted.0 - velocity.0, wanted.1 - velocity.1);
    let gap = dx.hypot(dy);
    if gap <= step || gap == 0.0 {
        *velocity = wanted;
    } else {
        velocity.0 += dx / gap * step;
        velocity.1 += dy / gap * step;
    }
}

/// Style 73: a caster that stands its ground.
///
/// `materialises` is true for the type that spends two seconds arriving before it can be hurt.
pub fn stationary_caster(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    rng: &mut SmallRng,
    materialises: bool,
) -> Outcome {
    let mut out = Outcome::default();
    if let Some(target) = world.target {
        face(npc, target);
    }
    npc.velocity.0 *= CASTER_DRAG;
    if npc.velocity.0.abs() < 0.1 {
        npc.velocity.0 = 0.0;
    }
    npc.dirty = true;

    if materialises && npc.ai[1] < CASTER_ARRIVAL {
        npc.ai[1] += 1.0;
        // Solid only once it has finished arriving; until then it is a fading-in ghost.
        npc.alpha = if npc.ai[1] > CASTER_FADE_IN {
            let along = (npc.ai[1] - CASTER_FADE_IN) / (CASTER_ARRIVAL - CASTER_FADE_IN);
            ((1.0 - along) * 255.0) as i32
        } else {
            255
        };
        npc.invulnerable = true;
        return out;
    }
    if materialises && npc.ai[1] == CASTER_ARRIVAL {
        npc.ai[1] += 1.0;
    }
    npc.invulnerable = false;
    npc.alpha = 0;

    let has_target = world.target.is_some_and(|t| t.alive);
    // With nobody in sight the timer is held at the bottom of its cooldown, so a caster that loses
    // you does not come back with a shot already charged.
    if npc.ai[0] == 0.0 && !has_target {
        npc.ai[0] = CASTER_COOLDOWN;
    }
    if npc.ai[0] < CASTER_RELOAD {
        npc.ai[0] += 1.0;
    }
    if world.was_hurt {
        npc.ai[0] = CASTER_FLINCH;
    }
    if npc.ai[0] == CASTER_RELOAD
        && let Some(target) = world.target.filter(|t| t.alive)
    {
        npc.ai[0] = CASTER_COOLDOWN;
        let (cx, cy) = npc.center();
        let from = (cx, cy - 10.0);
        // Aim scattered by a hundred pixels either way, then scaled by up to thirty per cent:
        // a cast never lands quite where the last one did.
        let mut aim = (target.center.0 - from.0, target.center.1 - from.1);
        aim.0 += rng.random_range(-CASTER_SPREAD..=CASTER_SPREAD) as f32;
        aim.1 += rng.random_range(-CASTER_SPREAD..=CASTER_SPREAD) as f32;
        aim.0 *= rng.random_range(70..=130) as f32 * 0.01;
        aim.1 *= rng.random_range(70..=130) as f32 * 0.01;
        let length = aim.0.hypot(aim.1);
        let unit = if length > 0.0 && length.is_finite() {
            (aim.0 / length, aim.1 / length)
        } else {
            // Straight up, which is what the game falls back to when the aim degenerates.
            (0.0, -1.0)
        };
        out.shots.push(Shot {
            projectile: CASTER_SHOT,
            damage: CASTER_SHOT_DAMAGE,
            position: from,
            velocity: (unit.0 * CASTER_SHOT_SPEED, unit.1 * CASTER_SHOT_SPEED),
            time_left: 300,
        });
    }
    out
}

/// Style 127: a pal waiting on its escort.
///
/// `escorts_alive` is how many of the two it was spawned with are still up.
pub fn pal(npc: &mut Npc, world: &World<'_, impl TileView>, escorts_alive: usize) -> Outcome {
    let mut out = Outcome::default();
    if let Some(target) = world.target {
        face(npc, target);
    }
    npc.velocity.0 *= PAL_DRAG;
    if npc.velocity.0.abs() < 0.1 {
        npc.velocity.0 = 0.0;
    }
    npc.dirty = true;

    match npc.ai[0] {
        // Waiting for the escort. While one is alive the pal will not time out.
        0.0 => {
            if escorts_alive == 0 {
                npc.ai[0] = 1.0;
            } else {
                npc.time_left = npc.time_left.max(3600);
            }
        }
        // Escort gone: waiting for the player to come and collect.
        1.0 => {
            if let Some(target) = world.target.filter(|t| t.alive) {
                let (cx, cy) = npc.center();
                if (target.center.0 - cx).hypot(target.center.1 - cy) < PAL_APPROACH {
                    npc.ai[0] = 2.0;
                    npc.ai[1] = 0.0;
                    npc.ai[2] = 0.0;
                }
            }
        }
        // Paying out, which takes two seconds and then it leaves.
        _ => {
            npc.ai[1] += 1.0;
            if npc.ai[1] >= PAL_PAYOUT_TICKS {
                out.spent = true;
                out.became = None;
            }
        }
    }
    out
}

/// Whether a tile column position is where a dummy was planted, used by the server to fill `ai`.
pub fn dummy_anchor(npc: &Npc) -> (i32, i32) {
    (npc.ai[0] as i32, npc.ai[1] as i32)
}

/// The tile a dummy stands on, in tile coordinates, worked out from where it was placed.
pub fn dummy_anchor_from_position(npc: &Npc) -> (i32, i32) {
    let (cx, cy) = npc.center();
    ((cx / TILE) as i32, (cy / TILE) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    /// Empty space with whatever tiles a test puts in it.
    #[derive(Default)]
    struct Air(HashMap<(i32, i32), Tile>);

    impl TileView for Air {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn tiles_with_floor(_at: i32) -> Air {
        Air::default()
    }

    fn world<'a>(tiles: &'a Air, target: Option<(f32, f32)>) -> World<'a, Air> {
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

    /// The pirate ghost, which is the type these tests borrow for anything that only needs a body.
    const PIRATE_GHOST: u16 = 662;

    fn ghost(x: f32) -> Npc {
        Npc::new(PIRATE_GHOST, (x, 0.0), 1).expect("pirate ghost")
    }

    /// The whole point of the shove: two ghosts on the same spot must not stay there.
    #[test]
    fn ghosts_do_not_stack() {
        let tiles = tiles_with_floor(40);
        let mut w = world(&tiles, Some((600.0, 0.0)));
        let mut a = ghost(0.0);
        let kin = [a.center()];
        w.kin = &kin;
        let before = a.velocity.0;
        pirate_ghost(&mut a, &w);
        // The neighbour is at its own centre, so the shove is degenerate and must not produce NaN.
        assert!(a.velocity.0.is_finite(), "shove produced {}", a.velocity.0);
        assert!(a.velocity.0 >= before || a.velocity.0.is_finite());

        // A neighbour to the left pushes it right, on top of its pull toward the player.
        let mut b = ghost(0.0);
        let left = [(b.center().0 - 20.0, b.center().1)];
        w.kin = &left;
        pirate_ghost(&mut b, &w);
        let mut c = ghost(0.0);
        w.kin = &[];
        pirate_ghost(&mut c, &w);
        assert!(
            b.velocity.0 > c.velocity.0,
            "crowded from the left it should move further right: {} vs {}",
            b.velocity.0,
            c.velocity.0
        );
    }

    /// With nobody left to chase a ghost fades and then goes, rather than hanging about forever.
    #[test]
    fn a_ghost_with_nobody_to_chase_fades_out_and_dies() {
        let tiles = tiles_with_floor(40);
        let w = world(&tiles, None);
        let mut g = ghost(0.0);
        g.velocity = (5.0, 5.0);
        let mut ticks = 0;
        let spent = loop {
            let out = pirate_ghost(&mut g, &w);
            ticks += 1;
            if out.spent || ticks > 200 {
                break out.spent;
            }
        };
        assert!(spent, "it should have faded away");
        assert_eq!(ticks, 255 / GHOST_FADE, "the fade takes alpha to full");
        assert!(
            g.velocity.0.abs() < 0.1,
            "and it should have coasted to a stop"
        );
    }

    /// A caster fires on a fixed cycle, and being hit pushes the next shot further away.
    #[test]
    fn hitting_a_caster_delays_its_next_cast() {
        let tiles = tiles_with_floor(40);
        let mut w = world(&tiles, Some((300.0, 0.0)));
        let mut rng = SmallRng::seed_from_u64(4);
        let mut quiet = Npc::new(PIRATE_GHOST, (0.0, 0.0), 1).unwrap();
        quiet.ai[0] = 0.0;

        let shots_over = |npc: &mut Npc, w: &World<'_, _>, rng: &mut SmallRng, ticks: usize| {
            let mut count = 0;
            for _ in 0..ticks {
                count += stationary_caster(npc, w, rng, false).shots.len();
            }
            count
        };

        let mut a = quiet.clone();
        let calm = shots_over(&mut a, &w, &mut rng, 400);
        assert!(calm > 0, "a caster left alone should get shots off");

        let mut b = quiet.clone();
        w.was_hurt = true;
        let harried = shots_over(&mut b, &w, &mut rng, 400);
        assert!(
            harried < calm,
            "being hit every tick should stop it casting: {harried} vs {calm}"
        );
    }

    /// A dummy is held up by its tile; take the tile away and it goes.
    #[test]
    fn a_dummy_without_its_tile_is_gone() {
        let tiles = tiles_with_floor(40);
        let w = world(&tiles, Some((0.0, 0.0)));
        let mut d = Npc::new(PIRATE_GHOST, (0.0, 0.0), 1).unwrap();
        // Point it at a tile that is not a dummy tile.
        d.ai[0] = 5.0;
        d.ai[1] = 41.0;
        assert!(training_dummy(&mut d, &w).spent, "no anchor, no dummy");
        assert_eq!(d.defense, 0, "a dummy never soaks a hit");
    }

    /// A pinned part is wherever its parent is, and nowhere at all once the parent is gone.
    #[test]
    fn a_pinned_part_tracks_its_parent_and_dies_with_it() {
        let mut part = ghost(0.0);
        part.velocity = (9.0, -3.0);
        let out = pinned(&mut part, Some((500.0, 250.0)));
        assert!(!out.spent);
        assert_eq!(part.velocity, (0.0, 0.0), "it has no motion of its own");
        assert_eq!(part.center(), (500.0, 250.0));
        assert!(pinned(&mut part, None).spent, "no parent, no part");
    }

    /// A pal will not leave while its escort lives, and pays out once you reach it.
    #[test]
    fn a_pal_waits_for_its_escort_then_for_you() {
        let tiles = tiles_with_floor(40);
        let far = world(&tiles, Some((5000.0, 0.0)));
        let mut p = ghost(0.0);
        p.time_left = 5;
        pal(&mut p, &far, 2);
        assert_eq!(p.ai[0], 0.0, "it waits while the escort is alive");
        assert!(p.time_left > 5, "and it will not time out meanwhile");

        pal(&mut p, &far, 0);
        assert_eq!(p.ai[0], 1.0, "escort gone, now it waits for you");
        pal(&mut p, &far, 0);
        assert_eq!(p.ai[0], 1.0, "but not from across the world");

        let near = world(&tiles, Some((p.center().0 + 20.0, p.center().1)));
        pal(&mut p, &near, 0);
        assert_eq!(p.ai[0], 2.0, "close enough to collect");
        let mut ticks = 0;
        let paid = loop {
            let out = pal(&mut p, &near, 0);
            ticks += 1;
            if out.spent || ticks > 400 {
                break out.spent;
            }
        };
        assert!(paid, "it should have paid out");
        assert_eq!(ticks, PAL_PAYOUT_TICKS as i32, "after exactly two seconds");
    }
}
