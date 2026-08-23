//! NPC entities: state, the fixed slot table, and tile-aware movement.
//!
//! Behaviour lives in [`super::npc_ai`]; this module is what every style shares — gravity,
//! collision against the world, and the bookkeeping the network layer needs.

use terrustia_proto::{
    Tile,
    npc::MAX_NPCS,
    npc_data::{NpcStats, npc_stats},
    tile_solid::{solid, solid_top},
};

/// Downward acceleration, from `NPC.gravity`.
pub const GRAVITY: f32 = 0.3;

/// Terminal speed, from `UpdateNPC_UpdateGravity`.
pub const MAX_FALL_SPEED: f32 = 10.0;

/// One world tile in pixels.
pub const TILE: f32 = 16.0;

/// Ticks an ordinary enemy survives with no player nearby before it despawns.
pub const DEFAULT_TIME_LEFT: i32 = 60 * 60 * 12;

/// Anything the movement code needs to know about the world.
///
/// A trait rather than a direct `World` reference so the physics can be tested against hand-built
/// terrain without standing up a whole world.
pub trait TileView {
    fn tile(&self, x: i32, y: i32) -> Tile;
}

/// A live NPC.
#[derive(Debug, Clone, PartialEq)]
pub struct Npc {
    pub npc_type: u16,
    /// Bumped each time a slot is reused, so a stale hit cannot land on the new occupant.
    pub generation: u8,
    /// Top-left corner, in pixels.
    pub position: (f32, f32),
    pub velocity: (f32, f32),
    pub life: i32,
    pub life_max: i32,
    /// Player slot being chased, or 255 for nobody.
    pub target: u16,
    pub direction: i8,
    pub direction_y: i8,
    pub sprite_direction: i8,
    pub ai: [f32; 4],
    /// The game's `localAI`: four more slots that are never sent over the wire.
    ///
    /// Keeping them separate matters. Several routines use an `ai` slot and a `localAI` slot for
    /// different things at the same time — a snail's `ai[1]` says which way round it is crawling
    /// while its `localAI[3]` counts how long it has been touching nothing — and folding the two
    /// together silently breaks the routine.
    pub local_ai: [f32; 4],
    pub stats: NpcStats,
    /// A lunar pillar's shield: how many of its minions are still unaccounted for.
    ///
    /// Nothing else uses it. It lives on the NPC rather than in a global because a world can hold
    /// four pillars at once and each one's shield falls separately.
    pub shield: i32,
    /// A size that is not the type's, for the routines that grow or shrink mid-life.
    ///
    /// `None` means "whatever the type says", which is the case for all but a handful.
    pub size: Option<(f32, f32)>,
    /// Armour as it stands right now, which is not always what the type says: a training dummy
    /// drops its own to zero, and several hardmode enemies harden when they curl up.
    pub defense: i32,
    /// How faded out it is, 0 solid and 255 invisible.
    ///
    /// Mostly a drawing concern, but not only: a pirate ghost with nobody to chase fades and the
    /// end of the fade is what kills it, so the number has to be kept on the server.
    pub alpha: i32,
    /// A multiplier on this one's contact damage, for the routines that hit harder in some phase
    /// than others. One means "whatever the type says".
    pub damage_bonus: f32,
    /// Set while a routine is in a phase that shrugs off knockback regardless of the type's
    /// resistance — a rolling tortoise is not going to be shoved off course.
    pub knockback_immune: bool,
    /// Set while a routine is in a phase that cannot be hurt — an arrival, a burrow, a shell.
    pub invulnerable: bool,
    /// Counts down while no player is near; the NPC despawns at zero.
    pub time_left: i32,
    pub on_ground: bool,
    /// Velocity as it was before this tick's movement.
    ///
    /// Several styles bounce off terrain by reflecting the velocity they *had*, not the zeroed one
    /// collision leaves behind, so it has to be kept.
    pub old_velocity: (f32, f32),
    /// Whether movement was stopped by terrain this tick.
    ///
    /// The game exposes these as `collideX` / `collideY` and several AI styles read them — a
    /// Fungi Spore bursts on either, and the fighter uses them to decide when to jump.
    pub collide_x: bool,
    pub collide_y: bool,
    /// How large this one is, from `SetDefaults`.
    ///
    /// A few routines read it as more than decoration: a hornet's speed is `2 - scale`, so a
    /// bigger one is slower, and its stinger's damage scales with it directly.
    pub scale: f32,
    /// Position as it was before this tick's movement.
    ///
    /// A routine that wants to know whether it actually got anywhere — a snowman checking whether
    /// it is stuck against a wall, a snail checking whether it is still on one — compares against
    /// this rather than trusting its own velocity.
    pub old_position: (f32, f32),
    /// Which way the sprite is turned, in radians.
    ///
    /// Purely visual for most types, but the wheels and worms keep their state in it, so it has to
    /// persist between ticks like anything else.
    pub rotation: f32,
    /// Whether the NPC was hit since its routine last ran.
    ///
    /// The game calls this `justHit`. A perched bird takes off on it, and several routines drop
    /// what they are doing when it is set.
    pub was_hurt: bool,
    /// Set whenever the state changed enough to be worth telling clients about.
    pub dirty: bool,
    /// Tile a town NPC calls home, if it has been given a house.
    pub home: Option<(i32, i32)>,
    /// Whether gravity is off this tick.
    ///
    /// The game keeps `noGravity` as mutable state on the NPC rather than as a property of the
    /// type: a bird has gravity while it is perched and none once it takes off, and its routine
    /// sets the flag both ways every tick. Starts from the type's default.
    pub no_gravity: bool,
    /// Whether terrain is ignored this tick, kept mutable for the same reason.
    pub no_tile_collide: bool,
    /// For a balloon, the slot of whatever is hanging from it.
    ///
    /// Unlike a worm segment, a passenger does not trail: it is held at a fixed point below its
    /// carrier and inherits its velocity outright.
    pub passenger: Option<u8>,
    /// For a boss part, the slot of the boss it belongs to.
    ///
    /// Unlike a worm segment or a balloon's passenger, a part steers itself; it only needs to know
    /// where its parent is and what that parent is doing.
    pub follows_boss: Option<u8>,
    /// For a worm segment, the slot of the segment ahead of it.
    ///
    /// A worm is a chain of separate NPCs; only the head steers, and every other link keeps a
    /// fixed distance behind the one in front.
    pub follows: Option<u8>,
    /// Whether a statue made this one.
    ///
    /// It is worth no coins and takes up no room in the spawn budget, which is the only reason a
    /// statue farm works: without it a wired statue would stop the world spawning anything else.
    pub from_statue: bool,
    /// What is currently burning, poisoning or cursing it.
    ///
    /// Kept on the NPC rather than in a side table because it is read every tick by the routine
    /// that decides damage and written by any client that lands a hit; a lookup either way round
    /// would be a scan of the whole roster.
    pub buffs: super::buffs::Buffs,
    /// Set when the buff list changed and clients have not been told yet.
    ///
    /// Separate from `dirty` because the two go out as different packets: `dirty` sends the
    /// NPC's position and health, this sends its buff list, and a burning enemy standing still
    /// needs only the second.
    pub buffs_dirty: bool,
    /// The personal name a town NPC, pet or slime carries on top of its type.
    ///
    /// Empty for everything else. A client asks for this the moment the NPC comes into view and
    /// shows the type's name until it is answered, so a server that never answers gives you a
    /// town full of people called "Guide".
    pub given_name: String,
    /// Coins this one is carrying beyond what its type is worth.
    ///
    /// The Coin Loss revenge system: money dropped on death is remembered against whatever killed
    /// you, and killing that back gives it up. It accumulates rather than being set, because two
    /// players can both feed the same enemy.
    pub extra_value: i32,
    /// Which of a type's two looks it wears, for the four types that have two.
    ///
    /// The Dryad, the Truffle, the Princess and the Guide each have an alternate; the game keeps
    /// the choice as a number rather than a flag because it is sent alongside the name.
    pub town_variation: i32,
}

