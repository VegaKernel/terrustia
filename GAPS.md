# GAPS

Everything known to be missing, wrong, or unverified in this server, as of 2026-08-24.

This document exists because the same failure kept repeating: a subsystem would be finished,
measured against itself, and reported as complete, while the thing it was supposed to enable did
not work. The packet audit closed at 136 of 148 messages and the world generator passed its own
playability check — and you still could not finish the game. So this list is organised by **what
it costs a player**, not by which module it lives in.

Every claim below has a file and line, or a reason it could not be checked. Where I am guessing,
it says so.

## Fixed so far

| | What changed |
|---|---|
| §1 | **Plantera drops the Temple Key.** Its whole classic drop set is in, the invented Power Cell rule is gone, and the test that was *named* for the key — while asserting the wrong item — now asserts the key. |
| §3 | **The Twins drop Hallowed Bars and Soul of Sight**, gated on `MissingTwin` so the pair's loot lands once. |
| §4 | **Skeletron, King Slime, Queen Slime, the Empress and the Cultist** all have their loot. `OneFromOptions` pools are implemented and rolled by the server. |
| §13 | **`projectile_data.rs` is generated** — 916 types, up from 27 hand-written. Every one of the 38 the AI names now exists, so the Destroyer's lasers, Golem's fireballs, the Moon Lord's deathray and the Empress's whole attack set all fire. `launch` now warns instead of failing silently. |
| §23 | **NPC stats scale.** `difficulty.rs` ports the game's curves and `GetStatScalingFactors`; the NPC store applies them at spawn, so no call site can forget. |
| §15 | **A panic no longer loses the world.** The tick is wrapped in `catch_unwind`; an unwind now saves and stops cleanly instead of taking everything since the last autosave with it. |
| §18 | **Duplicate names are refused at the door**, so the name-keyed Angler cooldown cannot be shared or shed by renaming. |
| §19 | Both stale claims corrected — and the panic count is now pinned by `tests/panic_budget.rs` rather than asserted in prose, because a number in a sentence has nothing keeping it true. |
| §25 | **Doors close behind town NPCs.** `DoorAction::Close` was being matched and discarded. |
| §14 *(partial)* | **Grass spreads again.** `world/growth.rs` ports `SpreadGrass` — exposed dirt beside grass turns, evil grass winning contested tiles — sampled around the players each tick. Herbs, trees, vines, cacti and falling sand are still absent. |
| §24 *(partial)* | **Spawn rates respond to the world.** Depth (caverns ×2.5 busier, underworld twice the cap), hardmode, night, blood moons and eclipses all apply, bounded by the game's own floor and ceiling. Player-carried modifiers — candles, potions, invisibility — stay absent because the server does not model a player's inventory. Town suppression still missing. |
| §2 | **Town NPCs move in.** `game/arrivals.rs` ports the `townNPCCanSpawn` chain — the Merchant on 50 silver, the Nurse once he is here and somebody has over 100 life, the Dryad after a boss, the Clothier after Skeletron, the Steampunker after a mechanical boss, and all eight rescued residents off their world-file flags. Bound NPCs still are not *placed* by worldgen, so a world generated here cannot rescue them yet. |
| §6 | **The town survives a restart.** The `.wld` NPC section is now parsed and written instead of carried through as a blob, so residents keep their names, positions and houses. Verified against a real Terraria world: it loads its two townsfolk and still round-trips byte-identically at 2,986,428 bytes. |
| §14 *(more)* | **Herbs grow and ripen.** Seven kinds by ground type, thinned so they do not carpet a field, and immature ones ripen. Potions are renewable again. Trees, vines, cacti and falling sand remain. |
| §5 | **`npc_drops.rs` is generated.** It was the last table in the project without a generator, and the only one whose absence had a name — 226 enemies short 643 drops between them. `tools/gen_drops.py` emits the *unconditional* subset (707 rules over 299 types, from 248 over 210) and deliberately refuses the rest: condition chains and option pools stay hand-written in `conditional_drops.rs`, because a generator that flattened them would hand out wrong loot forever while looking authoritative. Enemies with missing drops: **226 → 123**. |
| §5 | **`tools/check_drops.py`** compares both tables against `ItemDropDatabase` and exits non-zero when a *boss* lacks loot. It drove 10 boss fixes — **bosses missing loot went 20 → 10** — and is what makes the remainder tracked rather than unknown. |
| §24 | **Towns are safe.** Residents suppress spawns steeply — three of them is three times the interval and roughly half the cap — and an event overrules them, so a blood moon still comes to a full street. |
| §11.2 | **The playthrough check exists and passes.** All ten links of the progression chain hold live, end to end. See below. |
| §2 | **Bound townsfolk can be found and freed.** `game/rescues.rs` maps the six — Goblin Tinkerer, Wizard, **Mechanic**, Stylist, Angler, Tavernkeep — from their bound form to the resident they become; they spawn rarely underground while still bound, and talking to one frees them and sets the flag their arrival waits on. This was the last thing standing between the wiring system and anybody being able to use it, because the Mechanic sells the only wire in the game. |
| §14 | **The world is alive again.** Grass spreads, herbs grow and ripen, saplings become trees, vines hang from grass and pair to their biome, and unsupported sand falls. Trees are simplified on purpose — the plain trunk only, not the eight branch-and-root styles, because a wrong frame renders as garbage. Cacti are the one thing left. |
| §26 | **Kills are counted and banners are earned.** `banners.rs` is generated from `BannerSystem` — 447 enemy-to-banner mappings and 43 thresholds — and the counts live on the world, so a hundred zombies killed before a restart still count after it. The save's banner section was two zeroes with a comment admitting it. |
| §7 | **There is an administration system.** `admin/` adds groups with named permissions, accounts hashed with argon2, and bans by name, address or the client UUID the server had been storing and never reading. `/butcher` and friends now need the `world` permission; `/kick`, `/ban` and `/unban` need `players`. A **server console on stdin** — previously unused entirely — takes the same commands plus `say`, `players`, `save` and `stop`. |
| **new** | **The Moon Lord's death is a death.** It was routed through the *expiry* path, so beating the game dropped no luminite and set no flag. Every AI-driven death was inert the same way, lunar pillars included. |
| §11.1 | **The real Terraria server has been talked to** — in both directions — and it disagreed with us sixteen times. The largest: this server refused **every current client**, because it matched one exact version string and the installed game is a release past it. See §28 to §32 for the rest and for what is now pinned against Re-Logic's own bytes. |

### §11.2 — the playthrough check exists, and the chain holds

The thing this document kept naming as the only real proof, and kept deferring:

```
$ cargo run --release -p terrustia-client --example playthrough -- 127.0.0.1:7777
  ok    Eye of Cthulhu     gives [56, 880]
  ok    Eater of Worlds    gives [86]
  ok    Wall of Flesh      gives [367]
  ok    The Destroyer      gives [548]
  ok    Retinazer          gives [549]
  ok    Skeletron Prime    gives [547]
  ok    Plantera           gives [1141]
  ok    Golem              gives [1294]
  ok    Lunatic Cultist    gives [3549, 3372]
  ok    Moon Lord          gives [3460]

every link in the chain holds: this server can be played to the end.
```

It summons each boss, kills it, and watches for the specific item the next step depends on. Not a
real playthrough — nobody mines and nothing is crafted — but the *loot spine* of one, which is the
part that rots silently. A failure names what it costs and prints the NPC types it saw with their
lowest observed health, because "Plantera dropped no 1141" and "Plantera never spawned" need
different fixes and look identical otherwise.

**It immediately found a blocker that every other check had missed.**

`ai/mod.rs` mapped the Moon Lord core's `spent` outcome to `effects.expired`. The core cannot be
killed by damage — it comes apart over ten seconds once its three eyes are broken — so that ending
*was* the kill, and routing it through the expiry path meant the game's final boss was **removed
rather than killed: no luminite, and `downed_moon_lord` never set.** You could beat the game and
the world would not notice.

Worse, `effects.died` turned out to be inert everywhere. It set an NPC's life to zero and nothing
reaped it, so every AI-driven death — a burst spore, an uprooted plant, **a fallen lunar pillar** —
dropped nothing and recorded nothing. `tick_npcs` now reaps them through `npc_died` like any other
kill.

