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
it generates a world that is playable and now has forests, a green jungle and cacti — but still no
lakes and no settled water. Generation is the largest remaining gap and is being worked pass by
pass.

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

The largest gap. ~26 of Terraria's 106 passes have any counterpart.

| | Feature | Notes |
|---|---|---|
| ✅ | Terrain, caves, ore veins | |
| ✅ | Dungeon, temple, hive, underworld, evil chasms | Our own algorithms, not vanilla's |
| ✅ | Chests, altars, life crystals, shadow orbs | Enough to be beatable |
| ✅ | **Trees** | Frames transcribed from `WorldGen.GrowTree` — trunk, branches, roots, canopy. 349 on a 4200×1200 world |
| ✅ | Vines and jungle grass | The underground jungle's mud is lined with grass, which is what vines hang from: 2,415 vine tiles |
| ✅ | Cacti | |
| 🔴 | Lakes, settled water | `SettleLiquids` needs a generation-only settler we do not have |
| 🔴 | Flowers, mushrooms and herbs at generation | Surface plants exist; the taller decoration does not |
| 🔴 | Pots, statues, traps, piles, fallen logs | |
| 🔴 | Smoothed terrain | Everything is blocky |
| ⬜ | Seed-identical worlds | Sized at 219–372 engineer-days. Feature-complete is the goal; identical is not |

## NPCs, bosses and events

| | Feature | Notes |
|---|---|---|
| ✅ | All 691 NPC types | Each runs a transcribed routine; a test walks the whole roster |
| ✅ | All 20 bosses | Multi-phase, including Moon Lord's opening sequence and the cultist's tablet ritual |
| ✅ | Both moons, all four invasions, Old One's Army with Betsy, eclipse with Mothron | |
| ✅ | Rain, wind, sandstorms | |
| ✅ | Town NPC arrival and housing | Including the in-game housing screen |
| 🔴 | **Town NPC shops** | The merchant economy is unreachable |
| 🔴 | **Town NPCs fighting back** | The first Blood Moon after anyone moves in, the town stands still and dies |
| 🔴 | NPC happiness, price effects, moving out | |
| 🔴 | Slime Rain, Party, Lantern Night | |
| 🟡 | Enemy drops | ~123 ordinary enemies drop nothing yet. Boss and progression drops are complete and walked end to end |

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
| 🔴 | Whitelist | Bans only, which is reactive |
| 🔴 | REST / web admin | Console and in-game commands only |

## Hardening

| | Feature | Notes |
|---|---|---|
| ✅ | Decode path cannot panic or over-allocate | One `unsafe` block in the workspace, for the CPU clock |
| ✅ | A panic on the packet path saves the world and exits non-zero | So `Restart=on-failure` fires |
| ✅ | Connection ceiling, per-address cap, handshake deadline | |
| ✅ | Tile-edit spam limiter | Vanilla's own numbers |
| ✅ | Server claim requires a console token | |
| ⬜ | Server-authoritative inventory and damage | Vanilla trusts the client for both; diverging would change how the game plays |

## Platforms

| | | Notes |
|---|---|---|
| ✅ | Linux x86_64 / aarch64, macOS arm64 / x86_64, Windows x86_64 | All five pass `cargo check` |
| 🔴 | Container image, signed releases, packaging | Not published yet |

---

## Measured against the official server

Same 4200×1200 world, same machine, nobody connected.

| | vanilla 1.4.5.8 | terrustia |
|---|---|---|
| Startup | 2.26 s | 0.41 s |
| CPU, idle | 104% of a core | 0.7% |
| RAM, idle | 641.8 MB | 45.4 MB |
| Bandwidth over 5 min | 148,874 B | 133,400 B |

One caveat worth stating: "a verified full 60 Hz tick" was a claim resting on instrumentation that
turned out to be comparing CPU time against wall time. It is very likely still true and it is no
longer *verified*, so it is not claimed here until it has been measured again.
