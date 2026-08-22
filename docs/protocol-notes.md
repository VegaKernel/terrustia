# Terraria 1.4.5.7 protocol notes

Derived from the shipped `TerrariaServer.exe` (Steam appid 105600, macOS/Mono/FNA build,
version string `1.4.5.7`) on 2026-08-21. These are our own notes; no decompiled game code is
checked in.

Authoritative sources inside the assembly:

| What | Where |
|---|---|
| Numeric message ids | `Terraria.ID.MessageID` |
| Server parsing of client packets | `MessageBuffer.GetData`, `netMode == 2` branches |
| Client parsing of server packets — i.e. the spec for what we must send | `MessageBuffer.GetData`, `netMode == 1` branches |
| Serialisation of every packet | `NetMessage.SendData` |
| Tile section coding | `NetMessage.CompressTileBlock{,_Inner}` / `DecompressTileBlock{,_Inner}` |

## Version

**`curRelease` = 325**, so the handshake string is exactly **`"Terraria325"`**.

This is *not* guessable from the marketing version. Prior releases: 1.4.4.9 was 279. If the client
is updated, re-derive this before anything else — a mismatch makes the server reply with `Kick` and
is the first thing to check when a client bounces.

The world save format uses the same counter: the user's existing world is header version 319
(1.4.5.6), so a 1.4.5.7 client writes 325.

## Framing

`[u16 length][u8 message id][payload]`, little-endian. **The length counts the whole frame,
including its own two bytes**, so the minimum legal frame is 3 bytes and the payload is
`length - 3`.

Primitives follow .NET `BinaryWriter`: little-endian numbers, `bool` as one byte, strings as a
7-bit-encoded (LEB128) **byte** count followed by UTF-8, `Vector2` as two `f32`, colours as 3 bytes
RGB with no alpha.

`NetworkText` is `[u8 mode][string text]`, and when mode != 0 a `[u8 substitution count]` followed
by that many nested `NetworkText` values. Mode 0 = literal, 1 = formatted, 2 = localization key.

## Connection handshake

Server-side client state lives in `Netplay.Clients[i].State`; the client mirrors it in
`Netplay.Connection.State`. The sequence that gets a player into the world:

| Step | Direction | Packet | Payload |
|---|---|---|---|
| 1 | C→S | `1` Hello | `string "Terraria325"` |
| 2 | S→C | `3` PlayerInfo | `u8 playerSlot`, `bool serverWantsToRunCheckBytesInClientLoopThread` |
| 3 | C→S | `4` SyncPlayer, `5`×N SyncEquipment, `16`, `42`, `50`, `68` ClientUUID | appearance, inventory, vitals |
| 4 | C→S | `6` RequestWorldData | *(empty)* |
| 5 | S→C | `7` WorldData | see below |
| 6 | C→S | `8` SpawnTileData | `i32 x`, `i32 y`, `u8 team` |
| 7 | S→C | `7` WorldData again, then `9` StatusTextSize, then `10`×N TileSection | |
| 8 | S→C | `49` StartPlaying | *(empty)* |
| 9 | C→S | `12` PlayerSpawn | context = `SpawningIntoWorld` |
| 10 | S→C | `12` broadcast, `129` FinishedConnecting | |

Notes:

- Step 2 branches: with a server password set, the server instead sets state `-1` and sends
  `37` RequestPassword, expecting `38` SendPassword.
