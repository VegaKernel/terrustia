//! Effective ground/source information used by natural NPC selection.
//!
//! The physical floor an NPC stands on is not always the tile type Terraria uses to choose the
//! NPC. Platforms, Planter Boxes and placed Metal Bars are `tileSolidTop`: vanilla looks through
//! them for a real solid source below. Conveyor Belts changed in 1.4.5 and instead use the tile
//! directly underneath the belt.

use terrustia_proto::tile_solid::{solid, solid_top};

use crate::world::World;

/// Platform-like source lookup reaches 29 tiles below the physical floor.
pub const SOURCE_SCAN_BELOW: i32 = 29;

const CONVEYOR_LEFT: u16 = 421;
const CONVEYOR_RIGHT: u16 = 422;

fn conveyor(block: u16) -> bool {
    matches!(block, CONVEYOR_LEFT | CONVEYOR_RIGHT)
}

/// The effective source selected from one accepted natural-spawn location.
///
/// Both coordinates are retained deliberately. `physical_floor_y` is where the early location
/// search first found a solid floor. `source_y` is the tile whose type was selected after applying
/// the Platform/Planter/Metal Bar/Conveyor rules. They can differ by as many as 29 tiles, and
/// downstream rules do not all necessarily consume the same coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnSource {
    pub block: u16,
    pub physical_floor_y: i32,
    pub source_y: i32,
    /// True when no qualifying source existed below a platform-like floor and vanilla's documented
    /// fallback selected the original random chosen tile instead.
    pub used_chosen_fallback: bool,
}

impl SpawnSource {
    const fn new(block: u16, physical_floor_y: i32, source_y: i32) -> Self {
        Self {
            block,
            physical_floor_y,
            source_y,
            used_chosen_fallback: false,
        }
    }

    const fn chosen_fallback(block: u16, physical_floor_y: i32, chosen_y: i32) -> Self {
        Self {
            block,
            physical_floor_y,
            source_y: chosen_y,
            used_chosen_fallback: true,
        }
    }
}

/// Resolve the tile source Terraria uses for NPC selection while preserving its coordinates.
///
/// `chosen_y` is the original random candidate row before the floor scan. `floor_y` is the first
/// physical solid floor found by that earlier scan. An ordinary solid is its own source. A Conveyor
/// Belt used as the physical floor takes its source from exactly the tile immediately underneath
/// it. A solid-top floor instead searches downward through as many as 29 tiles and stops at the
/// first solid block that is not itself solid-top.
///
/// If no qualifying block is found below a platform-like floor, vanilla falls back to the original
/// chosen tile. Keeping `chosen_y` here fixes the old helper's known inability to represent that
/// fallback without making assumptions about the chosen tile type.
pub fn resolve(world: &World, x: i32, chosen_y: i32, floor_y: i32) -> SpawnSource {
    let floor = world.tile(x, floor_y);
    if conveyor(floor.block) {
        let source_y = floor_y + 1;
        return SpawnSource::new(world.tile(x, source_y).block, floor_y, source_y);
    }
    if !solid_top(floor.block) {
        return SpawnSource::new(floor.block, floor_y, floor_y);
    }

    for dy in 1..=SOURCE_SCAN_BELOW {
        let source_y = floor_y + dy;
        let tile = world.tile(x, source_y);
        if !tile.is_active() || !solid(tile.block) || solid_top(tile.block) {
            continue;
        }
        return SpawnSource::new(tile.block, floor_y, source_y);
    }

    let chosen = world.tile(x, chosen_y);
    SpawnSource::chosen_fallback(chosen.block, floor_y, chosen_y)
}