impl Npc {
    pub fn new(npc_type: u16, position: (f32, f32), generation: u8) -> Option<Self> {
        let stats = npc_stats(npc_type)?;
        Some(Self {
            npc_type,
            generation,
            position,
            velocity: (0.0, 0.0),
            life: stats.life_max,
            life_max: stats.life_max,
            target: 255,
            direction: 1,
            direction_y: 1,
            sprite_direction: 1,
            ai: [0.0; 4],
            local_ai: [0.0; 4],
            stats,
            shield: 0,
            size: None,
            defense: stats.defense,
            damage_bonus: 1.0,
            knockback_immune: false,
            alpha: 0,
            invulnerable: stats.dont_take_damage,
            time_left: DEFAULT_TIME_LEFT,
            old_velocity: (0.0, 0.0),
            on_ground: false,
            collide_x: false,
            collide_y: false,
            scale: terrustia_proto::npc_params::npc_scale(npc_type),
            old_position: position,
            rotation: 0.0,
            was_hurt: false,
            dirty: true,
            no_gravity: stats.no_gravity,
            no_tile_collide: stats.no_tile_collide,
            home: None,
            passenger: None,
            follows_boss: None,
            follows: None,
            from_statue: false,
            buffs: super::buffs::Buffs::new(),
            buffs_dirty: false,
            given_name: String::new(),
            extra_value: 0,
            town_variation: 0,
        })
    }

