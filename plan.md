# terrustia — working plan

The live tracker. Updated as work lands, not before. The full reasoning behind each item is in the
approved plan; this file is what is done, what is next, and what was found on the way.

**Legend:** `—` not started · `~` in progress · `✓` done and verified

## Rules this file is held to

A row becomes `✓` only when all four hold, or it says which one does not apply:

1. **Seen working in a real Terraria client** — for anything a player can see.
2. **Automated suite green**, with a test that *fails on the unfixed code* where behaviour changed.
3. **Measured, with the number written down** — for anything performance-shaped.
4. **Cross-checked against the real `TerrariaServer`** — for protocol and gameplay.

## Done

| | Item | Evidence |
|---|---|---|
| ✓ | **Worlds backed up before anything wrote to them** | `~/terraria-worlds-backup-20260825-003210.tar.gz`, 4 files, listing verified |
| ✓ | **Hardmode ore tiers survive a save** — was silent data loss causing mixed-ore corruption | `wld.rs` records `hardmode_ores`; `wld_save.rs` patches it. Test `an_altars_ore_choice_survives_a_save` **verified failing on the unfixed code** (`left: -1431655766`). Real-world round trip: `ores [221, 223, 227]` |
| ✓ | **Banner kill counts survive a save** — was silent data loss, and our own comments claimed otherwise | Test `banner_kills_survive_a_save`, **verified failing before the fix**. Real-world round trip: `banner 3 = Some(4242)`. A banner past the file's run is dropped rather than shifting the header — asserted |
| ✓ | **`roundtrip_wld` now probes both fields** | The check that would have caught this. Run against the real 4200×1200 world: *"round-trip is faithful: every tile, chest and sign survived"* |
| ✓ | **A new `World` field cannot be added without deciding whether it persists** | Destructure with no `..` in `world.rs`. **Verified by adding a field and watching the build break** — and it has already caught one since |
| ✓ | **Townsfolk and tile-entity sections are partial-tolerant** | A truncated section keeps what decoded and is *carried through* rather than rewritten from a partial read — which used to delete every resident, or every pylon past the first bad entry |
| ✓ | **`.wld` versions newer than we know are refused by name** | There was a floor and no ceiling; a future format would have loaded positionally and been corrupted on save |
| ✓ | **A panic on the packet path no longer kills the server silently** | `catch_unwind` covered `tick()` but not `handle_event()` — the untrusted-input path. It unwound *past* the shutdown save, and `main` still exited 0 so `Restart=on-failure` never fired. Test verified failing without the guard |
| ✓ | **`/register` can no longer freeze the server** | Argon2 ran inline on the game task with no permission check: tens of ms against a 16.67 ms tick, so any client could stall the world in a loop. Now off-task, one hash per slot, four server-wide, released on disconnect. Test measures 64 registrations against a hash it times first |
| ✓ | **Tick phases are measured on the same clock as the tick** | `worst_us` was CPU, `phase_us` was wall — so a phase could out-cost its own tick, and every phase figure ever logged was inflated. The `world` phase was also 13 systems in one lap; now split |
| ✓ | **The autosave no longer costs the tick** | **5,606 µs → 43–137 µs.** Only changed sections are copied into a buffer the previous save hands back. Verified live on the real world |
| ✓ | **`set_tile` no longer hashes** | The section `HashSet` was SipHashing a coordinate pair on every tile write — tens of thousands a tick under liquid load. Now one flag per section, under a kilobyte |
| ✓ | **Saving verifies before replacing, fsyncs, and keeps 3 backups** | An atomic rename over a corrupt file is an atomic loss. Live-verified: 5 saves, exactly 3 backups, no stray `.tmp`, oldest backup loads |
| ✓ | **Connection ceiling, per-address cap, handshake deadline** | The accept loop was unconditional. `idle_timeout` resets on any byte, so a trickle held a place for ever. Both verified failing with the guard removed |
| ✓ | **The whole workspace builds on Windows** | `clock.rs` used `clock_gettime`/`CLOCK_THREAD_CPUTIME_ID`, neither of which exists there — the sole compile blocker. Plus `ctrl_close`/`ctrl_shutdown`, without which a service stop skipped the save. **All five release targets checked** |
| ✓ | **Tile-edit spam limiter** | A regression *from* vanilla, not parity with it. All six numbers transcribed from `RemoteClient`. Tested at both ends: normal building never trips, a flood trips inside a second |
| ✓ | **A stranger cannot claim a fresh server** | Everyone had every permission until the first `/register`, and first-come won. Now needs a one-time token printed to the console |
| ✓ | **Announcements send localization keys** | Parity bug + localization bug + verbatim game text, fixed at once. Four kept as English deliberately, where I could not verify the key |
| ✓ | **The housing screen works** | Packet 60 inbound fell through to the ignore arm, so dragging an NPC into a room did nothing. Driven over a real socket; verified failing without the dispatch arm |
| ✓ | **Save durability + 3 rotating backups + `backups`/`rollback`** | Verified live: rollback restores, keeps the replaced world aside, refuses an unreadable backup, and stops rather than hot-swapping |
| ✓ | **All five release targets compile** | Windows was the sole blocker; `ctrl_close`/`ctrl_shutdown` added so a service stop does not skip the save |
| ✓ | **Licence split, workspace lints, `cargo deny`** | proto is genuinely MIT now; warnings denied everywhere; all four deny checks pass |
| ✓ | **CI, release and container workflows** | fmt/clippy/tests, five-target matrix, `cargo deny`, and a 60-second soak with assertions. Soak run locally: passes |
| ✓ | **`FEATURES.md` and a rewritten README** | Leads with "serve the world you already have"; says "there are no trees" plainly |
| ✓ | **`--world <name>` and `--worlds`** | Verified: `--world "The Successful Excrement"` resolves the space→underscore rename |
| ✓ | **Version refusals name both sides** | The message named 1.4.5.7 while the server spoke 326. Plus a test that fires when the game moves |
| ✓ | **Whitelist** | Empty means off, so it cannot lock the operator out on the day it lands |
| ✓ | **`cargo fmt` across the tree** | 46 files, its own commit |
| ✓ | **Trees, jungle grass, vines, cacti at generation** | 349 trees, 2,415 vine tiles, jungle grass 700→3,198. Frames transcribed from `WorldGen.GrowTree`. Found three bugs by measuring: wrong argument to `grow_cactus`, a vine scan that could never match, and a latent frame-sentinel bug that broke the save round trip for 1,987 tiles |
| ✓ | **Whitelist, SECURITY.md, CONTRIBUTING.md** | |
| ✓ | **Lakes** | 4 on a 4200×1200 world; surface water 5,764 → 6,069. Reimplemented, not transcribed — vanilla's siting filter reads `GenVars` this generator never writes |
| ✓ | **Fixed a bug in my own CI** | `cargo build --workspace --examples` builds examples and *not* the server, so the soak job would have failed on its first run |
| ✓ | **Doors build frame-important tiles correctly** | `Tile::block` on a framed type trips a debug assertion and ships -1 frames; six tests were failing on it |
| ✓ | **Blood moon / eclipse resume from a save** | Was read and thrown away on load; the bytes on disk were always correct, only the session forgot. Test verified failing on the unfixed code |
| ✓ | **Jungle and underground-desert chests carry vanilla's real items** | Transcribed signature-item lists from `AddBuriedChest`, wired ahead of the existing depth-tiered fallback. Tests assert the exact vanilla item ids |
| ✓ | **Settled water at generation** | Lakes, oceans and underworld lava all reach a stable rest state before a world ships. Reuses the runtime liquid simulator rather than porting vanilla's separate algorithm. Real world: 19.5ms, 38 rounds, converged |
| ✓ | **Flowers, mushrooms, alchemy herbs, sunflowers at generation** | The rest of Tier 1's Wave A. Zero frame-sentinel bugs across all four |
| ✓ | **`place_object` + pots, statues, piles, fallen logs at generation** | 4,032 pots, 143 statues, 35+8 piles measured on a real world. Two real bugs caught before landing: a floor check that read an inactive tile's leftover block id as solid ground, and a dropped step-back in statue placement that made every attempt overlap its own floor |
| ✓ | **The Lihzahrd Altar is placed in the jungle temple** — was silently making Golem unreachable in every generated world | A real client refuses the Power Cell interaction without an altar tile nearby — no server-side workaround exists for its absence. Found by a worldgen sizing pass reading vanilla's source, not by anyone playing the game. Tested across 40 seeds and the full range of rolled temple sizes; verified failing on the unfixed code |
| ✓ | **Persistent scratch directory** | `/private/tmp` gets reaped by the OS after a few days and took the decompiled Terraria source tree with it mid-session, out from under several running agents. `.scratch/` lives inside the repo instead |
| ✓ | **Piles' ground→style table, transcribed from source** | Replaces the version carried over from sizing notes. Caught two real bugs along the way: `BOULDER` was defined as tile id 26 (Demon Altar) instead of 138, and the small-pile floor check was missing vanilla's slope/half-brick exclusion |
| ✓ | **Worldgen Tier 1 is done: traps and smoothed terrain** | `traps.rs` (dart traps, mines, boulder traps, geysers, sand traps) and `smooth.rs` (`SmoothWorld`, reordered to run last — see its module doc). Real 4200×1200 world: 72 dart traps, 10 mines, 4 boulder traps, 1 geyser, 30,107 tiles smoothed. `wiring.rs` gained a real fix alongside traps: a land mine is tile 141, not a variant of the dart trap's tile 137 |

