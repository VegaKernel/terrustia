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


# A bare integer, but never one that is actually the tail of an identifier — `condition2` and
# `npcNetIds18` both end in digits that are not numbers in this text, and matching them as though
# they were is how Eye of Cthulhu ended up "owing" the player items 2, 3 and 282 that were really
# `condition2`, `condition3` and an unrelated `npcNetIds18` array's own name. Three separate bugs
# of this shape were found by hand-tracing every line that (mis)attributed something to npc 4 —
# see the git history of this fix for the trace.
NUM = re.compile(r"(?<![A-Za-z0-9_])-?\d+")


def _split_top_level(s: str) -> list[str]:
    """Split on commas at paren depth 0 only, so `Foo(1, 2), 3, 4` splits into two arguments —
    `Foo(1, 2)` and `3, 4` joined back — not four."""
    parts, depth, start = [], 0, 0
    for i, ch in enumerate(s):
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        elif ch == "," and depth == 0:
            parts.append(s[start:i])
            start = i + 1
    parts.append(s[start:])
    return parts


def _find_matching_paren(s: str, open_at: int) -> int:
    """Index of the `)` that closes the `(` at `open_at`."""
    depth = 0
    for i in range(open_at, len(s)):
        if s[i] == "(":
            depth += 1
        elif s[i] == ")":
            depth -= 1
            if depth == 0:
                return i
    return -1


def parse_game(root: Path) -> tuple[dict[int, set[int]], dict[int, set[int]]]:
    """Every (npc type -> item ids) pair the drop database registers.

    Reads whole statements rather than lines, because a rule chain is written across several.
    """
    text = (root / "Terraria.GameContent.ItemDropRules" / "ItemDropDatabase.cs").read_text(
        errors="replace"
    )
    # `int[] npcNetIds = new int[N] { ... };` is declared fresh, under the *same* name, in nearly
    # every one of these register-methods — it is a local, not a file-scoped constant. Collapsing
    # its (possibly multi-line) literal onto the declaration line lets the sequential scan below
    # resolve each `RegisterToMultipleNPCs(..., npcNetIds)` against whichever declaration of that
    # name most recently preceded it, the same way `current_type` already tracks `short type = N`.
    # Treating the name as globally unique — this file's previous shape — resolved every one of
    # them to whatever the *last* `npcNetIds` in the whole file happened to be: one real Zombie
    # drop list ended up attributed to hundreds of unrelated lines across a dozen functions.
    def _collapse(m: re.Match[str]) -> str:
        return m.group(0).replace("\n", " ")

    text = re.sub(r"int\[\]\s+\w+\s*=\s*new int\[\d+\]\s*\{[^}]*\}", _collapse, text, flags=re.S)

    drops: dict[int, set[int]] = {}
    mode_only: dict[int, set[int]] = {}
    # Track `short type = N;` so the many `RegisterToNPC(type, ...)` calls resolve.
    current_type: int | None = None
    arrays: dict[str, set[int]] = {}

    for raw in text.splitlines():
        line = raw.strip()

        if m := re.match(r"short type = (\d+);", line):
            current_type = int(m.group(1))
            continue
        if m := re.match(r"int\[\]\s+(\w+)\s*=\s*new int\[\d+\]\s*\{([^}]*)\}", line):
            arrays[m.group(1)] = {int(n) for n in NUM.findall(m.group(2)) if int(n) >= 0}
            continue

        if "RegisterToNPC(" not in line and "RegisterToMultipleNPCs(" not in line:
            continue

        # Which NPCs this line registers to.
        targets: set[int] = set()
        if m := re.search(r"RegisterToNPC\((\d+)\s*,", line):
            targets.add(int(m.group(1)))
        # Exact variable name `type` only — `type2`, `type18` and friends are different locals
        # entirely, and a bare substring check here silently reused a stale, unrelated npc id.
        elif re.search(r"RegisterToNPC\(type\s*,", line) and current_type is not None:
            targets.add(current_type)
        elif "RegisterToMultipleNPCs(" in line:
            open_at = line.index("RegisterToMultipleNPCs(") + len("RegisterToMultipleNPCs")
            close_at = _find_matching_paren(line, open_at)
            if close_at != -1:
                inner = line[open_at + 1 : close_at]
                # The first top-level argument is the rule expression; everything after it is the
                # id list — a bare integer literal, or a named `int[]` declared earlier.
                args = _split_top_level(inner)
                for tok in args[1:]:
                    tok = tok.strip()
                    if re.fullmatch(r"-?\d+", tok):
                        targets.add(int(tok))
                    elif tok in arrays:
                        targets.update(arrays[tok])
                    # Anything else (a method call, an unrecognised variable) is left unresolved
                    # rather than guessed at — silence here is a missed check, not a wrong one.

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
                for n in NUM.findall(call):
                    if int(n) > 0:
                        skipped.add(int(n))
                    break
        for name, call in re.findall(r"ItemDropRule\.(\w+)\(([^)]*)\)", line):
            if name.startswith("MasterMode") or name == "BossBag":
                continue
            # Every `*OneFromOptions*` variant leads with one or two chance denominators, not an
            # item — taking the first number as the item is what made every boss look as though
            # it owed the player an Iron Pickaxe. `Gel` is a different shape of the same problem:
            # its own signature is `(chanceDenominator, min, max)` with no item argument at all —
            # the item is implicitly Gel (23) every time — so treating its first number as an item
            # id invented a phantom drop out of a chance value on every slime-family enemy.
            if "OneFromOptions" in name or name == "Gel":
                continue
            for n in NUM.findall(call):
                value = int(n)
                if value > 0:
                    items.add(value)
                break  # only the first argument is the item
        # How many leading arguments are chance denominators rather than items, per constructor.
        # Unlisted variants fall back to 1, the shape every `OneFromOptions*` overload the game
        # actually uses shares except the `NormalvsExpert*` pair.
        LEADING_CHANCES = {
            "NormalvsExpertOneFromOptions": 2,
            "NormalvsExpertOneFromOptionsNotScalingWithLuck": 2,
            "OneFromOptionsWithNumerator": 2,
            "OneFromOptionsNotScalingWithLuckWithX": 2,
        }
        for name, call in re.findall(r"ItemDropRule\.(\w*OneFromOptions\w*)\(([^)]*)\)", line):
            numbers = [int(n) for n in NUM.findall(call)]
            skip = LEADING_CHANCES.get(name, 1)
            items.update(n for n in numbers[skip:] if n > 0)

        for npc in targets:
            if items:
                drops.setdefault(npc, set()).update(items)
            if skipped:
                mode_only.setdefault(npc, set()).update(skipped)

    return drops, mode_only


