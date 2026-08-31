//! Town NPC happiness: the price multiplier a resident quotes the player he is talking to.
//!
//! Transcribed from `Terraria.GameContent/ShopHelper.cs` and the profiles in
//! `Terraria.GameContent.Personalities/`. Vanilla folds four things into one number: whether the
//! resident has a home and is standing near it, how crowded that home is, which biome the *shopper*
//! is standing in, and which neighbours live within twenty-five tiles of the resident's front door.
//!
//! **Where this number is read is a seam worth stating plainly.** In release 326 the price a player
//! pays is computed entirely on the client. `ShopHelper` is only ever reached through
//! `Player.SetTalkNPC` (`Player.cs:4360-4375`), and every reader of the resulting
//! `currentShoppingSettings.PriceAdjustment` is gated on `Main.LocalPlayer` / `Main.myPlayer`:
//! the shop tooltip (`Main.cs:40799`, `:42511`, `:44544`), the buy and sell arithmetic
//! (`Player.GetItemExpectedPrice`, `Player.cs:35638-35677`), the Tax Collector's payout
//! (`Main.cs:40631`, `:40884`) and the Angler's reward roll (`Player.cs:56971`). No price and no
//! shop inventory crosses the wire for a town NPC, so a dedicated server cannot change what a
//! client charges. The server *does* run the same calculation (`MessageBuffer.cs:2264` hands
//! packet 40 to `SetTalkNPC` under `netMode == 2`) and then never reads the answer. This module
//! exists so that this server computes the same number vanilla's own server computes, which is
//! what makes it checkable against a real client's happiness text; it is not a lever on the client.
//!
//! Deliberate narrowings, each disclosed at its site below: no love potion (`npc.loveStruck`,
//! `ShopHelper.cs:103-106`) because nothing in this server applies one, and no happiness *report*
//! text, because that is localized game text and none of it is checked into this repository.
//!
//! `ShopHelper.GetSkeletonMerchantPrices` (`ShopHelper.cs:64-88`) and
//! `GetTravelingMerchantPrices` (`:90-97`) are dead code in this build: both are private and
//! nothing calls them. They are not transcribed.

/// How strongly a resident feels about a biome or a neighbour.
///
/// `Terraria.GameContent.Personalities/AffectionLevel.cs:3-9`. The backing numbers are the game's
/// own, and they are ordered rather than arbitrary: a biome list picks the *highest* affection
/// among the biomes the shopper is standing in (`BiomePreferenceListTrait.cs:44`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Affection {
    Hate = -100,
    Dislike = -50,
    Like = 50,
    Love = 100,
}

impl Affection {
    /// What this feeling does to the price. `ShopHelper.cs:35-41`, applied at `:203-291`.
    pub fn multiplier(self) -> f32 {
        match self {
            Affection::Love => 0.88,
            Affection::Like => 0.94,
            Affection::Dislike => 1.06,
            Affection::Hate => 1.12,
        }
    }
}

/// The lowest and highest a multiplier may end up. `ShopHelper.cs:12`, `:16`, applied at `:182`.
pub const LOWEST_MULTIPLIER: f32 = 0.75;
pub const HIGHEST_MULTIPLIER: f32 = 1.5;

/// The multiplier at or below which vanilla counts a resident as fully happy.
///
/// `ShopHelper.cs:14`, tested at `Player.cs:4376`. In this build it gates one achievement and
/// nothing else; in particular it does **not** gate pylons, whatever older builds did.
pub const MAX_HAPPINESS_MULTIPLIER: f32 = 0.82;

/// The shopping zones `ShopHelper` reads off the player it is quoting a price to.
///
/// These are `Player`'s own zone flags (`ForestBiome.cs` and its siblings each read one), and they
/// are independent: a spot can be several at once, which is why this is a set of flags rather than
/// one biome. `Player.cs:3807-3831` derives the two that are not raw zone flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Zones {
    /// `Player.ZoneBeach`, which the game calls the Ocean for shopping purposes.
    pub ocean: bool,
    pub snow: bool,
    pub desert: bool,
    pub jungle: bool,
    pub hallow: bool,
    /// `Player.ZoneGlowshroom`.
    pub mushroom: bool,
    pub corruption: bool,
    pub crimson: bool,
    pub dungeon: bool,
    /// `Player.ShoppingZone_BelowSurface` (`Player.cs:3819`): the player's centre tile is below the
    /// world's surface line. Note this is a *depth*, not a biome, and it stacks with the others.
    pub below_surface: bool,
}

