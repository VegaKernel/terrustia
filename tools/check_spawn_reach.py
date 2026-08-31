#!/usr/bin/env python3
"""Find NPCs the game spawns by itself that this server can never spawn at all.

An unreachable NPC is quiet. Nothing errors, no test fails, no log line appears: the type simply
never comes up, and the only way to notice is to go looking for it. That is how NPC 48 (Harpy) and
NPC 87 (Wyvern) sat in no spawn pool at all - the whole sky roster was missing, and every test the
project had still passed. This makes the search mechanical instead of accidental.

Two sides, and each is read rather than remembered:

* Vanilla's side comes out of the decompiled tree, from `NPC.Spawner` - every `SpawnNPC(x, y, T)`
  its ambient spawning can reach, with the type argument resolved through local variables,
  `Main.rand.Next(a, b)` ranges, `Utils.SelectRandom` lists and the class's own `Get*ToSpawn`
  helpers. Anything it cannot resolve is *reported*, never guessed, so this side is an honest
  lower bound: it can miss a gap, it cannot invent one.

* Our side comes from the server itself, by calling every ambient producer it has. The test
  `game::spawn::tests::every_ambient_producer_names_a_real_type` builds that set and prints it;
  this runs the test and reads the line. Nothing here parses Rust.

Both sides are ambient spawning only. Statues, `/spawn`, boss summon items, worm segments and the
transformations one NPC undergoes on another's death are out of scope on both sides.

Usage:
    python3 tools/check_spawn_reach.py /path/to/decompiled [--update] [--self-test]

The known gaps live in `docs/spawn-gaps.tsv`, which this writes with `--update` and compares
against otherwise, so a *new* hole fails the check instead of joining the noise.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
GAPS = REPO / "docs" / "spawn-gaps.tsv"

# The one test that builds and prints our side.
ROSTER_TEST = "game::spawn::tests::every_ambient_producer_names_a_real_type"
ROSTER_MARK = "SPAWN-REACH "

# Helper methods on `NPC.Spawner` that answer with a type rather than taking one.
HELPERS = (
    "GetBasicSlimeToSpawn",
    "GetGemBunnyToSpawn",
    "GetGemSquirrelToSpawn",
    "RollDragonflyType",
)


# ---------------------------------------------------------------------------- vanilla side


def brace_block(source: str, start: int) -> str:
    """The `{ ... }` block that opens at or after `start`, brace-matched."""
    opening = source.index("{", start)
    depth = 0
    for i in range(opening, len(source)):
        if source[i] == "{":
            depth += 1
        elif source[i] == "}":
            depth -= 1
            if depth == 0:
                return source[opening : i + 1]
    raise ValueError("unclosed block")


def class_body(source: str, header: str) -> str:
    """The text of a class, from its header to its matching closing brace."""
    return brace_block(source, source.index(header))


# A method header at one indent inside a class: a return type, a name, an argument list, a brace.
METHOD = re.compile(
    r"\n\t\t(?:(?:public|private|protected|internal|static|virtual|override|unsafe) )*"
    r"[\w<>\[\],.? ]+ \w+\([^;{}]*\)\s*\n\t\t\{"
)


def methods(class_text: str) -> list[str]:
    """Every method body in a class, so a local variable is resolved in its own scope only.

    Resolving them class-wide instead conflates every `num` in the file, which both invents ids
    a method cannot produce and hides the ones it can.
    """
    return [brace_block(class_text, match.end() - 1) for match in METHOD.finditer(class_text)]


def split_args(text: str) -> list[str]:
    """Split a call's argument list on commas that are not inside a nested bracket.

    Angle brackets are deliberately not counted: `>=` and `<` are comparisons far more often than
    they are generics here, and counting them sent the depth negative and swallowed whole blocks.
    """
    args, depth, current = [], 0, ""
    for ch in text:
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        if ch == "," and depth == 0:
            args.append(current.strip())
            current = ""
        else:
            current += ch
    if current.strip():
        args.append(current.strip())
    return args


def split_top(text: str, wanted: str) -> list[str]:
    """Split on a character that is not inside a nested bracket."""
    parts, depth, current = [], 0, ""
    for ch in text:
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        if ch == wanted and depth == 0:
            parts.append(current)
            current = ""
        else:
            current += ch
    parts.append(current)
    return parts


def calls(body: str, name: str) -> list[list[str]]:
    """Every call to `name` in `body`, as argument lists."""
    out = []
    for match in re.finditer(rf"\b{name}\(", body):
        depth, start = 0, match.end()
        for i in range(match.end() - 1, len(body)):
            if body[i] == "(":
                depth += 1
            elif body[i] == ")":
                depth -= 1
                if depth == 0:
                    out.append(split_args(body[start:i]))
                    break
    return out


LITERAL = re.compile(r"^-?\d+$")
RANGE = re.compile(r"^(?:\(\w+\))?Main\.rand\.Next\((-?\d+),\s*(-?\d+)\)$")
OFFSET = re.compile(r"^(\w+)\s*\+\s*Main\.rand\.Next\((\d+)\)$")
SELECT = re.compile(r"^Utils\.SelectRandom(?:<\w+>)?\(Main\.rand,\s*(.*)\)$", re.S)
ARRAY = re.compile(r"^new \w+\[\d*\]\s*\{(.*)\}$", re.S)
CAST = re.compile(r"^\((?:short|int|byte|ushort)\)\s*")


def wrapped_in_parens(expr: str) -> bool:
    """Whether the whole expression sits inside one redundant pair of parentheses."""
    if not expr.startswith("(") or not expr.endswith(")"):
        return False
    depth = 0
    for i, ch in enumerate(expr):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return i == len(expr) - 1
    return False


def resolve(expr: str, scope: str, spawner: str, seen: set[str]) -> tuple[set[int], set[str]]:
    """Resolve one type expression to the NPC ids it can produce.

    `scope` is the method the expression sits in, which is where its locals are looked up;
    `spawner` is the whole class, which is where its helper methods are. Returns the ids and
    whatever could not be resolved, which is reported rather than assumed.
    """
    expr = CAST.sub("", expr.strip()).strip()
    while wrapped_in_parens(expr):
        expr = CAST.sub("", expr[1:-1].strip()).strip()
    if LITERAL.match(expr):
        return {int(expr)}, set()
    match = RANGE.match(expr)
    if match:
        return set(range(int(match.group(1)), int(match.group(2)))), set()
    match = OFFSET.match(expr)
    if match:
        base, rest = resolve(match.group(1), scope, spawner, seen)
        span = int(match.group(2))
        return {b + step for b in base for step in range(span)}, rest
    # A ternary of type expressions is a choice between them, so both sides count.
    if "?" in expr and ":" in expr:
        head = split_top(expr, "?")
        if len(head) == 2:
            branches = split_top(head[1], ":")
            if len(branches) == 2:
                found, unresolved = set(), set()
                for branch in branches:
                    ids, rest = resolve(branch, scope, spawner, seen)
                    found |= ids
                    unresolved |= rest
                return found, unresolved
    match = SELECT.match(expr)
    if match:
        return resolve(match.group(1), scope, spawner, seen)
    match = ARRAY.match(expr)
    if match:
        parts = [CAST.sub("", p.strip()).strip() for p in split_args(match.group(1))]
        if parts and all(LITERAL.match(p) for p in parts):
            return {int(p) for p in parts}, set()
        return set(), {expr}
    parts = [p.strip() for p in split_args(expr)]
    if len(parts) > 1:
        if all(LITERAL.match(CAST.sub("", p).strip()) for p in parts):
            return {int(CAST.sub("", p).strip()) for p in parts}, set()
        return set(), {expr}
    # A list built up with `Add` and then drawn from, which is how the graveyard picks its ghosts.
    built = re.match(r"^(\w+)\.ToArray\(\)$", expr)
    if built:
        added = re.findall(rf"\b{re.escape(built.group(1))}\.Add\((-?\d+)\);", scope)
        if added:
            return {int(value) for value in added}, set()
        return set(), {expr}
    # A call to one of the class's own "what should spawn here" helpers.
    helper = re.match(r"^(\w+)\(", expr)
    if helper and helper.group(1) in HELPERS:
        return helper_types(spawner, helper.group(1)), set()
    # A local: resolve it through every assignment it has in this method.
    if re.fullmatch(r"[A-Za-z_]\w*", expr):
        if expr in seen:
            return set(), set()
        found, unresolved = set(), set()
        # `=` only, never `==`/`!=`/`>=`/`<=`: reading a comparison as an assignment is how a
        # whole block ends up as a "type expression".
        pattern = re.compile(rf"(?<![=!<>])\b{re.escape(expr)}\s*=(?!=)\s*([^;]+);")
        assignments = pattern.findall(scope)
        if not assignments:
            return set(), {expr}
        for rhs in assignments:
            ids, rest = resolve(rhs, scope, spawner, seen | {expr})
            found |= ids
            unresolved |= rest
        return found, unresolved
    return set(), {expr}


def helper_types(spawner: str, name: str) -> set[int]:
    """The ids a `Get*ToSpawn` helper can answer with, read out of its own body."""
    header = re.search(rf"\b\w+ {name}\([^;{{}}]*\)\s*\n", spawner)
    if not header:
        return set()
    body = brace_block(spawner, header.end())
    found = set()
    for literal in re.findall(
        r"(?:return|result\s*=|=)\s*\(?(?:\((?:short|int)\)\s*)?\(?(-?\d+)\)?\s*;", body
    ):
        found.add(int(literal))
    for low, high in re.findall(r"Main\.rand\.Next\((-?\d+),\s*(-?\d+)\)", body):
        found |= set(range(int(low), int(high)))
    return found


def roster_of(spawner: str) -> tuple[set[int], set[str]]:
    """Every NPC id a `Spawner` class body can spawn, plus what could not be resolved."""
    found, unresolved = set(), set()
    for scope in methods(spawner):
        for args in calls(scope, "SpawnNPC"):
            if len(args) < 3:
                continue  # `SpawnNPC()`, the entry point, which takes no type.
            ids, rest = resolve(args[2], scope, spawner, set())
            found |= ids
            unresolved |= rest
    # Negative ids are net ids for the coloured slime variants, not types of their own.
    return {i for i in found if i > 0}, unresolved


def vanilla_roster(tree: Path) -> tuple[set[int], set[str]]:
    source = (tree / "Terraria" / "NPC.cs").read_text(encoding="utf-8", errors="replace")
    return roster_of(class_body(source, "public class Spawner"))


def npc_names(tree: Path) -> dict[int, str]:
    source = (tree / "Terraria.ID" / "NPCID.cs").read_text(encoding="utf-8", errors="replace")
    names = {}
    for name, value in re.findall(r"public const short (\w+) = (-?\d+);", source):
        names.setdefault(int(value), name)
    return names


# ---------------------------------------------------------------------------- our side


def our_roster() -> set[int]:
    """Ask the server itself what it can spawn, by running the test that prints it."""
    result = subprocess.run(
        [
            "cargo",
            "test",
            "-q",
            "-p",
            "terrustia",
            "--lib",
            "--",
            "--exact",
            ROSTER_TEST,
            "--nocapture",
        ],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    for line in result.stdout.splitlines():
        if line.startswith(ROSTER_MARK):
            return {int(part) for part in line[len(ROSTER_MARK) :].split()}
    sys.exit(
        f"{ROSTER_TEST} printed no {ROSTER_MARK.strip()} line.\n"
        f"{result.stdout[-2000:]}\n{result.stderr[-2000:]}"
    )


# ---------------------------------------------------------------------------- the ledger


def read_gaps() -> dict[int, str]:
    if not GAPS.exists():
        return {}
    gaps = {}
    for line in GAPS.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        npc_id, name = line.split("\t")[:2]
        gaps[int(npc_id)] = name
    return gaps


def write_gaps(missing: dict[int, str]) -> None:
    lines = [
        "# NPC types vanilla's own ambient spawning can produce and this server cannot.",
        "# Generated by tools/check_spawn_reach.py --update from a decompiled tree. Never",
        "# hand-edited: rerun the tool and review the diff, as with the other generated tables.",
        "# id\tname",
    ]
    lines += [f"{npc_id}\t{name}" for npc_id, name in sorted(missing.items())]
    GAPS.write_text("\n".join(lines) + "\n", encoding="utf-8")


# ---------------------------------------------------------------------------- self-test

SELF_TEST = """
	public class Spawner
	{
		public void SpawnAnNPC(int spawnTileX, int spawnTileY)
		{
			if (skyMob)
			{
				SpawnNPC(spawnTileX * 16 + 8, spawnTileY * 16, 48);
			}
			else if (hardMode)
			{
				int num = Utils.SelectRandom<int>(Main.rand, 424, 424, 420);
				SpawnNPC(spawnTileX * 16 + 8, spawnTileY * 16, num, 1);
				short type8 = 3;
				if (Main.halloween)
				{
					type8 = 132;
				}
				SpawnNPC(spawnTileX * 16 + 8, spawnTileY * 16, type8);
				SpawnNPC(spawnTileX * 16 + 8, spawnTileY * 16, Main.rand.Next(305, 308));
				SpawnNPC(spawnTileX * 16 + 8, spawnTileY * 16, GetGemBunnyToSpawn());
				SpawnNPC(spawnTileX * 16 + 8, spawnTileY * 16, mysteryValue);
			}
		}

		public static int GetGemBunnyToSpawn()
		{
			int num = Main.rand.Next(100);
			if (num < 5)
			{
				return 651;
			}
			return 46;
		}
	}
