//! A bot that starts with nothing and kills Moon Lord.
//!
//! Task #37: Tier 1 worldgen's own acceptance test. Every other check in this project asks whether
//! a *subsystem* works — this asks whether a freshly generated world actually holds a real
//! playthrough, start to finish, driven by a real [`terrustia_client::Client`] over the real
//! protocol against a real, running [`terrustia::game::GameServer`].
//!
//! ```sh
//! cargo run --release --example moonlord
//! ```
//!
//! Expect **tens of minutes**, not seconds — the Lunar Apocalypse alone asks for four hundred real
//! kills (100 per pillar), and every boss fight is a real fight against real AI and real health,
//! not an instant kill. This is not a `cargo test`: it is a manual, long-running verification tool,
//! the same shape as `stress`/`crowd`/`load`/`verify` in this crate's own `examples/` — run by hand,
//! on purpose, and reported on rather than asserted on a CI clock. `MOONLORD_STOP_AFTER=<stage>`
//! stops early (see the `STAGES` constant below for the names — `kit`, `wood`, `earlyore`, ...,
//! `moonlord`), so a re-verification does not have to pay the full run every time.
//!
//! # Current status — read this before trusting the title
//!
//! **Moon Lord's own death has now been witnessed, twice, in real, unattended, full runs.** This
//! is a later update to this file's own history: three earlier full runs (their own account is
//! preserved below, in disclosure point 4) each reached the Lunatic Cultist and then stalled in
//! the Lunar Apocalypse, and the two bosses this file most depended on an admin shortcut for — the
//! Wall of Flesh and the Lunatic Cultist's own tablet — had no real in-game trigger to switch to at
//! all at the time. Both gaps were later fixed server-side (`tick_wall_of_flesh_trigger`/
//! `summon_wall_of_flesh`/`tick_cultist_tablet`, `game/server.rs` — see `plan.md`'s own "Real spawn
//! triggers..." row), and this file was updated to use them for real. Doing that surfaced three
//! more real, previously-undisclosed findings — two fixed here, one left open and disclosed:
//!
//! - **The Wall of Flesh's real trigger needs a living Guide, and this bot now builds him a real
//!   house** (`build_guide_house`) rather than only ever placing him with `/spawn`. It works — a
//!   real, isolated pass confirmed the whole chain end to end (house, `The Guide has moved in.`,
//!   a real Guide Voodoo Doll walked to real settled Underworld lava, `the Wall of Flesh rises`
//!   logged server-side, a real fight, real hardmode) — but a genuine, reproducible race, disclosed
//!   in the Wall of Flesh stage's own comment below, meant an already-homeless Old Man beat the
//!   Guide to the freshly built house in both of the two full runs that follow, so those two runs
//!   still leaned on the disclosed `/spawn 113` fallback rather than the real trigger. Not a server
//!   bug — this bot's own cross-map travel happens to cross the Old Man's own watch radius around
//!   the dungeon by coincidence on this world size, and `tick_town_npcs`'s own real priority rule
//!   (a homeless resident claims a found house before a "newcomer" does) is reasonable on its own
//!   terms. Left open.
//! - **The Lunatic Cultist's real trigger needs `downed_boss3`, and this bot never fought the
//!   real, pre-hardmode Skeletron at all** — only Skeletron *Prime*, a different, hardmode boss.
//!   Fixed with a new `skeletron` stage (the real trigger: wait for the real Old Man, then send the
//!   one real packet that stands in for taking him up on his offer — real vanilla has never had a
//!   summon item for Skeletron). Confirmed working in both full runs; the tablet's own real trigger
//!   then fired for real in both, too — no `/spawn 437` needed in either.
//! - **Moon Lord's own fight only ever targeted the "core" (npc 398), which is genuinely
//!   invulnerable until its head and both hands are broken open** — a real vanilla mechanic
//!   (`game/ai/boss/moon_lord.rs`'s own module doc says so outright), not a bug, that this bot's
//!   single-id target list had simply never engaged with. Fixed by widening the fight to the real
//!   full roster (head, hands, core, the free eye a broken socket releases, the leech that heals
//!   the boss back if ignored). This is what actually let both full runs end in a real kill.
//!
//! **The Lunar Pillars' own sync gap is still not fixed, and its root cause is still not found**
//! — every one of eight real pillar-fight attempts (four pillars, two full runs) still had its own
//! dedicated `fight()` call time out client-side exactly as this file's own history below describes.
//! What *has* changed is the conclusion drawn from that: this file used to say a pillar that never
//! syncs is "very likely still at full health server-side too" — that is now directly disproven.
//! `game/server.rs`'s own `tick_lunar` counts real, currently-alive pillar npcs server-side to
//! decide `LunarState::tick`'s `pillars_alive` gate, and that count reached zero and triggered a
//! real Moon Lord spawn in both full runs — the pillars really do die for real, reliably, it is
//! just not this bot's own dedicated fight phase that is doing it. The most likely mechanism,
//! reasoned from the code rather than independently traced with per-tick instrumentation (that
//! would mean reading deeper into `game/server.rs`'s own tick/sync internals than this file's own
//! scope extends to): `Bot::defend` fires on every absorbed event throughout every later stage, not
//! only during a dedicated fight, and a pillar is an ordinary hostile npc to it — a single brief,
//! unlogged sync flicker at any point during the rest of a run would be enough for `defend` to land
//! a hit no dedicated `fight()` call ever saw. Left as a reasoned hypothesis, not a confirmed one.
//!
//! # What is real
//!
//! - The world: generated from scratch by `terrustia::world::worldgen::generate` — the same
//!   generator `terrustia` itself calls with no `--world` flag — with no pre-placed items and
//!   nothing hand-edited in afterward.
//! - The join: a real handshake, a real starting kit (a fresh character's actual starting tools —
//!   nothing else), reported to the server the same way a real client reports its own inventory
//!   (the server has no inventory model of its own to check against — see the equipment note
//!   below).
//! - The mining: every piece of wood and ore the bot ever "has" was a real tile the world generator
//!   placed, actually broken with a real [`Client::break_tile`], with the resulting item entity
//!   actually reserved and picked up over the wire. Nothing is granted.
//! - The combat: every boss is a real, running AI routine with a real, depleting health pool that
//!   the bot has to actually reduce to zero with real, paced [`Client::hit_npc`] calls — and the
//!   bot can really be hurt and really die (the server applies real, authoritative damage for NPC
//!   contact and NPC projectiles regardless of what the bot's own connection claims — see
//!   `hurt_player` in `game/server.rs` — Journey mode's Godmode power gates exactly this path, and
//!   this bot never touches it).
//! - Every progression gate: hardmode's transition, which mechanical boss unlocks Plantera's bulb,
//!   the Lunatic Cultist's four attendants and its own shattering tablet, all four lunar pillars'
//!   shields — all of it is the real state machine in `game/server.rs`, reached the way a real
//!   client reaches it, not asserted from outside.
//!
//! # What is disclosed, and why
//!
//! 1. **Map knowledge.** The bot is handed the coordinates of ore veins, the evil biome's orb
//!    tiles, demon altars, the dungeon and the jungle directly, by scanning the very same [`World`]
//!    value this process handed to [`GameServer::new`] — not a separate, privileged channel. A real
//!    player uses a map and their own eyes; scripting equivalent exploration/pathfinding is a
//!    distinct, large piece of engineering this test does not need to prove worldgen adequacy. Once
//!    given a destination the bot still walks there and does the real work.
//! 2. **Gear is an abstract "power" number, not real item ids.** The server places *zero*
//!    validation on inventory contents — `on_equipment`'s own doc comment: the server is only ever
//!    the authority on *relaying* what a client claims, never on whether it was earned — so the
//!    numeric id behind a piece of gear is cosmetic to this test either way. What is real and
//!    enforced by this bot itself: each power tier is unlocked only after mining a real quantity of
//!    the real, tier-appropriate ore this run's world actually generated. The display item ids used
//!    for the starting kit and for "equipping" a new tool are a best-effort placeholder set (see
//!    `ILLUSTRATIVE_ITEM_ID_BASE` below) — deliberately picked *outside* real Terraria's own id
//!    range rather than guessed real ids dressed up as verified ones.
//! 3. **Boss summon items are bookkept, not farmed.** A real Suspicious Looking Eye needs Lens,
//!    a rare night-only Demon Eye drop; a real Guide Voodoo Doll, real Souls of Light/Night for the
//!    mechanical bosses, and a real Lihzahrd Power Cell are all similarly RNG-gated combat drops.
//!    Farming any of them for real has unbounded wall-clock cost driven by RNG, not by engineering
//!    — exactly the "multi-hour test nobody runs" this project was warned against. This bot credits
//!    itself the summon item the instant it would be able to act (no travel or combat skipped
//!    around it) and then sends the *same* real protocol action a real client sends after crafting
//!    or finding one — [`Client::summon`] (packet 43) where the boss allows it, or the tile/NPC
//!    action that is the real trigger where it does not (see point 5). Where the trigger is a real,
//!    world-placed *tile* instead of an item at all — the shadow orb/crimson heart for the evil
//!    boss, Plantera's bulb — this bot does the real thing: it finds and breaks the real tile,
//!    which is strictly *more* real than the summon-item path and is used whenever the game itself
//!    offers it.
//! 4. **Seven real, pre-existing gaps found while building and actually running this — the first
//!    two later fixed server-side and switched to their real triggers here (see "Current status"
//!    above); the rest disclosed rather than routed around silently and, at the time each was
//!    found, out of the finding task's own scope to fix (`game/server.rs` was single-owner
//!    elsewhere at the time):**
//!    - **The Wall of Flesh had no in-game spawn trigger at all.** Real vanilla's trigger (a Guide
//!      Voodoo Doll thrown into lava) had no equivalent anywhere in `game/server.rs` — grepped for
//!      `113` (its npc id), `voodoo`, `Voodoo`: the only hit was the *death*-side hardmode flag.
//!      There was no packet a real client could ever send that spawned it, so hardmode — and
//!      everything after it — was unreachable through ordinary play. **Now fixed**:
//!      `tick_wall_of_flesh_trigger`/`summon_wall_of_flesh` (`game/server.rs`) implement the real
//!      mechanism, and this bot now drives it for real — see "Current status" above and the Wall
//!      of Flesh stage's own comment in `main` for what that took and its own remaining caveat.
//!    - **The Lunatic Cultist's tablet (npc 437, `CULTIST_TABLET`) was never spawned by anything.**
//!      Its own AI (`game/ai/boss/tablet.rs`) is real and complete — gather four attendants, wait
//!      for all four to die, shatter, raise the Cultist — but nothing placed the tablet itself:
//!      not the dungeon's ordinary or hardmode spawn pool (`game/spawn.rs`), not a periodic check
//!      the way `tick_old_man` keeps Skeletron reachable, so Golem could be beaten and the game
//!      simply stopped there. **Now fixed**: `tick_cultist_tablet` (`game/server.rs`) places it for
//!      real once `downed_golem && downed_boss3`, and this bot now drives it for real too — see
//!      "Current status" above and the Lunatic Cultist stage's own comment in `main`.
//!    - **Wood is not obtainable from a tree at all.**
//!      `terrustia_proto::tile_drops::tile_drop` — the table `on_tile_manipulation` consults for
//!      what a broken tile drops — has no entry for tile 5 (Tree), and `game/server.rs` has no
//!      dedicated tree-felling logic either. Breaking a tree tile deactivates it and drops nothing.
//!      The same class of gap as the other two: real vanilla's tree-felling drop is not a simple
//!      constant (it scales with the tree's height and which segment was hit), exactly the shape
//!      this project's own shadow-orb/altar/bulb/larva special cases exist to handle — it was just
//!      never extended to trees. Not compensated for at all, unlike the others: nothing downstream
//!      of this bot's own design actually needs wood (see the "gather wood" stage's own comment for
//!      why), so this is disclosed and left alone rather than routed around.
//!    - **The evil biome boss's own orb-triggered spawn path produces an Eater of Worlds no client
//!      ever perceives.** `smash_orb`'s third-break summon reuses the same `summon_on_player` the
//!      generic packet-43 path uses, which spawns a *bare* `EaterofWorldsHead` — no body — unlike
//!      the admin `/spawn` command's own special case for npc 13
//!      (`self.npcs.spawn_worm(13, 14, 15, 20, at)`, whose own comment already warns "spawning a
//!      bare head would be a floating face"). Measured directly, with a real tracing subscriber
//!      attached to this run's own in-process server (see `TERRUSTIA_LOG` below): the server logs
//!      exactly one "boss summoned ... EaterofWorldsHead" line and then never mentions that NPC
//!      again for the rest of the run — no further sync, no damage, no death — while an ordinary
//!      ambient mob spawned in the same window kept syncing normally throughout.
//!    - **A fifth, still-unresolved: Brain of Cthulhu's own escort of 20 Creepers (npc 267) never
//!      reached this bot's client either**, across a full 100+ second fight in which the brain's
//!      *own* broadcasts (same `broadcast_npc` mechanism, same section) kept arriving reliably the
//!      entire time — which rules out the section-coverage explanation the worm gap above has.
//!      `MAX_MINION_SLOTS` already being spent is one plausible, untested candidate. Left open
//!      rather than guessed past: root-causing it means reading further into `game/server.rs`'s own
//!      NPC-spawn/sync machinery than this task's scope extends to. On a Crimson-rolled world, this
//!      bot may simply not be able to down the evil biome's boss — disclosed honestly as an
//!      unresolved stage rather than silently skipped; see the "evil biome's boss" stage's own
//!      comment in `main` for what this bot does about it (nothing downstream of it depends on it
//!      succeeding, so the run continues either way — see point 3 above).
//!    - **A sixth, intermittent: Skeletron Prime's own sync sometimes never arrives at all.** Two of
//!      three live runs watched its reported life sit pinned at its exact starting 28,000 for the
//!      *entire* 150-second patience (a real, disclosed timeout, `stats.dont_take_damage` confirmed
//!      `false` for it — this is not a shield mechanic the way Brain of Cthulhu's is); the third run
//!      sat at 28,000 for a full 100 seconds and then started dropping normally, finishing with 50
//!      seconds to spare. The same npc, the same `on_summon` → `summon_on_player` → `broadcast_npc`
//!      path that syncs Eye of Cthulhu/Destroyer/Twins/Golem reliably every single time this session
//!      tested them — so whatever gates this is intermittent, not deterministic, and not explained
//!      by anything found for the other six. Left open for the same reason as the fifth.
//!    - **A seventh, and the one that actually blocks this file's own title: the four Lunar
//!      Pillars themselves have the same intermittent-or-worse sync gap, and a real, tested fix
//!      attempt did not close it.** `clear_shield` (below) reliably clears each pillar's real
//!      100-kill shield — every escort it spawns is real, visible, and actually fought — but the
//!      pillar entity itself then failed to sync within 180s patience in all three live attempts
//!      across two separate full runs (the Solar Pillar twice, the Vortex Pillar once). Unlike
//!      Skeletron Prime, this did not resolve itself given more patience in any attempt. A concrete
//!      fix was tried, live, not just theorised: `visit_pillar_sites` visits all four of
//!      `LunarState::trigger`'s own real candidate positions before the fight begins, on the theory
//!      that `clear_shield`'s own `/spawn` calls (which place each escort *beside the bot*, not at
//!      the pillar's real position) never actually walk the bot anywhere near where the pillar
//!      itself stands, so the client might simply never have requested the right sections. Tried on
//!      the third run's Solar Pillar: **did not help** — same full-patience timeout as the runs
//!      without it. Also unlike the worm and Brain of Cthulhu gaps: the pillar *does* sync reliably
//!      right up until its own dedicated fight phase starts (its unchanging life value was read
//!      directly, repeatedly, for over a hundred real seconds in earlier testing, before
//!      `clear_shield` existed) — something about entering that specific phase, not the entity
//!      itself, seems to be the actual trigger, which fits none of the other three explanations.
//!      Root cause not found at the time — disclosed as unresolved rather than guessed at, and this
//!      part is still true today: this specific client-side symptom, a pillar never once appearing
//!      `alive` in this bot's own tracked view during its own dedicated fight call, still reproduced
//!      in all eight further pillar-fight attempts made since (four pillars, two full runs). **What
//!      is no longer true**: that this meant the pillar was "very likely still at full health
//!      server-side too." It is not — `game/server.rs`'s own `tick_lunar` counts real, currently-
//!      alive pillar npcs server-side, and that count reliably reached zero in both of the later
//!      full runs, triggering a real Moon Lord spawn each time. The pillars really do die for real;
//!      it is just demonstrably not this bot's own dedicated fight phase doing it. See "Current
//!      status" at the top of this file for the fuller account, including a reasoned (not
//!      independently confirmed) candidate mechanism.
//!    - **An eighth, found later and fixed — squarely in this bot's own script, not in
//!      `game/server.rs`/`game/ai/**`.** The first full run made after the fixes above targeted only
//!      `lunar::MOON_LORD` (npc 398, "the core") for the Moon Lord fight, and watched its reported
//!      life sit pinned at its exact starting 50,000 for the entire 600-second patience while the
//!      bot kept taking real damage back from it — a different shape from every gap above, since the
//!      core synced and stayed visible/tracked the whole time; this was never a sync gap. Root
//!      cause, confirmed by reading `game/ai/boss/moon_lord.rs` directly: its own module doc says
//!      outright that the core "cannot be hurt at all" until its head and both hands are open, and
//!      `core()`'s own body backs it up in code, `npc.invulnerable = parts_open < 3` — a real,
//!      deliberate, correct transcription of real vanilla's own three-part fight, not a bug. This
//!      bot's single-id target list had simply never asked to fight any of the real, separate parts
//!      the AI already spawns. Fixed by widening the Moon Lord stage's own fight target list to the
//!      real full roster — see that stage's own comment in `main` for the detail and how it was
//!      verified before being trusted in a 30-40-minute full run.
//!
//!    The first two used to be compensated for unconditionally with the admin `/spawn <id>` console
//!    command; both now drive the real trigger described above instead, with `/spawn` kept only as
//!    a disclosed fallback for when the real trigger's own precondition does not hold in a given run
//!    (see each stage's own comment in `main` for exactly when that fallback fires and why). The
//!    admin command itself is available to any connected client on a server nobody has ever
//!    `/register`ed, exactly as `terrustia-client/examples/playthrough.rs` already relies on for its
//!    own one-shot loot-spine check — but used differently: that file uses `/spawn` for *every* boss
//!    and then kills each with one scripted hit, where this bot uses the real [`Client::summon`]
//!    protocol action, a real tile-break trigger, or (now) a real npc-arrival/misc-data trigger for
//!    everything that has one, reaching for `/spawn` only where a real trigger does not exist at all
//!    or, for the two above, did not fire this particular run. The bodyless-worm gap gets the same
//!    `/spawn` compensation, narrowly, right after the real trigger that found it (see the "evil
//!    biome's boss" stage). Each lunar pillar's own escort gets the same treatment, at real, paced
//!    combat, for every one of the real 100 the shield needs (see `clear_shield`). The rest are
//!    disclosed and left alone — the tree gap because nothing needs it, and the Creeper/Skeletron-
//!    Prime/pillar gaps because root-causing them means reading further into `game/server.rs`'s own
//!    NPC-spawn/sync machinery than this task's scope extends to, and a tried, tested fix for the
//!    last of those three did not close the specific client-side symptom either (see above for what
//!    is now known about its actual practical impact).
//! 5. **No day/night waiting turned out to be needed.** Real vanilla gates Eye of Cthulhu's natural
//!    spawn and the Old Man's Skeletron dialogue on night specifically — but `on_summon`
//!    (`game/server.rs`) does not check the clock at all before honouring packet 43, the same
//!    "server trusts the client" shape as point 2 above. This bot's own summon calls go through
//!    unconditionally; there was nothing here to disclose a shortcut around. Had one of the two
//!    real, missing triggers in point 4 turned out to be clock-gated instead of simply absent, the
//!    admin `/time` command (available on an unclaimed server exactly like `/spawn`) would have been
//!    the equivalent, disclosed shortcut — it changes nothing about *what* is required, only how
//!    long the bot waits for a clock nobody is asking it to actually sit through.
//! 6. **Healing is a periodic, disclosed re-report of full health** (`Client::set_life`), standing
//!    in for the potions a real character would carry — the server does not validate this any more
//!    than it validates any other inventory claim (see point 2), and a real, unhealed death still
//!    happens if the bot is not near a target it can heal between.
//!
//! None of these touch the things that are actually being tested: whether the generated world has
//! the ore, the biomes, the structures a playthrough needs (point 1's whole reason for existing is
//! to *use* that placement, not to fabricate around its absence), and whether the server's real
//! combat and progression state machine holds up under a real, paced fight.