Two smaller things the walker taught me about itself, both worth keeping: a bot that goes quiet
while waiting for a boss is indistinguishable from a dead socket and gets dropped for idling, and
a uniform per-fight timeout is wrong when the Moon Lord's eyes are only damageable in windows —
cutting that fight short looks exactly like a missing drop.

### The one judgement call in the permission system

An unclaimed server — one where nobody has registered — stays wide open, exactly as it was before.
The gate engages the moment somebody runs `/register`, which also makes them the owner.

Locking the commands away the instant permissions landed, before anyone could possibly have an
account, is how a security feature becomes a thing people turn off. `an_unclaimed_server_is_open`
and `registering_claims_the_server` pin both halves, and
`claiming_the_server_locks_down_its_commands` proves it end to end over a real connection.

That test also caught a bug in itself: it passed once and failed on re-run, because an ephemeral
world was writing its admin file into whatever directory the server started in, so the second run
began already claimed. A world with nowhere to save now keeps its admin store in memory.

### What validating the drop generator found

Building `gen_drops.py` against the hand-written table it replaced — where anything *lost* is a
parsing bug — caught four silent extraction failures before any of them shipped: multi-line id
arrays, chained `RegisterToMultipleNPCs` calls, calls assigned to a local first, and
`NormalvsExpert`. Each would have deleted working loot.

It also found a bug in the **old** table. `npcNetIds12` is `{-6, -7, -8, -9, 676}` — negative
*variant* net ids — and the transcription had read them as NPC types 6 and 7, so the Eater of Souls
and the Devourer were dropping the Slime Staff. The generator is more correct than the hand-written
table it replaced, which is the whole argument for having one.

One known simplification, recorded rather than hidden: `NormalvsExpert` rules take their classic
chance, so an expert world under-rolls those twenty-four drops slightly.

### Found by a fourth pass, over the fixes themselves

Two bugs that only existed *because* of the repairs above, which is the argument for auditing a
change rather than trusting it:

- **A transformed NPC lost its difficulty.** `become_type` writes the raw table stats back over the
  scaled ones, so anything that changed form on an expert world quietly reverted to classic
  strength. The game avoids this by routing `Transform` through `SetDefaults`, which rescales. Each
  NPC now remembers what it was scaled by. `a_transformed_npc_keeps_its_scaling`.
- **Boss projectile damage did not scale.** Invisible until §13 was fixed, because before that the
  shots were dropped before they existed and their damage was moot. `HostileProjectileDamageMultiplier`
  now applies at launch, so a master-mode deathray hurts three times as much as a classic one.

That pass also *cleared* three things: player damage is correct as it stands (the receiving client
applies its own defence via `Main.CalculateDamagePlayersTake`, so sending raw damage is right);
bosses do despawn when their target dies, with a test; and all twenty-one AI effect channels are
consumed somewhere — the three that looked dropped are handled in `npc_ai.rs`.

New guard tests: `the_progression_chain_is_unbroken` walks eleven boss→item links and fails naming
which are missing; `difficulty_reaches_a_spawned_enemy` and `only_bosses_scale_with_player_count`
pin the scaling; `caverns_are_busier_than_the_surface` and `events_make_the_surface_dangerous` pin
the rates; `the_number_of_ways_to_panic_is_known` pins the panic budget;
`a_second_resident_arrives_once_the_world_earns_them` proves somebody other than the Guide can
move in; `the_townsfolk_survive_a_save` pins the new save section.

**1,392 passing, clippy clean.** Live: crowd and fuzz healthy, 691/691 NPC types still spawn and
sync, worst tick 326 µs of 16,666 (2%), a real Terraria world loads its residents and round-trips
byte-identically — and the progression chain is walked end to end with every link holding.

### The two audits that now run on demand

Both are the point of the exercise: they turn "we do not know what is missing" into a list, and
they can be re-run after every change rather than depending on somebody thinking to look.

```sh
python3 tools/check_drops.py <decompiled-tree>   # what loot the game gives that we do not
cargo test --test panic_budget                   # what can still take the world down
cargo run --release -p terrustia-client --example playthrough -- 127.0.0.1:7777
```

The last one is the only check here that asks whether the *game* works rather than whether a
subsystem does, and it is the one that found the Moon Lord blocker.

It also knows the difference between unlucky and missing. Golem's Picksaw is a one-in-four drop, so
four fights miss it a third of the time; rather than cry wolf, the walker reads the server's own
tables and reports `luck` when the item is listed but did not land, and `BROKE` only when nothing
in the tables could ever have dropped it. A checker nobody believes is worse than no checker.

---

**Three audit passes so far**, each asking a harder question than the last:

1. **§1-§12 — is it there?** What stops you finishing the game.
2. **§13-§22 — does it produce anything?** Systems that run and silently emit nothing.
3. **§23-§26 — does it produce the right amount?** Systems that run, emit, and are the wrong size.

Every pass found something the previous one had read past. §27 is what none of them covered.

---

## Severity

| | |
|---|---|
| **BLOCKER** | The game cannot be finished. No workaround. |
| **SEVERE** | A whole system is absent or unreachable. The game runs; large parts of it are missing. |
| **WRONG** | Implemented, but does something different from the real game. |
| **THIN** | Present and correct as far as it goes; incomplete. |
| **UNVERIFIED** | May be fine. Nobody has checked. |

---

## 1. BLOCKER — Plantera drops no Temple Key

**The run ends here.** Every other gap has a workaround; this one does not.

`ItemDropDatabase.cs:420` registers the Temple Key as `ItemDropRule.Common(1141)` — a flat 100%
drop under `NotExpert`. Our tables have no entry for NPC 262 at all: it is absent from
`classic_only` (`conditional_drops.rs:186-256`), absent from `npc_drops.rs`, and so falls through
to `_ => Vec::new()`.

Without the key the Jungle Temple door never opens. Lihzahrd Brick needs a Picksaw to mine, the
Picksaw drops from Golem, and Golem is inside the temple — so there is no way round it. That ends
the chain at: **no Golem → no Lunatic Cultist → no Lunar Pillars → no Moon Lord.**

There is a second, related bug in the same place. `conditional_drops.rs:161-163` reads:

```rust
// Plantera's death opens the temple, and the key is what opens it.
if at.downed_plantera && npc_type == 262 {
    out.push(always(1293));
}
```

The comment describes the Temple Key. Item 1293 is the **Lihzahrd Power Cell**, which in the real
game is a 1/50 drop from temple enemies (`ItemDropDatabase.cs:981`) and is *not* a Plantera drop at
all. So this rule drops the wrong item, on the wrong condition (repeat kills only), and its comment
describes a third thing that was never implemented.

**Done means:** Plantera drops item 1141 at 100% on a non-expert kill, and the invented 1293 rule
is removed. A playthrough reaches Golem.

---

## 2. BLOCKER — no town NPC ever moves in except the Guide

`tick_town_npcs` (`game/server.rs:5585-5636`) does exactly two things: it houses a town NPC that is
already alive and homeless, and if there is no Guide, it spawns the Guide. That is the whole
arrival system.

There is no condition table — no Merchant at 50 silver, no Nurse at 100 max health, no
Demolitionist holding an explosive, no Dryad after a boss, no Arms Dealer with a gun. None of the
roughly twenty standard townsfolk can ever appear.

And the six **bound NPCs you rescue never spawn either.** Checked directly against
`game/spawn.rs`: Bound Goblin (105), Bound Wizard (106), Bound Mechanic (107), Webbed Stylist
(123), Sleeping Angler (353), Unconscious Bartender (550) — none appear anywhere in the spawn
tables.

What that removes:

- **The Mechanic → no wire.** The entire wiring system is implemented, ported carefully, and
  documented in `docs/wiring.md` — and there is no way to buy wire.
- **The Goblin Tinkerer → no reforging**, so no accessory modifiers at all.
- **The Angler → the whole quest system is unreachable.** 41 fish are tabled
  (`terrustia-proto/src/angler.rs`), the quest rotates daily, the packets are handled — and the NPC
  who gives the quest never exists. Grep for `ANGLER` in `server.rs` returns only packet handlers.
- **The Nurse → no healing service.** The Dryad, Wizard, Witch Doctor, Steampunker, Cyborg,
  Truffle, Tavernkeep, Stylist, Painter, Zoologist, Party Girl, Tax Collector — all absent.

I have this as a BLOCKER rather than SEVERE because the Tavernkeep gates the Old One's Army, and
several progression items are only purchasable. Strictly, Moon Lord may still be reachable without
any of them; practically, this is not the game.