impl Zones {
    /// `Player.ShoppingZone_AnyBiome`, `Player.cs:3807-3817`.
    fn any_biome(self) -> bool {
        self.dungeon
            || self.corruption
            || self.crimson
            || self.mushroom
            || self.hallow
            || self.jungle
            || self.snow
            || self.ocean
            || self.desert
    }

    /// `Player.ShoppingZone_Forest`, `Player.cs:3821-3831`: the surface, with nothing else on it.
    pub fn forest(self) -> bool {
        !self.any_biome() && !self.below_surface
    }

    /// The three biomes that ruin any resident's mood outright, whoever they are.
    /// `ShopHelper._dangerousBiomes`, `ShopHelper.cs:28-33`, tested at `:353-368`.
    fn evil(self) -> bool {
        self.corruption || self.crimson || self.dungeon
    }
}

/// One biome a personality profile has an opinion about.
///
/// Only these eight ever appear in a profile; the evil three are handled by the blanket check
/// above and never as a preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    Ocean,
    Forest,
    Snow,
    Desert,
    Jungle,
    Underground,
    Hallow,
    Mushroom,
}

impl Biome {
    fn present_in(self, zones: Zones) -> bool {
        match self {
            Biome::Ocean => zones.ocean,
            Biome::Forest => zones.forest(),
            Biome::Snow => zones.snow,
            Biome::Desert => zones.desert,
            Biome::Jungle => zones.jungle,
            // `UndergroundBiome.cs:12` reads `ShoppingZone_BelowSurface`, not a cavern zone.
            Biome::Underground => zones.below_surface,
            Biome::Hallow => zones.hallow,
            Biome::Mushroom => zones.mushroom,
        }
    }
}

/// A town resident, as the price calculation needs to see one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resident {
    pub npc_type: u16,
    /// Where this one lives, in tiles, or `None` when homeless.
    pub home: Option<(i32, i32)>,
    /// Where this one is standing right now, in tiles (the game's `npc.Center / 16f`).
    pub center: (f32, f32),
}

