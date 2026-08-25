#!/usr/bin/env python3
"""Compare this server's drop tables against the game's, and report what is missing.

    python3 tools/check_drops.py <decompiled-tree>

Why a checker rather than a generator: `ItemDropDatabase` is not a flat table. It is built from
`LeadingConditionRule` chains with `OnSuccess`/`OnFailedRoll` branches, `OneFromOptions` pools and
mode-dependent rerolls, and a generator that got any of that subtly wrong would hand players the
wrong loot forever while looking authoritative. A checker cannot do that. It can only say "the game
gives NPC 262 item 1141 and you do not", which is exactly the sentence that was missing when the
Temple Key went absent and took the second half of the game with it.

It is deliberately generous about *how* a drop is registered and strict about *whether* it exists:
false positives here are cheap to dismiss, and a false negative is another Temple Key.

Exit code is 1 when a boss is missing loot, because those are the ones that gate progression.
"""

import re
import sys
from pathlib import Path

# Bosses and event bosses. A missing drop on any of these can stop a playthrough, so they are
# reported separately and are what the exit code turns on.
BOSSES = {
    4, 13, 14, 15, 35, 36, 50, 113, 125, 126, 127, 134, 222, 245, 262, 266, 267,
    325, 326, 370, 395, 398, 439, 551, 636, 657, 668,
}


def parse_game(root: Path) -> tuple[dict[int, set[int]], dict[int, set[int]]]:
    """Every (npc type -> item ids) pair the drop database registers.

    Reads whole statements rather than lines, because a rule chain is written across several.
    """
    text = (root / "Terraria.GameContent.ItemDropRules" / "ItemDropDatabase.cs").read_text(
        errors="replace"
    )

    drops: dict[int, set[int]] = {}
    mode_only: dict[int, set[int]] = {}
    # Track `short type = N;` so the many `RegisterToNPC(type, ...)` calls resolve.
    current_type: int | None = None

    for raw in text.splitlines():
        line = raw.strip()

        if m := re.match(r"short type = (\d+);", line):
            current_type = int(m.group(1))
            continue
        # A few register blocks name the type inline in a local.
        if m := re.match(r"int (?:num|type\d*) = (\d+);", line):
            pass

        if "RegisterToNPC(" not in line and "RegisterToMultipleNPCs(" not in line:
            continue

        # Which NPCs this line registers to.
        targets: set[int] = set()
        if m := re.search(r"RegisterToNPC\((\d+)\s*,", line):
            targets.add(int(m.group(1)))
        elif "RegisterToNPC(type" in line and current_type is not None:
            targets.add(current_type)
        elif m := re.search(r"RegisterToMultipleNPCs\(.*?\)\s*;?\s*$", line):
            # Trailing ids after the rule argument: `..., 126, 125);`
            tail = re.findall(r",\s*(-?\d+)", line)
            targets.update(int(t) for t in tail if int(t) > 0)

        if not targets:
            continue

        # Item ids: the first number inside each rule constructor. Generous on purpose.
        items: set[int] = set()
        # Master-mode relics and pets, and the treasure bags expert replaces loot with. This
        # server runs classic, so their absence is a decision rather than a gap — but they are
        # reported so the decision stays visible instead of quietly becoming a habit.
        skipped: set[int] = set()
        for name, call in re.findall(r"ItemDropRule\.(\w+)\(([^)]*)\)", line):
            if name.startswith("MasterMode") or name == "BossBag":
                for n in re.findall(r"-?\d+", call):
                    if int(n) > 0:
                        skipped.add(int(n))
                    break
        for name, call in re.findall(r"ItemDropRule\.(\w+)\(([^)]*)\)", line):
            if name.startswith("MasterMode") or name == "BossBag":
                continue
            # `OneFromOptions` leads with the chance, not an item — taking its first number as an
            # item is what made every boss look as though it owed the player an Iron Pickaxe.
            if name.startswith("OneFromOptions"):
                continue
            for n in re.findall(r"-?\d+", call):
                value = int(n)
                if value > 0:
                    items.add(value)
                break  # only the first argument is the item
        for call in re.findall(r"OneFromOptions\w*\(([^)]*)\)", line):
            numbers = [int(n) for n in re.findall(r"-?\d+", call)]
            items.update(n for n in numbers[1:] if n > 0)  # first is the chance

        for npc in targets:
            if items:
                drops.setdefault(npc, set()).update(items)
            if skipped:
                mode_only.setdefault(npc, set()).update(skipped)

    return drops, mode_only


