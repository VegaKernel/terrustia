#!/usr/bin/env python3
"""Generate `npc_drops.rs` — the *unconditional* half of the loot tables.

    python3 tools/gen_drops.py <decompiled-tree> crates/terrustia-proto/src/npc_drops.rs

`npc_drops.rs` was the only table in the project with no generator, and it showed: it held the 248
rules somebody transcribed by hand and stopped there, while the game registers far more. 226
ordinary enemies were short at least one drop between them, 643 in total.

**This generates only what can be generated safely.** The game's drop database is a tree of
`LeadingConditionRule` chains, `OneFromOptions` pools, mode-dependent rerolls and treasure bags, and
a generator that flattened those would hand players the wrong loot forever while looking
authoritative. So this takes the subset that is genuinely a flat table — an unconditional rule
registered straight to an NPC — and leaves everything conditional to the hand-written
`conditional_drops.rs`, where `tools/check_drops.py` keeps it honest.

The four constructors it understands all share `(itemId, chanceDenominator, min, max)`:
`Common`, `NotScalingWithLuck`, `Food`, `StatusImmunityItem`.

Chains matter and are preserved. `Common(a, 7).OnFailedRoll(Common(b, 7))` is *not* two independent
rolls: the second is only tried when the first misses, so flattening them would make both far more
common than they are. Those become one `DropChain`.
"""

import re
import sys
from pathlib import Path

# Constructors whose first four arguments are (item, chance, min, max) and which carry no
# condition of their own. Anything else is left for the hand-written table.
FLAT = ("Common", "NotScalingWithLuck", "Food", "StatusImmunityItem")


def balanced_args(line: str, start: int) -> list[str]:
    """Split one call's arguments, respecting nested parentheses.

    `RegisterToMultipleNPCs(ItemDropRule.Common(160, 200), npcNetIds21).OnFailedRoll(...)` has two
    arguments, and finding them by counting parentheses backwards from the end lands inside the
    chained call instead.
    """
    depth, args, current = 1, [], []
    for ch in line[start:]:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                break
        if depth == 1 and ch == ",":
            args.append("".join(current))
            current = []
        else:
            current.append(ch)
    args.append("".join(current))
    return args


def parse(root: Path) -> dict[int, list[list[tuple[int, int, int, int]]]]:
    """NPC type -> list of chains, each chain a list of (item, one_in, min, max)."""
    text = (root / "Terraria.GameContent.ItemDropRules" / "ItemDropDatabase.cs").read_text(
        errors="replace"
    )

    call = re.compile(
        r"ItemDropRule\.(" + "|".join(FLAT) + r")\(\s*(\d+)\s*(?:,\s*(\d+))?\s*(?:,\s*(\d+))?\s*(?:,\s*(\d+))?\s*\)"
    )

    # `int[] npcNetIds = new int[N] {...}` is declared fresh, under the *same* name, in ~15
    # different functions in this file (7 uses of the bare name "npcNetIds" alone) — it is a
    # local, not a file-scoped constant. A single whole-file dict keyed by name resolves every
    # `RegisterToMultipleNPCs(rule, npcNetIds)` call anywhere in the file to whichever declaration
    # was scanned *last*, silently misattributing every earlier group's items to the last one's
    # NPCs and dropping the rest entirely — this is exactly the bug `tools/check_drops.py` found
    # and fixed in its own array resolution; this generator had the identical bug and, until this
    # fix, produced a table that looked complete (no error, no warning) while silently losing every
    # `RegisterToMultipleNPCs` group that used a reused array name. Fixed the same way: collapse
    # each (possibly multi-line) declaration onto its own line, then resolve sequentially as the
    # scan reaches each use, so a call resolves against whichever declaration of that name most
    # recently preceded it — never a name resolved by declaration order that ignores its usage.
    text = re.sub(
        r"int\[\]\s+\w+\s*=\s*new int\[\d*\]\s*\{[^}]*\}",
        lambda m: m.group(0).replace("\n", " "),
        text,
        flags=re.S,
    )

    out: dict[int, list[list[tuple[int, int, int, int]]]] = {}
    current_type: int | None = None
    arrays: dict[str, list[int]] = {}

    for raw in text.splitlines():
        line = raw.strip()

        if m := re.match(r"short type = (\d+);", line):
            current_type = int(m.group(1))
            continue

        if m := re.match(r"int\[\]\s+(\w+)\s*=\s*new int\[\d*\]\s*\{([^}]*)\}", line):
            arrays[m.group(1)] = [int(n) for n in re.findall(r"-?\d+", m.group(2))]
            continue

        if "RegisterToNPC(" not in line and "RegisterToMultipleNPCs(" not in line:
            continue
        # Anything with a condition, a pool, a bag or a mode branch is not ours to flatten.
        #
        # A chain can *end* in one of these without the chain's own leading links needing it —
        # `Common(932).OnFailedRoll(Common(3095)).OnFailedRoll(Common(327)).OnFailedRoll(
        # ByCondition(NotExpert(), 154))` is a real, entirely-flat three-item chain with one
        # conditional fallback tacked on the end, and excluding the whole line on sight of
        # "ByCondition" anywhere in it silently dropped all three flat links too — for every NPC
        # that chain applied to. Truncate at the *first* excluded keyword instead, so a chain's
        # genuinely-flat prefix survives even when its tail does not; the conditional remainder
        # (here, item 154 for classic-mode kills) is left to `conditional_drops.rs`, same as any
        # other condition-gated drop.
        excluded = re.search(
            r"ByCondition|BossBag|MasterMode|OneFromOptions|ExpertGetsRerolls"
            r"|LeadingConditionRule|DropBasedOn|OneFromRules|DropNothing|Coins|WithRerolls"
            r"|RemixSeed",
            line,
        )
        if excluded:
            line = line[: excluded.start()]
            if not line or (
                "RegisterToNPC(" not in line and "RegisterToMultipleNPCs(" not in line
            ):
                continue

        # Which NPCs.
        targets: list[int] = []
        if m := re.search(r"RegisterToNPC\((-?\d+)\s*,", line):
            targets = [int(m.group(1))]
        elif "RegisterToNPC(type" in line and current_type is not None:
            targets = [current_type]
        elif (at := line.find("RegisterToMultipleNPCs(")) >= 0:
            # Searched for rather than matched at the start: several of these are assigned to a
            # local first — `IItemDropRule entry2 = RegisterToMultipleNPCs(...)` — and anchoring
            # to the start of the line quietly lost every slime's Slime Staff.
            #
            # The line often continues `.OnFailedRoll(...)` too, so the ids cannot be found by
            # counting back from the last paren. Balance from the opening one instead.
            args = balanced_args(line, at + len("RegisterToMultipleNPCs("))
            for arg in args[1:]:
                if ids := arrays.get(arg.strip()):
                    targets.extend(ids)
                else:
                    targets.extend(int(n) for n in re.findall(r"-?\d+", arg))
        targets = [t for t in targets if t > 0]
        if not targets:
            continue

        # The rules on this line, in order. A `.OnFailedRoll(` between two of them chains them.
        rules = [
            (
                int(m.group(2)),
                int(m.group(3) or 1),
                int(m.group(4) or 1),
                int(m.group(5) or 1),
            )
            for m in call.finditer(line)
        ]
        # `NormalvsExpert(item, classicChance, expertChance)` rolls at a different rate depending
        # on the world. The classic branch is taken here and the difference is a known
        # simplification: an expert world under-rolls these twenty-four drops slightly. Recorded in
        # GAPS.md rather than pretended away.
        rules += [
            (int(m.group(1)), int(m.group(2)), 1, 1)
            for m in re.finditer(r"NormalvsExpert\(\s*(\d+)\s*,\s*(\d+)", line)
        ]
        if not rules:
            continue

        chained = ".OnFailedRoll(" in line
        chains = [rules] if chained else [[rule] for rule in rules]

        for npc in targets:
            out.setdefault(npc, []).extend(chains)

    return out


