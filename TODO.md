# TODO

What is still worth doing, in priority order.

This file is intentionally the **current remainder**, not a history log. `GAPS.md` records what was
found and fixed and is therefore full of old intermediate states by design. A finished item does
not stay here as a zombie task just because nobody remembered to delete the paragraph.

Last reconciled against `master`: 2026-08-29.

---

## In flight

### 1. Pin delayed distant-NPC state with a real regression guard

GAPS §30 fixed a real state-desync bug: a one-off change to a distant NPC could be withheld by the
per-player sync throttle and then lost forever when the NPC's `dirty` flag was cleared.

That bug used to lack a deterministic integration test because every convenient NPC moved or
otherwise became dirty again and accidentally rescued the missing update. The server now has a real
`aiStyle == 0` inert path (`game/ai/inert.rs`), so the missing guard is finally expressible.

PR #1 adds a two-client test using Bound Goblin (105): both clients receive the initial state, one
moves outside section reach, the other applies a one-HP zero-knockback change, and the distant
client must receive the delayed state after the bounded skip window rather than remain stale.

Do not merge this merely because the code looks right: the repository's Actions API currently
reports no workflow runs at all, so the test still needs an actual run somewhere before it becomes
the proof it was written to be.

---

## Highest-value gameplay gaps

### 2. Bound-town-NPC spawn parity is wrong

The rescue **interaction** is implemented correctly in `game/rescues.rs`, but the way bound NPCs
enter the live world is not vanilla-like.

`game/spawn.rs::pick_bound` currently throws all six rescue candidates into one lottery and may
spawn any of them whenever an ordinary spawn candidate lands Underground or in the Cavern layer.
The only filters are "not already rescued" and "not already alive".

That is materially wrong. The six have different progression and place requirements:

- **Bound Goblin (105)**: after the Goblin Army has been defeated, in the Cavern layer.
- **Bound Wizard (106)**: Hardmode, in the Cavern layer.
- **Bound Mechanic (123)**: after Skeletron, in the Dungeon.
- **Webbed Stylist (354)**: in a Spider Nest / unsafe spider-wall area.
- **Sleeping Angler (376)**: at the Ocean, on sand or the water surface.
- **Unconscious Man / Tavernkeep (579)**: only after the Eater of Worlds or Brain of Cthulhu has
  been defeated; unlike the first four he is not a generic cavern-only rescue.

The fix should make eligibility a named rule rather than growing another opaque `match` inside the
spawn loop, then pin every progression gate and location class independently. In particular, a
fresh world must be unable to produce the Wizard, Mechanic or Tavernkeep just because somebody went
mining for long enough.

### 3. Real Terraria client against Terrustia

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

### 4. Ordinary enemy drop coverage

`tools/check_drops.py` remains the source of truth. The last audited figure was about 123 ordinary
enemies with missing loot while boss coverage was complete.

Most remainder is conditional: condition chains and option pools that `tools/gen_drops.py`
deliberately refuses to flatten. Add them to `conditional_drops.rs` in small reviewed batches and
re-run the checker after each batch. Do not replace a known incomplete table with a generated table
that is silently wrong about conditions.

### 5. Seed-identical world generation

The current generator builds a complete playable world and the save format round-trips through the
real game's reader/writer. It does **not** reproduce Terraria's world for a shared seed.

That is a separate, much larger parity project. Progress and the oracle live in
`docs/worldgen-parity.md` and `world/worldgen/manifest.rs`; continue pass-by-pass rather than mixing
seed parity into ordinary playability fixes.

---

## Structurally unverified

These are not known broken. They are known **unchecked**, which is precisely how many of the old
`GAPS.md` defects survived for so long.

### 6. AI behaviour parity

`ai/mod.rs` has routines wired for the used styles, but "a routine exists" is not behavioural
parity. Boss phase transitions, long-running fights and stalls need measurement against the game,
not just per-style unit coverage.

### 7. NPC spawn-pool composition

Spawn rates and caps are modelled and measured. The exact *composition* of what appears under each
combination of biome, depth, time, progression and event is not comprehensively diffed against
vanilla. The bound-NPC defect above is already one example of why that distinction matters.

### 8. Drop probabilities and ordering

Presence of registered drops was audited. `one_in` probabilities, stack ranges, conditional-chain
ordering and expert/master variants have not been comprehensively re-derived and compared.

### 9. Liquid, wiring and housing in motion

These systems have tests, but have not been exercised side-by-side against the real game for long
sessions. Static unit parity does not prove timing, interaction ordering or eventual convergence.

### 10. Features still largely unexamined

Fishing mechanics, golf, dyes, painting, pets, mounts, minecart tracks and much of the cosmetic
layer still need dedicated passes rather than assumptions based on packet coverage.

---

## Deferred by project decision

### 11. tShock-style feature menu

Deferred until the core server work is in better shape. The admin foundation already exists. The
usual candidates remain regions/world protection, warps and homes, item/tile bans and server-side
characters.

### 12. Steam friend-invite joins (P2P)

Ordinary Steam-launched clients connecting by IP use the same TCP protocol and need no special
packet handling. Friend-invite P2P is a transport-layer feature and remains deferred behind an
off-by-default implementation and an explicit licensing decision for the proprietary Steamworks
SDK in this AGPL project.

---

## Closed: do not reopen from stale notes

The following items used to be TODOs and are already implemented on `master`:

- **REST/web administration**: `crates/terrustia/src/panel/` serves the embedded Axum panel with
  authentication, players, kick/ban/unban, whitelist, world switching, settings, console/chat,
  WebSockets and the live world view.
- **World backups and rollback**: saves rotate a bounded `.bak1..bak3` chain and the console exposes
  `backups` and `rollback <n>`, validating a backup before restoration and stopping for a clean
  reload.
- **Cactus growth**: runtime `world/growth.rs` contains `grow_cactus`, and world generation also
  tracks generated cacti. Do not preserve the old "cacti do not grow" task.
- **The old once-per-second packet-18 claim**: later bandwidth work changed the clock correction;
  the old paragraph predates that work and must not be used as a current measurement.
- **The claim that no inert NPC exists**: `game/ai/inert.rs` now supplies the exact deterministic
  case the distant-NPC regression test needed.

Likewise, do not quote the old "2.9× vanilla bandwidth" headline as current: the same historical
TODO later contains a newer five-minute table where Terrustia sends 133,400 bytes against
vanilla's 148,874. Any new bandwidth claim needs a fresh paired capture with the commit hashes and
world recorded beside it.
