//! Turning one biome into another, block by block.
//!
//! Every biome in the game is the same set of materials wearing different colours: grass, stone,
//! ice, sand, hardened sand, sandstone, thorns, and the walls behind them. Converting a tile means
//! finding which of those categories it belongs to and swapping it for the same category's version
//! in the target biome. That one table drives the hardmode V, the spread of the corruption into a
//! forest, and a Clentaminator undoing it.
//!
//! A tile in no category is left alone, which is what keeps a conversion from eating a chest, a
//! door, or a player's brickwork.

/// Which biome a tile is being converted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    /// Back to ordinary forest.
    Purity,
    Corruption,
    Hallow,
    Crimson,
}

/// The material categories a tile can belong to.
///
/// These are the game's own `TileID.Sets.Conversion` sets. Everything in one category converts as
/// one thing, which is why grass of any evil turns to grass of any other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Material {
    JungleGrass,
    MushroomGrass,
    GolfGrass,
    Grass,
    Stone,
    Ice,
    Sand,
    HardenedSand,
    Sandstone,
    Thorn,
}

fn material_of(block: u16) -> Option<Material> {
    Some(match block {
        60 | 661 | 662 => Material::JungleGrass,
        70 => Material::MushroomGrass,
        477 | 492 => Material::GolfGrass,
        2 | 23 | 109 | 199 => Material::Grass,
        // Moss is stone that has grown something, and converts as stone.
        1 | 25 | 117 | 203 | 179..=183 | 381 | 534 | 536 | 539 | 625 | 627 => Material::Stone,
        161 | 163 | 164 | 200 => Material::Ice,
        53 | 112 | 116 | 234 => Material::Sand,
        397..=399 | 402 => Material::HardenedSand,
        396 | 400 | 401 | 403 => Material::Sandstone,
        32 | 69 | 352 | 655 => Material::Thorn,
        _ => return None,
    })
}

/// What a block becomes in a biome. `None` means it is not the kind of thing that converts.
pub fn block(block: u16, into: Biome) -> Option<u16> {
    let made = match (material_of(block)?, into) {
        // Jungle grass keeps its own identity in every biome that has one for it.
        (Material::JungleGrass, Biome::Purity) => 60,
        (Material::JungleGrass, Biome::Corruption) => 661,
        (Material::JungleGrass, Biome::Crimson) => 662,
        // ...but the hallow has no jungle of its own, so it takes the ordinary grass.
        (Material::JungleGrass, Biome::Hallow) => 109,
        // Mushroom grass is only ever itself or plain jungle.
        (Material::MushroomGrass, Biome::Purity) => 70,
        (Material::MushroomGrass, _) => return None,
        // Golf grass survives purity and the hallow and is eaten by the evils.
        (Material::GolfGrass, Biome::Purity) => 477,
        (Material::GolfGrass, Biome::Hallow) => 492,
        (Material::GolfGrass, Biome::Corruption) => 23,
        (Material::GolfGrass, Biome::Crimson) => 199,
        (Material::Grass, Biome::Purity) => 2,
        (Material::Grass, Biome::Corruption) => 23,
        (Material::Grass, Biome::Hallow) => 109,
        (Material::Grass, Biome::Crimson) => 199,
        (Material::Stone, Biome::Purity) => 1,
        (Material::Stone, Biome::Corruption) => 25,
        (Material::Stone, Biome::Hallow) => 117,
        (Material::Stone, Biome::Crimson) => 203,
        (Material::Ice, Biome::Purity) => 161,
        (Material::Ice, Biome::Corruption) => 163,
        (Material::Ice, Biome::Hallow) => 164,
        (Material::Ice, Biome::Crimson) => 200,
        (Material::Sand, Biome::Purity) => 53,
        (Material::Sand, Biome::Corruption) => 112,
        (Material::Sand, Biome::Hallow) => 116,
        (Material::Sand, Biome::Crimson) => 234,
        (Material::HardenedSand, Biome::Purity) => 397,
        (Material::HardenedSand, Biome::Corruption) => 398,
        (Material::HardenedSand, Biome::Hallow) => 402,
        (Material::HardenedSand, Biome::Crimson) => 399,
        (Material::Sandstone, Biome::Purity) => 396,
        (Material::Sandstone, Biome::Corruption) => 400,
        (Material::Sandstone, Biome::Hallow) => 403,
        (Material::Sandstone, Biome::Crimson) => 401,
        // Thorns only exist in the two evils and the jungle they came from.
        (Material::Thorn, Biome::Corruption) => 32,
        (Material::Thorn, Biome::Crimson) => 352,
        (Material::Thorn, Biome::Purity) => 69,
        (Material::Thorn, Biome::Hallow) => return None,
    };
    (made != block).then_some(made)
}

