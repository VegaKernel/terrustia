//! `crates/terrustia-proto/src/travel_shop.rs` — the Travelling Merchant's stock.
//!
//! `Chest.SetupTravelShop_GetItem` (`Terraria/Chest.cs`) is a chain of luck rolls, one per
//! candidate item, each overwriting the last, so the *final* match in the chain wins. Which
//! item, at which rarity tier, and under which world condition, is per-item data, parsed rather
//! than transcribed.
//!
//! Ported from `gen_travel_shop.py`.
//!
//! Not wired into [`crate::main`]'s `TABLES` yet: the `OFFERS` data is byte-identical, but a past
//! hand-edit (commit `65f4be3`) added a source comment and a regression test straight to the
//! committed `.rs` without adding either to this generator's `emit()`. Wiring this in as-is would
//! delete that regression test on the next regen. See the Codegen entry in `TODO.md`.

use std::path::Path;

use regex::Regex;

use crate::csharp::read_lossy;

/// The C# condition substring and the `Needs::` flag name it becomes, in the order the game's
/// own guard clauses are checked (which is also the order flags are OR'd together per entry).
const CONDITIONS: &[(&str, &str)] = &[
    ("Main.hardMode", "HARDMODE"),
    ("NPC.downedMechBossAny", "ANY_MECH"),
    ("NPC.downedMechBoss1", "DESTROYER"),
    ("NPC.downedMechBoss2", "TWINS"),
    ("NPC.downedMechBoss3", "PRIME"),
    ("NPC.downedBoss1", "EYE"),
    ("NPC.downedBoss3", "SKELETRON"),
    ("WorldGen.shadowOrbSmashed", "ORB_SMASHED"),
];

struct Entry {
    tier: i64,
    item: i64,
    needs: Vec<&'static str>,
    floor: i64,
}

