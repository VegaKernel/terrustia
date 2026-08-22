//! Projectiles: the things NPCs throw, and what happens to them afterwards.
//!
//! A projectile is a much simpler entity than an NPC — no targeting, no state machine, usually no
//! decisions at all — but it is the half of combat the server was missing. Every routine that
//! decided to shoot has been emitting its aim and cadence for a while; this is what makes those
//! decisions land.
//!
//! Three behaviours cover everything the pre-hardmode roster fires:
//!
//! * **Style 1**, the arc: it flies straight for a quarter of a second and then starts falling, a
//!   tenth of a pixel a tick, capped at sixteen. Feathers, stingers, snowballs and skulls.
//! * **Style 10**, the lob: it falls from the first tick and sticks where it lands.
//! * **Style 18**, the scythe: it spins, and between its thirtieth and hundredth tick it
//!   *accelerates* by six per cent a tick — which is why a demon's scythe is harmless when it
//!   leaves and lethal by the time it reaches you.

use terrustia_proto::projectile::{MAX_PROJECTILES, ProjectileKey, SERVER_OWNER};
use terrustia_proto::projectile_data::{ProjectileStats, projectile_stats};
use terrustia_proto::tile_solid::{solid, solid_top};

use super::npc::{TILE, TileView};

/// Terminal speed for anything that falls.
const TERMINAL: f32 = 16.0;
/// How long a style-1 projectile flies flat before gravity takes it.
const ARC_DELAY: f32 = 15.0;
/// ...and how hard it then falls.
const ARC_GRAVITY: f32 = 0.1;
/// The scythe's acceleration window and rate.
const SCYTHE_FROM: f32 = 30.0;
const SCYTHE_UNTIL: f32 = 100.0;
const SCYTHE_ACCEL: f32 = 1.06;
/// ...and how fast it spins.
const SCYTHE_SPIN: f32 = 0.8;

/// One projectile in flight.
#[derive(Debug, Clone, Copy)]
pub struct Projectile {
    pub key: ProjectileKey,
    pub projectile_type: u16,
    pub position: (f32, f32),
    pub velocity: (f32, f32),
    pub damage: i32,
    pub knockback: f32,
    pub ai: [f32; 3],
    pub rotation: f32,
    pub time_left: i32,
    /// How many more things it can hit. -1 means no limit.
    pub penetrate: i32,
    pub stats: ProjectileStats,
    /// Set whenever clients need telling about it.
    pub dirty: bool,
}

impl Projectile {
    pub fn width(&self) -> f32 {
        self.stats.width as f32
    }

    pub fn height(&self) -> f32 {
        self.stats.height as f32
    }

    pub fn center(&self) -> (f32, f32) {
        (
            self.position.0 + self.width() / 2.0,
            self.position.1 + self.height() / 2.0,
        )
    }

    /// Whether this box overlaps another.
    pub fn overlaps(&self, position: (f32, f32), size: (f32, f32)) -> bool {
        self.position.0 < position.0 + size.0
            && self.position.0 + self.width() > position.0
            && self.position.1 < position.1 + size.1
            && self.position.1 + self.height() > position.1
    }
}

/// Whether a tile stops a projectile.
fn blocking(tiles: &impl TileView, x: i32, y: i32) -> bool {
    let tile = tiles.tile(x, y);
    tile.is_active() && solid(tile.block) && !solid_top(tile.block)
}

/// Whether a box overlaps anything solid.
fn hits_terrain(tiles: &impl TileView, position: (f32, f32), size: (f32, f32)) -> bool {
    let left = (position.0 / TILE).floor() as i32;
    let right = ((position.0 + size.0 - 1.0) / TILE).floor() as i32;
    let top = (position.1 / TILE).floor() as i32;
    let bottom = ((position.1 + size.1 - 1.0) / TILE).floor() as i32;
    (left..=right).any(|x| (top..=bottom).any(|y| blocking(tiles, x, y)))
}

/// What a projectile's tick concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Flying,
    /// It hit something, ran out of time, or left the world.
    Spent,
}

