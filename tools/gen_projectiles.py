#!/usr/bin/env python3
"""Generate `projectile_data.rs` from `Projectile.SetDefaults`.

    python3 tools/gen_projectiles.py <decompiled-tree> crates/terrustia-proto/src/projectile_data.rs

Why this exists: the table it replaces was hand-written and held 27 types. The AI routines name 39,
so 32 of them did not exist — and `Projectiles::launch` returns `None` for an unknown type, which
made the shot silently not happen. The Destroyer fired no lasers, Golem no fireballs, the Moon Lord
no deathray, and the Empress of Light had no attacks at all. Nothing logged, because nothing
failed: the AI decided to fire and the result was dropped on the floor.

That is exactly the failure the project's own rule exists to prevent — per-type variation belongs
in a generated table, because a hand-written one stops at whatever the author needed that day.

`SetDefaults` is a flat `if (type == N) { ... } else if (type == N) { ... }` chain preceded by a
block of defaults, so it parses without needing to understand C#. Anything whose body is not plain
field assignments is skipped and reported rather than guessed at.
"""

import re
import sys
from pathlib import Path

# Fields we care about, and how to read them out of a line of C#.
NUMERIC = {
    "width": r"^\s*width\s*=\s*(-?\d+);",
    "height": r"^\s*height\s*=\s*(-?\d+);",
    "aiStyle": r"^\s*aiStyle\s*=\s*(-?\d+);",
    "penetrate": r"^\s*penetrate\s*=\s*(-?\d+);",
    "timeLeft": r"^\s*timeLeft\s*=\s*(-?\d+);",
    "extraUpdates": r"^\s*extraUpdates\s*=\s*(-?\d+);",
}
BOOLEAN = {
    "tileCollide": r"^\s*tileCollide\s*=\s*(true|false);",
    "hostile": r"^\s*hostile\s*=\s*(true|false);",
}
FLOAT = {"knockBack": r"^\s*knockBack\s*=\s*(-?[\d.]+)f?;"}

# `SetDefaults` sets these before the per-type chain; anything a type does not override keeps them.
DEFAULTS = {
    "width": 0,
    "height": 0,
    "aiStyle": 0,
    "penetrate": 1,
    "timeLeft": 3600,
    "extraUpdates": 0,
    "tileCollide": True,
    "hostile": False,
    "knockBack": 0.0,
}


def read_names(root: Path) -> dict[int, str]:
    """`ProjectileID`'s constant names, for readable logs."""
    text = (root / "Terraria.ID" / "ProjectileID.cs").read_text(errors="replace")
    names: dict[int, str] = {}
    for name, value in re.findall(r"public const short (\w+)\s*=\s*(\d+);", text):
        names.setdefault(int(value), name)
    return names


