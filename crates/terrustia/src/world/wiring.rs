//! Wire circuits: what happens when a lever is pulled.
//!
//! A circuit is not a graph anybody builds — it is whatever happens to be connected. Hitting a
//! switch floods outward along wire of that colour, one tile at a time, and every tile the flood
//! touches is *acted on*. That is why a wire laid carelessly across somebody else's contraption
//! joins the two together: there is no notion of a separate circuit, only of connectedness.
//!
//! Four colours run independently, so the same tile can be on four circuits at once and do
//! something different on each.
//!
//! What a tile does when the current reaches it is a table sixty-odd entries long in the game.
//! The ones that change what the world *is*, rather than what it looks like, are here:
//!
//! * **Actuators**, which toggle their block between solid and passable.
//! * **Traps** — darts, flames, spears, spiky balls and geysers — which hurt.
//! * **Statues**, which produce monsters, items or a fetched townsperson.
//! * **Teleporters**, which swap whoever is standing on one pad with whoever is on the other.
//! * **Pumps**, which move liquid from every inlet cell a circuit reaches to every outlet.
//!
//! Only the actuator and the pump are done inside the flood, because they are the two that need
//! nothing but the tiles. The rest are *reported*: firing a trap needs a die roll, a cooldown and
//! the projectile store; a statue needs the NPC table; a teleporter needs the players. All of
//! that lives on the server, so the flood hands back which tiles it reached and the server does
//! the work — [`trap_shot`] is the table it calls for each trap.
//!
//! The remaining entries are cosmetic: lamps, candles, chandeliers and the like change a frame
//! and nothing else, and a client does that for itself from the relayed hit.
//!
//! A tile the flood cannot act on still passes the current along, so a circuit through one is not
//! broken by it.

use std::collections::HashSet;

use terrustia_proto::tile::{Tile, TileFlags};

/// The four wire colours, which are four independent circuits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wire {
    Red,
    Blue,
    Green,
    Yellow,
}

impl Wire {
    pub const ALL: [Wire; 4] = [Wire::Red, Wire::Blue, Wire::Green, Wire::Yellow];

    fn flag(self) -> u16 {
        match self {
            Wire::Red => TileFlags::WIRE_RED,
            Wire::Blue => TileFlags::WIRE_BLUE,
            Wire::Green => TileFlags::WIRE_GREEN,
            Wire::Yellow => TileFlags::WIRE_YELLOW,
        }
    }

    /// Whether a tile carries this colour.
    pub fn on(self, tile: Tile) -> bool {
        tile.flags.has(self.flag())
    }
}

/// The tiles a player can hit to start a circuit.
///
/// A lever and a switch remember their state and flip it; a pressure plate and the rest simply
/// fire. Anything else is not a trigger and hitting it does nothing at all.
pub fn is_trigger(block: u16) -> bool {
    matches!(
        block,
        // Switch, lever, and the pressure plates.
        135 | 136 | 144 | 314 | 423 | 428 | 440 | 442 | 476
    )
}

/// Whether hitting this trigger flips a remembered state rather than only firing.
fn flips(block: u16) -> bool {
    matches!(block, 136 | 144)
}

/// How far a trigger's own footprint reaches, since a few are more than one tile.
fn footprint(block: u16) -> (i32, i32) {
    if block == 440 { (3, 3) } else { (1, 1) }
}

/// The most tiles one circuit will touch.
///
/// A circuit is whatever is connected, and what is connected can be the whole world: a player who
/// lays wire across a continent has built a circuit that a server must not spend a whole tick
/// walking. Stopping short is better than stalling — and a circuit this large is not a machine
/// anybody is using, it is a mistake or an attack.
pub const MAX_CIRCUIT: usize = 20_000;

/// What the world has to let a circuit do to it.
pub trait WiredWorld {
    fn tile(&self, x: i32, y: i32) -> Tile;
    fn set_tile(&mut self, x: i32, y: i32, tile: Tile);
    fn width(&self) -> i32;
    fn height(&self) -> i32;
}

