# terrustia

An async Terraria server written from scratch in Rust, targeting **Terraria 1.4.5.7**
(protocol release **325**).

A real 1.4.5.7 client connects, the world streams in, players walk around, edit tiles, fight
everything the game has, and chat.

**Serve a world made in Terraria.** The built-in generator makes terrain, caves and ore and
nothing else — no biomes, no dungeon, no underworld, no chests, no altars — so a world it made is
somewhere to stand, not somewhere to play. Everything this server *simulates* needs a world with
those things in it.

Every one of the game's 691 NPC types runs a routine transcribed from the decompiled source rather
than an approximation of it, and a test walks the whole roster to prove each one is reachable.

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
- **Day/night clock**

Not implemented:

- **Authoritative inventories.** Every player's inventory is kept and relayed, so a joining player
  sees what everyone is wearing and carrying. It is not *checked*: a client that claims to hold a
  key is believed. Every server-side consequence of an item — a drop, a lock, an event — is
  handled; verifying the claim is not.
- **World generation.** The generator makes a rolling surface, caves and ore veins. It does not
  make biomes, a dungeon, an underworld, chests, pots, shadow orbs, demon altars, floating islands
  or a jungle temple — so a world it made cannot be *played through*, whatever the server does on
  top of it. Generate a world in Terraria and serve that; everything here is built to run one.
- **Player weapons.** Projectiles an NPC throws or a trap fires are flown by the server; a
  player's own are simulated by their own client and relayed, which is what a vanilla server does
  too. The server still refuses any a client claims that would hurt other players.

## Running

```sh
cargo run --release -- --world path/to/World.wld     # what you want: serve a real world
cargo run --release -- --listen 127.0.0.1:7777 --world path/to/World.wld
cargo run --release                                  # bare terrain, for testing
cargo run --release -- --save world.wld              # generate bare terrain and keep it
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

Results at the time of writing, against Terraria 1.4.5.7:

- All 15 tile sections captured from the real server decode and **re-encode byte-identically**.
- Serving the *same* `.wld` file from both servers produces **byte-identical section streams** for
  all 15 sections, trailers included.
- The handshake packet sequence and sizes match vanilla's, including `WorldData` at exactly 159
  bytes plus the encoded world name.
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

GNU Affero General Public License v3.0 or later. See `LICENSE`.
