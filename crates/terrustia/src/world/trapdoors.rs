//! Trapdoors and tall gates shift shape when a wire signal reaches them, the same way a door
//! swings open or shut.
//!
//! Ported from `WorldGen.ShiftTrapdoor`/`WorldGen.ShiftTallGate` (`WorldGen.cs:51905-52236`). A
//! trapdoor is a moving two-tile form: shut it is a two-by-two block (386), open it is a
//! two-wide, one-tall gap (387) sitting in whichever of the closed form's own two rows had room.
//! A tall gate is simpler — a five-tall column that just swaps its whole type between shut (388)
//! and open (389), keeping the same frame — but still has to find the true top of the column from
//! whichever of its five tiles a wire signal happened to touch.

use terrustia_proto::Tile;

use super::World;

/// A trapdoor lying shut, a two-by-two block.
pub const TRAPDOOR_CLOSED: u16 = 386;
/// The same trapdoor lying open: two tiles wide, one tall.
pub const TRAPDOOR_OPEN: u16 = 387;
/// A tall gate standing shut, five tiles tall.
pub const TALL_GATE_CLOSED: u16 = 388;
/// The same gate raised clear.
pub const TALL_GATE_OPEN: u16 = 389;

/// One frame cell is 18 pixels, the same increment a door's own frames use.
const FRAME: i16 = 18;

/// A tall gate's own five rows are not all the same height — `TileObjectData` for tile 388/389
/// (`terrustia_proto::tile_object`'s own extracted `coord_heights: &[18, 16, 16, 16, 18]`): the
/// end caps stand a couple of pixels taller than the three plain segments between them.
const GATE_ROW_HEIGHTS: [i16; 5] = [18, 16, 16, 16, 18];
/// The frame-Y span one full style of the gate's sprite occupies (`tile_object`'s own extracted
/// `full_height`), which is what a placed tile's `frame_y` is taken modulo before walking
/// [`GATE_ROW_HEIGHTS`] to find which row it is.
const GATE_FRAME_HEIGHT: i16 = 94;

/// Whether a trapdoor closing into this tile may kill it and proceed rather than being blocked by
/// it — `Main.tileCut[type] || TileID.Sets.IsADripTile[type]` (`WorldGen.cs:52010`,
/// `52026`), which is narrower than a door's own swing-through set: `OpenDoor` also explicitly
/// cuts a stalactite (165, `WorldGen.cs:38093`) that neither `Main.tileCut` nor `IsADripTile`
/// actually covers, and `ShiftTrapdoor`'s own close-guard has no such extra case. Built on
/// [`super::doors::cuttable`], which already *is* `tileCut ∪ {165} ∪ IsADripTile` by its own doc,
/// with that one extra tile excluded again here rather than a second copy of the underlying list.
fn cuttable(block: u16) -> bool {
    block != 165 && super::doors::cuttable(block)
}

/// Shift the trapdoor at `(x, y)` open or shut, whichever its current shape allows — the closed
/// form only opens and the open form only shuts, so the caller does not have to know which one it
/// is before calling.
///
/// `player_above` only matters when closing (`WorldGen.cs:52006`,
/// `playerAbove.ToDirectionInt()`): the reformed closed block sits one row up from the open
/// doorway if a player was above it, one row down otherwise, and which side it settles on is what
/// the closed tile's own frame remembers for the next time it opens. `occupied` reports whether a
/// player or NPC stands in a tile — `Collision.EmptyTile(x, y, ignoreTiles: true)`'s own
/// `ignoreTiles: true` skips the ordinary "is a block already there" check entirely and tests only
/// entities, which is exactly what stops *opening* a trapdoor from swallowing whoever is standing
/// where it is about to appear; there is no equivalent check on the close side in source.
///
/// Returns whether anything actually moved.
pub fn shift_trapdoor(
    world: &mut World,
    x: i32,
    y: i32,
    player_above: bool,
    occupied: impl Fn(i32, i32) -> bool,
) -> bool {
    let tile = world.tile(x, y);
    if !tile.is_active() {
        return false;
    }
    match tile.block {
        TRAPDOOR_CLOSED => open_trapdoor(world, x, y, tile, occupied),
        TRAPDOOR_OPEN => close_trapdoor(world, x, y, tile, player_above),
        _ => false,
    }
}

