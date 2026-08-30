//! Whether a room counts as a house for a town NPC.
//!
//! Transcribed from `WorldGen.StartRoomCheck`, `CheckRoom` and `RoomNeeds`: flood-fill the open
//! space, require it to be enclosed on both axes, big enough but not enormous, and furnished with
//! a chair, a table, a light and a door.

use terrustia_proto::{
    housing::{
        MAX_ROOM_SIZE, MAX_ROOM_TILES, MIN_ROOM_TILES, counts_as_chair, counts_as_door,
        counts_as_table, counts_as_torch, housing_wall_tile, wall_encloses,
    },
    tile_solid::solid,
};

use crate::world::World;

/// Why a room is not a house.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomError {
    TooCloseToWorldEdge,
    StartedInASolidTile,
    TooBig,
    TooSmall,
    NotEnclosed,
    NoChair,
    NoTable,
    NoLight,
    NoDoor,
}

impl RoomError {
    /// A short line to show a player who asked why their house was rejected.
    pub fn describe(self) -> &'static str {
        match self {
            Self::TooCloseToWorldEdge => "that is too close to the edge of the world",
            Self::StartedInASolidTile => "that spot is inside a block",
            Self::TooBig => "the room is too large",
            Self::TooSmall => "the room is too small (it needs 60 open tiles)",
            Self::NotEnclosed => "the room is not sealed; it needs walls behind it",
            Self::NoChair => "the room needs a chair",
            Self::NoTable => "the room needs a table",
            Self::NoLight => "the room needs a light source",
            Self::NoDoor => "the room needs a door",
        }
    }
}

/// A validated house.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Room {
    /// Every open tile inside it.
    pub tiles: Vec<(i32, i32)>,
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

impl Room {
    /// A sensible spot to stand an NPC: the middle of the room, on its floor.
    pub fn home_tile(&self) -> (i32, i32) {
        ((self.left + self.right) / 2, self.bottom)
    }
}

/// Whether a tile blocks the flood fill.
fn blocks(world: &World, x: i32, y: i32) -> bool {
    let tile = world.tile(x, y);
    if !tile.is_active() {
        return false;
    }
    // A solid block stops the flood, except a solid-top platform you can walk up onto.
    if solid(tile.block) && !terrustia_proto::tile_solid::solid_top(tile.block) {
        return true;
    }
    // An OPEN door, trapdoor or tall gate is a barrier the room check does not pass through, even
    // though it is not a solid tile: `WorldGen.CheckRoom` returns `BlockingOpenGate` for tile types
    // 11, 386 and 389 (`WorldGen.cs:6113-6127`) - the same set as `housing_wall_tile`. Without this
    // the flood walked straight through an open doorway into the outside or a neighbouring room, so
    // the far side failed the enclosure check and a perfectly good house was reported homeless.
    housing_wall_tile(tile.block)
}

/// Whether a tile seals a room, either by being solid or by being a wall-like object.
fn seals(world: &World, x: i32, y: i32) -> bool {
    let tile = world.tile(x, y);
    if wall_encloses(tile.wall) {
        return true;
    }
    tile.is_active() && (solid(tile.block) || housing_wall_tile(tile.block))
}

/// Every tile of a room must be sealed within two tiles on both axes.
///
/// This is what stops an open-fronted shed counting: a gap wider than two tiles on either axis
/// leaves at least one tile of the room unsealed.
fn enclosed_at(world: &World, x: i32, y: i32) -> bool {
    let horizontal = (-2..=2).any(|d| seals(world, x + d, y));
    let vertical = (-2..=2).any(|d| seals(world, x, y + d));
    horizontal && vertical
}