    pub fn width(&self) -> f32 {
        self.size.map_or(self.stats.width as f32, |(w, _)| w)
    }

    pub fn height(&self) -> f32 {
        self.size.map_or(self.stats.height as f32, |(_, h)| h)
    }

    /// Change how big this one is, keeping it centred where it already was.
    ///
    /// A few routines resize themselves mid-life and mean it: a chattering teeth bomb swelling to
    /// a hundred and sixty pixels across *is* its blast, because the hitbox is what does the
    /// damage.
    pub fn resize(&mut self, width: f32, height: f32) {
        let (cx, cy) = self.center();
        self.size = Some((width, height));
        self.position = (cx - width / 2.0, cy - height / 2.0);
        self.dirty = true;
    }

    /// Centre of the NPC, which is what the AI aims with.
    pub fn center(&self) -> (f32, f32) {
        (
            self.position.0 + self.width() / 2.0,
            self.position.1 + self.height() / 2.0,
        )
    }

    /// Turn into another type in place, the way the game's `NPC.Transform` does.
    ///
    /// The slot, the position and the generation all survive; everything the type decides — stats,
    /// size, routine — is replaced, and the AI state is cleared so the new routine starts fresh.
    pub fn become_type(&mut self, npc_type: u16) {
        let Some(stats) = npc_stats(npc_type) else {
            return;
        };
        self.npc_type = npc_type;
        self.stats = stats;
        self.life_max = stats.life_max;
        self.life = stats.life_max;
        self.no_gravity = stats.no_gravity;
        self.no_tile_collide = stats.no_tile_collide;
        self.defense = stats.defense;
        self.damage_bonus = 1.0;
        self.knockback_immune = false;
        self.size = None;
        self.invulnerable = stats.dont_take_damage;
        self.alpha = 0;
        self.scale = terrustia_proto::npc_params::npc_scale(npc_type);
        self.ai = [0.0; 4];
        self.local_ai = [0.0; 4];
        self.was_hurt = false;
        self.dirty = true;
    }

    pub fn is_alive(&self) -> bool {
        self.life > 0
    }