/// Closed (386, two-by-two) to open (387, two-by-one) — `WorldGen.cs:51908-51992`.
fn open_trapdoor(
    world: &mut World,
    x: i32,
    y: i32,
    tile: Tile,
    occupied: impl Fn(i32, i32) -> bool,
) -> bool {
    // Normalize to the block's own top-left, wherever within it the wire signal actually landed.
    let left = x - i32::from((tile.frame_x % (2 * FRAME)) / FRAME);
    let top = y - i32::from((tile.frame_y % (2 * FRAME)) / FRAME);
    // Which way this trapdoor last closed, and so which way it opens back: 0 opens downward
    // (clearing the top row, the doorway landing on the bottom one), 1 opens upward.
    let orientation = tile.frame_x / (2 * FRAME);
    if !matches!(orientation, 0 | 1) {
        return false;
    }

    for dx in 0..2i32 {
        for dy in 0..2i32 {
            let part = world.tile(left + dx, top + dy);
            if !part.is_active() || part.block != TRAPDOOR_CLOSED {
                return false;
            }
        }
    }

    let (doorway_row, clear_row) = if orientation == 0 {
        (top + 1, top)
    } else {
        (top, top + 1)
    };
    if occupied(left, doorway_row) || occupied(left + 1, doorway_row) {
        return false;
    }

    for dx in 0..2i32 {
        world.set_tile(left + dx, clear_row, Tile::AIR);
    }
    for dx in 0..2i32 {
        let part = Tile::framed(TRAPDOOR_OPEN, dx as i16 * FRAME, 0);
        world.set_tile(left + dx, doorway_row, part);
    }
    true
}

/// Open (387) back to closed (386) — `WorldGen.cs:51993-52053`.
fn close_trapdoor(world: &mut World, x: i32, y: i32, tile: Tile, player_above: bool) -> bool {
    let left = x - i32::from((tile.frame_x % (2 * FRAME)) / FRAME);

    for dx in 0..2i32 {
        let part = world.tile(left + dx, y);
        if !part.is_active() || part.block != TRAPDOOR_OPEN {
            return false;
        }
    }

    // The row the reformed block's *other* half lands on: one below the doorway if a player was
    // above it, one above otherwise.
    let offset: i32 = if player_above { 1 } else { -1 };
    let landing_row = y + offset;
    for dx in 0..2i32 {
        let part = world.tile(left + dx, landing_row);
        if part.is_active() && !cuttable(part.block) {
            return false;
        }
    }
    for dx in 0..2i32 {
        let part = world.tile(left + dx, landing_row);
        if part.is_active() && cuttable(part.block) {
            world.set_tile(left + dx, landing_row, Tile::AIR);
        }
    }

    let base_row = if player_above { y } else { y - 1 };
    let style_x = if player_above { 2 * FRAME } else { 0 };
    for dx in 0..2i32 {
        for row_offset in 0..2i32 {
            let part = Tile::framed(
                TRAPDOOR_CLOSED,
                dx as i16 * FRAME + style_x,
                row_offset as i16 * FRAME,
            );
            world.set_tile(left + dx, base_row + row_offset, part);
        }
    }
    true
}