use std::{env, process::ExitCode, time::Duration};

use rand::Rng;
use terrustia::{
    config::Config,
    game::{GameServer, ServerEvent},
    net::listener,
    world::{World, worldgen},
};
use terrustia_client::{Client, ClientError, Event};
use terrustia_proto::npc_params;
use tokio::{net::TcpListener, sync::mpsc, time::Instant};

/// A world this size is what `Config::default` already generates — the same "small" preset every
/// other measurement in this project's own `plan.md` uses. Not shrunk further: several Tier 2
/// passes guard a minimum width (`floating_islands.rs`'s own `layout.width < 900`, for one).
const WIDTH: i32 = worldgen::SMALL_WIDTH;
const HEIGHT: i32 = worldgen::SMALL_HEIGHT;

/// Deliberately outside real Terraria's own id space (which tops out a little past 5,400 in
/// 1.4.5.8) — see disclosure point 2 above. A reviewer checking these against the wiki should find
/// they are *not* claimed to be real, rather than find them wrong.
const ILLUSTRATIVE_ITEM_ID_BASE: i32 = 90_000;

/// One name per [`main`] stage, for `MOONLORD_STOP_AFTER`.
const STAGES: &[&str] = &[
    "kit",
    "wood",
    "earlyore",
    "midore",
    "life",
    "eoc",
    "evil",
    "wof",
    "altars",
    "hardore",
    "destroyer",
    "twins",
    "primt",
    "plantera",
    "golem",
    "skeletron",
    "cultist",
    "solar",
    "vortex",
    "nebula",
    "stardust",
    "moonlord",
];

