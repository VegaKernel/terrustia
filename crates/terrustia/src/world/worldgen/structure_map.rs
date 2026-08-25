//! A tracker of placed structures' bounding rectangles, so a later pass can ask "is anything
//! already here" before placing something new.
//!
//! Transcribed from `Terraria.WorldBuilding.StructureMap` (`.scratch/decompiled/Terraria.WorldBuilding/StructureMap.cs`,
//! 98 lines — the earlier, blind Tier 2 sizing pass cited this number without pasting the file;
//! confirmed by reading it directly). This is *not* the `WorldUtils`/`Actions`/`Modifiers` shape
//! DSL the original sizing guess assumed Tier 2 needed — see `plan.md`'s correction. It is a small,
//! standalone rectangle tracker, and nothing more: floating islands avoiding each other, biome set
//! pieces avoiding the dungeon, and similar "don't place two things on top of each other" checks
//! all go through this one type in vanilla.
//!
//! **One deliberate omission.** Vanilla's version guards every method body with a `lock` because
//! world generation there can run in parallel across worker threads. This engine's generator is
//! single-threaded (`worldgen::build` runs the whole pipeline on one call stack), so the lock is
//! dead weight here rather than a safety requirement — omitted rather than transcribed, the same
//! judgment call `liquid_settle.rs`'s doc comment makes for its own vanilla-parallelism omissions.
//!
//! Not wired into `build()` — there is nothing to call this yet. It exists so the first real Tier
//! 2 pass has something to build against instead of inventing its own overlap check.

use crate::world::World;

/// An axis-aligned tile rectangle. Vanilla uses XNA's `Rectangle`; this is the same shape with
/// only what `StructureMap` and `ShapeData` actually use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    /// `Rectangle.Inflate`: grows (or shrinks, for a negative `amount`) by `amount` on every side,
    /// keeping the same center.
    pub fn inflated(&self, amount: i32) -> Rect {
        Rect::new(
            self.x - amount,
            self.y - amount,
            self.width + amount * 2,
            self.height + amount * 2,
        )
    }

    /// `Rectangle.Intersects`.
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}

/// `TileID.Sets.GeneralPlacementTiles` (`TileID.cs:311`) — `StructureMap::CanPlace`'s default
/// "already-active tile is acceptable to place over" set. `CreateBoolSet(true, ...)` in vanilla
/// means the *listed* ids are the exceptions (`false`); every other type defaults to `true`.
pub fn general_placement_tile(tile: u16) -> bool {
    !matches!(
        tile,
        225 | 41
            | 481
            | 43
            | 482
            | 44
            | 483
            | 226
            | 203
            | 112
            | 25
            | 70
            | 151
            | 21
            | 31
            | 696
            | 467
            | 12
            | 665
            | 639
            | 138
            | 664
            | 711
            | 712
            | 713
            | 714
            | 715
            | 716
    )
}

/// Transcribed from `StructureMap`.
#[derive(Debug, Default)]
pub struct StructureMap {
    /// Every placed structure's rectangle. Informational only — `can_place` does not check
    /// against this list, only `protected`, matching vanilla exactly (a surprising asymmetry in
    /// the source, not a transcription slip: `AddStructure` never blocks a later placement).
    structures: Vec<Rect>,
    /// Rectangles that actually gate `can_place`.
    protected: Vec<Rect>,
}