def parse(root: Path) -> tuple[dict[int, dict], list[int]]:
    """Every type `SetDefaults` describes, and the ones whose body we could not read cleanly."""
    text = (root / "Terraria" / "Projectile.cs").read_text(errors="replace").splitlines()

    start = next(i for i, l in enumerate(text) if "public void SetDefaults(int Type)" in l)

    # Conditions come singly and in groups: `if (type == 674 || type == 673)`. Missing the grouped
    # form is what left the Dark Mage with no portal and no heal.
    chain = re.compile(r"^\s*(?:else\s+)?if \((type == \d+(?:\s*\|\|\s*type == \d+)*)\)\s*$")
    # A group's body may narrow to one of its members again, and those lines apply only to it.
    nested = re.compile(r"^\s*(?:else\s+)?if \(type == (\d+)\)\s*$")

    out: dict[tuple[int, ...], list[str]] = {}
    i = start

    while i < len(text):
        line = text[i]
        match = chain.match(line)
        if match:
            kinds = tuple(int(n) for n in re.findall(r"type == (\d+)", match.group(1)))
            i += 2  # skip the `{`
            depth, body = 1, []
            while i < len(text) and depth > 0:
                inner = text[i]
                depth += inner.count("{") - inner.count("}")
                if depth > 0:
                    body.append(inner)
                i += 1
            out[kinds] = body
            continue
        if line.startswith("\t}") and out:
            break
        i += 1

    if_one = re.compile(r"^\s*if \(type == (\d+)\)\s*$")
    elif_one = re.compile(r"^\s*else if \(type == (\d+)\)\s*$")
    else_one = re.compile(r"^\s*else\s*$")

    def assign(line: str, stats: dict) -> None:
        for field, pattern in NUMERIC.items():
            if m := re.match(pattern, line):
                stats[field] = int(m.group(1))
        for field, pattern in BOOLEAN.items():
            if m := re.match(pattern, line):
                stats[field] = m.group(1) == "true"
        for field, pattern in FLOAT.items():
            if m := re.match(pattern, line):
                stats[field] = float(m.group(1))

    def read(lines: list[str], want: int) -> dict:
        """Fold a block's assignments for one type.

        A grouped block narrows again inside itself — `if (type == 76) ... else if (type == 77)
        ... else ...` — and the trailing bare `else` belongs to whichever member fell through.
        Reading that `else` unconditionally gave types 76 and 77 the size of type 78.
        """
        stats = dict(DEFAULTS)
        depth = 0
        taken_at: dict[int, bool] = {}
        i = 0
        while i < len(lines):
            line = lines[i]
            m_if, m_elif, m_else = if_one.match(line), elif_one.match(line), else_one.match(line)
            if m_if or m_elif or m_else:
                already = taken_at.get(depth, False) if (m_elif or m_else) else False
                if m_if:
                    take = int(m_if.group(1)) == want
                elif m_elif:
                    take = not already and int(m_elif.group(1)) == want
                else:
                    take = not already
                taken_at[depth] = already or take

                i += 1
                if i < len(lines) and lines[i].strip() == "{":
                    i += 1
                    inner, body = 1, []
                    while i < len(lines) and inner > 0:
                        inner += lines[i].count("{") - lines[i].count("}")
                        if inner > 0:
                            body.append(lines[i])
                        i += 1
                    if take:
                        for b in body:
                            assign(b, stats)
                continue
            assign(line, stats)
            depth += line.count("{") - line.count("}")
            i += 1
        return stats

    parsed: dict[int, dict] = {}
    unclear: list[int] = []
    for kinds, lines in out.items():
        for kind in kinds:
            stats = read(lines, kind)
            # A type with no size never moves; treat it as one we could not read.
            if stats["width"] == 0 and stats["height"] == 0:
                unclear.append(kind)
                continue
            parsed[kind] = stats
    return parsed, unclear


