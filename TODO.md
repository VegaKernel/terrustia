# TODO

What is still worth doing, in priority order.

This file is intentionally the **current remainder**, not a history log. `GAPS.md` records what was
found and fixed and is therefore full of old intermediate states by design. A finished item does
not stay here as a zombie task just because nobody remembered to delete the paragraph.

Last reconciled against `master`: 2026-08-29.

---

## In flight

### 1. Finish spawn-candidate geometry and aquatic composition

Deep water is now a real spawn medium on `master` (`987c005f`), not something rejected by the room
check before selection. Existing aquatic coverage is routed through water-specific sources instead
of dry pools, Crab remains a ground spawn, and Sleeping Angler has both his dry Sand route and an
Ocean water-surface route.

The remaining work is narrower and should stay explicit:

- vanilla validates a **2x3** clearance rectangle above the spawning tile; `spawn.rs::has_room`
  still checks only one vertical column, so a solid/lava tile immediately to the left can be missed;
- the generic Surface-water critter source is not implemented. Goldfish (55) was removed from the
  dry Ocean pool rather than left spawning on land, so it needs its real water/critter route;
- Sea Snail and other aquatic NPCs that were never in Terrustia's old spawn roster still need a
  composition pass rather than being smuggled into the medium fix;
- Ocean spawning still uses the project's coarse `Biome::Ocean` classification. Vanilla also has a
  secondary regular-Sand band out to 380 tiles from a true world edge plus vertical restrictions;
- a single `Biome` enum cannot express overlaps such as a Crimson/Jungle Ocean, so water-source
  priority in hybrid areas is not yet a complete SceneMetrics replacement;
- the exact Blood Feeder/Blood Jelly relative probability was not independently recovered from a
  1.4.5.8 oracle. The water fix preserves Terrustia's previous 1:1 relative weighting rather than
  pretending that weighting has now been certified.

Close the 2x3 clearance rule first because it is local, deterministic and affects every spawn
source. Keep the broader composition audit separate.

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

- **Deep-water spawn routing / Sleeping Angler water route**: `spawn_medium.rs` separates dry from
  deep-water candidates, `water_spawn.rs` owns the current aquatic source priority, aquatic IDs no
  longer live in dry pools, and Sleeping Angler can appear at the top of an Ocean water column.
  Merged as `987c005f`. The composition/geometry remainder is tracked above rather than hidden.
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
