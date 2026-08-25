# terrustia

A Terraria **1.4.5.8** server, written from scratch in Rust. Real clients connect to it, real
worlds load in it, and the game that gets played is the game.

## Point it at a world you already have

That is what it does best today, and it does it well:

```sh
terrustia --world ~/Library/Application\ Support/Terraria/Worlds/MyWorld.wld
```

Your world file, your clients, nothing else to install. Measured against the official server on the
same 4200×1200 world with nobody connected:

| | vanilla 1.4.5.8 | terrustia |
|---|---|---|
| Startup | 2.26 s | **0.41 s** |
| CPU, idle | 104% of a core | **0.7%** |
| RAM, idle | 641.8 MB | **45.4 MB** |
| Bandwidth over 5 minutes | 148,874 B | **133,400 B** |

Saves are verified before they replace anything, fsynced, and rotated through three backups. The
header is preserved byte-for-byte and patched, so everything this server does not model — Journey
research, the bestiary, pylon rooms — survives untouched.

## World generation is experimental

Generate a world instead and you get somewhere playable but visibly unfinished. **There are no
trees.** No lakes, no settled water, no smoothed terrain. About 22 of Terraria's 106 generation
passes have a counterpart so far; the rest are being worked through pass by pass.

**[FEATURES.md](FEATURES.md) is the honest, feature-by-feature answer** to what works, what is
partial and why, and what is deliberately out of scope. It is kept in step with the code rather
than written once — read it before deciding whether this suits you.

## Not affiliated with Re-Logic

Terraria is a trademark of Re-Logic. This project is an independent reimplementation of the
dedicated server and is not affiliated with, endorsed by, or supported by Re-Logic. You need a copy
of the game to play; this replaces the server, not the game.

## Status

Implemented:

- **Connection**: full 1.4.5 handshake and client state machine, optional server password
- **World streaming**: `WorldData`, tile sections, and the 1.4.5 pull-based section requests
- **Players**: join/leave, movement, health, mana, appearance, buffs, biome zones, teams, PvP
  toggle, death and respawn
- **Combat**: enemies hurt players on contact and with what they throw, with the game's own
  invulnerability window; players hurt enemies; both directions are the server's decision rather
  than a client's claim
- **Projectiles**: entity, physics and the arcing, lobbing and accelerating routines, with packets
  `27` and `29`. Every shot an NPC decides on flies, and players' own projectiles are relayed
- **Tiles**: break and place blocks and walls, wires, actuators, slopes, half bricks, and
  `TileSquare` rectangles so multi-tile objects work without reimplementing the game's placement
  rules
- **Items**: blocks and furniture both drop the right item when mined — the frame names a style,
  and the style names the item that placed it — drops fall and settle, the server reserves them
  for a nearby player, pickup and player-thrown items both sync
- **Chests**: open, read and edit contents, with the same one-player-at-a-time rule vanilla uses
- **Signs**: read and rewrite
- **NPCs and enemies**: the **whole roster — 691 types across all 128 AI styles, every one of them
  running a routine transcribed from the game rather than an approximation of it**. They spawn by
  biome, depth and time of day, chase, jump, fly, swim, burrow, perch, glide, dig and ambush; they
  take damage, die, drop their coin value and their loot. Worms spawn as linked chains, and cutting
  an Eater of Worlds in half leaves you fighting two of them.
- **Bosses**: every one, as its real phase machine — King Slime, the Eye of Cthulhu, the Eater of
  Worlds, the Brain of Cthulhu, Queen Bee, Skeletron, the Wall of Flesh, Deerclops, the Destroyer,
  the Twins, Skeletron Prime, Plantera, Golem, Duke Fishron, Queen Slime, the Empress of Light, the
  Lunatic Cultist, the four lunar pillars and the Moon Lord
- **Events**: goblin, frost legion, pirate and Martian invasions; the Old One's Army across all
  three tiers with its crystal, lane portals and wave tables; the pumpkin and frost moons with the
  game's own point quotas; the solar eclipse; blood moons; and the Lunar Apocalypse
- **Hardmode**: the Wall of Flesh cuts the two stripes through the world, altars seed the three ore
  tiers, and the biomes creep — held off by sunflowers and halved for good once Plantera is down
- **Weather**: wind and rain on the game's own simulation, and the sandstorms a strong wind raises
  over a desert
