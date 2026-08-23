# Tile entities

Most tiles are only a number. A handful carry state a tile cannot hold, and those are tile
entities — kept beside the world rather than in it.

**Code:** [`terrustia-proto/src/tile_entity.rs`](../crates/terrustia-proto/src/tile_entity.rs),
handlers in [`game/server.rs`](../crates/terrustia/src/game/server.rs).

## The eleven kinds

| Id | Kind | Tile | Holds |
|---:|---|---:|---|
| 0 | Training dummy | 378 | the slot of the NPC it has put out |
| 1 | Item frame | 395 | one item |
| 2 | Logic sensor | 423 | which condition it watches, and whether it is satisfied |
| 3 | Display doll (mannequin) | 470 | 9 armour, 9 dyes, 1 accessory, and a pose |
| 4 | Weapons rack | 471 | one item |
| 5 | Hat rack | 475 | 2 hats and 2 dyes |
| 6 | Food platter | 520 | one item |
| 7 | Teleportation pylon | 597 | nothing — its network is the tile's own frame |
| 8 | Dead Cells display jar | 704 | one item |
| 9 | Kite anchor | — | the item it was let out of |
| 10 | Critter anchor | — | the item it was let out of |

The numbering is `TileEntitiesManager.RegisterAll`'s registration order. The two anchors ride
other tiles and have no home of their own.

Three matter beyond decoration:

- A **training dummy** puts an NPC in front of itself when somebody comes near and takes it away
  when they leave. That is the only way NPC 488 ever exists.
- A **teleportation pylon** is a tile entity, which is why a pylon network is something the server
  has to keep rather than something a client can assert. Pylons are how a 1.4 world is crossed.
- A **logic sensor** is the only *input* wiring has that is not a lever somebody pulled.

## Two serialised forms, and the trap between them

An entity is written differently depending on where it is going. `TileEntity::write` takes a
`network: bool` for exactly this.

| | World file | Network (packet 86) |
|---|---|---|
| Kind byte | yes | yes |
| **Id** | **yes** | **no** |
| Position | yes | yes |
| Logic sensor's condition and state | **yes** | **no** |
| Everything else | yes | yes |

The trap: **the section stream carries the file form, not the network form.** The game writes it
with `TileEntity.Write`'s default argument, which is easy to read straight past. A section built
with the network form is silently short by four bytes per entity and desyncs everything after it.

A logic sensor's kind and state going to the file and to nobody else is correct rather than an
omission — a client learns what a sensor did from the tiles its circuit changed.

## Multi-slot entities send only what is filled

A mannequin has nineteen slots. Sending all of them every time would be a hundred bytes for a
bare one, so three bytes of presence flags come first and only the filled slots follow:

```
byte  equip_bits    slots 0..7 of the armour
byte  dye_bits      slots 0..7 of the dyes
byte  pose
byte  extra_bits    bit 0: the accessory; bit 1: equip[8]; bit 2: dyes[8]
                    then (short type, byte prefix, short stack) for each set bit, in order
```

A hat rack is the same idea with one byte covering two hats and two dyes.

## Updates go to everyone, not to those nearby

All nine of the game's own call sites use `SendData(86, -1, -1, ...)`. This matters and is not
merely following along:

A client keeps its copy of an entity after walking away, and a section is only re-sent when its
**tiles** change — which filling an item frame does not do. Sending only to players in range
would leave an absent client believing in whatever contents it last saw, permanently.

There are a few hundred of these in a world and they change only when somebody touches one, so
the cost is nothing like an NPC sync's.

## One player at a time

Packet 122 claims an entity. The server refuses a claim on one somebody else holds, which is what
stops two people emptying the same mannequin into their own inventories at once. A dropped
connection releases the claim — without that, a mannequin somebody was looking at when their
connection died stays locked for the rest of the world's life.

## Persistence

Tile entities are **section 5** of the `.wld`. This server reads that section and writes it back
from its own state rather than copying the bytes it loaded.

Copying would mean the world remembered the pylons it had when it was opened and nothing since: a
pylon placed while the server ran would vanish at the next save, and one that had been mined would
come back. See [world-file.md](world-file.md) for how the surrounding sections are handled.

## Packets

| Id | Direction | What |
|---|---|---|
| 86 | server → clients | one entity's whole state, or word that it is gone |
| 87 | client → server | "I placed one here" |
| 122 | both | which entity a player has open; −1 releases |
| 89 / 123 / 133 / 149 | client → server | put an item in a frame / rack / platter / jar |
| 121 | both | one slot of a mannequin (command 2 is the pose) |
| 124 | both | one slot of a hat rack (the dye flag is folded into the slot number, +2) |
| 156 | client → server | clip a kite or critter onto its anchor |