def _expand_npcs(label: str) -> list[int]:
    """`13..=15 | 20` -> `[13, 14, 15, 20]`. A match-arm label's range is inclusive on both ends,
    and taking only the two digit tokens the old code did (`13`, `15`) silently dropped every id
    in between — 14 was never checked against anything for exactly that reason."""
    npcs: list[int] = []
    for part in re.split(r"\s*\|\s*", label.strip()):
        if m := re.fullmatch(r"(\d+)\.\.=(\d+)", part):
            npcs.extend(range(int(m.group(1)), int(m.group(2)) + 1))
        elif part.isdigit():
            npcs.append(int(part))
    return npcs


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
        for npc in _expand_npcs(label):
            ours.setdefault(npc, set()).update(items)

    cond = (root / "crates" / "terrustia-proto" / "src" / "conditional_drops.rs").read_text()
    for m in re.finditer(
        r"^        (\d+(?:\.\.=\d+)?(?:\s*\|\s*\d+)*) => vec!\[(.*?)\],\n", cond, re.S | re.M
    ):
        npcs = _expand_npcs(m.group(1))
        items = {int(i) for i in re.findall(r"(?:always|sometimes|a_few)\((\d+)", m.group(2))}
        for npc in npcs:
            ours.setdefault(npc, set()).update(items)
    # A handful of drops in `conditional()` are gated by an `if npc_type == N { ... }` guard
    # instead of living in that npc's own match arm — the Eye of Cthulhu's world-evil ore is one,
    # correctly implemented and tested, but invisible to the scan above since it is not a `vec!`
    # arm at all. Brace-matched by hand because the block can nest (an `if/else` inside it, in
    # EoC's case) and a regex cannot balance that reliably.
    for m in re.finditer(r"if npc_type == (\d+)[^{]*\{", cond):
        npc = int(m.group(1))
        depth, i = 1, m.end()
        while depth > 0 and i < len(cond):
            if cond[i] == "{":
                depth += 1
            elif cond[i] == "}":
                depth -= 1
            i += 1
        body = cond[m.end() : i]
        items = {int(n) for n in re.findall(r"(?:always|sometimes|a_few)\((\d+)", body)}
        ours.setdefault(npc, set()).update(items)
    # The bag, trophy and mask maps, and the one-from pools.
    for pattern in (
        r"^        (\d+(?:\.\.=\d+)?(?:\s*\|\s*\d+)*) => (\d+),",
        # `one_from` pools, which may be one list or several on a line.
        r"^        (\d+) => &\[((?:&\[[\d, ]+\],?\s*)+)\],",
    ):
        for m in re.finditer(pattern, cond, re.M):
            npcs = _expand_npcs(m.group(1))
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
