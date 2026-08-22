//! NPC behaviour, one module per AI style.
//!
//! Two rules hold this apart from the 59,000-line original it is ported from:
//!
//! 1. **Per-type variation lives in data, never in branches.** Where the game writes
//!    `if (type == 25) speed = 5;` inside its AI, that number belongs in a table. A hand-written
//!    module here contains algorithms only.
//! 2. **Parity is tracked, not claimed.** Every style declares a [`Parity`] level, so a style that
//!    is still an approximation is visible in a test rather than hidden behind one.

pub mod ambush;
pub mod balloon;
pub mod bat;
pub mod bird;
pub mod boss;
pub mod caster;
pub mod creeper;
pub mod critter;
pub mod dragonfly;
pub mod eater;
pub mod eye;
pub mod fighter;
pub mod fish;
pub mod frost;
pub mod granite;
pub mod grub;
pub mod hardmode;
pub mod haunt;
pub mod inert;
pub mod mimic;
pub mod orb;
pub mod rooted;
pub mod sight;
pub mod skull;
pub mod slime;
pub mod snail;
pub mod spore;
pub mod swimmer;
pub mod town;
pub mod track;
pub mod tumbleweed;
pub mod worm;

use rand::rngs::SmallRng;

use terrustia_proto::npc_params::{FlierSteering, Steering};

use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Target;

/// The player hitbox every routine aims at, from `Player.SetDefaults`.
pub const PLAYER_WIDTH: i32 = 20;
pub const PLAYER_HEIGHT: i32 = 42;

/// World state the routines read.
///
/// The game reaches into `Main` for these from inside the AI; passing them in keeps the routines
/// testable and keeps `Main`-shaped global state out of the port.
#[derive(Debug, Clone, Copy)]
pub struct Conditions {
    pub blood_moon: bool,
    pub day: bool,
    pub eclipse: bool,
    /// Rain, which sends residents indoors just as nightfall does.
    pub raining: bool,
    /// Whether the day is windy enough for the things that need wind to do anything.
    pub windy: bool,
    /// Whether the nearest player is standing in the crimson, which is where the Brain lives.
    pub crimson: bool,
    /// ...and whether they are in the jungle, which is where the Queen lives.
    pub jungle: bool,
    /// ...and whether they are in the snow, which is where Deerclops hunts.
    pub snow: bool,
    /// Wind speed, signed: positive blows east.
    pub wind: f32,
    /// Whether the nearest player is standing in a desert, which is the only place a tumbleweed
    /// will stay.
    pub desert: bool,
    /// Pixel depth of the surface layer, below which "outdoors" stops applying.
    pub surface_y: f32,
    /// Whether the world is expert or better.
    ///
    /// A great many hardmode routines are not merely tuned differently in expert — they gain
    /// attacks they do not otherwise have, so this changes what an enemy *does*, not just how
    /// hard it hits.
    pub expert: bool,
    /// Whether hardmode has begun. Some routines behave differently before the wall falls.
    pub hardmode: bool,
    /// The world's size in tiles, for the handful of routines that steer away from its edges.
    pub world_size: (i32, i32),
}

impl Default for Conditions {
    /// A calm night in a large world: every flag false, as a derived `Default` would give, except
    /// the world size.
    ///
    /// The size is the one field that must not be zero. A routine that steers away from the edges
    /// of the world would decide it was against one, so "no world at all" is not a usable default
    /// the way "no wind" and "no blood moon" are.
    fn default() -> Self {
        Self {
            blood_moon: false,
            day: false,
            eclipse: false,
            raining: false,
            windy: false,
            crimson: false,
            jungle: false,
            snow: false,
            wind: 0.0,
            desert: false,
            surface_y: 0.0,
            expert: false,
            hardmode: false,
            world_size: (4200, 1200),
        }
    }
}

/// The top-left corner of a target's hitbox, which is what the collision routines want.
fn target_box(t: Target) -> (f32, f32) {
    (
        t.center.0 - PLAYER_WIDTH as f32 / 2.0,
        t.center.1 - PLAYER_HEIGHT as f32 / 2.0,
    )
}

