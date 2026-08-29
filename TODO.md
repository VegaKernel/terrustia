# TODO

What is still worth doing, in priority order.

This file is intentionally the **current remainder**, not a history log. `GAPS.md` records what was
found and fixed and is therefore full of old intermediate states by design. A finished item does
not stay here as a zombie task just because nobody remembered to delete the paragraph.

Last reconciled against `master`: 2026-08-29.

---

## In flight

### 1. Finish spawn-location postchecks and aquatic composition

Deep water is a real spawn medium (`987c005f`). The normal non-Space spawn-location path now keeps
the initially sampled random tile separate from the physical `SpawnTileY` floor, validates
solid/safe-wall state on the random tile, searches downward only to the normal spawn area's bottom,
checks the early safe rectangle and left-shifted 2x3 clearance against the resolved physical floor,
and preserves retry-vs-abort semantics for the 50-candidate search. The coordinate split/helpers
landed in `e2f4a201` and `5452d1bd`; runtime wiring landed in `f61a7989`.

The all-player 2088x1172 visibility gate and the two-cell overhead-liquid rule now consume that same
physical floor coordinate in runtime rather than the earlier random air cell. Mounted-player hitbox
geometry is still not modelled, so the visibility helper is exact for the ordinary 20x42 player
hitbox only.

Source selection is also explicit. `spawn_source.rs` handles Platform / Metal Bars / Planter Box
look-through and Terraria 1.4.5 Conveyor Belt semantics (`f3de39f8`). `SpawnSource` preserves the
physical floor, resolved source row and source block (`ab8ace0d`), and its no-source fallback was
corrected to retain the physical floor rather than an unrelated pre-floor random coordinate
(`c8227369`). The current water runtime still calls the compatibility `block()` projection; using
`resolve()` is now about exposing source coordinates/metadata to downstream rules, not repairing a
fallback bug.

Ocean work remains deliberately split from that migration. The documented 250/380 edge geometry is
pinned (`b3665d92`) and a source-aware water-pool API exists (`30b736c2`), but it is **not wired into
`try_spawn` yet**. Current upstream material is not phrased consistently enough to pretend the
250/380 environment precheck and the later Ocean-water source condition are the same operation:
the official spawning documentation describes both 250- and 380-tile environment routes, while
current tModLoader `SpawnCondition.Ocean` exposes the 250-tile water condition. Keep runtime on the
coarse path until that exact 1.4.5.x distinction is recovered.

The remaining work is narrower and should stay explicit:

- Dungeon post-selection has a tested helper (`259d2654`) for the six Dungeon Brick types and a
  non-zero wall, but runtime still needs source-backed `ZoneDungeon` and exact `spawnWallType`
  semantics before it can be enabled honestly;
- Mowed Grass / Mowed Hallowed Grass has a tested event-sensitive 1/10 helper (`259d2654`), but the
  runtime caller still needs to carry the existing `GameServer` Slime Rain and invasion state into
  spawn context. Do not introduce a second copy of those event states just to wire this rule;
- the special Space spawn-location path is not modelled. Vanilla does not require the same ground /
  early safe-area path for Space attempts, so the new normal `spawn_location` helper must not be
  advertised as universal;
- scope / binocular / Sniper Rifle spawn-range modifiers are not modelled; `e11fafef` pins only the
  normal rectangle. The current tModLoader preview still exposes `NPC.sHeight = 1080`; an official
  wiki talk-page report claims 1.4.5 may have changed the fixed spawning height to 1200, but that is
  not yet sufficiently source-backed to change Terrustia's constants;
- the generic Surface-water critter source is not implemented. Goldfish (55) was removed from the
  dry Ocean pool rather than left spawning on land, so it needs its real critter-pool route and the
  world/time/weather context that route actually depends on;
- Ocean runtime still uses the project's coarse `Biome::Ocean` classification. The 250/380 helper
  and source-aware pool are substrate only until the exact environment-vs-source distinction above
  is resolved;
- a single `Biome` enum cannot express overlaps such as a Crimson/Jungle Ocean, so water-source
  priority in hybrid areas is not yet a complete SceneMetrics replacement;
- the exact Blood Feeder/Blood Jelly relative probability was not independently recovered from a
  1.4.5.x oracle. The current water pool preserves Terrustia's previous 1:1 relative weighting rather
  than pretending that weighting has been certified;
- the full Pink Jellyfish / Shark / Squid Ocean relative table is likewise not oracle-derived. Sea
  Snail (220) is present with the independently verified Squid:Sea Snail 3:1 relation (`17d688b4`),
  while the older Pink/Shark/Squid relative weights remain an explicit approximation;
- `GROUND_SCAN`, `find_ground()` and the old combined chosen-point wrapper remain in `spawn.rs` only
  for legacy unit tests after `f61a7989`; they no longer drive `try_spawn`. Remove or relocate that
  dead compatibility surface so future parity work cannot mistake the old fixed-30 helper for the
  production floor-search rule.