/// What a circuit changed.
#[derive(Debug, Default)]
pub struct Fired {
    /// Tiles whose state changed and that clients have to be told about.
    pub changed: Vec<(i32, i32)>,
    /// Trap tiles the current reached, for the caller to resolve into shots.
    pub traps: Vec<(i32, i32)>,
    /// Statues the current reached, by their top-left tile.
    pub statues: Vec<(i32, i32)>,
    /// The first two distinct teleporters the current reached, which are the pair it joins.
    ///
    /// A third makes no difference: the game keeps room for two and ignores the rest, so a
    /// circuit wired through three teleporters links the first two it happens to walk to.
    pub teleporters: Vec<(i32, i32)>,
    /// The cells of every inlet pump the current reached...
    pub pump_in: Vec<(i32, i32)>,
    /// ...and of every outlet.
    pub pump_out: Vec<(i32, i32)>,
    /// How many tiles the current reached, for the record.
    pub reached: usize,
    /// Whether the circuit was cut short by its size cap.
    pub truncated: bool,
}

/// Hit a trigger, and run whatever it is connected to.
///
/// Every colour on the trigger's own tiles runs, each as its own flood, because the four are
/// independent circuits that happen to share a switch.
pub fn hit_switch(world: &mut impl WiredWorld, x: i32, y: i32) -> Fired {
    let mut out = Fired::default();
    let tile = world.tile(x, y);
    if !tile.is_active() || !is_trigger(tile.block) {
        return out;
    }

    // A lever or a switch remembers which way it is thrown.
    if flips(tile.block) {
        let mut flipped = tile;
        flipped.frame_y = if tile.frame_y == 0 { 18 } else { 0 };
        world.set_tile(x, y, flipped);
        out.changed.push((x, y));
    }

    let (w, h) = footprint(tile.block);
    for colour in Wire::ALL {
        let mut seeds = Vec::new();
        for dx in 0..w {
            for dy in 0..h {
                if colour.on(world.tile(x + dx, y + dy)) {
                    seeds.push((x + dx, y + dy));
                }
            }
        }
        if seeds.is_empty() {
            continue;
        }
        trip(world, colour, seeds, &mut out);
    }
    out
}

/// Flood the current outward from a set of seeds and act on everything it reaches.
fn trip(world: &mut impl WiredWorld, colour: Wire, seeds: Vec<(i32, i32)>, out: &mut Fired) {
    let mut seen: HashSet<(i32, i32)> = seeds.iter().copied().collect();
    let mut queue: Vec<(i32, i32)> = seeds;

    while let Some((x, y)) = queue.pop() {
        if seen.len() > MAX_CIRCUIT {
            out.truncated = true;
            break;
        }
        out.reached += 1;
        act(world, x, y, out);

        for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 2 || ny < 2 || nx >= world.width() - 2 || ny >= world.height() - 2 {
                continue;
            }
            if !colour.on(world.tile(nx, ny)) || !seen.insert((nx, ny)) {
                continue;
            }
            queue.push((nx, ny));
        }
    }
}