**Exceptions that do work:** the Old Man spawns at the dungeon (`server.rs:7621`) and summons
Skeletron (`:7651`), and the Travelling Merchant arrives properly (`:1004`).

**Done means:** an arrival-condition table generated from `NPC.SpawnNPC` / `NPC.CanTownNPCSpawn`,
plus bound-NPC placement during worldgen and natural spawning.

---

## 3. SEVERE — the Twins drop nothing, so Chlorophyte is unreachable

`ItemDropDatabase.cs:457-470`, under `MissingTwin` + `NotExpert`:

```csharp
leadingConditionRule2.OnSuccess(ItemDropRule.Common(1225, 1, 15, 30));  // Hallowed Bars
leadingConditionRule2.OnSuccess(ItemDropRule.Common(549, 1, 25, 40));   // Soul of Sight
```

Our tables give NPC 125 and 126 a 1-in-10 **trophy** and nothing else
(`npc_drops.rs:649-660`). The Destroyer and Skeletron Prime do drop their bars and souls
(`conditional_drops.rs:212-221`) — only the Twins were missed.

Soul of Sight is required for the Drax and the Pickaxe Axe, which are the only way to mine
Chlorophyte. So no Chlorophyte, no Turtle armour, no Chlorophyte tools.

This also needs a condition we do not have: `Conditions.MissingTwin` — the loot drops only when the
*other* twin is already dead.

---

## 4. SEVERE — six more bosses drop nothing but a trophy

Bosses with real loot (`conditional_drops.rs`, `classic_only`): 4, 13/14/15, 113, 127, 134, 222,
245, 266, 370, 398, 551, 668.

Bosses with **no loot entry at all**:

| Boss | Type | What is lost |
|---|---|---|
| Skeletron | 35/36 | Skeletron Hand, Book of Skulls, Bone gear |
| King Slime | 50 | Slime Staff, Ninja set, Royal Gel, Slime Hook |
| The Twins | 125/126 | see §3 |
| **Plantera** | 262 | see §1 |
| Lunatic Cultist | 439 | Ancient Manipulator — so no Luminite gear can ever be crafted |
| Queen Slime | 657 | the entire Queen Slime drop set |
| Empress of Light | 636 | the entire Empress drop set |

The Ancient Manipulator matters more than it looks: it is the crafting station for every Luminite
item, so even a successful Moon Lord kill leads nowhere.

---

## 5. SEVERE — the drop tables are hand-written and have no generator

`docs/generated-tables.md:21` lists `npc_drops.rs` with `—` in the generator column. **It is the
only table in the project without one**, which directly violates the rule the codebase states about
itself:

> Per-type variation lives in generated tables. Hand-written modules hold algorithms only.

That is the root cause of §1, §3 and §4. Hand transcription naturally stops at the tractable cases:
`npc_drops.rs` covers 209 NPC types and states it holds "all 248 unconditional rules", while the
game's `ItemDropDatabase` makes 621 registration calls. `conditional_drops.rs` is a second
hand-written file covering about fifteen bosses.

The excluded categories — boss bags, master-mode drops, `ByCondition`, `OneFromOptions`,
`LeadingConditionRule` — are precisely where all boss loot lives.

**Done means:** `tools/gen_drops.py` emitting both tables from `ItemDropDatabase`, with a condition
enum, plus a second independent checker in the style of `tools/check_recipes.py`.

---

## 6. SEVERE — nothing about town NPCs survives a restart

The world file's NPC section is never parsed and never rewritten.

- **Read:** `wld.rs:139-155` slices sections 4 onward as opaque blobs. The townsfolk section is
  never decoded, so a real Terraria world's existing NPCs — who they are, where they live, their
  names — are invisible to the server.
- **Write:** `wld_save.rs:122-130` copies every trailing section back verbatim except
  `TILE_ENTITY_SECTION`. For a generated world, `serialize_fresh` writes an empty NPC section.

So town NPCs live only for the session. Their given names (`npc.given_name`, assigned lazily at
`server.rs:4852`) are regenerated every restart, and housing assignments are lost. The bestiary,
pressure plates and town-room assignments are in the same position — carried through untouched,
never updated.

---

## 7. SEVERE — no administration of any kind

Stated outright in the code, at `game/server.rs:3563`:

```rust
/// There is no permission model: this is aimed at a server among friends, and every command
/// here is either read-only or something any player could achieve anyway.
```

That comment is not accurate about its own command list. Any connected player can run `/time` (set
the world to night), `/spawn <boss>`, `/butcher` (delete every NPC) and `/save`.

Absent entirely: accounts, groups, permissions, bans (by name, IP or UUID), mute, a `/kick`
command, a server console, protected regions, item bans, spawn protection, warps, and any audit
log. `kick()` exists and works (`server.rs:1461`) with only two handshake call sites.

`Player::uuid` (`game/player.rs:33`) is received and stored by `on_uuid` (`server.rs:2594`) and
**read by nothing** — it is the obvious identity hook and is currently dead weight.

Also latent: `run_command` lowercases the entire argument (`server.rs:3570`), which will corrupt
any player name, password or region name the moment a command takes one.

---

## 8. SEVERE — the server believes whatever a client claims

Documented honestly in `README.md`, but it belongs on this list because it is what a public server
would need.

- **Inventories are not authoritative.** `on_health` and `on_mana` (`server.rs:2081`, `:2094`)
  write whatever arrives straight into `Player` and rebroadcast.
- **No range check on tile edits.** `on_tile_manipulation` (`server.rs:2602`) checks `is_playing`
  and world bounds and nothing else — any client can edit any tile anywhere, at any distance, at
  any rate.
- **No item, tile or projectile ban list**, no stack-size validation, no spawn detection.

The one place the server does overrule a client is `on_client_projectile` (`server.rs:4727`), which
rejects mis-owned and hostile projectiles.

---

## 9. WRONG — smaller divergences found and not yet fixed

| What | Where | Effect |
|---|---|---|
| `SendSection` does not sync NPCs or chest contents for the section | vanilla does both at `NetMessage.cs:2732` | an NPC in freshly streamed ground stays invisible until its next periodic update |
| No `Main.SyncAnInvasion` on packet 6 | `MessageBuffer.cs:470` | cosmetic |
| Section batching is stricter than the game's `Tile.isTheSameAs` | `section.rs:105` | more, smaller runs — correct output, more bytes, and it hits the 65535-byte frame cap sooner than vanilla would |

---

## 10. THIN — systems that exist but are partial

- **Shops.** Only the Travelling Merchant's stock is server-side (correct — it is the only one the
  server must roll). `ShopOverride` (104) is unimplemented, so a server cannot change any town
  NPC's stock or prices. Normal shops run client-side off the packet-7 flags, which now includes
  the five that were missing.
- **Difficulty modes.** `game_mode` is read in four places — drop conditions
  (`server.rs:5478`), moon waves (`:7347`), NPC scaling (`:4000`). Expert and master are not
  systematically applied to NPC stats, AI, or the mechanics that differ.
- **Journey mode.** Not implemented. Creative powers are preserved verbatim in the save and never
  acted on. `SetCountsAsHostForGameplay` (139) and `ClientSyncedInventory` (138) are unhandled.
- **Special world seeds.** Not offered. `hurt_tiles.rs` documents two tiles it omits for this
  reason. Packet-7 flag bytes 8-10 — which carry `zenithWorld`, `remixWorld`, `notTheBees`,
  `getGoodWorld`, `skyblockWorld`, the slime unlocks and `fastForwardTimeToDusk` — have no names in
  our enum, so those bits can never be set. The layout is still correct: all eleven bytes are
  written.
- **Server-side characters.** Not implemented; there is no per-player persistence of any kind.

---

## 11. UNVERIFIED — the things nobody has actually checked

These are not known to be broken. They are known to be **unchecked**, which is a different and in
some ways worse category, because the first ten sections above are all things that looked fine
until someone looked.

1. ~~**No real Terraria client has ever connected.**~~ **Closed from the other side.** The blind
   spot was never really about the client — it was that both ends of every test were built on
   `terrustia-proto`, so a field read at the wrong width passed both. Pointing our client at the
   **real game's dedicated server** breaks that symmetry just as well, and needs nobody to be
   sitting at a keyboard. It found five real defects; see §28. What is still untested is a real
   client *reading* what this server writes — but every packet it will read is now either
   re-encoded byte-for-byte from the real server's own bytes or diffed against them field by field.