    /// Apply a hit, returning true if it killed the NPC.
    pub fn take_damage(&mut self, amount: i32, knockback: f32, direction: i8) -> bool {
        // `dont_take_damage` is the type saying it can never be hurt; `invulnerable` is a routine
        // saying not right now. Either one turns a hit into nothing.
        if self.stats.dont_take_damage || self.invulnerable {
            return false;
        }
        self.life -= amount.max(0);
        self.was_hurt = true;
        self.dirty = true;

        // knockback_resist is a multiplier: 0 means immovable, 1 fully affected. A routine can
        // override it outright while it is committed to a move.
        let resist = if self.knockback_immune {
            0.0
        } else {
            self.stats.knockback_resist
        };
        if resist > 0.0 && knockback > 0.0 {
            self.velocity.0 += f32::from(direction) * knockback * resist;
            self.velocity.1 -= knockback * resist * 0.5;
        }
        self.life <= 0
    }
}

/// Whether the box at `(left, top)` overlaps anything that blocks movement.
///
/// Platforms are skipped here: they are in the solid set but only stop something landing on them
/// from above, which [`move_vertical`] handles separately.
/// Whether a box overlaps anything solid, for an NPC that may pass through some tiles.
///
/// `npc_type` decides what counts as solid: almost every NPC is stopped by everything, but a sand
/// shark swims through sand, so the type has to be part of the question.
fn blocked_for(
    tiles: &impl TileView,
    npc_type: u16,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
) -> bool {
    let x0 = (left / TILE).floor() as i32;
    let x1 = ((left + width - 1.0) / TILE).floor() as i32;
    let y0 = (top / TILE).floor() as i32;
    let y1 = ((top + height - 1.0) / TILE).floor() as i32;

    for y in y0..=y1 {
        for x in x0..=x1 {
            let tile = tiles.tile(x, y);
            if tile.is_active()
                && solid(tile.block)
                && !solid_top(tile.block)
                && !terrustia_proto::npc_params::phases_through(npc_type, tile.block)
            {
                return true;
            }
        }
    }
    false
}

/// Whether a platform's top edge sits within the span an NPC would fall through this step.
fn platform_underfoot(
    tiles: &impl TileView,
    left: f32,
    feet_from: f32,
    feet_to: f32,
    width: f32,
) -> Option<f32> {
    let x0 = (left / TILE).floor() as i32;
    let x1 = ((left + width - 1.0) / TILE).floor() as i32;
    let y0 = (feet_from / TILE).floor() as i32;
    let y1 = (feet_to / TILE).floor() as i32;

    for y in y0..=y1 {
        let top = y as f32 * TILE;
        if top < feet_from - 0.01 || top > feet_to {
            continue;
        }
        for x in x0..=x1 {
            let tile = tiles.tile(x, y);
            if tile.is_active() && solid_top(tile.block) {
                return Some(top);
            }
        }
    }
    None
}

/// Move horizontally, stopping at the first wall.
fn move_horizontal(npc: &mut Npc, tiles: &impl TileView) {
    let next = npc.position.0 + npc.velocity.0;
    if !blocked_for(
        tiles,
        npc.npc_type,
        next,
        npc.position.1,
        npc.width(),
        npc.height(),
    ) {
        npc.position.0 = next;
        return;
    }
    npc.collide_x = true;

    // Step up to the wall a pixel at a time so the NPC ends flush against it rather than short of
    // it, which is what lets the fighter AI decide it is time to jump.
    let step = npc.velocity.0.signum();
    while !blocked_for(
        tiles,
        npc.npc_type,
        npc.position.0 + step,
        npc.position.1,
        npc.width(),
        npc.height(),
    ) {
        npc.position.0 += step;
        if (npc.position.0 - next).abs() < 1.0 {
            break;
        }
    }
    npc.velocity.0 = 0.0;
}