/// Whether an NPC has a clear line to a target.
pub fn can_see(tiles: &impl TileView, npc: &Npc, t: Target) -> bool {
    sight::can_hit(
        tiles,
        npc.position,
        (npc.stats.width, npc.stats.height),
        target_box(t),
        (PLAYER_WIDTH, PLAYER_HEIGHT),
    )
}

/// The nearest living player, at any distance.
///
/// This is `NPC.TargetClosest`, and the two things it does differently from a plain search matter:
/// it measures with Manhattan distance rather than a real one, and it has no range limit at all.
/// Enemies do not lose interest when you outrun them — the despawn timer is what eventually
/// removes them.
pub fn target_closest(npc: &Npc, targets: &[Target]) -> Option<Target> {
    let (cx, cy) = npc.center();
    targets
        .iter()
        .filter(|t| t.alive)
        .min_by(|a, b| {
            let reach = |t: &&Target| (t.center.0 - cx).abs() + (t.center.1 - cy).abs();
            reach(a).total_cmp(&reach(b))
        })
        .copied()
}

/// Turn to face a target on both axes, the way `SetTargetTrackingValues` does.
///
/// The comparison is between hitbox midpoints computed with integer division, so an NPC of odd
/// width faces the way the game's arithmetic says rather than the way real arithmetic would.
pub fn face(npc: &mut Npc, t: Target) {
    let (bx, by) = target_box(t);
    let mid_x = bx as i32 + PLAYER_WIDTH / 2;
    let mid_y = by as i32 + PLAYER_HEIGHT / 2;
    npc.direction = if (mid_x as f32) < npc.position.0 + (npc.stats.width / 2) as f32 {
        -1
    } else {
        1
    };
    npc.direction_y = if (mid_y as f32) < npc.position.1 + (npc.stats.height / 2) as f32 {
        -1
    } else {
        1
    };
}

/// Accelerate one axis toward the way it is facing.
///
/// Three terms, and the middle one carries the character: while the velocity still points the
/// wrong way the routine applies a nudge that either pushes back against the turn — giving a bat
/// its wide lazy arc — or hurries it along, which is what makes a wandering eye dart.
pub fn steer_axis(velocity: &mut f32, direction: i8, s: Steering) {
    steer_axis_gated(velocity, direction, s, true, true);
}

/// As [`steer_axis`], but with an extra condition on each arm.
///
/// The floating eye will only accelerate toward a target it has not already passed, so each arm
/// carries a position test alongside its speed test.
pub fn steer_axis_gated(
    velocity: &mut f32,
    direction: i8,
    s: Steering,
    toward_negative: bool,
    toward_positive: bool,
) {
    if direction == -1 && *velocity > -s.max && toward_negative {
        *velocity -= s.accel;
        if *velocity > s.overshoot_at {
            *velocity -= s.overshoot;
        } else if *velocity > 0.0 {
            *velocity += s.brake;
        }
        if *velocity < -s.max {
            *velocity = -s.max;
        }
    } else if direction == 1 && *velocity < s.engage_positive && toward_positive {
        *velocity += s.accel;
        if *velocity < -s.overshoot_at {
            *velocity += s.overshoot;
        } else if *velocity < 0.0 {
            *velocity -= s.brake;
        }
        if *velocity > s.max {
            *velocity = s.max;
        }
    }
}

/// Steer both axes at once.
pub fn steer(npc: &mut Npc, s: FlierSteering) {
    steer_axis(&mut npc.velocity.0, npc.direction, s.x);
    steer_axis(&mut npc.velocity.1, npc.direction_y, s.y);
}

