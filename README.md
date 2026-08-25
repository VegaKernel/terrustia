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

## Not affiliated with Re-Logic

Terraria is a trademark of Re-Logic. This project is an independent reimplementation of the
dedicated server and is not affiliated with, endorsed by, or supported by Re-Logic. You need a copy
of the game to play; this replaces the server, not the game.

## What works

The honest answer, feature by feature. Built from audits run against the code rather than against
these notes — the two documents this section replaces (a separate `README.md` status list and
`FEATURES.md`) had drifted far enough apart to disagree with each other about whether cacti grow.
This is the one place that answer lives now. **[`AUDIT.md`](AUDIT.md)** has the findings behind it:
what was wrong, what it would have done to a real save or a real server, and how each fix was
verified rather than assumed.

| | Meaning |
|---|---|
| ✅ | Implemented, and checked |
| 🟡 | Works, with the limitation named in the row. "Partial" without a qualifier is a lie by omission |
| 🔴 | Not implemented. A player will notice |
| ⬜ | Deliberately out of scope, with the reason |

**The short version:** point it at a `.wld` you already have and it serves it well — same clients,
same file, and measurably lighter than the official server. Point it at nothing and it generates a
world that is playable, decorated, settled and smoothed — forests, a green jungle, cacti, lakes,
settled water, pots, statues, piles, fallen logs, traps and rounded terrain are all there now.
Tier 1 worldgen is done; what's left is Tier 2/3's biome set pieces (floating islands, spider caves,
gem caves, pyramids and the rest), sized and tracked in `plan.md`.

### Protocol and connectivity

| | Feature | Notes |
|---|---|---|
| ✅ | Protocol release 326 (1.4.5.8) | Release 325 accepted too; they differ in the announced number and four bytes of packet 7 |
| ✅ | Handshake, world data, section streaming | Checked against a real `TerrariaServer`, not only against our own client |
| ✅ | Server password | |
| ✅ | Localized announcements | Keys with substitutions, as the game sends — so a non-English client reads its own language |
| 🟡 | Packet coverage | 109 of 163 message ids handled; most of the rest are outbound-only or dead in vanilla too. Genuinely missing: PvP buff spread (55), portal-gunning an NPC (100), spectating (150), shop overrides (104) |
| ⬜ | Steam P2P / lobbies | Steamworks' licence is incompatible with AGPL |
| ⬜ | Encryption | Terraria's protocol has none. `/login` sends a password as ordinary chat text — do not reuse a real one |

### World files

| | Feature | Notes |
|---|---|---|
| ✅ | Load `.wld` (format 279–325) | Refuses older and newer by name rather than guessing |
| ✅ | Save `.wld` | Header preserved byte-for-byte and patched, so state we do not model survives untouched |
| ✅ | Journey research, bestiary, pylon rooms, pressure plates | Carried through verbatim — verified by section index, not assumed |
| ✅ | Verified before replacing, fsynced, 3 rotating backups | An atomic rename over a corrupt file is an atomic loss |
| 🟡 | Bestiary | Existing data is preserved; kills during a session are not added to it |
| 🟡 | In-progress blood moon / eclipse | Not resumed from the file. The file's own bytes are undisturbed |

### World generation

~37 of Terraria's 106 passes have a counterpart. Tier 1 (the passes that make a world stop looking
like a prototype) is done; Tier 2 and 3 (biome set pieces, and the cosmetic/cleanup tail) are sized
in `plan.md` but not yet started.

