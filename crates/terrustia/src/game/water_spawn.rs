//! NPC selection for candidates submerged under more than one tile of water.
//!
//! The ordinary spawn tables are ground/air tables. Mixing aquatic NPCs into them made Sharks,
//! Jellyfish and Arapaimas eligible on dry floors while the room check simultaneously rejected the
//! deep water where they actually belong. This module mirrors vanilla's high-level water-source
//! priority and keeps the two media separate.

use super::spawn::{Biome, Depth};

const OCEAN: &[u16] = &[
    64,  // PinkJellyfish
    65,  // Shark
    221, // Squid
];
const BLOOD_WATER: &[u16] = &[
    241, // BloodFeeder
    242, // BloodJelly
];
const ARAPAIMA: &[u16] = &[157];
const PIRANHA: &[u16] = &[58];
// In Hardmode, on average two thirds of Piranhas are replaced by Angler Fish.
const HARDMODE_PIRANHA: &[u16] = &[58, 102, 102];
const BLUE_JELLY: &[u16] = &[63];
// Green Jellyfish replaces Blue Jellyfish two thirds of the time in Hardmode.
const HARDMODE_JELLY: &[u16] = &[63, 103, 103];

fn sand(block: u16) -> bool {
    matches!(block, 53 | 112 | 116 | 234)
}

/// The water-specific pool for one solid spawning tile.
///
/// This deliberately follows source priority rather than combining every matching biome into one
/// bag. A Hardmode Jungle water candidate becomes Arapaima territory before the generic Jungle
/// water source is considered; Crimson water likewise gets Blood Jelly/Feeder first. Ocean comes
/// before the generic underground-water Jellyfish source.
pub fn pool(depth: Depth, biome: Biome, hard_mode: bool, spawning_block: u16) -> &'static [u16] {
    if hard_mode && biome == Biome::Jungle {
        return ARAPAIMA;
    }
    if hard_mode && biome == Biome::Crimson {
        return BLOOD_WATER;
    }
    if biome == Biome::Ocean && sand(spawning_block) {
        return OCEAN;
    }

    // Jungle water is keyed by the spawning tile, or by being in Cavern and below. The current
    // biome classifier is player-area based and cannot replace the tile condition here.
    if spawning_block == 60 || matches!(depth, Depth::Cavern | Depth::Underworld) {
        return if hard_mode {
            HARDMODE_PIRANHA
        } else {
            PIRANHA
        };
    }

    if depth != Depth::Surface {
        return if hard_mode {
            HARDMODE_JELLY
        } else {
            BLUE_JELLY
        };
    }

    // Vanilla has a separate surface-water critter source. This server does not model that source
    // yet; returning no candidate is safer than falling back to a walking enemy in deep water.
    &[]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocean_water_has_existing_aquatic_enemies_and_never_the_crab() {
        let pool = pool(Depth::Surface, Biome::Ocean, false, 53);
        for npc in [64, 65, 221] {
            assert!(pool.contains(&npc), "missing ocean-water npc {npc}");
        }
        assert!(!pool.contains(&67), "Crab is a ground spawn, not a water spawn");
    }

    #[test]
    fn ocean_water_requires_sand() {
        assert!(pool(Depth::Surface, Biome::Ocean, false, 1).is_empty());
    }

    #[test]
    fn hardmode_crimson_water_preempts_the_generic_water_pool() {
        assert_eq!(
            pool(Depth::Cavern, Biome::Crimson, true, 1),
            BLOOD_WATER
        );
    }

    #[test]
    fn hardmode_jungle_water_preempts_piranhas() {
        assert_eq!(pool(Depth::Cavern, Biome::Jungle, true, 60), ARAPAIMA);
    }

    #[test]
    fn cavern_water_gets_piranhas_and_hardmode_replacement_weight() {
        assert_eq!(pool(Depth::Cavern, Biome::Forest, false, 1), PIRANHA);
        assert_eq!(
            pool(Depth::Cavern, Biome::Forest, true, 1),
            HARDMODE_PIRANHA
        );
        assert_eq!(
            HARDMODE_PIRANHA.iter().filter(|&&npc| npc == 102).count(),
            2
        );
    }

    #[test]
    fn underground_water_gets_jellyfish_and_hardmode_replacement_weight() {
        assert_eq!(
            pool(Depth::Underground, Biome::Forest, false, 1),
            BLUE_JELLY
        );
        assert_eq!(
            pool(Depth::Underground, Biome::Forest, true, 1),
            HARDMODE_JELLY
        );
        assert_eq!(
            HARDMODE_JELLY.iter().filter(|&&npc| npc == 103).count(),
            2
        );
    }

    #[test]
    fn every_water_pool_id_exists_in_this_build() {
        for npc in [58, 63, 64, 65, 102, 103, 157, 221, 241, 242] {
            assert!(
                terrustia_proto::npc_data::npc_stats(npc).is_some(),
                "water pool names unknown NPC {npc}"
            );
        }
    }

    #[test]
    fn generic_surface_water_does_not_fall_back_to_land_enemies() {
        assert!(pool(Depth::Surface, Biome::Forest, false, 1).is_empty());
    }
}
