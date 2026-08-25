# What works

The honest answer, feature by feature. Built from six audits run against the code rather than
against these notes — because the two documents this replaces had drifted far enough apart to
disagree with each other about whether cacti grow.

| | Meaning |
|---|---|
| ✅ | Implemented, and checked |
| 🟡 | Works, with the limitation named in the row. "Partial" without a qualifier is a lie by omission |
| 🔴 | Not implemented. A player will notice |
| ⬜ | Deliberately out of scope, with the reason |

**The short version:** point it at a `.wld` you already have and it serves it well —
same clients, same file, and measurably lighter than the official server. Point it at nothing and
it generates a world that is playable, decorated, settled and smoothed — forests, a green jungle,
cacti, lakes, settled water, pots, statues, piles, fallen logs, traps and rounded terrain are all
there now. Tier 1 is done; what's left is Tier 2/3's biome set pieces (floating islands, spider
caves, gem caves, pyramids and the rest), sized and tracked in `plan.md`.

---

## Protocol and connectivity

| | Feature | Notes |
|---|---|---|
| ✅ | Protocol release 326 (1.4.5.8) | Release 325 accepted too; they differ in the announced number and four bytes of packet 7 |
| ✅ | Handshake, world data, section streaming | Checked against a real `TerrariaServer`, not only against our own client |
| ✅ | Server password | |
| ✅ | Localized announcements | Keys with substitutions, as the game sends — so a non-English client reads its own language |
| 🟡 | Packet coverage | 109 of 163 message ids handled; most of the rest are outbound-only or dead in vanilla too. Genuinely missing: PvP buff spread (55), portal-gunning an NPC (100), spectating (150), shop overrides (104) |
| ⬜ | Steam P2P / lobbies | Steamworks' licence is incompatible with AGPL |
| ⬜ | Encryption | Terraria's protocol has none. `/login` sends a password as ordinary chat text — do not reuse a real one |

## World files

| | Feature | Notes |
|---|---|---|
| ✅ | Load `.wld` (format 279–325) | Refuses older and newer by name rather than guessing |
| ✅ | Save `.wld` | Header preserved byte-for-byte and patched, so state we do not model survives untouched |
| ✅ | Journey research, bestiary, pylon rooms, pressure plates | Carried through verbatim — verified by section index, not assumed |
| ✅ | Verified before replacing, fsynced, 3 rotating backups | An atomic rename over a corrupt file is an atomic loss |
| 🟡 | Bestiary | Existing data is preserved; kills during a session are not added to it |
| 🟡 | In-progress blood moon / eclipse | Not resumed from the file. The file's own bytes are undisturbed |