/// What one tile does when the current reaches it.
///
/// A tile with nothing to do is not an error and does not stop the current: the flood is over
/// connectedness, not over things that respond.
fn act(world: &mut impl WiredWorld, x: i32, y: i32, out: &mut Fired) {
    let tile = world.tile(x, y);
    // An actuator toggles its block between solid and passable. It runs whether or not the block
    // is active, which is the only way a block that has been actuated away can ever come back.
    if tile.flags.has(TileFlags::ACTUATOR) {
        let mut toggled = tile;
        toggled
            .flags
            .set(TileFlags::ACTUATED, !tile.flags.has(TileFlags::ACTUATED));
        world.set_tile(x, y, toggled);
        out.changed.push((x, y));
    }
    if tile.is_active() && matches!(tile.block, TRAPS | GEYSER) {
        out.traps.push((x, y));
    }
    if tile.is_active() && tile.block == TELEPORTER {
        // A teleporter is three tiles wide and the anchor is its left one. Only the first two
        // distinct ones matter: they are the pair the circuit joins.
        let anchor = (x - i32::from(tile.frame_x) / 18, y);
        if out.teleporters.len() < 2 && !out.teleporters.contains(&anchor) {
            out.teleporters.push(anchor);
        }
    }
    if tile.is_active() && matches!(tile.block, PUMP_IN | PUMP_OUT) {
        // A pump is two by two, and all four of its cells take part.
        let anchor = (
            x - {
                let column = i32::from(tile.frame_x) / 18;
                if column > 1 { column - 2 } else { column }
            },
            y - i32::from(tile.frame_y) / 18,
        );
        let cells = [
            (anchor.0, anchor.1 + 1),
            (anchor.0 + 1, anchor.1 + 1),
            anchor,
            (anchor.0 + 1, anchor.1),
        ];
        let side = if tile.block == PUMP_IN {
            &mut out.pump_in
        } else {
            &mut out.pump_out
        };
        for cell in cells {
            if side.len() < PUMP_CELLS && !side.contains(&cell) {
                side.push(cell);
            }
        }
    }
    if tile.is_active() && tile.block == STATUE {
        // A statue is six tiles and the flood reaches all six; what it does belongs to the statue,
        // not to the tile, so it is reported once by its anchor.
        let (_, within) = terrustia_proto::statues::style_at(tile.frame_x, tile.frame_y);
        let anchor = (x - within.0, y - within.1);
        if !out.statues.contains(&anchor) {
            out.statues.push(anchor);
        }
    }
}

/// The tile every dart, flame, spear and spiky-ball trap is a frame of.
const TRAPS: u16 = 137;
/// The geyser, which is its own tile because it is two wide.
const GEYSER: u16 = 443;
/// The tile every statue is a frame of.
const STATUE: u16 = 105;
/// The teleporter, which is three wide.
const TELEPORTER: u16 = 235;
/// The two pumps, which are two by two.
const PUMP_IN: u16 = 142;
const PUMP_OUT: u16 = 143;
/// The most pump cells one circuit run will pull from or push to.
///
/// The game keeps room for twenty and stops at nineteen, so a circuit wired through five pumps
/// only moves water through the first few it reaches.
const PUMP_CELLS: usize = 19;

/// How wide and tall a teleporter's catchment is, in pixels.
///
/// It reaches three tiles up from the teleporter's own row, which is why standing on one works
/// and walking past one at head height does not.
pub const TELEPORTER_BOX: f32 = 48.0;

/// Whether the two ends of a teleporter pair are far enough apart to be worth using.
///
/// Two within three tiles of each other would only shuffle whoever is standing on them, so the
/// game refuses the pair outright.
pub fn teleport_pair_is_useful(a: (i32, i32), b: (i32, i32)) -> bool {
    !(a.0 < b.0 + 3 && a.0 > b.0 - 3 && a.1 > b.1 - 3 && a.1 < b.1)
}

