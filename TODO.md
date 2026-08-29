# TODO: the v0.0.1 roadmap and the single backlog

This file IS the plan. Work that is known and deferred, not hidden, organised as the release
roadmap. The former `plan.md` (the pre-roadmap working ledger) and `GAPS.md` (the seven-pass audit
trail) are folded into this file and removed; their full text lives in git history, and everything
in them that is still live appears below. There is no separate gaps file.

## What v0.0.1 means

A fully working, stable, production-usable, vanilla-identical replacement for the Terraria 1.4.5.8
dedicated server. The one deliberate exception is worldgen: the remaining secret-seed
generation-content differences and remaining micro-biomes are deferred to v0.0.2. That narrow
deferral does not excuse unrelated inaccuracies. Versioning collapses to v0.0.x from here: the next
release after v0.0.1 is v0.0.2 (the worldgen release), and the old v0.1.0 label is retired.

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

## Phase 1: the v0.0.1 core campaign

Integrated, parity-first, per-subsystem. The from-scratch audit produces a findings ledger per
subsystem, and fixes fold into that subsystem's single visit (split the file, clear its panics,
apply the audit fixes, tidy) so heavy files are churned once. Single-owner hot files
(`game/server.rs`, `world/worldgen/mod.rs`) take one change at a time.

### Lane A: split `game/server.rs` by responsibility (in flight)

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

**C1, the known tail**, each with a fail-then-pass test:
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
- D1: teach the recipe generator to enumerate the ~566 loop-built decraft recipes so shimmer
  decraft is complete (a behavioural table change, distinct from the Lane H port).

**C2, the from-scratch audit** in about six consolidated lanes against the decompiled source, real
clients and captures, producing a ledger; fixes fold into the subsystem visits. Seed list carried
from the audit trail, still unverified or unchecked: drop *rates* (presence is checked, `one_in`
values and chain ordering only partly), AI behavioural parity per style measured against the game
in motion (coverage is complete, behaviour is asserted per-style not compared), liquid/wiring/
housing compared against the real game in motion, boss phase transitions and stall-ability, NPC
spawn *pool* composition, fishing, golf, dyes, painting and the cosmetic layer, plus the known
minor divergences: `SendSection` does not sync the section's NPCs the way vanilla does at
`NetMessage.cs:2732`, no `Main.SyncAnInvasion` on packet 6 (cosmetic), and section batching
stricter than `Tile.isTheSameAs` (correct output, more bytes).

**C3, the spawn lane**: adopt the fork's spawn-parity module structure once Xekep affirms the CLA
and the posted punch-list is fixed (or take the punch-list over if the fork goes quiet).

**C4**: expand the golden/deterministic vanilla-derived tests that CAN run per-commit in CI; the
live differential against a real `TerrariaServer` is a Phase 2 qualification step, since decompiled
or installed game material can never ship to hosted CI.

### Lane D: protocol classification, zero unknown IDs (in flight)

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

### Lane H: finish the codegen port (in flight)

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
tick under the 16.67 ms budget, stable memory, and a clean world save under load. One extended
multi-hour soak with a boss event under load runs before the first release only. The human
fresh-world Moon Lord playthrough is strongly expected, waivable only if the automated and
differential evidence is otherwise complete; anything found becomes a test. Final verification:
`just check` green across the six targets, fuzz and soak green, zero production panics on
hostile or environmental paths, zero unknown protocol IDs, the admin overhaul verified against a
real client and Playwright, no confirmed performance regressions. Then tag v0.0.1.

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