2. **No playthrough has ever been attempted.** There is no bot that starts with nothing and walks
   the progression chain. This is the check that would have caught §1, §2 and §3 immediately, and it
   has been "Phase 3, not built" in every plan written for this project. That is not a scheduling
   accident: it is the only test that can prove the completion claims wrong.
3. **No generated world has been opened in Terraria.** The `.wld` writer was verified by
   re-implementing `LoadWorld_Version2` in Python, which is strong evidence and not the same as the
   game opening the file.
4. **Drop *rates* are unverified.** Presence was checked for the entries that exist; the `one_in`
   values, stack ranges and chain ordering were not re-derived from the source.
5. **AI parity is claimed per style, not measured.** `ai/mod.rs:398` marks styles 0-127 as
   `Ported`, and a test asserts every style claiming parity has a routine wired up — but nothing
   compares behaviour against the game. (Coverage itself is complete: style 98 is unused in
   1.4.5.7 and no NPC uses a style above 127.)
6. **Liquid, wiring and housing** are ported and unit-tested, never compared against the real game
   in motion.

---

## 12. Known, sized, and deliberately deferred

Not gaps in the sense above — decisions with reasons written down.

- **Seed-identical world generation.** 219-372 engineer-days. The oracle is built and green.
  `docs/worldgen-parity.md`. Generation is complete and playable; it just is not *Terraria's* world
  for a given seed.
- **Steam P2P (friend invites).** Needs the Steamworks SDK under AppID 105600 and a licence
  decision, because this project is AGPL. Protocol-level Steam support is already complete — a
  Steam-launched client connecting by IP is byte-identical to any other.
- **`DevCommands` (94).** Deliberately unhandled. A public server that honours it is a public
  server anyone can rewrite.
- **Host migration** (`SpectatePlayer` 150, `HostToken` 161) — does not apply to a dedicated server.

---

# Second pass

The first pass followed the progression chain. This one went after what §23 said it had skipped:
systems that exist and might be wrong, rather than systems that are missing.

---

## 13. SEVERE — most NPC ranged attacks silently do nothing

**32 of the 39 projectile types the AI tries to fire do not exist.**

`Projectiles::launch` (`game/projectile.rs:348`) opens with `let stats =
projectile_stats(projectile_type)?;` — an early return on `None`. `projectile_data.rs` holds stats
for 27 types. The consuming loop at `server.rs:4567-4577` then simply skips:

```rust
for shot in std::mem::take(&mut ai_out.shots) {
    self.shots_thrown += 1;
    if let Some(index) = self.projectiles.launch(...) {
        self.broadcast_projectile(index);
    }
}
```

No log, no error. The AI decides to fire, the shot is counted, and nothing is created or
broadcast. What that removes:

| | Type | |
|---|---|---|
| The Destroyer's lasers | 100 | its only ranged attack |
| Golem's fireballs | 258 | |
| Moon Lord's Phantasmal Deathray and Spheres | 455, 462 | the fight's signature attacks |
| Duke Fishron's bubbles | 385 | |
| **Empress of Light — all five attacks** | 872, 873, 874, 919, 923 | she has nothing left |
| Every caster enemy's bolt | 435 | |
| Dark Mage's bolt, heal and portal | 673, 674, 675 | so the Old One's Army loses its healer |
| Betsy's fireball and flame breath | 686, 687 | |
| Pumpking's spheres, Martian Saucer's deathray | 326, 447 | |
| Antlion, cannon, copter, crawler spit, doom, jellyfish, lightning bug, nautilus, nebula floater, nimbus, rider, sandnado, santa, seed, tablet shard | various | |

Contact damage still works, so bosses are not harmless — they are melee-only, and several are
close to trivial as a result. This is the biggest single behavioural divergence found so far, and
it is invisible from the outside: everything reports success.

A smaller bug rides along. `shots_thrown` increments before the `launch` attempt, so the counter
`/npcs` prints counts shots that were never fired.

**Done means:** `projectile_data.rs` generated from `ProjectileID`/`Projectile.SetDefaults` rather
than hand-listed, covering every type any NPC routine names — and `launch` logging a warning
instead of returning `None` silently.

---

## 14. SEVERE — the world never changes on its own

`WorldGen.UpdateWorld` (`WorldGen.cs:72033`) is the game's random tile-update loop. Each tick it
samples tiles across the world and grows things. **None of it is implemented.**

Searched for and absent from the entire crate: grass spreading, herb and plant growth, tree growth
from saplings, vine extension, cactus growth, mushroom-grass spread, falling sand. The only tile
that grows anywhere is Plantera's bulb (`world/bulbs.rs:40`). The tick loop
(`server.rs:768-830`) runs liquids, biome spread, weather, wiring, timers, lunar, items, NPCs,
projectiles, contact damage, spawning, town NPCs, the merchant and the Old Man — and no tile
updates.

What that costs a player:

- **Trees never regrow.** Acorns do nothing. Wood is a finite resource in the world as generated.
- **Herbs never regrow.** Daybloom, Blinkroot, Moonglow are one-time picks, so potions are finite.
- **Grass never spreads**, so you cannot make a biome — no jungle from mud, no mushroom biome, and
  the Truffle could not move in even if town NPCs worked (§2).
- **Sand does not fall.**

`tick_spread` (`server.rs:7509`) does implement hardmode corruption/hallow creep, which is the one
piece of `UpdateWorld` that made it in.

---

## 15. SEVERE — a panic loses the world back to the last autosave

There is no `catch_unwind` anywhere in the crate, and the game is a single actor task. On a panic:
`&mut game` completes in the `tokio::select!` (`main.rs:143`), `game.await` returns a `JoinError`
(`:151`), and the process logs "the game task did not shut down cleanly" and exits. **The shutdown
save lives inside the game loop, so it never runs.** Everything since the last autosave is gone.

The panic surface itself is small and mostly proven — see §19 for the count — but "small" is not
"zero", and the consequence is total. A supervisor that caught the panic, saved, and restarted
would turn a lost world into a hiccup.

---

## 16. WRONG — doors are relayed but never modelled

`on_door` (`server.rs:2776`) relays the toggle and deliberately does not change the tiles, because
a door swings between a 1×3 closed tile and a 2×3 open one with recomputed frames. Its own comment
says so.

Consequences the comment does not draw out:

- **The server's world is wrong** until some client happens to push a tile square over those
  tiles. Every server-side system that reads them — NPC pathing, housing validity, the section
  stream sent to a *joining* player — sees the stale state.
- **A door's state at save time is whatever it was when the world loaded.** Open doors close
  themselves across a restart.
- Blood-moon zombies that break doors, and town NPCs that open and close them
  (`ai/town.rs:21`), are both acting on a world the server does not agree with.

---

## 17. THIN — town NPCs never fight back

`ai/town.rs:23` scopes the style to movement and housing, and names what is left out: shops,
dialogue, **the attack states**, sitting, pet idle animations.

So during a Blood Moon, a Goblin Army or the Old One's Army, the townsfolk stand still and are
killed. `tick_town_casualties` (`server.rs:5041`) exists to handle them dying, which is the half of
the interaction that got built.

---

## 18. WRONG — two players can share a name

`on_sync_player` (`server.rs:1713-1717`) takes the name straight from the packet and rejects only
an empty one. There is no uniqueness check, no reservation, no binding to the stored UUID.

That is not only cosmetic. `angler_finished_today` is a `HashSet<String>` **keyed by name**
(`server.rs:484`), deliberately, so a player cannot reconnect to re-claim the Angler reward. With
duplicate names, two players share one cooldown — and either can dodge it by renaming.

---

## 19. WRONG — two documentation claims are false

Both would mislead the next person to read them, which is worse than silence.

**`docs/performance.md` claims three `unwrap`/`expect` calls outside tests, and names them.** There
are **seven**: `config.rs:42`, `net/record.rs:163`, `net/record.rs:164`, `game/buffs.rs:345`,
`terrustia-proto/src/reader.rs:19`, `world/worldgen/layout.rs:277`,
`game/ai/boss/moon.rs:118`. All are on proven invariants, so the safety claim holds; the count does
not, and two of them are mine from today. A prose number with nothing keeping it honest drifts.
This should be a test that counts them, not a sentence.