"""


def self_test() -> None:
    found, unresolved = roster_of(class_body(SELF_TEST, "public class Spawner"))
    expected = {48, 424, 420, 3, 132, 305, 306, 307, 651, 46}
    assert found == expected, f"{sorted(found)} != {sorted(expected)}"
    assert unresolved == {"mysteryValue"}, unresolved
    print("self-test ok: literals, SelectRandom lists, locals, rand ranges and helpers all read")


# ---------------------------------------------------------------------------- main


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tree", nargs="?", help="path to the decompiled Terraria tree")
    parser.add_argument("--update", action="store_true", help="rewrite docs/spawn-gaps.tsv")
    parser.add_argument("--self-test", action="store_true", help="check the parser and exit")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0
    if not args.tree:
        parser.error("a decompiled tree is required")

    tree = Path(args.tree)
    vanilla, unresolved = vanilla_roster(tree)
    ours = our_roster()
    names = npc_names(tree)

    missing = {i: names.get(i, f"npc{i}") for i in sorted(vanilla - ours)}
    extra = sorted(ours - vanilla)

    print(f"vanilla ambient roster: {len(vanilla)} types")
    print(f"reachable here:         {len(ours)} types")
    print(f"unreachable here:       {len(missing)} types")
    if unresolved:
        print(
            f"\n{len(unresolved)} vanilla type expressions could not be resolved, so the list "
            "above is a lower bound:"
        )
        for expr in sorted(unresolved):
            print(f"  {expr}")
    if extra:
        print(
            f"\n{len(extra)} types are reachable here and not from `NPC.Spawner` (an event roster "
            "vanilla spawns elsewhere, or an over-reach worth checking):"
        )
        print("  " + " ".join(str(i) + " " + names.get(i, "?") for i in extra))

    print("\nunreachable:")
    for npc_id, name in missing.items():
        print(f"  {npc_id}\t{name}")

    if args.update:
        write_gaps(missing)
        print(f"\nwrote {GAPS.relative_to(REPO)}; review the diff before committing")
        return 0

    known = read_gaps()
    if not known:
        print(f"\nno {GAPS.relative_to(REPO)} yet; run with --update to record this as the baseline")
        return 1
    new = sorted(set(missing) - set(known))
    fixed = sorted(set(known) - set(missing))
    if new:
        print("\nNEW unreachable types since the last baseline:")
        for npc_id in new:
            print(f"  {npc_id}\t{missing[npc_id]}")
    if fixed:
        print("\nno longer unreachable (rerun with --update to record it):")
        for npc_id in fixed:
            print(f"  {npc_id}\t{known[npc_id]}")
    return 1 if new or fixed else 0


if __name__ == "__main__":
    sys.exit(main())