impl StructureMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// `CanPlace(area, padding)`, using [`general_placement_tile`] as the default valid-tiles set.
    pub fn can_place(&self, world: &World, area: Rect, padding: i32) -> bool {
        self.can_place_with(world, area, padding, general_placement_tile)
    }

    /// `CanPlace(area, validTiles, padding)`. `valid` answers "is it fine for the area to already
    /// hold an active tile of this type" — anything else active inside the (padded) area fails
    /// the check, the same way an occupied rectangle in `protected` does.
    pub fn can_place_with(
        &self,
        world: &World,
        area: Rect,
        padding: i32,
        valid: impl Fn(u16) -> bool,
    ) -> bool {
        if area.x < 0
            || area.y < 0
            || area.right() > world.width() - 1
            || area.bottom() > world.height() - 1
        {
            return false;
        }
        let padded = area.inflated(padding);
        if self.protected.iter().any(|p| padded.intersects(p)) {
            return false;
        }
        for x in padded.x..padded.right() {
            for y in padded.y..padded.bottom() {
                let tile = world.tile(x, y);
                if tile.is_active() && !valid(tile.block) {
                    return false;
                }
            }
        }
        true
    }

    /// The union of every structure ever added via [`Self::add_structure`] or
    /// [`Self::add_protected_structure`]. `None` where vanilla returns `Rectangle.Empty` — no
    /// structures recorded yet.
    pub fn bounding_box(&self) -> Option<Rect> {
        self.structures.iter().copied().reduce(|acc, r| {
            let left = acc.x.min(r.x);
            let top = acc.y.min(r.y);
            let right = acc.right().max(r.right());
            let bottom = acc.bottom().max(r.bottom());
            Rect::new(left, top, right - left, bottom - top)
        })
    }

    /// Records a structure for [`Self::bounding_box`]. Does **not** block a later `can_place` —
    /// see the field doc on `structures`. Use [`Self::add_protected_structure`] for that.
    pub fn add_structure(&mut self, area: Rect, padding: i32) {
        self.structures.push(area.inflated(padding));
    }

    /// Records a structure that also blocks later placement until [`Self::reset`].
    pub fn add_protected_structure(&mut self, area: Rect, padding: i32) {
        let inflated = area.inflated(padding);
        self.structures.push(inflated);
        self.protected.push(inflated);
    }

    /// Clears `protected` only — `structures` (and so `bounding_box`) survives, matching vanilla.
    pub fn reset(&mut self) {
        self.protected.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::worldgen::tiles;

    fn small_world() -> World {
        World::empty(200, 200, "structure_map")
    }

    #[test]
    fn an_unprotected_structure_does_not_block_a_later_placement() {
        let world = small_world();
        let mut map = StructureMap::new();
        map.add_structure(Rect::new(10, 10, 20, 20), 0);
        assert!(map.can_place(&world, Rect::new(15, 15, 5, 5), 0));
    }

    #[test]
    fn a_protected_structure_blocks_an_overlapping_placement() {
        let world = small_world();
        let mut map = StructureMap::new();
        map.add_protected_structure(Rect::new(10, 10, 20, 20), 0);
        assert!(!map.can_place(&world, Rect::new(15, 15, 5, 5), 0));
        // Clear of it entirely: fine.
        assert!(map.can_place(&world, Rect::new(100, 100, 5, 5), 0));
    }

    #[test]
    fn padding_widens_what_a_protected_structure_blocks() {
        let world = small_world();
        let mut map = StructureMap::new();
        map.add_protected_structure(Rect::new(10, 10, 10, 10), 0);
        // Just outside the raw rectangle...
        assert!(map.can_place(&world, Rect::new(20, 10, 5, 5), 0));
        // ...but the same probe with padding on the stored structure now overlaps.
        let mut padded_map = StructureMap::new();
        padded_map.add_protected_structure(Rect::new(10, 10, 10, 10), 5);
        assert!(!padded_map.can_place(&world, Rect::new(20, 10, 5, 5), 0));
    }

    #[test]
    fn reset_clears_protection_but_not_the_bounding_box() {
        let world = small_world();
        let mut map = StructureMap::new();
        map.add_protected_structure(Rect::new(10, 10, 20, 20), 0);
        map.reset();
        assert!(map.can_place(&world, Rect::new(15, 15, 5, 5), 0));
        assert_eq!(map.bounding_box(), Some(Rect::new(10, 10, 20, 20)));
    }

    #[test]
    fn bounding_box_is_the_union_of_every_structure() {
        let mut map = StructureMap::new();
        assert_eq!(map.bounding_box(), None);
        map.add_structure(Rect::new(0, 0, 10, 10), 0);
        map.add_structure(Rect::new(50, 5, 10, 10), 0);
        assert_eq!(map.bounding_box(), Some(Rect::new(0, 0, 60, 15)));
    }

    #[test]
    fn an_active_tile_outside_the_valid_set_blocks_placement() {
        let mut world = small_world();
        world.set_tile(15, 15, terrustia_proto::Tile::block(tiles::LIHZAHRD_BRICK));
        let map = StructureMap::new();
        // Lihzahrd brick (226) is one of GeneralPlacementTiles' exceptions.
        assert!(!map.can_place(&world, Rect::new(10, 10, 10, 10), 0));
    }

    #[test]
    fn an_active_ordinary_tile_does_not_block_placement() {
        let mut world = small_world();
        world.set_tile(15, 15, terrustia_proto::Tile::block(tiles::DIRT));
        let map = StructureMap::new();
        assert!(map.can_place(&world, Rect::new(10, 10, 10, 10), 0));
    }

    #[test]
    fn a_future_pass_reserves_then_a_conflicting_second_pass_backs_off() {
        // Illustrative: this is the actual use this type exists for. A first pass claims an area;
        // a second, unrelated pass checks before placing and steps aside instead of overlapping.
        let world = World::empty(4200, 1200, "structure_map");
        let mut map = StructureMap::new();
        let floating_island = Rect::new(500, 100, 60, 30);
        assert!(map.can_place(&world, floating_island, 20));
        map.add_protected_structure(floating_island, 20);

        let candidate_two = Rect::new(520, 110, 60, 30);
        assert!(
            !map.can_place(&world, candidate_two, 20),
            "overlaps the first island"
        );

        let candidate_three = Rect::new(700, 100, 60, 30);
        assert!(
            map.can_place(&world, candidate_three, 20),
            "clear of the first island"
        );
    }
}
