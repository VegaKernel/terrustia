<div align="center">

<img src="docs/assets/banner.svg" alt="terrustia" width="720">

<br>

[![CI](https://github.com/bybrooklyn/terrustia/actions/workflows/ci.yml/badge.svg)](https://github.com/bybrooklyn/terrustia/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-00afff?style=flat-square)](LICENSE)
[![proto crate: MIT](https://img.shields.io/badge/proto%20crate-MIT-00d7ff?style=flat-square)](crates/terrustia-proto/LICENSE)
![Built with Rust](https://img.shields.io/badge/built%20with-Rust-005fff?style=flat-square&logo=rust&logoColor=white)
![Terraria 1.4.5.8](https://img.shields.io/badge/Terraria-1.4.5.8-00afff?style=flat-square)
![protocol 326](https://img.shields.io/badge/protocol-326-0087ff?style=flat-square)
![platforms](https://img.shields.io/badge/platforms-Linux%20·%20macOS%20·%20Windows-0087ff?style=flat-square)

**A Terraria 1.4.5.8 server, written from scratch in Rust.**
Real clients connect to it, real worlds load in it, and the game that gets played is the game.

[Quickstart](#point-it-at-a-world-you-already-have) ·
[Running](#running) ·
[Hosting](#hosting-and-setup) ·
[How it fits together](#how-it-fits-together) ·
[What works](#what-works) ·
[What the audits found](#what-the-audits-found) ·
[Verification](#verification)

</div>

---

> [!NOTE]
> Terraria is a trademark of Re-Logic. This is an independent reimplementation of the **dedicated
> server**, not affiliated with, endorsed by, or supported by Re-Logic. You need a copy of the game
> to play — this replaces the server, not the game.

## Point it at a world you already have

That is what it does best today, and it does it well:

```sh
terrustia --world ~/Library/Application\ Support/Terraria/Worlds/MyWorld.wld
```

Your world file, your clients, nothing else to install. Measured against the official server on the
same 4200×1200 world with nobody connected:

<div align="center">

| | vanilla 1.4.5.8 | terrustia |
|---|:---:|:---:|
| Startup | 2.26 s | **0.41 s** |
| CPU, idle | 104% of a core | **0.7%** |
| RAM, idle | 641.8 MB | **45.4 MB** |
| Bandwidth over 5 minutes | 148,874 B | **133,400 B** |

</div>

Saves are verified before they replace anything, fsynced, and rotated through three backups. The
header is preserved byte-for-byte and patched, so everything this server does not model — Journey
research, the bestiary, pylon rooms — survives untouched.

**The short version:** point it at a `.wld` you already have and it serves it well — same clients,
same file, and measurably lighter than the official server. Point it at nothing and it generates a
world that is playable, decorated, settled and smoothed: forests, a green jungle, cacti, lakes,
settled water, pots, statues, piles, fallen logs, traps and rounded terrain, floating islands,
spider and gem caves, pyramids, living trees, jungle shrines, underground cabins, an oasis, the
glowing mushroom biome, and the full cosmetic/cleanup tail. Vanilla's seven secret seeds — plus two
more real ones an earlier pass had missed — are detected by their real magic strings (`--seed
"getfixedboi"` works and persists), and one of the nine, No Traps World, is fully wired. What's left
of worldgen is 7 of 15 micro-biome classes and the other eight seeds' own generation-content
differences, sized in [`plan.md`](plan.md) and deferred to v0.1.0.

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

## Hosting and setup

No single "the" easy path — Docker, a native binary behind systemd, and OS packages (Homebrew,
winget, AUR) are all documented and supported equally; see [Packaging](#packaging) below.

- **`terrustia --setup`** runs a short interactive wizard (dedicated config directory, world name,
  max players, whether to turn the web panel on) and starts the server with what it wrote. It also
  runs automatically on a first, zero-flag launch when the working directory is the same directory
  the executable itself is in and nothing terrustia-shaped is there yet — the shape a raw binary
  double-clicked right out of `~/Downloads` actually has. Plain `terrustia` with no flags, run from
  anywhere else, is unchanged: still the original non-interactive "generate a world and serve it."
  The wizard's dedicated directory is refused outright if anything is already in it, and the world
  itself is generated into the platform's own Terraria world directory (the same place `--new`
  writes to) — never beside the executable, so double-clicking a raw binary can never scatter a
  world file and a config into wherever it happened to land.
- **Environment variables** configure everything a `terrustia.toml` can, for Docker/automation use
  where there is often no shell around the process to pass a flag and no volume mount to put a file
  on: `TERRUSTIA_LISTEN`, `TERRUSTIA_MAX_PLAYERS`, `TERRUSTIA_WORLD_NAME`, `TERRUSTIA_PANEL_ENABLED`,
  and so on — every key in `terrustia.toml.example` has a `TERRUSTIA_<UPPERCASE_KEY>` equivalent.
  Precedence is defaults < `terrustia.toml` < environment < an explicit CLI flag.
- **UPnP**: on startup, terrustia asks the router to forward the game port automatically. When no
  UPnP-capable router answers, or it refuses, this logs a specific fallback message naming the port
  and the local address to forward it to by hand — never a fatal error. Set `upnp_enabled = false`
  to turn it off. This has nothing to do with the web panel, which stays bound to loopback
  regardless.
- **`terrustia update`**: on boot, terrustia checks GitHub for a newer, signature-verified release
  and says so — console log, plus an in-game notice to the first recognised admin who signs in
  afterward. Applying it is a separate, deliberate step: run `terrustia update` yourself. It shells
  out to the real `cosign` binary, checking the same keyless GitHub Actions signing chain
  `release.yml` signs with — no separate trust root.

### Packaging

| Target | Where | Status |
|---|---|---|
| Homebrew | `packaging/homebrew/terrustia.rb` | Builds from source with `cargo install`. `brew style` clean; verified with a real `cargo install --path` build. `brew audit`/`--build-from-source` could not run in this environment (its Xcode CLT is below Homebrew's required minimum) |
| systemd | `packaging/terrustia.service` | Verified by running the unit's literal `ExecStart` and sending it the exact `SIGTERM` its `KillSignal` names — which is how a real shutdown deadlock was found and fixed (see [What the audits found](#what-the-audits-found)) |
| Docker | `Dockerfile`, `.github/workflows/docker.yml` | Multi-arch image, cosign-signed, smoke-tested serving with no config. `HEALTHCHECK` present |
| winget | `packaging/winget/manifests/…` | Validated against the current (1.12.0) `microsoft/winget-cli` schemas — all three manifests pass structurally. Publishing needs a PR into `microsoft/winget-pkgs` |
| AUR | `packaging/aur/PKGBUILD` | Builds from source; matches Arch's Rust packaging guidelines; `shellcheck`-clean. `makepkg`/`namcap` need a real Arch environment. Publishing needs an AUR account |

Every `url`/checksum above that points at a `v0.0.1` release asset is a disclosed placeholder — that
tag does not exist yet (see [`plan.md`](plan.md)).

---

## How it fits together

Three crates. `terrustia-proto` is the wire format with **no I/O** — primitives, packets,
tile-section coding, and the tile/NPC/housing tables extracted from the game — so every packet
round-trips in a unit test without a socket. `terrustia-client` is a headless client that speaks the
protocol a real client speaks, so the same code drives integration tests, probes a real
`TerrariaServer` for comparison, or runs as a bot. `terrustia` is the async server.

The server is a **single-writer actor**: one task owns the world and the player table, so there are
no locks on the hot path and packet ordering is deterministic. Each connection gets a read task that
feeds that actor and a write task draining a bounded queue. The web panel, when it is on, taps the
same event stream the terminal prints — no second, parallel path.

```mermaid
flowchart TB
    C(["Terraria clients<br/>up to 255"]) -->|"packets in"| R
    W -->|"packets out"| C
    subgraph proc["terrustia — one process"]
      direction TB
      R["read task"] -->|"events"| A
      A -->|"broadcasts"| W["write task<br/>bounded queue"]
      A["single-writer GAME ACTOR<br/>owns world + player table<br/>no locks on the hot path"]
      G["worldgen"] --> A
      A --> WLD[("world state")]
      WLD -->|"verify → fsync → 3 backups"| SAVE[".wld on disk"]
      A -. "same event stream" .-> P["web panel<br/>loopback · off by default"]
    end
```

### How this was derived

Terraria 1.4.5 changed the protocol and, at the time of writing, no public documentation covers it —
the community references, and the existing Rust crates, all describe 1.4.4.9 (release 279).
Everything here was transcribed from the shipped `TerrariaServer.exe`, decompiled with `ilspycmd`.
`docs/protocol-notes.md` records the findings in our own words; **no decompiled game code is checked
into this repository.**

Differences from 1.4.4 that matter, and that stale documentation gets wrong:

- The handshake string is `Terraria325`.
- Packet `3` carries a trailing bool after the player slot.
- Tile sections are a bare DEFLATE stream with **no** leading "is compressed" flag byte.
- `WorldData` has eleven world-flag bytes and a trailing extra-spawn-point list.
- The server no longer pushes sections as players move; clients pull them with packet `159`.

## What works

The honest answer, feature by feature — built from audits run against the code, not against these
notes. [`AUDIT.md`](AUDIT.md) has the findings behind it; [What the audits found](#what-the-audits-found)
below tells the stories worth telling.

<div align="center">

| | Meaning |
|:---:|---|
| ✅ | Implemented, and checked |
| 🟡 | Works, with the limitation named in the row. "Partial" without a qualifier is a lie by omission |
| 🔴 | Not implemented. A player will notice |
| ⬜ | Deliberately out of scope, with the reason |

</div>

<details>
<summary><b>Protocol and connectivity</b></summary>

| | Feature | Notes |
|---|---|---|
| ✅ | Protocol release 326 (1.4.5.8) | Release 325 accepted too; they differ in the announced number and four bytes of packet 7 |
| ✅ | Handshake, world data, section streaming | Checked against a real `TerrariaServer`, not only against our own client |
| ✅ | Server password | |
| ✅ | Localized announcements | Keys with substitutions, as the game sends — so a non-English client reads its own language |
| 🟡 | Packet coverage | 111 of 163 message ids handled; most of the rest are outbound-only or dead in vanilla too. Genuinely missing: portal-gunning an NPC (100), spectating (150), shop overrides (104) |
| ⬜ | Steam P2P / lobbies | Steamworks' licence is incompatible with AGPL |
| ⬜ | Encryption | Terraria's protocol has none. `/login` sends a password as ordinary chat text — do not reuse a real one |

</details>

<details>
<summary><b>World files</b></summary>

| | Feature | Notes |
|---|---|---|
| ✅ | Load `.wld` (format 279–326) | Refuses older and newer by name rather than guessing |
| ✅ | Save `.wld` | Header preserved byte-for-byte and patched, so state we do not model survives untouched |
| ✅ | Journey research, bestiary, pylon rooms, pressure plates | Carried through verbatim — verified by section index, not assumed |
| ✅ | Verified before replacing, fsynced, 3 rotating backups | An atomic rename over a corrupt file is an atomic loss |
| ✅ | `--new <name>` | Generates a fresh world straight into the platform's own Terraria world directory (creating it first if nothing has ever saved there), under vanilla's own space→underscore filename convention. Refuses rather than overwrites if that name is taken |
| 🟡 | Bestiary | Existing data is preserved; kills during a session are not added to it |
| 🟡 | In-progress blood moon / eclipse | Not resumed from the file. The file's own bytes are undisturbed |

</details>

<details>
<summary><b>World generation</b></summary>

Tier 1 (the passes that make a world stop looking like a prototype), Tier 2 (biome set pieces) and
Tier 3 (the cosmetic/cleanup tail) are all done. What's left: 7 of 15 real `MicroBiome` classes, and
eight of the nine known secret seeds' own generation-content differences (the ninth, No Traps World,
is done). Both are v0.1.0 scope.

| | Feature | Notes |
|---|---|---|
| ✅ | Terrain, caves, ore veins | |
| ✅ | Dungeon, hive, underworld, evil chasms | Our own algorithms, not vanilla's |
| ✅ | Jungle temple, **including the Lihzahrd Altar** | Its absence once made Golem unreachable in every generated world — see [What the audits found](#what-the-audits-found) |
| ✅ | Demon/crimson altars, life crystals, shadow orbs | Enough to be beatable |
| ✅ | Chests | Depth-tiered everywhere; vanilla's own jungle and underground-desert item lists where the biome matches |
| ✅ | **Trees** | Frames transcribed from `WorldGen.GrowTree` — trunk, branches, roots, canopy |
| ✅ | Vines and jungle grass | The underground jungle's mud is lined with grass, which is what vines hang from |
| ✅ | Cacti | |
| ✅ | Lakes | Sited on level ground with a solid floor |
| ✅ | **Settled water** | Lakes, oceans and underworld lava all reach a stable rest state before hand-off — reuses the runtime liquid simulator rather than porting vanilla's generation-time algorithm |
| ✅ | Flowers, mushrooms, alchemy herbs, sunflowers | |
| ✅ | Pots, statues, piles, fallen logs | Statue order is load-bearing and transcribed verbatim (73 entries); the piles' ground→style table straight from `WorldGen.cs:18963-19030` |
| ✅ | Traps | Dart traps, land mines, boulder traps, geysers, plus the desert's sand trap — transcribed from `placeTrap`/`PlaceSandTrap`. A real 4200×1200 world: 72 dart traps, 10 mines, 4 boulder traps, 1 geyser |
| ✅ | Smoothed terrain (`SmoothWorld`) | Transcribed, with one deliberate reordering — this generator smooths *last*, after decoration; see `smooth.rs`. 30,107 tiles smoothed on the same world |
| ✅ | Floating islands, spider/gem caves, pyramids, living trees, jungle shrines, underground cabins, oasis, glowing mushroom biome (Tier 2) | A ~200-line structure-overlap tracker (`StructureMap`) turned out to be enough for all nine — no port of vanilla's shape/structure DSL needed |
| 🟡 | Micro-biomes | 8 of 15 real `MicroBiome` classes done. The other 7 each need a genuinely separate subsystem this project doesn't have yet (a trappable-chest mechanism, a second tree-growth engine, a wandering-tunnel shape…); sized individually in `plan.md` |
| ✅ | Moss, wall variety, waterfalls, thin ice, speleothems, exposed gems, lily pads/coral/cacti, the seven-pass tile-cleanup bundle (Tier 3) | All 8 sizing-table items landed, each with its own disclosed narrowing — see `plan.md`'s Done rows |
| 🟡 | Secret seeds (Celebrationmk10, Drunk World, Not the Bees, Remix, No Traps, "get fixed boi", Don't Starve, For the Worthy, Skyblock) | All nine detected by their **real** magic strings (an earlier pass had six of seven wrong — Remix's real trigger is `dontdigup`, Drunk World has only the numeric 5162020, and so on), fixed against source, plus two more the original investigation never named. All nine **persist** through save/reload and reach a client's packet 7. **No Traps World is fully wired** (0 trap tiles vs 397 on an ordinary seed). The other eight seeds' generation-content differences are detected and persisted but not yet implemented — sized in `plan.md`, deferred to v0.1.0 |
| ⬜ | Seed-identical worlds | Sized at 219–372 engineer-days. Feature-complete is the goal; identical is not |

</details>

<details>
<summary><b>NPCs, bosses and events</b></summary>

| | Feature | Notes |
|---|---|---|
| ✅ | All 691 NPC types | Each runs a transcribed routine; a test walks the whole roster, across 126 distinct AI styles |
| ✅ | All 20 bosses | Multi-phase, including Moon Lord's opening sequence and the cultist's tablet ritual |
| ✅ | Both moons, all four invasions, Old One's Army with Betsy, eclipse with Mothron | |
| ✅ | Rain, wind, sandstorms | |
| ✅ | Town NPC arrival and housing | Including the in-game housing screen |
| ✅ | Town NPC shops | Opening and using a shop is entirely client-side in vanilla; the one thing the server owns (packet 40) was already correct, and a test proves it, relayed to other players |
| ✅ | **Town NPCs fighting back** | All 28 real vanilla combat-capable town NPCs, across every attack class — target and damage nearby hostiles, verified end to end over a real socket including the shot landing |
| ✅ | NPC happiness, price effects, moving out | Moving out is the housing screen's "kick out" (packet 60); happiness/pricing is player-local computation in vanilla too, and terrustia sends everything its one call site reads. Verified by source-tracing, not by a real client's shop UI (nothing here can launch one) |
| ✅ | The birthday party | Natural daily rolls, the Party Monolith (click or wire), and the every-party-ends-at-nightfall rule, for genuine and forced parties |
| ✅ | Slime Rain | The gated daily roll, the ~7-second warning countdown vanilla actually uses, and King Slime arriving once enough Blue Slimes die |
| ✅ | Lantern Night | The gated daily roll, and the guarantee that the next roll succeeds the first time any of 17 tracked boss-kill/hardmode flags is cleared |
| 🟡 | Enemy drops | Boss loot has zero remaining unjustified gaps. 6 ordinary-enemy gaps remain, each traced against source: 5 are `RemixSeed`-only branches out of scope, one a pre-existing nested-fallback-chain shape. The checker bugs behind the rest are in [What the audits found](#what-the-audits-found) |

</details>

<details>
<summary><b>Items, mechanics, Journey mode, multiplayer, hardening, platforms</b></summary>

**Items and mechanics**

| | Feature | Notes |
|---|---|---|
| ✅ | Buffs and debuffs, item entities, shimmer | |
| ✅ | Wiring, logic gates, timers, teleporters | |
| ✅ | Chests, signs, tile entities, pylons | |
| ✅ | Boss summon items, Angler quests, fishing NPCs | |
| ✅ | Pets, mounts, minecart tracks | Pets/mounts are client-authoritative in vanilla too — a gap in this file's own tracking, not the server. A minecart track switch's flip did nothing at all; fixed |
| ⬜ | Crafting validation | Terraria has no craft packet; the client is authoritative in vanilla too |
| ⬜ | Armour set bonuses, accessories | Client-side in vanilla; the server applies plain defence |

**Journey mode**

| | Feature | Notes |
|---|---|---|
| ✅ | **Every Journey power** | All 15 real vanilla powers across all 5 wire shapes — four time-skip buttons, four toggles, four sliders (including `Difficulty`, a continuous 0–3 game-mode replacement read at dozens of call sites), and three per-player powers — bit-packed across up to 255 players, with a real anti-cheat property (a client can't toggle another player's slot) pinned by a two-client test |
| ✅ | Existing research survives a save | Preserved verbatim; it simply cannot grow here |

**Multiplayer**

| | Feature | Notes |
|---|---|---|
| ✅ | PvP, teams, deaths, respawn, chat | |
| ✅ | Accounts, groups, permissions, bans by name/address/uuid | Argon2, off the game task |
| 🟡 | Chat commands | 18 of them. No warps, regions, or item bans — the deferred TShock-shaped work |
| ✅ | Whitelist | Empty means off, so it cannot lock the operator out on the day it is enabled |
| ✅ | Web admin panel | A full subsystem embedded in the binary, off by default: player list with kick/ban, whitelist, world switching (a real graceful restart), a live console/chat stream, a metrics dashboard, backups/rollback, groups/accounts admin, world creation, and a stylized live world view — player avatars coloured from their own real skin/hair/gear over the wire, no game assets shipped or read. Always localhost-only |

**Hardening**

| | Feature | Notes |
|---|---|---|
| ✅ | Decode path cannot panic or over-allocate | One `unsafe` block in the workspace, for the CPU clock |
| ✅ | A panic on the packet path saves the world and exits non-zero | So `Restart=on-failure` fires |
| ✅ | A real `SIGTERM` stops the server with a shutdown save — even with the web panel running | Two related bugs found by hand; see [What the audits found](#what-the-audits-found) |
| ✅ | Connection ceiling, per-address cap, handshake deadline | |
| ✅ | Tile-edit spam limiter | Vanilla's own six numbers, transcribed from `RemoteClient` |
| ✅ | Server claim requires a console token | |
| ✅ | `/world undo <player> <duration>` | Admin-only grief recovery, up to 72h back. In-memory and time-windowed on purpose; disclosed in `tile_log.rs` |
| ✅ | `terrustia update`: check-and-notify on boot, signature-verified, manual apply | Shells out to real `cosign` against the same keyless signing chain `release.yml` signs with |
| ⬜ | Server-authoritative inventory and damage | Vanilla trusts the client for both; diverging would change how the game plays |

**Platforms**

| | | Notes |
|---|---|---|
| ✅ | Linux x86_64/aarch64, macOS arm64/x86_64, Windows x86_64 | All five pass `cargo check` |
| 🟡 | Container image, signed releases, packaging | The container workflow has run for real: multi-arch image built, pushed, cosign-signed, smoke-tested. Signed releases still untested — that workflow only triggers on a `v*` tag |

</details>

## What the audits found

This project's own scope is "transcribe vanilla, and be honest about the gaps." Several of the fixes
worth the most were things that looked fine until an audit ran against the actual code. The full
trail is in [`AUDIT.md`](AUDIT.md); these are the ones worth telling.

- **The missing Lihzahrd Altar made Golem unreachable in every world this generator ever produced.**
  A real client refuses to let a player even attempt the Power Cell interaction without an altar
  nearby — so its absence was invisible until a sizing pass caught it. Fixed, tested across 40 seeds.
- **`SIGTERM` never stopped the server when the panel was on.** Found by running
  `packaging/terrustia.service`'s literal `ExecStart` and sending the exact `SIGTERM` its
  `KillSignal` names. Two bugs, nested: the panel supervisor's clone of the shutdown channel
  outlived a plain `.abort()`, and once that was fixed, the inner axum task it had spawned survived
  the same `.abort()` too. Both closed — the second with an abort-on-drop guard — so the graceful
  shutdown save can't be silently skipped. `TimeoutStopSec=90` would eventually have masked it with
  a hard kill, but only *after* the graceful path had already failed.
- **The first autosave cost 89% of the frame budget.** Instrumentation once claimed a verified full
  60 Hz tick; it was comparing CPU time to wall time, so it was withdrawn and re-measured. A typical
  tick costs **184–330 µs of a 16,666 µs budget**, and the autosave — once the most expensive thing
  an idle server did — now costs 43–137 µs because only changed sections are copied. But that was
  from a server's *second* autosave on. The first, with no prior buffer to diff against, cost
  **14,833 µs** until this repo's own first CI soak run caught it; fixed by building the buffer
  during startup instead of inside a counted tick.
- **88 of 255 connections dropped on a synchronized join burst.** Disclosed here first: at the real
  protocol maximum of 255 players all joining at once, the outbound queue overflowed and 88
  connections were dropped. Fixed and re-verified to **zero dropped connections**, with a regression
  test (`tests/queue_capacity.rs`) confirmed failing against the unfixed queue size first.
- **The drop-table checkers had bugs of their own.** `tools/check_drops.py` and `tools/gen_drops.py`
  carried parsing false positives, a variable-name collision that silently misattributed whole
  registration blocks, and an over-broad exclusion that discarded a chain's genuinely-flat prefix.
  Fixing the checkers recovered the great majority of the ordinary-enemy drop gaps outright.
- **Building the packaging for real found CI bugs invisible to local `cargo check`.** The
  multi-arch container workflow only passed after several real fixes that a local build never
  exercised — see [`AUDIT.md`](AUDIT.md).

## Verification

Beyond the unit and integration tests, several `terrustia-client` examples check this implementation
against the **real game** rather than against itself — `probe` (dump and compare the packet
sequence), `diff_sections`/`verify_sections` (compare captures at the tile level and re-encode real
payloads), `verify` (join, spawn things, and confirm enemies move, shoot, hurt, and drop loot),
`stress`/`crowd`/`load` (hold the world full while the server reports its own per-phase tick costs),
and `bot` (join, walk east, report — run against both servers and compare).

```sh
cargo run --release --example probe -- 127.0.0.1:7778          # the real TerrariaServer
cargo run --release --example probe -- 127.0.0.1:7777          # terrustia
cargo run --release --example verify -- 127.0.0.1:7777
cargo run --release -p terrustia --example stress -- 127.0.0.1:7777 60
```

Results at the time of writing, against Terraria 1.4.5.8:

- All 15 tile sections captured from the real server decode and **re-encode byte-identically**.
- Serving the *same* `.wld` from both servers produces **byte-identical section streams** for all 15
  sections, trailers included.
- The handshake packet sequence and sizes match vanilla's, `WorldData` at exactly 163 bytes plus the
  encoded world name; a real `WorldData` payload decodes and **re-encodes to identical bytes**.
- Both servers serving the same world agree on **45 of 47 packet 7 fields** — the two that differ are
  the clock and the wind, which both servers simulate as they run.
- Re-saving a 2.9 MB world produces a file **byte-identical except the revision counter**.
- A world edited through this server, saved, then handed to the real `TerrariaServer` **loads and
  serves correctly**, edits in place. A world this server *generated* survives the full round trip
  `our writer → Re-Logic's reader → Re-Logic's writer → our reader`: **46 of 47 packet 7 fields
  identical**, all **308 chests with their 1224 item stacks intact**.
- The `bestiary` example spawns **all 691 NPC types** over the real protocol and confirms every one
  arrives and syncs.
- Every table was diffed against the game's own, mechanically: **5,103 NPC stat fields** across 686
  types and **5,488 flag fields**; the projectile types this server flies; both tile-solidity
  bitsets and the frame-importance table, all 754 entries each; the 345 constant-drop tile types;
  and every unconditional drop rule.
- **255 players — the real protocol maximum** — on vanilla's own "Large" preset (8400×2400, 4× the
  tile count): the worst tick costs **3,451 µs of the 16,666 budget**, better than 4.8× headroom,
  with **zero dropped connections** (see [What the audits found](#what-the-audits-found)).
- The `fuzz` example throws **fifty thousand malformed packets** at a running server and confirms it
  is still answering, world uncorrupted, log clean.

More detail, and the full method behind the performance figures, is in
[`docs/performance.md`](docs/performance.md).

## Layout

| Crate | What it holds |
|---|---|
| `terrustia-proto` | The wire format, with no I/O: primitives, packets, tile-section coding, and the tile, NPC and housing tables extracted from the game |
| `terrustia-client` | A headless client: handshake, world view, movement, chat, tile and item actions |
| `terrustia` | The async server: world state, game loop, connection handling, `.wld` reading and writing |

## Licence

The server and the client are under the **GNU Affero General Public License v3.0 or later**; see
[`LICENSE`](LICENSE).

`terrustia-proto` is **MIT**, on purpose — see [`crates/terrustia-proto/LICENSE`](crates/terrustia-proto/LICENSE).
It is a description of Terraria's wire format with no I/O and no game logic in it, and anyone writing
a Terraria tool in Rust should be able to use that without taking on the server's licence.

The generated tables in that crate were produced *from* a decompiled copy of the game by the scripts
in `tools/`, each of which takes the decompiled tree as an argument. No decompiled source, no game
assets and no game text ship in this repository — the one exception being the town-NPC name pools in
`town_names.rs`, whose provenance is documented in `docs/generated-tables.md`.
