//! Swinging doors open and shut, in the world rather than only on the wire.
//!
//! This server used to broadcast the toggle and leave its own tiles alone, on the reasoning that
//! every client would open the door for itself and the visible result would be the same. It is not
//! the same, and the difference is not cosmetic: the *server* went on believing every door in the
//! world was shut, so a town NPC standing at one decided to open it, was told nothing had changed,
//! and decided to open it again — for ever.
//!
//! Measured on a real world with a town on it, that came to **18,165 door packets in five minutes**
//! — sixty a second, and 48% of everything the server sent. On a freshly generated world with no
//! houses it never showed up at all.
//!
//! Ported from `WorldGen.OpenDoor` and `WorldGen.CloseDoor`. The shape of a door is the whole of the
//! problem: shut it is one tile wide and three tall (type 10); open it is **two** wide and three
//! tall (type 11), hinged on the side it swings towards. So opening writes six tiles and clears
//! three, and closing does the reverse — which is why "just broadcast it" was tempting.

use terrustia_proto::Tile;

use super::World;

/// A door that is shut.
pub const DOOR_CLOSED: u16 = 10;
/// The same door standing open.
pub const DOOR_OPEN: u16 = 11;

/// One frame is 18 pixels; a door's three-tile column repeats every 54.
const FRAME: i16 = 18;
const DOOR_HEIGHT_PX: i16 = 54;

/// Open the door at `(x, y)`, swinging it in `direction`.
///
/// `direction` is -1 for left and anything else for right, matching the game. Returns whether
/// anything actually moved, so a caller can tell "opened it" from "there was nothing to open" and
/// not announce the second as the first.
pub fn open(world: &mut World, x: i32, y: i32, direction: i8) -> bool {
    let tile = world.tile(x, y);
    if !tile.is_active() || tile.block != DOOR_CLOSED {
        return false;
    }

    // Walk up to the top of the door. `frameY` counts down the three tiles and wraps every 54, so
    // the offset within the door falls straight out of it.
    let within = tile.frame_y % DOOR_HEIGHT_PX;
    let top = y - i32::from(within / FRAME);

    // Which style of door this is. Every 54 pixels of `frameX` is another kind — plain wooden,
    // shadewood, lihzahrd and the rest — and the open form has its own pair of columns for each.
    let style = tile.frame_x / DOOR_HEIGHT_PX;
    let mut frame_base = style * 72;

    // The whole three-tile column has to be a shut door, or this is not one.
    for dy in 0..3 {
        let part = world.tile(x, top + dy);
        if !part.is_active() || part.block != DOOR_CLOSED {
            return false;
        }
    }

    // Where the open door will stand, and which column it has to swing into.
    let (left, swinging_into) = if direction == -1 {
        frame_base += 36;
        (x - 1, x - 1)
    } else {
        (x, x + 1)
    };

    // It cannot open into anything solid.
    for dy in 0..3 {
        let blocking = world.tile(swinging_into, top + dy);
        if blocking.is_active() {
            return false;
        }
    }

    let frame_y_base = (tile.frame_y / DOOR_HEIGHT_PX) * DOOR_HEIGHT_PX % (36 * DOOR_HEIGHT_PX);

    // Clear the shut door's column first: it is one tile wide and the open one is two, so leaving
    // it would put a stray third column of door beside the opening.
    for dy in 0..3 {
        world.set_tile(x, top + dy, Tile::AIR);
    }
    for dy in 0..3i32 {
        for dx in 0..2i32 {
            // `framed` rather than `block`: a door is frame-important, and a frame-important tile
            // built as a plain block carries -1 frames until they are overwritten. That is a
            // debug assertion here and a corrupt sprite on the client.
            let part = Tile::framed(
                DOOR_OPEN,
                frame_base + (dx as i16) * FRAME,
                frame_y_base + (dy as i16) * FRAME,
            );
            world.set_tile(left + dx, top + dy, part);
        }
    }
    true
}

/// Shut the door at `(x, y)`, wherever in its two-by-three block that position lands.
///
/// Returns whether anything moved.
pub fn close(world: &mut World, x: i32, y: i32) -> bool {
    // The position may be any of the six tiles the open door occupies, so find its corner.
    let Some((left, top, style)) = open_door_corner(world, x, y) else {
        return false;
    };

    // Which side it was hinged on decides where the shut door goes back to. The right-hand columns
    // of the open form are the ones with 36 added to the style's base.
    let hinged_left = (world.tile(left, top).frame_x - style * 72) >= 36;
    let shut_x = if hinged_left { left + 1 } else { left };

    for dy in 0..3i32 {
        for dx in 0..2i32 {
            world.set_tile(left + dx, top + dy, Tile::AIR);
        }
    }
    let frame_y_base = world.tile(shut_x, top).frame_y;
    for dy in 0..3i32 {
        let part = Tile::framed(
            DOOR_CLOSED,
            style * DOOR_HEIGHT_PX,
            frame_y_base + (dy as i16) * FRAME,
        );
        world.set_tile(shut_x, top + dy, part);
    }
    true
}