pub fn generate(root: &Path) -> String {
    let chest_cs = read_lossy(&root.join("Terraria/Chest.cs"));

    let start = chest_cs
        .find("public static void SetupTravelShop_GetItem(")
        .expect("no SetupTravelShop_GetItem");
    let end = chest_cs[start..]
        .find("public static void SetupTravelShop()")
        .expect("no SetupTravelShop")
        + start;
    let body = &chest_cs[start..end];

    // `if ([minimumRarity <= F &&] playerWithHighestLuck.RollLuck(rarity[N]) == 0 [&& cond...])
    //     { it = ITEM; }`
    //
    // Two candidates carry their own leading `minimumRarity <= F &&` guard instead of relying on
    // an enclosing `if (minimumRarity > N) return;` checkpoint — they sit before any such
    // checkpoint in the function, so without their own guard they would be offered no matter how
    // high `minimumRarity` climbs. The optional leading group captures that inline floor
    // directly; every other candidate leaves it unmatched and keeps falling back to the
    // checkpoint-derived floor below.
    let pattern = Regex::new(
        r"if \((?:minimumRarity <= (\d+) && )?playerWithHighestLuck\.RollLuck\(rarity\[(\d+)\]\) == 0([^)]*)\)\s*\{\s*it = (\d+);",
    )
    .unwrap();
    // The `minimumRarity` guards partition the chain; track the floor in force at each match.
    let floor_re = Regex::new(r"if \(minimumRarity > (\d+)\)").unwrap();
    let floors: Vec<(usize, i64)> = floor_re
        .captures_iter(body)
        .map(|m| (m.get(0).unwrap().start(), m[1].parse().unwrap()))
        .collect();

    let mut entries: Vec<Entry> = Vec::new();
    for m in pattern.captures_iter(body) {
        let whole = m.get(0).unwrap();
        let inline_floor = m.get(1);
        let tier: i64 = m[2].parse().unwrap();
        let tail = &m[3];
        let item: i64 = m[4].parse().unwrap();
        // A nested RollLuck in the tail means a compound roll this table cannot express; skip it
        // and say so rather than emitting something that looks right.
        if tail.contains("RollLuck") {
            continue;
        }
        let needs: Vec<&'static str> = CONDITIONS
            .iter()
            .filter(|(src, _)| tail.contains(src))
            .map(|&(_, flag)| flag)
            .collect();
        let floor = if let Some(f) = inline_floor {
            f.as_str().parse().unwrap()
        } else {
            let mut floor = 0;
            for &(at, value) in &floors {
                if at < whole.start() {
                    floor = value;
                }
            }
            floor
        };
        entries.push(Entry {
            tier,
            item,
            needs,
            floor,
        });
    }
    assert!(
        entries.len() >= 30,
        "only {} shop entries parsed; the chain's shape changed",
        entries.len()
    );

    let flag_set: std::collections::BTreeSet<&'static str> = entries
        .iter()
        .flat_map(|e| e.needs.iter().copied())
        .collect();
    let flags: Vec<&'static str> = flag_set.into_iter().collect();

    let mut lines: Vec<String> = vec![
        "//! The Travelling Merchant's stock, generated from the game's own chain.\n\
         //!\n\
         //! His inventory is not a list but a *chain of rolls*: one per candidate item, each overwriting\n\
         //! the last, so the final match wins. Rarer items sit at higher tiers and are rolled against\n\
         //! longer odds, and several only appear once the world has been through something.\n\
         //!\n\
         //! Generated by `terrustia-codegen` from Terraria 1.4.5.7. Do not edit by hand.\n\
         \n\
         /// What a world must already have been through for an item to be offered.\n\
         ///\n\
         /// A bit set rather than an enum: a few items want two things at once.\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct Needs(pub u16);\n\
         \n\
         impl Needs {\n    \
             pub const NONE: Self = Self(0);\n"
            .to_string(),
    ];
    for (i, flag) in flags.iter().enumerate() {
        lines.push(format!("    pub const {flag}: u16 = 1 << {i};"));
    }
    lines.push(
        "\n    pub fn met_by(self, world: Needs) -> bool {\n        \
             self.0 & !world.0 == 0\n    \
         }\n\
         }\n\
         \n\
         /// One thing the merchant might be carrying.\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct Offer {\n    \
             pub item: u16,\n    \
             /// Which rarity tier's odds it is rolled against. Higher is rarer.\n    \
             pub tier: u8,\n    \
             /// What the world must have been through.\n    \
             pub needs: Needs,\n    \
             /// The `minimumRarity` floor below which the game stops considering this part of the chain.\n    \
             pub floor: u8,\n\
         }\n"
            .to_string(),
    );

    lines.push("/// The chain, in the order the game walks it. Later entries win.".to_string());
    lines.push(format!("pub const OFFERS: [Offer; {}] = [", entries.len()));
    for e in &entries {
        let bits = if e.needs.is_empty() {
            "0".to_string()
        } else {
            e.needs
                .iter()
                .map(|f| format!("Needs::{f}"))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        lines.push(format!(
            "    Offer {{ item: {}, tier: {}, needs: Needs({bits}), floor: {} }},",
            e.item, e.tier, e.floor
        ));
    }
    lines.push("];".to_string());

    lines.push(
        "\n/// How many rarity tiers there are, and the odds each is rolled against.\n\
         ///\n\
         /// `Chest.SetupTravelShop`'s starting array. The odds are one-in-N, so a tier-five item at 1200\n\
         /// turns up about once in every twelve hundred rolls of that step.\n\
         pub const TIER_ODDS: [i32; 6] = [1, 1, 1, 6, 300, 1200];\n\
         \n\
         #[cfg(test)]\n\
         mod tests {\n    \
             use super::*;\n\
             \n    \
             /// Every offer names a real item and a tier the odds table has.\n    \
             #[test]\n    \
             fn every_offer_is_well_formed() {\n        \
                 for offer in OFFERS {\n            \
                     assert!(offer.item > 0, \"an offer with no item\");\n            \
                     assert!(\n                \
                         (offer.tier as usize) < TIER_ODDS.len(),\n                \
                         \"item {} names tier {}, past the odds table\",\n                \
                         offer.item,\n                \
                         offer.tier\n            \
                     );\n        \
                 }\n    \
             }\n\
             \n    \
             /// A fresh world can be offered something, and a finished one can be offered more.\n    \
             #[test]\n    \
             fn progress_widens_the_stock() {\n        \
                 let open = |world: Needs| {\n            \
                     OFFERS\n                \
                         .iter()\n                \
                         .filter(|o| o.needs.met_by(world))\n                \
                         .count()\n        \
                 };\n        \
                 let fresh = open(Needs::NONE);\n        \
                 let late = open(Needs(u16::MAX));\n        \
                 assert!(fresh > 0, \"a fresh world should have stock\");\n        \
                 assert!(late > fresh, \"{late} late against {fresh} fresh\");\n    \
             }\n\
             \n    \
             /// The condition test is a subset check, not equality: a world that has been through more\n    \
             /// than an item asks for still qualifies.\n    \
             #[test]\n    \
             fn extra_progress_does_not_disqualify() {\n        \
                 let wants = Needs(Needs::HARDMODE);\n        \
                 assert!(wants.met_by(Needs(Needs::HARDMODE)));\n        \
                 assert!(wants.met_by(Needs(u16::MAX)));\n        \
                 assert!(!wants.met_by(Needs::NONE));\n    \
             }\n\
         }\n"
            .to_string(),
    );

    format!("{}\n", lines.join("\n"))
}
