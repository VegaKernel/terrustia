#!/usr/bin/env python3
"""Generate crates/terrustia-proto/src/buffs.rs from the decompiled tables.

Four things live here, all of them per-type data the hand-written buff code must not carry:
  * which buff ids are debuffs (Main.debuff), which decides what AddBuff may evict
  * which are whip marks (BuffID.Sets.IsAnNPCWhipDebuff), which immunity branches on
  * what each NPC type is immune to (NPCID.Sets.DebuffImmunitySets + ShimmerImmunity)
  * which buffs a PvP-flagged player may spread to another (Main.pvpBuff), which gates packet 55
"""
import re
import sys
import pathlib

ROOT = pathlib.Path(sys.argv[1])
OUT = pathlib.Path(sys.argv[2])

main_cs = (ROOT / "Terraria/Main.cs").read_text(errors="replace")
buff_cs = (ROOT / "Terraria.ID/BuffID.cs").read_text(errors="replace")
npc_cs = (ROOT / "Terraria.ID/NPCID.cs").read_text(errors="replace")

BUFF_COUNT = int(re.search(r"public static readonly int Count = (\d+);", buff_cs).group(1))


def bool_set(text, name):
    m = re.search(rf"public static bool\[\] {name} = Factory\.CreateBoolSet\(([^)]*)\);", text)
    if not m:
        raise SystemExit(f"no bool set {name}")
    body = m.group(1).strip()
    if not body:
        return set()
    parts = [p.strip() for p in body.split(",") if p.strip()]
    # A leading `false` flips the set's default; none of the ones read here do that.
    assert parts[0] not in ("true", "false"), name
    return {int(p) for p in parts}


# --- Main.debuff -----------------------------------------------------------
debuff = {int(m.group(1)) for m in re.finditer(r"\n\t\tdebuff\[(\d+)\] = true;", main_cs)}
if not debuff:
    raise SystemExit("no debuff entries found")

# --- Main.pvpBuff -----------------------------------------------------------
pvp_buff = {int(m.group(1)) for m in re.finditer(r"\n\t\tpvpBuff\[(\d+)\] = true;", main_cs)}
if not pvp_buff:
    raise SystemExit("no pvpBuff entries found")

whip = bool_set(buff_cs, "IsAnNPCWhipDebuff")
removable = bool_set(buff_cs, "CanBeRemovedByNetMessage")
frozen_time = bool_set(buff_cs, "TimeLeftDoesNotDecrease")
shimmer_immune = bool_set(npc_cs, "ShimmerImmunity")

# --- NPCID.Sets.DebuffImmunitySets ----------------------------------------
start = npc_cs.index("public static Dictionary<int, NPCDebuffImmunityData> DebuffImmunitySets")
# Walk braces from the opening `{` of the initialiser to its match.
open_at = npc_cs.index("{", npc_cs.index("=", start))
depth = 0
for i in range(open_at, len(npc_cs)):
    if npc_cs[i] == "{":
        depth += 1
    elif npc_cs[i] == "}":
        depth -= 1
        if depth == 0:
            end = i
            break
body = npc_cs[open_at + 1 : end]

# Each entry is either `{ N, null },` or `{ N, new NPCDebuffImmunityData { ... } }`.
entries = {}
pos = 0
while True:
    m = re.compile(r"\{\s*(\d+),\s*").search(body, pos)
    if not m:
        break
    npc_type = int(m.group(1))
    rest = body[m.end() :]
    if rest.lstrip().startswith("null"):
        entries[npc_type] = None
        pos = m.end() + rest.index("null") + 4
        continue
    # Find the NPCDebuffImmunityData initialiser block and match its braces.
    init = rest.index("{")
    d = 0
    for j in range(init, len(rest)):
        if rest[j] == "{":
            d += 1
        elif rest[j] == "}":
            d -= 1
            if d == 0:
                inner = rest[init + 1 : j]
                pos = m.end() + j
                break
    data = {
        "whips": "ImmuneToWhips = true" in inner,
        "all": "ImmuneToAllBuffsThatAreNotWhips = true" in inner,
        "specific": [],
    }
    sm = re.search(r"SpecificallyImmuneTo = new int\[\d+\]\s*\{([^}]*)\}", inner)
    if sm:
        data["specific"] = [int(x) for x in re.findall(r"\d+", sm.group(1))]
    entries[npc_type] = data

if len(entries) < 500:
    raise SystemExit(f"only {len(entries)} immunity entries; the parse is wrong")


def immunity_mask(npc_type):
    """The set of buff ids `npc_type` cannot be given, as NPC.SetDefaults builds it."""
    data = entries.get(npc_type)
    immune = set()
    if data:
        if data["whips"] or data["all"]:
            for b in range(1, BUFF_COUNT):
                is_whip = b in whip
                if (is_whip and data["whips"]) or (not is_whip and data["all"]):
                    immune.add(b)
        immune |= set(data["specific"])
    # The three corrections SetDefaults applies afterwards, in its order.
    if 20 in immune:
        immune.add(30)
        immune.add(375)
    if 69 in immune:
        immune.add(36)
    if npc_type in shimmer_immune:
        immune.add(353)
    else:
        immune.discard(353)
    return immune


NPC_COUNT = max(entries) + 1
masks = {t: immunity_mask(t) for t in range(NPC_COUNT)}

# Most types share one of a handful of masks, so intern them and index per type.
unique = {}
order = []
for t in range(NPC_COUNT):
    key = tuple(sorted(masks[t]))
    if key not in unique:
        unique[key] = len(order)
        order.append(key)
index = [unique[tuple(sorted(masks[t]))] for t in range(NPC_COUNT)]