/// Move vertically, landing on solid ground or on a platform fallen onto from above.
fn move_vertical(npc: &mut Npc, tiles: &impl TileView) {
    npc.on_ground = false;
    let next = npc.position.1 + npc.velocity.1;

    if npc.velocity.1 > 0.0 {
        let feet_from = npc.position.1 + npc.height();
        let feet_to = next + npc.height();
        if let Some(top) =
            platform_underfoot(tiles, npc.position.0, feet_from, feet_to, npc.width())
        {
            npc.position.1 = top - npc.height();
            npc.velocity.1 = 0.0;
            npc.on_ground = true;
            return;
        }
    }

    if !blocked_for(
        tiles,
        npc.npc_type,
        npc.position.0,
        next,
        npc.width(),
        npc.height(),
    ) {
        npc.position.1 = next;
        return;
    }
    npc.collide_y = true;

    let step = npc.velocity.1.signum();
    while !blocked_for(
        tiles,
        npc.npc_type,
        npc.position.0,
        npc.position.1 + step,
        npc.width(),
        npc.height(),
    ) {
        npc.position.1 += step;
        if (npc.position.1 - next).abs() < 1.0 {
            break;
        }
    }
    if npc.velocity.1 > 0.0 {
        npc.on_ground = true;
    }
    npc.velocity.1 = 0.0;
}

/// Advance an NPC's position by one tick, applying gravity and collision.
pub fn step_physics(npc: &mut Npc, tiles: &impl TileView) {
    npc.old_position = npc.position;
    npc.old_velocity = npc.velocity;
    npc.collide_x = false;
    npc.collide_y = false;
    if !npc.no_gravity {
        npc.velocity.1 = (npc.velocity.1 + GRAVITY).min(MAX_FALL_SPEED);
    }

    if npc.no_tile_collide {
        npc.position.0 += npc.velocity.0;
        npc.position.1 += npc.velocity.1;
        npc.on_ground = false;
        return;
    }

    move_horizontal(npc, tiles);
    move_vertical(npc, tiles);
}

/// The fixed table of NPC slots.
#[derive(Debug)]
pub struct NpcStore {
    slots: Vec<Option<Npc>>,
    /// Incremented per spawn so reused slots get a fresh generation.
    next_generation: u8,
}

impl Default for NpcStore {
    fn default() -> Self {
        Self::new()
    }
}

impl NpcStore {
    pub fn new() -> Self {
        Self {
            slots: (0..MAX_NPCS).map(|_| None).collect(),
            next_generation: 0,
        }
    }

    /// Spawn a worm: a head, a run of body segments and a tail, each linked to the one ahead.
    ///
    /// Returns the head's slot. Worms are the reason NPC slots are addressed by index on the wire:
    /// the segments have to refer to each other.
    pub fn spawn_worm(
        &mut self,
        head: u16,
        body: u16,
        tail: u16,
        segments: usize,
        position: (f32, f32),
    ) -> Option<u8> {
        let head_index = self.spawn(head, position)?;
        let mut previous = head_index;
        for i in 0..segments {
            let part = if i + 1 == segments { tail } else { body };
            let Some(index) = self.spawn(part, position) else {
                break;
            };
            if let Some(npc) = self.get_mut(index) {
                npc.follows = Some(previous);
            }
            previous = index;
        }
        Some(head_index)
    }

    pub fn spawn(&mut self, npc_type: u16, position: (f32, f32)) -> Option<u8> {
        let index = self.slots.iter().position(Option::is_none)?;
        self.next_generation = self.next_generation.wrapping_add(1);
        let npc = Npc::new(npc_type, position, self.next_generation)?;
        self.slots[index] = Some(npc);
        u8::try_from(index).ok()
    }

    pub fn get(&self, index: u8) -> Option<&Npc> {
        self.slots.get(usize::from(index))?.as_ref()
    }

    pub fn get_mut(&mut self, index: u8) -> Option<&mut Npc> {
        self.slots.get_mut(usize::from(index))?.as_mut()
    }