- **Liquids**: water, lava and honey fall and level, and react where they meet — obsidian, crispy
  honey, honey blocks
- **Town NPCs**: housing validation to the game's own rules, move-in, and the movement routine —
  the leash around their house, going indoors at nightfall and in rain, opening doors and stopping
  at ledges. Enemies can kill them, and their armour comes off everything the world has beaten, so
  a blood moon that walks through a town is a threat rather than scenery. Shops need nothing from
  the server: a client builds one from the world flags, and those are sent in full
- **Chat**: player chat plus `/help`, `/players`, `/time`, `/save`, `/where`, `/spawn`, `/npcs`,
  `/butcher`, `/house`
- **Worlds**: a reader for `.wld` saves (format 279 and newer) and a writer that saves losslessly,
  with autosave and a save on shutdown. A world loaded from a file keeps its own header byte for
  byte; a generated one gets a header written from scratch at format 325. Progression flags,
  weather and the lunar state survive a save. World *generation* is terrain only — see below
- **Wiring**: circuits run on the server — the flood over whatever is connected, four colours as
  four independent circuits. Actuators toggle the block they sit on; traps throw what they are
  framed to throw, on the game's own cooldowns; statues produce monsters, items or a fetched
  townsperson, under the game's crowding limits; teleporters swap whoever is on each pad; and
  pumps move liquid, refusing to mix water into lava. **Timers** keep a contraption running with
  nobody touching it, and **logic gates** — all six kinds, including the faulty one that rolls a
  die — read their stack of lamps and start a circuit of their own, so a machine built out of
  gates works rather than just lighting up. A timer left running when the world was saved is
  still running when it is served again — a deliberate divergence, since the game keeps that list
  only in memory and a restart would otherwise kill every contraption in the world
- **Tile entities**: placed and remembered, with the training dummy raising and dismissing its NPC
- **Pylon travel**: every pylon is announced as a player joins and as one is planted or mined, and
  a travel request is served — refused unless the traveller is near a pylon of their own and two
  townsfolk live within the destination's scan box, which is the game's rule. The biome
  requirement is not enforced, so a pylon planted in the wrong biome still works here
- **Banners and the bestiary**: kills are counted per banner, survive a restart, and are sent to
  every client, so the counters fill in as they should and the banner drops on the threshold
- **The Dryad's report**: how much of the world is hallow, corruption and crimson, counted one
  column per tick the way the game counts it — surface weighted five times over
- **Day/night clock**

Not implemented:

- **Authoritative inventories.** Every player's inventory is kept and relayed, so a joining player
  sees what everyone is wearing and carrying. It is not *checked*: a client that claims to hold a
  key is believed. Every server-side consequence of an item — a drop, a lock, an event — is
  handled; verifying the claim is not.
- **Seed-identical world generation.** The generator builds a complete, finishable world — biomes,
  a dungeon, the underworld, chests, shadow orbs, demon altars, the jungle temple — verified by
  `cargo run --release -p terrustia --example playable`. What it does *not* do is reproduce
  Terraria's own world for a given seed, which is a far larger job sized in `docs/worldgen-parity.md`.
  So a seed shared with another player will not give you their world.
- **Player weapons.** Projectiles an NPC throws or a trap fires are flown by the server; a
  player's own are simulated by their own client and relayed, which is what a vanilla server does
  too. The server still refuses any a client claims that would hurt other players.

## Running

```sh
cargo run --release -- --world path/to/World.wld     # what you want: serve a real world
cargo run --release -- --listen 127.0.0.1:7777 --world path/to/World.wld
cargo run --release                                  # generate a world; playable, but ephemeral
cargo run --release -- --save world.wld              # generate one and keep it
cargo run --release -- --record capture.trcap        # record every byte, for docs/real-client.md
```

Worlds are saved on shutdown, every `autosave_secs`, and on `/save`. A world loaded from a file
saves back over itself; set `save_file`, or pass `--save`, to write somewhere else — which is also
how a generated world is given somewhere to live.

Then in Terraria: **Multiplayer → Join via IP → `127.0.0.1`**, port `7777`.

Configuration is optional; `terrustia.toml` in the working directory overrides the defaults. See
`terrustia.toml.example`. `TERRUSTIA_LOG=debug` raises the log level.