def bits(ids):
    """A buff-id set as a little-endian bitmap of u64 words."""
    words = [0] * ((BUFF_COUNT + 63) // 64)
    for b in ids:
        if 0 <= b < BUFF_COUNT:
            words[b // 64] |= 1 << (b % 64)
    return words


def bool_table(name, members, count, doc):
    lines = [doc, f"pub const {name}: [bool; {count}] = ["]
    row = []
    for i in range(count):
        row.append("true," if i in members else "false,")
        if len(row) == 16:
            lines.append("    " + " ".join(row))
            row = []
    if row:
        lines.append("    " + " ".join(row))
    lines.append("];")
    return "\n".join(lines)


out = []
out.append(
    """//! Buff tables, generated from the game's own.
//!
//! Nothing here is an algorithm. The rules that read these live in `game::npc` and
//! `game::server`; what varies per buff id or per NPC type is data, and data belongs in a
//! table rather than in a hand-written match a later version would silently invalidate.
//!
//! Generated by `tools/gen_buffs.py` from Terraria 1.4.5.7. Do not edit by hand.

/// How many buff ids exist. `BuffID.Count`.
pub const BUFF_COUNT: usize = %d;
"""
    % BUFF_COUNT
)

out.append(
    bool_table(
        "DEBUFF",
        debuff,
        BUFF_COUNT,
        """
/// Whether a buff id is a debuff, from `Main.debuff`.
///
/// `AddBuff` reads this to decide what it may evict when an NPC's twenty slots are full: a
/// good buff can be pushed out to make room, a debuff never can.""",
    )
)

out.append(
    bool_table(
        "WHIP_MARK",
        whip,
        BUFF_COUNT,
        """
/// Whether a buff id is a whip's mark, from `BuffID.Sets.IsAnNPCWhipDebuff`.
///
/// Immunity treats the two kinds separately — several bosses shrug off every debuff but can
/// still be tagged by a whip — so the distinction has to be kept.""",
    )
)

out.append(
    bool_table(
        "REMOVABLE_BY_REQUEST",
        removable,
        BUFF_COUNT,
        """
/// Whether a client may ask the server to take a buff off an NPC, from
/// `BuffID.Sets.CanBeRemovedByNetMessage`.
///
/// Empty in this version, and deliberately so: the packet exists and the game validates
/// against this set, so every request is refused. Keeping the table means a later version
/// that fills it in needs no code change.""",
    )
)

out.append(
    bool_table(
        "TIME_DOES_NOT_DECREASE",
        frozen_time,
        BUFF_COUNT,
        """
/// Buffs whose timer does not run down, from `BuffID.Sets.TimeLeftDoesNotDecrease`.""",
    )
)

out.append(
    bool_table(
        "PVP_BUFF",
        pvp_buff,
        BUFF_COUNT,
        """
/// Buffs a PvP-flagged player may spread to another PvP-flagged player, from `Main.pvpBuff`.
///
/// Gates packet 55 (`AddPlayerBuffPvP`): a hostile-marked player who lands one of these on
/// another hostile-marked player asks the server to relay it, rather than the target's own
/// client trusting the attacker's client directly.""",
    )
)

words_per = (BUFF_COUNT + 63) // 64
out.append(
    f"""
/// The distinct immunity masks, as bitmaps over buff ids.
///
/// Six hundred and ninety-one NPC types share far fewer than that many distinct sets of
/// immunities, so the masks are interned and [`IMMUNITY_OF`] indexes into them.
const MASKS: [[u64; {words_per}]; {len(order)}] = ["""
)
for key in order:
    w = bits(key)
    out.append("    [" + ", ".join(f"0x{x:016x}" for x in w) + "],")
out.append("];")

out.append(
    f"""
/// Which mask each NPC type uses, from `NPCID.Sets.DebuffImmunitySets` with the corrections
/// `NPC.SetDefaults` applies on top (poison implies bleeding and hemorrhage, ichor implies
/// broken armour, and shimmer immunity is set from its own list either way).
const IMMUNITY_OF: [u16; {NPC_COUNT}] = ["""
)
row = []
for i in range(NPC_COUNT):
    row.append(f"{index[i]},")
    if len(row) == 24:
        out.append("    " + " ".join(row))
        row = []
if row:
    out.append("    " + " ".join(row))
out.append("];")

out.append(
    """
/// Whether `npc_type` can be given `buff`.
///
/// An unknown type is immune to nothing, which matches the game: `SetDefaults` clears the
/// whole array for a type with no entry.
pub fn npc_is_immune(npc_type: u16, buff: u16) -> bool {
    let buff = buff as usize;
    if buff == 0 || buff >= BUFF_COUNT {
        return true;
    }
    let Some(&slot) = IMMUNITY_OF.get(npc_type as usize) else {
        return false;
    };
    MASKS[slot as usize][buff / 64] >> (buff % 64) & 1 == 1
}

/// Whether a buff id is one the game counts as a debuff.
pub fn is_debuff(buff: u16) -> bool {
    DEBUFF.get(buff as usize).copied().unwrap_or(false)
}

/// Whether a PvP-flagged player may spread `buff` to another over packet 55.
pub fn is_pvp_spreadable(buff: u16) -> bool {
    PVP_BUFF.get(buff as usize).copied().unwrap_or(false)
}
"""
)

OUT.write_text("\n".join(out) + "\n")
print(f"wrote {OUT}: {BUFF_COUNT} buffs, {NPC_COUNT} npc types, {len(order)} distinct masks")
print(f"  debuffs {len(debuff)}, whip marks {sorted(whip)}, removable {sorted(removable)}")
print(f"  pvp-spreadable {sorted(pvp_buff)}")
