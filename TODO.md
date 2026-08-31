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
release matrix; the codegen port finished in Rust; and a 255-player qualification run per release
candidate. A human fresh-character Moon Lord playthrough is a strongly-expected but waivable
qualification step.

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
and the posted punch-list is fixed (or take the punch-list over if the fork goes quiet).

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

- **Dense-file splits**, paired with panic-clearing and idiomatic cleanup in the same visit:
  `world/wiring.rs` (2,575), `panel/mod.rs` (1,746), `world/world.rs` (1,636), `world/wld.rs`
  (1,511), `game/spawn.rs` (1,432), `world/worldgen/traps.rs` (1,404), `game/ai/mod.rs` (1,365),
  `world/worldgen/mod.rs` (1,281), `world/wld_save.rs` (1,233), `world/worldgen/structures.rs`
  (1,197), `game/npc.rs` (1,179), `game/npc_ai.rs` (1,164), `term.rs` (1,154), `game/ai/town.rs`
  (1,150), `game/buffs.rs` (1,136), `game/ai/critter.rs` (1,123), `game/army.rs` (1,088). The
  generated proto tables are excluded: codegen output, never hand-edited, size is fine.
- **The one dependency cut**: hand-roll UPnP to drop `igd-next` (see the dependency section below).
  Scheduled in-campaign but not a release gate; its worst failure is a boot convenience that
  already falls back to a logged manual-port-forward message.
- **Performance discipline**: maintain the benchmarks, measure meaningful changes, reject confirmed
  regressions on CPU, memory, latency, startup, saves and joins, and keep the instrumentation. The
  deep optimisation campaign comes after the feature waves, not now.
- **Docs**: de-slop `AUDIT.md` and `docs/*.md` (em dashes and the usual tells); this file replaces
  `plan.md` and `GAPS.md`.

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