**`shimmer.rs:7-11` says decrafting is "deliberately absent" and "a system this server does not
model at all".** It was built: `decraft_recipe` (`server.rs:1170`), a `Decraft` type
(`server.rs:272`), 2,551 decraftable recipes. `docs/shimmer.md:60-89` documents it correctly, so
the module comment now contradicts the document it points at.

---

## 20. THIN — operational gaps

- **No config reload.** `terrustia.toml` is read once at startup (`config.rs:65`) and never
  re-read. Every setting needs a restart, and a restart is a save-and-hope.
- **No log file and no rotation.** `term.rs` writes to stdout only. No file sink, no syslog, no
  JSON, no audit trail of who did what.
- **No metrics endpoint.** The tick-phase instrumentation exists and is only visible as `DEBUG`
  lines in the console.

---

## 21. The test-shaped root cause, again

**No test anywhere asserts that any boss drops any progression item.** Checked across all 103
integration tests and both drop modules.

What the drop tests do assert: that a demon eye rolls its rare drop before its lens
(`npc_drops.rs:2605`), that an NPC with no rules drops nothing (`:2622`), that "a good many of the
roster drop something" (`:2640`), and that most things have no conditional drop
(`conditional_drops.rs:470`).

Every one of those is a *breadth* check. Not one walks the chain. That is precisely why §1, §3 and
§4 survived 1,324 passing tests — and §13 shows the same shape in a different subsystem: the AI
fires, the counter increments, nothing is asserted about what came out.

The cheap version of the fix is a table-driven test: for each boss, assert the specific item its
progression depends on. The real version is §23.2.

---

## 22. Checked in this pass and found fine

Recorded so the next audit does not spend time here again.

- **Furniture drops.** `tile_drops.rs` covers 345 tile types and documents that it excludes the 64
  whose drop depends on a frame style — but `drop_of` (`server.rs:3760`) falls back to
  `tile_object` + `placed_items` for exactly those. Furniture does drop correctly.
- **AI style coverage is complete.** Styles 0-127 are marked `Ported` (`ai/mod.rs:398`). Style 98
  is absent from that list but no NPC in 1.4.5.7 uses it, and no NPC uses a style above 127.
- **Biome detection.** `biome_at` (`spawn.rs:231`) counts tiles locally around a point, which is
  what spawn pools need. The game's global `CountTiles` census is not required for it.
- **Save version support.** `MIN_VERSION = 279` (`wld.rs:28`); older worlds are refused with a
  clear message rather than misparsed.
- **Death, respawn, PvP, teams, NPC despawn, item despawn** all exist and are wired.
- **Events.** Blood moon, solar eclipse, Pumpkin and Frost Moon, and all four invasions (Goblin,
  Frost Legion, Pirate, Martian) are implemented, along with the Old One's Army and the lunar
  apocalypse.

---

# Third pass

§27 said the fix was to stop asking "does this system exist" and start asking "does it produce
anything". This pass applied that to difficulty, spawning and the door state machine.

The theme it found is different from the first two. §1-§12 were things *missing*. §13-§22 were
things that *ran and produced nothing*. These are things that **run, produce output, and produce
the wrong magnitude** — which is the hardest kind to see, because everything looks alive.

---

## 23. SEVERE — NPC stats never scale, for difficulty or for player count

`Npc::become_type` (`game/npc.rs:268`) assigns the table value straight through:

```rust
self.life_max = stats.life_max;
self.life = stats.life_max;
```

The words `expert` and `master` do not appear anywhere in `game/npc.rs` or
`terrustia-proto/src/npc.rs`. There is no scaling of any kind.

The game calls `ScaleStats` on every spawn (`NPC.cs:8370`, `:17888`), which dispatches to three
things we have none of (`NPC.cs:18178-18190`):

- `ScaleStats_ByDifficulty` — expert and master multipliers on life, damage and defence
- `ScaleStats_ForExpertHardmode`
- `ScaleStats_ByPlayerCount` — **multiplayer scaling**, which is the whole reason a boss is not
  trivial with eight people hitting it

So: a real Terraria **expert or master world served here has classic-strength enemies**, and an
eight-player fight is the same difficulty as a solo one. Stack that on §13 — bosses with no ranged
attacks — and the intended difficulty curve is simply not present.

Note that `game_mode` *is* read for drops (`server.rs:5478`) and moon waves (`:7347`), so the value
is available and correct. Nothing applies it to stats.

---

## 24. SEVERE — spawn rates are the surface-daytime default, everywhere

`spawn.rs` uses two flat constants and nothing else:

```rust
pub const SPAWN_RATE: u32 = 600;   // spawn.rs:15
pub const MAX_SPAWNS: f32 = 5.0;   // spawn.rs:18
```

with a per-tick roll of `rng.random_range(0..SPAWN_RATE) != 0` (`:521`) and a cap of
`MAX_SPAWNS * (1.0 + 0.3 * players)` (`:514`).

Those are the game's `defaultSpawnRate` and `defaultMaxSpawns` — the *baseline* that
`GetSpawnRate` (`NPC.cs:474`) then modifies before use. Every modifier is absent:

| Modifier | Game | Effect of missing it |
|---|---|---|
| Underground | rate ×0.4, max ×1.9 | caves are roughly **2.5× too quiet** |
| Underworld | max ×2 | half as busy as it should be |
| Hardmode | rate ×0.9, max +1 | hardmode does not get more dangerous |
| Biome (jungle, corruption, crimson, hallow, dungeon, meteor, ocean, snow, desert, mushroom) | large per-biome tables | every biome spawns at the same rate |
| **Town NPC count** (`NPC.cs:610-628`) | rate rises steeply per resident | **a town is never safe** |
| Water/peace candle, battle/calming potion | multiplicative | no way to raise or lower spawns |
| Blood moon, eclipse | large increases | events do not intensify |

The town one is the most player-visible: in the real game a base with residents is quiet, and here
it is exactly as dangerous as open wilderness. (Moot today, since only the Guide can move in —
see §2 — but it will bite the moment that is fixed.)

---

## 25. WRONG — doors that NPCs open are never closed

Town NPCs produce a close action and the server throws it away (`server.rs:4562`):

```rust
crate::game::ai::town::DoorAction::Close { .. }
| crate::game::ai::town::DoorAction::None => {}
```

The fighter side cannot close one either: `fighter::Action` (`ai/fighter.rs:47`) has exactly
`None`, `OpenDoor` and `BreakDoor`. There is no close action anywhere in the codebase.

`ai/town.rs:21` states the opposite as settled behaviour — "a closed door gets opened and then
closed behind it."

With §16 this leaves the door's state incoherent three ways at once:

- **the server's world** never changes, because `on_door` deliberately does not model the tiles;
- **every client** receives the open broadcast and never a close, so doors accumulate open
  permanently on screen;
- **the save** keeps whatever state the world was loaded with.

At night that is the difference between a sealed house and an open one, and no two observers agree
about which it is.

---

## 26. THIN — nothing counts kills, so banners never drop and the bestiary never fills

There is no per-type kill counter anywhere in the server. `wld_save.rs:554` is explicit about the
consequence:

```rust
w.i16(0).i16(0); // no banner kill counts, no claimable banners
```

So the 50-kill banner rewards never arrive, and the bestiary — already preserved verbatim rather
than updated (§6) — stays empty however much is killed. Both are content rather than progression,
but both are things a player will look for and not find.

---

## 28. What the real Terraria server said

The fifth pass, and the first that asked a question none of the others could: **not "is our reading
of the game right?" but "what does the game actually put on the wire?"**

Terraria is installed on this machine. Its dedicated server can be run against a scratch copy of a
world, and `terrustia-client` can be pointed at it. That is the independent opinion this document
has been asking for since the first pass — the bytes on the far end of that socket were produced by
Re-Logic's code and owe this project nothing — and it turns out not to need a person at a keyboard
at all. `cargo run -p terrustia-client --example conform -- 127.0.0.1:7930 capture.trcap` connects,
records every byte, decodes each frame, and re-encodes it to check the bytes come back identical.

Two servers were run side by side on the same world file, and their packet 7 diffed field by field.

**What it found.**