## Audit findings, ranked

Six audits ran against the code. Everything below is either fixed above, or has a row further down.

| | Finding | State |
|---|---|---|
| 🔴 | Ore tiers + banner kills lost on every save of a loaded world | **fixed** |
| 🔴 | `/register` runs Argon2 inline on the game task, no permission check, no rate limit — one client freezes the server | **fixed** |
| 🔴 | `catch_unwind` wraps `tick()` but not `handle_event()`; a panic on the packet path kills the server, skips the shutdown save, and exits `SUCCESS` so `Restart=on-failure` never fires | **fixed** |
| 🟠 | `phase_us` is wall time, `worst_us` is CPU time — every phase cost ever logged is inflated | **fixed** |
| 🟠 | Autosave copies the whole world on the game task: 8–13 ms of a 16.67 ms budget on a *small* world | **fixed** |
| 🟠 | Announcements send literal English where vanilla sends localization keys — parity bug + localization bug + verbatim game text in source | **fixed** |
| 🟠 | Town NPCs are all-or-nothing on parse failure; tile entities truncate silently from the first failure | **fixed** |
| 🟡 | No upper `.wld` version bound — a future format would be misparsed and corrupted on save | **fixed** |
| 🟡 | `set_tile` SipHashes a section key on every tile write | **fixed** |
| 🟡 | Packet 60 inbound dropped: the in-game housing UI (drag NPC to a room, evict) does nothing | **fixed** |
| 🟡 | Worldgen oracle verifies nothing — `passes.rs:54` is `PASSES: &[Pass] = &[]` | open |
| 🟢 | Journey research and Bestiary data **are** preserved (verified by section index) | no action |
| 🟢 | 59 dependencies, all permissive, zero GPL; `cargo audit` clean; one `unsafe` block, justified | no action |

