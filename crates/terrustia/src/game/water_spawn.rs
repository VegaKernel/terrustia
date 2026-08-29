//! NPC selection for candidates submerged under more than one tile of water.
//!
//! The ordinary spawn tables are ground/air tables. Mixing aquatic NPCs into them made Sharks,
//! Jellyfish and Arapaimas eligible on dry floors while the room check simultaneously rejected the
//! deep water where they actually belong. This module mirrors vanilla's high-level water-source
//! priority and keeps the two media separate.

use super::spawn::{Biome, Depth};

// Preserve Terrustia's existing 1:1:1 weighting between Pink Jellyfish, Shark and Squid while
// restoring Sea Snail at its source-backed rate of one third as often as Squid. Multiplying the
// old entries by three keeps their relative probabilities unchanged; one Sea Snail entry then
// gives Squid:Sea Snail = 3:1 without pretending the whole Ocean distribution is oracle-exact.
const OCEAN: &[u16] = &[
    64, 64, 64, // PinkJellyfish
    65, 65, 65, // Shark
    221, 221, 221, // Squid
    220, // SeaSnail
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

/// Width of vanilla's inner Ocean-spawn band from either true world border.
pub const OCEAN_INNER_BAND: i32 = 250;
/// Width of the secondary regular-Sand-only Ocean-spawn band.
pub const OCEAN_OUTER_BAND: i32 = 380;
/// The outer-band ceiling is this many tiles above the vertical midpoint of the Underground layer.
pub const OCEAN_OUTER_CEILING_OFFSET: i32 = 40;

fn sand(block: u16) -> bool {
    matches!(block, 53 | 112 | 116 | 234)
}

/// Whether a resolved Sand source is in one of vanilla's two ordinary Ocean NPC spawn regions.
///
/// The current official spawning rules describe two independent routes:
/// - any Sand variant in the inner 250-tile band, above the Cavern layer;
/// - regular Sand only in the outer 380-tile band, above a ceiling 40 tiles above the midpoint of
///   the Underground layer.
///
/// The edge comparison follows Terraria's long-standing world-border checks (`x < band` or
/// `x > maxTilesX - band`): a source exactly at 250/380 is outside that band. This helper is kept
/// separate from [`pool`] until the remaining source-Y/fallback pipeline is carried through
/// `try_spawn`, so the documented geometry can be reviewed without silently changing selection.
pub fn ocean_source_eligible(
    world_width: i32,
    x: i32,
    source_y: i32,
    surface: i32,
    rock_layer: i32,
    source_block: u16,
) -> bool {
    let inner = x < OCEAN_INNER_BAND || x > world_width - OCEAN_INNER_BAND;
    if inner && sand(source_block) && source_y < rock_layer {
        return true;
    }

    let outer = x < OCEAN_OUTER_BAND || x > world_width - OCEAN_OUTER_BAND;
    let underground_midpoint = (surface + rock_layer) / 2;
    let outer_ceiling = underground_midpoint - OCEAN_OUTER_CEILING_OFFSET;
    outer && source_block == 53 && source_y < outer_ceiling
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
    fn inner_ocean_band_accepts_every_sand_variant_above_caverns() {
        for x in [0, 249, 1751, 1999] {
            for block in [53, 112, 116, 234] {
                assert!(ocean_source_eligible(2000, x, 299, 200, 300, block));
            }
        }
        assert!(!ocean_source_eligible(2000, 250, 299, 200, 300, 112));
        assert!(!ocean_source_eligible(2000, 1750, 299, 200, 300, 112));
        assert!(!ocean_source_eligible(2000, 249, 300, 200, 300, 53));
    }

    #[test]
    fn outer_ocean_band_is_regular_sand_only_and_has_its_own_ceiling() {
        // Underground midpoint is 250; the outer-band ceiling is therefore y=210.
        for x in [250, 379, 1621, 1750] {
            assert!(ocean_source_eligible(2000, x, 209, 200, 300, 53));
            assert!(!ocean_source_eligible(2000, x, 210, 200, 300, 53));
            for variant in [112, 116, 234] {
                assert!(!ocean_source_eligible(2000, x, 209, 200, 300, variant));
            }
        }
    }

    #[test]
    fn exact_outer_band_edges_are_outside() {
        assert!(!ocean_source_eligible(2000, 380, 100, 200, 300, 53));
        assert!(!ocean_source_eligible(2000, 1620, 100, 200, 300, 53));
        assert!(ocean_source_eligible(2000, 379, 100, 200, 300, 53));
        assert!(ocean_source_eligible(2000, 1621, 100, 200, 300, 53));
    }

    #[test]
    fn non_sand_never_qualifies_as_an_ocean_source() {
        for block in [0, 1, 60, 147, 161] {
            assert!(!ocean_source_eligible(2000, 1, 100, 200, 300, block));
        }
    }

    #[test]
    fn ocean_water_has_existing_aquatic_enemies_and_never_the_crab() {
        let pool = pool(Depth::Surface, Biome::Ocean, false, 53);
        for npc in [64, 65, 220, 221] {
            assert!(pool.contains(&npc), "missing ocean-water npc {npc}");
        }
        assert!(!pool.contains(&67), "Crab is a ground spawn, not a water spawn");
    }

    #[test]
    fn sea_snail_is_one_third_as_common_as_squid_without_reweighting_old_ocean_entries() {
        let squid = OCEAN.iter().filter(|&&npc| npc == 221).count();
        let snail = OCEAN.iter().filter(|&&npc| npc == 220).count();
        assert_eq!(squid, snail * 3);

        let pink = OCEAN.iter().filter(|&&npc| npc == 64).count();
        let shark = OCEAN.iter().filter(|&&npc| npc == 65).count();
        assert_eq!(pink, squid, "preserve the old Pink Jellyfish:Squid ratio");
        assert_eq!(shark, squid, "preserve the old Shark:Squid ratio");
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
        for npc in [58, 63, 64, 65, 102, 103, 157, 220, 221, 241, 242] {
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