/// Bounce off terrain, keeping enough speed to clear whatever was hit.
///
/// The reflection uses the velocity from *before* the move, because collision has already zeroed
/// the live one. Shared by the bat and eye styles, which write it identically.
pub fn bounce(npc: &mut Npc) {
    if npc.collide_x {
        npc.velocity.0 = npc.old_velocity.0 * -0.5;
        if npc.direction == -1 && npc.velocity.0 > 0.0 && npc.velocity.0 < 2.0 {
            npc.velocity.0 = 2.0;
        }
        if npc.direction == 1 && npc.velocity.0 < 0.0 && npc.velocity.0 > -2.0 {
            npc.velocity.0 = -2.0;
        }
    }
    if npc.collide_y {
        npc.velocity.1 = npc.old_velocity.1 * -0.5;
        if npc.velocity.1 > 0.0 && npc.velocity.1 < 1.0 {
            npc.velocity.1 = 1.0;
        }
        if npc.velocity.1 < 0.0 && npc.velocity.1 > -1.0 {
            npc.velocity.1 = -1.0;
        }
    }
}

/// Swim upward out of water rather than flying through it.
///
/// Half a pixel a tick against whatever the routine was doing, capped at four. The bat and eye
/// styles share it verbatim.
pub fn rise_out_of_water(npc: &mut Npc) {
    if npc.velocity.1 > 0.0 {
        npc.velocity.1 *= 0.95;
    }
    npc.velocity.1 -= 0.5;
    if npc.velocity.1 < -4.0 {
        npc.velocity.1 = -4.0;
    }
}

/// A projectile a routine wants launched.
///
/// The projectile subsystem does not exist yet, so these are collected and counted rather than
/// flown. Emitting them from the routine keeps the decision — cadence, aim, scatter, reload — in
/// the port rather than waiting on the entity that carries it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    pub projectile: u16,
    pub damage: i32,
    pub position: (f32, f32),
    pub velocity: (f32, f32),
    pub time_left: u16,
}

/// The types worth counting each tick, because some routine's behaviour turns on how many are up.
pub const CENSUS_TYPES: [u16; 4] = [
    terrustia_proto::npc_params::CREEPER,
    terrustia_proto::npc_params::WALL_LEECH,
    terrustia_proto::npc_params::PAL_ESCORT,
    terrustia_proto::npc_params::DUTCHMAN_GUN,
];

impl<T: TileView> World<'_, T> {
    /// How many of `npc_type` are alive right now. Types nobody asked about read as none.
    /// The world's width in tiles.
    pub fn world_width(&self) -> i32 {
        self.conditions.world_size.0
    }

    /// The world's height in tiles.
    pub fn world_height(&self) -> i32 {
        self.conditions.world_size.1
    }

    pub fn count(&self, npc_type: u16) -> usize {
        self.census
            .iter()
            .find(|(ty, _)| *ty == npc_type)
            .map_or(0, |(_, n)| *n)
    }
}

/// A world with nothing going on in it, for the tests that only care about one thing at a time.
///
/// Every routine's tests need a `World`, and spelling the whole struct out in each module means a
/// new field is thirty edits. This is the one place that knows what "nothing going on" is.
#[cfg(test)]
pub fn calm<T: TileView>(tiles: &T, target: Option<crate::game::npc_ai::Target>) -> World<'_, T> {
    World {
        tiles,
        target,
        wet: false,
        target_wet: false,
        conditions: Conditions::default(),
        was_hurt: false,
        target_velocity: (0.0, 0.0),
        census: &[],
        parent: None,
        parent_state: 0.0,
        parent_health: 1.0,
        crowding: (0.0, 0.0),
        avoid: &[],
    }
}

/// How close a style's implementation is to the game's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    /// Transcribed from the decompiled routine: same constants, same decisions.
    Ported,
    /// Recognisable but invented. Constants here are guesses and are not parity.
    Approximate,
}