/// The wall categories, which do not line up with the tile ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WallKind {
    Grass,
    Stone,
    HardenedSand,
    Sandstone,
    /// The four cave-wall variants, which each convert to their own numbered counterpart.
    Cave(u8),
}

fn wall_kind(wall: u16) -> Option<WallKind> {
    Some(match wall {
        63 | 65 | 66 | 68 | 69 | 70 | 81 => WallKind::Grass,
        1 | 3 | 28 | 83 | 349 => WallKind::Stone,
        216..=219 => WallKind::HardenedSand,
        187 | 220..=222 => WallKind::Sandstone,
        188 | 192 | 200 | 212 => WallKind::Cave(0),
        189 | 193 | 201 | 213 => WallKind::Cave(1),
        190 | 194 | 202 | 214 => WallKind::Cave(2),
        191 | 195 | 203 | 215 => WallKind::Cave(3),
        _ => return None,
    })
}

/// What a wall becomes in a biome.
pub fn wall(wall: u16, into: Biome) -> Option<u16> {
    let made = match (wall_kind(wall)?, into) {
        (WallKind::Grass, Biome::Purity) => 63,
        (WallKind::Grass, Biome::Corruption) => 69,
        (WallKind::Grass, Biome::Hallow) => 70,
        (WallKind::Grass, Biome::Crimson) => 81,
        (WallKind::Stone, Biome::Purity) => 349,
        (WallKind::Stone, Biome::Corruption) => 3,
        (WallKind::Stone, Biome::Hallow) => 28,
        (WallKind::Stone, Biome::Crimson) => 83,
        (WallKind::HardenedSand, Biome::Purity) => 216,
        (WallKind::HardenedSand, Biome::Corruption) => 217,
        (WallKind::HardenedSand, Biome::Hallow) => 219,
        (WallKind::HardenedSand, Biome::Crimson) => 218,
        (WallKind::Sandstone, Biome::Purity) => 187,
        (WallKind::Sandstone, Biome::Corruption) => 220,
        (WallKind::Sandstone, Biome::Hallow) => 222,
        (WallKind::Sandstone, Biome::Crimson) => 221,
        (WallKind::Cave(n), biome) => {
            let base = match biome {
                Biome::Purity => 212,
                Biome::Corruption => 188,
                Biome::Hallow => 200,
                Biome::Crimson => 192,
            };
            base + u16::from(n)
        }
    };
    (made != wall).then_some(made)
}

