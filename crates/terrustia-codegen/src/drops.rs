//! `crates/terrustia-proto/src/npc_drops.rs` — the *unconditional* half of the loot tables.
//!
//! Reads `Terraria.GameContent.ItemDropRules/ItemDropDatabase.cs` and keeps only the four
//! constructors whose first four arguments are `(item, chanceDenominator, min, max)` and which
//! carry no condition of their own: `Common`, `NotScalingWithLuck`, `Food`,
//! `StatusImmunityItem`. Everything conditional (boss bags, `ByCondition`, `OneFromOptions`,
//! mode-dependent rerolls) is left to the hand-written `conditional_drops.rs`.
//!
//! `.OnFailedRoll(...)` chains are preserved: `Common(a, 7).OnFailedRoll(Common(b, 7))` becomes
//! one [`DropChain`] of two rules, not two independent one-rule chains, because the second is
//! only tried when the first misses.
//!
//! Ported from `gen_drops.py`.

use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;

use crate::csharp::read_lossy;

/// Constructors whose first four arguments are (item, chance, min, max) and which carry no
/// condition of their own. Anything else is left for the hand-written table.
const FLAT: &str = "Common|NotScalingWithLuck|Food|StatusImmunityItem";

type Rule = (i64, i64, i64, i64);
type Chain = Vec<Rule>;

/// Split one call's arguments, respecting nested parentheses, starting just past the opening `(`
/// (so `depth` begins at 1 for the call itself).
///
/// `RegisterToMultipleNPCs(ItemDropRule.Common(160, 200), npcNetIds21).OnFailedRoll(...)` has two
/// arguments, and finding them by counting parentheses backwards from the end lands inside the
/// chained call instead.
fn balanced_args(line: &str, start: usize) -> Vec<String> {
    let mut depth: i32 = 1;
    let mut args: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in line[start..].chars() {
        let mut closed = false;
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                closed = true;
            }
        }
        if closed {
            break;
        }
        if depth == 1 && ch == ',' {
            args.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    args.push(current);
    args
}