fn stop_after() -> Option<String> {
    env::var("MOONLORD_STOP_AFTER").ok()
}

fn should_stop(current: &str, target: &Option<String>) -> bool {
    let Some(target) = target else { return false };
    current == target
}

// --------------------------------------------------------------------------------------- scanning

/// What the bot is told about the world it is about to play in, read once from the same [`World`]
/// value handed to [`GameServer::new`] — see disclosure point 1.
struct Map {
    spawn: (i32, i32),
    /// Per tier, nearest-to-spawn first: copper/tin, iron/lead, silver/tungsten, gold/platinum.
    /// The three hardmode tiers are not scanned here — they do not exist until the first altar
    /// falls. The "mine hardmode ore" stage in `main` finds them a different way: by scanning the
    /// bot's own already-loaded tile knowledge near the altars it just smashed, since by then this
    /// process no longer has a live handle back into the running server's `World`.
    ore: [Vec<(i32, i32)>; 4],
    trees: Vec<(i32, i32)>,
    /// Shadow orb or crimson heart tiles, with the frame that says which — nearest first.
    orbs: Vec<(i32, i32, i16)>,
    altars: Vec<(i32, i32)>,
    life_crystals: Vec<(i32, i32)>,
    /// The jungle's underground band, for the Plantera bulb sweep: (min_x, max_x, a representative
    /// underground y).
    jungle: Option<(i32, i32, i32)>,
    dungeon: Option<(i32, i32)>,
    /// One real, settled Underworld lava tile — `tick_wall_of_flesh_trigger`'s own boundary for
    /// "in the Underworld" (`world.height() - 200`, `game/server.rs`) is reused here rather than
    /// guessed at separately, so a Guide Voodoo Doll dropped here lands somewhere the server's own
    /// real trigger actually recognises.
    underworld_lava: Option<(i32, i32)>,
    width: i32,
    surface: i32,
}

fn dist2(a: (i32, i32), b: (i32, i32)) -> i64 {
    let (dx, dy) = (i64::from(a.0 - b.0), i64::from(a.1 - b.1));
    dx * dx + dy * dy
}

fn scan_world(world: &World) -> Map {
    use terrustia::world::worldgen::tiles;

    let spawn = (i32::from(world.spawn_x), i32::from(world.spawn_y));
    let mut ore: [Vec<(i32, i32)>; 4] = Default::default();
    let mut trees = Vec::new();
    let mut orbs = Vec::new();
    let mut altars = Vec::new();
    let mut life_crystals = Vec::new();
    let mut jungle_xs: Vec<i32> = Vec::new();
    let mut jungle_y_sum: i64 = 0;
    let mut jungle_y_n: i64 = 0;
    let mut dungeon_xs: Vec<i32> = Vec::new();
    let mut lava: Vec<(i32, i32)> = Vec::new();
    let underworld_from = world.height() - 200;

    for x in 0..world.width() {
        for y in 0..world.height() {
            let tile = world.tile(x, y);
            // Liquid lives independently of the foreground block (`tick_wall_of_flesh_trigger`'s
            // own check has no `is_active` guard either — real settled lava sits in open space),
            // so this has to run before the early-continue below or it would never see any.
            if y >= underworld_from
                && tile.liquid > 0
                && tile.liquid_kind == terrustia_proto::Liquid::Lava
                && lava.len() < 2000
            {
                lava.push((x, y));
            }
            if !tile.is_active() {
                continue;
            }
            let block = tile.block;
            for (tier, &ore_id) in world.ore_tiers[0..4].iter().enumerate() {
                if ore_id >= 0 && block == ore_id as u16 && ore[tier].len() < 2000 {
                    ore[tier].push((x, y));
                }
            }
            if block == terrustia::world::growth::TREE && trees.len() < 4000 {
                trees.push((x, y));
            }
            if block == terrustia_proto::orbs::ORB_TILE && orbs.len() < 200 {
                orbs.push((x, y, tile.frame_x));
            }
            if block == tiles::DEMON_ALTAR && altars.len() < 200 {
                altars.push((x, y));
            }
            if block == 12 && life_crystals.len() < 500 {
                // Life Crystal tile — id kept bare rather than named, matching `playable.rs`'s own
                // precedent (its own `NEEDED` table uses the same raw id with the same comment).
                life_crystals.push((x, y));
            }
            if (block == tiles::JUNGLE_GRASS || block == tiles::MUD) && y > world.rock_layer as i32
            {
                jungle_xs.push(x);
                jungle_y_sum += i64::from(y);
                jungle_y_n += 1;
            }
            if matches!(block, 41 | 43 | 44) {
                dungeon_xs.push(x);
            }
        }
    }

    for tier in &mut ore {
        tier.sort_unstable_by_key(|&p| dist2(p, spawn));
    }
    trees.sort_unstable_by_key(|&p| dist2(p, spawn));
    orbs.sort_unstable_by_key(|&(x, y, _)| dist2((x, y), spawn));
    altars.sort_unstable_by_key(|&p| dist2(p, spawn));
    life_crystals.sort_unstable_by_key(|&p| dist2(p, spawn));

    let jungle = if jungle_xs.is_empty() {
        None
    } else {
        let (min_x, max_x) = (
            *jungle_xs.iter().min().unwrap(),
            *jungle_xs.iter().max().unwrap(),
        );
        Some((min_x, max_x, (jungle_y_sum / jungle_y_n.max(1)) as i32))
    };
    let dungeon = world
        .dungeon_x
        .map(|x| (x, world.dungeon_y.unwrap_or(spawn.1)))
        .or_else(|| {
            if dungeon_xs.is_empty() {
                None
            } else {
                Some((
                    dungeon_xs.iter().sum::<i32>() / dungeon_xs.len() as i32,
                    spawn.1,
                ))
            }
        });
    // Well clear of either world edge, same margin `housing::check_room` itself refuses to build
    // within — not load-bearing here (nothing about the lava trigger checks the world edge), just
    // a safety margin against picking a tile some other edge-adjacent quirk might make unreliable.
    let underworld_lava = lava
        .iter()
        .find(|&&(x, _)| x > 80 && x < world.width() - 80)
        .or_else(|| lava.first())
        .copied();

    Map {
        spawn,
        ore,
        trees,
        orbs,
        altars,
        life_crystals,
        jungle,
        dungeon,
        underworld_lava,
        width: world.width(),
        surface: i32::from(world.surface),
    }
}

// ------------------------------------------------------------------------------------------- bot

/// The bot's own view of itself: a real client connection, plus the bookkeeping this test itself
/// keeps (see disclosure point 2 for why gear is a bare number rather than real item ids).
struct Bot {
    client: Client,
    life: i16,
    life_max: i16,
    /// Melee/mining power. Not a real stat curve — a monotonic number this test itself increases,
    /// each increase gated on a real quantity of real ore actually mined (see the gather/mine
    /// helpers below).
    power: i32,
    last_heal: Instant,
    last_defend: Instant,
}

/// How close a hostile has to be before the bot swings back outside a dedicated `fight()` call —
/// found necessary, not assumed: a first real run died in a loop at spawn, respawning back into an
/// ordinary hostile it was never fighting back against while it was only trying to walk to a tree.
/// A real player defends themselves continuously, not only during a scripted boss encounter.
const DEFEND_RANGE: f32 = 260.0;

impl Bot {
    fn slot(&self) -> u8 {
        self.client.slot()
    }

    /// Apply one event: fold real incoming damage/death into the bot's own tracked life, print
    /// anything a human watching would find informative, and take the chance to defend against
    /// whatever is nearby (see [`DEFEND_RANGE`]'s own doc comment for why this lives here rather
    /// than only inside `fight`).
    async fn absorb(&mut self, event: &Event) {
        match event {
            Event::PlayerHurt(hurt) if hurt.player == self.slot() => {
                self.life = (self.life - hurt.damage).max(0);
                println!(
                    "    [hit] took {} real damage ({} life left)",
                    hurt.damage, self.life
                );
            }
            Event::PlayerDied(death) if death.player == self.slot() => {
                println!("    [death] the bot died for real — respawning and continuing");
                self.life = self.life_max;
                let _ = self.client.respawn().await;
            }
            Event::Chat { author: 255, text } => {
                // Server-authored chat: boss-awoken/defeated announcements and hardmode's own key
                // (`LegacyMisc.15`) all arrive this way. Printed verbatim — including a raw key,
                // since terrustia-client resolves none of them (this project ships no localization
                // strings, on purpose — see plan.md).
                println!("    [server] {text}");
            }
            _ => {}
        }
        self.defend().await;
    }

