# Shimmer

The 1.4.4 transmutation pool. An item dropped into it becomes another item, a creature, or — for
coins — luck.

**Code:** [`terrustia-proto/src/shimmer.rs`](../crates/terrustia-proto/src/shimmer.rs) (generated),
`shimmer()` in [`world/items.rs`](../crates/terrustia/src/world/items.rs), `tick_shimmer` in
[`game/server.rs`](../crates/terrustia/src/game/server.rs).

## It is not instant, and that matters

An item does not transmute on contact. It **sinks** — 0.01 per tick, so about a second and a half
— and changes at nine tenths of the way in. Pull it out before then and the counter runs back
down. That delay is the whole feel of the mechanic: shimmer is reversible until it isn't.

The threshold is held at 0.9 rather than 1.0, as the game holds it, so the transformation is
visible rather than happening to something that has already disappeared.

One detail worth recording: it takes **91** ticks, not 90. Adding `0.01f32` ninety times lands a
hair under 0.9. The game accumulates the same way and arrives at the same place, so the off-by-one
is faithful rather than sloppy — and the test asserts a range with that written next to it.

## What the test is

The game checks the tile **above** the item's position, not the one it occupies: an item is
sinking through a surface, not standing in a pool. Getting that wrong makes shimmer either
untriggerable or triggered by being near it.

## What it turns things into

Generated from `ItemID.Sets` and `NPCID.Sets`:

| Table | Entries | What |
|---|---:|---|
| `TRANSFORMS` | 316 | item → item |
| `COUNTS_AS` | 6 | items that shimmer as though they were another |
| `NPC_TRANSFORMS` | 114 | a caught creature → another creature |

Stored as sorted key/value pairs and binary-searched, not as a full array — a few hundred pairs
out of several thousand items would otherwise be mostly zeroes.

**Transform pairs are usually symmetric.** Wood becomes stone and stone becomes wood. So an item
carries a `shimmered` flag once transmuted, and without it a dropped item would flicker between
its two forms forever.

## Coins

Not transmuted but spent: a stack becomes coin luck and vanishes.

| Coin | Worth |
|---|---:|
| Copper | 1 each |
| Silver | 100 each |
| Gold | 10,000 each |
| Platinum | **1,000,000 flat** |

Platinum is capped at one coin's worth however many go in — the game's own rule, and what stops a
stack being worth a billion. The arithmetic saturates rather than wrapping.

## The gap: decrafting

An item with no transform and no creature falls back, in the game, to being **decrafted** into
its recipe's ingredients. That needs the whole recipe database — thousands of entries and a
crafting system this server does not model at all.

Such an item simply does not shimmer here. It sinks, reaches the threshold, and is marked as
having been through so it stops asking the same question every tick.

This is a real gap, not a decision, and it is the largest thing still missing from the mechanic.
It is recorded here rather than papered over with a guess, because a *wrong* decraft — giving back
the wrong ingredients — would be much worse than giving back none.

## Packets

| Id | Direction | What |
|---|---|---|
| 146 action 0 | server → clients | the sparkle where something transmuted |
| 146 action 1 | server → clients | coins became this much luck, here |
