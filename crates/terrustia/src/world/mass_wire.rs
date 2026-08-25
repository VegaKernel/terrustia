//! The Grand Design: laying and cutting wire along a path rather than one tile at a time.
//!
//! Every wiring tool past the first works this way. A player drags from one tile to another and
//! the server runs the whole line, which is why it is the server's job rather than a stream of
//! single-tile edits — the client does not know how much wire the player has, and a run that
//! stops halfway has to stop at the same tile for everybody.
//!
//! The path is not a straight line between the two points. It is an **L**: all the way along one
//! axis, then all the way along the other, with which axis goes first decided by the direction
//! the player is facing. That is what lets one drag lay a corner, and it is the reason a run
//! looks wrong if you get the order backwards.
//!
//! Running out of wire stops the run *at that tile* and leaves everything before it in place.
//! The player is then told what was actually spent, which is what stops a client believing it
//! still has wire the server has already used.

use terrustia_proto::{Tile, TileFlags};

/// What a wiring tool is set to do. `WiresUI.Settings.MultiToolMode`, a bit set.
///
/// Cutter is not a separate mode but a modifier: with it set the colours say what to *remove*
/// rather than what to lay, which is why one tool does both jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolMode(pub u8);

impl ToolMode {
    pub const RED: u8 = 1;
    pub const GREEN: u8 = 2;
    pub const BLUE: u8 = 4;
    pub const YELLOW: u8 = 8;
    pub const ACTUATOR: u8 = 0x10;
    pub const CUTTER: u8 = 0x20;

    fn has(self, bit: u8) -> bool {
        self.0 & bit != 0
    }

    fn cutting(self) -> bool {
        self.has(Self::CUTTER)
    }

    /// Whether the mode asks for anything at all. An empty one is a client bug or a crafted
    /// packet, and running it would walk the whole path for nothing.
    pub fn does_anything(self) -> bool {
        self.0 & (Self::RED | Self::GREEN | Self::BLUE | Self::YELLOW | Self::ACTUATOR) != 0
    }
}

/// What the tool has to spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Supplies {
    pub wire: i32,
    pub actuators: i32,
}

/// One tile the run changed, and which tile-manipulation action says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub x: i32,
    pub y: i32,
    /// The action byte a tile-manipulation packet carries for this change.
    pub action: u8,
}

/// What a run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub changes: Vec<Change>,
    /// How much wire the run actually used.
    pub wire_spent: i32,
    /// ...and how many actuators.
    pub actuators_spent: i32,
    /// Whether the run stopped early for want of materials.
    pub ran_out: bool,
}

// The tile-manipulation actions, from `MessageBuffer`'s own numbering.
const PLACE_WIRE: u8 = 5;
const KILL_WIRE: u8 = 6;
const PLACE_ACTUATOR: u8 = 8;
const KILL_ACTUATOR: u8 = 9;
const PLACE_WIRE2: u8 = 10;
const KILL_WIRE2: u8 = 11;
const PLACE_WIRE3: u8 = 12;
const KILL_WIRE3: u8 = 13;
const PLACE_WIRE4: u8 = 16;
const KILL_WIRE4: u8 = 17;

/// What the world has to offer the run.
///
/// A trait rather than a `World` reference so the path logic can be tested against a hand-built
/// grid — the corner cases here are all about *which tiles are visited*, which a real world makes
/// no easier to see.
pub trait Tiles {
    fn tile(&self, x: i32, y: i32) -> Tile;
    fn set_tile(&mut self, x: i32, y: i32, tile: Tile);
    fn in_bounds(&self, x: i32, y: i32) -> bool;
}

impl Tiles for crate::world::World {
    fn tile(&self, x: i32, y: i32) -> Tile {
        crate::world::World::tile(self, x, y)
    }
    fn set_tile(&mut self, x: i32, y: i32, tile: Tile) {
        crate::world::World::set_tile(self, x, y, tile);
    }
    fn in_bounds(&self, x: i32, y: i32) -> bool {
        crate::world::World::in_bounds(self, x, y)
    }
}