impl Resident {
    /// The point the crowding scan measures from: home, or wherever a homeless one is standing.
    /// `ShopHelper.cs:298-302` for the shopper, `:312-316` for each neighbour.
    fn anchor(&self) -> (f32, f32) {
        match self.home {
            Some((x, y)) => (x as f32, y as f32),
            None => self.center,
        }
    }
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// The Princess. She is the only resident whose crowding rules differ. `NPCID.Princess`.
const PRINCESS: u16 = 663;

/// Types that look like town NPCs but are not really residents: the Old Man, the Travelling
/// Merchant and the Skeleton Merchant. `ShopHelper.IsNotReallyTownNPC`, `ShopHelper.cs:370-378`.
fn not_really_a_town_npc(npc_type: u16) -> bool {
    matches!(npc_type, 37 | 368 | 453)
}

/// `NPCID.Sets.IsTownPet`, `NPCID.cs:4446`: cats, dogs, bunnies and the town slimes.
///
/// This set is exactly the set of types the game gives `housingCategory = 1`
/// (`NPC.cs:17256`, `:17340`, `:17526`, `:17639`), which is what makes it do double duty below:
/// a pet has no opinion of its own (`ShopHelper.cs:107`), and because the crowding scan only
/// counts neighbours of the *same* housing category (`ShopHelper.cs:310` inverts
/// `TownRoomManager.CanNPCsLiveWithEachOther`, `TownRoomManager.cs:151-161`) and the shopper is
/// never a pet by the time that scan runs, a pet never crowds anybody either.
fn is_town_pet(npc_type: u16) -> bool {
    matches!(npc_type, 637 | 638 | 656 | 670 | 678..=684)
}

/// Which biomes each resident loves, likes, dislikes or hates.
///
/// `PersonalityDatabasePopulator.Populate_BiomePreferences`, `PersonalityDatabasePopulator.cs:27-147`,
/// in that file's order. Residents absent from this table have no biome opinion at all.
const BIOME_PREFERENCES: &[(u16, &[(Affection, Biome)])] = {
    use Affection::{Dislike, Hate, Like, Love};
    use Biome::{Desert, Forest, Hallow, Jungle, Mushroom, Ocean, Snow, Underground};
    &[
        (22, &[(Like, Forest), (Dislike, Ocean)]),        // Guide
        (17, &[(Like, Forest), (Dislike, Desert)]),       // Merchant
        (588, &[(Like, Forest), (Dislike, Underground)]), // Golfer
        (633, &[(Like, Forest), (Dislike, Desert)]),      // Bestiary Girl
        (441, &[(Like, Snow), (Dislike, Hallow)]),        // Tax Collector
        (124, &[(Like, Snow), (Dislike, Underground)]),   // Mechanic
        (209, &[(Like, Snow), (Dislike, Jungle)]),        // Cyborg
        (142, &[(Love, Snow), (Hate, Desert)]),           // Santa Claus
        (207, &[(Like, Desert), (Dislike, Forest)]),      // Dye Trader
        (19, &[(Like, Desert), (Dislike, Snow)]),         // Arms Dealer
        (178, &[(Like, Desert), (Dislike, Jungle)]),      // Steampunker
        (20, &[(Like, Jungle), (Dislike, Desert)]),       // Dryad
        (228, &[(Like, Jungle), (Dislike, Hallow)]),      // Witch Doctor
        (227, &[(Like, Jungle), (Dislike, Forest)]),      // Painter
        (369, &[(Like, Ocean), (Dislike, Desert)]),       // Angler
        (229, &[(Like, Ocean), (Dislike, Underground)]),  // Pirate
        (353, &[(Like, Ocean), (Dislike, Snow)]),         // Stylist
        (38, &[(Like, Underground), (Dislike, Ocean)]),   // Demolitionist
        (107, &[(Like, Underground), (Dislike, Jungle)]), // Goblin Tinkerer
        (54, &[(Like, Underground), (Dislike, Hallow)]),  // Clothier
        (108, &[(Like, Hallow), (Dislike, Ocean)]),       // Wizard
        (18, &[(Like, Hallow), (Dislike, Snow)]),         // Nurse
        (208, &[(Like, Hallow), (Dislike, Underground)]), // Party Girl
        (550, &[(Like, Hallow), (Dislike, Snow)]),        // Tavernkeep
        (160, &[(Like, Mushroom)]),                       // Truffle
    ]
};

/// Which neighbours each resident loves, likes, dislikes or hates.
///
/// `AllPersonalitiesModifier.ModifyShopPrice_Relationships`, `AllPersonalitiesModifier.cs:41-505`,
/// kept in that switch's order because floating-point multiplication is not associative and the
/// order the game applies them in is the order that reproduces its number exactly.
///
/// The Princess (663) is absent on purpose: her side of the relationship is not a fixed list, and
/// is handled in [`price_multiplier`] from `AllPersonalitiesModifier.cs:15-40`.
const NPC_PREFERENCES: &[(u16, &[(u16, Affection)])] = {
    use Affection::{Dislike, Hate, Like, Love};
    &[
        // Merchant
        (17, &[(588, Like), (18, Like), (441, Dislike), (369, Hate)]),
        // Nurse
        (
            18,
            &[
                (19, Love),
                (108, Like),
                (208, Dislike),
                (20, Dislike),
                (633, Hate),
            ],
        ),
        // Painter
        (
            227,
            &[(20, Love), (208, Like), (209, Dislike), (160, Dislike)],
        ),
        // Dye Trader
        (207, &[(19, Like), (227, Like), (178, Dislike), (229, Hate)]),
        // Party Girl
        (
            208,
            &[
                (108, Love),
                (353, Like),
                (17, Dislike),
                (441, Hate),
                (633, Love),
            ],
        ),
        // Angler
        (369, &[(208, Like), (38, Like), (441, Like), (550, Hate)]),
        // Stylist
        (
            353,
            &[(207, Love), (229, Like), (550, Dislike), (107, Hate)],
        ),
        // Demolitionist
        (
            38,
            &[(550, Love), (124, Like), (107, Dislike), (19, Dislike)],
        ),
        // Dryad
        (20, &[(228, Like), (160, Like), (369, Dislike), (588, Hate)]),
        // Tavernkeep
        (550, &[(38, Love), (107, Like), (22, Dislike), (207, Hate)]),
        // Arms Dealer
        (19, &[(18, Love), (178, Like), (588, Dislike), (38, Hate)]),
        // Goblin Tinkerer
        (107, &[(124, Love), (207, Like), (54, Dislike), (353, Hate)]),
        // Witch Doctor
        (228, &[(20, Like), (22, Like), (18, Dislike), (160, Hate)]),
        // Clothier
        (54, &[(160, Love), (441, Like), (18, Dislike), (124, Hate)]),
        // Mechanic
        (124, &[(107, Love), (209, Like), (19, Dislike), (54, Hate)]),
        // Tax Collector
        (
            441,
            &[
                (17, Love),
                (208, Like),
                (38, Dislike),
                (124, Dislike),
                (142, Hate),
            ],
        ),
        // Pirate
        (229, &[(369, Love), (550, Like), (353, Dislike), (22, Hate)]),
        // Wizard
        (108, &[(588, Love), (17, Like), (228, Dislike), (209, Hate)]),
        // Steampunker
        (
            178,
            &[
                (209, Love),
                (227, Like),
                (208, Dislike),
                (108, Dislike),
                (20, Dislike),
            ],
        ),
        // Cyborg
        (
            209,
            &[
                (353, Like),
                (229, Like),
                (178, Like),
                (108, Hate),
                (633, Dislike),
            ],
        ),
        // Santa Claus
        (142, &[(441, Hate)]),
        // Golfer
        (
            588,
            &[
                (227, Like),
                (369, Love),
                (17, Hate),
                (229, Dislike),
                (633, Like),
            ],
        ),
        // Guide
        (22, &[(54, Like), (178, Dislike), (227, Hate), (633, Like)]),
        // Truffle
        (160, &[(22, Love), (20, Like), (54, Dislike), (228, Hate)]),
        // Bestiary Girl
        (633, &[(369, Dislike), (19, Hate), (228, Love), (588, Like)]),
    ]
};

/// The price multiplier `npc` quotes a player standing in `zones`.
///
/// `others` is every other live town NPC in the world; this function does the distance filtering
/// itself, exactly as `ShopHelper.GetNearbyResidentNPCs` does (`ShopHelper.cs:293-330`). `remix`
/// is the world's `Main.remixWorld` seed flag, which switches the whole system off
/// (`ShopHelper.cs:107`).
///
/// Follows `ShopHelper.ProcessMood` (`ShopHelper.cs:99-178`) step for step.
pub fn price_multiplier(npc: &Resident, others: &[Resident], zones: Zones, remix: bool) -> f32 {
    // `ShopHelper.cs:103-106` also multiplies by 0.9 for `npc.loveStruck` here. Nothing in this
    // server applies a Love Potion, so there is no flag to read and the clause is left out rather
    // than faked with a constant `false`.
    let mut adjustment = 1.0f32;

    // `ShopHelper.cs:107-110`. Note the early return skips the clamp at the end, so these types
    // land on exactly 1.0 rather than on the clamp's floor.
    if remix || is_town_pet(npc.npc_type) || not_really_a_town_npc(npc.npc_type) {
        return adjustment;
    }

    // `ShopHelper.cs:111-122`. Each of these *assigns* 1000 rather than multiplying, and later
    // discounts still apply on top; the final clamp is what turns 1000 into 1.5.
    let far_from_home = npc
        .home
        .is_some_and(|(x, y)| distance((x as f32, y as f32), npc.center) > 120.0);
    if npc.home.is_none() || far_from_home {
        adjustment = 1000.0;
    }
    if adjustment < 1000.0 && zones.evil() {
        adjustment = 1000.0;
    }

    // `ShopHelper.GetNearbyResidentNPCs`, `ShopHelper.cs:293-330`. Twenty-five tiles is "in this
    // house", a hundred and twenty is "in this village".
    let anchor = npc.anchor();
    let mut in_house = 0usize;
    let mut in_village = 0usize;
    let mut neighbours: Vec<u16> = Vec::new();
    for other in others {
        if not_really_a_town_npc(other.npc_type) || is_town_pet(other.npc_type) {
            continue;
        }
        let gap = distance(anchor, other.anchor());
        if gap < 25.0 {
            neighbours.push(other.npc_type);
            in_house += 1;
        } else if gap < 120.0 {
            in_village += 1;
        }
    }

    // `ShopHelper.cs:124-155`. The Princess pays no crowding penalty and gets no solitude bonus,
    // but is the only one who is miserable when left alone.
    let princess = npc.npc_type == PRINCESS;
    let crowding_step: f32 = if princess { 1.0 } else { 1.05 };
    if princess && in_house < 2 && in_village < 2 {
        adjustment = 1000.0;
    }
    if in_house > 3 {
        for _ in 3..in_house {
            adjustment *= crowding_step;
        }
    }
    if !princess && in_house <= 2 && in_village < 4 {
        adjustment *= 0.95;
    }

    // The profile's biome list: the strongest feeling among the biomes the shopper is standing in
    // wins, and only that one applies. `BiomePreferenceListTrait.cs:38-53`.
    if let Some((_, preferences)) = BIOME_PREFERENCES.iter().find(|(id, _)| *id == npc.npc_type)
        && let Some((affection, _)) = preferences
            .iter()
            .filter(|(_, biome)| biome.present_in(zones))
            .max_by_key(|(affection, _)| *affection)
    {
        adjustment *= affection.multiplier();
    }

    // `AllPersonalitiesModifier.cs:15-36`: the Princess loves up to three of her neighbours,
    // picked at random. Every pick is a Love, so the *multiplier* is deterministic even though the
    // names she says are not, and this server does not produce the names.
    if princess {
        let mut distinct: Vec<u16> = neighbours.clone();
        distinct.sort_unstable();
        distinct.dedup();
        for _ in 0..distinct.len().min(3) {
            adjustment *= Affection::Love.multiplier();
        }
    }
    // `AllPersonalitiesModifier.cs:37-40`: everybody else likes having her around.
    if !princess && neighbours.contains(&PRINCESS) {
        adjustment *= Affection::Like.multiplier();
    }
    if let Some((_, preferences)) = NPC_PREFERENCES.iter().find(|(id, _)| *id == npc.npc_type) {
        for (other, affection) in *preferences {
            if neighbours.contains(other) {
                adjustment *= affection.multiplier();
            }
        }
    }

    limit_and_round(adjustment)
}

/// `ShopHelper.LimitAndRoundMultiplier`, `ShopHelper.cs:180-185`.
///
/// One disclosed difference: C#'s `Math.Round` rounds a midpoint to even and Rust's `f32::round`
/// rounds it away from zero. Reaching an exact midpoint would take a product of the 0.88 / 0.94 /
/// 0.95 / 1.05 / 1.06 / 1.12 factors that lands precisely on a half of a hundredth, which none of
/// them do; the two agree on every reachable value.
fn limit_and_round(adjustment: f32) -> f32 {
    (adjustment.clamp(LOWEST_MULTIPLIER, HIGHEST_MULTIPLIER) * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn housed(npc_type: u16, home: (i32, i32)) -> Resident {
        Resident {
            npc_type,
            home: Some(home),
            center: (home.0 as f32, home.1 as f32),
        }
    }

    /// A resident alone in a quiet forest gets the solitude bonus and nothing else.
    ///
    /// `ShopHelper.cs:151-155` (`LoveSpace`, 0.95); the Guide also likes the Forest
    /// (`PersonalityDatabasePopulator.cs:27-31`, 0.94). 0.95 * 0.94 = 0.893, rounded to 0.89.
    #[test]
    fn a_lone_guide_in_a_forest() {
        let guide = housed(22, (500, 100));
        let zones = Zones::default();
        assert!(zones.forest());
        assert_eq!(price_multiplier(&guide, &[], zones, false), 0.89);
    }

    /// Homelessness ruins the mood outright, and the clamp is what caps the ruin.
    /// `ShopHelper.cs:111-114`, `:180-185`.
    #[test]
    fn homeless_is_capped_at_the_ceiling() {
        let stray = Resident {
            npc_type: 22,
            home: None,
            center: (500.0, 100.0),
        };
        assert_eq!(
            price_multiplier(&stray, &[], Zones::default(), false),
            HIGHEST_MULTIPLIER
        );
    }

    /// So does wandering more than 120 tiles from a home that exists. `ShopHelper.cs:341-351`.
    #[test]
    fn far_from_home_is_capped_at_the_ceiling() {
        let wanderer = Resident {
            npc_type: 22,
            home: Some((500, 100)),
            center: (700.0, 100.0),
        };
        assert_eq!(
            price_multiplier(&wanderer, &[], Zones::default(), false),
            HIGHEST_MULTIPLIER
        );
    }

    /// An evil biome under the *shopper's* feet ruins it too. `ShopHelper.cs:119-122`, `:353-368`.
    #[test]
    fn a_shopper_standing_in_the_corruption_pays_the_ceiling() {
        let guide = housed(22, (500, 100));
        let corrupt = Zones {
            corruption: true,
            ..Zones::default()
        };
        assert_eq!(price_multiplier(&guide, &[], corrupt, false), 1.5);
    }

    /// Crowding costs 1.05 per resident past the third, and nothing at all up to it.
    /// `ShopHelper.cs:136-150`.
    #[test]
    fn crowding_only_bites_past_three_housemates() {
        let guide = housed(22, (500, 100));
        // Three housemates: no penalty, but also no solitude bonus (that wants two or fewer).
        // Only the Forest like applies.
        let three: Vec<Resident> = (0..3).map(|i| housed(100 + i, (501, 100))).collect();
        assert_eq!(
            price_multiplier(&guide, &three, Zones::default(), false),
            0.94
        );
        // A fifth resident is two past the third, so 1.05 twice: 0.94 * 1.1025 = 1.036 -> 1.04.
        let five: Vec<Resident> = (0..5).map(|i| housed(100 + i, (501, 100))).collect();
        assert_eq!(
            price_multiplier(&guide, &five, Zones::default(), false),
            1.04
        );
    }

    /// Town pets never crowd anybody: the scan only counts neighbours of the same housing
    /// category. `ShopHelper.cs:310`, `TownRoomManager.cs:151-161`, `NPCID.cs:4446`.
    #[test]
    fn pets_do_not_crowd() {
        let guide = housed(22, (500, 100));
        let pets: Vec<Resident> = [637, 638, 656, 670, 678]
            .iter()
            .map(|t| housed(*t, (501, 100)))
            .collect();
        // Still alone as far as the calculation is concerned: solitude bonus and Forest like.
        assert_eq!(
            price_multiplier(&guide, &pets, Zones::default(), false),
            0.89
        );
    }

    /// A pet has no opinion of its own and returns before the clamp. `ShopHelper.cs:107-110`.
    #[test]
    fn a_pet_quotes_a_flat_price() {
        let cat = Resident {
            npc_type: 637,
            home: None,
            center: (0.0, 0.0),
        };
        assert_eq!(price_multiplier(&cat, &[], Zones::default(), false), 1.0);
    }

    /// So do the three that only look like residents. `ShopHelper.cs:370-378`.
    #[test]
    fn the_travelling_merchant_is_not_a_resident() {
        for npc_type in [37, 368, 453] {
            let visitor = Resident {
                npc_type,
                home: None,
                center: (0.0, 0.0),
            };
            assert_eq!(
                price_multiplier(&visitor, &[], Zones::default(), false),
                1.0,
                "type {npc_type} should not be moody"
            );
        }
    }

    /// A remix world switches the whole system off. `ShopHelper.cs:107`.
    #[test]
    fn remix_worlds_have_no_happiness() {
        let stray = Resident {
            npc_type: 22,
            home: None,
            center: (0.0, 0.0),
        };
        assert_eq!(price_multiplier(&stray, &[], Zones::default(), true), 1.0);
    }

    /// The biome list takes the strongest feeling, not the first.
    /// `BiomePreferenceListTrait.cs:40-52`. Santa loves the Snow and hates the Desert; a snowy
    /// desert is a love, because Love (100) outranks Hate (-100).
    #[test]
    fn the_strongest_biome_feeling_wins() {
        let santa = housed(142, (500, 100));
        let both = Zones {
            snow: true,
            desert: true,
            ..Zones::default()
        };
        // Solitude bonus 0.95, then Love 0.88: 0.836 -> 0.84.
        assert_eq!(price_multiplier(&santa, &[], both, false), 0.84);
    }

    /// A neighbour opinion applies when that neighbour lives within twenty-five tiles, and stops
    /// applying past it. `ShopHelper.cs:317-326`, `AllPersonalitiesModifier.cs:423-427`.
    #[test]
    fn santa_hates_the_tax_collector_next_door_but_not_across_town() {
        let santa = housed(142, (500, 100));
        let next_door = [housed(441, (510, 100))];
        // No solitude bonus is lost (one housemate is still two or fewer), Snow is not in play,
        // so this is the Hate alone: 0.95 * 1.12 = 1.064 -> 1.06.
        assert_eq!(
            price_multiplier(&santa, &next_door, Zones::default(), false),
            1.06
        );
        // Fifty tiles away is "village", not "house": the opinion does not apply, and four in the
        // village would be needed to lose the solitude bonus.
        let across_town = [housed(441, (550, 100))];
        assert_eq!(
            price_multiplier(&santa, &across_town, Zones::default(), false),
            0.95
        );
    }

    /// The Princess: alone she is miserable, and in company she loves up to three neighbours.
    /// `ShopHelper.cs:126-135`, `AllPersonalitiesModifier.cs:15-36`.
    #[test]
    fn the_princess_wants_company() {
        let princess = housed(PRINCESS, (500, 100));
        assert_eq!(
            price_multiplier(&princess, &[], Zones::default(), false),
            HIGHEST_MULTIPLIER
        );
        // Two neighbours, two Loves, no crowding penalty (her step is 1.0) and no solitude bonus:
        // 0.88^2 = 0.7744 -> 0.77.
        let two: Vec<Resident> = [22, 17].iter().map(|t| housed(*t, (501, 100))).collect();
        assert_eq!(
            price_multiplier(&princess, &two, Zones::default(), false),
            0.77
        );
        // Four neighbours: only three Loves are applied (0.88^3 = 0.681472), and the floor is
        // what stops her there.
        let four: Vec<Resident> = [22, 17, 18, 19]
            .iter()
            .map(|t| housed(*t, (501, 100)))
            .collect();
        assert_eq!(
            price_multiplier(&princess, &four, Zones::default(), false),
            LOWEST_MULTIPLIER
        );
    }

    /// Everybody else simply likes having her nearby. `AllPersonalitiesModifier.cs:37-40`.
    #[test]
    fn a_neighbour_likes_the_princess() {
        let guide = housed(22, (500, 100));
        let princess = [housed(PRINCESS, (501, 100))];
        // Solitude 0.95, Forest 0.94, Princess 0.94: 0.83942 -> 0.84.
        assert_eq!(
            price_multiplier(&guide, &princess, Zones::default(), false),
            0.84
        );
    }

    /// The floor holds. `ShopHelper.cs:12`, `:182`.
    #[test]
    fn the_multiplier_never_falls_below_the_floor() {
        // The Truffle in a mushroom biome, surrounded by everyone he loves and likes.
        let truffle = housed(160, (500, 100));
        let friends: Vec<Resident> = [22, 20].iter().map(|t| housed(*t, (501, 100))).collect();
        let mushroom = Zones {
            mushroom: true,
            ..Zones::default()
        };
        let quoted = price_multiplier(&truffle, &friends, mushroom, false);
        assert!(
            (LOWEST_MULTIPLIER..=HIGHEST_MULTIPLIER).contains(&quoted),
            "{quoted} outside the clamp"
        );
        assert_eq!(quoted, LOWEST_MULTIPLIER);
    }

    /// Every id in both tables is a real NPC in this build, and no id appears twice.
    #[test]
    fn the_tables_are_well_formed() {
        for (npc_type, preferences) in BIOME_PREFERENCES {
            assert!(
                crate::npc_data::npc_stats(*npc_type).is_some(),
                "biome table names unknown NPC {npc_type}"
            );
            assert!(!preferences.is_empty());
        }
        for (npc_type, preferences) in NPC_PREFERENCES {
            assert!(
                crate::npc_data::npc_stats(*npc_type).is_some(),
                "neighbour table names unknown NPC {npc_type}"
            );
            for (other, _) in *preferences {
                assert!(
                    crate::npc_data::npc_stats(*other).is_some(),
                    "NPC {npc_type} has an opinion of unknown NPC {other}"
                );
            }
        }
        let mut ids: Vec<u16> = BIOME_PREFERENCES.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "an NPC has two biome profiles");
        let mut ids: Vec<u16> = NPC_PREFERENCES.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "an NPC has two neighbour profiles");
    }
}