## World generation

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
| ✅ | Pots, statues, piles, fallen logs | Statue order is load-bearing and transcribed verbatim (73 entries). The piles' ground→style table is now transcribed directly from the `Piles` pass's primary loop (`WorldGen.cs:18963-19030`), replacing the earlier version carried over from sizing notes — also caught and fixed a wrong boulder-floor tile id and a missing slope/half-brick check in the small-pile placer along the way |
| ✅ | Traps | Dart traps, land mines, boulder traps and geysers, plus the desert's sand trap — transcribed from `placeTrap`/`PlaceSandTrap`/the driving `Traps` pass. Ordinary-world path only; every secret-seed branch is skipped (see the secret-seeds row below). A real 4200×1200 world: 72 dart traps, 10 mines, 4 boulder traps, 1 geyser |
| ✅ | Smoothed terrain (`SmoothWorld`) | Transcribed, with one deliberate reordering: this generator runs smoothing *last*, after every decoration, rather than vanilla's before-decoration placement — see `smooth.rs`'s doc comment for why, and how the altar and other fixtures stay protected either way. 30,107 tiles smoothed on the same 4200×1200 world |
| 🔴 | Floating islands, spider/gem caves, pyramids, living trees, jungle shrines, underground cabins, oasis, micro-biomes, glowing mushroom biome (Tier 2) | Sized for real against restored source in `plan.md` — no longer needs the heavy shape/structure DSL that was assumed there; a ~200-line structure-overlap tracker covers it |
| 🔴 | Secret seeds (Celebrationmk10, Drunk World, Not the Bees, Remix, No Traps, "get fixed boi", Don't Starve) | In scope, not a disclosed exception like Steam P2P — deprioritized behind ordinary-world parity, tracked in `plan.md`'s backlog |
| 🔴 | Moss, wall variety, waterfalls, thin ice, cleanup passes (Tier 3) | Sized in `plan.md`; mostly small single-purpose tile scans |
| ⬜ | Seed-identical worlds | Sized at 219–372 engineer-days. Feature-complete is the goal; identical is not |

## NPCs, bosses and events

| | Feature | Notes |
|---|---|---|
| ✅ | All 691 NPC types | Each runs a transcribed routine; a test walks the whole roster |
| ✅ | All 20 bosses | Multi-phase, including Moon Lord's opening sequence and the cultist's tablet ritual |
| ✅ | Both moons, all four invasions, Old One's Army with Betsy, eclipse with Mothron | |
| ✅ | Rain, wind, sandstorms | |
| ✅ | Town NPC arrival and housing | Including the in-game housing screen |
| ✅ | Town NPC shops | Opening and using a shop is entirely client-side in vanilla — no packet populates it, and the click gate (`townNPC`, derived from `type` alone, plus `velocity.Y == 0`) is satisfied by an ordinary NPC sync. The one thing the server owns, packet 40, was already correct (proven for the bound-NPC case by the rescue mechanic); a new test proves it for an ordinary town NPC too, relayed to other players. No happiness-driven pricing or shop overrides yet — tracked separately below and as packet 104 |
| 🟡 | **Town NPCs fighting back** | Four representative types across all of vanilla's attack classes — Merchant (ranged), Arms Dealer (ranged), Wizard (ranged), Dye Trader (melee) — target and damage nearby hostiles, verified end to end over a real socket including the shot actually landing. Reimplemented cadence, not vanilla's exact windup timing (see `game::ai::town_combat`'s module doc). The other ~23 vanilla combat-capable town NPCs are mechanical to add from here but not yet done, so a town with only those types still stands still |
| 🔴 | NPC happiness, price effects, moving out | |
| 🔴 | Slime Rain, Party, Lantern Night | |
| 🟡 | Enemy drops | `tools/check_drops.py` itself was found to have real parsing bugs producing false positives (it briefly "proved" Eye of Cthulhu owed the player an Iron Pickaxe); fixed, and boss loot is now genuinely complete — 3 real missing trophies (Moon Lord, Empress of Light, Deerclops) and the Martian Saucer's weapon pool were found and added this way. ~111 ordinary enemies are still short at least one drop, a real and lower number than previously measured now that the checker's own noise is gone, but not yet individually fixed |

## Items and mechanics

| | Feature | Notes |
|---|---|---|
| ✅ | Buffs and debuffs, item entities, shimmer | |
| ✅ | Wiring, logic gates, timers, teleporters | |
| ✅ | Chests, signs, tile entities, pylons | |
| ✅ | Boss summon items, Angler quests, fishing NPCs | |
| ⬜ | Crafting validation | Terraria has no craft packet; the client is authoritative in vanilla too |
| ⬜ | Armour set bonuses, accessories | Client-side in vanilla; the server applies plain defence |
| 🔴 | Pets, mounts, minecart tracks | No server-side existence |

## Journey mode

| | Feature | Notes |
|---|---|---|
| 🔴 | **Every Journey power** | Net module 4 is not defined, so godmode, time and weather freeze, the sliders, research and duplication are all silently swallowed |
| ✅ | Existing research survives a save | Preserved verbatim; it simply cannot grow here |

## Multiplayer

| | Feature | Notes |
|---|---|---|
| ✅ | PvP, teams, deaths, respawn, chat | |
| ✅ | Accounts, groups, permissions, bans by name/address/uuid | Argon2, off the game task |
| 🟡 | Chat commands | 17 of them. No warps, regions, or item bans — that is the deferred TShock-shaped work |
| ✅ | Whitelist | Empty means off, so it cannot lock the operator out on the day it is enabled |
| 🔴 | REST / web admin | Console and in-game commands only |

## Hardening

| | Feature | Notes |
|---|---|---|
| ✅ | Decode path cannot panic or over-allocate | One `unsafe` block in the workspace, for the CPU clock |
| ✅ | A panic on the packet path saves the world and exits non-zero | So `Restart=on-failure` fires |
| ✅ | Connection ceiling, per-address cap, handshake deadline | |
| ✅ | Tile-edit spam limiter | Vanilla's own six numbers, transcribed from `RemoteClient` |
| ✅ | Server claim requires a console token | |
| ⬜ | Server-authoritative inventory and damage | Vanilla trusts the client for both; diverging would change how the game plays |

## Platforms

| | | Notes |
|---|---|---|
| ✅ | Linux x86_64 / aarch64, macOS arm64 / x86_64, Windows x86_64 | All five pass `cargo check` |
| 🟡 | Container image, signed releases, packaging | The repository exists now (`github.com/bybrooklyn/terrustia`) and the container workflow has actually run: multi-arch image built, pushed, cosign-signed, and smoke-tested serving with no configuration. Getting there for real found and fixed three bugs invisible to local `cargo check` — CI targeting a branch (`main`) that never existed here, cross-compile targets landing in the wrong toolchain, and `crossterm` missing the Cargo feature its own Windows backend needs to compile. Signed releases still untested — no tagged release has been cut yet, and that workflow only triggers on a `v*` tag |

---

## Measured against the official server

Same 4200×1200 world, same machine, nobody connected.

| | vanilla 1.4.5.8 | terrustia |
|---|---|---|
| Startup | 2.26 s | 0.41 s |
| CPU, idle | 104% of a core | 0.7% |
| RAM, idle | 641.8 MB | 45.4 MB |
| Bandwidth over 5 min | 148,874 B | 133,400 B |

A note on the tick, because the claim changed twice. "A verified full 60 Hz tick" originally rested
on instrumentation that was comparing CPU time against wall time, so it was withdrawn. With that
fixed, it has been measured again on the same world: a typical tick costs **184–330 µs of a
16,666 µs budget**, and the autosave — previously the most expensive thing an idle server did, at
up to 7,656 µs — now costs 43–137 µs because only changed sections are copied.

The remaining figures above were measured externally, at the process level, and were never affected
by that instrumentation.