/// Move liquid from a set of inlet cells to a set of outlet cells.
///
/// Each inlet is emptied into the outlets in turn, and only into ones holding the same liquid —
/// an empty outlet takes on whatever arrives. A pump cannot mix water into lava; it simply skips
/// the outlets that would.
///
/// Returns the cells that changed, for the caller to broadcast and re-settle.
pub fn transfer_liquid(
    world: &mut impl WiredWorld,
    inlets: &[(i32, i32)],
    outlets: &[(i32, i32)],
) -> Vec<(i32, i32)> {
    let mut changed = Vec::new();
    for &(ix, iy) in inlets {
        let mut source = world.tile(ix, iy);
        if source.liquid == 0 {
            continue;
        }
        let kind = source.liquid_kind;
        let mut moved_any = false;
        for &(ox, oy) in outlets {
            if source.liquid == 0 {
                break;
            }
            let mut sink = world.tile(ox, oy);
            if sink.liquid == u8::MAX {
                continue;
            }
            // An empty outlet takes on whatever it is given; a full-enough one only accepts more
            // of what it already holds.
            if sink.liquid != 0 && sink.liquid_kind != kind {
                continue;
            }
            let room = u8::MAX - sink.liquid;
            let amount = source.liquid.min(room);
            if amount == 0 {
                continue;
            }
            sink.liquid += amount;
            sink.liquid_kind = kind;
            source.liquid -= amount;
            world.set_tile(ox, oy, sink);
            changed.push((ox, oy));
            moved_any = true;
        }
        if moved_any {
            if source.liquid == 0 {
                source.liquid_kind = terrustia_proto::tile::Liquid::Water;
            }
            world.set_tile(ix, iy, source);
            changed.push((ix, iy));
        }
    }
    changed
}

/// A shot a wired trap wants to take.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    pub projectile_type: u16,
    /// Where the projectile appears, in world pixels.
    pub position: (f32, f32),
    pub velocity: (f32, f32),
    pub damage: i32,
    /// How long this tile must wait before it can fire again.
    pub cooldown: i32,
    /// Where the cooldown is recorded, which is not always the tile that fired: a geyser is two
    /// tiles wide and both halves share one.
    pub cools_at: (i32, i32),
}

/// What a trap tile throws, given its frame.
///
/// The kind is `frame_y / 18` and the direction is in `frame_x`, which is why a trap turned round
/// in the world is turned round here too rather than being a separate tile.
pub fn trap_shot(tile: Tile, x: i32, y: i32, rng: &mut impl rand::Rng) -> Option<Shot> {
    let (px, py) = (x as f32 * 16.0, y as f32 * 16.0);
    if tile.block == GEYSER {
        // The frame's left half tells us which of the two tiles is the anchor, and the top half
        // from the bottom half tells us which way the steam goes.
        let half = i32::from(tile.frame_x) / 36;
        let anchor_x = x - (i32::from(tile.frame_x) - half * 36) / 18;
        let up = half < 2;
        return Some(Shot {
            projectile_type: 654,
            position: (
                (anchor_x + 1) as f32 * 16.0,
                (y + i32::from(!up)) as f32 * 16.0,
            ),
            velocity: (0.0, if up { -8.0 } else { 8.0 }),
            damage: 20,
            cooldown: 200,
            cools_at: (anchor_x, y),
        });
    }

    let kind = i32::from(tile.frame_y) / 18;
    let cools_at = (x, y);
    match kind {
        // The darts and the flame: one tile, aimed by frame_x, ten pixels clear of the muzzle.
        0 | 1 | 2 | 5 => {
            let dx = match tile.frame_x {
                0 => -1,
                18 => 1,
                _ => 0,
            };
            let dy = if tile.frame_x >= 36 {
                if tile.frame_x >= 72 { 1 } else { -1 }
            } else {
                0
            };
            let (projectile_type, damage, speed) = match kind {
                0 => (98u16, 20, 12.0),
                1 => (184, 40, 12.0),
                2 => (187, 40, 5.0),
                _ => (980, 30, 12.0),
            };
            Some(Shot {
                projectile_type,
                position: (px + 8.0 + 10.0 * dx as f32, py + 8.0 + 10.0 * dy as f32),
                velocity: (dx as f32 * speed, dy as f32 * speed),
                damage,
                cooldown: 200,
                cools_at,
            })
        }
        // The spiky ball, which is thrown with a spread rather than aimed.
        3 => {
            let (dx, dy) = trap_facing(tile.frame_x);
            let mut spread = |d: i32| {
                let low = -20 + if d == 1 { 20 } else { 0 };
                let high = 21 - if d == -1 { 20 } else { 0 };
                4.0 * d as f32 + rng.random_range(low..high) as f32 * 0.05
            };
            Some(Shot {
                projectile_type: 185,
                position: (px + 8.0 + 14.0 * dx as f32, py + 8.0 + 14.0 * dy as f32),
                velocity: (spread(dx), spread(dy)),
                damage: 40,
                cooldown: 300,
                cools_at,
            })
        }
        // The spear, which is the only one that reaches back out of the wall it is set in.
        4 => {
            let (dx, dy) = trap_facing(tile.frame_x);
            Some(Shot {
                projectile_type: 186,
                position: (px + 8.0 + 18.0 * dx as f32, py + 8.0 + 18.0 * dy as f32),
                velocity: (8.0 * dx as f32, 8.0 * dy as f32),
                damage: 60,
                cooldown: 90,
                cools_at,
            })
        }
        _ => None,
    }
}

