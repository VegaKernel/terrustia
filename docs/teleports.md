# Server-side teleports

Five items ask the server to move a player rather than moving them themselves. They arrive as one
packet — id 73 — with a single byte saying which.

| Byte | Item | Where it looks |
|---:|---|---|
| 0 | Teleportation Potion | anywhere above the underworld |
| 1 | Magic Conch | the ocean on the far side of the world from where you are |
| 2 | Demon Conch | the underworld, near the middle first, then anywhere in it |
| 3 | Shellphone (spawn) | the world's spawn point |
| 4 | *no-space rescue* | spawn, when a player is crushed with nowhere to stand |

**Code:** [`game/teleport.rs`](../crates/terrustia/src/game/teleport.rs),
handler `on_server_teleport` in [`game/server.rs`](../crates/terrustia/src/game/server.rs).

## Why the server and not the client

Each has to **search the world** for somewhere safe to land, which means seeing tiles the client
may not have loaded. That is the whole reason these five are a server request while every other
teleport (magic mirror, pylon, recall potion) is the client naming a destination it already knows.

## The search

`Utils.CheckForGoodTeleportationSpot`, ported. Per candidate:

1. Pick a tile at random inside the caller's rectangle, clamped 45 tiles from every edge.
2. Reject if the player's box already overlaps something solid.
3. Reject on the wall rules (below).
4. **Fall** down from there until something is underfoot, up to `max_fall` tiles.
5. Reject if the fall ran out rather than ended.
6. Reject on liquid / lava / hazard, per the caller's wants.
7. Reject if there is no way out that is not straight down.

Step 4 is why a potion tends to land you on a cave floor rather than in mid-air, and the fall
limit is why a candidate over a chasm does not search all the way to the underworld.

### The wall rules

| Wall | Refused when |
|---|---|
| any, if `avoid_walls` | always — this is how the Demon Conch avoids landing you inside a house |
| 87 (Lihzahrd brick) | Plantera is not down — the temple stays sealed |
| dungeon walls (7–9, 94–99) | below the surface and Skeletron is not down |

The dungeon rule is deliberately surface-only: the dungeon's walls above ground are its entrance,
which is meant to be reachable.

## Two things that came out of testing, not reading

### The half-pixel

`Collision.HurtTiles` compares against `tile.Y - 0.5f`, not `tile.Y`. Without that slack, a player
standing exactly on top of a bed of spikes has their feet on the tile boundary — and an ordinary
overlap test calls that "not touching". The search would cheerfully land somebody on spikes.

Found because a test that should have refused a spike bed accepted one.

There is a second wrinkle in the same test: tiles in `TileID.Sets.Suffocate` shrink the box by two
pixels on each side, because they hurt by *enclosing* rather than by contact. Brushing past sand
is not being buried in it.

### Vanilla's last four checks do almost nothing

The game ends its search with four one-tile step tests. Each starts from a *neighbouring* tile and
ends at the spot already known to be clear, so they pass close to by construction — they exist to
catch slope and platform edge cases, not enclosure. A player-shaped pocket in solid rock would
satisfy all four.

That is fine in a real world, where such pockets essentially do not occur. This port adds one
check the game does not have: **is there a way out that is not straight down?** Straight down is
excluded on purpose — floor underfoot is not a wall, it is the point of having landed.

The first attempt at that check asked for a way out in *every* direction, which rejected flat
ground. That is the bug to watch for if this is ever changed.

## Failing is an answer

A search that finds nowhere leaves the player where they are. That is the game's own behaviour and
the right one: a conch that fails is a wasted item, a conch that drops you into lava is a lost
character.

## The hazard table

[`terrustia-proto/src/hurt_tiles.rs`](../crates/terrustia-proto/src/hurt_tiles.rs), generated from
`TileID.Sets`: the union of `TouchDamageBleeding`, `Suffocate`, `TouchDamageHot`, and everything
with a non-zero `TouchDamageImmediate`. Eighteen of 754 tiles, six of which suffocate.

Two the game can make dangerous are deliberately **absent**: `Collision.CanTileHurt` gates tile
230 behind the Ravaged seed and tile 80 behind Don't Starve, and this server offers neither. On it
they are ordinary blocks.
