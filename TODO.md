# TODO: the v0.0.1 roadmap and the single backlog

This file IS the plan. Work that is known and deferred, not hidden, organised as the release
roadmap. The former `plan.md` (the pre-roadmap working ledger) and `GAPS.md` (the seven-pass audit
trail) are folded into this file and removed; their full text lives in git history, and everything
in them that is still live appears below. There is no separate gaps file.

## What v0.0.1 means

A fully working, stable, production-usable, vanilla-identical replacement for the Terraria 1.4.5.8
dedicated server. There are two deliberate, documented exceptions. The first is worldgen: the
remaining secret-seed generation-content differences and remaining micro-biomes are deferred to
v0.0.2. The second is one place where vanilla is provably wrong: vanilla's liquid levelling rounds
with `Math.Round` and so slowly creates water (a faithful port was built as a test probe and
measured doing exactly that, +2 units on a thrash-prone pool), which on a long-running server would
flood worlds; terrustia keeps a conserving model that levels and settles correctly but does not
reproduce the duplication. The divergence is locked by the `faithful_port_converges_but_is_not_
conservative` test that records both measurements. Neither exception excuses unrelated
inaccuracies. Versioning collapses to v0.0.x from here: the next release after v0.0.1 is v0.0.2 (the
worldgen release), and the old v0.1.0 label is retired.

**The v0.0.1 gates:** parity completion plus a from-scratch re-audit; the error-handling and
data-safety sweep; the `server.rs` architectural split; a zero-unknown-ID protocol classification;
the admin overhaul (namespaced permissions, audit log, moderation toolkit); Windows ARM64 in the
release matrix; the codegen port finished in Rust; town NPC happiness and shop pricing; and a
255-player qualification run per release candidate. A human fresh-character Moon Lord playthrough is
a strongly-expected but waivable qualification step.

**Town NPC happiness and shop pricing, added to the gates 2026-08-31.** The C3 audit found no
happiness, no price multiplier, no `ShopHelper` equivalent and no pylon happiness threshold
anywhere in the repo: an absent subsystem rather than a bug, which is why no earlier pass reported
it and why it had never been named here at all. Under a vanilla-identical bar an absent vanilla
system is a gap like any other, so it went into scope and was built from
`Terraria.GameContent.Personalities/` rather than deferred.

**It has since landed, and this entry described the world before that.**
`crates/terrustia-proto/src/happiness.rs` is 734 lines transcribing `ShopHelper.ProcessMood`
(`ShopHelper.cs:99-178`) with its own test module, wired through `server/mod.rs:1424-1490` and
reported by `/happy` (`console.rs:810-830`), with `examples/happiness_cost.rs` measuring what it
costs. The multiplier is taken once per chat, on `SetTalkNPC` (`dispatch.rs:1685-1695`), the same
moment vanilla takes it. Anything still outstanding under this gate needs naming from a fresh
reading of the code, not from this paragraph. Left here rather than deleted because the gate's history is the point: a system missing
from both the code and the plan is invisible twice over, and so is one this file still calls
missing after it exists.

## Phase 0: preconditions (complete)

Recorded for the trail: the Container CI musl-toolchain fix landed and the workflow is green; the
~20 stranded agent worktrees were surveyed (nothing unique) and removed; every MASTER-FIXPLAN P0
item was verified against `main` with the gaps ticketed into the lanes below; the audit-wave branch
is retired in favour of one topic branch per lane off `main`; and the fork-collaboration review
(spawn parity, VegaKernel/Xekep) was posted to PR #1 with the CLA-affirmation ask.

## Phase 1: the v0.0.1 core campaign (complete)

Integrated, parity-first, per-subsystem. The from-scratch audit produced a findings ledger per
subsystem, and fixes folded into that subsystem's single visit (split the file, clear its panics,
apply the audit fixes, tidy) so heavy files were churned once. Single-owner hot files
(`game/server.rs`, `world/worldgen/mod.rs`) took one change at a time.