| | What was wrong |
|---|---|
| **The version check refused the installed game.** | `is_supported` matched one exact string, `"Terraria325"`. The installed game is 1.4.5.8 and announces **326**. Every current client was refused at the door with "You are not using the same version as this server." Now a range, so both releases connect. |
| **Packet 7 was four bytes short.** | Release 326 appends `dungeonX` and `dungeonY` as two `i16`s after the extra-spawn-point list. Nothing in the 1.4.5.7 source mentions them; they were found as four bytes left over that no field accounted for, and identified by matching them against two worlds' own `.wld` files. This is not cosmetic: a 326 client reads those four bytes whether or not they were sent, so it would have parsed the head of the *next* packet as a dungeon position and stayed desynchronised for the rest of the session. |
| **Every loaded world served the wrong sky.** | The thirteen background styles and thirteen tree-top variations are in the world file — in three separate runs — and the parser read past all three. Every biome drew the style-zero backdrop. Generated worlds had no styles at all; `worldgen/scenery.rs` now rolls them the way `RandomizeBackgrounds` does. |
| **The Dryad reported nought per cent of everything.** | Packet 57 carries how much of the world is hallow, corrupt and crimson, and was never sent. `world/census.rs` ports `CountTiles` — one column per tick, the surface weighted five times over, dirt skipped, and a denominator of six specific tile types rather than everything solid. On the test world it independently computed **4% crimson, the same figure the real server sent**. |
| **The housing screen was empty.** | Packet 60 says where each town NPC lives, and was never sent. Ours now matches the real server's byte for byte for a settled NPC. |
| **Chests looked empty until opened.** | The game sends the full contents of every chest inside a section *with* that section, so the client can craft from nearby chests, quick-stack into them and search them without opening anything. The real server sent 280 such frames on join where this one sent none. |
| **The host was never marked.** | Packet 139 tells a client its slot counts as the host, which the game decides purely by whether the connection came from the loopback address. |
| **The bestiary showed nought kills for everything.** | Net module 11 carries every banner's kill count, and was never sent. The world has recorded these since §26 and nothing was telling anybody. Ours is now **byte-identical** to the real server's 1765-byte frame, and the counter also ticks live on each kill rather than only on the next join. |
| **The pylon network was scenery.** | Module 8 announces each pylon on join, and this server sent none — so a client standing at one opened a travel map with nowhere to go, however many pylons the world had. Placement, mining, saving and loading all worked; nothing told anybody. Now announced, and the teleport request that comes back is handled. |
| **Water moved by tile square, not by net module 0.** | Functionally it worked, but it is not what a client expects, and it cost roughly six times the bytes — a settling pool dirties a stripe of tiles every tick, so this was a flood of its own. Now module 0, pinned byte-for-byte against a real server's frame. The coordinate is one `i32` with **x in the high half**, which reads as a plausible position either way round on a square world and puts every splash in the wrong place on a real one. |

**The pylon network was scenery.** Reading the game's join sequence line by line — which is what
the capture pointed at — turned up one more: `PylonSystem.OnPlayerJoining` sends one net module 8
per pylon, and this server sent none. Pylons were saved, loaded, placed and mined correctly; the
client was simply never told any existed, so standing at one opened a travel map with nowhere to
go. Announcing them, and handling the teleport request that comes back, makes the network work:
`a_pylon_with_a_town_around_it_carries_a_player` proves the whole round trip.

One detail there was nearly a bug of its own. A pylon's network lives in its *tile's* frame, and
mining the tile is what removes the entity — so by the time there is a removal to announce, the
frame is gone. I first sent zero for it and wrote a comment claiming the client ignores the network
on a removal. Checking rather than trusting that: `TeleportPylonInfo.Equals` compares position
**and** type, so a removal with the wrong type removes nothing, and a mined jungle pylon would have
sat on every travel map for the rest of the session. The server now remembers each pylon's network
from the moment it announces it.

The biome requirement is deliberately not enforced — deciding whether a stretch of ground counts as
a jungle needs `SceneMetrics`, which this server does not have. A pylon planted in the wrong biome
works here and would not in the game. That is permissive rather than broken, and it is written down
in the code rather than left to be discovered.

**One of those fixes opened a hole, and it was nearly missed.** Moving liquid to module 0 broke
nothing in the test suite — because the only liquid test joined a *fresh* client afterwards and
read the pool out of a section loaded from scratch. It would have passed just as happily if the
server had told nobody anything while the water was falling. `a_connected_client_is_told_when_water_moves`
stays connected throughout, and was confirmed to fail against the disabled broadcast before being
kept. The client was taught to read module 0 for the same reason: a probe that ignores a message
cannot notice it stopped arriving.

**Where packet 7 now stands.** Both servers on the same world file, diffed field by field:

```
45/47 fields identical
time              real 13500          ours 14359      the clock has moved on
windSpeedTarget   real 0.1789         ours -0.303     both simulate wind; ours loads 0.179 correctly
```

Nothing is left that is not a live value both servers change as they run.

**What is now checked against real bytes rather than against ourselves.** All fifteen of the real
server's compressed tile sections inflate, decode and **re-encode byte-identically** — which
exercises the tile bit-flags, the run-length batching, the frame-importance table and the chest,
sign and tile-entity trailers, hundreds of decisions that all have to be right at once for the
bytes to come back equal. Packet 7 round-trips byte-identically from a captured payload, pinned in
`packets.rs` as `REAL_SERVER_PACKET_7`. Packets 57, 60 and 139 are pinned against the real
server's bytes too, and net modules 0 and 11 are byte-identical to the frames a real server sent —
module 11's all 1765 bytes of it.

**What this still does not prove.** A real client has not read what this server writes. That is a
smaller gap than it was — every packet it will read is now either re-encoded from Re-Logic's own
bytes or diffed against them — but "the encoder agrees with theirs on the packets we captured" is
not "a client renders it correctly", and a session only covers what it covers. The census printed
by `conform` names what each run touched, precisely so an untested packet is not quietly mistaken
for a passing one.

---

## 29. What the real server did with what *we* sent

§28 checked one direction only. Everything in it reads a real server's bytes and proves this
project can decode them — which covers everything the server says and **nothing the client says**.
Those are separate risks. A field written at the wrong offset in an *outgoing* packet is invisible
to any amount of decoding; it shows up as a real server quietly ignoring the request, which looks
exactly like a server that received nothing.

So pass six asks the other question, with `examples/provoke`: **does a server that owes nothing to
this code act on what we send it?** Perform an action, then watch for the consequence.

**The first version of the probe was wrong, and it blamed the real server for it.** It acted and
then waited for its own edit to come back, and reported three of five actions "IGNORED". Terraria
relays a tile edit with `TrySendData(17, -1, whoAmI, ...)` — everyone *except* the client that sent
it, which has already made the change locally. The real server was behaving perfectly. (Ours does
the same, checked rather than assumed: `broadcast(edit.encode()?, Some(slot))`.) The probe now uses
two clients, one acting and one watching, which is the only arrangement that can tell "the server
understood" from "the server replied to me".

Rerun that way, against the real 1.4.5.8 server:

```
request a distant section        ok       section (16, 3)
say something                    ok       from 0: "provoke: hello"
report a position                ok       slot 0 at 33840,3712
mine a tile                      ok       action 0 at 2095,232
place a tile                     ok       action 1 at 2095,232
open a chest                     ok       packet 32 (SyncChestItem)

6 of 6 actions were acted on
```

Six for six — the outbound side of the handshake, movement, tile editing, chat and chest access is
understood by Re-Logic's own code. But the comparison found one thing anyway.

**Every chat line would have shown the speaker's name twice.** The real server relayed
`"provoke: hello"`; this one relayed `"<provoke-actor> provoke: hello"`. The author's slot travels
in its own byte, and `ChatHelper.DisplayMessage` prefixes `Main.player[author].name` itself
whenever that byte is a real slot — so a server that also writes the name into the text gets it
rendered twice, and puts the tag inside the speech bubble over the player's head as well.

**The test asserted the bug.** `two_players_see_each_other_join_move_and_chat` ended with
`assert!(text.contains("bob"), "chat should carry the sender's name")` — the defect written down as
a requirement, which is the same shape as the Temple Key test in §1 that was *named* for the key
while asserting a different item. It now asserts the text is exactly what was said, and that the
author byte is what carries who said it.

Both servers now produce the identical line.

---

## 30. Widening the outbound probe, and a state-desync bug

Six actions out of the roughly forty a client can send is thin: every untested one is a request
that could be silently ignored. Pass seven doubled the matrix — walls, dropped items, NPC damage,
doors, signs — and ran it against both servers.

Eleven of twelve on the real 1.4.5.8 server (the twelfth, reading a sign, found no sign in the
sections it had loaded, so it was not tried rather than failed). Ten of twelve on ours, and the one
that differed turned out to be worth the whole exercise.