- On a version mismatch the server sends `2` Kick carrying a `NetworkText`.
- Packet `3` gained a trailing `bool` in 1.4.5; it was a bare slot byte in 1.4.4.
- Packet `11` TileFrameSection is marked deprecated in 1.4.5 ("framing happens as needed after
  TileSection is sent"), so we do not need to send it.
- On receiving `8` the server re-sends `7` before the sections. Harmless to mirror, and we do.
- `12` PlayerSpawn advances the server-side state from 3 to 10, at which point the player is live.

## Packet `7` WorldData

Read in this exact order. Anything omitted or reordered leaves the client stalled with no error.

```
i32   time
u8    flags: bit0 dayTime, bit1 bloodMoon, bit2 eclipse
u8    moonPhase
i16   maxTilesX
i16   maxTilesY
i16   spawnTileX
i16   spawnTileY
i16   worldSurface
i16   rockLayer
i32   worldId
str   worldName
u8    gameMode
[16]  world unique id (GUID bytes)
u64   worldGeneratorVersion
u8    moonType
u8 ×13  background styles, in setBG order 0,10,11,12,1,2,3,4,5,6,7,8,9
u8    iceBackStyle
u8    jungleBackStyle
u8    hellBackStyle
f32   windSpeedTarget
u8    numClouds
i32 ×3  treeX
u8  ×4  treeStyle
i32 ×3  caveBackX
u8  ×4  caveBackStyle
u8  ×13 tree-top variations (TreeTopsInfo, one byte per AreaId, Count == 13)
f32   maxRaining
u8 ×11  world-flag bitfields (boss kills, hardmode, event state, world seeds — see below)
u8    sundialCooldown
u8    moondialCooldown
i16 ×7  saved ore tiers: copper, iron, silver, gold, cobalt, mythril, adamantite
i8    invasionType
u64   lobbyId
f32   sandstormIntendedSeverity
u8    extra spawn point count, then that many (i16 x, i16 y)
```

There are **eleven** flag bytes, verified by tallying every `reader.Read*` call in the client's
case-7 branch (36 `ReadByte`, 17 `ReadInt16`/`ReadInt32`, 3 `ReadSingle`, 2 `ReadUInt64`, 1 each of
`ReadString`, `ReadSByte`, `ReadBytes`). They carry world progression. A fresh world sends all zeros except where noted;
byte 1 bit 6 is `ServerSideCharacter` and must stay 0 unless we implement server-side characters.
Byte ordering, first to last: (1) shadowOrbSmashed / downedBoss1-3 / hardMode / downedClown /
ServerSideCharacter / downedPlantBoss, (2) mech bosses / cloudBGActive / crimson / pumpkinMoon /
snowMoon, (3) bit1 fastForwardTimeToDawn, bit2 slime rain, king slime / queen bee / fishron /
martians / cultist, (4) moonlord and seasonal event kills / ManualParty, (5) pirates / frost /
goblins / sandstorm / DD2 state, (6) combat book / lanterns / pillars / forced holidays,
(7) bought pets / drunkWorld / empress / queen slime / getGoodWorld, (8) anniversary / don't-starve
/ deerclops / not-the-bees / remix / slime unlocks, (9) more slime unlocks / fastForwardTimeToDusk, (10) world seed flags
(noTraps, zenith, truffle, vampire, infected, teamBasedSpawns, skyblock, dualDungeons), and
(11) skyblock low tiles / forced holidays / lightning seeds.

Excluding the world name and any extra spawn points, the payload is exactly **159 bytes**.

`crimson` (byte 2, bit 5) is worth setting deliberately — it selects the evil biome the client
renders.

## Tile sections — packet `10`

**The entire payload is a raw DEFLATE stream** (.NET `DeflateStream`, so no zlib or gzip header).
There is no uncompressed length field and **no leading "is compressed" flag byte** — that flag
exists in older community documentation and is wrong for 1.4.5.

Inside the deflate stream:

```
i32 xStart
i32 yStart
i16 width
i16 height
<RLE tile run> ...
i16 chestCount,  then (i16 id, i16 x, i16 y, str name) ×N
i16 signCount,   then (i16 id, i16 x, i16 y, str text) ×N
i16 entityCount, then serialised tile entities ×N
```

A section is **200 × 150 tiles**; `sectionX = x / 200`, `sectionY = y / 150`. Each section is one
packet. On `8` the server sends the 5×3 block of sections around spawn (`sectionX-2 .. +5`,
`sectionY-1 .. +3`), clamped to the world, plus the same block around the requested position.

### Per-tile encoding

Up to four flag bytes, then the payload fields, then the run length. Each flag byte's bit 0 means
"another flag byte follows", so they chain.

**flags1**
| Bit | Meaning |
|---|---|
| 0x01 | flags2 follows |
| 0x02 | tile is active (has a block) |
| 0x04 | has a wall |
| 0x08 | liquid: water (or shimmer, when flags3 0x80 is set) |
| 0x10 | liquid: lava |
| 0x18 | liquid: honey |
| 0x20 | tile type needs a second byte (type > 255) |
| 0x40 | run length is 1 byte |
| 0x80 | run length is 2 bytes |

**flags2**
| Bit | Meaning |
|---|---|
| 0x01 | flags3 follows |
| 0x02 | red wire |
| 0x04 | blue wire |
| 0x08 | green wire |
| 0x70 | slope: 1 = half brick, otherwise `slope + 1`, shifted left 4 |

**flags3**
| Bit | Meaning |
|---|---|
| 0x01 | flags4 follows |
| 0x02 | actuator |
| 0x04 | actuated (inactive) |
| 0x08 | tile has a paint colour |
| 0x10 | wall has a paint colour |
| 0x20 | yellow wire |
| 0x40 | wall type needs a second byte (wall > 255) |
| 0x80 | the liquid is shimmer |

**flags4**
| Bit | Meaning |
|---|---|
| 0x02 | invisible block |
| 0x04 | invisible wall |
| 0x08 | fullbright block |
| 0x10 | fullbright wall |

Payload after the flag bytes, each part present only if its flag says so:

1. tile type low byte, then high byte if flags1 0x20
2. `i16 frameX`, `i16 frameY` — only when `Main.tileFrameImportant[type]`
3. tile paint colour
4. wall type low byte, then wall paint colour
5. liquid amount
6. wall type high byte
7. run length: low byte, then high byte when the run exceeds 255

### Run-length semantics

The count is the number of *additional* identical tiles after the first, so a lone tile writes no
count at all and sets neither 0x40 nor 0x80. Runs only form between tiles that compare equal *and*
whose type allows batching. When the count exceeds 255 both bytes are written, low first, and
**0x80 is set instead of 0x40**, not in addition to it.

## Other packets in the vertical slice

```
 9 StatusTextSize   i32 statusMax, NetworkText text, u8 flags
12 PlayerSpawn      u8 player, i16 spawnX, i16 spawnY, i32 respawnTimer,
                    i16 deathsPVE, i16 deathsPVP, u8 team, u8 spawnContext
13 PlayerControls   u8 player, u8 ctrl1, u8 ctrl2, u8 ctrl3, u8 ctrl4, u8 selectedItem,
                    Vector2 position,
                    Vector2 velocity        (only when ctrl2 bit 2 set)
                    u16 mountType           (only when ctrl2 bit 7 set)
                    Vector2 ×2 potion-of-return (only when ctrl3 bit 6 set)
                    Vector2 cameraTarget    (only when ctrl4 bit 5 set)
14 PlayerActive     u8 player, bool active
16 PlayerHealth     u8 player, i16 statLife, i16 statLifeMax
18 TimeSet          bool dayTime, i32 time, i16 sunModY, i16 moonModY
36 PlayerZone       u8 player, u8 zone1..zone5, u8 townNPCs
42 PlayerMana       u8 player, i16 statMana, i16 statManaMax
68 ClientUUID       str uuid
```

`13` control bit 6 of the first byte is facing direction (set = facing right), and bit 2 of the
second byte gates whether velocity is present — a still player omits it entirely.

## The `.wld` save format

Transcribed from `Terraria.IO.WorldFile`. Tiles are encoded **exactly** as they are in a network
section — same flag chain, same field order, same run lengths — so the two share one decoder. The
differences are that the file walks the world **column by column** rather than row by row, and that
it carries its own frame-importance table.

```
i32   version                     (325 for 1.4.5.7, 319 for 1.4.5.6)
[7]   "relogic"
u8    file type                   (2 = world)
u32   revision
u64   favorite
i16   section count
i32   × count  section offsets
u16   tile-type count, then that many bits of frame-importance
```

The importance bitset is packed **least significant bit first**: the writer seeds its mask at 0x80
as a sentinel, so the first entry pulls a byte and reads bit 0, then walks 1, 2, 4 … 0x80 before
pulling the next byte. Reading it most-significant-bit-first parses the first column of tiles
correctly and then desynchronises, which is a confusing way to find the mistake.

Section 0 is the world header; section 1 the tiles; 2 chests; 3 signs. Because the offsets are
absolute, a reader can take the header fields it needs and seek straight to the tiles rather than
parsing the long tail of progression flags.

Header fields, up to the last one we need:

```
str   world name
str   seed text                   (v >= 179)
u64   world generator version
[16]  unique id                   (v >= 181)
i32   world id
i32   ×4  world rectangle in pixels
i32   maxTilesY, then maxTilesX   (height first)
i32   game mode                   (v >= 209)
bool  drunk(222) getGood(227) tenthAnniversary(238) dontStarve(239)
      notTheBees(241) remix(249) noTraps(266) zenith(267) skyblock(302)
i64   creation time               (v >= 141)
i64   last played                 (v >= 284)
u8    moon type
i32   treeX[3], treeStyle[4], caveBackX[3], caveBackStyle[4]
i32   iceBackStyle, jungleBackStyle, hellBackStyle
i32   spawnTileX, spawnTileY
f64   worldSurface, rockLayer, time
bool  dayTime
i32   moonPhase
bool  bloodMoon, eclipse
i32   dungeonX, dungeonY
bool  crimson
...   progression flags, skipped via the section offset
```

Chests: `i16 count`, then before version 294 a shared `i16` capacity. Each chest is
`i32 x, i32 y, str name`, then from version 294 its own `i32` slot count, then that many slots of
`i16 stack` followed by `i32 item id, u8 prefix` when the stack is non-zero.

Signs: `i16 count`, then `str text, i32 x, i32 y` each.

## Verification against the shipping game

The claims above were checked against Terraria 1.4.5.7 rather than assumed, using the `probe` and
`verify_sections` examples:

- A probe client completed a real handshake with the shipped `TerrariaServer`, confirming the
  framing, the `Terraria325` string, the 2-byte packet `3`, and the packet `4` field order.
- `WorldData` arrived at 170 bytes for a 10-character world name, matching the 159-byte fixed size
  derived here.
- All 15 tile sections captured from the real server decode with this implementation and
  **re-encode byte-identically**.
- Serving the same `.wld` file from both the real server and this one produces **byte-identical
  section streams** for all 15 sections, trailers included.

The last point also pinned down one behaviour worth recording: chests and signs appear in a
section's trailer in the order the row-major tile walk reaches their anchor tile, so they are
ordered by row and then column — not by their index in the save file.

## World objects

```
19 ToggleDoorState    u8 action, i16 x, i16 y, u8 direction
                      0 open door, 1 close door, 2/3 trapdoor, 4/5 tall gate
31 RequestChestOpen   i16 x, i16 y
32 SyncChestItem      i16 chest, u8 slot, i16 stack, u8 prefix, i16 type
33 SyncPlayerChest    i16 chest, i16 x, i16 y, u8 nameLen, [str name]
155 SyncChestSize     i16 chest, i16 slots
46 RequestSign        i16 x, i16 y
47 SignText           i16 sign, i16 x, i16 y, str text, u8 player, u8 editing
```

Opening a chest is a three-part reply: `155` with the slot count, then one `32` per slot, then
`33`. Chests gained per-chest capacities in 1.4.5, so a client that never receives `155` draws the
wrong grid. `33` with a chest index of -1 means "closed"; the byte before the name is its length
*and* the string that follows carries its own length prefix again — both are required. A name
outside 1..=20 bytes is not sent at all.

The server refuses to open a chest another player already has open, and refuses a `32` from anyone
who does not have that chest open.

## Packet `20`, `AreaTileChange`

A rectangle of tiles pushed as a unit, and the reason a server can support furniture, doors and
trees without reimplementing placement and framing.

```
i16 x, i16 y, u8 width, u8 height, u8 changeType
then, per tile, walked COLUMN by column:
  u8 flags1: 0x01 active, 0x04 has wall, 0x08 has liquid, 0x10 wire1,
             0x20 half brick, 0x40 actuator, 0x80 actuated
  u8 flags2: 0x01 wire2, 0x02 wire3, 0x04 has tile colour, 0x08 has wall colour,
             0x10/0x20/0x40 slope bits (1, 2, 4 — added, not packed), 0x80 wire4
  u8 flags3: 0x01 fullbright block, 0x02 fullbright wall,
             0x04 invisible block, 0x08 invisible wall
  [u8 tile colour] [u8 wall colour]
  if active: u16 type (always two bytes here), and frameX/frameY when frame-important
  if has wall: u16 wall
  if has liquid: u8 amount, u8 type
```

Note the differences from a tile section: three flag bytes rather than four, no run-length, the
tile type always two bytes, the slope spread across three separate bits, and column-major order.

## Item entities

```
21 SyncItem        i16 index, Vector2 position, Vector2 velocity, i16 stack, u8 prefix,
                   u8 flags, i16 type,
                   [bool shimmered, f32 shimmerTime]   when flags bit 2
                   [u8 enemyGrabDelay]                 when flags bit 3
22 ItemOwner       i16 index, u8 owner, 7bit reservation, u8 grabDelayPlayer,
                   7bit grabDelay, Vector2 position
151 SyncItemDespawn i16 index
```

The lifecycle is split across both sides:

- The server spawns an item and broadcasts `21`.
- It reserves the item for a nearby player with `22`. Only the reserving player may act on it.
- That client performs the pickup itself and reports it with `151`, which the server relays.
- A client dropping something from its inventory sends `21` with index **400**, the sentinel asking
  the server to allocate a slot.

Bits 0 and 1 of the flags byte carry spawn-ownership intent from the caller; bits 2 and 3 only
indicate whether the optional trailers are present, so they must be recomputed when re-encoding.
Both shimmer fields are gated on one bit and the game short-circuits, so neither is present when it
is clear.

### Which item a tile drops

`WorldGen.KillTile_GetItemDrops` is 2135 lines and 818 cases. Only the cases whose entire body is a
single constant assignment are transcribed here — 334 tile types, covering dirt, stone, ores, wood,
sand, ice and the other plain blocks. The remaining 75 pick their drop from a frame style, a world
flag or a random roll and are deliberately absent, because handing a player the wrong item is worse
than handing them none.

Two of these are easy to get backwards from the names alone, and are worth stating: Gold is tile 8
and drops item 13, while Silver is tile 9 and drops item 14.

## Passwords

With `Netplay.ServerPassword` set, the server answers `1` Hello with `37` RequestPassword instead
of a slot, and waits for `38` SendPassword carrying the password as a string. A match sends `3` and
the handshake continues; a mismatch sends `2` Kick.

The password exchange happens while the connection is still in its initial state, so a server must
also require that a valid `Hello` was seen first — otherwise a client can skip the version check by
sending only a password.

## NPCs

```
 23 SyncNPC     u8 index, u8 generation, Vector2 position, Vector2 velocity, u16 target,
                u8 flags1, u8 flags2,
                f32 x (one per AI slot flagged non-zero),
                i16 netId,
                [u8 scaledPlayers]  when flags2 bit 0
                [f32 difficulty]    when flags2 bit 2
                [u8 sizeOfHealth, then i8/i16/i32 health]  unless flags1 bit 7
                [u8 releaseOwner]   only for catchable critters
 28 DamageNPC   u8 index, u8 generation, i16 damage, f32 knockback, u8 direction+1, u8 crit
162 DamageAck   (empty) — sent back so the client stops resending the hit
```

`flags1`: bit 0 direction, bit 1 directionY, bits 2–5 which AI slots are non-zero, bit 6 sprite
direction, bit 7 "at full health". `flags2` covers per-player stat scaling, statue spawns,
difficulty and shimmer.

The packing matters: a full-health NPC with no AI state costs 24 bytes, while the worst case is
more than twice that. **Health is omitted entirely at full health** — the receiver substitutes the
type's maximum from its own table, so a decoder has to resolve it rather than leave a sentinel.
The health field is sized against `lifeMax`, not the current value.

`generation` is bumped whenever a slot is reused, and a hit whose generation does not match is
dropped. Without it, a hit sent at an NPC that dies in flight would land on whatever spawns into
that slot next.

Position is offset by `NPCID.Sets.SyncAnchor[type] * size`. Every type anchors at zero except King
Slime, whose sprite grows as it loses health.

Damage taken is `max(1, damage - defense/2)`, doubled on a critical (`Main.CalculateDamageNPCsTake`).

### Data extracted from the build

- **Stats for 691 of 697 NPC types** from `NPC.SetDefaults`, which assigns them in a long
  `if (type == N)` chain. Blocks are shared between types via `type == A || type == B`, which is
  easy to miss: matching only single-type conditions silently drops the Bunny and 100 others.
- **The pre-hardmode roster (201 types)** by walking `Spawner.SpawnAnNPC` with its brace structure
  intact and collecting the conditions guarding each spawn. Excluding only `Main.hardMode` is not
  enough — the Solar Eclipse and the later invasions are themselves hardmode-only, so Mothron
  arrives with no literal hardmode test above it. A static walk also cannot follow the spawn
  helpers, so the staples they choose at runtime are listed explicitly.
- **Solid and platform tile tables** from `Main.tileSolid` / `tileSolidTop`. A platform is in
  *both*: collision has to check `solid_top` rather than treating `solid` alone as "blocks me".
- **Housing rules** from `WorldGen.StartRoomCheck`, `CheckRoom` and `RoomNeeds`: at least 60 open
  tiles, no more than 750 or 100 on a side, every tile sealed within two tiles on both axes, and a
  chair, table, light and door. Natural dirt walls do not seal a room; built walls do.

Physics constants are the game's own: gravity 0.3, terminal fall speed 10.

The coloured slimes — Green, Purple, Jungle and the rest — are **negative net ids**, variants of
Blue Slime rather than types of their own. Packet 23 carries `netId` as a signed short for exactly
this reason, so a decoder must not clamp it to zero when deriving the base type.

## Projectiles (packets 27 and 29)

A projectile is not identified by its slot. Packet `27` opens with a packed `i32` **key**: eight
bits of owner, ten of index, fourteen of generation. The generation is the point of it — slots turn
over constantly, and without it a `KillProjectile` that arrives a moment late would destroy whatever
had since taken the slot. A late packet with a stale generation simply fails to match.

The rest of `27` is bit-packed like the NPC sync. One flag byte says which of `ai[0]`, `ai[1]`,
the banner, the damage, the knockback and the original damage are non-zero, plus a bit meaning "a
second flag byte follows". That second byte exists only to carry one more bit, for `ai[2]`. So a
plain arrow costs five bytes of optional payload and a fully loaded one costs twenty.

The server rejects two things a client might send: a projectile claiming an owner other than the
sender, and any type in `Main.projHostile`. Enemy projectiles are the server's to create.

Packet `29` is the key again plus the position it died at. A non-finite position means "just
remove it" rather than "kill it here", which is how the game distinguishes a despawn from a hit.

## Hurting players (packets 117 and 118)

Both carry a **death reason** rather than a number, because the game names what killed you. The
reason is itself bit-packed: one flag byte for eight possible sources — player, NPC, projectile
index, "other" cause, projectile type, item type, item prefix, custom text — and only the present
ones are written. An NPC's claw is one `i16`; a projectile fired by another player is four fields.

Two details are easy to get wrong. The **direction travels offset by one** in a byte, so `-1`,
`0` and `1` arrive as `0`, `1` and `2`. And `117` carries a **cooldown counter** as a signed byte
after the flags, where `-1` means the ordinary invulnerability window; the game uses the others for
damage that ignores it.

The invulnerability window is not in the protocol at all — it is the server's own bookkeeping.
Without it a player standing inside a zombie takes sixty hits a second rather than two.

## Loot

`ItemDropDatabase` is a tree, not a table. Most of it reduces to one shape: roll a one-in-N chance,
and if it *misses*, roll the next thing instead. A skeleton's four possible weapons are a single
such chain, and a demon eye's black lens sits behind its rarer drop the same way. Flattening those
chains into independent rolls would make several drops far more common than they are, so the chain
structure has to survive into whatever you generate.

Conditional loot is a second layer over that, and three parts of it carry the game's structure
rather than its flavour. A boss in expert drops a **treasure bag** as well as its ordinary loot,
and the bag is where the expert-only accessory lives. Every boss drops a **trophy** one time in ten
— the Twins are the one boss whose halves have different trophies but share a bag. And the hardmode
**crafting materials** are gated three ways at once: on hardmode, on the biome underfoot, and on
depth, so a Soul of Night needs the corruption *and* the rock layer and neither alone will do.

## The world flags of packet `7`

Eleven bytes, sixty-two named flags, and a server that sends zeroes leaves every client believing
it has joined a brand-new world however far along the save is. The order is the order
`NetMessage.SendData` writes them, which is not the order the world file stores them in.

The layout worth recording, because getting one bit wrong quietly closes a shop:

```
byte 0  shadowOrbSmashed, downedBoss1, downedBoss2, downedBoss3,
        hardMode, downedClown, ServerSideCharacter, downedPlantBoss
byte 1  mech1, mech2, mech3, mechAny, cloudBGActive, crimson, pumpkinMoon, snowMoon
byte 2  —, fastForwardToDawn, slimeRain, downedSlimeKing,
        downedQueenBee, downedFishron, downedMartians, downedAncientCultist
byte 3  downedMoonlord, halloweenKing, halloweenTree, xmasIceQueen,
        xmasSantank, xmasTree, downedGolem, partyIsUp
byte 4  pirates, frostLegion, goblins, sandstorm, dd2Ongoing, dd2T1, dd2T2, dd2T3
byte 5  combatBook, lanternNight, towerSolar, towerVortex,
        towerNebula, towerStardust, halloweenToday, xmasToday
byte 6  boughtCat, boughtDog, boughtBunny, freeCake,
        drunkWorld, empressOfLight, queenSlime, getGoodWorld
byte 7  tenthAnniversary, dontStarve, deerclops, notTheBees,
        remixWorld, slimeBlueSpawn, combatBookTwo, —
```

Bit 6 of byte 0 is `ServerSideCharacter`, not a downed flag — the game skips it in the sequence,
which is easy to misread as an unused bit and shift everything after it.

**Shops need nothing else.** `Chest.SetupShop` runs on the client, reads `Main.LocalPlayer`, and
gates on exactly twenty-two of these flags plus `bloodMoon` and `eclipse`. A server that sends the
flag block correctly has implemented shops; one that does not cannot fix them by sending anything
else.

## Tile entities

Furniture that remembers something is kept beside the world rather than in it, and the kinds are
numbered by *registration order* in `TileEntitiesManager.RegisterAll`, not by tile id:

```
0 TrainingDummy   1 ItemFrame    2 LogicSensor   3 DisplayDoll
4 WeaponsRack     5 HatRack      6 FoodPlatter   7 TeleportationPylon
8 DeadCellsJar    9 KiteAnchor  10 CritterAnchor
```

Packet `87` places one: `i16 x, i16 y, u8 kind`. The two anchors ride other tiles and have no home
tile of their own; every other kind must be standing on its own tile or the placement is a lie.

The training dummy is the only one with behaviour: it is a tile entity that puts NPC 488 in front
of itself when a player comes within a hundred tiles and takes it away again when they leave. The
NPC carries the tile position in `ai[0..1]`, which is the whole of how it notices its tile has been
mined out from under it.

## Locked chests

A locked chest is the same tile shifted along its frame strip, so unlocking is arithmetic rather
than a tile swap: subtract 36 from `frameX` for a dungeon or golden chest, 180 for a biome chest,
and 36 for the temple chest (tile 467). All four tiles of the two-by-two carry the frame and all
four must move together.

The biome chests and the temple are gated on Plantera. That gate is the server's to hold: they are
the whole reward for beating her.

## Liquids

`Liquid.Update` falls first and levels sideways afterwards, and the levelling averages across seven
tiles where it can, five where it cannot, three where it cannot manage that, and two as a last
resort. Averaging wide is what makes a pool find its level in a few ticks rather than creeping one
tile at a time.

Two details are worth diverging from deliberately on a server. The game rounds each tile's share
independently, which **creates liquid** every time a pool settles; dividing exactly and handing out
the remainder costs nothing visible and means a world cannot flood itself. And a span already level
to within one unit has to be left alone, or the indivisible remainder is handed back and forth
between neighbours forever and a still pool costs as much as a flowing one.