/// Find the top-left corner of the open door covering a position, and which style it is.
fn open_door_corner(world: &World, x: i32, y: i32) -> Option<(i32, i32, i16)> {
    let tile = world.tile(x, y);
    if !tile.is_active() || tile.block != DOOR_OPEN {
        return None;
    }
    let within_y = tile.frame_y % DOOR_HEIGHT_PX;
    let top = y - i32::from(within_y / FRAME);
    // An open door's two columns are eighteen pixels apart within its style; an odd column is the
    // right-hand one.
    let style = tile.frame_x / 72;
    let within_x = tile.frame_x - style * 72;
    let left = if within_x % 36 >= FRAME { x - 1 } else { x };
    Some((left, top, style))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain wooden door, shut, with its top at `(x, y)`.
    fn shut_door(world: &mut World, x: i32, y: i32) {
        for dy in 0..3i32 {
            world.set_tile(x, y + dy, Tile::framed(DOOR_CLOSED, 0, (dy as i16) * FRAME));
        }
    }

    #[test]
    fn opening_a_door_replaces_one_column_with_two() {
        let mut world = World::empty(40, 40, "doors");
        shut_door(&mut world, 10, 10);

        assert!(open(&mut world, 10, 11, 1), "the door should swing");

        // The shut column is gone and a two-wide opening stands in its place.
        for dy in 0..3 {
            assert_eq!(world.tile(10, 10 + dy).block, DOOR_OPEN);
            assert_eq!(world.tile(11, 10 + dy).block, DOOR_OPEN);
        }
    }

    /// The whole point: after opening, the world no longer says there is a shut door there.
    ///
    /// This is what stops a town NPC deciding to open the same door sixty times a second — which is
    /// exactly what it did while the toggle was broadcast and the tiles left alone.
    #[test]
    fn an_opened_door_no_longer_reads_as_shut() {
        let mut world = World::empty(40, 40, "doors");
        shut_door(&mut world, 10, 10);
        open(&mut world, 10, 11, 1);

        for dy in 0..3 {
            assert_ne!(
                world.tile(10, 10 + dy).block,
                DOOR_CLOSED,
                "still shut at row {dy}"
            );
        }
        assert!(
            !open(&mut world, 10, 11, 1),
            "opening it again should do nothing"
        );
    }

    #[test]
    fn a_door_swings_the_way_it_is_pushed() {
        let mut world = World::empty(40, 40, "doors");
        shut_door(&mut world, 10, 10);
        open(&mut world, 10, 10, -1);
        // Swinging left puts the opening at x-1 and x.
        assert_eq!(world.tile(9, 10).block, DOOR_OPEN);
        assert_eq!(world.tile(10, 10).block, DOOR_OPEN);
        assert!(!world.tile(11, 10).is_active(), "and not to the right");
    }

    #[test]
    fn a_door_will_not_open_into_a_wall() {
        let mut world = World::empty(40, 40, "doors");
        shut_door(&mut world, 10, 10);
        for dy in 0..3 {
            world.set_tile(11, 10 + dy, Tile::block(1));
        }
        assert!(!open(&mut world, 10, 10, 1), "there is stone in the way");
        assert_eq!(world.tile(10, 10).block, DOOR_CLOSED, "and it stays shut");
    }

    #[test]
    fn a_door_closes_back_to_one_column() {
        let mut world = World::empty(40, 40, "doors");
        shut_door(&mut world, 10, 10);
        assert!(open(&mut world, 10, 10, 1));
        assert!(close(&mut world, 10, 10), "and shuts again");

        for dy in 0..3 {
            assert_eq!(world.tile(10, 10 + dy).block, DOOR_CLOSED);
            assert!(
                !world.tile(11, 10 + dy).is_active(),
                "the second column should be clear again"
            );
        }
    }

    #[test]
    fn closing_from_any_part_of_the_opening_works() {
        // A caller knows where the door was, not which of its six tiles it is holding.
        for (from_x, from_y) in [(10, 10), (11, 10), (10, 12), (11, 11)] {
            let mut world = World::empty(40, 40, "doors");
            shut_door(&mut world, 10, 10);
            open(&mut world, 10, 10, 1);
            assert!(
                close(&mut world, from_x, from_y),
                "closing from {from_x},{from_y}"
            );
            assert_eq!(world.tile(10, 10).block, DOOR_CLOSED);
        }
    }

    #[test]
    fn there_is_nothing_to_open_where_there_is_no_door() {
        let mut world = World::empty(40, 40, "doors");
        assert!(!open(&mut world, 5, 5, 1));
        world.set_tile(5, 5, Tile::block(1));
        assert!(!open(&mut world, 5, 5, 1), "stone is not a door");
        assert!(!close(&mut world, 5, 5));
    }
}