| | Feature | Notes |
|---|---|---|
| ✅ | Terrain, caves, ore veins | |
| ✅ | Dungeon, hive, underworld, evil chasms | Our own algorithms, not vanilla's |
| ✅ | Jungle temple, **including the Lihzahrd Altar** | The altar was missing for a while without anyone noticing — a real client refuses to let a player even attempt the Power Cell interaction without one nearby, so its absence made Golem unreachable in every world this generator ever produced. Found by a sizing pass, fixed, tested across 40 seeds |
| ✅ | Demon/crimson altars, life crystals, shadow orbs | Enough to be beatable |
| ✅ | Chests | Depth-tiered everywhere; vanilla's own jungle and underground-desert item lists where the biome matches |
| ✅ | **Trees** | Frames transcribed from `WorldGen.GrowTree` — trunk, branches, roots, canopy |
| ✅ | Vines and jungle grass | The underground jungle's mud is lined with grass, which is what vines hang from |
| ✅ | Cacti | |
| ✅ | Lakes | Sited on level ground with a solid floor |
| ✅ | **Settled water** | Lakes, oceans and underworld lava all reach a stable rest state before the world is handed off — reuses the runtime liquid simulator rather than porting vanilla's separate generation-time algorithm |
| ✅ | Flowers, mushrooms, alchemy herbs, sunflowers | |
| ✅ | Pots, statues, piles, fallen logs | Statue order is load-bearing and transcribed verbatim (73 entries). The piles' ground→style table is transcribed directly from the `Piles` pass's primary loop (`WorldGen.cs:18963-19030`) |
| ✅ | Traps | Dart traps, land mines, boulder traps and geysers, plus the desert's sand trap — transcribed from `placeTrap`/`PlaceSandTrap`/the driving `Traps` pass. Ordinary-world path only; every secret-seed branch is skipped (see the secret-seeds row below). A real 4200×1200 world: 72 dart traps, 10 mines, 4 boulder traps, 1 geyser |
| ✅ | Smoothed terrain (`SmoothWorld`) | Transcribed, with one deliberate reordering: this generator runs smoothing *last*, after every decoration, rather than vanilla's before-decoration placement — see `smooth.rs`'s doc comment for why, and how the altar and other fixtures stay protected either way. 30,107 tiles smoothed on the same 4200×1200 world |
| 🔴 | Floating islands, spider/gem caves, pyramids, living trees, jungle shrines, underground cabins, oasis, micro-biomes, glowing mushroom biome (Tier 2) | Sized for real against restored source in `plan.md` — no longer needs the heavy shape/structure DSL that was assumed there; a ~200-line structure-overlap tracker covers it |
| 🔴 | Secret seeds (Celebrationmk10, Drunk World, Not the Bees, Remix, No Traps, "get fixed boi", Don't Starve) | In scope, not a disclosed exception like Steam P2P — deprioritized behind ordinary-world parity, tracked in `plan.md`'s backlog |
| 🔴 | Moss, wall variety, waterfalls, thin ice, cleanup passes (Tier 3) | Sized in `plan.md`; mostly small single-purpose tile scans |
| ⬜ | Seed-identical worlds | Sized at 219–372 engineer-days. Feature-complete is the goal; identical is not |

### NPCs, bosses and events

| | Feature | Notes |
|---|---|---|
| ✅ | All 691 NPC types | Each runs a transcribed routine; a test walks the whole roster, across 126 distinct AI styles |
| ✅ | All 20 bosses | Multi-phase, including Moon Lord's opening sequence and the cultist's tablet ritual |
| ✅ | Both moons, all four invasions, Old One's Army with Betsy, eclipse with Mothron | |
| ✅ | Rain, wind, sandstorms | |
| ✅ | Town NPC arrival and housing | Including the in-game housing screen |
| ✅ | Town NPC shops | Opening and using a shop is entirely client-side in vanilla — no packet populates it, and the click gate (`townNPC`, derived from `type` alone, plus `velocity.Y == 0`) is satisfied by an ordinary NPC sync. The one thing the server owns, packet 40, was already correct; a test proves it, relayed to other players. No happiness-driven pricing or shop overrides yet — tracked separately below and as packet 104 |
| 🟡 | **Town NPCs fighting back** | Four representative types across all of vanilla's attack classes — Merchant (ranged), Arms Dealer (ranged), Wizard (ranged), Dye Trader (melee) — target and damage nearby hostiles, verified end to end over a real socket including the shot actually landing. The other ~23 vanilla combat-capable town NPCs are mechanical to add from here but not yet done, so a town with only those types still stands still |
| 🔴 | NPC happiness, price effects, moving out | |
| 🔴 | Slime Rain, Party, Lantern Night | |
| 🟡 | Enemy drops | `tools/check_drops.py` had real parsing bugs producing false positives; fixed, and boss loot is now genuinely complete — every trophy, every unique weapon pool. ~111 ordinary enemies are still short at least one drop |

### Items and mechanics

| | Feature | Notes |
|---|---|---|
| ✅ | Buffs and debuffs, item entities, shimmer | |
| ✅ | Wiring, logic gates, timers, teleporters | |
| ✅ | Chests, signs, tile entities, pylons | |
| ✅ | Boss summon items, Angler quests, fishing NPCs | |
| ⬜ | Crafting validation | Terraria has no craft packet; the client is authoritative in vanilla too |
| ⬜ | Armour set bonuses, accessories | Client-side in vanilla; the server applies plain defence |
| 🔴 | Pets, mounts, minecart tracks | No server-side existence |

### Journey mode

| | Feature | Notes |
|---|---|---|
| 🔴 | **Every Journey power** | Net module 4 is not defined, so godmode, time and weather freeze, the sliders, research and duplication are all silently swallowed |
| ✅ | Existing research survives a save | Preserved verbatim; it simply cannot grow here |

### Multiplayer

| | Feature | Notes |
|---|---|---|
| ✅ | PvP, teams, deaths, respawn, chat | |
| ✅ | Accounts, groups, permissions, bans by name/address/uuid | Argon2, off the game task |
| 🟡 | Chat commands | 17 of them. No warps, regions, or item bans — that is the deferred TShock-shaped work |
| ✅ | Whitelist | Empty means off, so it cannot lock the operator out on the day it is enabled |
| 🔴 | Web admin panel | In progress — see `plan.md` |

### Hardening

| | Feature | Notes |
|---|---|---|
| ✅ | Decode path cannot panic or over-allocate | One `unsafe` block in the workspace, for the CPU clock |
| ✅ | A panic on the packet path saves the world and exits non-zero | So `Restart=on-failure` fires |
| ✅ | Connection ceiling, per-address cap, handshake deadline | |
| ✅ | Tile-edit spam limiter | Vanilla's own six numbers, transcribed from `RemoteClient` |
| ✅ | Server claim requires a console token | |
| ⬜ | Server-authoritative inventory and damage | Vanilla trusts the client for both; diverging would change how the game plays |

### Platforms

| | | Notes |
|---|---|---|
| ✅ | Linux x86_64 / aarch64, macOS arm64 / x86_64, Windows x86_64 | All five pass `cargo check` |
| 🟡 | Container image, signed releases, packaging | The container workflow has actually run: multi-arch image built, pushed, cosign-signed, and smoke-tested serving with no configuration. Getting there for real found and fixed real CI bugs invisible to local `cargo check` alone — see `AUDIT.md`. Signed releases still untested; that workflow only triggers on a `v*` tag |

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

Beyond the unit and integration tests, several examples check this implementation against the real
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
the Eye of Cthulhu runs its phases and summons, and that killing something drops loot.

```sh
cargo run --release --example verify -- 127.0.0.1:7777
```

`stress` fills the world with three rounds of the whole roster and holds it there while the server
reports its own per-phase tick costs, and `crowd` joins a given number of players and walks them
about:

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
  bytes** — every field, in order, at the right width.
- Both servers serving the *same* world file agree on **45 of 47 packet 7 fields**; the two that
  differ are the clock and the wind, which both servers simulate as they run.
- Re-saving a 2.9 MB world produces a file **byte-identical to the original except the revision
  counter**, which the game increments too.
- A world edited through this server, saved, and then handed to the real `TerrariaServer` **loads
  and serves correctly**, with the edits in place.
- A world this server **generated from scratch** survives the full round trip `our writer →
  Re-Logic's reader → Re-Logic's writer → our reader`: **46 of 47 packet 7 fields identical**
  afterwards, and all **308 chests with their 1224 item stacks intact**.
- The `bestiary` example spawns **all 691 NPC types** on a running server over the real protocol and
  confirms every one arrives and syncs.
- Every table was diffed against the game's own, mechanically: **5,103 NPC stat fields** across 686
  types and **5,488 flag fields** on top of them; the 27 projectile types this server flies; both
  tile-solidity bitsets and the frame-importance table, all 754 entries each; the 345 tile types
  whose drop the game states as a constant; and every unconditional drop rule.
- The `crowd` example joins **twenty-four players**, spreads them across the world and walks them at
  the rate a real client reports at. The worst tick over the whole run — bestiary, fuzz, crowd and
  stress against one server — is **520 microseconds against a budget of 16,666**, at 50 MB.
- The `fuzz` example throws **fifty thousand malformed packets** at a running server and checks it
  is still answering afterwards, with the world uncorrupted and nothing in the log.

### Measured against the official server

Same 4200×1200 world, same machine, nobody connected.

| | vanilla 1.4.5.8 | terrustia |
|---|---|---|
| Startup | 2.26 s | 0.41 s |
| CPU, idle | 104% of a core | 0.7% |
| RAM, idle | 641.8 MB | 45.4 MB |
| Bandwidth over 5 min | 148,874 B | 133,400 B |

A note on the tick, because the claim has changed three times now. "A verified full 60 Hz tick"
originally rested on instrumentation that was comparing CPU time against wall time, so it was
withdrawn. With that fixed, it was measured again on the same world: a typical tick costs
**184–330 µs of a 16,666 µs budget**, and the autosave — previously the most expensive thing an
idle server did, at up to 7,656 µs — now costs 43–137 µs because only changed sections are copied.
That third figure had a real gap the other two didn't: it was measured from a server's *second*
autosave onward. The first one, with no prior buffer to diff against, cost 14,833 µs — 89% of the
whole budget — until this repository's own first real CI soak run caught it and it was fixed by
building that buffer during startup instead of inside a counted tick.

The remaining figures above were measured externally, at the process level, and were never affected
by that instrumentation.

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