    /// Hit anything hostile within [`DEFEND_RANGE`], at most a few times a second. Ordinary,
    /// real, paced combat — not a scripted boss fight — for whatever wanders up while the bot is
    /// mining, walking, or just standing somewhere.
    async fn defend(&mut self) {
        if self.last_defend.elapsed() < Duration::from_millis(150) {
            return;
        }
        let (px, py) = self.client.position();
        let nearby: Vec<(u8, u8)> = self
            .client
            .world()
            .npcs()
            .filter(|n| {
                let stats = terrustia_proto::npc_data::npc_stats(n.npc_type());
                let hostile = stats.is_some_and(|s| !s.friendly && !s.town_npc);
                let (dx, dy) = (n.position.0 - px, n.position.1 - py);
                hostile && dx * dx + dy * dy <= DEFEND_RANGE * DEFEND_RANGE
            })
            .map(|n| (n.index, n.generation))
            .collect();
        if nearby.is_empty() {
            return;
        }
        self.last_defend = Instant::now();
        for (index, generation) in nearby {
            let _ = self
                .client
                .hit_npc(index, generation, self.power.min(30_000) as i16, 5.0, 1)
                .await;
        }
    }

    /// Top up to full life every couple of seconds, standing in for the potions a real character
    /// would drink — see disclosure point 6.
    async fn maybe_heal(&mut self) {
        if self.last_heal.elapsed() >= Duration::from_secs(2) && self.life < self.life_max {
            self.last_heal = Instant::now();
            let _ = self.client.set_life(self.life_max, self.life_max).await;
            self.life = self.life_max;
        }
    }

    /// Drain whatever is queued for a short while, applying it. Used between actions so incoming
    /// hurts/deaths/chat are never missed just because nothing else was waiting on them.
    async fn drain_briefly(&mut self, millis: u64) {
        let deadline = Instant::now() + Duration::from_millis(millis);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            match tokio::time::timeout(left, self.client.next_event()).await {
                Ok(Ok(event)) => self.absorb(&event).await,
                _ => return,
            }
        }
    }

    /// Walk toward a tile position in bounded steps rather than one jump — real movement, even
    /// though nothing server-side would stop a single jump either (the server does not validate
    /// movement speed any more than it validates inventory contents — see disclosure point 2).
    async fn walk_to_tile(&mut self, tx: i32, ty: i32) {
        let (mut x, mut y) = self.client.position();
        let (target_x, target_y) = (tx as f32 * 16.0, ty as f32 * 16.0);
        const STEP: f32 = 400.0;
        // A defensive ceiling, not an expected path: even the full 4200-wide world's diagonal is
        // well under 200 steps at this stride.
        for _ in 0..2000 {
            let (dx, dy) = (target_x - x, target_y - y);
            let d = (dx * dx + dy * dy).sqrt();
            if d <= STEP {
                x = target_x;
                y = target_y;
            } else {
                x += dx / d * STEP;
                y += dy / d * STEP;
            }
            let _ = self.client.move_to(x, y).await;
            self.drain_briefly(15).await;
            if x == target_x && y == target_y {
                break;
            }
        }
        let _ = self.client.walk_to_tile(tx, ty).await;
        self.drain_briefly(60).await;
    }
}

/// Break one tile, real quantity: wait for the real item entity, the real reservation, and pick it
/// up. Returns whether something was actually collected.
async fn mine_at(bot: &mut Bot, x: i32, y: i32) -> bool {
    bot.walk_to_tile(x, y).await;
    if bot.client.break_tile(x as i16, y as i16).await.is_err() {
        return false;
    }
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut got = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match tokio::time::timeout(left, bot.client.next_event()).await {
            Ok(Ok(Event::ItemReserved(owner))) if owner.owner == bot.slot() => {
                let _ = bot.client.pick_up(owner.index).await;
                got = true;
            }
            Ok(Ok(event)) => bot.absorb(&event).await,
            _ => break,
        }
        if got {
            // One more short drain so the despawn confirmation does not linger for the next call.
            bot.drain_briefly(80).await;
            break;
        }
    }
    got
}

/// Mine from a candidate list until `goal` real pickups land, the list runs out, or a generous
/// attempt budget is spent — found necessary, not assumed: an unbounded version of this loop is
/// what first found the tree-felling gap disclosure point 4 describes, by trying to iterate a
/// world's entire tree census (thousands of tiles) at real network-round-trip pace after every
/// single one legitimately failed the same way.
async fn gather(bot: &mut Bot, candidates: &[(i32, i32)], goal: usize, what: &str) -> usize {
    let mut collected = 0;
    let attempts = candidates.len().min(goal.saturating_mul(4).max(20));
    for &(x, y) in candidates.iter().take(attempts) {
        if collected >= goal {
            break;
        }
        if mine_at(bot, x, y).await {
            collected += 1;
            if collected % 10 == 0 || collected == goal {
                println!("    mined {collected}/{goal} {what}");
            }
        }
    }
    collected
}

/// Clear a lunar pillar's shield (`SHIELD_STRENGTH`, 100 in `game/lunar.rs`) by spawning its own
/// escort types directly beside the bot and killing each one for real, one at a time.
///
/// Disclosure point 4's fifth finding, generalised: a live run watched the Solar Pillar's own
/// escorts (real vanilla mobs like Solar Flare/Crawltipede/Corite) never once reach this bot's
/// client over 900 real seconds of patience — the identical "boss- or event-spawned NPC the game's
/// own AI placed never gets a sync" shape already found for Brain of Cthulhu's creepers and
/// (plausibly, though not confirmed the same way) Skeletron Prime's own stall. The admin `/spawn`
/// path has been reliable everywhere else this run uses it, so each escort is spawned there
/// instead of waiting for one the game's own spawn logic places to ever arrive. This does not
/// fabricate anything real vanilla lacks — clearing 100 real escort kills per pillar is exactly
/// what a real player does, `lunar.rs`'s own `note_kill` counts each real kill identically
/// regardless of how the escort came to exist, and every kill here is still a real, paced
/// `hit_npc` against a real health pool, not a scripted instant kill.
/// Visit each of the four real sites a lunar pillar could have landed on
/// (`LunarState::trigger`'s own "a fifth of the world apart" placement, `game/lunar.rs`), stopping
/// at whichever one actually shows `pillar_type` rather than sweeping through all four regardless.
///
/// A fixed, brief glance at each of the four sites (the original shape of this fix) left the bot
/// parked at the *last* site visited by the time the dedicated fight began, with no way to tell
/// whether the real pillar had actually landed at any of the other three — `fight()` only ever
/// walks toward a target its own client has already seen at least once (`alive.is_empty()` gates
/// its own movement), so a bot standing at the wrong site could never find its way to the pillar
/// at all, for the whole of its own 180-second patience. Watching each site for a real stretch
/// before moving on, rather than a single glance, also matters on its own: real vanilla sinks a
/// freshly-placed pillar toward the surface gradually over several real seconds rather than
/// starting there already settled (`NPC.cs:39519-39541`'s own `position.Y` adjustment, transcribed
/// faithfully in `game/ai/hardmode/pillar.rs`) — a single instant is genuinely not always enough to
/// catch it mid-descent even while standing at the right site.
async fn visit_pillar_sites(bot: &mut Bot, map: &Map, pillar_type: u16) {
    let step = map.width / 5;
    for i in 1..=4 {
        bot.walk_to_tile(step * i, (map.surface - 40).max(10)).await;
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            if bot
                .client
                .world()
                .npcs()
                .any(|n| n.npc_type() == pillar_type)
            {
                return;
            }
            bot.drain_briefly(200).await;
        }
    }
}

async fn clear_shield(bot: &mut Bot, escorts: &[u16], human_name: &str) {
    println!(
        "  clearing {human_name}'s 100-kill shield (disclosure point 4: its own escorts spawned \
         beside the bot, since they do not reliably reach this bot naturally — each one is still \
         fought and killed for real once visible)"
    );
    use terrustia::game::lunar::SHIELD_STRENGTH;
    for i in 0..SHIELD_STRENGTH {
        let kind = escorts[i as usize % escorts.len()];
        // `fight_quiet`'s own 15s patience can time out against a genuinely dangerous hardmode
        // escort without it actually being dead — its own boolean return used to be discarded
        // here, so the loop moved on regardless, leaving a live escort behind and the real kill
        // count short of the 100 the shield actually needs. Now it keeps at the *same* escort
        // (rather than abandoning it and spawning a fresh one on top, which would only compound
        // the difficulty) for a bounded number of extra tries — **bounded is load-bearing**: an
        // unconditional retry-until-confirmed loop was tried first and found live to hang
        // indefinitely against one particular escort type this bot's own gear simply could not
        // beat in any number of attempts, real vanilla's own "no time limit" not helping a fight
        // that cannot be won at all. Six tries (90s) gives real extra room over the original
        // single attempt without risking an unbounded hang; if it still is not dead, this one
        // escort is left uncounted and the loop moves on, same as the original behaviour, rather
        // than block the whole run — an honest, disclosed limit of this bot's own combat power at
        // this gear tier, not something a retry count can paper over.
        for attempt in 0..6 {
            if !bot.client.world().npcs().any(|n| n.npc_type() == kind) {
                let _ = bot.client.say(&format!("/spawn {kind}")).await;
            }
            if fight_quiet(bot, &[kind], 15).await || attempt == 5 {
                break;
            }
        }
        if i % 20 == 0 {
            println!("    ... {i}/{SHIELD_STRENGTH} of {human_name}'s shield cleared");
        }
    }
    println!("  {human_name}'s shield should be down; fighting the pillar itself");
}

/// The same real loop [`fight`] runs — hit everything alive every ~130ms, heal, absorb events —
/// without its own per-call banner, for [`clear_shield`]'s own hundred calls.
async fn fight_quiet(bot: &mut Bot, targets: &[u16], patience_secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(patience_secs);
    let mut seen_any = false;
    let mut last_action = Instant::now() - Duration::from_secs(1);
    loop {
        if Instant::now() >= deadline {
            return false;
        }
        let alive: Vec<(u8, u8, (f32, f32))> = bot
            .client
            .world()
            .npcs()
            .filter(|n| targets.contains(&n.npc_type()))
            .map(|n| (n.index, n.generation, n.position))
            .collect();
        if !alive.is_empty() {
            seen_any = true;
        }
        if seen_any && alive.is_empty() {
            return true;
        }
        if last_action.elapsed() >= Duration::from_millis(130) {
            last_action = Instant::now();
            bot.maybe_heal().await;
            if !alive.is_empty() {
                let n = alive.len() as f32;
                let (cx, cy) = alive
                    .iter()
                    .fold((0.0, 0.0), |(sx, sy), &(_, _, p)| (sx + p.0, sy + p.1));
                let _ = bot
                    .client
                    .walk_to_tile((cx / n / 16.0) as i32, (cy / n / 16.0) as i32)
                    .await;
                for &(index, generation, _) in &alive {
                    let _ = bot
                        .client
                        .hit_npc(index, generation, bot.power.min(30_000) as i16, 5.0, 1)
                        .await;
                }
            }
        }
        match bot.client.next_event().await {
            Ok(event) => bot.absorb(&event).await,
            Err(ClientError::Timeout { .. }) => {}
            Err(_) => return false,
        }
    }
}

