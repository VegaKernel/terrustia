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
| ✓ | **The autosave no longer costs the tick** | **5,606 µs → 43–137 µs.** Only changed sections are copied into a buffer the previous save hands back. Verified live on the real world. This claim had a real gap until the repo's first CI soak run caught it: the *first* autosave of a server's life had no prior buffer to diff against and paid 14,833 µs — 89% of a tick — inside a counted tick. Fixed by pre-warming the buffer during startup, before any tick is being counted; see the CI row below |
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
| ✓ | **Town NPC shops confirmed already working — the 🔴 row was stale, not the feature** | Traced vanilla source: opening/using a shop is 100% client-side, no packet involved. The one server-owned piece, packet 40, was already correct; proved for the ordinary (non-bound) case with a new two-client integration test, previously only covered for the bound-NPC rescue path |
| ✓ | **`check_drops.py` had seven real parsing bugs, found and fixed** | It initially "proved" Eye of Cthulhu owed the player an Iron Pickaxe. Traced every false positive against `ItemDropDatabase.cs` by hand rather than trusting the summary. With it actually trustworthy: 3 real trophy gaps (Moon Lord, Empress of Light, Deerclops) and the Martian Saucer's weapon pool were genuinely missing and are now fixed; boss loot has zero remaining unjustified gaps |
| ✓ | **`StructureMap`/`ShapeData`, the real Tier 2 foundation** | Corrects the earlier blind sizing pass's ~5,253-line DSL estimate: nothing in Tier 2/3 actually calls that pipeline, only these two small types (208 real lines total). Not wired into `build()` yet — nothing calls it until the first real Tier 2 pass exists |
| ✓ | **Town NPCs fight back** | One representative of each of vanilla's four `AttackType` classes (Merchant ranged, Arms Dealer ranged, Wizard ranged, Dye Trader melee) — proves the mechanism end to end rather than porting all ~27 combat-capable town NPCs at once. Verified over a real two-client socket: a Merchant's shot is broadcast to both clients and the target's synced health actually drops. 4.70 µs/tick for 12 town NPCs × 60 hostiles |
| ✓ | **`github.com/bybrooklyn/terrustia` created and pushed — real CI, for the first time ever** | "Written and validated, never run" turned out to be hiding two genuine, independent bugs neither `cargo check` locally nor any human review had caught, because nothing had ever actually run these workflows against a real runner: (1) `ci.yml`/`docker.yml` triggered on a `main` branch that has never existed here — every commit in this repo's history is on `master` — so ordinary pushes would never have run CI at all; (2) `rust-toolchain.toml` pins Rust 1.97.1, but the workflows installed cross-compile targets into the unrelated "stable" (1.98.0) toolchain, so `cargo build --target` inside the repo — which always resolves to the pinned 1.97.1 — found the target missing. Fixed by declaring `targets` in `rust-toolchain.toml` itself, the same self-declaring pattern it already used for `components`. A *third* bug turned up once CI actually ran for real: `crossterm`'s `default-features = false` dropped its `"windows"` feature, which gates the `winapi`/`crossterm_winapi` deps its own Windows backend source needs to compile at all — untestable from this session's macOS/Linux environment, only catchable by an actual Windows runner. A *fourth*, this one a genuine runtime bug rather than a CI-config mismatch: the soak job failed on "a tick went over half its budget," and the real cause was the first-autosave gap the "autosave no longer costs the tick" row above now documents. Container image built, pushed, cosign-signed, and smoke-tested with no configuration — genuinely working, not just written |
| ✓ | **FEATURES.md merged into README.md, FEATURES.md deleted** | One canonical document instead of two describing the same thing — every cross-reference (SECURITY.md, CONTRIBUTING.md, three source doc-comments, the release workflow) repointed |
| ✓ | **`AUDIT.md`** | Public-facing track record — data safety, availability, correctness, two corrected sizing estimates, the four CI/release bugs. Every claim spot-checked against `git log` before publishing, not just transcribed from this file |
| ✓ | **A real played-in world's preserved bytes verified surviving a round trip (GAPS §31)** | The earlier round-trip check only ever used a freshly generated world, which has no real Journey research/bestiary/pressure-plate state to lose. `roundtrip_wld` now diffs every trailing preserved section byte-for-byte against a real, actually-played world (a copy of it) — all 5 sections identical. Cross-checked further: the re-saved file loaded in a real `TerrariaServer` and served a real connecting client correctly |
| ✓ | **`gen_drops.py` had the same variable-collision bug `check_drops.py` was fixed for, never got the fix; recovered 45 of 111 drop-table gaps** | Fixed the generator's `npcNetIds` collision, an over-broad exclusion that dropped a chain's flat prefix along with its conditional tail, and the checker's blindness to `match { N => { ... } }` block arms. A real bug found along the way: `one_from()`'s blanket expert-mode guard was emptying Mimics' loot pool in expert mode — Mimics aren't bosses and gate on hardmode, not difficulty. 66 gaps remain, each individually traced and left for a stated reason (a chance-gated pool neither table represents yet, or an untracked `Conditions` dimension) |
| ✓ | **`caves()` no longer leaves the wall it carved through in place** | `terrain::fill` paints wall for the whole underground before caves ever run, and `caves()` kept it on every carved tile — so `cave_flood::count` (the shared utility `GemCaves`/`SpiderCaves`/`LivingTrees` all use to find an unwalled site) saturated to its cap everywhere, with no unwalled pocket to find anywhere in a generated world. Fixed narrowly (`hollow_no_wall`/`hollow_blob_no_wall`, `caves()` only — the dungeon/hive/chasms/temple keep their wall on purpose). Vanilla's own pass order is the confirmation: wall-filling (`CaveWallsInEnclosedSpaces`/`CaveWallVariety`) is Tier 3, run well after the Tier 2 passes that need bare pockets. A second, separate defect surfaced and is not yet fixed: `caves()` produces one large interconnected network rather than vanilla's mix of isolated pockets, so siting still needs rework even with walls cleared |
| ✓ | **Web admin panel foundation**: embedded in the main binary (`rust-embed`+`axum`, bundling mechanism from `../alchemist`), login reusing the real account store, a live WebSocket status view | Two real issues fixed before landing: the session token used a hand-rolled xorshift seeded from mostly-guessable inputs for a credential that grants full panel control — replaced with the same OS CSPRNG already used for password salts; and a build-breaking gap where `web-panel/dist/` (gitignored build output) doesn't exist on a fresh checkout, which fails the *entire* crate's compile via `rust-embed`'s macro — fixed with a `build.rs` fallback (matching `../alchemist`'s own) plus real `npm run build` steps added to all three CI workflows, verified against a real GitHub runner |
| ✓ | **Tile action log + `/world undo <player> <duration>`** | Admin-only, in-memory, 72h retention (`tile_log.rs`). Driven over a real two-client socket: a griefer breaks/places tiles, a witness who never sent the undo sees both reverted via the real `AREA_TILE_CHANGE` broadcast. The integration test itself had a real bug worth recording: `wait_for` discards whatever it scans past while it looks for a match, and the server sends the two revert broadcasts *before* the confirmation chat line — waiting on the confirmation first silently ate both real frames, so the later wait for them timed out. Fixed by waiting in the order the server actually sends them, not the order that read naturally. Wired into both the in-game admin dispatch and the server console (matching every other admin command's dual availability). 6 unit tests + 2 integration tests, all green |
| ✓ | **`--new <name>` generates into the platform's own Terraria world directory** | Reuses `worlds::directory()`/its space→underscore convention so a generated world sits beside every world Terraria itself made. Refuses a name already taken, and refuses a name carrying its own path (`/`, `\`, `..`) rather than joining it — an absolute-looking segment silently replaces the base in `Path::join`, which would otherwise let a bad name escape the world directory entirely. A real bug found writing the end-to-end test: the world directory itself may not exist yet on a machine that has never run Terraria, and the very first save failed outright (`reading .../Fork_Test_World.wld.tmp: No such file or directory`) until `--new` was made to create it. Verified by spawning the real compiled binary against a scratch `HOME`/`XDG_DATA_HOME`/`USERPROFILE` (never the machine's real Terraria directory): the world lands, a second `--new` under the same name refuses rather than clobbering the first |
| ✓ | **`cargo-fuzz` over the decoder found a real OOM within 90 seconds** | `decode_section_stream` checked `width`/`height` only for being negative, never for being too large, before `Vec::with_capacity`. A structurally-plausible 8-byte header claiming a 31232×12815 section drove an attempted ~400 million-`Tile` allocation and aborted the process — directly contradicting this project's own already-published "decode path cannot panic or over-allocate" claim. Fixed by bounding `tile_count()` to `SECTION_WIDTH * SECTION_HEIGHT`; the crash input is now a permanent regression test. CI runs both fuzz targets for a bounded 60s each against a curated real-input corpus |
| ✓ | **Section-ownership check on tile edits (vanilla parity)** | `MessageBuffer.cs`'s packet-17 handler rejects (relays but never applies) an edit for a section the client was never sent — `Player::sent_sections` already mirrored that exact state, just wasn't read at the edit path. A real bug caught while implementing: the fix also had to gate the "tile broke" side effects (item drops, altar/orb smashes, boss wake) on the same check, or a client could spam edits for unowned sections and still collect real drops for a tile never actually removed |
| ✓ | **`liquid.rs`'s `level()`: three heap allocations per call → zero** | Pinned first (a deliberately non-round 351-unit settle, so the spare unit's exact position is order-sensitive) — confirmed the pin catches a real regression before trusting it. Replaced three `Vec`s built from the same tile span, read twice, with two fixed-size stack arrays read once. ~35-45% faster at the medians on a real flooded 190×35 pool, 300 ticks; behavior unchanged, which the pin test is what proves |
| ✓ | **Worldgen Tier 2, first 3 of ~10 items: spider caves, gem caves, oasis — sited and wired into `build()`, all three now place real, non-zero counts on a real generated world** | Both `GemCaves` and `SpiderCaves` share `cave_flood::count` to find an unwalled pocket, and both ported a vanilla size-cap rejection (`found.tiles >= 300`/`>= 3500`) that saturated on essentially every candidate given this generator's `caves()` producing one large connected network rather than vanilla's isolated pockets — dropped the cap, capped each pass's own decoration footprint instead. `GemCaves` had a second blocker: vanilla's `rockCount == 0` check rejected ~98% of real candidates, because the 300-tile flood budget spends itself wandering open interior more often than it reaches an actual stone boundary tile in this topology — dropped, since the `y` sample range (`layout.rock + 30..`) already guarantees rock-layer siting on its own. `Oasis` needed two unrelated fixes: reordered in `build()` to run right after liquid settling (matching vanilla's own pass order — `Oasis` runs before essentially every decorative pass, including cacti, the literal end of vanilla's own list), and its window-scan check widened to accept `HARDENED_SAND` alongside `SAND` (this generator's own desert material curve puts hardened sand 6 tiles under the surface, well inside the window's depth-24 reach). Measured across seeds 999/4242/12345: spider caves 21/seed, gem caves 12/seed, oases 1/1/3 — all up from 0 before the fix. Real regression caught mid-review: `gem_caves.rs`/`spider_caves.rs` both lacked `oasis.rs`'s own small-world guard, panicking `next_range` on the tiny synthetic worlds several unrelated persistence tests build via the full `generate()` pipeline — fixed with the same guard shape, each with its own regression test. The remaining ~7 Tier 2 items (floating islands, living trees, pyramids, underworld ruins, jungle shrines, underground cabins, micro-biomes, glowing mushrooms) are untouched by this batch |
| ✓ | **Console `panel` command toggles the web panel on and off without a restart** | `panel::supervise` owns the panel's actual lifecycle for the life of the process — `abort()`s the running `JoinHandle` to stop it (dropping one only detaches, it does not stop the task), calls `panel::run` to start it — driven by an unbounded channel whose other end is a new `GameServer::panel_toggle` field, wired in via a `with_panel_toggle` builder method rather than a `new()` parameter (seventeen existing call sites across the workspace's tests construct a `GameServer` with no use for one). The startup path (`main`, `panel_enabled` from config) is unchanged — still fails loudly on a bind error before anything is serving; a *runtime* toggle failure only logs and leaves the panel off, since by then real players may be connected. Verified two ways: a unit test proving the console command sends exactly one pulse (and none at all when no toggle channel is wired, the case every other `GameServer` test is in), and a real end-to-end test toggling a real TCP listener on and off twice over an actual socket |
| ✓ | **Fixed real flakiness in `new_world_cli.rs`**: a fixed 12-second sleep raced a real subprocess's first autosave | Found running the workspace's full test suite (not the isolated file) — both of the file's tests spawn a real OS process each and run concurrently by default, and under contention a subprocess's first save did not land inside the fixed window. Replaced with a poll loop (`wait_for_file`, the same `deadline`-loop shape `gameplay.rs` already uses elsewhere in this project) that returns as soon as the file appears rather than assuming a fixed wall-clock budget is always enough |
| ✓ | **Web panel frontend tooling switched from npm to bun** | All three CI workflows' `actions/setup-node`+`npm ci`/`npm run build` steps replaced with `oven-sh/setup-bun`+`bun install --frozen-lockfile`/`bun run build`; `package-lock.json` removed from the repo in favour of `bun.lock`. Verified with a real clean install and build (`bun install --frozen-lockfile && bun run build`, plus `bun run check` for the type-check path) — this is also what surfaced the fix below, which a `cargo build` alone would never touch |
| ✓ | **A real Svelte 5 reactivity warning in `Status.svelte`, found building the frontend for real for the first time this session** | `state_referenced_locally`: passing the `session` prop directly to `watchStatus` looks, to the compiler, like it should have been a closure. It is not a bug here — `watchStatus` takes a plain string and reads it once, to open one socket for the component's whole lifetime, and `session` never changes after this component mounts — but the reasoning was undocumented. Suppressed explicitly (`svelte-ignore`) with a comment recording why, rather than left as a warning nobody had explained |
| ✓ | **Worldgen Tier 2, +1: jungle shrines + their chest, sited and wired into `build()`** | Hollow huts on jungle grass, one of five wood/brick materials, each holding a torch and a chest reusing `structures::biome_chest_loot`'s already-tested jungle branch. First real consumer of `StructureMap` (built earlier this session, never wired to anything): `build()` now threads one shared instance through every Tier 2 pass that sites a set piece, matching vanilla's session-global `GenVars.structures` so future set pieces (underground cabins, floating islands) will not overlap this one either. One deliberate siting deviation, disclosed in the module doc: this generator's `Layout` already knows the real jungle band before any pass runs, so shrines site directly against it rather than re-deriving vanilla's own coarse "whichever half the dungeon isn't on" approximation — strictly more accurate, not a loosening. A real bug caught by the pinning test: the chest was first placed one row too high, inside the hollow interior rather than on the shell's own untouched bottom row — vanilla's "floor gap" loops never actually reach that row either (transcribed as written, not corrected, matching this session's standing rule for dead vanilla branches), so that row is the real floor both here and in vanilla. Measured across seeds 999/4242/12345 on a real 4200×1200 world: 10, 10, 8 shrines |
| ✓ | **Worldgen Tier 2, +1: pyramids, sited and wired into `build()`** | A faithful, line-for-line transcription of `Pyramid()`'s stateful winding-tunnel-and-treasure-room carve — the solid triangular sandstone-brick mass, its wall lining, a direction-flipping wander tunnel, one lens-shaped treasure room with a chest/piles/banners/pots, and a long exit dig. Two real fixes: sites chosen directly against `layout.desert` rather than porting vanilla's `DunesBiome`-placed dune (an undiscovered `Biome`-class dependency this generator doesn't have — same class of gap underground cabins hits, below); and the site/entrance-shaft checks widened from plain `SAND` to also accept `HARDENED_SAND`, the same fix `oasis.rs` and `gem_caves.rs` already needed, for the same reason — every real site lands at exactly the depth this generator's own desert curve has already moved past loose sand. Two real transcription bugs caught mid-review, before any test ever ran: the digging phase's "have we broken out of the solid mass" check had above/below backwards, and wall-lining targets were on the wrong tiles (vanilla walls the row below and the next column over, not the tile being cleared itself) — both found by re-reading the source line by line after an initial free paraphrase went wrong, not by trial and error. A third bug caught *by* a test: the phase-two tunnel clearing used a full tile reset (`Tile::AIR`) instead of only toggling the active flag, which would have erased the wall lining a moment after placing it. Verified with a real flood-fill from a placed chest confirming a genuine several-hundred-tile connected tunnel network (not just "hollow tiles exist somewhere near a chest") — the first version of that check wrongly expected the tunnel to reach open air at the surface; re-reading `Pyramid()`'s own exit-tunnel phase against the failure showed it digs *further down*, not back up, matching real vanilla pyramids being fully buried, discovered by digging in rather than walked into. A fourth real bug, found by measuring generation time rather than reading: two pyramids could site close enough to overlap, since skipping `DunesBiome` (above) also loses the implicit `StructureMap` spacing that biome would have provided — `build_pyramid`'s own digging phases then have to walk a much larger *merged* mass, measured making one real seed's generation time balloon. Fixed with a disclosed 300-tile minimum-spacing check between sites. A separate, real per-seed timing outlier chased at length turned out **not** to be a code bug at all: full per-pass instrumentation across the entire `build()` pipeline showed `pyramids::scatter` itself consistently costing single-digit milliseconds even on the "slow" seed, and the same seed got *slower* on repeated identical runs — the signature of shared-disk I/O pressure (this session's shared machine hit 99% disk capacity mid-investigation), not a deterministic algorithmic issue. Measured across seeds 999/4242/12345 on a real 4200×1200 world: 3, 3, 2 pyramids |
| ✓ | **Worldgen Tier 2, +1: underground cabins, sited and wired into `build()`** | The `MicroBiome`/`Biome`-class pattern this project's own sizing table flagged as needing to be read before micro-biomes could reuse it — read in full (`CaveHouseBiome` 92 lines, `HouseUtils` 286, `HouseBuilder` 911, 7 per-material subclasses ~247), transcribed narrower than furniture-complete: real site-finding (`FindRoom`'s three-probe wall search, `GetRoomSolidPercentage`'s inclusion roll, `GetHouseType`'s material-count vote, `AreRoomsValid`'s lava/`StructureMap` check), all seven real materials (wood/desert/granite/ice/jungle/marble/mushroom, each its own tile/wall/beam/door/chest id), room carving, doors (reusing [`place_object`], already frame-correct), sloped stairs and a support beam between stacked rooms, and one chest reusing `structures::chests`' own depth-tiered loot (widened to `pub(crate)` for this). Disclosed and skipped: the furniture catalog (`FillRooms`' paintings/banners/pianos/bookcases), each material's `AgeRoom` weathering (dithered wall/tile decay, stalactites, hanging vines — real `Dither`/`Blotches`/`ActionStalagtite` DSL surface this generator has no equivalent for), the desert Bast statue and jungle sharpener/desert extractinator (each a single rare placement gated by a world-wide budget counter), and every secret-seed variant (the ~250-line `PotentiallyConvertToSeedHouse` reskin, rainbow/tenth-anniversary painting, `GenerateBiggerAbandonedHouses`' alternate room-chain generator) — the driving pass's own buried-chest loops are *not* re-ported at all, since `structures::chests`' own doc comment already covers and discloses that gap. Two real bugs found before this could place anything on a real world: `find_solid` (this generator's `WorldUtils.Find`+`Down`/`IsSolid` transcription) never checked the origin tile itself before stepping, so `create_rooms`' own `found == origin` rejection could never fire — fixed to check position zero first, matching `GenSearch`'s real loop, which also fixed a symptom worth naming: before the fix every seed placed exactly 10/10/10 cabins (suspiciously identical, hitting the retry cap every time); after, real seed-varied counts (3/6/2) emerged, the same signature every other Tier 2 pass's real measurement has. Second, caught by `a_generated_world_survives_a_save`: `carve_room` wrote each room's material to *every* tile first, then cleared `ACTIVE` on the interior without resetting `block` back to empty — harmless at runtime (nothing reads `block` on an inactive tile) but a real save/reload divergence, since the writer does not preserve a stale block value on a tile it treats as air. Fixed by never writing material to interior tiles in the first place. A third, caught by the same test: `connect_stacked_rooms`' stair ramp used literal platform tiles (id 19) with a bare `-1` frame sentinel — platforms are frame-important, the exact bug class already found and fixed once for doors — fixed by ramping with the room's own solid wall material instead (climbable the same way, just not drop-through-able), and vanilla's separate vertical-exit platforms above/below the stack are left as plain open gaps rather than guessing at platform framing this generator has no table for. Measured across seeds 999/4242/12345 on a real 4200×1200 world: 3, 6, 2 cabins |
| ✓ | **Town NPC combat: all 28 real vanilla `AttackType` NPCs now covered** (24 new, beyond the 4 already done) | Each transcribed directly from its own `NPC.cs` state-10/12/14/15 branch — real projectile/damage/speed/knockback per NPC, not a generic fallback. Two real findings: Dryad's ranged attack faithfully deals zero pre-scaling damage in vanilla (the `type==20` branch never sets the damage local) — transcribed as-is, not "fixed"; Cyborg picks one of three random projectiles per shot in vanilla, simplified here to always fire the rocket launcher variant, disclosed alongside every other per-entry simplification (hardmode upgrades, Pirate's burst/special, Truffle/Princess's "spawn near target" shape) in the module doc. A real pre-existing bug found verifying this, unrelated to the new work: `npc_data.rs` had Skeleton Merchant's (453) `town_npc` flag as `false`, structurally impossible since vanilla's `AttackType` set only ever applies to real town NPCs — fixed to `true`. Verified with a real end-to-end test, one fresh server per NPC for isolation: each of the 24 lands, gets a hostile spawned beside it, and either fires the correct projectile type or (melee) directly damages the target, over a real socket. Getting that test right surfaced two of its own bugs worth naming: waiting for the NPC to land before spawning its hostile let it wander off first (fixed by spawning both back-to-back, matching the existing Merchant test); reusing one server across all 24 iterations let entities pile up until landing-detection got unreliable (fixed with a fresh server per NPC) |
| ✓ | **The minecart track-switch bug, fixed** — a wired track switch (`Minecart.FlipSwitchTrack`) previously did nothing at all | It runs through a completely different vanilla code path than `hit_switch`'s own frame toggle (`HitSwitch`'s own tile-314 case only relays the current; it never touches the tile), which is exactly why nothing ever called the piece that does. `FrontTrack()`/`BackTrack()` are themselves just `frameX`/`frameY` in vanilla — no new tile field needed, only reading the two this project already has — and only ordinary (`_trackType == 0`) frames with a real stored back-track actually swap; the fix's first attempt misclassified which frames were switchable, caught by its own test failing before it was fixed properly with a real classification table. A second, more structural bug surfaced while wiring this in: `run_from`'s seed-tile exclusion (added so a timer's own circuit can't re-trigger itself) was unconditionally skipping the *starting* tile for every trigger type — correct for lever/switch/timer, wrong for a track switch, which has no pre-flip step and relies entirely on its own flood reaching it. Scoped the exclusion to skip everything except a minecart track. 4 new tests, each real-fail-before/real-pass-after |
| ✓ | **Closed 60 of the 66 remaining drop-table gaps** | Added `conditional_drops::chance_pools` (~20 gaps) for real `OneFromOptions(N, ...)` chance-gated pools — a genuinely different shape from the existing `one_from`, whose pools always succeed. Extended `Conditions` with `blood_moon`/`npc_from_statue` (a statue farm must not be able to grind blood-moon-exclusive drops — a real anti-farming check, not an oversight to skip), `eclipse`, `downed_mech_any`/`downed_all_mech_bosses` (two genuinely different gates — Pixie needs one mechanical boss down, the Reaper needs all three; conflating them was a real risk this session's own test now pins against), and `pumpkin_moon_wave`, with the pumpkin moon's real wave-scaled gate formula (`ItemDropDatabase.cs`'s `PumpkinMoonDropGatingChance`) worked out by hand and pinned in a test against the game's own arithmetic. `self.rng` turned out to already be in scope at the `drop_loot` call site — no new rng-threading needed, simpler than this row's own earlier estimate assumed. Found and fixed 4 real bugs in `tools/check_drops.py` itself along the way: it couldn't see match arms with guard clauses, couldn't see the new `chance_pools` function's `pool(...)` calls at all, couldn't see a runtime-computed `Conditional { item: N, ... }` struct literal (the pumpkin moon's own rate cannot be a compile-time constant), and its `one_from` regex broke on a `rustfmt`-wrapped multi-line array — all four were hiding real, already-correct code as false gaps, not reporting new ones. The 6 gaps left are each individually traced: 5 are `RemixSeed`-only branches (verified directly against source — the same class as the already-known Duke Fishron item-157 case) genuinely out of scope, and npc 44's is the pre-existing documented nested-fallback-chain shape. SkeletonHead's `RedHatSkeletron` boss-loot gap (task #44) was traced to its real mechanism — an AI-slot flag set once at Skeletron's spawn from the summoning player's headgear, not a live per-drop equipment check as originally assumed — but left unimplemented: it needs NPC-spawn-path work outside a drop-table pass's actual scope |

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
| ✓ | Round-trip a real *played-in* world through real Terraria (GAPS §31 never exercised the preserved path) |
| ✓ | Tile action log + `world undo <player> <duration>` |
| ✓ | `--new <name>` to generate into the world directory |

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

**Correction to the correction, found reading `CaveHouseBiome.cs` for underground cabins.** "None
of their helpers" was wrong: `HouseUtils.cs` (the room-finding half of `CaveHouseBiome.Place`,
which every `Biome`-class house genuinely needs, not just underground cabins) calls
`WorldUtils.Find` with a `Searches.Chain`/`Searches.Down`/`Left`/`Right`/`Up`/`Conditions.IsSolid`/
`HasLava`, plus `Shapes.Rectangle` and `Actions.Count`/`Actions.TileScanner`/`Modifiers.IsSolid` to
find and measure candidate rooms. Not the full 5,253-line framework — this is a genuinely narrow
slice, five or six specific operations, not the whole composable pipeline — but a real one, not
zero. The "~200-line tracker covers everything" headline needs a footnote, not a full reversal:
`StructureMap`/`ShapeData` are still what most of Tier 2/3 needs, but at least this one item (and
plausibly others among the 15 micro-biome classes, which share `CaveHouseBiome`'s `MicroBiome` base
and may share this same room-finding helper — not yet checked) needs a purpose-built stand-in for
`WorldUtils.Find`'s "walk from a point in a direction until a condition holds" and a rectangle
tile-count/histogram scanner too, not just the overlap tracker. Matching this project's own stated
preference (a narrow, purpose-built implementation over porting the general framework — the same
choice `cave_flood.rs` already made for its own flood-fill rather than a generic search), these
would be small, targeted functions, not a DSL port — but they are new surface area the original
"zero" claim said would never be needed.

| | Item | Vanilla pass(es) | Real size | DSL? | Difficulty |
|---|---|---|---|---|---|
| Tier 2 | Floating islands + houses | `FloatingIslands`+`FloatingIslandHouses` | ~1,950 (incl. `CloudIsland`/`SnowCloudIsland`/`DesertCloudIsland`/`CloudLake`) | none | Hard — big, plain transcription like Tier 1 |
| Tier 2 | Living trees + walls | `LivingTrees`+`LivingTreeWalls` | **≥1,409, corrected from ≥274** — `GrowLivingTree` alone is 638 lines (`WorldGen.cs:28255-28893`), plus `GrowLivingTree_CanPlaceLeaves` (22), `GrowLivingTree_HorizontalTunnel` (236), `GrowLivingTree_MakePassage` (272) and the still-unmeasured `GrowLivingTreePassageRoom`+`LivingTreeWalls` pass on top of the 241-line driving pass | none | **Hard, corrected from Medium** — on par with floating islands, not a mid-sized item |
| Tier 2 | Spider caves | `SpiderCaves` | ~142 | none | Easy–medium |
| Tier 2 | Gem caves | `GemCaves` | 45 | none | Easy |
| Tier 2 | Pyramids | `DunesAndPyramidLocations`+`Pyramids` | **~478, confirmed close to the original ~640 estimate** — `DunesAndPyramidLocations` 63 + the driving `Pyramids` pass 110 + `Pyramid()` itself 305 (`WorldGen.cs:27948-28253`, a stateful winding-tunnel carve with a mid-tunnel treasure room) | none | Medium — genuinely one of the few accurately-sized remaining items |
| Tier 2 | Underworld ruined houses + hellforges | `Underworld`+`Hellforges` | **≥1,036, corrected from ≥273+2 unmeasured** — the terrain-filling head of `Underworld` is already covered by this generator's own `structures::underworld`; the real new work is its tail call to `AddHellHouses` (550 lines, `WorldGen.cs:32431-32980`), which itself calls `HellFort` (435 lines) and `HellFort_AttemptToCrumbleWall` (still unmeasured), plus the small 51-line `Hellforges` pass | none | **Hard, corrected from Medium** |
| Tier 2 | Jungle shrines + chests | `JungleShrines`+`ChestsInJungleShrines`+`LihzahrdTemplePart2` | ~201, confirmed ≈191 real (156+35) once landed — see Done | none | Medium — **done this session, see Done table** |
| Tier 2 | Underground cabins — **done, see Done table below** | `UndergroundHousesAndBuriedChests` | ≥1,556 real vanilla lines read in full (`CaveHouseBiome` 92, `HouseUtils` 286, `HouseBuilder` 911, 7 materials ~247) | `StructureMap` only, in the end — every `WorldUtils`/`Actions`/`Modifiers` call `HouseBuilder`'s own core (siting, carving, doors, stairs, beams, one chest) needed turned out replaceable with plain loops over this generator's own tile API, the same pattern every other Tier 2 pass already uses | Landed narrower than furniture-catalog-complete — see the Done row for exactly what's transcribed vs disclosed-skipped |
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
  not-the-bees, remix, no-traps, "get fixed boi", Don't Starve), and neither `README.md` nor this
  plan ever stated whether they were in scope. Asked rather than assumed, the same way Steam P2P
  was flagged rather than defaulted — **in scope, but deprioritized behind ordinary-world parity**.
  Tracked as a `README.md` 🔴 row and in the backlog below, not a disclosed exception like Steam
  P2P.
- The Lihzahrd Altar fix (already shipped, tested across 40 seeds) uses a different mechanism than
  vanilla: a 7-offset retry loop inside `structures::temple()` with a `debug_assert` fallback,
  where vanilla has a dedicated unconditional pass (`LihzahrdAltar`, 32 lines) placing the altar at
  precomputed `GenVars.lAltarX/lAltarY` with no failure path. Functionally verified and passing —
  this is a structural note for a future hardening pass, not a bug: our retry loop could in theory
  hit its assert on a pathological layout that vanilla's precomputed-coordinate approach can't.

| | Item |
|---|---|
| — | Worldgen Tier 2, item by item, now that `StructureMap`/`ShapeData` exist (see Done) |
| — | Worldgen Tier 3, item by item — the ~25 easy items first |
| — | A bot that starts with nothing and kills Moon Lord — Tier 1's own acceptance test, now that traps and `SmoothWorld` have landed |
| — | Town NPC happiness, price effects, moving out — combat is done (see Done: "Town NPC combat: all 28 real vanilla `AttackType` NPCs now covered") |
| — | Journey mode |
| — | 3 missing events (Slime Rain, Party, Lantern Night) |
| — | 6 remaining drop-table gaps, down from 66 (see Done: "Closed 60 of the 66 remaining drop-table gaps") — each individually traced to a remix-seed-only branch or the pre-existing documented nested-fallback-chain shape (npc 44), none fixable without scope this project has already deliberately excluded |
| — | Pets/mounts: already client-authoritative and working (`a_pet_summon_item_equipped_in_the_misc_slot_relays_to_another_player`, `gameplay.rs`) — this row is a documentation gap, not open work. Minecart tracks: the track-switch bug is fixed (see Done, above); nothing else in this area is known to be broken |
| — | Skeletron's `RedHatSkeletron` vanity-set condition — needs player equipment state, which lives in `server.rs`, not `conditional_drops.rs`. Found while fixing drop tables, correctly left for whoever's in `server.rs` next |
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
8. Item 7 above ("zero Tier 2/3 passes call that pipeline") was itself re-measured and found
   incomplete: `HouseUtils.cs`, which every `Biome`-class house (underground cabins, and plausibly
   several of the 15 micro-biome classes too) genuinely needs for room-finding, does call a real —
   if narrow — slice of `WorldUtils`/`Actions`/`Modifiers`/`Shapes`. See "Correction to the
   correction" above.