/// Compatibility wrapper returning only the selected block type.
///
/// Existing callers do not yet retain `chosen_y`, so this deliberately passes `floor_y` as the
/// fallback coordinate and therefore preserves the pre-descriptor behaviour for the pathological
/// floating-platform case. New code that has the original chosen coordinate should use [`resolve`]
/// directly; callers can then be migrated without one giant spawn-selector rewrite.
pub fn block(world: &World, x: i32, floor_y: i32) -> u16 {
    resolve(world, x, floor_y, floor_y).block
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    fn world() -> World {
        World::empty(100, 100, "spawn source")
    }

    #[test]
    fn ordinary_solid_is_its_own_source() {
        let mut world = world();
        assert!(world.set_tile(50, 40, Tile::block(53)));
        assert_eq!(
            resolve(&world, 50, 35, 40),
            SpawnSource::new(53, 40, 40)
        );
        assert_eq!(block(&world, 50, 40), 53);
    }

    #[test]
    fn platform_looks_through_to_the_solid_below_and_keeps_both_rows() {
        let mut world = world();
        assert!(world.set_tile(50, 40, Tile::block(19)));
        assert!(world.set_tile(50, 47, Tile::block(53)));
        assert_eq!(
            resolve(&world, 50, 35, 40),
            SpawnSource::new(53, 40, 47)
        );
        assert_eq!(block(&world, 50, 40), 53);
    }

    #[test]
    fn metal_bar_and_planter_box_use_the_same_solid_top_rule() {
        for floor in [239, 380] {
            let mut world = world();
            assert!(solid_top(floor), "{floor} must be solid-top in this build");
            assert!(world.set_tile(50, 40, Tile::block(floor)));
            assert!(world.set_tile(50, 45, Tile::block(60)));
            assert_eq!(
                resolve(&world, 50, 35, 40),
                SpawnSource::new(60, 40, 45)
            );
            assert_eq!(block(&world, 50, 40), 60);
        }
    }

    #[test]
    fn stacked_solid_top_tiles_are_skipped() {
        let mut world = world();
        assert!(world.set_tile(50, 40, Tile::block(19)));
        assert!(world.set_tile(50, 44, Tile::block(380)));
        assert!(world.set_tile(50, 46, Tile::block(239)));
        assert!(world.set_tile(50, 49, Tile::block(1)));
        assert_eq!(
            resolve(&world, 50, 35, 40),
            SpawnSource::new(1, 40, 49)
        );
        assert_eq!(block(&world, 50, 40), 1);
    }

    #[test]
    fn conveyor_floor_uses_exactly_the_tile_directly_below() {
        for belt in [CONVEYOR_LEFT, CONVEYOR_RIGHT] {
            let mut world = world();
            assert!(world.set_tile(50, 40, Tile::block(belt)));
            assert!(world.set_tile(50, 41, Tile::block(53)));
            assert!(world.set_tile(50, 45, Tile::block(1)));
            assert_eq!(
                resolve(&world, 50, 35, 40),
                SpawnSource::new(53, 40, 41)
            );
            assert_eq!(block(&world, 50, 40), 53);
        }
    }

    #[test]
    fn platform_scan_stops_on_a_conveyor_without_reapplying_the_conveyor_rule() {
        let mut world = world();
        assert!(world.set_tile(50, 40, Tile::block(19)));
        assert!(world.set_tile(50, 45, Tile::block(CONVEYOR_LEFT)));
        assert!(world.set_tile(50, 46, Tile::block(53)));
        assert_eq!(
            resolve(&world, 50, 35, 40),
            SpawnSource::new(CONVEYOR_LEFT, 40, 45)
        );
        assert_eq!(block(&world, 50, 40), CONVEYOR_LEFT);
    }

    #[test]
    fn platform_scan_falls_back_to_the_original_chosen_tile() {
        let mut world = world();
        assert!(world.set_tile(50, 35, Tile::block(4)));
        assert!(!solid(4), "chosen test tile must not itself be a solid floor");
        assert!(world.set_tile(50, 40, Tile::block(19)));

        assert_eq!(
            resolve(&world, 50, 35, 40),
            SpawnSource {
                block: 4,
                physical_floor_y: 40,
                source_y: 35,
                used_chosen_fallback: true,
            }
        );
        // Compatibility wrapper intentionally preserves the old fallback until callers migrate.
        assert_eq!(block(&world, 50, 40), 19);
    }

    #[test]
    fn platform_scan_is_bounded_to_twenty_nine_tiles() {
        let mut world = world();
        assert!(world.set_tile(50, 35, Tile::block(4)));
        assert!(world.set_tile(50, 40, Tile::block(19)));
        assert!(world.set_tile(50, 69, Tile::block(53)));
        assert_eq!(
            resolve(&world, 50, 35, 40),
            SpawnSource::new(53, 40, 69)
        );

        assert!(world.set_tile(50, 69, Tile::AIR));
        assert!(world.set_tile(50, 70, Tile::block(53)));
        let source = resolve(&world, 50, 35, 40);
        assert!(source.used_chosen_fallback);
        assert_eq!(source.block, 4);
        assert_eq!(source.source_y, 35);
    }
}
