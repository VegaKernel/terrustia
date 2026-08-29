//! The medium immediately above a vanilla NPC spawning tile.
//!
//! Vanilla first finds a solid spawning tile, then decides what can appear from the liquid in the
//! two tiles immediately above it. Treating that as part of "room" was a design mistake: it made
//! deep water invalid before the spawn pool ever had a chance to choose a Shark, Jellyfish or
//! Sleeping Angler.

use terrustia_proto::Liquid;

use crate::world::World;

/// Which broad spawn source the two tiles above a solid spawning tile select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMedium {
    /// No more than one tile of liquid above the spawning tile.
    Dry,
    /// Both tiles immediately above the spawning tile contain water.
    Water,
}

/// Classify the two tiles immediately above a solid spawning tile.
///
/// Vanilla's split is deliberately binary here: two liquid-containing tiles mean the "more than
/// one tile deep" path. That path is valid only when both are water; deep honey, shimmer or lava
/// cannot become an aquatic spawn source. A single liquid tile is still the ordinary <= 1-tile
/// path and is therefore [`SpawnMedium::Dry`].
pub fn classify(world: &World, x: i32, ground_y: i32) -> Option<SpawnMedium> {
    let first = world.tile(x, ground_y - 1);
    let second = world.tile(x, ground_y - 2);

    if first.liquid == 0 || second.liquid == 0 {
        return Some(SpawnMedium::Dry);
    }

    if first.liquid_kind == Liquid::Water && second.liquid_kind == Liquid::Water {
        Some(SpawnMedium::Water)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    fn world() -> World {
        World::empty(100, 100, "spawn medium")
    }

    #[test]
    fn empty_air_is_dry() {
        assert_eq!(classify(&world(), 50, 50), Some(SpawnMedium::Dry));
    }

    #[test]
    fn one_liquid_tile_is_still_the_shallow_path() {
        let mut world = world();
        assert!(world.set_tile(
            50,
            49,
            Tile::AIR.with_liquid(Liquid::Water, u8::MAX)
        ));
        assert_eq!(classify(&world, 50, 50), Some(SpawnMedium::Dry));
    }

    #[test]
    fn two_water_tiles_select_the_water_path() {
        let mut world = world();
        for y in [48, 49] {
            assert!(world.set_tile(
                50,
                y,
                Tile::AIR.with_liquid(Liquid::Water, u8::MAX)
            ));
        }
        assert_eq!(classify(&world, 50, 50), Some(SpawnMedium::Water));
    }

    #[test]
    fn deep_non_water_liquid_is_not_a_valid_spawn_medium() {
        for kind in [Liquid::Lava, Liquid::Honey, Liquid::Shimmer] {
            let mut world = world();
            for y in [48, 49] {
                assert!(world.set_tile(
                    50,
                    y,
                    Tile::AIR.with_liquid(kind, u8::MAX)
                ));
            }
            assert_eq!(classify(&world, 50, 50), None, "{kind:?}");
        }
    }

    #[test]
    fn mixed_deep_liquids_are_not_water() {
        let mut world = world();
        assert!(world.set_tile(
            50,
            49,
            Tile::AIR.with_liquid(Liquid::Water, u8::MAX)
        ));
        assert!(world.set_tile(
            50,
            48,
            Tile::AIR.with_liquid(Liquid::Honey, u8::MAX)
        ));
        assert_eq!(classify(&world, 50, 50), None);
    }
}