**A change to a distant NPC was dropped, permanently.** NPC state is withheld from players whose
loaded sections do not cover it — that is what keeps a busy world from flooding every connection,
and the game does the same. But the withholding has to be a *delay*, not a drop, and this was a
drop: the NPC was marked dirty, the single broadcast it earned was withheld from everyone far
away, the dirty flag was cleared regardless, and nothing ever set it again. Every distant player
kept the stale value for the rest of the session.

It showed up as hitting the Old Man at the dungeon while a second client stood at spawn: the blow
landed on the server (250 → 249, confirmed by tracing) and reached nobody. Anything that changes an
NPC once and then goes quiet — a hit, a buff, a change of mind — was affected. The fix is that
`broadcast_near` now reports whether it withheld from anyone, and the NPC stays dirty if it did.

**Two more, found on the way.** Both were the same mistake in different places, and neither is
visible from inside:

- **NPC generation was one counter for the whole server**, where the game keeps one *per slot*
  (`NewNPCInstanceInSlot` reads `Main.npc[slot].generation`, adds one, and skips zero). The number
  exists so a client can tell "slot 5, the one I was told about" from "slot 5, somebody else now".
  A global counter repeats after 256 spawns *anywhere* — minutes on a busy server — against 256
  reuses of one slot. Worse, ours wrapped straight through **zero**, which the game explicitly
  refuses to emit and a real client asserts against outright.
- **Projectile generation had it too**, against the game's `++slotGenerations[num]`. Fourteen bits
  on the wire, so ours repeated after 16384 launches anywhere rather than 16384 reuses of a slot —
  and the whole purpose of that field, as this project's own comment says, is that a kill packet
  arriving a moment late fails to match rather than destroying a bystander.

Both were spotted from a capture: the real server sent generation 0 for two fresh NPCs where this
one sent 1 and 2.

**Two false alarms, and what they cost.** The analysis script read packet 23's NPC index as a
`i16` when it is a `u8` followed by the generation byte — so it reported slot 256 and slot 513, and
made two servers that agreed look as though they disagreed. Separately, a `tracing::warn!` added to
count withheld syncs printed nothing under the test harness's subscriber, and "zero withholds" was
taken at face value through three rounds of reasoning before an `eprintln!` showed twenty-eight.
Both cost real time, and both were instrumentation rather than the thing being measured.

**The regression test for the desync does not exist, and that is deliberate.** Four versions were
written and every one passed against the *unfixed* server, for three different reasons in turn: the
observer had never moved, so the server had it at the origin and considered it near everything; the
NPC was still falling, so it re-marked itself dirty and the lost update was replaced by the next
one; and finally, no spawnable NPC in this server ever goes quiet enough for the single-update case
to be arranged at all. The last version asserted its own precondition — "no unprompted sync for two
seconds" — and failed on it honestly, which is how that was established.

A test that passes whether or not the defect is present is worse than no test, because it reads as
cover. So the fix stands on the probe, which reproduced the fault three times out of three before
and passes three out of three after, and this paragraph stands in for the guard that is missing.
Arranging one needs an NPC that can be made genuinely inert, which this server does not currently
have.

---

## 31. Our writer against Re-Logic's reader

Every check so far has run over a socket. None of them touches the other artefact this server
produces: a `.wld` file. And that direction has the same asymmetry the protocol work started with —
our reader and our writer agree with each other by construction, so a field written at the wrong
width round-trips perfectly here and is nonsense to the game.

It matters more than usual just now, because §28 added three runs of fields to the save path — the
thirteen backgrounds, the thirteen tree tops and the cloud count — and nothing we own could have
noticed if they were written wrong.

So: generate a world with no header to copy from, hand it to the real 1.4.5.8 server, and see.

```
Loading world data: 100%
Terraria Server v1.4.5.8
Listening on port 7933
```

It loads. Then the sharper question, and the one worth the setup — **let the game re-save it and
read it back**:

```
our writer  →  Re-Logic's reader  →  Re-Logic's writer  →  our reader
```

Serving the before and after files from this server and diffing packet 7 field by field:

```
46/47 fields identical
time   15717 vs 15716    one tick, from the two captures being taken a moment apart
```

Every header field survives, the three new runs included. And the part that would have been the
quietest failure:

```
chests  308 (1224 item stacks inside)   before
chests  308 (1224 item stacks inside)   after
```

**Terraria deleted none of them.** That is the check the chest-footprint fix has been waiting for
since the first pass: a record whose tiles were carved away by a later generator pass survives a
save here and is silently deleted, with its loot, the first time the *game* writes the world. Of
twenty-one worlds sampled when that was found, seventeen had orphans. This world's generator
dropped one record of the 309 it placed and kept 308, and the game agreed with all 308.

**One embarrassment worth recording.** The first attempt at this handed the real server a world it
had made itself. `terrustia.toml` sets `world_file`, so a run with only `--seed` and `--save`
*loads* that world rather than generating one — the giveaway was a "generated" world named
`The Successful Excrement` with the same spawn as the user's own. The user's file was never
written to (the original still carries its April timestamp), but the test proved nothing until it
was re-run against a config with no `world_file` in it.

---

## 32. Five minutes instead of five seconds

Every capture so far has been a few seconds long, and `conform` prints its census precisely because
a short session proves nothing about what it did not contain. So: five minutes against each server,
on the same world, with the client wandering back and forth.

**Nothing failed to decode.** 3,980 frames over 27 distinct ids from the real server, including
packets no earlier capture had ever contained — 34 projectile syncs, 32 NPC damage packets, 222
tile squares, 306 item despawns, 1,910 NPC syncs. Zero decode failures, and all eight of its packet
7s re-encoded byte-identically. The `ProjectileKey` packing in particular had never been checked
against a real frame before.

Comparing the two censuses turned up three things.

**Enemies lingered for twelve minutes instead of twelve and a half seconds.** `DEFAULT_TIME_LEFT`
was `60 * 60 * 12`; the game's `NPC.activeTime` is **750**. Fifty-seven times too long. The visible
symptom was a flying enemy that had left through the top of the world and was *still being
simulated and broadcast* five hundred and twenty-one tiles above it, five hundred and fifteen times,
at coordinates no client can draw — because nothing was going to reap it for another seven minutes.
The everyday cost is quieter and much larger: every creature that wanders away from a player holds
its slot and its share of the sync budget for twelve minutes.

**NPC syncs ran at twenty times the game's sustained rate.** This server sent every changed NPC
every six ticks — ten a second. The game has two mechanisms and had neither of them here:

* a **token bucket** (`NPC.netSpam`): a sync costs 30, the bucket drains 1 a tick and allows 3
  packets back to back, so the sustained rate is one every half second — 5 rather than 30 for a
  boss, because a boss fight is the one place a client cannot afford to guess;
* **proximity streaming** (`StreamUpdatesToNearbyPlayers`): for anything actually moving, weighted
  by how near each player is — full weight within 250 pixels, halving outward, nothing past 1500 —
  so a creature you are standing next to updates several times a second and the same creature
  across the world does not.

They only make sense together: the bucket alone would make everything jerky, and the stream alone
would not bound anything.

```
                         real   ours before   ours after
bytes from server       62008        378554       193998
frames                   3980          9516         4063
NPC syncs                1910          8805         3350
worst NPC height            —   -521 tiles     0 tiles
```

The despawn *range* was the last of it, and was fixed too: a 2000-pixel radius where the game's
`CheckActive` uses a rectangle of a screen either side of the creature — 960 by 600 half-extents,
widened by its own size. The radius was more than three times too generous vertically, keeping
things alive far above and below anyone who could possibly see them, and every one of those holds
a slot and its share of the sync budget.

```
                         real   ours before    after both    after all three
bytes from server       62008        378554        193998             180548
frames                   3980          9516          4063               3676
NPC syncs                1910          8805          3350               2962
```

Frame count is now within 8% of the real server's, from 2.4 times it. What bandwidth remains above
theirs is a difference in *shape* rather than volume — ours is NPC-heavy where theirs spreads across
many small packets, because this server does not yet produce the item churn, projectile traffic and
liquid modules that make up a third of the real server's frames on the same world.