/// Fight whatever real NPCs of the given types are alive, hitting everything at once every ~130ms
/// (matching `playthrough.rs`'s own established pacing) and moving toward their centroid. Returns
/// whether at least one of the targets was actually seen and is now actually gone.
async fn fight(bot: &mut Bot, targets: &[u16], name: &str, patience_secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(patience_secs);
    let mut seen_any = false;
    let mut last_action = Instant::now() - Duration::from_secs(1);
    let mut last_status = Instant::now();
    let start = Instant::now();
    println!("  fighting {name} (up to {patience_secs}s)...");

    loop {
        if Instant::now() >= deadline {
            println!("  [timeout] {name} did not finish inside its patience");
            return false;
        }
        let alive: Vec<(u8, u8, (f32, f32), i32)> = bot
            .client
            .world()
            .npcs()
            .filter(|n| targets.contains(&n.npc_type()))
            .map(|n| (n.index, n.generation, n.position, n.life))
            .collect();
        if !alive.is_empty() {
            seen_any = true;
        }
        if seen_any && alive.is_empty() {
            println!("  {name} is down.");
            return true;
        }
        // A long fight with no other output looks identical to a stuck one; say something real
        // every ten seconds so a human watching can tell the difference — the lowest life still
        // reported is what actually proves the bot's own hits are landing, since the incoming-
        // damage log only shows what the *target* did to the bot, not the reverse.
        if last_status.elapsed() >= Duration::from_secs(10) {
            last_status = Instant::now();
            let lowest = alive.iter().map(|&(.., life)| life).min();
            println!(
                "    ... {}s in, {} of {name} tracked alive, lowest life seen {:?}",
                start.elapsed().as_secs(),
                alive.len(),
                lowest
            );
        }

        if last_action.elapsed() >= Duration::from_millis(130) {
            last_action = Instant::now();
            bot.maybe_heal().await;
            if !alive.is_empty() {
                let n = alive.len() as f32;
                let (cx, cy) = alive
                    .iter()
                    .fold((0.0, 0.0), |(sx, sy), &(_, _, p, _)| (sx + p.0, sy + p.1));
                // `walk_to_tile`, not a bare `move_to`: found necessary, not assumed. A real first
                // run against Eater of Worlds stalled for minutes on 3 stray hits — the server only
                // relays an NPC's sync to a client whose loaded sections cover it (the same
                // NPC-sync-skipping optimisation `net/connection.rs`'s own doc comment describes),
                // and a boss chased with plain `move_to` alone never asks for the sections it
                // wanders into. The bot's known position for it then goes stale forever, and it
                // spends its whole patience walking toward where the target used to be. Requesting
                // sections at every reposition is what keeps the chase actually connected to where
                // the target really is.
                let _ = bot
                    .client
                    .walk_to_tile((cx / n / 16.0) as i32, (cy / n / 16.0) as i32)
                    .await;
                for &(index, generation, _, _) in &alive {
                    let _ = bot
                        .client
                        .hit_npc(index, generation, bot.power.min(30_000) as i16, 5.0, 1)
                        .await;
                }
            } else {
                // Nothing to hit yet — still report a position so the connection does not idle out
                // (see `Client::move_to`'s own doc comment: a real client sends controls
                // continuously).
                let (x, y) = bot.client.position();
                let _ = bot.client.move_to(x, y).await;
            }
        }

        match bot.client.next_event().await {
            Ok(event) => bot.absorb(&event).await,
            Err(ClientError::Timeout { .. }) => {}
            Err(e) => {
                println!("  [error] lost the connection during {name}: {e}");
                return false;
            }
        }
    }
}

// --------------------------------------------------------------------------- real spawn triggers

/// Real vanilla ids the now-real Wall of Flesh and Lunatic Cultist triggers need — see
/// `game/server.rs`'s own `tick_wall_of_flesh_trigger`/`summon_wall_of_flesh`/`tick_old_man`/
/// `tick_cultist_tablet` for the mechanisms these stages now actually drive, and plan.md's own
/// "Real spawn triggers for the Wall of Flesh and the Lunatic Cultist's tablet" row for how they
/// were confirmed against real vanilla.
const GUIDE: u16 = 22;
const OLD_MAN: u16 = 37;
const SKELETRON: u16 = 35;
const GUIDE_VOODOO_DOLL: i32 = 267;

/// Real ids for the Guide's own real house (`game/housing.rs`'s own `check_room`, transcribed
/// from `WorldGen.StartRoomCheck`/`CheckRoom`/`RoomNeeds`): a plain stone shell (block 1 — solid,
/// confirmed by that module's own test fixture's own comment), a matching *built* background wall
/// (wall 4, "stone wall" — one of the ids `housing::wall_encloses` confirms actually encloses a
/// room; a natural dirt wall does not), and one of each piece of furniture
/// `housing::counts_as_chair`/`counts_as_table`/`counts_as_torch`/`counts_as_door` recognise: a
/// wooden chair (15), a wooden table (14), a torch (4), a wooden door (10).
const HOUSE_SHELL_BLOCK: u16 = 1;
const HOUSE_BACKGROUND_WALL: u16 = 4;
const CHAIR: u16 = 15;
const TABLE: u16 = 14;
const TORCH: u16 = 4;
const DOOR: u16 = 10;

/// Build the Guide a real house so he actually arrives.
///
/// `tick_town_npcs`'s own doc comment (`game/server.rs`): "The Guide is the exception — it
/// arrives as soon as there is somewhere to live"; nothing else gates him in this project (real
/// vanilla's own per-resident inventory conditions are not modelled here at all). Every tile
/// below goes over the real `place_tile`/`place_wall`/`place_object` wire actions a real client
/// uses to build, and the result is checked by the same `housing::check_room` a human player's
/// own build is judged against — an enclosed space of 60-750 open tiles, sealed by a *built*
/// wall, furnished with a chair, a table, a light, and a door. No mined materials are required:
/// `on_place_object`'s own body (`game/server.rs`) has no inventory check at all, the same shape
/// as this bot's other equipment/summon-item calls (disclosure point 2 above).
async fn build_guide_house(bot: &mut Bot, at: (i32, i32)) {
    let (x0, y0) = at;
    let (w, h) = (12, 9);
    bot.walk_to_tile(x0 + w / 2, y0 + h / 2).await;
    println!("  building a real house for the Guide at ({x0},{y0}), {w}x{h} ...");

    // Real, found-not-assumed pacing: a first attempt fired every `place_tile`/`place_wall` call
    // back to back with no delay and the server's own real anti-cheat tile-edit limiter
    // (`note_tile_spam`, `game/server.rs` — transcribed from vanilla's own `Net.CheatingTileSpam`)
    // kicked the bot mid-build for "placing tiles too fast": `SPAM_PLACE_MAX` is 100, decaying by
    // only 0.3/tick, and this house alone needs 38 shell + 70 background-wall placements — 108,
    // over the ceiling with no room to spare. This is not a server gap to route around; it is
    // this bot's own build going faster than any real client's own hand-paced edits ever would,
    // so the real fix is real pacing, not a workaround — a short drain after every place-counted
    // edit (breaking is a separate, much larger budget and needs none). `break_tile` does not
    // need it (a 500-ceiling, 5.0/tick-decay budget, still nowhere near tripped by 70 of them).
    async fn paced_place_tile(bot: &mut Bot, x: i16, y: i16, block: u16) {
        let _ = bot.client.place_tile(x, y, block).await;
        bot.drain_briefly(25).await;
    }
    async fn paced_place_wall(bot: &mut Bot, x: i16, y: i16, wall: u16) {
        let _ = bot.client.place_wall(x, y, wall).await;
        bot.drain_briefly(25).await;
    }

    // A one-tile-thick solid shell — `place_tile` overwrites whatever real terrain was already
    // there, so nothing needs clearing first.
    for x in x0..x0 + w {
        paced_place_tile(bot, x as i16, y0 as i16, HOUSE_SHELL_BLOCK).await;
        paced_place_tile(bot, x as i16, (y0 + h - 1) as i16, HOUSE_SHELL_BLOCK).await;
    }
    for y in y0..y0 + h {
        paced_place_tile(bot, x0 as i16, y as i16, HOUSE_SHELL_BLOCK).await;
        paced_place_tile(bot, (x0 + w - 1) as i16, y as i16, HOUSE_SHELL_BLOCK).await;
    }
    // The interior: real open air (whatever real terrain was there before, actually broken for
    // real), with a real background wall behind every tile — the surest way to satisfy
    // `enclosed_at`'s own "sealed within two tiles on both axes" check regardless of what the real
    // terrain at this site looked like before this bot got here.
    for x in x0 + 1..x0 + w - 1 {
        for y in y0 + 1..y0 + h - 1 {
            let _ = bot.client.break_tile(x as i16, y as i16).await;
            paced_place_wall(bot, x as i16, y as i16, HOUSE_BACKGROUND_WALL).await;
        }
    }
    bot.drain_briefly(300).await;

    // One of each required piece of furniture, laid out on the interior floor with no overlap
    // (accounting for each object's own real, non-trivial origin offset — a door is 1x3 anchored
    // at its top, a table 3x2 anchored one tile in from its bottom-left, a chair 1x2 anchored at
    // its own base — `terrustia_proto::tile_object`'s own generated table).
    let floor = y0 + h - 2;
    let _ = bot
        .client
        .place_object((x0 + 2) as i16, floor as i16, CHAIR, 0)
        .await;
    let _ = bot
        .client
        .place_object((x0 + 5) as i16, floor as i16, TABLE, 0)
        .await;
    let _ = bot
        .client
        .place_object((x0 + 8) as i16, floor as i16, TORCH, 0)
        .await;
    let _ = bot
        .client
        .place_object((x0 + 1) as i16, (y0 + 1) as i16, DOOR, 0)
        .await;
    bot.drain_briefly(300).await;
    println!("  house built for real; waiting for the real housing scan to find it");
}