/// The parity level of the routine driving an AI style.
///
/// Anything absent has no implementation at all.
pub fn parity(style: i32) -> Option<Parity> {
    let level = match style {
        // Ported from the decompiled source.
        0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19
        | 20 | 21 | 22 | 23 | 24 | 26 | 27 | 28 | 29 | 38 | 42 | 43 | 44 | 49 | 50 | 54 | 55
        | 56 | 62 | 63 | 65 | 66 | 67 | 25 | 39 | 40 | 41 | 64 | 68 | 70 | 72 | 73 | 80 | 89
        | 91 | 92 | 95 | 96 | 99 | 100 | 101 | 104 | 116 | 93 | 102 | 103 | 122 | 124 | 127
        | 113 | 114 | 115 | 118 | 119 | 123 | 125 | 126 => Parity::Ported,
        _ => return None,
    };
    Some(level)
}

/// Whether this module owns the style, as opposed to the older approximations.
pub fn is_ported(style: i32) -> bool {
    parity(style) == Some(Parity::Ported)
}

/// Everything a ported routine may ask the world for.
pub struct World<'a, T: TileView> {
    pub tiles: &'a T,
    pub target: Option<Target>,
    /// Whether the NPC is standing in liquid.
    pub wet: bool,
    /// Whether its target is.
    pub target_wet: bool,
    pub conditions: Conditions,
    /// Whether the NPC was hit since its last tick.
    pub was_hurt: bool,
    /// How fast the target is moving, for the routines that lead it.
    pub target_velocity: (f32, f32),
    /// How many of each NPC type are alive, for the routines that wait on their escort, their
    /// armour or their swarm. Only the types something asks about are counted.
    pub census: &'a [(u16, usize)],
    /// For a boss part, where its parent is and how big it is.
    pub parent: Option<boss::skeletron::Parent>,
    /// ...and which state that parent is in.
    pub parent_state: f32,
    /// ...and what fraction of its health it has left.
    pub parent_health: f32,
    /// What this one keeps its distance from: the rest of its own kind for a pirate ghost,
    /// anything alive at all for a shimmerfly. Empty unless the style asks for it, because
    /// building it is a scan of the whole table.
    pub avoid: &'a [(f32, f32)],
    /// A unit push away from whatever nearby the NPC would rather not be next to.
    ///
    /// The routines that read this cannot see other NPCs, so the caller averages the directions
    /// away from anything dangerous close by and hands the result in. Zero means all clear.
    pub crowding: (f32, f32),
}

/// What a ported routine did beyond moving its NPC.
#[derive(Debug, Default)]
pub struct Effects {
    /// NPCs a routine conjured.
    pub spawn: Vec<crate::game::npc_ai::Spawn>,
    pub doors: Vec<fighter::Action>,
    /// Doors a town NPC opened or pulled shut behind itself.
    pub town_doors: Vec<town::DoorAction>,
    pub shots: Vec<Shot>,
    /// Set when the routine wants its NPC gone.
    pub expired: bool,
    /// Set when the routine killed its NPC outright, as a spore does when it bursts.
    pub died: bool,
    /// Set when a routine's roar should leave everyone nearby slowed.
    pub roared: bool,
    /// A type this NPC should become: a lost girl dropping her disguise, a truffle worm fleeing.
    pub transform: Option<u16>,
    /// Set when what this NPC just did calls in an invasion.
    pub called_invasion: bool,
    /// Set when it went off rather than merely dying, which hurts whatever is next to it.
    pub detonated: bool,
    /// How long the thing it just turned into should sit still before doing anything.
    pub rest_for: i32,
    /// Where this NPC wants whatever it is carrying to hang.
    pub carry: Option<(f32, f32)>,
}