/// Which biome a block already belongs to, if any.
///
/// This is what a spread asks before it does anything: a tile already of the right biome is not
/// worth converting, and a tile of no biome at all cannot be.
pub fn biome_of(block: u16) -> Option<Biome> {
    match block {
        23 | 25 | 32 | 112 | 163 | 398 | 400 | 661 => Some(Biome::Corruption),
        199 | 203 | 234 | 200 | 352 | 399 | 401 | 662 => Some(Biome::Crimson),
        109 | 116 | 117 | 164 | 402 | 403 | 492 => Some(Biome::Hallow),
        1 | 2 | 53 | 60 | 70 | 161 | 396 | 397 | 477 => Some(Biome::Purity),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY_BIOME: [Biome; 4] = [
        Biome::Purity,
        Biome::Corruption,
        Biome::Hallow,
        Biome::Crimson,
    ];

    /// A material converts round the biomes and comes home unchanged.
    #[test]
    fn a_block_makes_the_round_trip() {
        for start in [1u16, 2, 53, 161, 397, 396] {
            let mut here = start;
            for biome in [Biome::Corruption, Biome::Crimson, Biome::Hallow] {
                here = block(here, biome).unwrap_or(here);
                assert_ne!(here, start, "{start} did not convert into {biome:?}");
            }
            let home = block(here, Biome::Purity).unwrap_or(here);
            assert_eq!(home, start, "{start} did not come home from {here}");
        }
    }

    /// Converting to the biome a block is already in does nothing.
    #[test]
    fn converting_in_place_is_a_no_op() {
        for biome in EVERY_BIOME {
            for candidate in 0..700u16 {
                let Some(made) = block(candidate, biome) else {
                    continue;
                };
                assert_eq!(
                    block(made, biome),
                    None,
                    "{candidate} became {made}, which converts again"
                );
            }
        }
    }

    /// A conversion never invents a block this build does not have.
    #[test]
    fn conversions_only_name_real_blocks() {
        for biome in EVERY_BIOME {
            for candidate in 0..700u16 {
                if let Some(made) = block(candidate, biome) {
                    assert!(
                        crate::tile_sets::TILE_COUNT > made,
                        "{candidate} -> {made} is past the end of the table"
                    );
                }
            }
        }
    }

    /// Things that are not terrain are left alone: a chest is a chest in any biome.
    #[test]
    fn furniture_does_not_convert() {
        // A chest, a door, a workbench, dungeon brick, wood, obsidian.
        for untouched in [21u16, 10, 18, 41, 30, 56] {
            for biome in EVERY_BIOME {
                assert_eq!(
                    block(untouched, biome),
                    None,
                    "{untouched} converted into {biome:?}"
                );
            }
        }
    }

    /// Each evil's stone is its own, and the hallow's is different again.
    #[test]
    fn each_biome_has_its_own_stone() {
        let stones: Vec<u16> = EVERY_BIOME
            .iter()
            .map(|b| block(1, *b).unwrap_or(1))
            .collect();
        assert_eq!(stones, vec![1, 25, 117, 203]);
        let unique: std::collections::HashSet<u16> = stones.iter().copied().collect();
        assert_eq!(unique.len(), 4);
    }

    /// Walls convert too, and the cave walls keep their variant.
    #[test]
    fn cave_walls_keep_their_variant() {
        for variant in 0..4u16 {
            let purity = 212 + variant;
            assert_eq!(wall(purity, Biome::Corruption), Some(188 + variant));
            assert_eq!(wall(purity, Biome::Hallow), Some(200 + variant));
            assert_eq!(wall(purity, Biome::Crimson), Some(192 + variant));
            assert_eq!(wall(188 + variant, Biome::Purity), Some(purity));
        }
    }

    /// A block's own biome is recognised, and it is the one converting to it produces.
    #[test]
    fn a_block_knows_its_own_biome() {
        for biome in EVERY_BIOME {
            for candidate in [1u16, 2, 53, 161] {
                let made = block(candidate, biome).unwrap_or(candidate);
                assert_eq!(
                    biome_of(made),
                    Some(biome),
                    "{candidate} became {made}, which does not read as {biome:?}"
                );
            }
        }
    }

    /// The hallow has no thorns and no jungle, and says so rather than inventing one.
    #[test]
    fn the_hallow_has_no_thorns() {
        assert_eq!(block(32, Biome::Hallow), None, "corrupt thorns");
        assert_eq!(block(352, Biome::Hallow), None, "crimson thorns");
        // Jungle grass in the hallow becomes ordinary hallowed grass rather than nothing.
        assert_eq!(block(60, Biome::Hallow), Some(109));
    }
}