/// Drive one projectile for a tick.
pub fn step(projectile: &mut Projectile, tiles: &impl TileView) -> Outcome {
    projectile.time_left -= 1;
    if projectile.time_left <= 0 {
        return Outcome::Spent;
    }

    // Fast projectiles move in several smaller steps, which is what keeps them from tunnelling
    // through a one-tile wall.
    let steps = projectile.stats.extra_updates + 1;
    for _ in 0..steps {
        match projectile.stats.ai_style {
            1 => {
                projectile.ai[0] += 1.0;
                if projectile.ai[0] >= ARC_DELAY {
                    projectile.velocity.1 = (projectile.velocity.1 + ARC_GRAVITY).min(TERMINAL);
                }
                projectile.rotation = projectile.velocity.1.atan2(projectile.velocity.0) + 1.57;
            }
            10 => {
                // A lob: it falls from the moment it leaves, and slows as it goes.
                projectile.velocity.1 = (projectile.velocity.1 + 0.41).min(TERMINAL);
                projectile.velocity.0 *= 0.98;
                projectile.rotation += 0.1;
            }
            18 => {
                projectile.rotation += SCYTHE_SPIN;
                projectile.ai[0] += 1.0;
                // The window that makes a demon scythe frightening.
                if (SCYTHE_FROM..SCYTHE_UNTIL).contains(&projectile.ai[0]) {
                    projectile.velocity.0 *= SCYTHE_ACCEL;
                    projectile.velocity.1 *= SCYTHE_ACCEL;
                }
            }
            _ => {
                // Everything else flies straight and simply faces the way it is going.
                projectile.rotation = projectile.velocity.1.atan2(projectile.velocity.0) + 1.57;
            }
        }

        let next = (
            projectile.position.0 + projectile.velocity.0 / steps as f32,
            projectile.position.1 + projectile.velocity.1 / steps as f32,
        );
        if projectile.stats.tile_collide
            && hits_terrain(tiles, next, (projectile.width(), projectile.height()))
        {
            return Outcome::Spent;
        }
        projectile.position = next;
    }

    projectile.dirty = true;
    Outcome::Flying
}

/// The fixed table of projectile slots.
#[derive(Debug)]
pub struct ProjectileStore {
    slots: Vec<Option<Projectile>>,
    /// Incremented per launch, so a reused slot carries a fresh generation.
    next_generation: u16,
}

impl Default for ProjectileStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectileStore {
    pub fn new() -> Self {
        Self {
            slots: (0..MAX_PROJECTILES).map(|_| None).collect(),
            next_generation: 1,
        }
    }

    /// Launch one, returning its slot.
    pub fn launch(
        &mut self,
        projectile_type: u16,
        position: (f32, f32),
        velocity: (f32, f32),
        damage: i32,
        time_left: i32,
    ) -> Option<u16> {
        let stats = projectile_stats(projectile_type)?;
        let index = self.slots.iter().position(Option::is_none)?;
        self.next_generation = self.next_generation.wrapping_add(1) & 0x3FFF;
        let projectile = Projectile {
            key: ProjectileKey {
                owner: SERVER_OWNER,
                index: index as u16,
                generation: self.next_generation,
            },
            projectile_type,
            // The aim point is the centre; the entity is placed by its corner.
            position: (
                position.0 - stats.width as f32 / 2.0,
                position.1 - stats.height as f32 / 2.0,
            ),
            velocity,
            damage,
            knockback: stats.knockback,
            ai: [0.0; 3],
            rotation: 0.0,
            time_left: if time_left > 0 {
                time_left
            } else {
                stats.time_left
            },
            penetrate: stats.penetrate,
            stats,
            dirty: true,
        };
        self.slots[index] = Some(projectile);
        Some(index as u16)
    }

    pub fn get(&self, index: u16) -> Option<&Projectile> {
        self.slots.get(index as usize)?.as_ref()
    }

    pub fn get_mut(&mut self, index: u16) -> Option<&mut Projectile> {
        self.slots.get_mut(index as usize)?.as_mut()
    }