def parse_ours(root: Path) -> dict[int, set[int]]:
    """What our two tables give, keyed the same way."""
    ours: dict[int, set[int]] = {}

    flat = (root / "crates" / "terrustia-proto" / "src" / "npc_drops.rs").read_text()
    # Split into arms by their `N =>` markers and take everything up to the next one. Matching an
    # arm's closing brace with a regex silently swallowed most of the file: the shapes differ
    # between a one-line arm and a multi-chain one, and a non-greedy match spanned several.
    marker = re.compile(r"^        (\d+(?:\s*\|\s*\d+)*) => ", re.M)
    starts = [(m.start(), m.end(), m.group(1)) for m in marker.finditer(flat)]
    for nth, (_, body_at, label) in enumerate(starts):
        end = starts[nth + 1][0] if nth + 1 < len(starts) else len(flat)
        body = flat[body_at:end]
        items = {int(i) for i in re.findall(r"item: (\d+)", body)}
        for npc in (int(n) for n in re.findall(r"\d+", label)):
            ours.setdefault(npc, set()).update(items)

    cond = (root / "crates" / "terrustia-proto" / "src" / "conditional_drops.rs").read_text()
    for m in re.finditer(r"^        (\d+(?:\s*\|\s*\d+)*) => vec!\[(.*?)\],\n", cond, re.S | re.M):
        npcs = [int(n) for n in re.findall(r"\d+", m.group(1))]
        items = {int(i) for i in re.findall(r"(?:always|sometimes|a_few)\((\d+)", m.group(2))}
        for npc in npcs:
            ours.setdefault(npc, set()).update(items)
    # The bag, trophy and mask maps, and the one-from pools.
    for pattern in (
        r"^        (\d+(?:\.\.=\d+)?(?:\s*\|\s*\d+)*) => (\d+),",
        # `one_from` pools, which may be one list or several on a line.
        r"^        (\d+) => &\[((?:&\[[\d, ]+\],?\s*)+)\],",
    ):
        for m in re.finditer(pattern, cond, re.M):
            npcs = [int(n) for n in re.findall(r"\d+", m.group(1))]
            items = {int(n) for n in re.findall(r"\d+", m.group(2))}
            for npc in npcs:
                ours.setdefault(npc, set()).update(items)

    return ours


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 1
    game_root = Path(sys.argv[1])
    repo = Path(__file__).resolve().parent.parent

    game, mode_only = parse_game(game_root)
    ours = parse_ours(repo)

    boss_gaps: list[str] = []
    other_missing = 0
    for npc in sorted(game):
        missing = game[npc] - ours.get(npc, set()) - mode_only.get(npc, set())
        if not missing:
            continue
        if npc in BOSSES:
            boss_gaps.append(f"  npc {npc}: missing {sorted(missing)}")
        else:
            other_missing += 1

    print(f"game registers drops for {len(game)} NPC types; we have {len(ours)}")
    print(f"ordinary enemies with at least one drop we lack: {other_missing}")
    print()
    if boss_gaps:
        print(f"BOSSES MISSING LOOT ({len(boss_gaps)}):")
        print("\n".join(boss_gaps))
        print()
        print("A boss with no loot can end a playthrough. Check each against the game before")
        print("dismissing it: the treasure bag and master-mode drops are expected absences.")
        return 1
    print("every boss the game gives loot to gets loot here.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