## Verified at the end of the session

```
cargo fmt --check          clean
cargo clippy               0 warnings (warnings denied workspace-wide)
cargo deny check           advisories ok, bans ok, licenses ok, sources ok
cargo test --workspace     1,489 passing, 0 failing
five release targets       all ok
tools/soak_ci.sh           passed
real-world round trip      faithful; every field survives
```

## Next

### Block A — stop losing data, then publish

| | Item |
|---|---|
| — | Round-trip a real *played-in* world through real Terraria (GAPS §31 never exercised the preserved path) |
| — | **Create the GitHub repo and push** (workflows written, never run) |
| — | Tile action log + `world undo <player> <duration>` |
| — | `--new <name>` to generate into the world directory |

### Block B — earn the tag

| | Item |
|---|---|
| — | `AUDIT.md` + question round |
| — | Section-ownership check on tile edits (vanilla parity) |
| — | `cargo-fuzz` over the decoder; commit `.trcap` fixtures and replay in CI |
| — | **Tag v0.0.1** |

### Block C — make it fast, honestly

| | Item |
|---|---|
| — | Liquid pinning test, then `liquid.rs` read/alloc reductions |
| — | Benchmarks: large world × 16 (tuning), × 255 (ceiling, pass/fail) |
| — | Section encoding off the tick; parallel worldgen |
| — | Packaging: Homebrew, winget, AUR, systemd unit, container `HEALTHCHECK` |
| — | `terrustia update` with signature verification |
| ✓ | Sticky console (history, tab completion), equal-width startup panels, save destination/autosave rows | |

### Block D — make it complete

**Worldgen Tier 1 is done.** Trees, jungle grass/vines, cacti, lakes, settled water,
flowers/mushrooms/herbs/sunflowers, pots/statues/piles/fallen logs, the Lihzahrd Altar, traps and
smoothed terrain are all wired into `build()` — see Done, above.

