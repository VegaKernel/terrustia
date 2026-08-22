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
//! What a tile does when the current reaches it is a table sixty-odd entries long in the game. The
//! part implemented here is the part this server already models: an actuator toggles the block it
//! is on between solid and passable, which is the one wire effect that changes what the world
//! *is* rather than what it looks like. Traps, statues, teleporters and pumps are left alone —
//! they need projectile spawning from tile frames, the statue spawn tables and a liquid pump model
//! — and a tile the flood cannot act on still passes the current along, so a circuit through one
//! is not broken by it.

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
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