## Layout

| Crate | What it holds |
|---|---|
| `terrustia-proto` | The wire format, with no I/O: primitives, packets, tile-section coding, and the tile, NPC and housing tables extracted from the game |
| `terrustia-client` | A headless client: handshake, world view, movement, chat, tile and item actions |
| `terrustia` | The async server: world state, game loop, connection handling, `.wld` reading and writing |

The client crate exists to check the server against something other than itself. It speaks the
protocol a real client speaks, so the same code can drive integration tests, probe a real
`TerrariaServer` for comparison, or run as a bot.

The protocol crate is deliberately I/O-free so every packet round-trips in a unit test without a
socket. The server is a single-writer actor: one task owns the world and the player table, so there
are no locks on the hot path and packet ordering is deterministic. Each connection gets a read task
that feeds that actor and a write task draining a bounded queue.

## How this was derived

Terraria 1.4.5 changed the protocol and, at the time of writing, no public documentation covers it
— the community references, and the existing Rust crates, all describe 1.4.4.9 (release 279).
Everything here was instead transcribed from the shipped `TerrariaServer.exe`, decompiled with
`ilspycmd`. `docs/protocol-notes.md` records the findings in our own words; no decompiled game code
is checked into this repository.

Differences from 1.4.4 that matter, and that stale documentation gets wrong:

- The handshake string is `Terraria325`.
- Packet `3` carries a trailing bool after the player slot.
- Tile sections are a bare DEFLATE stream with **no** leading "is compressed" flag byte.
- `WorldData` has eleven world-flag bytes and a trailing extra-spawn-point list.
- The server no longer pushes sections as players move; clients pull them with packet `159`.

## Verification

Beyond the unit and integration tests, two examples check this implementation against the real
game rather than against itself.

`probe` drives a client handshake and dumps the packet sequence, so vanilla and terrustia can be
compared side by side:

```sh
cargo run --release --example probe -- 127.0.0.1:7778   # the real TerrariaServer
cargo run --release --example probe -- 127.0.0.1:7777   # terrustia
```

`diff_sections` compares two captures at the *tile* level rather than the byte level, which is what
distinguishes an encoding bug from the world simply having changed underneath you:

```sh
cargo run --release -p terrustia --example diff_sections -- /tmp/mine /tmp/vanilla
```

`verify_sections` decodes tile-section payloads captured from a real server and re-encodes them,
which is the strongest available check on the hardest part of the format:

```sh
PROBE_DUMP_DIR=/tmp/sections cargo run --release --example probe -- 127.0.0.1:7778
cargo run --release --example verify_sections -- /tmp/sections
```

`verify` plays the game: it joins a running server, spawns things, and checks that enemies move,
that the ones that shoot put projectiles in the air, that standing among them costs health, that
the Eye of Cthulhu runs its phases and summons, and that killing something drops loot. Unit tests
prove each routine in isolation; this proves the whole thing works together over the real protocol.

```sh
cargo run --release --example verify -- 127.0.0.1:7777
```

`stress` fills the world with three rounds of the whole roster and holds it there while the server
reports its own per-phase tick costs, and `crowd` joins a given number of players and walks them
about. Between them they are what a server's cost actually looks like — one measures NPCs, the
other measures players, and several of the per-tick surveys scale with the second:

```sh
TERRUSTIA_LOG=terrustia=debug cargo run --release -- --world World.wld   # then, elsewhere:
cargo run --release -p terrustia --example stress -- 127.0.0.1:7777 60
cargo run --release -p terrustia --example crowd -- 127.0.0.1:7777 24 30
```

`load` fills the world with a crowd of every kind of enemy and reports the traffic, for checking
that the server keeps its tick budget under pressure:

```sh
cargo run --release --example load -- 127.0.0.1:7777 30
```

The `bot` example joins, walks east and reports what it sees, and is meant to be run against both
servers and compared:

```sh
cargo run --release --example bot -- 127.0.0.1:7778   # vanilla
cargo run --release --example bot -- 127.0.0.1:7777   # terrustia
```

Results at the time of writing, against Terraria 1.4.5.8:

- All 15 tile sections captured from the real server decode and **re-encode byte-identically**.
- Serving the *same* `.wld` file from both servers produces **byte-identical section streams** for
  all 15 sections, trailers included.