**Worldgen Tiers 2 and 3**: re-sized for real once the decompiled source came back (the first pass
was done mostly blind, mid-session, after a tmp-reaper wiped the source tree — several numbers
below correct that pass rather than merely refining it).

**Headline correction: the 5,253-line `Terraria.WorldBuilding` DSL estimate was wrong.** Measured
directly: zero Tier 2/3 pass bodies, and none of their helpers, call the heavy `WorldUtils`/
`Actions`/`Modifiers`/`Shapes` pipeline. What they actually use is `StructureMap` (98 lines, an
overlap-rectangle tracker reached via `GenVars.structures`) and a `Biome`-class convention
(`GenVars.configuration.CreateBiome<T>()`) for ~15 self-contained micro-biome routines.
`CaveWallVariety` touches `ShapeData` (114 lines, a point-set container), not the full DSL. **A
~200-line structure-overlap tracker covers everything measured — not a 5,253-line framework.**

| | Item | Vanilla pass(es) | Real size | DSL? | Difficulty |
|---|---|---|---|---|---|
| Tier 2 | Floating islands + houses | `FloatingIslands`+`FloatingIslandHouses` | ~1,950 (incl. `CloudIsland`/`SnowCloudIsland`/`DesertCloudIsland`/`CloudLake`) | none | Hard — big, plain transcription like Tier 1 |
| Tier 2 | Living trees + walls | `LivingTrees`+`LivingTreeWalls` | ≥274 + `GrowLivingTree` unmeasured | none | Medium |
| Tier 2 | Spider caves | `SpiderCaves` | ~142 | none | Easy–medium |
| Tier 2 | Gem caves | `GemCaves` | 45 | none | Easy |
| Tier 2 | Pyramids | `DunesAndPyramidLocations`+`Pyramids` | ~640 (incl. `Pyramid()`, `DunesBiome`) | none | Medium |
| Tier 2 | Underworld ruined houses + hellforges | `Underworld`+`Hellforges` | ≥273 + 2 helpers unmeasured | none | Medium |
| Tier 2 | Jungle shrines + chests | `JungleShrines`+`ChestsInJungleShrines`+`LihzahrdTemplePart2` | ~201 | none | Medium |
| Tier 2 | Underground cabins | `UndergroundHousesAndBuriedChests` | ≥252 | `StructureMap` only | Medium |
| Tier 2 | Oasis | `Oasis` | 26 | none | Easy |
| Tier 2 | **Micro-biomes** (was one unsized bullet — now the largest single Tier 2 item) | `MicroBiomes`+`Marble`+`Granite` | **~2,700+** (15 classes in `Terraria.GameContent.Biomes/`, 4,240 lines total) | `StructureMap` + `Biome` pattern | Medium per-biome, large in aggregate |
| Tier 2 | Glowing mushroom biome (missing from both the old item list *and* this sizing pass's report — found and measured directly while integrating it) | `GlowingMushroomPatches`+`GlowingMushroomPlantsUndergroundAndJunglePlants` | 219 + 43 = **262** | none (0 DSL calls, 17 plain loops) | Medium — resolves the old "never read, difficulty unknown" flag |
| Tier 3 | Moss + moss caves | `MossAndMossCaves`+`LongMoss` | 284 | none | Medium |
| Tier 3 | Speleothems + exposed gems | `SpeleothemsAndGemTrees`+2 exposed-gem passes+shared web pass | 229 | none | Easy–medium |
| Tier 3 | Wall variety | `CaveWallVariety`+`CaveWallsInEnclosedSpaces` | 194 | `ShapeData` (light) | Easy–medium |
| Tier 3 | Dirt-wall cleanup | `DirtWallCleanup` | 116 | none | Easy |
| Tier 3 | Tile cleanup (bundle of 6 passes) | `TileCleanup`+`QuickCleanup`+`FinalCleanup`(356, corrected from a mis-bounded 65,864)+`BrokenTrapCleanup`+`GravitatingSandCleanup`+`SurfaceOreAndStone`+`SurfaceDirtWallsToGrassWalls` | **1,121** | none | Bigger than "small tile scans" implied, but each pass is mechanically simple |
| Tier 3 | Waterfalls | `Waterfalls` | 59 | none | Easy |
| Tier 3 | Thin ice | `FragileIceOverIceBiomeWater` (the actual `ThinIceBiome` is counted under Tier 2 micro-biomes — don't double-book) | 30 | none | Easy |
| Tier 3 | Lilypads/cattails/coral/palms | `LilypadsCattailsBambooAndSeaweed`+`CactusPalmTreesAndCoral` | 277 | none | Medium |

Measured totals: Tier 2 ≈ 6,500+ lines (plus a handful of still-unmeasured helper functions —
`GrowLivingTree`, `GetMaxPossibleRoomsInABigAbandonedHouse`, `GenerateUnderworldStartingMound`),
Tier 3 ≈ 2,310 lines. Both lower than the old blind estimate once the DSL correction is applied —
the old ~7,773/~4,792 numbers were inflated by assuming framework work that isn't actually needed.

Two more things this pass turned up, checked immediately rather than left as flags:
- **Not a gap**: vanilla's `PotsGraveyardsAndBoulderPiles` (the pass `pots.rs`/`piles.rs` already
  transcribe) also contains a `SpawnGraveyardBiomesEverywhere()` call — but it's gated behind
  `dontStarveWorldGen && drunkWorldGen && getGoodWorldGen` or the `graveyardBloodmoonStart` secret
  seed. Ordinary worlds never get pre-placed tombstones from this pass in vanilla either, so
  terrustia's pots/piles/statues ✅ row is correct as-is. Confirmed by reading the gate condition
  directly, not by re-trusting the sizing pass's own flag.
- **Resolved**: terrustia had no handling at all for vanilla's secret seeds (Celebrationmk10, drunk,
  not-the-bees, remix, no-traps, "get fixed boi", Don't Starve), and neither `FEATURES.md` nor this
  plan ever stated whether they were in scope. Asked rather than assumed, the same way Steam P2P
  was flagged rather than defaulted — **in scope, but deprioritized behind ordinary-world parity**.
  Tracked as a `FEATURES.md` 🔴 row and in the backlog below, not a disclosed exception like Steam
  P2P.
- The Lihzahrd Altar fix (already shipped, tested across 40 seeds) uses a different mechanism than
  vanilla: a 7-offset retry loop inside `structures::temple()` with a `debug_assert` fallback,
  where vanilla has a dedicated unconditional pass (`LihzahrdAltar`, 32 lines) placing the altar at
  precomputed `GenVars.lAltarX/lAltarY` with no failure path. Functionally verified and passing —
  this is a structural note for a future hardening pass, not a bug: our retry loop could in theory
  hit its assert on a pathological layout that vanilla's precomputed-coordinate approach can't.

| | Item |
|---|---|
| — | Build the ~200-line `StructureMap`/`ShapeData` subset (not the full 5,253-line DSL — see correction above) |
| — | Worldgen Tier 2, item by item, once the tracker above exists |
| — | Worldgen Tier 3, item by item — the ~25 easy items first |
| — | A bot that starts with nothing and kills Moon Lord — Tier 1's own acceptance test, now that traps and `SmoothWorld` have landed |
| — | Town NPCs (shops, combat, happiness) |
| — | Journey mode |
| — | 3 missing events (Slime Rain, Party, Lantern Night) |
| — | ~123 enemy drop tables |
| — | Pets, mounts, minecart tracks |
| — | Secret seeds (Celebrationmk10, Drunk World, Not the Bees, Remix, No Traps, "get fixed boi", Don't Starve) — in scope, deprioritized behind ordinary-world parity |

## Corrections to earlier claims

Kept because they are the reason the bugs above went unnoticed for so long.

1. Worldgen was **not** the whole remaining gap.
2. The manifest oracle verifies **nothing** — `PASSES` is empty.
3. `SettleLiquids` needs new code, not transcription.
4. `phase_us` and `worst_us` are different clocks.
5. "Verified 60 Hz" rested on (4), so it is withdrawn until re-measured.
6. `GAPS.md` §31's round trip used a *generated* world, so it never exercised the preserved-header
   path — the one that touches real players' files.
7. The first Tier 2/3 sizing pass claimed a ~5,253-line `Terraria.WorldBuilding` DSL gated half of
   Tier 2 — made mostly blind, mid-session, after the decompiled source was wiped. Re-measured
   directly against restored source: zero Tier 2/3 passes call that pipeline. A ~200-line
   `StructureMap`/`ShapeData` subset covers everything actually used.
7. `server.rs:8496` and `GAPS.md:38` both claimed banner kills survive a restart. They did not.