**And one fix that had to be thrown away.** The first response to the escaped flyer was a global
"outside the world, with the game's own hundred-pixel margin, means gone". It was wrong, and the
test suite said so immediately: **King Slime legitimately spawns above the world and falls in**, so
the check killed him on the tick he was summoned. That is exactly why the game's bounds tests live
inside the one or two AI routines that need them rather than in the despawn path, and why the
runaway is the despawn *timer's* job. The bounds check is gone; the timer is correct.

---

## 33. Beating vanilla on memory and traffic

Parity was the point up to here; this section is about being *better*, and it starts from the two
columns where the head-to-head went the wrong way: idle memory and bandwidth.

### The tile array was more than half wasted

`Tile` is a comfortable value type — ten named fields, sixteen bytes with padding. Fine on the
stack, ruinous in an array five million long: every byte of the struct costs five megabytes, and the
full sixteen came to **80.6 MB** before the server had done anything.

Measured on a real world, most of those bytes are paid by tiles with no use for them:

| field | cost | tiles that need it |
|---|---|---|
| `frame_x` / `frame_y` | 20.2 MB | **1.87%** |
| `color` / `wall_color` | 10.1 MB | **0.00%** |
| `slope` | 5.0 MB | 1.17%, and it needs three bits |
| `liquid_kind` | 5.0 MB | only where there is liquid, and it needs two |

So `world/packed.rs` stores an eight-byte `PackedTile` and keeps the two rare pairs in side tables.
Frames cost 1.1 MB that way against 20.2 MB inline; paint costs nothing at all.

**The `Tile` API did not change.** `World::tile` reassembles one on the way out and `set_tile` takes
one apart on the way in, so the hundred-odd places that read `tile.frame_x` were untouched — the
whole change is one new file and four lines in `world.rs`, because `World.tiles` was private and
everything already went through two accessors.

Two details decide whether it is actually cheaper:

* A side table has to **shrink as well as grow**. One that only ever grew would end up holding an
  entry for every position that had ever been a door, and be worse than the four bytes it replaced.
* A tile with nothing in a table must not **pay a hash to find that out**. The packed byte carries a
  bit for each table, so the ninety-eight per cent that are dirt and stone pay a bit test. Getting
  this wrong in the other direction — clearing an entry that was never there — cost ten million
  hash operations during a world load and took startup from 0.2 s to 3.6 s before it was caught.

### Traffic was three separate things

A byte-level breakdown of a five-minute session, rather than a frame count, showed where it went —
and corrected a figure I had been quoting wrongly. Earlier comparisons put vanilla at 62 KB against
our 181 KB; that was a five-minute capture of ours against a **few-second** one of vanilla's. Like
for like, vanilla sends 168,000 bytes.

| | vanilla | before | after |
|---|---|---|---|
| Total | 168,000 | 180,548 | **126,756** |
| NPC syncs | 71,184 | 102,359 | **53,597** |
| Tile sections | 67,293 | 68,207 | **66,811** |
| Clock | 0 | 3,648 | 60 |

**NPC syncs.** A counter on each of the two sync paths settled it immediately: 251 full syncs
against 5 streamed ones, so the rate limiter was doing all the work and the real problem was
upstream. `dirty` was set whenever position, velocity or direction changed — every tick for anything
moving. But a client runs the same routines and extrapolates, so an NPC walking in a straight line
at a steady speed needs no packets at all. `dirty` now means *a decision a client could not have
worked out*: a turn, a new target, a change of speed beyond what gravity alone explains.

**The clock.** Once a second, and **vanilla never sends packet 18 at all** — nothing in the game's
source calls `SendData(18)`. It keeps clients right by resending packet 7 when something changes.
Ours is now once a minute: a correction against an hour of drift, at a cost not worth measuring.

**Sections.** Deflate at default level, now at best. A section is encoded once and cached, so the
extra work is paid once per section and the smaller result on every join for the life of the
server. They are the largest single thing this server sends, and are now smaller than vanilla's.

### Doors were opened sixty times a second and never actually opened

The optimisation work had been measured on a freshly generated world. Repeating it on the **user's
own world** — one with a town, and therefore houses, and therefore doors — turned up something no
synthetic world could have shown:

```
id name                    frames      bytes    avg   share
19 ToggleDoorState          18165    163,485      9   48.3%
```

**Eighteen thousand door packets in five minutes**, sixty a second, and nearly half of everything the
server sent.

The cause was a shortcut with a comment explaining itself: opening a door was broadcast as a toggle
and the server's own tiles were left alone, on the reasoning that every client would open it for
itself and the visible result would be the same. It is not the same. The *server* went on believing
every door in the world was shut, so a town NPC standing at one decided to open it, observed that
nothing had changed, and decided to open it again — for ever. The NPC never got through the door
either, which is the gameplay half of the same bug.

`world/doors.rs` now ports `WorldGen.OpenDoor` and `CloseDoor` properly. The shape is the whole of
the problem, and is why the shortcut was tempting: shut, a door is one tile wide and three tall;
open, it is **two** wide and three tall, hinged on the side it swings towards. Opening writes six
tiles and clears three.

**And one test was describing a door that cannot exist.** It built all three tiles with `frameY = 0`.
A real door carries 0, 18 and 36, and that is how both the game and this port find which tile is the
top when somebody pushes on the middle one — so the test's door had no discernible top and could not
be opened by correct code. It passed before only because the old implementation never looked at the
tiles at all. Worth noting that this server's generator places no doors, so every door in play comes
from a world Terraria wrote, with the frames Terraria gave it.

### One test was quietly racing

`spawn_npc` waited for the next NPC sync of *any* kind, which is a race with the world spawning its
own creatures. It stayed hidden while NPC state went out ten times a second — the wanted one usually
won — and surfaced the moment that rate came down to the game's. It now photographs the roster first
and waits for a slot that is not in it, keyed by slot *and* generation, since a spawn may take the
slot of something that has just died.

---

## 27. How this list is still incomplete

Stated plainly, because the pattern this document exists to break is exactly the one where a list
like this gets treated as exhaustive.

- **It was built by reading, not by playing.** Everything here came from tracing code against the
  decompiled game. A gap that only shows up in motion — a boss that soft-locks, an AI that stalls,
  a spawn that never fires — cannot be found this way. Item 11.2 is the fix. §28 is the other half
  of it: reading the decompiled source can only ever tell you what the *source you have* does, and
  the installed game turned out to be a release ahead of it.
- **Each pass found something the one before walked past, in code it had already read.** §13 (boss
  attacks that never fire) was in the same file the first pass traced; §23 and §24 (no difficulty
  or spawn scaling) were in files the second pass opened. The passes did not get more careful —
  they asked a different question each time:
  1. *Is it there?* → §1-§12
  2. *Does it produce anything?* → §13-§22
  3. *Does it produce the right amount?* → §23-§26
  4. *Does it match what the game actually sends?* → §28
  5. *Does the game act on what we send it?* → §29, §30
  6. *Does the game agree with what we write to disk?* → §31
  7. *Does it still agree after five minutes rather than five seconds?* → §32

  The third question is the nastiest of the first three, because a system with wrong magnitudes
  looks completely healthy from outside: enemies spawn, bosses fight, damage lands. The fourth and
  fifth are different in kind — they are the ones that cannot be answered by reading at all, and
  between them they found a version check that refused every current client, a packet four bytes
  short, every chat line carrying the speaker's name twice, and a change to a distant NPC being
  dropped rather than delayed. All four had been read past by earlier passes. There is no obvious sixth question, which is itself a reason not to trust
  that this list is finished.
- **Still unchecked:** fishing mechanics, golf, dyes, painting, pets, mounts, minecart tracks, and
  the cosmetic layer. Boss phase transitions and whether any AI routine can stall. NPC spawn *pool*
  composition — the rates are right now, but not necessarily what appears.
- **Fixing things creates gaps of its own.** Both pass-four findings existed only because of the
  pass-three repairs: scaling introduced a path that dropped scaling, and making projectiles exist
  made their damage matter for the first time. A fix is a change, and a change deserves the same
  suspicion as the code it replaced.
- **I got §1 wrong twice while writing the first pass.** First I searched only `npc_drops.rs` and
  concluded no boss dropped anything, missing `conditional_drops.rs` entirely. Then I searched for
  the Pwnhammer as item 274, which is not its id. The finding survived both mistakes, but it is a
  fair measure of how much a single grep is worth.

**The structural fix is item 11.2.** Until a bot can start with nothing and kill Moon Lord, any
statement that this server is complete is an inference from subsystem coverage — which is precisely
the reasoning that produced §1 through §4, and then §13.