/// Shift the tall gate at `(x, y)` — `WorldGen.cs:52183-52236`. `closing` picks the direction
/// outright rather than reading it off the current tile the way a trapdoor does, matching source:
/// a gate refuses if it is not already the type that direction expects.
///
/// Unless `forced`, refuses while `occupied` says a player or NPC stands anywhere in the column
/// the gate is about to occupy (`Collision.EmptyTile(x, y + k, ignoreTiles: true)`,
/// `WorldGen.cs:52217`) — real vanilla's own wire trigger never passes `forced`, so a wired gate
/// still will not close on someone standing in the doorway; `forced` exists for a caller that
/// needs to override that, the way this project's own `doors::close` is always forced for its one
/// caller.
///
/// Returns whether anything actually moved.
pub fn shift_tall_gate(
    world: &mut World,
    x: i32,
    y: i32,
    closing: bool,
    forced: bool,
    occupied: impl Fn(i32, i32) -> bool,
) -> bool {
    let (target, current) = if closing {
        (TALL_GATE_CLOSED, TALL_GATE_OPEN)
    } else {
        (TALL_GATE_OPEN, TALL_GATE_CLOSED)
    };
    let tile = world.tile(x, y);
    if !tile.is_active() || tile.block != current {
        return false;
    }

    // Walk the real per-row heights to find which row of the frame this tile is, and so how far
    // above it the column's own top sits.
    let mut remaining = tile.frame_y % GATE_FRAME_HEIGHT;
    let mut row = 0usize;
    while row < GATE_ROW_HEIGHTS.len() && remaining - GATE_ROW_HEIGHTS[row] >= 0 {
        remaining -= GATE_ROW_HEIGHTS[row];
        row += 1;
    }
    let top = y - row as i32;

    for dy in 0..GATE_ROW_HEIGHTS.len() as i32 {
        let part = world.tile(x, top + dy);
        if !part.is_active() || part.block != current {
            return false;
        }
    }
    if !forced {
        for dy in 0..GATE_ROW_HEIGHTS.len() as i32 {
            if occupied(x, top + dy) {
                return false;
            }
        }
    }

    for dy in 0..GATE_ROW_HEIGHTS.len() as i32 {
        let mut part = world.tile(x, top + dy);
        part.block = target;
        world.set_tile(x, top + dy, part);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed trapdoor with its top-left at `(x, y)`, oriented to open in `orientation`'s
    /// direction (0 downward, 1 upward — see [`open_trapdoor`]'s own doc).
    fn shut_trapdoor(world: &mut World, x: i32, y: i32, orientation: i16) {
        for dx in 0..2i32 {
            for dy in 0..2i32 {
                let part = Tile::framed(
                    TRAPDOOR_CLOSED,
                    dx as i16 * FRAME + orientation * 2 * FRAME,
                    dy as i16 * FRAME,
                );
                world.set_tile(x + dx, y + dy, part);
            }
        }
    }

    fn no_one_there(_: i32, _: i32) -> bool {
        false
    }

    #[test]
    fn opening_downward_leaves_the_top_row_clear_and_the_doorway_below() {
        let mut world = World::empty(40, 40, "trapdoors");
        shut_trapdoor(&mut world, 10, 10, 0);

        assert!(shift_trapdoor(&mut world, 10, 10, true, no_one_there));

        assert!(!world.tile(10, 10).is_active(), "top row cleared");
        assert!(!world.tile(11, 10).is_active());
        assert_eq!(world.tile(10, 11).block, TRAPDOOR_OPEN, "doorway below");
        assert_eq!(world.tile(11, 11).block, TRAPDOOR_OPEN);
    }

    #[test]
    fn opening_upward_leaves_the_bottom_row_clear_and_the_doorway_above() {
        let mut world = World::empty(40, 40, "trapdoors");
        shut_trapdoor(&mut world, 10, 10, 1);

        assert!(shift_trapdoor(&mut world, 10, 10, true, no_one_there));

        assert_eq!(world.tile(10, 10).block, TRAPDOOR_OPEN, "doorway on top");
        assert_eq!(world.tile(11, 10).block, TRAPDOOR_OPEN);
        assert!(!world.tile(10, 11).is_active(), "bottom row cleared");
        assert!(!world.tile(11, 11).is_active());
    }

    /// A player or NPC standing where the doorway would open refuses the shift outright, matching
    /// `Collision.EmptyTile(..., ignoreTiles: true)`'s own entity-only check.
    #[test]
    fn opening_refuses_while_something_stands_in_the_doorway() {
        let mut world = World::empty(40, 40, "trapdoors");
        shut_trapdoor(&mut world, 10, 10, 0);

        assert!(!shift_trapdoor(&mut world, 10, 10, true, |_, y| y == 11));
        assert_eq!(world.tile(10, 10).block, TRAPDOOR_CLOSED, "still shut");
    }

    // Opening downward from a shut door at rows [10, 11] leaves the open doorway at row 11 alone
    // (row 10 clear above it) — every "closing" test below starts from exactly that.

    #[test]
    fn closing_with_a_player_above_reforms_below_the_doorway() {
        let mut world = World::empty(40, 40, "trapdoors");
        shut_trapdoor(&mut world, 10, 10, 0);
        assert!(shift_trapdoor(&mut world, 10, 10, true, no_one_there));

        // A player above the row-11 doorway stands at row 10 or higher, so the reformed block
        // must not touch row 10 — it lands at [11, 12] instead.
        assert!(shift_trapdoor(&mut world, 10, 11, true, no_one_there));

        for dx in 0..2 {
            assert_eq!(world.tile(10 + dx, 11).block, TRAPDOOR_CLOSED);
            assert_eq!(world.tile(10 + dx, 12).block, TRAPDOOR_CLOSED);
        }
        assert!(!world.tile(10, 10).is_active(), "row 10 untouched");
    }

    #[test]
    fn closing_with_a_player_below_reforms_back_at_the_original_rows() {
        let mut world = World::empty(40, 40, "trapdoors");
        shut_trapdoor(&mut world, 10, 10, 0);
        assert!(shift_trapdoor(&mut world, 10, 10, true, no_one_there));

        // A player below the doorway stands at row 12 or lower, so the reformed block lands back
        // at the door's original [10, 11] instead.
        assert!(shift_trapdoor(&mut world, 10, 11, false, no_one_there));

        for dx in 0..2 {
            assert_eq!(world.tile(10 + dx, 10).block, TRAPDOOR_CLOSED);
            assert_eq!(world.tile(10 + dx, 11).block, TRAPDOOR_CLOSED);
        }
    }

    /// A cuttable plant where the trapdoor is about to close is killed, not a wall — the same
    /// courtesy `doors::open` already gives a vine in its own swing path.
    #[test]
    fn closing_kills_a_cuttable_plant_in_the_way() {
        let mut world = World::empty(40, 40, "trapdoors");
        shut_trapdoor(&mut world, 10, 10, 0);
        assert!(shift_trapdoor(&mut world, 10, 10, true, no_one_there));
        world.set_tile(10, 10, Tile::block(52)); // a vine, back where the doorway will land

        assert!(shift_trapdoor(&mut world, 10, 11, false, no_one_there));
        assert_eq!(
            world.tile(10, 10).block,
            TRAPDOOR_CLOSED,
            "the vine is gone"
        );
    }

    /// A real wall in the way still refuses, exactly as before.
    #[test]
    fn closing_still_refuses_a_solid_block_in_the_way() {
        let mut world = World::empty(40, 40, "trapdoors");
        shut_trapdoor(&mut world, 10, 10, 0);
        assert!(shift_trapdoor(&mut world, 10, 10, true, no_one_there));
        world.set_tile(10, 10, Tile::block(1)); // plain stone

        assert!(!shift_trapdoor(&mut world, 10, 11, false, no_one_there));
        assert_eq!(world.tile(10, 11).block, TRAPDOOR_OPEN, "still open");
    }

    /// A tall gate with its top at `(x, y)`, in whichever of `TALL_GATE_CLOSED`/`_OPEN` `block`
    /// names, framed the way a real placement would be.
    fn place_tall_gate(world: &mut World, x: i32, y: i32, block: u16) {
        let mut frame_y = 0i16;
        for (row, &height) in GATE_ROW_HEIGHTS.iter().enumerate() {
            world.set_tile(x, y + row as i32, Tile::framed(block, 0, frame_y));
            frame_y += height;
        }
    }

    #[test]
    fn a_closed_gate_opens_from_any_of_its_five_rows() {
        for hit_row in 0..5i32 {
            let mut world = World::empty(40, 40, "tall gates");
            place_tall_gate(&mut world, 10, 10, TALL_GATE_CLOSED);

            assert!(
                shift_tall_gate(&mut world, 10, 10 + hit_row, false, false, no_one_there),
                "hitting row {hit_row}"
            );
            for row in 0..5 {
                assert_eq!(world.tile(10, 10 + row).block, TALL_GATE_OPEN);
            }
        }
    }

    #[test]
    fn an_open_gate_closes_and_keeps_its_frame() {
        let mut world = World::empty(40, 40, "tall gates");
        place_tall_gate(&mut world, 10, 10, TALL_GATE_OPEN);
        let frames_before: Vec<i16> = (0..5).map(|row| world.tile(10, 10 + row).frame_y).collect();

        assert!(shift_tall_gate(
            &mut world,
            10,
            12,
            true,
            false,
            no_one_there
        ));

        for row in 0..5 {
            let tile = world.tile(10, 10 + row);
            assert_eq!(tile.block, TALL_GATE_CLOSED);
            assert_eq!(
                tile.frame_y, frames_before[row as usize],
                "row {row} frame should be untouched"
            );
        }
    }

    #[test]
    fn an_unforced_close_refuses_while_something_stands_in_the_column() {
        let mut world = World::empty(40, 40, "tall gates");
        place_tall_gate(&mut world, 10, 10, TALL_GATE_OPEN);

        assert!(!shift_tall_gate(&mut world, 10, 10, true, false, |_, y| y == 12));
        assert_eq!(world.tile(10, 10).block, TALL_GATE_OPEN, "still open");
    }

    #[test]
    fn a_forced_close_ignores_occupancy() {
        let mut world = World::empty(40, 40, "tall gates");
        place_tall_gate(&mut world, 10, 10, TALL_GATE_OPEN);

        assert!(shift_tall_gate(&mut world, 10, 10, true, true, |_, _| true));
        assert_eq!(world.tile(10, 10).block, TALL_GATE_CLOSED);
    }

    #[test]
    fn opening_the_wrong_direction_does_nothing() {
        let mut world = World::empty(40, 40, "tall gates");
        place_tall_gate(&mut world, 10, 10, TALL_GATE_CLOSED);
        // Asking to close an already-closed gate should refuse.
        assert!(!shift_tall_gate(
            &mut world,
            10,
            10,
            true,
            false,
            no_one_there
        ));
        assert_eq!(world.tile(10, 10).block, TALL_GATE_CLOSED);
    }
}