All eight lanes (A-H) below landed. The from-scratch re-audit (Lane C's C2) then ran as its own
wave and is recorded under Lane C. The lane detail is kept below as the built record; the only
open Phase 1 item is C3 (adopting the fork's spawn module), which is blocked on the fork, not on
us. What remains before the tag is Phase 2 qualification.

### Lane A: split `game/server.rs` by responsibility (done)

The 16,058-line, 108-panic-site elephant becomes a `game/server/` directory: `dispatch` (the
`handle_packet` receive match), `tick` (the loop and phase orchestration), `panel` (panel-request
handlers), `console` (`run_command`/`run_admin_command`/`run_console`), `systems` (per-system
update calls), and a thin `mod.rs` keeping the `GameServer` state and actor entry. Zero behaviour
change; production panics on caller- or environment-triggerable paths cleared in the moved code;
the single-writer actor preserved; suite, clippy and fmt green per extraction.

### Lane B: error handling and data safety

- Clear every non-test `.unwrap()`/`.expect()`/panicking index/truncating cast from paths the
  outside world can trigger; replace each with propagation and an operator-facing message. The
  `net::listener::bind` mapping for `os error 28` is the pattern: keep the error kind, add advice.
- Capped backoff in the accept loop on persistent `accept()` failure, so descriptor exhaustion does
  not become a hot loop.
- ENOSPC, read-only filesystems and vanished directories handled on every write path: world save
  and autosave, rotating backups, the admin store, the setup-wizard config. Write to a temp path
  and rename into place everywhere; never lose the last good save to a half write. A failed
  autosave warns and retries: console and panel on the first failure, and after a few consecutive
  failures an in-game broadcast that saves are failing and progress is at risk.
- From the P0 verification: a game-side reaper for stale non-Playing slots older than
  `handshake_timeout` (the connection-level 64-frame deadline is escapable by sending frames), and
  the persistence refusal in Lane C1 below.

### Lane C: parity completion and the from-scratch re-audit

**C1, the known tail (done)**, each landed with a fail-then-pass test. Kept as the built record:
- HC8: nebula headcrab applies buff 163 (needs a player-buff channel on the AI `Effects`/`Outcome`).
- HC9/HC10: Solar Sroller multi-bounce and Sand Shark sand-swim collision physics in `npc.rs`.
- The four AI-state drop gaps: Skeletron's RedHatSkeletron set (5624/5625/5626/5628/5737 when
  `ai[3] == 1`), Pumpking's weapon pool, Mourning Wood 327, Mothron 477 item 1570; needs a
  conditions field threaded into drop resolution.
- L2: liquid destroys furniture (`tileLavaDeath`/`tileWaterDeath` table via codegen; a partial
  table would kill the wrong tiles, so it stays a no-op until the table exists).
- Trapdoor and tall-gate wiring: real `ShiftTrapdoor`/`ShiftTallGate` domain logic.
- B13: Empress of Light damage re-derived from vanilla's seven case blocks.
- BI8: slime re-targets only during an active (flag3) hop.
- Server MINORs: NPC-buff broadcast scope, summon combat books (-11/-17), the teleport guard on
  player controls, the chest-open (packet 80) rigged-input check.
- Persistence: `wld.rs` refuses out-of-order section pointers with an error instead of an empty
  blob, with a corrupt-`.wld` fixture.

**C2, the from-scratch audit (done)** ran in six consolidated read-only lanes against the
decompiled source, tracing behaviour to root cause on both sides, and produced the consolidated
ledger (about 12 blockers, 50 majors, 30 minors; the recurring shape was systems that ran and
produced output but the wrong amount, which is why earlier passes read past them). Four cross-cutting
root causes were named and fixed once each: the single-integer spawn identity (R1), difficulty
scaling applied in the wrong layer (R2), the server originating damage vanilla computes client-side
(R3), and Outcome flags produced but never consumed (R4). The fixes then landed as a wave, each
finding with a fail-then-pass test citing the vanilla line:

- **R1** multi-slot spawn identity (`Spawn.ai`), the shared prep both AI lanes built on.
- **World runtime (FIX-1a/1b/1c/1d)**: growth and hardmode spread cadence (L3-01, the corruption/
  hallow-never-creep blocker); liquid evaporation, merge origination, pacing and border margin;
  BFS wire flood with per-colour pumps and teleporters; wind and weather; crystal-shard and
  chlorophyte regrowth; the CheckMech split (per-colour skip, momentary detonator) and wired-light
  toggling.
- **Persistence (FIX-2)**: the Lunar Pillar save/load blocker (a free endgame skip) and the trailing
  round-trips (town rooms, pressure plates, bestiary, journey powers, travelling merchant).
- **Boss AI (FIX-3/3b/3c)**: the Moon Lord finale and True Eyes (dead code, boss unkillable-as-
  designed), the mech-boss dawn despawn, Wall-of-Flesh lasers, the Martian Saucer phases, the Moon
  Lord fixed per-part attack timeline, and the full boss minor tail.
- **Combat and damage (FIX-4)**: the extraUpdates N+1 slow-motion blocker, the knockback curve, and
  the R2/R3 difficulty-scaling and damage-origination corrections.
- **Spawning and town (FIX-5/5b/5c)**: bound-NPC progression gates, arrival item sets, the eight
  blocked townsfolk, the real biome-classification box, weighted spawn pools with per-type rates and
  caps, the pre-Skeletron Dungeon Guardian gate, housing-through-doors, town regen, and pylon travel
  validation against vanilla's five checks.
- **Protocol and worldgen (FIX-6)**: the AreaTileChange field-merge blocker (ordinary building was
  deleting a world's liquid and paint), the netmodule gaps, and the dungeon-loot and worldgen pass
  ordering.
- **Security and infra (FIX-7)**: the panel account-delete reach-check blocker, terminal-escape
  sanitisation, and the CI and config hardening.

**Phase 3 re-audit (done)**: a read-only pass over everything the wave changed, on the project's own
lesson that a fix is a change and deserves the same suspicion. It found one major (an Old One's Army
finishing-kill clamp that over-applied to every wave) and four minors, all fixed.

**Deliberate seams, measured not skipped**: the liquid-levelling conservation divergence (see "What
v0.0.1 means"); and C7-01, the Nebula Brain floater-hurry, which needs the NEBULA_FLOATER charge-up
projectile AI (ai_style 102) that is not built yet, documented at its drop site. A handful of small
narrowings are disclosed in-code where they were made (for example the liquid cycles round-robin,
the CheckMech cross-frame refusal modelled per-trip, and a couple of cosmetic gaps).

Known minor divergences that remain by design: `SendSection` does not sync the section's NPCs the
way vanilla does at `NetMessage.cs:2732`, there is no `Main.SyncAnInvasion` on packet 6 (cosmetic),
and section batching is stricter than `Tile.isTheSameAs` (correct output, more bytes).

**Area-of-interest culling on player movement and projectile syncs**, a deliberate and measured
divergence. Vanilla relays a player's movement to every other player (`NetMessage.SendData(13)`
excludes only the sender) and a client's projectile syncs the same way. terrustia routes both through
`broadcast_near`, the loaded-section cull that NPC sync already used, so an update goes only to
players whose sections could contain it. What a distant client loses is the fullscreen map marker
moving smoothly rather than in steps; it cannot draw the player or the projectile at that range, and
a skip budget (four for projectiles, matching the game's own rule, thirty for movement because it
arrives every tick rather than every sixth) is what stops anything distant freezing outright.

Kept because it was measured, not because it sounded right. Two 255-player runs at the pre-mitigation
queue depth, differing only in whether the cull was wired up, at matched NPC load: without it, 14
`outbound queue full` drops, 245 of 255 clients held, and the outbound queue running literally full
at 73,465 of 73,472; with it, zero drops, 255 of 255 held, and a peak of 38,713. `connection.rs`'s
own comment had predicted exactly this fix and left it for whoever owned the broadcast next.

**C3, the spawn lane**: adopt the fork's spawn-parity module structure once Xekep affirms the CLA
and the posted punch-list is fixed (or take the punch-list over if the fork goes quiet). This is a
restructure of code we already own and have now audited and fixed, not a gap: `game/spawn.rs` is
2,340 lines transcribed from `Spawner.SpawnAnNPC` with 34 tests. Nothing about the release waits on
it.

**C5, the third from-scratch audit (2026-08-30) and its fix wave.** Eleven read-only lanes over the
whole tree, ten parity lanes against the decompiled source plus one over-engineering lane kept
separate so simplification proposals could not contaminate parity work. Generated tables were in
scope and re-derived with independent parsers rather than sampled. It found **9 blockers, 45 majors
and 44 minors**, recorded in `.scratch/audit-2026-08-30/LEDGER.md` with the fix assignments in
`FIXPLAN.md` beside it.

The recurring shape was the same one C2 named and is worth restating, because six passes had read
past it: **nothing crashed**. Every blocker was a system that ran, produced output, and produced the
wrong amount. Gel's registrations matched a generator regex that silently returned nothing. The
liquid wake queue terminated only because its cap discarded roughly 97% of the work it was given.
`damage_bonus` was written in 13 production sites and read in none outside `#[cfg(test)]`, so every
boss enrage multiplier was inert while the tests that read the field passed.

Three of the four data-table blockers and both liquid causes were generator or design faults that
`just regen` and the test suite faithfully reproduced. Two further defects were found during the
fixing rather than the audit, both by building a better instrument rather than reading harder:

- **Bone (item 154) was unobtainable**, so every bone recipe was uncraftable. Its only two
  registrations are `ByCondition` lines (`ItemDropDatabase.cs:1162-1164`), and `tools/check_drops.py`
  could not see them: its argument slices made every `ByCondition` rule in the game source invisible,
  and its epilogue explicitly excused treasure bags and master-mode drops, which is where a fourth
  blocker was hiding. Fixing the checker surfaced Bone within the hour.
- **Nothing here ever woke the tile above.** Vanilla does it twice (`Liquid.cs:947-966` and
  `:1518-1521`); without it a column draining from the bottom never learns its floor has gone. This
  was the larger of the two causes of water hanging in mid-air, and the audit did not have it.

Both are the argument for the instrument campaign below.

**The secret seeds, quantified for the first time, and it is a disclosure problem as much as a
parity one.** An audit lane counted the sites rather than estimating them:

- **`Main.getGoodWorld` (For the Worthy): 101 sites in `NPC.cs`, 79 of them in NPC AI.** Three are
  implemented (the Wall of Flesh pace, the lunar pillar surface clamp, `DESTROYER_SEGMENTS_GOOD`),
  five more are explicitly disclosed as absent at their sites, and **roughly 71 are silently
  absent**, including all eleven of the Eye of Cthulhu's and all nine of the Twins'.
- **`Main.remixWorld`: 85 sites, zero consumed.** The seed is detected, persisted, and
  **advertised to clients** (`world.rs:788`, `F::RemixWorld`), while no AI reads it.
  `world/hardmode.rs:602-603` still says "this project has no remix seed", which is stale twice over.
- **`WorldGen.Skyblock.lowTiles`: about 20 sites, zero consumed.** Detected and persisted, unread.

The parity gap is ordinary deferred work. The **disclosure** gap is not: the server currently tells a
client it is running a remix world and then does not behave like one, and a stale comment tells a
reader the seed does not exist when it does. Under this project's own rules a narrowing is disclosed
at its site, so either these seeds are wired up or their absence is stated where a reader will meet
it, and the advertisement to clients is reconsidered. That is a v0.0.1 decision, not a v0.0.2 one,
because it concerns what the server claims about itself.

**C4 (done)**: expanded the golden/deterministic vanilla-derived tests that CAN run per-commit in
CI; the live differential against a real `TerrariaServer` remains a Phase 2 qualification step, since
decompiled or installed game material can never ship to hosted CI.

### Lane D: protocol classification, zero unknown IDs (done)

One authoritative, machine-readable per-ID table for the full 0..=162 surface (direction,
client/server send, live/dead/legacy, dedicated-server applicability, Steam/social/host-only,
terrustia recv and send implementation, source evidence, tests), validated against the actual code
by the evolved `tools/packet_audit.py` so drift is a red check, with `docs/packet-coverage.md`
generated from it. Classifications carried from the audit trail: `DevCommands` (94) deliberately
unhandled (a public server that honours it can be rewritten by anyone); host migration
(`SpectatePlayer` 150, `HostToken` 161) not applicable to a dedicated server; `ShopOverride` (104)
unimplemented and classified rather than faked.

### Lane E: the admin overhaul

- **E1, namespaced permissions**: per-command leaves with dotted families and wildcards
  (`server.kick`, `server.ban`, `server.mute`, `world.time`, `panel.console`; `server.*`, `*`),
  extending the existing string-set store. Ships a four-tier ladder: `default` (self-service
  only), `moderator` (kick/mute/look, panel view), `admin` (bans, world and panel management, no
  group-editing and no raw console, so it cannot self-escalate), `owner` (`*`). Includes the
  registration path a future plugin uses to declare its own permissions; the plugin API itself is
  post-v0.0.1.
- **E2**: the coarse match table in `run_command` becomes specific namespaced checks.
- **E3, panel roles and management**: a moderator logs in and does only what its permissions allow
  (today it cannot log in at all); `/api/console` behind its own high permission; a
  permissions-management view extending the existing groups/accounts views; the raw stdin console
  stays fully privileged, unchanged.
- **E4, the audit log**: a dedicated append-only file beside the world (issuer, timestamp, target,
  reason for ban/unban/kick/mute/register/group-change/claim), independent of the admin TOML store,
  with issuer and timestamp added to `Ban` as current-state, a read surface (console command and
  panel view), and size-based rotation with generous configurable caps.
- **E5, moderation**: real `mute` (chat suppression, duration, persistence, permission-gated,
  audited) plus temp-mute escalation, shadow-mute (the muted player sees their own messages echoed,
  staff see them flagged) and per-account chat cooldowns. All off by default: a fresh server feels
  exactly like vanilla until the operator opts in.

Out of scope for v0.0.1, planned after: regions and spawn protection, warps, item/tile/projectile
restrictions, general policy machinery, server-side characters, stronger anti-cheat (today the
server trusts client health/mana and has no stack validation or ban lists; the audit trail's
detail is preserved under Phase 3).

### Lane F: plaintext-transport hardening

Document the plaintext transport plainly; keep Argon2; guarantee passwords are never logged; add
login-attempt throttling (per-IP and per-account exponential backoff with jitter, in-memory, reset
on success, no lockout, so brute force is impractical and account-name lockout-griefing is
impossible); never treat the Terraria UUID as proof of identity. From the P0 verification:
constant-time comparison for the claim token (both the console and panel paths still compare with
plain `!=`) and an fsync in the admin store's save.

### Lane G: platforms

Add `aarch64-pc-windows-msvc` to the CI and release matrices (six official targets), built and
smoke-tested on GitHub's native `windows-11-arm` runner (falling back to cross-compilation if
runner availability disappoints); keep `riscv64gc` compiling as a compile-only target; keep the
matrix affordable.

### Lane H: finish the codegen port (done)

The eight remaining Python generators (`gen_drops`, `gen_projectiles`, `gen_banners`, `gen_buffs`,
`gen_angler`, `gen_shimmer`, `gen_town_names`, `gen_travel_shop`) become `terrustia-codegen`
modules, each verified byte-identical against its committed table; `just regen` points at the
codegen crate and the last `tools/gen_*.py` are deleted. The three checker scripts stay in Python
by decision (`check_drops.py`, `check_recipes.py`, `packet_audit.py`); note they need the
decompiled tree, so they run locally at qualification time, never in hosted CI (`just check-data`).

Two of the eight, `gen_shimmer.py` and `gen_travel_shop.py`, initially failed byte-identical for a
reason unrelated to the port: past hand-edits (`78d07de`, `65f4be3`) had updated `shimmer.rs`'s
decraft doc paragraph and added `travel_shop.rs`'s BlackCounterweight/YellowCounterweight source
comment and regression test straight to the committed tables, without updating either generator's
`emit()` to match, in violation of "generated tables are never hand-edited". Reconciled by teaching
both generators to emit exactly what is committed rather than touching either table.

### Cross-cutting through Phase 1

- **Dense-file splits**, paired with panic-clearing and idiomatic cleanup in the same visit.
  Measured in **production lines**, with `#[cfg(test)]` bodies excluded (re-measured 2026-08-31;
  the earlier list counted total lines and so listed ten files that were never dense):
  `game/server/systems.rs` (6,573), `game/server/dispatch.rs` (4,943), `game/server/mod.rs`
  (2,788), `panel/mod.rs` (2,139, no tests at all), `world/wiring.rs` (1,690 production against
  1,627 test, not the 2,575 total this list used to quote), `game/spawn.rs` (1,644),
  `world/wld.rs` (1,364), `game/ai/mod.rs` (1,237), `game/npc.rs` (1,186), `world/wld_save.rs`
  (1,053). The generated proto tables are excluded: codegen output, never hand-edited, size is
  fine. So are `crates/terrustia/tests/*.rs`, which carry no `#[cfg(test)]` and are test files
  entire (`gameplay.rs` would otherwise rank first at 6,907 lines).

  Off the list, all under 1,000 production lines: `world/worldgen/traps.rs` (989),
  `world/worldgen/structures.rs` (950), `term.rs` (929), `world/world.rs` (916), `game/army.rs`
  (847), `game/buffs.rs` (823), `world/worldgen/mod.rs` (670), `game/ai/town.rs` (654),
  `game/ai/critter.rs` (654), `game/npc_ai.rs` (555). Note the three at the top of the new list
  are the hot files the guardrail below sequences last, so the list is now ordered by size and
  worked in roughly the opposite order.
- **Feature-cohesive layout and a periodic hygiene scan** (requested 2026-08-31, explicitly lower
  priority than parity work and never allowed to derail it). The dense-file list above is organised
  by size; this is the layer above it, organised by subject. A reader who wants to know how Martian
  Madness works should find it in one place rather than tracing it through `event.rs`,
  `invasion.rs`, `moons.rs`, `spawn.rs`, `systems.rs`, `ai/hardmode/saucer.rs` and `npc_params.rs`.
  The `game/server/` split by responsibility is the precedent that worked.

  Two guardrails, because a reorganisation that churns a file without making anything clearer is
  pure cost. First, size alone is not a reason: a long file implementing one algorithm transcribed
  faithfully from vanilla is not a problem, and moving transcribed code away from the shape of its
  source makes every future parity check harder. Second, the hot files (`game/server/systems.rs`,
  `dispatch.rs`, `game/ai/`) are under near-constant edit, so any move touching them is sequenced
  after the wave that owns them, never during.

  The periodic scan is the durable half: files past a line threshold, modules with too many inbound
  dependencies, exact-body-hash duplicate helpers (name matching is not enough, an earlier pass
  found eleven copies of one helper that turned out to be **two** subtly different helpers whose
  merge would have been a behaviour change), doc comments naming a file or function that no longer
  exists, and `pub` items with no reader outside their own module. Most of that is a script; the
  last one is better served by `unreachable_pub` on `crates/terrustia` alone, never on
  `terrustia-proto`, whose public surface is the whole point of the crate.

  **The first scan already found four things worth acting on** (2026-08-31, full detail in
  `.scratch/audit-2026-08-30/HYGIENE.md`):

  1. **The dense-file list above was measured on the wrong number. Fixed 2026-08-31.** It counted
     total lines including `#[cfg(test)]` bodies, so ten of its seventeen entries were never dense
     (`world/wiring.rs` is 1,690 production against 1,627 test, not 2,575) while the three largest
     production files in the tree were absent from it entirely, `game/server/systems.rs`,
     `dispatch.rs` and `game/server/mod.rs`, the last being the file Lane A meant to leave thin.
     The list above now counts production lines only.
  2. **Stale prose.** About 190 references to `game/server.rs` survive the Lane A split, across code
     comments, `docs/*.md` (one of them a link that 404s) and AGENTS.md. Many sit in files under
     active edit, so this is a single sweep to run once the parity lanes land, not piecemeal.
  3. **`docs/generated-tables.md` documents a workflow that no longer exists**: a runnable command
     block invoking ten `tools/gen_*.py` scripts that Lane H deleted.
  4. **Table provenance, fixed in part.** `npc_data.rs`, `tile_object.rs` and `placed_items.rs`
     (21,768 lines together) described themselves as generated while having **no generator**, and
     none is on rule 7's list. So `just regen` never touched them, yet a reader seeing "GENERATED"
     would either refuse to correct a wrong number or expect a regeneration to preserve their fix.
     `npc_data.rs` was in fact hand-edited on 2026-08-31, correctly, in a file whose header forbade
     it. That is the same trap Lane H hit with `shimmer.rs` and `travel_shop.rs`. The three headers
     now say plainly that no generator exists and corrections are made in place with a citation.
     **Writing real generators for them remains open** and is the proper fix.

  **The owner's own example, Martian Madness in its own folder, is declined with reasons.**
  `game/ai/mod.rs::run` is a `match npc.stats.ai_style` mirroring vanilla's `NPC.AI()` switch arm for
  arm, and every module under `game/ai/` is named for the style it implements. A `martian/` folder
  would cut `invasion.rs` in half (the probe is Martian, the Flying Dutchman is Pirate) and
  `charger.rs` too (the drone is Martian, the Solar corite is the Lunar event), leaving the tree half
  indexed by style and half by event, and moving transcribed code away from the shape of its source.
  `game/ai/army/` exists only because the Old One's Army roster happens to share a style band, which
  does not generalise. What Martian Madness actually needs is the subsystem map (now written down)
  and the roughly 140 lines of orchestration currently scattered across five ranges of `systems.rs`,
  which the `systems.rs` split below delivers properly.

  Three splits proposed, in order: `panel/mod.rs` (2,139 production lines, no in-file tests, not
  transcribed code) by resource, **before** Lane E adds its views; `game/{housing,arrivals,rescues}.rs`
  into `game/town/`, **before** the newly-gated happiness and pricing work needs a home; and
  `systems.rs` into `game/server/systems/` along its already-contiguous feature bands, **after** the
  parity lanes let go of it. Explicitly do not touch: `wiring.rs` (one algorithm from `Wiring.cs`),
  `wld.rs` and `wld_save.rs` (sequential readers in the file format's own order), `npc_params.rs`
  (banded by AI style on purpose, which is exactly why its Martian constants sit 2,300 lines apart),
  `game/spawn.rs`, and all of `game/ai/`.
  Scheduled in-campaign but not a release gate; its worst failure is a boot convenience that
  already falls back to a logged manual-port-forward message.
- **The autosave snapshot is the most expensive thing an idle server does, and it is on the tick
  thread** (found by the owner running a real server, 2026-08-31). A `phase=snapshot phase_us=12833`
  warning on a world with **two NPCs and no players**: 77% of a tick's budget spent copying the
  world for a save.

  Measured on a fresh 4200x1200 world before writing anything down. It scales with how much the
  world has changed since the last save, not with the tick: 30 to 36 sections and 2.0 to 3.1 ms at a
  15-second autosave, 68 sections and 6.3 ms at the default 300, and 12.8 ms on a loaded world with
  a town. **Not a regression**, which was the first hypothesis and the wrong one: the same probe on
  the pre-wave build copies 36 sections in 3,059 us, slightly worse than now. It has always cost
  this. It was invisible because the phase timer used to bill the work to the wrong bucket, and
  because a comment in `save_world_in_background` claimed every save after the first was "already
  150-200 us" and was never re-measured. That comment now carries the real table.

  **Half done.** The spike is off the tick: a save is now armed rather than taken, the tile copying
  is drained three sections a tick by `tick_snapshot_drain`, and the save fires from
  `try_fire_pending_save` only on a tick where `World::snapshot_pending()` is zero. The
  point-in-time question answered itself: `set_tile` re-marks every section it touches, so a
  section copied early and then edited goes back on the list and is copied again, and a buffer whose
  pending count reaches zero is bit-identical to the live world at that instant. Assembled across
  ticks, delivered at one. `record_town_npcs`/`record_lunar_pillars`/`record_journey_powers` run on
  the firing tick and never the arming one, or the object tables would be newer than the tiles,
  which is the tear this exists to avoid. A 600-tick deadline bounds the wait, `drain_ticks` on the
  `world snapshot taken` line makes a deferral visible, and the escape logs a `warn!`.

  Measured by running both builds against the owner's own 4200x1200 world for 185 s each, autosaving
  every 20 s with nobody connected, nine saves apiece:

  ```text
                        before                          after
    snapshot_us   454 2116 3548 2632 213 750 413    295 308 757 124 194 121 335 224 36
                  871 378                           (max 757, was 3,548)
    sections      6 6 11 9 5 10 9 5 7               0 0 0 0 0 0 0 0 0
    drain_ticks   -                                 1 2 1 2 2 2 2 3 2
    worst tick    3,826 us                          2,218 us
  ```

  `sections_copied` is zero on every save: the firing tick copies no tiles at all, only the 20 us of
  side and object tables. What is left on the worst tick is a *drain* tick, and the per-section cost
  it pays turns out to vary 17x with page residency (213 us for 5 sections, 3,548 for 11), which is
  why the cap is three and not the eight a warm benchmark suggested.

  **What that leaves on the table, and it is the bigger half.** The drain spreads the work; it does
  not remove it. The unit of "changed" is 30,000 times bigger than the change: instrumenting
  `set_tile` on an idle fresh 4200x1200 world over six consecutive 20-second autosaves found 150 to
  260 tiles actually changing per window, and that marked 24 to 37 sections, an amplification of
  about 5,000x. Roughly 200 real edits drag about 990,000 tiles into the copy.

  So the follow-up is to track changed **tiles** rather than the sections they sit in, with a cap
  and a fall back to the section bitset (still maintained in parallel) once it overflows, for the
  bulk cases: an explosion, a Clentaminator sweep, hardmode generation, a mass-wire operation.
  Worldgen is already exempt, because `track_dirty` is false during it. Measured on the owner's real
  4200x1200 world with `examples/snapcost`, scattered so no two picks share a cache line and through
  the dearer public `tile`/`set_tile` pair, so an upper bound:

  ```text
      200 loose tiles      1 us     what an idle window actually changes
      4,000 loose tiles   30 us
      one section         16 us     warm; about 90 to 100 us cold on a live server
      all 168 sections  2,710 us
      a refresh with nothing dirty  21 us  (side tables + object tables, the floor)
  ```

  About 7.5 ns a loose tile against 16 us a section, so one section is worth about 2,100 loose
  tiles and an idle window changes about 7 tiles per marked section: three to four orders of
  magnitude of headroom. An idle save's tile copying would fall from 480 us (30 warm sections) to
  1 us, and the floor would become the 21 us fixed cost.

  A `Vec<u32>` of tile indices beats the per-tile bitset that was the alternative. Capped at 65,536
  entries it is 256 KB of reserve and about 800 bytes in use, against 630 KB of bitset that is
  always resident and has to be scanned end to end on every save: at the 28 GB/s this machine
  copies tiles at, that scan alone is about 22 us, more than copying the 200 tiles it would find.
  Duplicates in the list (one tile written repeatedly) cost 7.5 ns each and are bounded by the cap.

  The two compose rather than compete: with a tile list the pending count reaches zero on the
  arming tick in the idle case, so the save fires at once and the drain costs nothing, and the drain
  stays as the safety net for exactly the bulk case that overflows the list back to sections.

  **Ruled out, so nobody re-derives it:** an equality check in `set_tile` to skip rewrites of a
  tile's existing value. The same instrumentation counted `noop_writes=0` on every one of the six
  windows. Every gameplay write is a real change, so the check would cost a `TileStore::get` per
  write and save nothing.

- **Performance discipline**: maintain the benchmarks, measure meaningful changes, reject confirmed
  regressions on CPU, memory, latency, startup, saves and joins, and keep the instrumentation. The
  deep optimisation campaign comes after the feature waves, not now.

  **Ruled 2026-08-31: no merge may be measurably slower than `main` on a hot path.** Parity work
  does add real work, because vanilla does things this server was skipping, so the rule is not "never
  cost anything": it is that a lane measures the cost, optimises until it is negligible against the
  16.67 ms per-tick budget at 255 players, and reports the number. Correctness is never traded away
  to hit it; the implementation is what gets optimised, not the behaviour.

  The standard was set by the case that forced the rule. Moving `biome_at` ahead of the spawn rate
  roll, which parity requires, cost 82 us per scan, or **20.8 ms per tick at 255 players, over the
  entire frame budget on its own**. Vanilla pays nothing for this because the client runs
  `SceneMetrics`. A `BiomeCache` brought it to 345 us, about 2% of budget, and
  `crates/terrustia/examples/biome_scan_cost.rs` reproduces both numbers so the claim stays checkable.
  Where a fix is locally slower and correct, as the walled liquid cases are now that water completes
  its fall instead of stranding partway, that is documented with its measurement rather than hidden.

  Shorter 255-player soaks run as frequent regression checks and are never treated as definitive
  while the machine is contended; the full quiet-machine 30-minute run is reserved for milestones and
  release candidates. The README's comparison table against the official server is refreshed on the
  same cadence, so its numbers never drift from the build they describe.
- **Docs**: de-slop `AUDIT.md` and `docs/*.md` (em dashes and the usual tells); this file replaces
  `plan.md` and `GAPS.md`.

## Instrumenting for the next audit

Runs after the C3 fix wave lands and **before** the coverage-gap pass, because most of what it
builds does that pass's job mechanically and deterministically, and because a checker that cannot
see a class of defect makes a clean run indistinguishable from a real one.

The C3 audit found nine blockers by hand. Not one of them crashed. Gel's registrations matched a
generator regex that silently returned nothing; Bone was registered only through two `ByCondition`
lines and dropped from no NPC at all; the liquid wake queue terminated only because its cap was
discarding roughly 97% of the work it was given. Every one was a system that ran, produced output,
and produced the wrong amount, which is precisely what a reader looking for something broken reads
past, six passes running.

Worse, **the tools built to catch this were part of why it was missed**. `check_drops.py`'s epilogue
excused treasure bags and master-mode drops, the exact two categories a blocker was hiding in, and
its `[^)]*` argument slices made every `ByCondition` rule in the game source invisible, which is
where Bone lived. Fixing that checker surfaced Bone within the hour. The same shape had already
appeared twice: three release bars that nothing evaluated, and a `cpu_us` double-count in the
instrument built to measure the fourth.

So the lesson is not "audit harder". It is that these defects have mechanical signatures, and the
leverage is in tools that find a *class* rather than a person finding an instance. Ranked by value
over effort; the first three are roughly a day each.

1. **Mutation-test the verifiers.** The highest-leverage item here and the only one that checks the
   checkers. If `check_drops.py` cannot see a `ByCondition` rule, then deleting a `ByCondition`
   drop from the committed table does not make it fail, and that is directly testable: corrupt or
   remove entries programmatically, run the checker and the suite, and assert every mutation is
   caught. A surviving mutant is a blind spot by definition. This would have found the Bone gap
   without anyone knowing Bone existed. `cargo-mutants` covers the Rust side; the tables need a
   small script. Run at qualification the way `just fuzz` is, not per commit.

2. **Reachability as a gate, from an independent implementation.** "Every item vanilla can produce,
   can we produce, and vice versa" is a set difference, and it collapses Gel, Bone, the lunar
   fragments, the four missing treasure bags, the 102 unreachable items, the 57 master-mode items
   and the 80 missing projectiles into one query. Most of it exists: the audit lane wrote parsers
   over `ItemDropDatabase.cs` and a full interpreter of `SetupRecipes`, and the C3 wave repaired
   the checker. What is missing is that it must exit non-zero and be wired into `just check-data`.
   **It must stay a second, independent implementation.** Re-running the generator and diffing
   against its own output proves nothing; that is the same tautology as asserting `BUFF_COUNT`
   against the array the same generator sized.

3. **A dead-write lint. Built; the open work is triage, not construction.** `damage_bonus` was
   assigned in 13 production sites and read in none outside `#[cfg(test)]`. So were `wet`, slime's
   `ai[3]`, `TreeOutcome::fleeing` and `FairyOutcome::wants_treasure`: one class, five findings,
   two of them blockers, and the same root cause (R4) as four blockers in the C2 wave. Rust's own
   `dead_code` misses it because a test read counts as a read.

   This entry used to estimate "about a hundred lines over `syn`". It exists:
   `crates/terrustia-codegen/src/bin/deadwrite.rs`, ~700 lines with its own tests, an `ALLOWED`
   list carrying a written reason per excused field, and a `just check-dead-writes` recipe wired
   into `just check-data`. It reached zero findings on 2026-08-31: of the 18 it was reporting,
   `confused`, `dryad_ward` and `tipsy` were wired to real consumers, `ArmyState::stand`,
   `ArmyState::champion_down` and `dungeon_side` were deleted as redundant state, `angler_quests`
   and `golf_score` now feed the rebroadcast the way `NetMessage.cs:1156-1160` does, and the rest
   went onto `ALLOWED` with a traced reason each. Keeping it at zero is the standing work.

4. **Invariants in the soak, not just thresholds.** Liquid conservation is a property: the total in
   a sealed world does not change however many passes run. FIX-B found its blocker by measuring
   exactly that across nine release sizes, where the existing tests used 40x30 worlds, pools of at
   most 180 tiles and zero fall distance. The same shape applies to tile-state legality, NPC
   position bounds, and drop distributions over many kills.

5. **Generate the golden pins.** The Frost Legion and Pirate invasion sizes were swapped *and a
   test asserted the swap*, so a correct implementation would have failed the suite. That is
   structurally impossible when a constant transcribed from vanilla is codegen output rather than
   hand-typed: a wrong pin cannot survive being derived from the source it exists to pin.

6. **A flaky CLI test is an unmeasured test, and A/B-ing one needs a matched rebuild.**
   `new_world_cli` fails whenever the world file never lands, timing out on the full 120s. It
   reproduces only when the whole *test binary set* has just been relinked, which any lib-file
   change causes; `touch`ing `main.rs` rebuilds the binary alone and hides it completely. Measured
   the wrong way, `main` looks clean 7 runs out of 7 and whatever branch is under test looks guilty
   3 out of 3. Measured with a matched relink, `main` fails 3 runs in 4, so it is pre-existing and
   any A/B that does not relink both trees identically will confidently blame the wrong change.
   Eliminated by measurement, each: the network (5 failures in 8 with the update check and UPnP
   both off), autosave (15 saves in 15s at `autosave_secs = 1`, first at 1.108s), leaked processes
   and held ports (none at the instant of failure; every test port is unique), first-execution
   code-signature validation (0.42s), and machine load. It is a heisenbug: a diagnostic that dumps
   the child's output on failure stops it reproducing. The next attempt should have the spawned
   server log to a file unconditionally, so a failing run leaves evidence without the act of
   looking changing the run. Full write-up in `.scratch/audit-2026-08-30/FLAKE-new-world-cli.md`.

**Explicitly not on this list: another audit pass by reading.** The C3 pass found 99 findings and
still missed Bone and the absent upward wake in `Liquid.Update`, both of which turned up during
fixing, and both of which were found by building an instrument rather than reading harder. Reading
passes are good at discovering a class and poor at exhausting one, and they are not repeatable, so
a clean one carries little information. Use them to find the class, then automate the class.

## Phase 2: release qualification

Per release candidate: the manual differential against a real `TerrariaServer` (`probe`/`verify`;
hosted CI can never hold the game); `just check-data` against the decompiled tree; and the
255-player qualification run, separate from per-commit CI, with an objective bar: 255 real headless
clients join a full-size world and hold 30 minutes, zero server panics, no disconnect storm, p99
tick under the 16.67 ms budget, peak server RSS under 1 GiB, and a clean world save under load. The
human
fresh-world Moon Lord playthrough is strongly expected, waivable only if the automated and
differential evidence is otherwise complete; anything found becomes a test. Final verification:
`just check` green across the six targets, fuzz and soak green, zero production panics on
hostile or environmental paths, zero unknown protocol IDs, the admin overhaul verified against a
real client and Playwright, no confirmed performance regressions. Then tag v0.0.1.

**The bar is enforced, not narrated.** `tools/soak_scale.sh` judges every clause of it and exits
non-zero on any failure. That is worth writing down because for a long time it did not, and three of
the five clauses were unmeasurable or unmeasured:

- **p99 tick had no data source.** `cpu_us` reached the log only through the stall branch, which
  fires when the *machine* is held off the processor, so the number quoted as tick cost was the cost
  of whichever tick happened to coincide with a hitch. A thirty-minute run produced five samples, all
  stall-coincident; a clean run produced none. The per-window line that carries the real figure was
  `debug` while the server defaults to `info`, and the warning for a genuinely over-budget tick named
  the same quantity `worst_us` rather than `cpu_us`, so the one line that mattered was the one the
  harness could not read.
- **Client retention counted the wrong thing.** The soak client discarded its send result and let a
  read error break only its inner drain loop, then returned success unconditionally, so a client
  whose connection the server had closed slept out its hold and exited zero. Runs where the server
  dropped 218 of 255 clients were recorded as `255/255 connected and held`.
- **Memory was printed, not judged.** The curve went to the output for a reader to interpret, which
  is how a run climbing to 689 MiB was recorded as a pass.

**Memory: peak RSS under 1 GiB at 255 players.** A number rather than "stable", because the adjective
is what let a multi-gigabyte figure stand unchallenged. What that figure was measuring turned out to
be the soak client failing to read: capped near 2,100 events a second, its receive window closed, the
server's outbound queues backed up behind it, and the kernel eventually killed connections with
`ETIMEDOUT`, which reads exactly like a server shedding clients under load. With the client draining
properly, a 255-player half-hour holds around 140 MiB and peaks near 200. The runs that exceed the
ceiling are the ones taken while the test box itself is contended, and the external-stall count in
the same output is what distinguishes the two. A thirty-minute run cannot separate a slow leak from
burst working set, so the bar tests the ceiling and leak detection stays with the extended soak.

**The extended multi-hour boss soak is waived for v0.0.1** and carried to the next release. Its
distinct value over the thirty-minute run is leak detection over a long horizon, and the shorter run
now enforces a memory ceiling rather than printing a curve, which covers the failure mode that
matters most for a first release. Recorded as a decision rather than skipped quietly, because the
lesson of this qualification work is that an unenforced stated bar is indistinguishable from a met
one.

## Phase 3: after v0.0.1, in order

1. **v0.0.2, the worldgen release**: the seven remaining secret seeds' generation content (Not the
   Bees, Drunk World, Remix, Celebrationmk10, "get fixed boi", Don't Starve, Skyblock; Don't
   Starve alone touches 53+ scattered branch points across nearly the whole of `WorldGen.cs`, and
   the others are comparable or larger) and the 7 of 15 remaining micro-biomes (each needs a
   genuinely separate subsystem: a trappable-chest mechanism, a second tree-growth engine, a
   wandering-tunnel shape, and so on). The six deferred drop-table gaps ride along: five need
   Remix's own generation content, the sixth is the documented npc-44 nested-fallback shape.
2. **Regions and spawn protection**: the first built-in addition.
3. **The plugin API**: Rust first (permissions land in v0.0.1, so the model exists; regions and the
   admin interfaces settle into real use cases first), then C# once the host API has proven
   itself. Event hooks, commands, permissions (reusing E1's registration path), opaque handles,
   validated operations, lifecycle, storage, unload/reload semantics; never expose
   `&mut GameServer`; the ordinary server stays self-contained with no .NET runtime.
4. **The optimisation campaign**, after the feature waves: eliminate unnecessary work, then
   algorithms and data structures, then memory/allocation/layout (the ~10 MB idle figure is an
   aspirational research target tracked per component, not a promise), then CPU hotspots, then
   safe parallelism, then explicit SIMD (AVX2/AVX-512, NEON, RVV, runtime dispatch, scalar
   fallback everywhere), then generated-assembly inspection, then hand-written assembly where it
   measurably wins. Profile first; every accelerated path proven bit-identical against the
   reference with fuzzing, Miri and sanitisers where applicable.
5. **Server-side characters**: much later; its own properly designed storage and auth system.

**Deferred with reasons written down** (carried from the audit trail and the tick-rate research):
- **Higher tick rates (120/240/480)**: demoted to an off-by-default experimental research note.
  Unmodified clients are hard-pinned to 60 Hz (fixed-timestep accumulator), run their own NPC AI
  prediction and advance world time themselves, so a faster server mostly manufactures desync; the
  mechanical "configurable multiple" alone is a large scattered-literal sweep where every miss is a
  silent pacing bug. Picked up only when a concrete need appears.
- **One-time join credentials**: a short-lived single-use token issued through the panel and typed
  into Terraria's normal password prompt, so a reusable password never crosses the plaintext
  protocol. Does not encrypt the connection and does not stop an active MITM; it makes an observed
  credential worthless.
- **TShock-style built-ins** (warps, restrictions, richer moderation) so normal administration
  never requires plugins.
- **Seed-identical world generation**: 219-372 engineer-days by the standing estimate; the oracle
  is built and green (`docs/worldgen-parity.md`). Generation is complete and playable; it is not
  Terraria's world for a given seed, and closing that is its own campaign.
- **Steam P2P (friend invites)**: needs the Steamworks SDK under AppID 105600 and a licence
  decision against the AGPL. Protocol-level Steam support is already complete; a Steam-launched
  client connecting by IP is byte-identical to any other.
- **Operational polish**: config reload without a restart; a general log-file sink with rotation
  (the moderation audit log in Lane E is separate and is in v0.0.1).
- **The extended multi-hour boss soak**, waived for v0.0.1 (see Phase 2) and due for the next
  release. It is the only run long enough to tell a slow leak from burst working set, which the
  thirty-minute qualification run explicitly does not attempt.

## TUI and hosting polish (opportunistic, never derails release work)

- **Smooth-gradient boot logo via a terminal image protocol.** The 5-row block-glyph banner cannot
  match `docs/assets/banner.svg`'s gradient as text. Plan: bake `docs/assets/boot-logo.svg` (the
  text-free transparent source) to a 2x transparent PNG offline via `rsvg-convert` and
  `include_bytes!` it; hand-roll the iTerm2 OSC 1337 and kitty graphics emitters (skip sixel);
  detect via the `supports-terminal-graphics` env heuristics gated behind `Palette::is_enabled()`,
  treating `TMUX` as unsupported; fall back image -> 256-colour `banner()` -> plain. No new
  dependencies: a ~15-line base64 encoder plus two emitters; explicitly not `viuer`. Gotcha:
  cursor advance after the image differs between iTerm2 and kitty.
- **Hanging indent for wrapped log lines** (manual wrapping at the terminal width).
- **Narrow-terminal awareness** for the boot block layout.

## Dependency pruning (decided; the record stays visible)

The default server build resolves **171 external crates**. **Decision (2026-08-29): stability over
crate count.** The only cut being made is hand-rolling UPnP away from `igd-next` (-31 crates, no
new dependencies). Everything else that could be cut is a working, in several cases
already-verified subsystem and is deliberately kept: a mature dependency is worth more than the
crates it costs, and rewriting one resets verification that has been earned.

Measured with `cargo tree -e no-dev --workspace --no-dedupe` (plain `cargo metadata` returns a
feature-unified maximal graph and undercounted `igd-next` by 20 crates; do not trust it for
feature-gated ownership). Exclusive ownership: `igd-next` 31 (hand-roll, decided), `ureq` 13 plus
`tempfile` 2 (keep: rustls/ring is the irreducible core of a secure `terrustia update`),
`rust-embed` 8 (keep), `crossterm` 7 (keep: Windows console path), `toml` 6 (keep, deferred),
`tracing-subscriber` 4 (keep, deferred), `argon2` 4 (keep: the one KDF this workspace does not
hand-roll), `axum` 23 with `rust-embed` (keep: the panel is Playwright-verified and a transport
rewrite resets that to zero). The combined `igd-next` + `rust-embed` + `axum` lever (171 -> 96) is
recorded in git history with the full crate list; it was weighed and declined.

**The UPnP hand-roll**: only `search_gateway` and `add_port` are used; UPnP-IGD control traffic is
plain HTTP/1.1 over the LAN with raw `IP:port` literals, so none of `url`/`idna`/ICU is needed.
A small pure module (SSDP M-SEARCH datagram, LOCATION parse, a tolerant tag scanner for
`serviceType`/`controlURL` and SOAP faults, URL splitting, the `AddPortMapping` envelope, a minimal
HTTP/1.1 exchange) with the socket I/O as a thin shell; the public API stays exactly
`pub async fn attempt(listen: SocketAddr)`. All parsing pure and unit-tested, since the live path
needs a real router.

## Release

Tag v0.0.1 when the Phase 1 gates are met and Phase 2 passes. Do not hold the release for
post-release nice-to-haves.
