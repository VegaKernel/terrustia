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
| ✓ | **Doors build frame-important tiles correctly** | `Tile::block` on a framed type trips a debug assertion and ships -1 frames; six tests were failing on it |

## Audit findings, ranked

Six audits ran against the code. Everything below is either fixed above, or has a row further down.

| | Finding | State |
|---|---|---|
| 🔴 | Ore tiers + banner kills lost on every save of a loaded world | **fixed** |
| 🔴 | `/register` runs Argon2 inline on the game task, no permission check, no rate limit — one client freezes the server | **fixed** |
| 🔴 | `catch_unwind` wraps `tick()` but not `handle_event()`; a panic on the packet path kills the server, skips the shutdown save, and exits `SUCCESS` so `Restart=on-failure` never fires | **fixed** |
| 🟠 | `phase_us` is wall time, `worst_us` is CPU time — every phase cost ever logged is inflated | **fixed** |
| 🟠 | Autosave copies the whole world on the game task: 8–13 ms of a 16.67 ms budget on a *small* world | **fixed** |
| 🟠 | Announcements send literal English where vanilla sends localization keys — parity bug + localization bug + verbatim game text in source | open |
| 🟠 | Town NPCs are all-or-nothing on parse failure; tile entities truncate silently from the first failure | **fixed** |
| 🟡 | No upper `.wld` version bound — a future format would be misparsed and corrupted on save | **fixed** |
| 🟡 | `set_tile` SipHashes a section key on every tile write | **fixed** |
| 🟡 | Packet 60 inbound dropped: the in-game housing UI (drag NPC to a room, evict) does nothing | open |
| 🟡 | Worldgen oracle verifies nothing — `passes.rs:54` is `PASSES: &[Pass] = &[]` | open |
| 🟢 | Journey research and Bestiary data **are** preserved (verified by section index) | no action |
| 🟢 | 59 dependencies, all permissive, zero GPL; `cargo audit` clean; one `unsafe` block, justified | no action |

## Next

### Block A — stop losing data, then publish

| | Item |
|---|---|
| — | Round-trip a real *played-in* world through real Terraria (GAPS §31 never exercised the preserved path) |
| — | Rotating backups, verify-before-replace, `world rollback` / `world backups` |
| — | Windows: `clock.rs` thread CPU, `ctrl_close`/`ctrl_shutdown`, `.tmp` cleanup, `fsync` before rename |
| — | Announcements as localization keys |
| — | Commit backlog (~8 topical commits), `cargo fmt`, version → 0.0.1 |
| — | Licence split: AGPL root, MIT for `terrustia-proto` (currently inherits AGPL) |
| — | `[workspace.lints]`, `#![forbid(unsafe_code)]` on proto+client, `deny.toml`, `*.cs` ignored |
| — | `FEATURES.md`; README leads with "serve the world you already have"; non-affiliation line |
| — | Repo, CI, release + container workflows, cosign keyless signing |
| — | `--world <name>` finds the platform world directory; `--worlds` lists; `--new` |
| — | Reject unknown protocol releases by name; a test that fires when the game moves |

### Block B — earn the tag

| | Item |
|---|---|
| — | `AUDIT.md` + question round |
| — | `max_connections`, per-IP cap, handshake deadline |
| — | Tile-edit spam limiter + section-ownership check (vanilla parity) |
| — | First-registration bootstrap that is not a race |
| — | `cargo-fuzz` over the decoder; commit `.trcap` fixtures and replay in CI |
| — | **Tag v0.0.1** |

### Block C — make it fast, honestly

| | Item |
|---|---|
| — | Liquid pinning test, then `liquid.rs` read/alloc reductions |
| — | Benchmarks: large world × 16 (tuning), × 255 (ceiling, pass/fail) |
| — | Section encoding off the tick; parallel worldgen |
| — | Whitelist; tile action log; `world undo <player> <duration>` |
| — | Packaging: Homebrew, winget, AUR, systemd unit, container `HEALTHCHECK` |
| — | `terrustia update` with signature verification |
| — | Sticky console, new commands, startup panel alignment |

### Block D — make it complete

| | Item |
|---|---|
| — | `place_object` helper (unblocks 8 worldgen passes) |
| — | Worldgen Tier 1: Wave A (7 easy), Wave B (6 middle), then the four big rocks |
| — | A bot that starts with nothing and kills Moon Lord |
| — | Town NPCs (shops, combat, happiness) · Journey mode · 3 missing events · ~123 drops · pets/mounts/rails |
| — | Worldgen Tiers 2 and 3 |

## Corrections to earlier claims

Kept because they are the reason the bugs above went unnoticed for so long.

1. Worldgen was **not** the whole remaining gap.
2. The manifest oracle verifies **nothing** — `PASSES` is empty.
3. `SettleLiquids` needs new code, not transcription.
4. `phase_us` and `worst_us` are different clocks.
5. "Verified 60 Hz" rested on (4), so it is withdrawn until re-measured.
6. `GAPS.md` §31's round trip used a *generated* world, so it never exercised the preserved-header
   path — the one that touches real players' files.
7. `server.rs:8496` and `GAPS.md:38` both claimed banner kills survive a restart. They did not.