/// Which way a spiky-ball or spear trap points, which it states differently from the darts.
fn trap_facing(frame_x: i16) -> (i32, i32) {
    match frame_x / 18 {
        0 | 1 => (0, 1),
        2 => (0, -1),
        3 => (-1, 0),
        4 => (1, 0),
        _ => (0, 0),
    }
}

/// Whether another spiky ball is welcome, given how far away the ones already out are.
///
/// The game keeps a budget of two hundred and charges each ball against it on a sliding scale —
/// fifty for one within fifty pixels, one for one nearly a screen away. It is what stops a plate
/// held down by a slime from filling a corridor with several hundred of them.
pub fn spiky_ball_allowed(distances: impl Iterator<Item = f32>) -> bool {
    let mut budget = 200i32;
    for d in distances {
        budget -= match d {
            d if d < 50.0 => 50,
            d if d < 100.0 => 15,
            d if d < 200.0 => 10,
            d if d < 300.0 => 8,
            d if d < 400.0 => 6,
            d if d < 500.0 => 5,
            d if d < 700.0 => 4,
            d if d < 900.0 => 3,
            d if d < 1200.0 => 2,
            _ => 1,
        };
    }
    budget > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashMap;

    struct Board(HashMap<(i32, i32), Tile>);

    impl WiredWorld for Board {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
        fn set_tile(&mut self, x: i32, y: i32, tile: Tile) {
            self.0.insert((x, y), tile);
        }
        fn width(&self) -> i32 {
            500
        }
        fn height(&self) -> i32 {
            500
        }
    }

    fn wired(block: u16, colour: Wire) -> Tile {
        let mut tile = if terrustia_proto::tile_sets::frame_important(block) {
            Tile::framed(block, 0, 0)
        } else {
            Tile::block(block)
        };
        tile.flags.set(colour.flag(), true);
        tile
    }

    fn actuated(colour: Wire) -> Tile {
        let mut tile = Tile::block(1);
        tile.flags.set(colour.flag(), true);
        tile.flags.set(TileFlags::ACTUATOR, true);
        tile
    }

    /// A lever wired to an actuator toggles the block, and toggles it back.
    #[test]
    fn a_lever_actuates_a_block() {
        let mut board = Board(HashMap::new());
        // A lever at 100,100 with wire running to an actuated block at 105,100.
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(105, 100, actuated(Wire::Red));

        assert!(!board.tile(105, 100).flags.has(TileFlags::ACTUATED));
        let fired = hit_switch(&mut board, 100, 100);
        assert!(fired.reached >= 6, "the current should have run the length");
        assert!(
            board.tile(105, 100).flags.has(TileFlags::ACTUATED),
            "the block should have been actuated away"
        );

        hit_switch(&mut board, 100, 100);
        assert!(
            !board.tile(105, 100).flags.has(TileFlags::ACTUATED),
            "and back again"
        );
    }

    /// A lever remembers which way it is thrown.
    #[test]
    fn a_lever_flips() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        assert_eq!(board.tile(100, 100).frame_y, 0);
        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(100, 100).frame_y, 18, "thrown");
        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(100, 100).frame_y, 0, "and thrown back");
    }

    /// A pressure plate fires without remembering anything.
    #[test]
    fn a_plate_does_not_flip() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(135, Wire::Red));
        board.set_tile(101, 100, actuated(Wire::Red));
        let before = board.tile(100, 100).frame_y;
        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(100, 100).frame_y, before, "a plate has no state");
        assert!(board.tile(101, 100).flags.has(TileFlags::ACTUATED));
    }

    /// The four colours are four circuits: red does not run what blue is wired to.
    #[test]
    fn the_colours_are_separate_circuits() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        // A red run to one block, a blue run to another, neither touching the other's wire.
        for x in 101..104 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(104, 100, actuated(Wire::Red));
        for x in 101..104 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_BLUE, true);
            board.set_tile(x, 105, wire);
        }
        board.set_tile(104, 105, actuated(Wire::Blue));

        hit_switch(&mut board, 100, 100);
        assert!(
            board.tile(104, 100).flags.has(TileFlags::ACTUATED),
            "the red circuit ran"
        );
        assert!(
            !board.tile(104, 105).flags.has(TileFlags::ACTUATED),
            "and the blue one, which the lever is not on, did not"
        );
    }

    /// ...but a switch carrying two colours runs both.
    #[test]
    fn a_switch_on_two_colours_runs_both() {
        let mut board = Board(HashMap::new());
        let mut lever = wired(136, Wire::Red);
        lever.flags.set(TileFlags::WIRE_BLUE, true);
        board.set_tile(100, 100, lever);
        board.set_tile(101, 100, actuated(Wire::Red));
        board.set_tile(100, 101, actuated(Wire::Blue));

        hit_switch(&mut board, 100, 100);
        assert!(board.tile(101, 100).flags.has(TileFlags::ACTUATED), "red");
        assert!(board.tile(100, 101).flags.has(TileFlags::ACTUATED), "blue");
    }

    /// Hitting something that is not a trigger does nothing at all.
    #[test]
    fn only_a_trigger_starts_a_circuit() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(1, Wire::Red));
        board.set_tile(101, 100, actuated(Wire::Red));
        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.reached, 0);
        assert!(!board.tile(101, 100).flags.has(TileFlags::ACTUATED));
    }

    /// A tile the circuit cannot act on still passes the current along.
    #[test]
    fn an_inert_tile_does_not_break_the_circuit() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        // A plain wired stone block in the middle of the run.
        board.set_tile(101, 100, wired(1, Wire::Red));
        board.set_tile(102, 100, actuated(Wire::Red));
        hit_switch(&mut board, 100, 100);
        assert!(
            board.tile(102, 100).flags.has(TileFlags::ACTUATED),
            "the current should have passed through the stone"
        );
    }

    /// A circuit big enough to be a mistake is cut short rather than stalling the tick.
    #[test]
    fn an_enormous_circuit_is_cut_short() {
        let mut board = Board(HashMap::new());
        board.set_tile(2, 100, wired(136, Wire::Red));
        // A wire running most of the width of the test world, folded so it fits.
        let mut laid = 0;
        'lay: for y in 100..400 {
            for x in 3..490 {
                let mut wire = Tile::AIR;
                wire.flags.set(TileFlags::WIRE_RED, true);
                board.set_tile(x, y, wire);
                laid += 1;
                if laid > MAX_CIRCUIT + 5_000 {
                    break 'lay;
                }
            }
            // Join the rows so it is one circuit.
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(489, y + 1, wire);
        }
        let fired = hit_switch(&mut board, 2, 100);
        assert!(fired.truncated, "it should have given up");
        assert!(
            fired.reached <= MAX_CIRCUIT + 1,
            "and stopped near the cap, not {} in",
            fired.reached
        );
    }

    /// A dart trap facing each way throws its dart that way, clear of its own tile.
    #[test]
    fn a_dart_trap_shoots_the_way_it_faces() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(1);
        for (frame_x, want) in [
            (0i16, (-12.0f32, 0.0f32)),
            (18, (12.0, 0.0)),
            (36, (0.0, -12.0)),
            (72, (0.0, 12.0)),
        ] {
            let shot = trap_shot(Tile::framed(TRAPS, frame_x, 0), 100, 200, &mut rng)
                .expect("a dart trap fires");
            assert_eq!(shot.projectile_type, 98, "frame_x {frame_x}");
            assert_eq!(shot.velocity, want, "frame_x {frame_x}");
            assert_eq!(shot.damage, 20);
            assert_eq!(shot.cooldown, 200);
            // Ten pixels clear of the tile centre, in the direction it is pointing. `signum` is
            // no use here: it calls zero positive.
            let unit = |v: f32| if v == 0.0 { 0.0 } else { v.signum() };
            assert_eq!(
                shot.position,
                (1608.0 + 10.0 * unit(want.0), 3208.0 + 10.0 * unit(want.1)),
                "frame_x {frame_x}"
            );
        }
    }

    /// Each row of the trap tile is a different trap, and they do not share a projectile, a
    /// damage or a cooldown.
    #[test]
    fn every_row_of_the_trap_tile_is_a_different_trap() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(2);
        let seen: Vec<_> = (0..6)
            .filter_map(|row| {
                trap_shot(Tile::framed(TRAPS, 18, row * 18), 50, 60, &mut rng)
                    .map(|s| (s.projectile_type, s.damage, s.cooldown))
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                (98, 20, 200),  // dart
                (184, 40, 200), // poison dart
                (187, 40, 200), // flamethrower
                (185, 40, 300), // spiky ball
                (186, 60, 90),  // spear
                (980, 30, 200), // venom dart
            ]
        );
    }

    /// A spiky ball is thrown with a spread, and only ever away from the trap: the random part
    /// can slow it but never turn it round.
    #[test]
    fn a_spiky_ball_is_thrown_with_a_spread_but_never_backwards() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(3);
        for _ in 0..200 {
            // frame_x 3 * 18 points left.
            let shot = trap_shot(Tile::framed(TRAPS, 54, 54), 10, 10, &mut rng).unwrap();
            assert!(shot.velocity.0 <= -3.0, "went {:?}", shot.velocity);
            assert!(shot.velocity.1.abs() <= 1.0, "wandered {:?}", shot.velocity);
        }
    }

    /// A geyser is two tiles wide and both halves cool down together, or one plate would fire it
    /// twice.
    #[test]
    fn both_halves_of_a_geyser_share_one_cooldown() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(4);
        let left = trap_shot(Tile::framed(GEYSER, 0, 0), 300, 400, &mut rng).unwrap();
        let right = trap_shot(Tile::framed(GEYSER, 18, 0), 301, 400, &mut rng).unwrap();
        assert_eq!(left.cools_at, right.cools_at);
        assert_eq!(left.cools_at, (300, 400));
        assert_eq!(left.velocity, (0.0, -8.0), "the top half blows upward");

        let below = trap_shot(Tile::framed(GEYSER, 72, 0), 300, 400, &mut rng).unwrap();
        assert_eq!(below.velocity, (0.0, 8.0), "the bottom half blows down");
    }

    /// The spiky-ball budget is spent fastest by the ones nearest the trap.
    #[test]
    fn spiky_balls_are_rationed_by_how_close_the_others_are() {
        assert!(spiky_ball_allowed(std::iter::empty()));
        assert!(
            spiky_ball_allowed(std::iter::repeat_n(2000.0, 100)),
            "a hundred of them across the map is still fine"
        );
        assert!(
            !spiky_ball_allowed(std::iter::repeat_n(10.0, 4)),
            "four underfoot is not"
        );
    }

    /// A trap the current reaches is reported rather than fired, because firing it needs a die
    /// roll and a cooldown the flood knows nothing about.
    #[test]
    fn the_flood_reports_traps_instead_of_firing_them() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(105, 100, wired(TRAPS, Wire::Red));

        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.traps, vec![(105, 100)]);
        // And the tile is untouched: a trap is not a thing the flood changes.
        assert_eq!(board.tile(105, 100).block, TRAPS);
    }

    /// A pump moves what the inlet holds into the outlet, up to what the outlet can take.
    #[test]
    fn a_pump_moves_liquid_from_inlet_to_outlet() {
        let mut board = Board(HashMap::new());
        let mut full = Tile::AIR;
        full.liquid = 200;
        full.liquid_kind = terrustia_proto::tile::Liquid::Water;
        board.set_tile(10, 10, full);

        let changed = transfer_liquid(&mut board, &[(10, 10)], &[(50, 50)]);
        assert_eq!(board.tile(10, 10).liquid, 0, "the inlet emptied");
        assert_eq!(board.tile(50, 50).liquid, 200, "and the outlet filled");
        assert_eq!(
            board.tile(50, 50).liquid_kind,
            terrustia_proto::tile::Liquid::Water,
            "an empty outlet takes on what arrives"
        );
        assert!(changed.contains(&(10, 10)) && changed.contains(&(50, 50)));
    }

    /// A pump will not mix water into lava: it skips the outlets that would and keeps the rest.
    #[test]
    fn a_pump_refuses_to_mix_liquids() {
        let mut board = Board(HashMap::new());
        let mut water = Tile::AIR;
        water.liquid = 100;
        water.liquid_kind = terrustia_proto::tile::Liquid::Water;
        board.set_tile(10, 10, water);
        let mut lava = Tile::AIR;
        lava.liquid = 50;
        lava.liquid_kind = terrustia_proto::tile::Liquid::Lava;
        board.set_tile(50, 50, lava);

        transfer_liquid(&mut board, &[(10, 10)], &[(50, 50)]);
        assert_eq!(board.tile(10, 10).liquid, 100, "nothing moved");
        assert_eq!(board.tile(50, 50).liquid, 50);
        assert_eq!(
            board.tile(50, 50).liquid_kind,
            terrustia_proto::tile::Liquid::Lava
        );
    }

    /// An outlet takes only what it has room for, and the inlet keeps the rest for the next one.
    #[test]
    fn a_pump_fills_its_outlets_in_turn() {
        let mut board = Board(HashMap::new());
        let mut full = Tile::AIR;
        full.liquid = 255;
        board.set_tile(10, 10, full);
        let mut nearly = Tile::AIR;
        nearly.liquid = 200;
        board.set_tile(50, 50, nearly);

        transfer_liquid(&mut board, &[(10, 10)], &[(50, 50), (51, 50)]);
        assert_eq!(
            board.tile(50, 50).liquid,
            255,
            "the first filled to the brim"
        );
        assert_eq!(
            board.tile(51, 50).liquid,
            200,
            "and the rest went to the next"
        );
        assert_eq!(board.tile(10, 10).liquid, 0);
    }

    /// A teleporter pair three tiles apart would only shuffle whoever is standing on it, so the
    /// game refuses it outright.
    #[test]
    fn a_teleporter_pair_has_to_go_somewhere() {
        assert!(teleport_pair_is_useful((100, 100), (400, 100)));
        assert!(teleport_pair_is_useful((100, 100), (100, 400)));
        assert!(!teleport_pair_is_useful((101, 100), (100, 101)));
    }

    /// The flood reports the first two teleporters it reaches, and only two: a third is ignored
    /// rather than replacing one of the pair.
    #[test]
    fn the_flood_pairs_the_first_two_teleporters() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..140 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        for x in [110i32, 120, 130] {
            let mut pad = wired(TELEPORTER, Wire::Red);
            pad.frame_x = 0;
            board.set_tile(x, 100, pad);
        }

        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.teleporters.len(), 2, "only two make a pair");
        assert!(
            fired
                .teleporters
                .iter()
                .all(|p| [110, 120, 130].contains(&p.0))
        );
    }
}