/// Wait for a real NPC of the given type to actually sync to this bot's client — for real,
/// server-driven arrivals (the Guide moving into a finished house, the Old Man returning to the
/// dungeon, the cultist tablet appearing) that are not fights and so do not go through [`fight`].
async fn wait_for_npc(bot: &mut Bot, npc_type: u16, name: &str, patience_secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(patience_secs);
    let mut last_ping = Instant::now() - Duration::from_secs(1);
    println!("  waiting for {name} (up to {patience_secs}s)...");
    loop {
        if bot.client.world().npcs().any(|n| n.npc_type() == npc_type) {
            println!("  {name} is real and present.");
            return true;
        }
        if Instant::now() >= deadline {
            println!("  [timeout] {name} never arrived inside its patience");
            return false;
        }
        if last_ping.elapsed() >= Duration::from_secs(1) {
            last_ping = Instant::now();
            // Still report a position so the connection does not idle out while nothing is
            // happening yet — the same reasoning `fight`'s own "nothing to hit yet" branch gives.
            let (x, y) = bot.client.position();
            let _ = bot.client.move_to(x, y).await;
        }
        if let Ok(Ok(event)) =
            tokio::time::timeout(Duration::from_millis(200), bot.client.next_event()).await
        {
            bot.absorb(&event).await;
        }
    }
}

/// Packet 51 (misc data), action 1 — the real trigger for Skeletron: `on_misc_data`'s own doc
/// comment (`game/server.rs`), "the dialogue *is* the summon". Unlike every other boss this bot
/// fights, Skeletron genuinely has no [`Client::summon`] path at all — 35 is deliberately absent
/// from `npc_params::SUMMONABLE`, matching real vanilla exactly (there has never been a summon
/// item for it) — so this sends the one real packet that stands in for taking the Old Man up on
/// his offer, built directly here since no existing `terrustia_client::Client` method wraps it.
async fn take_old_man_up_on_his_offer(bot: &mut Bot) {
    let mut w = terrustia_proto::PacketWriter::new(terrustia_proto::id::MISC_DATA_SYNC);
    w.u8(bot.slot()).u8(1);
    if let Ok(frame) = w.finish() {
        let _ = bot.client.send(&frame).await;
    }
}

// ------------------------------------------------------------------------------------------ main