/// Run a wiring tool from one tile to another.
///
/// `facing_right` decides which leg of the **L** is walked first, exactly as the game reads the
/// player's facing. Two drags between the same pair of points lay different corners depending on
/// which way the player happened to be looking, and that is the intended behaviour rather than
/// an accident.
pub fn run(
    tiles: &mut impl Tiles,
    from: (i32, i32),
    to: (i32, i32),
    mode: ToolMode,
    supplies: Supplies,
    facing_right: bool,
) -> Outcome {
    let mut out = Outcome::default();
    if !mode.does_anything() {
        return out;
    }
    let mut left = supplies;

    let step_x = (to.0 - from.0).signum();
    let step_y = (to.1 - from.1).signum();

    // The first leg runs along one axis from the start; the second along the other, from the
    // corner to the end. Which is which is the player's facing.
    let mut stopped = false;

    {
        // Leg one.
        let (fixed_is_x, start, end, step) = if facing_right {
            (true, from.1, to.1, step_y)
        } else {
            (false, from.0, to.0, step_x)
        };
        let mut i = start;
        while i != end && !stopped {
            let at = if fixed_is_x { (from.0, i) } else { (i, from.1) };
            stopped = !step_at(tiles, at, mode, &mut left, &mut out);
            i += step;
        }
    }

    if !stopped {
        // Leg two, from the corner to the destination.
        let (fixed_is_y, start, end, step) = if facing_right {
            (true, from.0, to.0, step_x)
        } else {
            (false, from.1, to.1, step_y)
        };
        let mut i = start;
        while i != end && !stopped {
            let at = if fixed_is_y { (i, to.1) } else { (to.0, i) };
            stopped = !step_at(tiles, at, mode, &mut left, &mut out);
            i += step;
        }
    }

    // The destination itself, which neither leg reaches: both stop one short of their end.
    if !stopped {
        step_at(tiles, to, mode, &mut left, &mut out);
    }

    out.wire_spent = supplies.wire - left.wire;
    out.actuators_spent = supplies.actuators - left.actuators;
    out.ran_out = stopped;
    out
}

