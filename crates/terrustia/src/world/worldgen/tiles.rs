//! The tile and wall ids the generator places, named.
//!
//! Every one is checked against `TileID` in the decompiled game rather than remembered. A wrong
//! id here does not fail: it builds a world that looks nearly right and cannot be played, which
//! is much harder to notice than a crash.

pub const DIRT: u16 = 0;
pub const STONE: u16 = 1;
pub const GRASS: u16 = 2;
pub const PLANTS: u16 = 3;
pub const TORCH: u16 = 4;
pub const IRON: u16 = 6;
pub const COPPER: u16 = 7;
pub const GOLD: u16 = 8;
pub const SILVER: u16 = 9;
pub const HEART: u16 = 12;
pub const CHEST: u16 = 21;
pub const DEMONITE: u16 = 22;
pub const CORRUPT_GRASS: u16 = 23;
pub const EBONSTONE: u16 = 25;
pub const DEMON_ALTAR: u16 = 26;
pub const SHADOW_ORB: u16 = 31;
pub const CLAY: u16 = 40;
pub const BLUE_DUNGEON_BRICK: u16 = 41;
pub const GREEN_DUNGEON_BRICK: u16 = 43;
pub const PINK_DUNGEON_BRICK: u16 = 44;
pub const SPIKES: u16 = 48;
pub const BANNERS: u16 = 91;
/// `GoldBrick` — one of the five jungle-shrine wood/brick materials `GenVars.jungleHut` rolls.
pub const GOLD_BRICK: u16 = 45;
pub const COBWEB: u16 = 51;
pub const SAND: u16 = 53;
pub const OBSIDIAN: u16 = 56;
pub const ASH: u16 = 57;
pub const HELLSTONE: u16 = 58;
pub const MUD: u16 = 59;
pub const JUNGLE_GRASS: u16 = 60;
pub const SAPPHIRE: u16 = 63;
pub const RUBY: u16 = 64;
pub const EMERALD: u16 = 65;
pub const TOPAZ: u16 = 66;
pub const AMETHYST: u16 = 67;
pub const DIAMOND: u16 = 68;
pub const MUSHROOM_GRASS: u16 = 70;
pub const EBONSAND: u16 = 112;
/// The other four jungle-shrine materials `GenVars.jungleHut` rolls, alongside [`GOLD_BRICK`].
pub const IRIDESCENT_BRICK: u16 = 119;
pub const MUDSTONE: u16 = 120;
pub const SILT: u16 = 123;
pub const SNOW: u16 = 147;
pub const ICE: u16 = 161;
pub const CRIMSON_GRASS: u16 = 199;
pub const CRIMSTONE: u16 = 203;
pub const CRIMTANE: u16 = 204;
pub const RICH_MAHOGANY: u16 = 158;
pub const TIN_BRICK: u16 = 175;
pub const LARVA: u16 = 231;
pub const CRIMSAND: u16 = 234;
pub const HIVE: u16 = 225;
pub const HONEY_BLOCK: u16 = 229;
pub const LIHZAHRD_BRICK: u16 = 226;
pub const MARBLE: u16 = 367;
pub const GRANITE: u16 = 368;
pub const SANDSTONE: u16 = 396;
pub const HARDENED_SAND: u16 = 397;
/// `TileID.SandstoneBrick` — the pyramid's own worked-stone material, distinct from the natural
/// desert [`SANDSTONE`] (396) it shares a name with in English but not in `TileID`.
pub const SANDSTONE_BRICK: u16 = 151;
/// `TileID.Cloud` — the floating island's own material, both islands (`CloudIsland`) and lakes
/// (`CloudLake`).
pub const CLOUD: u16 = 189;
/// `TileID.Sunplate` — the floating island house's own build material (`IslandHouse`,
/// `WorldGen.cs:80394`).
pub const SUNPLATE: u16 = 202;

/// The wall ids, from `WallID`.
pub mod walls {
    pub const STONE: u16 = 1;
    pub const DIRT: u16 = 2;
    pub const EBONSTONE: u16 = 3;
    pub const BLUE_DUNGEON: u16 = 7;
    pub const GREEN_DUNGEON: u16 = 8;
    pub const PINK_DUNGEON: u16 = 9;
    pub const OBSIDIAN_BACK: u16 = 14;
    pub const MUD: u16 = 15;
    pub const JUNGLE: u16 = 64;
    pub const FLOWER: u16 = 63;
    pub const SNOW: u16 = 40;
    pub const ICE: u16 = 71;
    pub const CRIMSTONE: u16 = 83;
    pub const CAVE: u16 = 61;
    /// `WallID.Sandstone` — the natural desert wall. Distinct from [`SANDSTONE_BRICK`], the
    /// pyramid's own worked interior wall.
    pub const SANDSTONE: u16 = 187;
    /// `WallID.SandstoneBrick` — the pyramid's interior wall (`WorldGen.cs`'s `Pyramid()` sets
    /// `wall = 34` throughout its own carving).
    pub const SANDSTONE_BRICK: u16 = 34;
    pub const LIHZAHRD_BRICK: u16 = 87;
    /// The unsafe hive wall — `WallID.HiveUnsafe`, used only as a clearance-scan exclusion (a
    /// jungle shrine refuses to site near it) alongside [`LIHZAHRD_BRICK`] above.
    pub const HIVE: u16 = 86;
    pub const HARDENED_SAND: u16 = 216;
    /// The five wall materials matching `tiles::GOLD_BRICK`/`IRIDESCENT_BRICK`/`MUDSTONE`/
    /// `RICH_MAHOGANY`/`TIN_BRICK`, in `GenVars.jungleHut`'s own roll order (`WorldGen.cs:11345`).
    pub const GOLD_BRICK: u16 = 10;
    pub const IRIDESCENT_BRICK: u16 = 23;
    pub const MUDSTONE_BRICK: u16 = 24;
    pub const RICH_MAHOGANY: u16 = 42;
    pub const TIN_BRICK: u16 = 45;
    /// The six gem walls `Spread.Gem` (`WorldGen.cs:3592`) rolls from, `48 + randGem()` — in the
    /// same 0-5 index order `randGemTile` (`WorldGen.cs:9707`) uses for its matching tile: 0
    /// amethyst, 1 topaz, 2 sapphire, 3 emerald, 4 ruby, 5 diamond.
    pub const GEM_WALLS: [u16; 6] = [48, 49, 50, 51, 52, 53];
    /// `WallID.Cloud` — the background a floating island (and its lake variant) gets filled with
    /// once fully enclosed (`WorldGen.cs:79513`/`:79939`).
    pub const CLOUD: u16 = 73;
    /// `WallID.DiscWall` — the floating island house's own interior wall (`IslandHouse`,
    /// `WorldGen.cs:80395`; the real name is a holdover from a cut item, not a description of what
    /// it looks like here).
    pub const SUNPLATE: u16 = 82;
}