def emit(drops: dict[int, list[list[tuple[int, int, int, int]]]], total: int) -> str:
    lines = [
        "//! What NPCs drop when they die, generated from `ItemDropDatabase`.",
        "//!",
        "//! This is the **unconditional** half: a rule registered straight to an NPC with nothing",
        "//! gating it. The other half — boss bags, master-mode drops, `ByCondition`,",
        "//! `OneFromOptions`, mode-dependent rerolls — lives in [`crate::conditional_drops`],",
        "//! written by hand because flattening a condition tree is how you hand somebody the wrong",
        "//! loot forever without noticing. `tools/check_drops.py` compares both against the game.",
        "//!",
        "//! **Chains are preserved and matter.** `Common(a, 7).OnFailedRoll(Common(b, 7))` is one",
        "//! chain, not two rolls: the second is tried only when the first misses. Flattening them",
        "//! would make both far more common than the game intends, which is the kind of bug that",
        "//! looks like generosity until somebody compares drop rates.",
        "//!",
        "//! This file was hand-written once and held 248 rules. It was the only table in the",
        "//! project with no generator, and 226 enemies were short at least one drop between them.",
        "//!",
        "//! Generated by `tools/gen_drops.py`. Do not edit by hand.",
        "",
        "/// One thing an NPC might drop.",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub struct Drop {",
        "    pub item: u16,",
        "    /// A one-in-this chance. One means always.",
        "    pub one_in: u32,",
        "    pub min: i16,",
        "    pub max: i16,",
        "}",
        "",
        "/// A run of alternatives, tried in order until one of them lands.",
        "pub type DropChain = &'static [Drop];",
        "",
        f"/// How many rules the table holds, across {len(drops)} NPC types.",
        f"pub const RULES: usize = {total};",
        "",
        "/// What a type drops.",
        "pub fn drops(npc_type: u16) -> &'static [DropChain] {",
        "    match npc_type {",
    ]

    for npc in sorted(drops):
        chains = drops[npc]
        lines.append(f"        {npc} => &[")
        for chain in chains:
            lines.append("            &[")
            for item, one_in, low, high in chain:
                lines.append("                Drop {")
                lines.append(f"                    item: {item},")
                lines.append(f"                    one_in: {one_in},")
                lines.append(f"                    min: {low},")
                lines.append(f"                    max: {high},")
                lines.append("                },")
            lines.append("            ],")
        lines.append("        ],")

    lines += [
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
        f"        assert_eq!(RULES, {total});",
        '        assert!(!drops(3).is_empty(), "a zombie drops something");',
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
    return "\n".join(lines)


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 1
    root, out = Path(sys.argv[1]), Path(sys.argv[2])
    drops = parse(root)
    total = sum(len(c) for c in drops.values())
    if total < 200:
        print(f"error: only parsed {total} rules; the parser is wrong", file=sys.stderr)
        return 1
    out.write_text(emit(drops, total))
    print(f"wrote {total} rules across {len(drops)} NPC types to {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