/// Do whatever the mode says at one tile.
///
/// Returns false when the run has to stop — which happens only for want of materials, never for
/// a tile that simply had nothing to do.
fn step_at(
    tiles: &mut impl Tiles,
    (x, y): (i32, i32),
    mode: ToolMode,
    left: &mut Supplies,
    out: &mut Outcome,
) -> bool {
    if !tiles.in_bounds(x, y) {
        // Out of the world is skipped rather than fatal, as the game skips it: a drag that runs
        // off the edge still does its work on the part that was inside.
        return true;
    }

    let mut tile = tiles.tile(x, y);
    let mut touched = false;

    if mode.cutting() {
        for (bit, flag, action) in [
            (ToolMode::RED, TileFlags::WIRE_RED, KILL_WIRE),
            (ToolMode::GREEN, TileFlags::WIRE_GREEN, KILL_WIRE3),
            (ToolMode::BLUE, TileFlags::WIRE_BLUE, KILL_WIRE2),
            (ToolMode::YELLOW, TileFlags::WIRE_YELLOW, KILL_WIRE4),
            (ToolMode::ACTUATOR, TileFlags::ACTUATOR, KILL_ACTUATOR),
        ] {
            if mode.has(bit) && tile.flags.has(flag) {
                tile.flags.set(flag, false);
                out.changes.push(Change { x, y, action });
                touched = true;
            }
        }
        // Cutting gives nothing back, which is why a mistake with the Grand Design is expensive.
    } else {
        for (bit, flag, action) in [
            (ToolMode::RED, TileFlags::WIRE_RED, PLACE_WIRE),
            (ToolMode::GREEN, TileFlags::WIRE_GREEN, PLACE_WIRE3),
            (ToolMode::BLUE, TileFlags::WIRE_BLUE, PLACE_WIRE2),
            (ToolMode::YELLOW, TileFlags::WIRE_YELLOW, PLACE_WIRE4),
        ] {
            if !mode.has(bit) || tile.flags.has(flag) {
                continue;
            }
            if left.wire <= 0 {
                // Out of wire: the run stops here, keeping everything laid before it.
                if touched {
                    tiles.set_tile(x, y, tile);
                }
                return false;
            }
            left.wire -= 1;
            tile.flags.set(flag, true);
            out.changes.push(Change { x, y, action });
            touched = true;
        }
        if mode.has(ToolMode::ACTUATOR) && !tile.flags.has(TileFlags::ACTUATOR) {
            if left.actuators <= 0 {
                if touched {
                    tiles.set_tile(x, y, tile);
                }
                return false;
            }
            left.actuators -= 1;
            tile.flags.set(TileFlags::ACTUATOR, true);
            out.changes.push(Change {
                x,
                y,
                action: PLACE_ACTUATOR,
            });
            touched = true;
        }
    }

    if touched {
        tiles.set_tile(x, y, tile);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Grid {
        tiles: std::collections::HashMap<(i32, i32), Tile>,
        width: i32,
        height: i32,
    }

    impl Grid {
        fn new() -> Self {
            Self {
                tiles: std::collections::HashMap::new(),
                width: 200,
                height: 200,
            }
        }
    }

    impl Tiles for Grid {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.tiles.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
        fn set_tile(&mut self, x: i32, y: i32, tile: Tile) {
            self.tiles.insert((x, y), tile);
        }
        fn in_bounds(&self, x: i32, y: i32) -> bool {
            x >= 0 && y >= 0 && x < self.width && y < self.height
        }
    }

    fn plenty() -> Supplies {
        Supplies {
            wire: 10_000,
            actuators: 10_000,
        }
    }

    /// A straight horizontal drag lays wire on every tile it crosses, ends included.
    #[test]
    fn a_straight_run_wires_every_tile() {
        let mut grid = Grid::new();
        let out = run(
            &mut grid,
            (10, 20),
            (15, 20),
            ToolMode(ToolMode::RED),
            plenty(),
            true,
        );
        assert_eq!(out.wire_spent, 6, "six tiles from ten to fifteen inclusive");
        assert!(!out.ran_out);
        for x in 10..=15 {
            assert!(
                grid.tile(x, 20).flags.has(TileFlags::WIRE_RED),
                "tile {x} should be wired"
            );
        }
        assert!(!grid.tile(16, 20).flags.has(TileFlags::WIRE_RED));
        assert!(!grid.tile(9, 20).flags.has(TileFlags::WIRE_RED));
    }

    /// A diagonal drag lays an L, not a diagonal: every tile of both legs.
    #[test]
    fn a_diagonal_drag_lays_a_corner() {
        let mut grid = Grid::new();
        let out = run(
            &mut grid,
            (10, 20),
            (13, 23),
            ToolMode(ToolMode::RED),
            plenty(),
            true,
        );
        // Facing right walks the vertical leg first, then the horizontal one along the far row.
        for y in 20..=23 {
            assert!(
                grid.tile(10, y).flags.has(TileFlags::WIRE_RED),
                "the vertical leg should cover (10, {y})"
            );
        }
        for x in 10..=13 {
            assert!(
                grid.tile(x, 23).flags.has(TileFlags::WIRE_RED),
                "the horizontal leg should cover ({x}, 23)"
            );
        }
        // ...and nothing on the diagonal between them.
        assert!(!grid.tile(11, 21).flags.has(TileFlags::WIRE_RED));
        assert_eq!(out.wire_spent, 7, "four down and three across");
    }

    /// Which way the player faces decides which corner the L turns.
    #[test]
    fn facing_decides_which_way_the_corner_goes() {
        let mut right = Grid::new();
        run(
            &mut right,
            (10, 20),
            (13, 23),
            ToolMode(ToolMode::RED),
            plenty(),
            true,
        );
        let mut left = Grid::new();
        run(
            &mut left,
            (10, 20),
            (13, 23),
            ToolMode(ToolMode::RED),
            plenty(),
            false,
        );
        assert!(
            right.tile(10, 23).flags.has(TileFlags::WIRE_RED),
            "facing right turns at the bottom-left"
        );
        assert!(
            left.tile(13, 20).flags.has(TileFlags::WIRE_RED),
            "facing left turns at the top-right"
        );
        assert_ne!(
            right.tile(10, 23).flags.has(TileFlags::WIRE_RED),
            left.tile(10, 23).flags.has(TileFlags::WIRE_RED),
            "the two corners should differ"
        );
    }

    /// Running out of wire stops the run and keeps what was already laid.
    #[test]
    fn running_out_of_wire_stops_where_it_ran_out() {
        let mut grid = Grid::new();
        let out = run(
            &mut grid,
            (10, 20),
            (30, 20),
            ToolMode(ToolMode::RED),
            Supplies {
                wire: 5,
                actuators: 0,
            },
            true,
        );
        assert!(
            out.ran_out,
            "twenty-one tiles cannot be done with five wire"
        );
        assert_eq!(out.wire_spent, 5, "and exactly five is what it spent");
        for x in 10..15 {
            assert!(grid.tile(x, 20).flags.has(TileFlags::WIRE_RED));
        }
        assert!(
            !grid.tile(15, 20).flags.has(TileFlags::WIRE_RED),
            "the tile it ran out on is untouched"
        );
    }

    /// Wire already there costs nothing, so re-running a line is free.
    #[test]
    fn wire_already_there_is_not_paid_for_twice() {
        let mut grid = Grid::new();
        run(
            &mut grid,
            (10, 20),
            (15, 20),
            ToolMode(ToolMode::RED),
            plenty(),
            true,
        );
        let again = run(
            &mut grid,
            (10, 20),
            (15, 20),
            ToolMode(ToolMode::RED),
            plenty(),
            true,
        );
        assert_eq!(again.wire_spent, 0);
        assert!(
            again.changes.is_empty(),
            "nothing changed, so nothing is sent"
        );
    }

    /// Four colours at once costs four wire a tile.
    #[test]
    fn every_colour_is_paid_for_separately() {
        let mut grid = Grid::new();
        let mode = ToolMode(ToolMode::RED | ToolMode::GREEN | ToolMode::BLUE | ToolMode::YELLOW);
        let out = run(&mut grid, (10, 20), (12, 20), mode, plenty(), true);
        assert_eq!(out.wire_spent, 12, "three tiles times four colours");
        let tile = grid.tile(11, 20);
        for flag in [
            TileFlags::WIRE_RED,
            TileFlags::WIRE_GREEN,
            TileFlags::WIRE_BLUE,
            TileFlags::WIRE_YELLOW,
        ] {
            assert!(tile.flags.has(flag));
        }
    }

    /// The cutter takes wire away and gives nothing back.
    #[test]
    fn the_cutter_removes_without_refunding() {
        let mut grid = Grid::new();
        run(
            &mut grid,
            (10, 20),
            (15, 20),
            ToolMode(ToolMode::RED | ToolMode::BLUE),
            plenty(),
            true,
        );
        // Cut only the red, leaving the blue.
        let out = run(
            &mut grid,
            (10, 20),
            (15, 20),
            ToolMode(ToolMode::RED | ToolMode::CUTTER),
            Supplies {
                wire: 0,
                actuators: 0,
            },
            true,
        );
        assert_eq!(out.wire_spent, 0, "cutting costs nothing");
        assert_eq!(out.changes.len(), 6, "and reports every tile it cut");
        for x in 10..=15 {
            assert!(!grid.tile(x, 20).flags.has(TileFlags::WIRE_RED));
            assert!(
                grid.tile(x, 20).flags.has(TileFlags::WIRE_BLUE),
                "the blue should be untouched"
            );
        }
    }

    /// Actuators come out of their own supply, not the wire.
    #[test]
    fn actuators_are_their_own_supply() {
        let mut grid = Grid::new();
        let out = run(
            &mut grid,
            (10, 20),
            (14, 20),
            ToolMode(ToolMode::RED | ToolMode::ACTUATOR),
            Supplies {
                wire: 100,
                actuators: 2,
            },
            true,
        );
        assert!(out.ran_out, "two actuators do not cover five tiles");
        assert_eq!(out.actuators_spent, 2);
        assert_eq!(
            out.wire_spent, 3,
            "the tile it ran out on had its wire laid first"
        );
    }

    /// A mode that asks for nothing does nothing, rather than walking the whole path.
    #[test]
    fn an_empty_mode_does_nothing() {
        let mut grid = Grid::new();
        let out = run(&mut grid, (0, 0), (100, 100), ToolMode(0), plenty(), true);
        assert_eq!(out, Outcome::default());
    }

    /// A drag that leaves the world does its work on the part that is inside it.
    #[test]
    fn a_run_off_the_edge_does_what_it_can() {
        let mut grid = Grid::new();
        let out = run(
            &mut grid,
            (197, 20),
            (203, 20),
            ToolMode(ToolMode::RED),
            plenty(),
            true,
        );
        assert!(!out.ran_out, "the edge is not running out of wire");
        assert_eq!(out.wire_spent, 3, "197, 198 and 199 are in the world");
        assert!(grid.tile(199, 20).flags.has(TileFlags::WIRE_RED));
    }
}