/// `int[] npcNetIds = new int[N] {...}` is declared fresh, under the *same* name, in many
/// different functions in this file — it is a local, not a file-scoped constant. Collapse each
/// (possibly multi-line) declaration onto its own line first, so a later sequential scan resolves
/// a `RegisterToMultipleNPCs(rule, npcNetIds)` call against whichever declaration of that name
/// most recently preceded it, never one resolved by declaration order alone.
fn collapse_array_declarations(text: &str) -> String {
    let re = Regex::new(r"int\[\]\s+\w+\s*=\s*new int\[\d*\]\s*\{[^}]*\}").unwrap();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in re.find_iter(text) {
        out.push_str(&text[last..m.start()]);
        out.push_str(&m.as_str().replace('\n', " "));
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

/// NPC type -> list of chains, each chain a list of (item, one_in, min, max).
fn parse(root: &Path) -> (BTreeMap<i64, Vec<Chain>>, usize) {
    let raw = read_lossy(&root.join("Terraria.GameContent.ItemDropRules/ItemDropDatabase.cs"));
    let text = collapse_array_declarations(&raw);

    let call = Regex::new(&format!(
        r"ItemDropRule\.({FLAT})\(\s*(\d+)\s*(?:,\s*(\d+))?\s*(?:,\s*(\d+))?\s*(?:,\s*(\d+))?\s*\)"
    ))
    .unwrap();
    let type_re = Regex::new(r"^short type = (\d+);").unwrap();
    let arr_decl_re = Regex::new(r"^int\[\]\s+(\w+)\s*=\s*new int\[\d*\]\s*\{([^}]*)\}").unwrap();
    let num_re = Regex::new(r"-?\d+").unwrap();
    // Anything with a condition, a pool, a bag or a mode branch is not ours to flatten. A chain
    // can *end* in one of these without the chain's own leading links needing it, so truncate at
    // the first excluded keyword instead of dropping the whole line — a chain's genuinely-flat
    // prefix survives even when its tail does not.
    let excluded_re = Regex::new(
        r"ByCondition|BossBag|MasterMode|OneFromOptions|ExpertGetsRerolls|LeadingConditionRule|DropBasedOn|OneFromRules|DropNothing|Coins|WithRerolls|RemixSeed",
    )
    .unwrap();
    let register_single_re = Regex::new(r"RegisterToNPC\((-?\d+)\s*,").unwrap();
    let normal_vs_expert_re = Regex::new(r"NormalvsExpert\(\s*(\d+)\s*,\s*(\d+)").unwrap();

    let mut out: BTreeMap<i64, Vec<Chain>> = BTreeMap::new();
    let mut current_type: Option<i64> = None;
    let mut arrays: std::collections::HashMap<String, Vec<i64>> = std::collections::HashMap::new();

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();

        if let Some(caps) = type_re.captures(trimmed) {
            current_type = Some(caps[1].parse().unwrap());
            continue;
        }

        if let Some(caps) = arr_decl_re.captures(trimmed) {
            let name = caps[1].to_string();
            let nums: Vec<i64> = num_re
                .find_iter(&caps[2])
                .map(|m| m.as_str().parse().unwrap())
                .collect();
            arrays.insert(name, nums);
            continue;
        }

        if !trimmed.contains("RegisterToNPC(") && !trimmed.contains("RegisterToMultipleNPCs(") {
            continue;
        }

        let mut line = trimmed.to_string();
        if let Some(m) = excluded_re.find(&line) {
            line.truncate(m.start());
            if line.is_empty()
                || (!line.contains("RegisterToNPC(") && !line.contains("RegisterToMultipleNPCs("))
            {
                continue;
            }
        }

        // Which NPCs.
        let mut targets: Vec<i64> = Vec::new();
        if let Some(caps) = register_single_re.captures(&line) {
            targets.push(caps[1].parse().unwrap());
        } else if line.contains("RegisterToNPC(type") {
            if let Some(t) = current_type {
                targets.push(t);
            }
        } else if let Some(at) = line.find("RegisterToMultipleNPCs(") {
            let start = at + "RegisterToMultipleNPCs(".len();
            let args = balanced_args(&line, start);
            for arg in args.iter().skip(1) {
                let key = arg.trim();
                if let Some(ids) = arrays.get(key) {
                    targets.extend(ids.iter().copied());
                } else {
                    targets.extend(
                        num_re
                            .find_iter(arg)
                            .map(|m| m.as_str().parse::<i64>().unwrap()),
                    );
                }
            }
        }
        targets.retain(|&t| t > 0);
        if targets.is_empty() {
            continue;
        }

        // The rules on this line, in order. A `.OnFailedRoll(` between two of them chains them.
        let mut rules: Chain = Vec::new();
        for caps in call.captures_iter(&line) {
            let item: i64 = caps[2].parse().unwrap();
            let one_in: i64 = caps.get(3).map_or(1, |m| m.as_str().parse().unwrap());
            let min: i64 = caps.get(4).map_or(1, |m| m.as_str().parse().unwrap());
            let max: i64 = caps.get(5).map_or(1, |m| m.as_str().parse().unwrap());
            rules.push((item, one_in, min, max));
        }
        // `NormalvsExpert(item, classicChance, expertChance)` rolls at a different rate depending
        // on the world. The classic branch is taken here and the difference is a known
        // simplification: an expert world under-rolls these twenty-four drops slightly. Recorded
        // in GAPS.md rather than pretended away.
        for caps in normal_vs_expert_re.captures_iter(&line) {
            let item: i64 = caps[1].parse().unwrap();
            let one_in: i64 = caps[2].parse().unwrap();
            rules.push((item, one_in, 1, 1));
        }
        if rules.is_empty() {
            continue;
        }

        let chained = line.contains(".OnFailedRoll(");
        let chains: Vec<Chain> = if chained {
            vec![rules]
        } else {
            rules.into_iter().map(|r| vec![r]).collect()
        };

        for &npc in &targets {
            out.entry(npc).or_default().extend(chains.clone());
        }
    }

    let total: usize = out.values().map(|v| v.len()).sum();
    (out, total)
}

fn emit(drops: &BTreeMap<i64, Vec<Chain>>, total: usize) -> String {
    let mut lines: Vec<String> = vec![
        "//! What NPCs drop when they die, generated from `ItemDropDatabase`.".into(),
        "//!".into(),
        "//! This is the **unconditional** half: a rule registered straight to an NPC with nothing"
            .into(),
        "//! gating it. The other half — boss bags, master-mode drops, `ByCondition`,".into(),
        "//! `OneFromOptions`, mode-dependent rerolls — lives in [`crate::conditional_drops`],".into(),
        "//! written by hand because flattening a condition tree is how you hand somebody the wrong"
            .into(),
        "//! loot forever without noticing. `tools/check_drops.py` compares both against the game."
            .into(),
        "//!".into(),
        "//! **Chains are preserved and matter.** `Common(a, 7).OnFailedRoll(Common(b, 7))` is one"
            .into(),
        "//! chain, not two rolls: the second is tried only when the first misses. Flattening them"
            .into(),
        "//! would make both far more common than the game intends, which is the kind of bug that"
            .into(),
        "//! looks like generosity until somebody compares drop rates.".into(),
        "//!".into(),
        "//! This file was hand-written once and held 248 rules. It was the only table in the".into(),
        "//! project with no generator, and 226 enemies were short at least one drop between them."
            .into(),
        "//!".into(),
        "//! Generated by `terrustia-codegen` from Terraria 1.4.5.7. Do not edit by hand.".into(),
        "".into(),
        "/// One thing an NPC might drop.".into(),
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]".into(),
        "pub struct Drop {".into(),
        "    pub item: u16,".into(),
        "    /// A one-in-this chance. One means always.".into(),
        "    pub one_in: u32,".into(),
        "    pub min: i16,".into(),
        "    pub max: i16,".into(),
        "}".into(),
        "".into(),
        "/// A run of alternatives, tried in order until one of them lands.".into(),
        "pub type DropChain = &'static [Drop];".into(),
        "".into(),
        format!("/// How many rules the table holds, across {} NPC types.", drops.len()),
        format!("pub const RULES: usize = {total};"),
        "".into(),
        "/// What a type drops.".into(),
        "pub fn drops(npc_type: u16) -> &'static [DropChain] {".into(),
        "    match npc_type {".into(),
    ];

    for (&npc, chains) in drops {
        lines.push(format!("        {npc} => &["));
        for chain in chains {
            lines.push("            &[".into());
            for &(item, one_in, low, high) in chain {
                lines.push("                Drop {".into());
                lines.push(format!("                    item: {item},"));
                lines.push(format!("                    one_in: {one_in},"));
                lines.push(format!("                    min: {low},"));
                lines.push(format!("                    max: {high},"));
                lines.push("                },".into());
            }
            lines.push("            ],".into());
        }
        lines.push("        ],".into());
    }

    lines.extend(
        [
            "        _ => &[],",
            "    }",
            "}",
            "",
            "#[cfg(test)]",
            "mod tests {",
            "    use super::*;",
            "",
            "    /// The table is populated, and did not silently regenerate empty.",
            "    #[test]",
            "    fn the_table_is_populated() {",
        ]
        .map(String::from),
    );
    lines.push(format!("        assert_eq!(RULES, {total});"));
    lines.extend(
        [
            "        assert!(!drops(3).is_empty(), \"a zombie drops something\");",
            "    }",
            "",
            "    /// A chain is one roll after another, not several at once.",
            "    ///",
            "    /// The caller stops at the first success. If these were separate chains a skeleton",
            "    /// would hand out every weapon at once instead of at most one.",
            "    #[test]",
            "    fn chains_stay_chained() {",
            "        let chained = (0..700u16)",
            "            .flat_map(drops)",
            "            .filter(|chain| chain.len() > 1)",
            "            .count();",
            "        assert!(chained > 0, \"no chains survived generation\");",
            "    }",
            "",
            "    /// Nothing rolls a zero-in-N chance, which would divide by zero downstream.",
            "    #[test]",
            "    fn every_chance_is_rollable() {",
            "        for kind in 0..700u16 {",
            "            for chain in drops(kind) {",
            "                for rule in *chain {",
            "                    assert!(rule.one_in >= 1, \"npc {kind} has an impossible chance\");",
            "                    assert!(rule.max >= rule.min, \"npc {kind} has a backwards stack\");",
            "                }",
            "            }",
            "        }",
            "    }",
            "",
            "    /// A type with no rules drops nothing rather than panicking.",
            "    #[test]",
            "    fn something_with_no_rules_drops_nothing() {",
            "        assert!(drops(u16::MAX).is_empty());",
            "    }",
            "}",
            "",
        ]
        .map(String::from),
    );

    lines.join("\n")
}

pub fn generate(root: &Path) -> String {
    let (drops, total) = parse(root);
    assert!(
        total >= 200,
        "only parsed {total} rules; the parser is wrong"
    );
    emit(&drops, total)
}