/// Drive an NPC whose style is [`Parity::Ported`].
///
/// Dispatch lives here rather than beside the approximations so that claiming parity for a style
/// and actually running its routine are the same edit. The final arm is unreachable by
/// construction, and a test walks the whole roster to prove it.
pub fn run<T: TileView>(npc: &mut Npc, world: &World<'_, T>, rng: &mut SmallRng) -> Effects {
    let mut effects = Effects::default();
    let target = world.target;
    match npc.stats.ai_style {
        0 => inert::update(npc, target),
        1 => slime::update(npc, target, npc.on_ground),
        // No graveyard biome yet, so nothing keeps the eyes out past dawn.
        2 => eye::update(npc, world, false),
        6 => worm::update(npc, world, false),
        7 => {
            let home = npc.home.map(|(tile_x, tile_y)| town::Home {
                tile_x,
                floor_y: town::floor_under(world.tiles, tile_x, tile_y, i32::MAX / 2),
            });
            let action = town::update(npc, world, home, rng);
            if action != town::DoorAction::None {
                effects.town_doors.push(action);
            }
        }
        5 => {
            // No expert mode yet, so the eater of souls uses its classic acceleration.
            if let Some(shot) = eater::update(npc, world, rng, false) {
                effects.shots.push(shot);
            }
        }
        3 => {
            let action = fighter::update(npc, world.tiles, target, world.conditions);
            if action != fighter::Action::None {
                effects.doors.push(action);
            }
        }
        9 => orb::update(npc, target),
        19 => {
            if let Some(shot) = ambush::antlion(npc, world) {
                effects.shots.push(shot);
            }
        }
        20 => track::spike_ball(npc, target, rand::Rng::random_range(rng, 0..15)),
        13 => effects.died = rooted::plant(npc, world) == rooted::Outcome::Uprooted,
        17 => rooted::vulture(npc, world),
        8 => {
            let cast = caster::update(npc, world, rng);
            if let Some((npc_type, position)) = cast.summon {
                effects.spawn.push(crate::game::npc_ai::Spawn {
                    npc_type,
                    position,
                    velocity: (0.0, 0.0),
                    parent: None,
                });
            }
            if let Some(shot) = cast.shot {
                effects.shots.push(shot);
            }
        }
        67 => snail::update(npc, world, rng),
        4 => effects.spawn.extend(boss::eye::update(npc, world)),
        11 => effects.spawn.extend(boss::skeletron::head(npc, world)),
        27 => {
            let advance = boss::wall::wall(
                npc,
                world,
                world.count(terrustia_proto::npc_params::WALL_LEECH),
                rng,
            );
            effects.spawn.extend(advance.spawn);
            effects.shots.extend(advance.shots);
            effects.expired = advance.gone;
        }
        28 => {
            if let Some(shot) = boss::wall::eye(npc, world, world.parent, world.parent_health) {
                effects.shots.push(shot);
            }
            effects.expired = world.parent.is_none();
        }
        29 => {
            effects.expired = !boss::wall::hungry(npc, world, world.parent, world.parent_health);
        }
        123 => {
            let rampage = boss::deerclops::update(npc, world, rng);
            effects.shots.extend(rampage.shots);
            effects.roared = rampage.roared;
            effects.expired = rampage.gone;
        }
        43 => {
            let hive = boss::queen_bee::update(npc, world, rng);
            effects.spawn.extend(hive.bees);
            effects.shots.extend(hive.stingers);
        }
        12 => {
            // A hand is bound to its head, whose state the caller reads off the NPC table.
            let outcome = boss::skeletron::hand(
                npc,
                world.parent,
                world.parent_state == boss::skeletron::HOVERING,
                world.parent_state == boss::skeletron::LEAVING,
                world.target,
            );
            effects.expired = outcome == boss::skeletron::HandOutcome::Orphaned;
        }
        54 => {
            // The creeper count and the biome are the caller's to know.
            let swarm = boss::brain::update(
                npc,
                world,
                world.count(terrustia_proto::npc_params::CREEPER),
                world.conditions.crimson,
                world.target_velocity,
                rng,
            );
            for (position, velocity) in swarm.creepers {
                effects.spawn.push(crate::game::npc_ai::Spawn {
                    npc_type: terrustia_proto::npc_params::CREEPER,
                    position,
                    velocity,
                    parent: None,
                });
            }
            effects.expired = swarm.gone;
        }
        15 => {
            let court = boss::king_slime::update(npc, world, rng);
            for (npc_type, position, velocity) in court.shed {
                effects.spawn.push(crate::game::npc_ai::Spawn {
                    npc_type,
                    position,
                    velocity,
                    parent: None,
                });
            }
        }
        126 => mimic::update(npc, world, rng),
        23 => hardmode::hoverers::flying_weapon(npc, world),
        39 => hardmode::roller::roller(npc, world, rng),
        64 => critter::firefly(npc, world, rng),
        68 => {
            let landing = bird::waterfowl(npc, world, rng);
            if let Some(walker) = landing.becomes {
                effects.transform = Some(walker);
                effects.rest_for = landing.rests_for;
            }
        }
        102 => effects
            .shots
            .extend(hardmode::sand::sand_elemental(npc, world, rng).shots),
        103 => hardmode::sand::sand_shark(npc, world),
        40 => {
            let out = hardmode::crawler::crawler(npc, world, rng);
            effects.shots.extend(out.shots);
            effects.transform = out.became;
        }
        41 => {
            let out = hardmode::leaper::leaper(npc, world);
            effects.expired = out.spent;
            effects.detonated = out.detonated;
        }
        25 => {
            // The Snowman Gangsta only takes an interest during the Frost Moon, which this server
            // does not run yet, so it hops without ever turning to face anybody.
            let indifferent = npc.npc_type == terrustia_proto::npc_params::SNOWMAN_GANGSTA;
            hardmode::hopper::hopper(npc, world, indifferent);
        }
        80 => {
            let out = hardmode::invasion::probe(npc, world);
            effects.expired = out.spent;
            effects.called_invasion = out.called_the_invasion;
        }
        93 => {
            let cannon = world.count(terrustia_proto::npc_params::DUTCHMAN_GUN);
            let out = hardmode::invasion::dutchman(npc, world, rng, cannon);
            effects.spawn.extend(out.spawn);
            effects.expired = out.spent;
        }
        72 => {
            // A pinned part is drawn on its parent, so it needs the parent's centre rather than
            // its corner.
            let at = world
                .parent
                .map(|(position, (w, h))| (position.0 + w / 2.0, position.1 + h / 2.0));
            effects.expired = hardmode::fixtures::pinned(npc, at).spent;
        }
        73 => {
            // Only the Martian turret spends its first two seconds deploying, untouchable; the
            // other style-73 types are simply standing there already.
            let materialises = npc.npc_type == terrustia_proto::npc_params::MARTIAN_TURRET;
            let out = hardmode::fixtures::stationary_caster(npc, world, rng, materialises);
            effects.shots.extend(out.shots);
        }
        92 => effects.expired = hardmode::fixtures::training_dummy(npc, world).spent,
        122 => effects.expired = hardmode::fixtures::pirate_ghost(npc, world).spent,
        124 => hardmode::fixtures::slime_chest(npc),
        127 => {
            let escorts = world.count(terrustia_proto::npc_params::PAL_ESCORT);
            effects.expired = hardmode::fixtures::pal(npc, world, escorts).spent;
        }
        49 => effects.shots.extend(hardmode::hoverers::nimbus(npc, world)),
        56 => hardmode::drifters::dungeon_spirit(npc, world),
        62 => effects
            .shots
            .extend(hardmode::hoverers::copter(npc, world, rng)),
        70 => {
            effects.expired = hardmode::hoverers::detonating_bubble(npc, world, rng).spent;
        }
        100 => effects.expired = hardmode::hoverers::ancient_light(npc).spent,
        101 => {
            let out =
                hardmode::hoverers::ancient_doom(npc, world.parent.map(|_| world.parent_health));
            effects.shots.extend(out.shots);
            effects.expired = out.spent;
        }
        116 => {
            let (cx, cy) = npc.center();
            let line = critter::water_line(
                world.tiles,
                (cx / crate::game::npc::TILE) as i32,
                (cy / crate::game::npc::TILE) as i32,
            );
            hardmode::hoverers::water_strider(npc, line, rng, world.wet);
        }
        63 => hardmode::drifters::flocko(npc, world),
        89 => {
            let out = hardmode::drifters::mothron_egg(npc, world.was_hurt, rng);
            effects.transform = out.became;
        }
        95 => effects.transform = hardmode::drifters::stardust_cell(npc).became,
        96 => {
            let out = hardmode::drifters::stardust_jellyfish(npc, world, rng);
            effects.shots.extend(out.shot);
        }
        99 => effects.expired = hardmode::drifters::solar_goop(npc).spent,
        // Nothing but a marker: it removes itself the moment it is asked to do anything.
        104 => effects.expired = true,
        // Sandstorms are not modelled, so the wind never reaches a tumbleweed.
        26 => tumbleweed::update(npc, world, false),
        113 | 125 => {
            if balloon::update(npc, world) == balloon::Outcome::Popped {
                effects.died = true;
            }
            effects.carry = Some(balloon::carry_point(npc));
        }
        114 => dragonfly::update(npc, world, rng),
        65 => critter::butterfly(npc, world, rng),
        // One hit in six knocks it down, but only when the hit was hard enough to register.
        91 => granite::update(
            npc,
            world,
            world.was_hurt && rand::Rng::random_ratio(rng, 1, 6),
        ),
        22 => {
            // The drift it picks when it has nothing else to go on: -1.5, 0 or 1.5.
            let drift = rand::Rng::random_range(rng, -1..2) as f32 * 1.5;
            haunt::update(npc, world, drift);
        }
        38 => {
            if let Some(shot) = frost::update(npc, world) {
                effects.shots.push(shot);
            }
        }
        10 => {
            if let Some(shot) = skull::update(npc, world) {
                effects.shots.push(shot);
            }
        }
        18 => swimmer::jellyfish(npc, world),
        // Shoaling is the caller's business; nothing here can see the rest of the shoal.
        44 => swimmer::flying_fish(npc, world, (0.0, 0.0)),
        21 => track::wheel(npc),
        115 => critter::ladybug(npc, world, rng),
        118 => critter::seahorse(npc, world, rng),
        119 => effects.shots.extend(critter::dandelion(npc, world, rng)),
        42 => effects.transform = ambush::lost_girl(npc, world),
        66 => effects.transform = grub::update(npc, world, rng),
        14 => {
            if let Some(shot) = bat::update(npc, world, rng) {
                effects.shots.push(shot);
            }
        }
        16 => fish::update(npc, target, world.wet, world.target_wet),
        24 => {
            npc.no_gravity = !bird::update(npc, target, world.was_hurt);
        }
        50 => {
            let hit_something = npc.collide_x || npc.collide_y;
            effects.died = spore::update(npc, target, hit_something) == spore::Outcome::Burst;
        }
        55 => {
            // The Brain's position is threaded in through ai[2..3] by the server, which knows
            // where every NPC is; a creeper with no Brain removes itself.
            let brain = (npc.ai[2] != 0.0 || npc.ai[3] != 0.0).then_some((npc.ai[2], npc.ai[3]));
            let charging = rand::Rng::random_ratio(rng, 1, creeper::CHARGE_CHANCE);
            effects.expired =
                creeper::update(npc, brain, target, charging) == creeper::Outcome::BrainGone;
        }
        style => unreachable!("style {style} claims parity but has no routine here"),
    }
    effects
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;
    use terrustia_proto::{npc_data::npc_stats, prehardmode::PRE_HARDMODE};

    #[derive(Default)]
    struct Flat(HashMap<(i32, i32), Tile>);

    impl TileView for Flat {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    /// A claimed port has to be a wired one.
    ///
    /// Style 24 spent a while marked [`Parity::Ported`] while the dispatch still sent birds to the
    /// old approximation, which is exactly the failure the parity table exists to prevent. Running
    /// every ported type through [`run`] makes the claim and the wiring the same edit: the final
    /// arm of that match is `unreachable!`, so a style marked ported without a routine panics here.
    #[test]
    fn every_ported_style_actually_runs_its_routine() {
        let mut tiles = Flat::default();
        for x in 0..400 {
            for y in 700..710 {
                tiles.0.insert((x, y), Tile::block(1));
            }
        }
        let mut rng = rand::rngs::SmallRng::seed_from_u64(1);
        let mut seen = Vec::new();
        for npc_type in PRE_HARDMODE {
            let stats = npc_stats(npc_type).expect("stats");
            if parity(stats.ai_style) != Some(Parity::Ported) {
                continue;
            }
            let mut npc = Npc::new(npc_type, (3000.0, 11_100.0), 1).expect("spawnable");
            let world = World {
                tiles: &tiles,
                target: Some(Target {
                    slot: 0,
                    center: (3200.0, 11_100.0),
                    velocity: (0.0, 0.0),
                    alive: true,
                }),
                wet: false,
                target_wet: false,
                conditions: Conditions::default(),
                was_hurt: false,
                target_velocity: (0.0, 0.0),
                census: &[],
                parent: None,
                parent_state: 0.0,
                parent_health: 1.0,
                crowding: (0.0, 0.0),
                avoid: &[],
            };
            // Panics here rather than silently doing nothing, which is the point.
            let _ = run(&mut npc, &world, &mut rng);
            seen.push(stats.ai_style);
        }
        seen.sort_unstable();
        seen.dedup();
        assert!(!seen.is_empty());
    }

    #[test]
    fn every_pre_hardmode_style_has_some_implementation() {
        let mut missing = Vec::new();
        for npc_type in PRE_HARDMODE {
            let stats = npc_stats(npc_type).expect("roster types all have stats");
            if parity(stats.ai_style).is_none() {
                missing.push((npc_type, stats.name, stats.ai_style));
            }
        }
        assert!(missing.is_empty(), "no behaviour at all for: {missing:?}");
    }

    /// Every type in the roster runs a routine transcribed from the game, not an approximation.
    ///
    /// This is the goal the whole `ai` module exists to reach, so it is an assertion rather than a
    /// report: anything that slips back to [`Parity::Approximate`] fails here by name.
    #[test]
    fn every_roster_type_is_a_port_rather_than_an_approximation() {
        let mut approximate = Vec::new();
        for npc_type in PRE_HARDMODE {
            let stats = npc_stats(npc_type).expect("stats");
            if parity(stats.ai_style) != Some(Parity::Ported) {
                approximate.push((npc_type, stats.name, stats.ai_style));
            }
        }
        assert!(
            approximate.is_empty(),
            "still approximations: {approximate:?}"
        );
    }

    /// How much of the *whole* game is ported, not just what appears before hardmode.
    #[test]
    fn report_full_game_progress() {
        use terrustia_proto::npc_data::{NPC_COUNT, npc_stats};
        let (mut done, mut left) = (0, 0);
        let mut missing_styles = std::collections::BTreeSet::new();
        for npc_type in 0..NPC_COUNT {
            let Some(stats) = npc_stats(npc_type) else {
                continue;
            };
            if parity(stats.ai_style) == Some(Parity::Ported) {
                done += 1;
            } else {
                left += 1;
                missing_styles.insert(stats.ai_style);
            }
        }
        println!(
            "whole game: {done} of {} types ported, {left} left across {} styles",
            done + left,
            missing_styles.len()
        );
        println!("styles left: {missing_styles:?}");
    }

    /// Prints how far the port has got, for the record.
    #[test]
    fn report_parity_progress() {
        let (mut ported, mut approx) = (0, 0);
        for npc_type in PRE_HARDMODE {
            let stats = npc_stats(npc_type).expect("stats");
            match parity(stats.ai_style) {
                Some(Parity::Ported) => ported += 1,
                Some(Parity::Approximate) => approx += 1,
                None => {}
            }
        }
        println!(
            "parity: {ported} of {} roster types ported, {approx} still approximate",
            PRE_HARDMODE.len()
        );
        assert_eq!(ported + approx, PRE_HARDMODE.len());
    }
}
