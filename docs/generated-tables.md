# Generated tables

## The rule

**Per-type variation lives in generated tables. Hand-written modules hold algorithms only.**

There are 697 NPC types, 754 tiles, 401 buffs and several thousand items. Any rule that differs
per type is *data*. A hand-written match over 697 cases is wrong the moment the game changes, and
wrong invisibly — nothing fails, a few types just quietly behave like the wrong thing.

So the shape everywhere is: a table generated from the game's own, and a small hand-written module
that reads it.

## What is generated

| File | Lines | From | Generator |
|---|---:|---|---|
| `npc_data.rs` | 13,323 | `NPC.SetDefaults` | — |
| `tile_object.rs` | 6,099 | `TileObjectData.Initialize` | — |
| `npc_params.rs` | 4,721 | `NPCID.Sets`, `NPC.SetDefaults` | — |
| `npc_drops.rs` | 2,644 | `NPCLoot` / `ItemDropRules` | — |
| `placed_items.rs` | 2,550 | `Item.SetDefaults` | — |
| `town_names.rs` | 517 | localisation + `NPC.getNewNPCNameInner` | `gen_town_names.py` |
| `buffs.rs` | ~450 | `Main.debuff`, `BuffID.Sets`, `NPCID.Sets.DebuffImmunitySets` | `gen_buffs.py` |
| `tile_drops.rs` | 395 | `WorldGen.KillTile_GetItemDrops` | — |
| `conditional_drops.rs` | 490 | drop rules with conditions | — |
| `statues.rs` | 313 | `Wiring.HitSwitch` statue cases | — |
| `hurt_tiles.rs` | ~120 | `TileID.Sets` + `Collision.CanTileHurt` | `gen_hurt_tiles.py` |
| `angler.rs` | ~120 | `Main.AnglerQuestSwap` | `gen_angler.py` |

The ones with a generator in [`tools/`](../tools) can be rebuilt:

```sh
D=<path-to-decompiled-1.4.5.7-tree>
python3 tools/gen_buffs.py      "$D" crates/terrustia-proto/src/buffs.rs
python3 tools/gen_town_names.py "$D" crates/terrustia-proto/src/town_names.rs
python3 tools/gen_hurt_tiles.py "$D" crates/terrustia-proto/src/hurt_tiles.rs
python3 tools/gen_angler.py     "$D" crates/terrustia-proto/src/angler.rs
```

Each script fails loudly if the source's shape has changed — a parse that finds too few entries
raises rather than emitting a table that is quietly short. That matters more than it sounds: a
generator that silently produces an empty set turns "immune to nothing" into the default for
every NPC in the game.

## Writing a generator

Things learned the hard way:

**Parse the conditions, do not read them off.** `AnglerQuestSwap`'s fifteen availability rules are
a run of guard clauses. Transcribing them by hand is one typo away from asking a fresh world for a
hardmode fish, which costs the player a whole day. Parsing them means the table is checkable
against the source by re-running the script.

**Watch for conditions that are not about the type.** An early extractor for pre-289 header fields
attributed conditional overrides (`remixWorld`, `!hardMode`) to the *type* rather than to the
condition, and reported 41 disagreements that did not exist. Only the third attempt was right.

**Intern repeated data.** 697 NPC types share only 34 distinct debuff-immunity masks. Emitting 697
bitmaps would be 40× the bytes for the same table.

**Say what is deliberately absent.** `hurt_tiles.rs` omits two tiles the game can make dangerous,
because it gates them behind world seeds this server does not offer. That is recorded in the
file's own doc comment, so the next person to compare against the game finds the answer rather
than the discrepancy.

## What is *not* generated, and why

`tile_sets.rs`, `tile_solid.rs` and the frame-importance table were transcribed rather than
generated. They are stable across versions and were verified mechanically against the source —
754×2 solidity entries and 754 frame-importance flags, all compared. Regenerating them is worth
doing if they ever drift, but they have not.

`is_dungeon_wall` in `game/teleport.rs` is a nine-entry `matches!` rather than a table. That is a
deliberate exception: the set is tiny and has not changed in four major versions, and a
nine-element generated file would be more machinery than the thing it holds.