Next priority: clean the obsolete fixed-30 spawn helpers, then wire the already-tested Mowed Grass
postcheck using the server's existing event state. Keep Dungeon and Ocean-380 behind their remaining
source-oracle questions rather than filling gaps with guesses.

### 2. Real Terraria client against Terrustia

The protocol tests are strong but still share `terrustia-proto` on both ends. The independent blind
spot is therefore unchanged: a real Terraria GUI client has not yet been recorded reading what this
server writes.

Use the existing recorder and replay path documented in `docs/real-client.md`:

```sh
cargo run --release -- --record capture.trcap
cargo run --release -p terrustia --example replay -- capture.trcap
```

The useful session is not merely "connected once": join fully, walk far enough to stream new
sections, edit tiles, use a chest, talk to a town NPC/shop, exchange damage, disconnect and rejoin.
Check the resulting capture in so future CI can replay bytes that this repository did not produce.

### 3. Ordinary enemy drop coverage

`tools/check_drops.py` remains the source of truth. The last audited figure was about 123 ordinary
enemies with missing loot while boss coverage was complete.

Most remainder is conditional: condition chains and option pools that `tools/gen_drops.py`
deliberately refuses to flatten. Add them to `conditional_drops.rs` in small reviewed batches and
re-run the checker after each batch. Do not replace a known incomplete table with a generated table
that is silently wrong about conditions.

### 4. Seed-identical world generation

The current generator builds a complete playable world and the save format round-trips through the
real game's reader/writer. It does **not** reproduce Terraria's world for a shared seed.

That is a separate, much larger parity project. Progress and the oracle live in
`docs/worldgen-parity.md` and `world/worldgen/manifest.rs`; continue pass-by-pass rather than mixing
seed parity into ordinary playability fixes.

---

## Structurally unverified

These are not known broken. They are known **unchecked**, which is precisely how many of the old
`GAPS.md` defects survived for so long.

### 5. AI behaviour parity

`ai/mod.rs` has routines wired for the used styles, but "a routine exists" is not behavioural
parity. Boss phase transitions, long-running fights and stalls need measurement against the game,
not just per-style unit coverage.

### 6. NPC spawn-pool composition

Spawn rates and caps are modelled and measured. The exact *composition* of what appears under each
combination of biome, depth, time, progression, medium and event is not comprehensively diffed
against vanilla. The now-fixed bound-NPC lottery and water-medium bug are examples of why that
distinction matters.

### 7. Drop probabilities and ordering

Presence of registered drops was audited. `one_in` probabilities, stack ranges, conditional-chain
ordering and expert/master variants have not been comprehensively re-derived and compared.

### 8. Liquid, wiring and housing in motion

These systems have tests, but have not been exercised side-by-side against the real game for long
sessions. Static unit parity does not prove timing, interaction ordering or eventual convergence.

### 9. Features still largely unexamined

Fishing mechanics, golf, dyes, painting, pets, mounts, minecart tracks and much of the cosmetic
layer still need dedicated passes rather than assumptions based on packet coverage.

---

## Deferred by project decision

### 10. tShock-style feature menu

Deferred until the core server work is in better shape. The admin foundation already exists. The
usual candidates remain regions/world protection, warps and homes, item/tile bans and server-side
characters.

### 11. Steam friend-invite joins (P2P)

Ordinary Steam-launched clients connecting by IP use the same TCP protocol and need no special
packet handling. Friend-invite P2P is a transport-layer feature and remains deferred behind an
off-by-default implementation and an explicit licensing decision for the proprietary Steamworks
SDK in this AGPL project.

---

## Closed: do not reopen from stale notes

The following items used to be TODOs and are already implemented on `master`:

- **Normal physical-floor spawn-location pipeline**: random-candidate validity is separate from the
  physical `SpawnTileY` floor; normal floor search is bounded by the spawn area's bottom rather than
  a fixed 30 tiles; early safe range and left-shifted 2x3 clearance use the resolved floor; and
  visibility/non-Water postchecks preserve no-retry abort semantics at that floor. Helpers landed as
  `e2f4a201` / `5452d1bd`, runtime wiring as `f61a7989`.
- **All-player post-selection visibility gate**: the 16x16 physical spawn space is checked against a
  pixel-exact 2088x1172 rectangle for every active ordinary 20x42 player and an overlap aborts the
  current attempt without retrying another candidate. Geometry originally merged as `2bab8904`;
  final physical-floor runtime ownership is part of `f61a7989`. Mounted hitbox geometry remains
  tracked above.
- **Spawn-source descriptor / corrected fallback**: `SpawnSource` preserves the physical floor and
  resolved source row/block; tests pin Platform/Metal Bar/Planter, 1.4.5 Conveyor and the exact
  29-tile source lookup boundary. Descriptor merged as `ab8ace0d`; the incorrect pre-floor fallback
  interpretation was corrected to a physical-floor fallback in `c8227369`.