- The handshake packet sequence and sizes match vanilla's, including `WorldData` at exactly 163
  bytes plus the encoded world name.
- A real server's `WorldData` payload, captured verbatim, decodes and **re-encodes to the identical
  bytes** — every field, in order, at the right width. This is what turned up the two `dungeonX`/
  `dungeonY` shorts release 326 appends, which are in no version of the decompiled source this
  project was written against.
- Both servers serving the *same* world file agree on **45 of 47 packet 7 fields**; the two that
  differ are the clock and the wind, which both servers simulate as they run.
- The same headless client, run against both servers, reports **identical** world data, section
  counts, terrain under spawn, and tiles 420 blocks east — including the sections each server
  streams in response to a walk.
- Re-saving a 2.9 MB world produces a file **byte-identical to the original except the revision
  counter**, which the game increments too.
- A 12 MB world in the **older format 279** — where the chest section states its capacity once for
  the whole file rather than per chest, and the claimable-banner list does not exist yet — loads,
  re-saves and reloads with every tile, chest and sign intact. Its header alignment was checked
  against the file itself rather than against this reader: the four saved ore tiers land on tiles
  7, 167, 9 and 169, which are copper, lead, silver and platinum.
- A world edited through this server, saved, and then handed to the real `TerrariaServer` **loads
  and serves correctly**, with the edits in place.
- A world this server **generated from scratch** — no header copied from anywhere — survives the
  full round trip `our writer → Re-Logic's reader → Re-Logic's writer → our reader`: **46 of 47
  packet 7 fields identical** afterwards (the other is the clock), and all **308 chests with their
  1224 item stacks intact**. The game deleting orphaned chest records on its first save is the
  failure that fix was written for, and this is the check that confirms it against the game.
- A world this server **generated** — with no header to copy from — is written at format 325, and
  every section boundary lands exactly on its own pointer when walked by a decoder written from
  the game's source independently of this reader. It reloads with zero differing tiles out of
  1.26 million, and serving it back runs the whole 691-type bestiary.
- Pointing both servers at the same pristine `.wld` and capturing their spawn streams gives
  **15 of 15 sections byte-identical and zero differing tiles** out of 450,000.
- The `bestiary` example spawns **all 691 NPC types** on a running server over the real protocol
  and confirms every one arrives and syncs: 691 of 691, exercising 126 distinct AI styles.
- The `watch` example joins a running server, stands where it is told, and reports what spawns
  nearby — which is how the hardmode pools were checked against a real world rather than a test
  fixture.
- Every table was diffed against the game's own, mechanically:
  **5,103 NPC stat fields** across 686 types and **5,488 flag fields** on top of them; the 27
  projectile types this server flies; both tile-solidity bitsets and the frame-importance table,
  all 754 entries each; the 345 tile types whose drop the game states as a constant; and all 248
  unconditional drop rules. What that turned up is in the git log — fourteen NPCs recorded as
  one-hit props, six tiles players could walk through, sixteen wrong projectile lifetimes, and a
  loot table missing three quarters of its rules, the Eater of Worlds' ore among them.
- The `crowd` example joins **twenty-four players**, spreads them across the world and walks them
  at the rate a real client reports at. They hold with nothing dropped, and the worst tick over
  the whole run — bestiary, fuzz, crowd and stress against one server — is **520 microseconds
  against a budget of 16,666**, at 50 MB.
- The `fuzz` example throws **fifty thousand malformed packets** at a running server — half noise,
  half structurally plausible traffic naming tiles at the extremes of an `i16` — and checks it is
  still answering afterwards, with the world uncorrupted and nothing in the log.

## Licence

The server and the client are under the **GNU Affero General Public License v3.0 or later**; see
`LICENSE`.

`terrustia-proto` is **MIT**, on purpose — see `crates/terrustia-proto/LICENSE`. It is a
description of Terraria's wire format with no I/O and no game logic in it, and anyone writing a
Terraria tool in Rust should be able to use that without taking on the server's licence.

The generated tables in that crate were produced *from* a decompiled copy of the game by the
scripts in `tools/`, each of which takes the decompiled tree as an argument. No decompiled source,
no game assets and no game text ship in this repository — the one exception being the town-NPC name
pools in `town_names.rs`, whose provenance is documented in `docs/generated-tables.md`.