def emit(stats: dict[int, dict], names: dict[int, str], unclear: list[int]) -> str:
    lines = [
        "//! Stats for every projectile the game defines, generated from `Projectile.SetDefaults`.",
        "//!",
        "//! The server flies the ones an NPC fires or a trap throws; a player's own are simulated",
        "//! by their client and relayed, exactly as against a vanilla server. The whole table is",
        "//! here anyway, because [`ProjectileStats::hostile`] is what a client's claim is checked",
        "//! against — so a type missing from this file is a type a client could lie about.",
        "//!",
        "//! This was hand-written once and held 27 of them. The AI names 39, and",
        "//! `Projectiles::launch` returns `None` for anything absent, so 32 kinds of shot were",
        "//! silently never fired: the Destroyer's lasers, Golem's fireballs, the Moon Lord's",
        "//! deathray, every one of the Empress of Light's attacks.",
        "//!",
        "//! Generated by `tools/gen_projectiles.py`. Do not edit by hand.",
        "",
        "/// Everything the server needs to know about a projectile type.",
        "#[derive(Debug, Clone, Copy, PartialEq)]",
        "pub struct ProjectileStats {",
        "    /// The `ProjectileID` constant name, for logs.",
        "    pub name: &'static str,",
        "    pub width: i32,",
        "    pub height: i32,",
        "    /// Which behaviour routine drives it.",
        "    pub ai_style: i32,",
        "    /// How many things it can hit before it dies. -1 means no limit.",
        "    pub penetrate: i32,",
        "    /// Ticks it lives for. The game's own default is 3600.",
        "    pub time_left: i32,",
        "    /// Whether terrain stops it.",
        "    pub tile_collide: bool,",
        "    /// Whether it hurts players, and so whether a client may claim to own one.",
        "    pub hostile: bool,",
        "    /// Extra movement steps per tick, which is how the fast ones stay accurate.",
        "    pub extra_updates: i32,",
        "    pub knockback: f32,",
        "}",
        "",
        f"/// How many types the table holds.",
        f"pub const COUNT: usize = {len(stats)};",
        "",
        "/// Stats for a projectile type, or `None` for one the game does not define.",
        "pub fn projectile_stats(projectile_type: u16) -> Option<ProjectileStats> {",
        "    let stats = match projectile_type {",
    ]
    for kind in sorted(stats):
        s = stats[kind]
        name = names.get(kind, f"Projectile{kind}")
        lines += [
            f"        {kind} => ProjectileStats {{",
            f'            name: "{name}",',
            f"            width: {s['width']},",
            f"            height: {s['height']},",
            f"            ai_style: {s['aiStyle']},",
            f"            penetrate: {s['penetrate']},",
            f"            time_left: {s['timeLeft']},",
            f"            tile_collide: {str(s['tileCollide']).lower()},",
            f"            hostile: {str(s['hostile']).lower()},",
            f"            extra_updates: {s['extraUpdates']},",
            f"            knockback: {s['knockBack']:.1f},",
            "        },",
        ]
    lines += [
        "        _ => return None,",
        "    };",
        "    Some(stats)",
        "}",
        "",
        "#[cfg(test)]",
        "mod tests {",
        "    use super::*;",
        "",
        "    /// The table covers what the world actually throws.",
        "    #[test]",
        "    fn the_table_is_populated() {",
        f"        assert_eq!(COUNT, {len(stats)});",
        "    }",
        "",
        "    /// Spot checks against the game, on the ones whose absence broke a boss fight.",
        "    #[test]",
        "    fn the_boss_projectiles_are_here() {",
        "        for (kind, what) in [",
        '            (100u16, "the Destroyer\'s laser"),',
        '            (258, "Golem\'s fireball"),',
        '            (455, "the Moon Lord\'s deathray"),',
        '            (462, "the Moon Lord\'s phantasmal sphere"),',
        '            (385, "Duke Fishron\'s bubble"),',
        '            (435, "a caster\'s bolt"),',
        "        ] {",
        "            assert!(",
        "                projectile_stats(kind).is_some(),",
        '                "{what} (type {kind}) is missing, so it would never be fired",',
        "            );",
        "        }",
        "    }",
        "",
        "    /// A type the game does not define stays `None`.",
        "    #[test]",
        "    fn an_unknown_type_has_no_stats() {",
        "        assert!(projectile_stats(u16::MAX).is_none());",
        "    }",
        "}",
        "",
    ]
    if unclear:
        print(
            f"note: {len(unclear)} types had no size in SetDefaults and were skipped "
            f"(first few: {unclear[:8]})",
            file=sys.stderr,
        )
    return "\n".join(lines)


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 1
    root, out = Path(sys.argv[1]), Path(sys.argv[2])
    names = read_names(root)
    stats, unclear = parse(root)
    if len(stats) < 500:
        print(f"error: only parsed {len(stats)} types; the parser is wrong", file=sys.stderr)
        return 1
    out.write_text(emit(stats, names, unclear))
    print(f"wrote {len(stats)} projectile types to {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