    pub fn remove(&mut self, index: u16) -> Option<Projectile> {
        self.slots.get_mut(index as usize)?.take()
    }

    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = (u16, &Projectile)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.as_ref().map(|p| (i as u16, p)))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u16, &mut Projectile)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(i, p)| p.as_mut().map(|p| (i as u16, p)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Air(HashMap<(i32, i32), Tile>);

    impl TileView for Air {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn launched(projectile_type: u16, velocity: (f32, f32)) -> Projectile {
        let mut store = ProjectileStore::new();
        let index = store
            .launch(projectile_type, (1000.0, 1000.0), velocity, 15, 0)
            .expect("a known type");
        *store.get(index).unwrap()
    }

    #[test]
    fn a_launch_gets_a_slot_and_a_fresh_generation() {
        let mut store = ProjectileStore::new();
        let first = store.launch(38, (0.0, 0.0), (6.0, 0.0), 15, 0).unwrap();
        let a = store.get(first).unwrap().key;
        store.remove(first);
        let second = store.launch(38, (0.0, 0.0), (6.0, 0.0), 15, 0).unwrap();
        let b = store.get(second).unwrap().key;
        assert_eq!(a.index, b.index, "the slot is reused");
        assert_ne!(
            a.generation, b.generation,
            "but a stale kill packet must not match it"
        );
    }

    #[test]
    fn a_launch_is_centred_on_its_aim_point() {
        let p = launched(38, (6.0, 0.0));
        let centre = p.center();
        assert!((centre.0 - 1000.0).abs() < 0.01);
        assert!((centre.1 - 1000.0).abs() < 0.01);
    }

    /// The arc is the whole character of a thrown feather: flat, then falling.
    #[test]
    fn a_feather_flies_flat_and_then_falls() {
        let tiles = Air::default();
        let mut p = launched(38, (6.0, 0.0));
        for _ in 0..(ARC_DELAY as i32 - 1) {
            step(&mut p, &tiles);
        }
        assert_eq!(p.velocity.1, 0.0, "still flat");
        for _ in 0..30 {
            step(&mut p, &tiles);
        }
        assert!(p.velocity.1 > 0.0, "now falling, got {}", p.velocity.1);
    }

    #[test]
    fn nothing_falls_faster_than_terminal() {
        let tiles = Air::default();
        let mut p = launched(38, (0.0, 0.0));
        for _ in 0..2000 {
            if step(&mut p, &tiles) == Outcome::Spent {
                break;
            }
        }
        assert!(p.velocity.1 <= TERMINAL);
    }

    /// A demon's scythe is slow when it leaves and lethal when it arrives.
    #[test]
    fn a_demon_scythe_speeds_up_on_the_way_to_you() {
        let tiles = Air::default();
        let mut p = launched(44, (0.2, 0.0));
        let leaving = p.velocity.0.abs();
        for _ in 0..(SCYTHE_UNTIL as i32) {
            step(&mut p, &tiles);
        }
        let arriving = p.velocity.0.abs();
        assert!(
            arriving > leaving * 10.0,
            "should have picked up speed: {leaving} to {arriving}"
        );
    }

    #[test]
    fn a_scythe_stops_accelerating_eventually() {
        let tiles = Air::default();
        let mut p = launched(44, (0.2, 0.0));
        for _ in 0..(SCYTHE_UNTIL as i32) {
            step(&mut p, &tiles);
        }
        let settled = p.velocity.0;
        for _ in 0..50 {
            step(&mut p, &tiles);
        }
        assert_eq!(p.velocity.0, settled, "it does not accelerate forever");
    }

    #[test]
    fn terrain_stops_what_it_should_and_not_what_it_should_not() {
        let mut tiles = Air::default();
        for y in 0..200 {
            tiles.0.insert((64, y), Tile::block(1));
        }
        // A feather collides.
        let mut feather = launched(38, (16.0, 0.0));
        let mut stopped = false;
        for _ in 0..40 {
            if step(&mut feather, &tiles) == Outcome::Spent {
                stopped = true;
                break;
            }
        }
        assert!(stopped, "a feather should hit the wall");

        // A skull passes straight through.
        let mut skull = launched(299, (16.0, 0.0));
        for _ in 0..30 {
            assert_eq!(step(&mut skull, &tiles), Outcome::Flying);
        }
    }

    #[test]
    fn a_projectile_runs_out_of_time() {
        let tiles = Air::default();
        let mut p = launched(38, (0.0, 0.0));
        p.time_left = 3;
        assert_eq!(step(&mut p, &tiles), Outcome::Flying);
        assert_eq!(step(&mut p, &tiles), Outcome::Flying);
        assert_eq!(step(&mut p, &tiles), Outcome::Spent);
    }

    /// Fast projectiles move in substeps, which is what stops them tunnelling.
    #[test]
    fn a_fast_projectile_cannot_tunnel_through_a_single_tile_wall() {
        let mut tiles = Air::default();
        for y in 0..200 {
            tiles.0.insert((70, y), Tile::block(1));
        }
        // The eye laser has two extra updates and moves nine pixels a step.
        let mut laser = launched(83, (30.0, 0.0));
        assert!(laser.stats.extra_updates > 0);
        let mut stopped = false;
        for _ in 0..40 {
            if step(&mut laser, &tiles) == Outcome::Spent {
                stopped = true;
                break;
            }
        }
        assert!(stopped, "it should not have passed through");
    }

    #[test]
    fn overlap_is_measured_from_the_corners() {
        let p = launched(38, (0.0, 0.0));
        assert!(p.overlaps((1000.0, 1000.0), (4.0, 4.0)));
        assert!(!p.overlaps((2000.0, 2000.0), (4.0, 4.0)));
    }

    #[test]
    fn an_unknown_type_will_not_launch() {
        let mut store = ProjectileStore::new();
        assert!(store.launch(60_000, (0.0, 0.0), (1.0, 0.0), 1, 0).is_none());
    }
}