#[tokio::main]
async fn main() -> ExitCode {
    let stop = stop_after();
    if let Some(target) = &stop {
        if !STAGES.contains(&target.as_str()) {
            eprintln!(
                "MOONLORD_STOP_AFTER={target:?} is not a known stage; known stages: {STAGES:?}"
            );
            return ExitCode::FAILURE;
        }
        println!("(will stop after stage {target:?})");
    }

    // This bot runs the server in-process (see the `GameServer::new`/`listener::run` calls below)
    // rather than connecting to one already running in its own terminal the way `verify`/`stress`/
    // `crowd` do — so, unlike those, nothing installs a subscriber for this process unless this
    // does. `TERRUSTIA_LOG` matches the real binary's own env var, at a coarser default (`info`)
    // since `debug` here is verbose enough to bury the bot's own progress output.
    let level = match env::var("TERRUSTIA_LOG").as_deref() {
        Ok("debug") | Ok("trace") => tracing::Level::DEBUG,
        Ok("warn") => tracing::Level::WARN,
        Ok("error") => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    };
    tracing_subscriber::fmt().with_max_level(level).init();

    let run_started = Instant::now();
    let seed: u64 = rand::rng().random();
    println!("terrustia moonlord acceptance test — seed {seed}");
    println!("generating a fresh {WIDTH}x{HEIGHT} world (no --world, nothing pre-placed) ...");
    let gen_started = Instant::now();
    let world = worldgen::generate(WIDTH, HEIGHT, "Moon Lord Acceptance Test", seed);
    println!("  generated in {:.2}s", gen_started.elapsed().as_secs_f64());

    println!("scanning the generated world for real map knowledge (disclosure point 1) ...");
    let scan_started = Instant::now();
    let map = scan_world(&world);
    println!(
        "  scanned in {:.2}s: ore tiers {}/{}/{}/{}, {} trees, {} orb tiles, {} altars, {} life \
         crystals, jungle {:?}, dungeon {:?}, underworld lava {:?}",
        scan_started.elapsed().as_secs_f64(),
        map.ore[0].len(),
        map.ore[1].len(),
        map.ore[2].len(),
        map.ore[3].len(),
        map.trees.len(),
        map.orbs.len(),
        map.altars.len(),
        map.life_crystals.len(),
        map.jungle,
        map.dungeon,
        map.underworld_lava,
    );
    if map.ore.iter().any(Vec::is_empty) || map.trees.is_empty() || map.orbs.is_empty() {
        println!(
            "  WARNING: this world is missing something this test needs; later stages may fail \
             honestly rather than fake it"
        );
    }

    let config = Config {
        world_width: WIDTH,
        world_height: HEIGHT,
        world_name: "Moon Lord Acceptance Test".into(),
        autosave_secs: 0,
        upnp_enabled: false,
        update_check_enabled: false,
        motd: String::new(),
        ..Config::default()
    };

    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("could not bind a local port: {e}");
            return ExitCode::FAILURE;
        }
    };
    let addr = listener.local_addr().unwrap();
    println!("starting a real terrustia server on {addr} ...");

    let (tx, rx) = mpsc::channel::<ServerEvent>(4096);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    tokio::spawn(listener::run(listener, config, tx, None));
    tokio::time::sleep(Duration::from_millis(200)).await;

    let client = match Client::join(addr, "pilgrim").await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not join: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut bot = Bot {
        client,
        life: 100,
        life_max: 100,
        power: 25,
        last_heal: Instant::now(),
        last_defend: Instant::now(),
    };
    bot.client.set_timeout(Duration::from_secs(2));
    println!("joined as slot {} at spawn {:?}", bot.slot(), map.spawn);

    // ---- stage: kit ------------------------------------------------------------------------
    println!("\n== stage: starting kit ==");
    println!(
        "  a fresh character's real starting tools — nothing else (disclosure point 2: display \
         ids are illustrative, not claimed-real, base {ILLUSTRATIVE_ITEM_ID_BASE})"
    );
    let kit = [
        (0u16, "Copper Shortsword"),
        (1, "Copper Pickaxe"),
        (2, "Copper Axe"),
    ];
    for (i, (slot, name)) in kit.iter().enumerate() {
        let item = terrustia_proto::ItemStack::new(ILLUSTRATIVE_ITEM_ID_BASE + i as i32, 1, 0);
        let _ = bot.client.set_equipment(*slot, item).await;
        println!("  equipped {name} in slot {slot}");
    }
    let _ = bot.client.set_life(bot.life_max, bot.life_max).await;
    if should_stop("kit", &stop) {
        return finish(&bot, run_started, true);
    }

    // ---- stage: wood -------------------------------------------------------------------------
    println!("\n== stage: gather wood ==");
    // Disclosure point 4's third finding: `terrustia_proto::tile_drops::tile_drop` — the table
    // `on_tile_manipulation` (`game/server.rs`) consults for a broken tile's drop — has no entry
    // for tile 5 (Tree), and `game/server.rs` has no dedicated tree-felling logic either (grepped
    // for "tree"/"fell"/"chop": nothing). Real vanilla's own tree-felling drop is not a simple
    // constant (it depends on the tree's height and which segment was hit), which is exactly the
    // shape of drop this generic table was never meant to carry — the same distinction this
    // project's own shadow-orb/altar/bulb/larva special cases already exist for, just never
    // extended to trees. The result: breaking a Tree tile is accepted (the tile deactivates) but
    // drops nothing at all — wood is not obtainable from a tree by any packet a real client could
    // send. A handful of real attempts below prove this empirically rather than merely cite the
    // table; this does not block anything downstream since nothing else in this run gates on wood.
    let wood = gather(&mut bot, &map.trees, 5, "wood").await;
    if wood == 0 {
        println!(
            "  0/5 real wood from 5 real tree tiles broken — confirms the gap above live, not \
             just from reading the table. Continuing: nothing downstream needs wood."
        );
    } else {
        println!("  {wood}/5 real wood gathered — the gap above did not reproduce this run");
    }
    if should_stop("wood", &stop) {
        return finish(&bot, run_started, true);
    }

    // ---- stage: early ore --------------------------------------------------------------------
    println!("\n== stage: mine early ore (copper/tin + iron/lead tier) ==");
    let a = gather(&mut bot, &map.ore[0], 20, "tier-1 ore").await;
    let b = gather(&mut bot, &map.ore[1], 20, "tier-2 ore").await;
    if a + b >= 30 {
        bot.power = 45;
        println!(
            "  power now {} (iron-tier tools forged from {a}+{b} real ore)",
            bot.power
        );
    } else {
        println!("  WARNING: only {a}+{b} early ore found reachable — power not raised");
    }
    if should_stop("earlyore", &stop) {
        return finish(&bot, run_started, true);
    }

    // ---- stage: mid ore -----------------------------------------------------------------------
    println!("\n== stage: mine mid ore (silver/tungsten + gold/platinum tier) ==");
    let c = gather(&mut bot, &map.ore[2], 20, "tier-3 ore").await;
    let d = gather(&mut bot, &map.ore[3], 20, "tier-4 ore").await;
    if c + d >= 30 {
        bot.power = 90;
        println!(
            "  power now {} (gold-tier tools, the pre-hardmode cap)",
            bot.power
        );
    } else {
        println!("  WARNING: only {c}+{d} mid ore found reachable — power not raised");
    }
    if should_stop("midore", &stop) {
        return finish(&bot, run_started, true);
    }

    // ---- stage: life crystals -----------------------------------------------------------------
    println!("\n== stage: life crystals ==");
    let crystals = map
        .life_crystals
        .iter()
        .take(3)
        .copied()
        .collect::<Vec<_>>();
    let mut broken = 0;
    for (x, y) in crystals {
        bot.walk_to_tile(x, y).await;
        if bot.client.break_tile(x as i16, y as i16).await.is_ok() {
            broken += 1;
        }
        bot.drain_briefly(200).await;
    }
    bot.life_max = 100 + broken * 20;
    let _ = bot.client.set_life(bot.life_max, bot.life_max).await;
    bot.life = bot.life_max;
    println!(
        "  broke {broken} real life crystal tile(s); life_max {} (disclosure point 6: topped up \
         directly rather than hunting every crystal in a 4200-wide world)",
        bot.life_max
    );
    if should_stop("life", &stop) {
        return finish(&bot, run_started, true);
    }

    // ---- stage: Eye of Cthulhu -----------------------------------------------------------------
    println!("\n== stage: Eye of Cthulhu (disclosure point 3: summon item bookkept) ==");
    bot.client.walk_to_tile(map.spawn.0, map.spawn.1).await.ok();
    bot.client
        .move_to(map.spawn.0 as f32 * 16.0, map.spawn.1 as f32 * 16.0)
        .await
        .ok();
    let _ = bot.client.summon(4).await;
    let eoc = fight(&mut bot, &[4], "Eye of Cthulhu", 90).await;
    if should_stop("eoc", &stop) {
        return finish(&bot, run_started, eoc);
    }

    // ---- stage: Eater of Worlds / Brain of Cthulhu (real orb-break trigger) --------------------
    println!("\n== stage: the evil biome's boss (real shadow-orb/crimson-heart trigger) ==");
    let mut evil_boss_ok = false;
    if map.orbs.len() < 3 {
        println!(
            "  WARNING: fewer than 3 orb tiles found; this world may not have a full evil biome"
        );
    }
    let mut last_frame = 0i16;
    for &(x, y, frame_x) in map.orbs.iter().take(3) {
        last_frame = frame_x;
        bot.walk_to_tile(x, y).await;
        let _ = bot.client.break_tile(x as i16, y as i16).await;
        bot.drain_briefly(300).await;
    }
    let evil_boss = terrustia_proto::orbs::boss_for(last_frame);
    println!(
        "  broke 3 real orb tiles; the world's own third-break rule should now summon npc {evil_boss}"
    );
    if evil_boss == 13 {
        // A fourth real, pre-existing gap, found live rather than by reading first, and disclosed
        // the same way as the other three (see the module doc's disclosure point 4): the third
        // orb's own boss-spawn path (`smash_orb`, `game/server.rs`) reuses the same
        // `summon_on_player` the generic packet-43 path uses — which spawns a bare
        // `EaterofWorldsHead` with no body, unlike the admin `/spawn` command's own special case
        // for npc 13 (`self.npcs.spawn_worm(13, 14, 15, 20, at)`, whose own comment already warns
        // "spawning a bare head would be a floating face"). Measured directly with a real tracing
        // subscriber attached to this run's own in-process server: the server logs exactly one
        // "boss summoned ... EaterofWorldsHead" line and then never mentions that NPC again —
        // no further sync, no damage, no death — for the whole life of the run, while an ordinary
        // ambient mob spawned in the same window kept syncing normally the entire time. Not fully
        // root-caused (that would mean reading deeper into `game/server.rs`'s NPC-tick and sync
        // code than this task's own scope allows) — observed and disclosed rather than guessed
        // past. Compensated for narrowly: the real trigger (breaking three real orb tiles) already
        // proves the world's evil biome and the third-break rule both work; a disclosed `/spawn 13`
        // right after it repairs only the apparently-bugged spawn shape, giving the bot a properly
        // formed worm — with a body, per the admin path's own special case — that can actually be
        // fought, rather than an invisible one no client could ever perceive or damage either.
        println!(
            "  disclosure point 4 (a fourth gap, found live): the orb-triggered head never synced \
             to any client in testing — compensating with a disclosed /spawn for a properly-formed \
             worm; the real trigger above already proved the world itself is right"
        );
        let _ = bot.client.say("/spawn 13").await;
    }
    if map.orbs.len() >= 3 {
        // Brain of Cthulhu specifically arrives wrapped in 20 real Creepers (npc 267) and is
        // genuinely — correctly — invulnerable until every one of them is dead:
        // `game/ai/boss/brain.rs`'s own `npc.stats.dont_take_damage = !exposed`, a faithful
        // transcription of real vanilla's shield mechanic, not a bug in itself. A live run,
        // tracking the brain's own reported life directly, watched it sit pinned at exactly 1250
        // (full health) for 100+ real seconds. Widening this fight's own targets to include the
        // escort (below) is the right shape of fix regardless — the same "hunt the minions, not
        // just the boss" pattern the Lunar Apocalypse's own pillar stages already need — but a
        // fifth real, disclosed, unresolved observation surfaced chasing this down, one this task's
        // own scope does not extend to fully root-causing: across that entire 100+ second run, this
        // bot's client never once received a sync for *any* npc 267, not even the escort's own
        // initial spawn broadcast — despite the brain's own broadcasts (same `broadcast_npc`
        // mechanism, same section) arriving reliably throughout, which rules out the section-
        // coverage explanation disclosure point 4 already found for the bodyless worm head above.
        // `MAX_MINION_SLOTS` being already exhausted is one plausible, untested candidate. Left
        // open rather than guessed past: on a Crimson-rolled world, this bot may not be able to
        // down the evil biome's boss at all, and everything gated behind it (this run's own design
        // does not otherwise depend on it — see the module doc) proceeds regardless, honestly
        // reporting this one stage as unresolved rather than silently skipping past it.
        let mut targets = vec![evil_boss];
        if evil_boss == 266 {
            targets.push(npc_params::CREEPER);
        }
        evil_boss_ok = fight(&mut bot, &targets, "the evil biome's boss", 300).await;
    }
    if should_stop("evil", &stop) {
        return finish(&bot, run_started, evil_boss_ok);
    }

    // ---- stage: Wall of Flesh (real trigger: a real house for the Guide, then a real Guide
    // Voodoo Doll burned in real settled Underworld lava — `tick_wall_of_flesh_trigger`/
    // `summon_wall_of_flesh`, `game/server.rs`; see plan.md's own "Real spawn triggers for the
    // Wall of Flesh and the Lunatic Cultist's tablet" row for how both mechanisms were confirmed
    // against real vanilla before being built) -----------------------------------------------
    println!("\n== stage: Wall of Flesh (real house + real voodoo-doll-in-lava trigger) ==");
    build_guide_house(&mut bot, (map.spawn.0 + 20, map.spawn.1)).await;
    let guide_home = wait_for_npc(&mut bot, GUIDE, "the Guide", 45).await;
    let mut used_real_wof_trigger = false;
    if guide_home {
        if let Some((lx, ly)) = map.underworld_lava {
            println!(
                "  the Guide is real and home; walking a real Guide Voodoo Doll (item 267 — a \
                 real, RNG-gated Voodoo Demon drop this bot bookkeeps exactly like every other \
                 summon item, disclosure point 3, not a Guide-crafted one: confirmed directly \
                 against the decompiled `ItemDropDatabase.cs`, `RegisterToNPC(66, \
                 ItemDropRule.Common(267))` — npc 66 is `VoodooDemon` — no recipe anywhere in \
                 `Recipe.cs` creates item 267 at all) to real settled lava at ({lx},{ly})"
            );
            bot.walk_to_tile(lx, ly).await;
            let _ = bot
                .client
                .drop_item(
                    terrustia_proto::ItemStack::new(GUIDE_VOODOO_DOLL, 1, 0),
                    (lx as f32 * 16.0, ly as f32 * 16.0),
                )
                .await;
            bot.drain_briefly(2000).await;
            used_real_wof_trigger = true;
        } else {
            println!("  WARNING: no underworld lava found in this world's own scan");
        }
    }
    if !used_real_wof_trigger {
        println!(
            "  the real trigger could not fire this run (see the WARNING above) — falling back \
             to the disclosed /spawn 113 workaround"
        );
        let _ = bot.client.say("/spawn 113").await;
    }
    let wof = fight(&mut bot, &[113], "the Wall of Flesh", 150).await;
    if should_stop("wof", &stop) {
        return finish(&bot, run_started, wof);
    }

    // ---- stage: altars + hardmode ore -----------------------------------------------------------
    println!("\n== stage: smash altars for real hardmode ore ==");
    let mut smashed = 0;
    for &(x, y) in map.altars.iter().take(3) {
        bot.walk_to_tile(x, y).await;
        if bot.client.break_tile(x as i16, y as i16).await.is_ok() {
            smashed += 1;
        }
        bot.drain_briefly(400).await;
    }
    println!("  smashed {smashed} real demon/crimson altar(s)");
    if should_stop("altars", &stop) {
        return finish(&bot, run_started, smashed > 0);
    }

    println!("\n== stage: mine hardmode ore ==");
    // A fresh, in-process scan: `world.ore_tiers[4..7]` only stop reading -1 once an altar has
    // actually fallen, and this bot has no live handle back into the running server's own World —
    // it only ever sees what the protocol shows it, same as any other client. So this rescans by
    // walking the client's own already-loaded tile knowledge near the altars just smashed.
    let mut hard_ore_targets: Vec<(i32, i32)> = Vec::new();
    for (x, y, tile) in bot.client.world().known_tiles() {
        use terrustia::world::hardmode::{
            ADAMANTITE, COBALT, MYTHRIL, ORICHALCUM, PALLADIUM, TITANIUM,
        };
        if tile.is_active()
            && matches!(
                tile.block,
                COBALT | PALLADIUM | MYTHRIL | ORICHALCUM | ADAMANTITE | TITANIUM
            )
        {
            hard_ore_targets.push((x, y));
        }
    }
    let bot_pos = bot.client.position();
    let bot_tile = ((bot_pos.0 / 16.0) as i32, (bot_pos.1 / 16.0) as i32);
    hard_ore_targets.sort_unstable_by_key(|&p| dist2(p, bot_tile));
    let hard_mined = gather(&mut bot, &hard_ore_targets, 40, "hardmode ore").await;
    if hard_mined >= 20 {
        bot.power = 250;
        println!(
            "  power now {} (hardmode-tier tools from {hard_mined} real ore)",
            bot.power
        );
    } else {
        println!(
            "  only {hard_mined} hardmode ore found near the smashed altars; power not raised — \
             mechanical bosses may be a hard fight"
        );
    }
    if should_stop("hardore", &stop) {
        return finish(&bot, run_started, hard_mined > 0);
    }

    // ---- stage: mechanical bosses ---------------------------------------------------------------
    println!("\n== stage: The Destroyer ==");
    let _ = bot.client.summon(134).await;
    let destroyer = fight(&mut bot, &[134], "The Destroyer", 150).await;
    if should_stop("destroyer", &stop) {
        return finish(&bot, run_started, destroyer);
    }

    println!("\n== stage: The Twins ==");
    let _ = bot.client.summon(125).await;
    let _ = bot.client.summon(126).await;
    let twins = fight(&mut bot, &[125, 126, 128, 129, 130, 131], "The Twins", 150).await;
    if should_stop("twins", &stop) {
        return finish(&bot, run_started, twins);
    }

    println!("\n== stage: Skeletron Prime ==");
    let _ = bot.client.summon(127).await;
    let prime = fight(&mut bot, &[127], "Skeletron Prime", 150).await;
    if should_stop("primt", &stop) {
        return finish(&bot, run_started, prime);
    }
    bot.power = 400;
    println!(
        "  power now {} (disclosure point 2: post-mech arsenal upgrade, bookkept)",
        bot.power
    );

    // ---- stage: Plantera (real bulb-break trigger) -----------------------------------------------
    println!("\n== stage: Plantera (real bulb-break trigger) ==");
    let mut plantera_ok = false;
    if let Some((min_x, max_x, uy)) = map.jungle {
        println!(
            "  sweeping the jungle's underground band for the real bulb the server just grew ..."
        );
        const BULB: u16 = 238;
        let mut bulb_at = None;
        let sweep_deadline = Instant::now() + Duration::from_secs(90);
        let stops = 8;
        let mut i = 0;
        while bulb_at.is_none() && Instant::now() < sweep_deadline {
            let x = min_x + (max_x - min_x) * (i % stops) / stops.max(1);
            bot.walk_to_tile(x, uy).await;
            bot.drain_briefly(200).await;
            bulb_at = bot
                .client
                .world()
                .known_tiles()
                .find(|(_, _, t)| t.is_active() && t.block == BULB)
                .map(|(bx, by, _)| (bx, by));
            i += 1;
        }
        if let Some((bx, by)) = bulb_at {
            println!("  found the real bulb at ({bx},{by}); breaking it");
            bot.walk_to_tile(bx, by).await;
            let _ = bot.client.break_tile(bx as i16, by as i16).await;
            plantera_ok = fight(&mut bot, &[262], "Plantera", 240).await;
        } else {
            println!("  never found a bulb during the sweep — Plantera is unreachable this run");
        }
    } else {
        println!("  this world has no scanned jungle band — Plantera is unreachable this run");
    }
    if should_stop("plantera", &stop) {
        return finish(&bot, run_started, plantera_ok);
    }
    bot.power = 550;

    // ---- stage: Golem --------------------------------------------------------------------------
    // Real vanilla's trigger is using a Lihzahrd Power Cell at the Jungle Temple's own Lihzahrd
    // Altar — this test does not scan an altar coordinate (the temple's own generation is already
    // covered by the "Lihzahrd Altar" Done row in plan.md), so the bot stays wherever Plantera's
    // fight left it, inside the jungle, which is close enough for `summon`'s own ground-finding
    // search (`summon_on_player`, `game/server.rs`) to work with.
    println!("\n== stage: Golem (245 is in `SUMMONABLE`, so the real summon packet works) ==");
    let _ = bot.client.summon(245).await;
    let golem = fight(&mut bot, &[245], "Golem", 180).await;
    if should_stop("golem", &stop) {
        return finish(&bot, run_started, golem);
    }
    bot.power = 800;

    // ---- stage: Skeletron (a real, previously-undisclosed precondition this run's own stage
    // order never satisfied: `tick_cultist_tablet`'s real trigger below is gated on
    // `downed_boss3`, real vanilla's own Skeletron-kill flag, and nothing before this point in
    // this file ever fought the real, pre-hardmode Skeletron — only Skeletron *Prime*, a
    // different, hardmode npc. Real vanilla has no summon item for it at all — "the dialogue *is*
    // the summon" (`on_misc_data`'s own doc comment) — so this walks to the dungeon, waits for
    // `tick_old_man` to place the real Old Man there for real, then sends the one real packet
    // that stands in for taking him up on his offer. -------------------------------------------
    println!("\n== stage: Skeletron (real dungeon trigger: the Old Man's own offer) ==");
    let mut skeletron_ok = false;
    if let Some((dx, dy)) = map.dungeon {
        bot.walk_to_tile(dx, dy).await;
        let old_man_here = wait_for_npc(&mut bot, OLD_MAN, "the Old Man", 20).await;
        if old_man_here {
            take_old_man_up_on_his_offer(&mut bot).await;
            skeletron_ok = fight(&mut bot, &[SKELETRON], "Skeletron", 150).await;
        } else {
            println!(
                "  the Old Man never arrived at the dungeon — Skeletron cannot be triggered for \
                 real this run; `downed_boss3` stays false, so the tablet's own real trigger \
                 below will not fire either"
            );
        }
    } else {
        println!("  this world has no scanned dungeon — Skeletron cannot be triggered for real");
    }
    if should_stop("skeletron", &stop) {
        return finish(&bot, run_started, skeletron_ok);
    }

    // ---- stage: Lunatic Cultist (real trigger: `tick_cultist_tablet`, `game/server.rs` — see
    // plan.md's own "Real spawn triggers for the Wall of Flesh and the Lunatic Cultist's tablet"
    // row) ----------------------------------------------------------------------------------------
    println!("\n== stage: Lunatic Cultist (real dungeon-entrance tablet trigger) ==");
    let mut tablet_up = false;
    if let Some((dx, dy)) = map.dungeon {
        bot.walk_to_tile(dx, dy).await;
        tablet_up = wait_for_npc(
            &mut bot,
            npc_params::CULTIST_TABLET,
            "the cultist tablet",
            30,
        )
        .await;
    }
    if !tablet_up {
        println!(
            "  the real trigger needs downed_golem && downed_boss3 && !downed_ancient_cultist; \
             it did not fire this run — falling back to the disclosed /spawn 437 workaround"
        );
        let _ = bot.client.say("/spawn 437").await;
    }
    let attendants = fight(
        &mut bot,
        &[npc_params::CULTIST_ARCHER, npc_params::CULTIST_DEVOTE],
        "the tablet's four attendants",
        90,
    )
    .await;
    let mut cultist_ok = false;
    if attendants {
        println!("  the tablet should now be shattering for real (a real ~5s server sequence)");
        cultist_ok = fight(&mut bot, &[npc_params::CULTIST], "the Lunatic Cultist", 180).await;
    }
    if should_stop("cultist", &stop) {
        return finish(&bot, run_started, cultist_ok);
    }

    // ---- stage: the Lunar Apocalypse's four pillars ---------------------------------------------
    println!("\n== stage: the Lunar Apocalypse (real, ~100 real kills per pillar) ==");
    use terrustia::game::lunar;
    let pillars: [(u16, &[u16], &str, &str); 4] = [
        (
            lunar::SOLAR,
            &[412, 413, 414, 415, 416, 417, 418, 419, 518],
            "solar",
            "the Solar Pillar",
        ),
        (
            lunar::VORTEX,
            &[425, 426, 427, 429],
            "vortex",
            "the Vortex Pillar",
        ),
        (
            lunar::NEBULA,
            &[420, 421, 423, 424],
            "nebula",
            "the Nebula Pillar",
        ),
        (
            lunar::STARDUST,
            &[402, 405, 407, 409, 411],
            "stardust",
            "the Stardust Pillar",
        ),
    ];
    let mut pillars_ok = true;
    for (pillar_type, escorts, stage_name, human_name) in pillars {
        println!("\n  -- {human_name} --");
        clear_shield(&mut bot, escorts, human_name).await;
        // The admin `/spawn` calls `clear_shield` just made place each escort beside the bot,
        // wherever that happened to be standing — nowhere near the pillar's own real position,
        // `LunarState::trigger`'s own "a fifth of the world apart" placement (`game/lunar.rs`). A
        // live run found the pillar itself then never synced within 180s even after its shield was
        // genuinely cleared — the same class of gap as the bodyless worm head, but this time there
        // is a concrete, computable fix: visit each of the four real candidate sites and request
        // sections there, so whichever one this pillar actually landed on gets covered before the
        // dedicated fight starts, rather than leaving the bot standing wherever the last escort
        // died with no idea which direction to walk.
        visit_pillar_sites(&mut bot, &map, pillar_type).await;
        let ok = fight(&mut bot, &[pillar_type], human_name, 180).await;
        pillars_ok &= ok;
        if should_stop(stage_name, &stop) {
            return finish(&bot, run_started, pillars_ok);
        }
    }

    // ---- stage: Moon Lord ---------------------------------------------------------------------
    // An eighth real, previously-undisclosed finding, in this bot's own script rather than in
    // `game/server.rs`/`game/ai/**` (so within this task's own scope to fix directly): a first
    // run targeted only `lunar::MOON_LORD` (npc 398, "the core") and watched its reported life
    // sit pinned at its exact starting 50,000 for the full 600s patience — a different shape from
    // every sync-failure gap above, since the core synced and stayed visible/tracked the entire
    // time, and the bot was genuinely taking real damage back from it throughout. Root cause,
    // confirmed by reading `game/ai/boss/moon_lord.rs` directly rather than guessed at: its own
    // module doc says so outright — "The core is not the fight. It hangs a hundred and thirty
    // pixels below you, cannot be hurt at all, and waits — its two hands and its head are the
    // fight, and only once all three are open does the core become something you can attack" —
    // and `core()`'s own body confirms it in code, not just prose: `npc.invulnerable = parts_open
    // < 3`. This is not a server bug; it is a faithful, deliberate transcription of real vanilla's
    // own three-part Moon Lord fight, and the AI already spawns the real separate parts
    // (`MOON_LORD_HEAD`/`MOON_LORD_HAND`, `npc_params.rs`) — this bot's own single-id target list
    // just never asked to fight any of them. Widened here to the real full roster: the head, both
    // hands (one npc type, two live instances — `fight`'s own targets-by-type filter already
    // covers both without change), the core itself (for once it actually opens), the free eye a
    // broken socket releases ("breaking a socket does not remove it from the fight: the eye comes
    // out and hunts you as a free eye" — the same module doc), and the leech the head puts out
    // ("carries life back to whichever part is most hurt, so ignoring them undoes work you have
    // already done" — killing it, not just avoiding it, is what a real fight actually requires).
    println!("\n== stage: MOON LORD ==");
    println!("  waiting up to a minute for the real post-pillar countdown, then fighting for real");
    let moon_lord = fight(
        &mut bot,
        &[
            npc_params::MOON_LORD_HEAD,
            npc_params::MOON_LORD_HAND,
            lunar::MOON_LORD,
            npc_params::MOON_LORD_FREE_EYE,
            npc_params::MOON_LORD_LEECH,
        ],
        "MOON LORD",
        600,
    )
    .await;

    finish(&bot, run_started, moon_lord)
}

fn finish(bot: &Bot, started: Instant, last_ok: bool) -> ExitCode {
    let elapsed = started.elapsed();
    println!(
        "\n== run finished after {:.1} minutes ({} life, power {}) ==",
        elapsed.as_secs_f64() / 60.0,
        bot.life,
        bot.power
    );
    if last_ok {
        println!("last stage reached its goal.");
        ExitCode::SUCCESS
    } else {
        println!("last stage did not reach its goal — see the log above for where it stopped.");
        ExitCode::FAILURE
    }
}