/// Check whether the open space containing `(x, y)` is a valid house.
pub fn check_room(world: &World, x: i32, y: i32) -> Result<Room, RoomError> {
    if x < 10 || y < 10 || x >= world.width() - 10 || y >= world.height() - 10 {
        return Err(RoomError::TooCloseToWorldEdge);
    }
    if blocks(world, x, y) {
        return Err(RoomError::StartedInASolidTile);
    }

    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![(x, y)];
    let mut tiles = Vec::new();
    let (mut left, mut right, mut top, mut bottom) = (x, x, y, y);
    let (mut chair, mut table, mut torch, mut door) = (false, false, false, false);

    while let Some((cx, cy)) = stack.pop() {
        if cx < 10 || cy < 10 || cx >= world.width() - 10 || cy >= world.height() - 10 {
            return Err(RoomError::TooCloseToWorldEdge);
        }
        if !seen.insert((cx, cy)) {
            continue;
        }

        let tile = world.tile(cx, cy);
        if tile.is_active() {
            // Furniture is recorded, then treated as part of the wall rather than walked through.
            chair |= counts_as_chair(tile.block);
            table |= counts_as_table(tile.block);
            torch |= counts_as_torch(tile.block);
            door |= counts_as_door(tile.block);
        }
        if blocks(world, cx, cy) {
            continue;
        }

        if !enclosed_at(world, cx, cy) {
            return Err(RoomError::NotEnclosed);
        }

        tiles.push((cx, cy));
        if tiles.len() > MAX_ROOM_TILES {
            return Err(RoomError::TooBig);
        }
        left = left.min(cx);
        right = right.max(cx);
        top = top.min(cy);
        bottom = bottom.max(cy);
        if right - left >= MAX_ROOM_SIZE || bottom - top >= MAX_ROOM_SIZE {
            return Err(RoomError::TooBig);
        }

        // The game pushes all eight neighbours, so a room joined only diagonally still counts.
        for dx in -1..=1 {
            for dy in -1..=1 {
                if dx != 0 || dy != 0 {
                    stack.push((cx + dx, cy + dy));
                }
            }
        }
    }

    if tiles.len() < MIN_ROOM_TILES {
        return Err(RoomError::TooSmall);
    }
    if !chair {
        return Err(RoomError::NoChair);
    }
    if !table {
        return Err(RoomError::NoTable);
    }
    if !torch {
        return Err(RoomError::NoLight);
    }
    if !door {
        return Err(RoomError::NoDoor);
    }

    Ok(Room {
        tiles,
        left,
        right,
        top,
        bottom,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    /// Build a sealed room of open space with stone walls behind it, plus furniture.
    ///
    /// The interior is `(x0+1..x0+w-1, y0+1..y0+h-1)`.
    fn house(w: i32, h: i32, furnish: bool) -> (World, i32, i32) {
        let mut world = World::empty(200, 200, "house");
        let (x0, y0) = (50, 50);

        // Solid shell.
        for x in x0..x0 + w {
            for y in y0..y0 + h {
                let edge = x == x0 || x == x0 + w - 1 || y == y0 || y == y0 + h - 1;
                if edge {
                    world.set_tile(x, y, Tile::block(1));
                } else {
                    // Interior: open, but with a built wall behind it.
                    let mut air = Tile::AIR;
                    air.wall = 4; // stone wall, which counts as a house wall
                    world.set_tile(x, y, air);
                }
            }
        }

        if furnish {
            world.set_tile(x0 + 2, y0 + h - 2, Tile::framed(15, 0, 0)); // chair
            world.set_tile(x0 + 4, y0 + h - 2, Tile::framed(14, 0, 0)); // table
            world.set_tile(x0 + 6, y0 + h - 2, Tile::framed(4, 0, 0)); // torch
            world.set_tile(x0 + 1, y0 + h - 2, Tile::framed(10, 0, 0)); // door
        }
        (world, x0 + 3, y0 + 2)
    }

    #[test]
    fn a_furnished_sealed_room_is_a_house() {
        let (world, x, y) = house(12, 9, true);
        let room = check_room(&world, x, y).expect("should be a valid house");
        assert!(room.tiles.len() >= MIN_ROOM_TILES);
        let (hx, hy) = room.home_tile();
        assert!(hx > 50 && hy > 50);
    }

    #[test]
    fn a_room_without_furniture_is_refused_item_by_item() {
        let (mut world, x, y) = house(12, 9, false);
        assert_eq!(check_room(&world, x, y), Err(RoomError::NoChair));

        world.set_tile(53, 57, Tile::framed(15, 0, 0));
        assert_eq!(check_room(&world, x, y), Err(RoomError::NoTable));

        world.set_tile(55, 57, Tile::framed(14, 0, 0));
        assert_eq!(check_room(&world, x, y), Err(RoomError::NoLight));

        world.set_tile(57, 57, Tile::framed(4, 0, 0));
        assert_eq!(check_room(&world, x, y), Err(RoomError::NoDoor));

        world.set_tile(51, 57, Tile::framed(10, 0, 0));
        assert!(check_room(&world, x, y).is_ok(), "now it is a house");
    }

    #[test]
    fn a_room_that_is_too_small_is_refused() {
        // 6x5 leaves 4x3 = 12 open tiles, well under the 60 needed.
        let (world, x, y) = house(6, 5, true);
        assert_eq!(check_room(&world, x, y), Err(RoomError::TooSmall));
    }

    #[test]
    fn a_room_with_a_hole_in_the_wall_is_not_enclosed() {
        let (mut world, x, y) = house(12, 9, true);
        assert!(check_room(&world, x, y).is_ok());

        // Knock a three-tile gap in the ceiling and clear the walls behind it, so the room opens
        // to the sky. A gap of one or two tiles is still sealed by the two-tile reach.
        for dx in 0..4 {
            world.set_tile(53 + dx, 50, Tile::AIR);
            for dy in 1..4 {
                let mut air = Tile::AIR;
                air.wall = 0;
                world.set_tile(53 + dx, 50 + dy, air);
            }
        }
        assert!(
            matches!(
                check_room(&world, x, y),
                Err(RoomError::NotEnclosed) | Err(RoomError::TooCloseToWorldEdge)
            ),
            "an open roof should not be a house"
        );
    }

    #[test]
    fn natural_dirt_walls_do_not_make_a_house() {
        let (mut world, x, y) = house(12, 9, true);
        // Swap the built stone wall for a natural dirt one.
        for tx in 51..61 {
            for ty in 51..58 {
                let mut t = world.tile(tx, ty);
                if !t.is_active() {
                    t.wall = 2; // dirt
                    world.set_tile(tx, ty, t);
                }
            }
        }
        assert_eq!(check_room(&world, x, y), Err(RoomError::NotEnclosed));
    }

    /// An open door in an outside wall does not open the house to the world: the flood must not
    /// walk through it (`WorldGen.CheckRoom` BlockingOpenGate, `WorldGen.cs:6113-6127`). Fails
    /// before the fix, when `blocks` only stopped on solid tiles, so the flood escaped through the
    /// open doorway and the far side failed the enclosure check - the house was reported homeless.
    #[test]
    fn an_open_door_in_an_outside_wall_is_not_a_way_out() {
        let (mut world, x, y) = house(12, 9, true);
        assert!(
            check_room(&world, x, y).is_ok(),
            "the sealed house is valid to begin with"
        );

        // Replace a tile of the left wall (x0 = 50) with an OPEN door (type 11). Beyond it, at
        // x = 49, is open sky with no house wall behind it.
        world.set_tile(50, 55, Tile::framed(11, 0, 0));
        assert!(
            check_room(&world, x, y).is_ok(),
            "an open door is a wall to the room check, not a hole to walk out of",
        );

        // Sanity: a genuine hole (plain air, no door) in the same spot with the far side open does
        // let the flood escape, so the room is not enclosed - the fix is specific to the door.
        let mut air = Tile::AIR;
        air.wall = 0;
        world.set_tile(50, 55, air);
        for dy in 53..58 {
            let mut outside = Tile::AIR;
            outside.wall = 0;
            world.set_tile(49, dy, outside);
        }
        assert_eq!(
            check_room(&world, x, y),
            Err(RoomError::NotEnclosed),
            "a real hole, unlike an open door, does leave the room unsealed",
        );
    }

    #[test]
    fn starting_inside_a_block_is_refused() {
        let (world, _, _) = house(12, 9, true);
        assert_eq!(
            check_room(&world, 50, 50),
            Err(RoomError::StartedInASolidTile)
        );
    }

    #[test]
    fn a_point_near_the_world_edge_is_refused() {
        let world = World::empty(200, 200, "edge");
        assert_eq!(
            check_room(&world, 2, 2),
            Err(RoomError::TooCloseToWorldEdge)
        );
    }

    #[test]
    fn every_failure_has_something_to_tell_the_player() {
        for e in [
            RoomError::TooCloseToWorldEdge,
            RoomError::StartedInASolidTile,
            RoomError::TooBig,
            RoomError::TooSmall,
            RoomError::NotEnclosed,
            RoomError::NoChair,
            RoomError::NoTable,
            RoomError::NoLight,
            RoomError::NoDoor,
        ] {
            assert!(!e.describe().is_empty());
        }
    }
}