    pub fn remove(&mut self, index: u8) -> Option<Npc> {
        self.slots.get_mut(usize::from(index))?.take()
    }

    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = (u8, &Npc)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_ref().map(|npc| (i as u8, npc)))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u8, &mut Npc)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_mut().map(|npc| (i as u8, npc)))
    }

    /// Total spawn slots in use, which is what the spawn cap is measured against.
    pub fn used_slots(&self) -> f32 {
        self.slots
            .iter()
            .flatten()
            // A statue's monster costs nothing against the cap, so a farm does not starve the
            // rest of the world of spawns.
            .filter(|npc| !npc.stats.town_npc && !npc.from_statue)
            .map(|npc| npc.stats.npc_slots)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Terrain built from a closure, for testing movement without a world.
    struct Terrain<F>(F);

    impl<F: Fn(i32, i32) -> Option<u16>> TileView for Terrain<F> {
        fn tile(&self, x: i32, y: i32) -> Tile {
            match (self.0)(x, y) {
                // Platforms and other multi-tile types are frame-important, so they have to be
                // built with frames rather than as plain blocks.
                Some(block) if terrustia_proto::tile_sets::frame_important(block) => {
                    Tile::framed(block, 0, 0)
                }
                Some(block) => Tile::block(block),
                None => Tile::AIR,
            }
        }
    }

    /// Solid ground at and below tile row 10.
    fn ground() -> Terrain<impl Fn(i32, i32) -> Option<u16>> {
        Terrain(|_x: i32, y: i32| if y >= 10 { Some(1) } else { None })
    }

    fn zombie_at(x: f32, y: f32) -> Npc {
        Npc::new(3, (x, y), 1).expect("zombie stats")
    }

    #[test]
    fn transforming_keeps_the_slot_and_replaces_everything_the_type_decides() {
        // A lost girl becoming a nymph: same entity, entirely different creature.
        let mut girl = Npc::new(195, (1000.0, 1000.0), 7).expect("lost girl");
        girl.ai = [1.0, 2.0, 3.0, 4.0];
        girl.life = 1;
        let position = girl.position;

        girl.become_type(196);
        assert_eq!(girl.npc_type, 196);
        assert_eq!(girl.stats.name, "Nymph");
        assert_eq!(girl.life, girl.life_max, "and comes back at full health");
        assert_eq!(girl.ai, [0.0; 4], "with a fresh routine");
        assert_eq!(girl.position, position, "but does not move");
        assert_eq!(girl.generation, 7, "and keeps its identity");
    }

    #[test]
    fn an_npc_falls_and_lands_on_the_ground() {
        let mut npc = zombie_at(32.0, 0.0);
        let terrain = ground();
        for _ in 0..200 {
            step_physics(&mut npc, &terrain);
            if npc.on_ground {
                break;
            }
        }
        assert!(npc.on_ground, "never landed");
        // Its feet should rest exactly on the top of row 10.
        assert!(
            (npc.position.1 + npc.height() - 160.0).abs() <= 1.0,
            "rested at {}",
            npc.position.1
        );
        assert_eq!(npc.velocity.1, 0.0);
    }

    #[test]
    fn falling_speed_is_capped() {
        let mut npc = zombie_at(32.0, 0.0);
        let empty = Terrain(|_: i32, _: i32| None);
        for _ in 0..500 {
            step_physics(&mut npc, &empty);
        }
        assert_eq!(npc.velocity.1, MAX_FALL_SPEED);
    }

    #[test]
    fn a_wall_stops_horizontal_movement() {
        // A wall at tile x = 5, with ground below.
        let terrain = Terrain(|x: i32, y: i32| {
            if y >= 10 || (x == 5 && y >= 7) {
                Some(1)
            } else {
                None
            }
        });
        let mut npc = zombie_at(32.0, 100.0);
        npc.velocity.0 = 2.0;
        for _ in 0..200 {
            step_physics(&mut npc, &terrain);
            npc.velocity.0 = 2.0;
        }
        assert!(
            npc.position.0 + npc.width() <= 5.0 * TILE + 1.0,
            "walked into the wall: x={}",
            npc.position.0
        );
    }

    #[test]
    fn a_flying_npc_ignores_gravity() {
        // Eater of Souls has noGravity.
        let mut npc = Npc::new(6, (32.0, 0.0), 1).unwrap();
        assert!(npc.stats.no_gravity);
        let terrain = ground();
        step_physics(&mut npc, &terrain);
        assert_eq!(npc.velocity.1, 0.0, "gravity should not apply");
    }

    #[test]
    fn a_no_tile_collide_npc_passes_through_ground() {
        // Eye of Cthulhu ignores terrain entirely.
        let mut npc = Npc::new(4, (32.0, 100.0), 1).unwrap();
        assert!(npc.stats.no_tile_collide);
        npc.velocity.1 = 5.0;
        let terrain = ground();
        for _ in 0..20 {
            step_physics(&mut npc, &terrain);
        }
        assert!(
            npc.position.1 > 160.0,
            "should have passed through the floor"
        );
    }

    #[test]
    fn a_platform_is_landed_on_but_not_a_wall() {
        // Wood platform (19) across row 10, nothing else.
        let terrain = Terrain(|_x: i32, y: i32| if y == 10 { Some(19) } else { None });

        let mut walker = zombie_at(32.0, 100.0);
        walker.velocity.0 = 2.0;
        for _ in 0..60 {
            step_physics(&mut walker, &terrain);
            walker.velocity.0 = 2.0;
        }
        assert!(walker.on_ground, "should stand on the platform");
        assert!(
            walker.position.0 > 60.0,
            "a platform must not block sideways movement"
        );
    }

    #[test]
    fn knockback_scales_with_resistance() {
        let mut slime = Npc::new(1, (0.0, 0.0), 1).unwrap();
        assert_eq!(slime.stats.knockback_resist, 1.0);
        slime.take_damage(5, 4.0, 1);
        assert!(slime.velocity.0 > 0.0, "a full-resist NPC should be pushed");

        // Eye of Cthulhu has knockback_resist 0 and should not move.
        let mut boss = Npc::new(4, (0.0, 0.0), 1).unwrap();
        assert_eq!(boss.stats.knockback_resist, 0.0);
        boss.take_damage(5, 4.0, 1);
        assert_eq!(boss.velocity.0, 0.0);
    }

    #[test]
    fn damage_kills_when_health_runs_out() {
        let mut slime = Npc::new(1, (0.0, 0.0), 1).unwrap();
        assert!(!slime.take_damage(10, 0.0, 1));
        assert_eq!(slime.life, 15);
        assert!(slime.take_damage(15, 0.0, 1), "should report the kill");
        assert!(!slime.is_alive());
    }

    #[test]
    fn slots_are_reused_with_a_fresh_generation() {
        let mut store = NpcStore::new();
        let first = store.spawn(3, (0.0, 0.0)).unwrap();
        let gen_a = store.get(first).unwrap().generation;
        store.remove(first);

        let second = store.spawn(3, (0.0, 0.0)).unwrap();
        assert_eq!(second, first, "the freed slot is reused");
        assert_ne!(
            store.get(second).unwrap().generation,
            gen_a,
            "a reused slot must not keep its old generation"
        );
    }

    #[test]
    fn spawning_an_unknown_type_fails_rather_than_inventing_one() {
        let mut store = NpcStore::new();
        assert_eq!(store.spawn(u16::MAX, (0.0, 0.0)), None);
        assert!(store.is_empty());
    }

    #[test]
    fn town_npcs_do_not_count_against_the_spawn_cap() {
        let mut store = NpcStore::new();
        store.spawn(22, (0.0, 0.0)); // Guide
        assert_eq!(store.used_slots(), 0.0);
        store.spawn(3, (0.0, 0.0)); // Zombie
        assert_eq!(store.used_slots(), 1.0);
    }
}