- **Ocean geometry/source-aware substrate**: the documented 250/380 edge geometry is pinned in
  `water_spawn.rs` (`b3665d92`) and a `SpawnSource`-aware selection API exists (`30b736c2`). Neither
  commit is a claim that the 380 branch is live in `try_spawn`; that remaining integration/oracle
  question is explicitly tracked above.
- **Dungeon and Mowed-Grass postcheck substrate**: deterministic helpers pin all six Dungeon Brick
  types plus the wall requirement and the event-sensitive Mowed/Mowed-Hallow 1/10 rule. Merged as
  `259d2654`; runtime context wiring remains tracked above.
- **Post-selection overhead-liquid gate**: after a candidate survives the retryable location checks,
  Honey/Lava/Shimmer in either of the two cells directly above the physical `SpawnTileY` abort the
  current spawn attempt without trying another point; Water/dry cells remain valid. Initial rule
  merged as `06dc3de3`; final physical-floor runtime ownership is part of `f61a7989`.
- **Safe-wall random-candidate rejection**: `terrustia-proto::wall_house` pins the 1.4.5
  `Main.wallHouse` property through wall id 366, including natural unsafe variants, and the initial
  random candidate rejects player-safe walls before physical-floor search. Table merged as
  `9a5704fd`; stage separation landed in `e2f4a201`.
- **Normal asymmetric spawn/safe rectangles**: candidate sampling uses the pinned 84 west / 83 east /
  46 up / 45 down normal rectangle; the early safe rectangle uses 62 west / 61 east / 35 up / 34
  down. Merged as `e11fafef`. The 1.4.5 fixed-height question is explicitly tracked above.
- **Water source-resolver integration (block projection)**: deep-water source selection consumes
  `spawn_source::block(...)`, so Platform/Metal Bar/Planter floors no longer hide Sand/Jungle source
  tiles from aquatic selection in the ordinary found-source case. Merged as `267b28e9`; the richer
  descriptor remains available for future source-coordinate consumers.
- **Spawn-source tile resolver**: `game/spawn_source.rs` distinguishes the physical floor from the
  effective source block, looks through `tileSolidTop` Platform/Metal Bar/Planter floors, and handles
  the Terraria 1.4.5 Conveyor Belt rule. Merged as `f3de39f8`, generalized by `ab8ace0d`, and fallback
  semantics corrected by `c8227369`.
- **Sea Snail Ocean source**: NPC 220 is present in Ocean water with the independently verified
  Squid:Sea Snail 3:1 relation while preserving Terrustia's previous relative weights among Pink
  Jellyfish, Shark and Squid. Merged as `17d688b4`; this is not a claim that the entire Ocean table
  has been oracle-derived.
- **Deep-water spawn routing / Sleeping Angler water route**: `spawn_medium.rs` separates dry from
  deep-water candidates, `water_spawn.rs` owns the current aquatic source priority, aquatic IDs no
  longer live in dry pools, and Sleeping Angler can appear at the top of an Ocean water column.
  Merged as `987c005f`. The composition/source remainder is tracked above rather than hidden.
- **Bound-town-NPC eligibility**: `game/bound_spawn.rs` owns the per-rescue progression and location
  rules, saved-rescue state, duplicate suppression for both bound and freed forms, and surface-event
  suppression; `spawn.rs` consumes those rules. Merged as `b4caf607`.
- **Delayed distant-NPC regression guard**: the GAPS §30 retry contract is pinned by
  `crates/terrustia/tests/npc_sync.rs`, using two real TCP clients and an inert Bound Goblin so no
  unrelated movement can accidentally rescue a lost one-off health update. Merged as `6c53fd25`.
  The fork currently exposes no GitHub Actions runs, so this is not being described as CI-verified.
- **REST/web administration**: `crates/terrustia/src/panel/` serves the embedded Axum panel with
  authentication, players, kick/ban/unban, whitelist, world switching, settings, console/chat,
  WebSockets and the live world view.
- **World backups and rollback**: saves rotate a bounded `.bak1..bak3` chain and the console exposes
  `backups` and `rollback <n>`, validating a backup before restoration and stopping for a clean
  reload.
- **Cactus growth**: runtime `world/growth.rs` contains `grow_cactus`, and world generation also
  tracks generated cacti. Do not preserve the old "cacti do not grow" task.
- **Shimmer decrafting**: `GameServer::tick_shimmer` already handles whole recipe batches, source
  remainders, world-evil recipe variants, per-unit alchemy loss, max-stack splitting and the
  resulting world-item updates. Do not resurrect the stale "decrafting missing" note.
- **The old once-per-second packet-18 claim**: later bandwidth work changed the clock correction;
  the old paragraph predates that work and must not be used as a current measurement.
- **The claim that no inert NPC exists**: `game/ai/inert.rs` supplies the deterministic case used by
  the distant-NPC regression guard.

Likewise, do not quote the old "2.9x vanilla bandwidth" headline as current: the same historical
TODO later contains a newer five-minute table where Terrustia sends 133,400 bytes against
vanilla's 148,874. Any new bandwidth claim needs a fresh paired capture with the commit hashes and
world recorded beside it.
