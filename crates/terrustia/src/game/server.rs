//! The single-writer game task.
//!
//! One task owns the world and the player table, so there are no locks on the hot path and packet
//! ordering is deterministic. Connections talk to it over an `mpsc` of [`ServerEvent`]; it talks
//! back through each player's outbound queue.

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    time::{Duration, Instant},
};

use bytes::Bytes;
use rand::{Rng, SeedableRng, rngs::SmallRng};
use std::path::Path;
use terrustia_proto::{
    ItemStack, NetworkText, Tile, TileFlags, id,
    items::{ItemOwner, SyncItem, decode_item_despawn},
    net_module::{self, IncomingChat},
    npc::{DamageNpc, SyncNpc, damage_ack, damage_taken},
    npc_data::npc_stats,
    objects::{
        self, DoorToggle, RequestChestOpen, RequestSign, SignText, SyncChestItem, SyncPlayerChest,
    },
    packets::{
        self, Hello, PlayerControls, PlayerHealth, PlayerMana, PlayerSpawn, SpawnTileData,
        TileAction, TileManipulation,
    },
    reader::PacketReader,
    section::encode_section_packet,
    square::TileSquare,
    tile_drops::tile_drop,
    tile_sets::frame_important,
};
use tokio::{
    sync::{mpsc, oneshot},
    time::{MissedTickBehavior, interval},
};

use tracing::{debug, error, info, warn};

use crate::{
    config::Config,
    game::player::{ConnState, Player},
    game::{
        clock,
        event::{Invasion, InvasionState},
        housing,
        npc::{NpcStore, TileView},
        npc_ai::{self, Target},
        spawn,
    },
    net::Frame,
    world::{
        Sign, World,
        items::{self, ItemStore},
        wld_save,
        world::{DAY_LENGTH, NIGHT_LENGTH},
    },
};

/// Signs hold a page of text at most; anything longer is a client that is not playing fair.
const MAX_SIGN_TEXT: usize = 1000;

/// How far a player can be from an item and still have it reserved for them, in pixels.
///
/// Generous on purpose: the reservation only grants the right to pick the item up, and a client
/// needs to hold it before its own grab animation begins.
const ITEM_GRAB_RANGE: f32 = 400.0;

/// Vanilla runs at 60 ticks per second and the clock packets assume it.
const TICK: Duration = Duration::from_nanos(16_666_667);

/// How often the world clock is pushed to clients.
const TIME_SYNC_TICKS: u64 = 60;

/// How often the worst tick in the window is reported, when it is worth reporting.
const TICK_REPORT_EVERY: u64 = 600;

/// The parts of a tick, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    World,
    Sections,
    Items,
    Npcs,
    Projectiles,
    Damage,
    Spawning,
    Housing,
    Sync,
}

impl Phase {
    const NAMES: [&'static str; 9] = [
        "world",
        "sections",
        "items",
        "npcs",
        "projectiles",
        "damage",
        "spawning",
        "housing",
        "sync",
    ];
}

/// Where one tick's time went.
///
/// `cpu` is what the tick cost; `wall` is how long it took to happen. They differ by however long
/// the OS gave this core to something else, which on a machine that is also running the game can
/// be tens of milliseconds. Keeping them apart is what stops a busy laptop from being reported as
/// a slow server.
#[derive(Debug, Default, Clone, Copy)]
struct TickCost {
    cpu: Duration,
    wall: Duration,
    phases: [Duration; Phase::NAMES.len()],
}

impl TickCost {
    /// The phase that took the longest, which is the one worth naming in a warning.
    fn worst_phase(&self) -> (&'static str, Duration) {
        self.phases
            .iter()
            .enumerate()
            .max_by_key(|(_, d)| **d)
            .map_or(("none", Duration::ZERO), |(i, d)| (Phase::NAMES[i], *d))
    }
}

/// Spawn slots a boss fight may fill with summoned minions.
const MAX_MINION_SLOTS: f32 = 40.0;

/// The Guide, who arrives as soon as there is a house.
const GUIDE: u16 = 22;

/// Ticks between housing scans. The room check is a flood fill, so it is not run every tick.
const HOUSING_SCAN_INTERVAL: u64 = 60 * 5;

/// One invader every this many ticks. An invasion arrives steadily rather than all at once.
const INVASION_SPAWN_EVERY: u64 = 45;

/// The chest tile. Placing one needs a container behind it, not just tiles.
const CHEST_BLOCK: u16 = 21;

/// Ticks between NPC position broadcasts. Clients interpolate between them.
const NPC_SYNC_INTERVAL: u64 = 6;

/// Which section a world position falls in.
fn section_of(at: (f32, f32)) -> (i32, i32) {
    (
        (at.0 / crate::game::npc::TILE) as i32 / terrustia_proto::section::SECTION_WIDTH,
        (at.1 / crate::game::npc::TILE) as i32 / terrustia_proto::section::SECTION_HEIGHT,
    )
}

/// Whether somebody standing here has that section loaded.
fn near_section(standing: (f32, f32), section: (i32, i32)) -> bool {
    let theirs = section_of(standing);
    (theirs.0 - section.0).abs() <= SECTION_REACH && (theirs.1 - section.1).abs() <= SECTION_REACH
}

/// How far around a meteor's centre to push at clients, which is comfortably past the crater.
const METEOR_REACH: i32 = 40;

/// How often to check that the Old Man is still at his post.
const OLD_MAN_CHECK_INTERVAL: u64 = 60 * 5;
/// How near a player has to be, in tiles, before it is worth putting him back.
const OLD_MAN_NOTICE: f32 = 250.0;

/// How many sections either way still count as near enough to be told about.
///
/// One section is 200 by 150 tiles, so this reaches 600 by 450 — comfortably past what anybody
/// can see, which is the point: a client should be told about something before it comes on
/// screen, not as it arrives.
const SECTION_REACH: i32 = 1;

/// How many times in a row an NPC's state may be withheld from a player who is not near it.
///
/// Withholding it entirely would leave a distant NPC frozen where a client last saw it; the game
/// lets four go by and then sends one anyway.
const MAX_NPC_SYNC_SKIPS: u8 = 4;

// The projectiles whose debuffs are worth however many of them are stuck in the target. Each is
// its own debuff — Daybreak, javelins, tentacle spikes, blood butcherer knives, stardust cells —
// and the count is what decides the rate, so a single spear is a scratch and eight are lethal.
const DAYBREAK_SPEAR: u16 = 636;
const JAVELIN: u16 = 598;
const TENTACLE_SPIKE: u16 = 971;
const BLOOD_BUTCHERER: u16 = 975;
const STARDUST_CELL: u16 = 614;

// The three commands a mannequin's slot-sync message can carry, besides the armour slots which
// are everything else. Named because the numbers appear nowhere else and mean nothing on sight.
const DOLL_DYE: u8 = 1;
const DOLL_POSE: u8 = 2;
const DOLL_MISC: u8 = 3;

/// What a client sends to say it has closed whatever tile entity it had open.
const NO_TILE_ENTITY: i32 = -1;

// The five things packet 73 can ask for, in the order `MessageBuffer` reads them.
const TELEPORT_POTION: u8 = 0;
const MAGIC_CONCH: u8 = 1;
const DEMON_CONCH: u8 = 2;
const SHELLPHONE_SPAWN: u8 = 3;
const NO_SPACE_RESCUE: u8 = 4;

/// How far in from either edge the ocean reaches. `WorldGen.beachDistance`.
const BEACH_DISTANCE: i32 = 380;
/// ...and how much of that is water rather than sand worth standing on.
const BEACH_MARGIN: i32 = 50;

/// A player's box, which decides where their top-left corner goes for a given tile.
const PLAYER_HALF_WIDTH: f32 = 10.0;
const PLAYER_HEIGHT: f32 = 42.0;

/// The world position of a tile's top-left corner.
fn tile_corner(x: i16, y: i16) -> (f32, f32) {
    (
        f32::from(x) * crate::game::npc::TILE,
        f32::from(y) * crate::game::npc::TILE,
    )
}

/// A target dummy, whose whole purpose is to be hit without ever dying.
const TARGET_DUMMY_AI: i32 = 92;
/// The one other type the game marks immortal outright.
const IMMORTAL_TYPE: u16 = 690;

/// Whether an NPC cannot lose life at all, however much is done to it. `NPC.immortal`.
///
/// Distinct from "takes no damage": a target dummy shows the numbers, which is the entire point
/// of it, and simply never goes below the life it started with.
fn is_immortal(npc: &crate::game::npc::Npc) -> bool {
    npc.stats.ai_style == TARGET_DUMMY_AI || npc.npc_type == IMMORTAL_TYPE
}

/// The count a running timer is set back to whenever it fires.
///
/// Five minutes of ticks, and a multiple of every timer period, which is what keeps two timers of
/// the same kind firing on the same tick however long they have been running.
const TIMER_WINDOW: i32 = 18_000;

/// Every timer in the world that is switched on.
///
/// Walked once, when the world is loaded. It is the whole world, but only once and only for the
/// tile type, which costs a few milliseconds even on a large map.
fn running_timers_in(world: &World) -> HashMap<(i32, i32), i32> {
    let mut found = HashMap::new();
    for x in 0..world.width() {
        for y in 0..world.height() {
            if crate::world::wiring::timer_is_running(world.tile(x, y)) {
                found.insert((x, y), TIMER_WINDOW);
            }
        }
    }
    if !found.is_empty() {
        info!(
            timers = found.len(),
            "picked up timers that were left running"
        );
    }
    found
}

/// Chat colour for server notices.
const SERVER_CHAT_COLOUR: [u8; 3] = [255, 240, 20];

/// Things the connection tasks tell the game task about.
pub enum ServerEvent {
    Join {
        addr: SocketAddr,
        out: mpsc::Sender<Bytes>,
        /// Receives the assigned slot, or `None` when the server is full.
        slot: oneshot::Sender<Option<u8>>,
    },
    Packet {
        slot: u8,
        frame: Frame,
    },
    Leave {
        slot: u8,
    },
}

pub struct GameServer {
    config: Config,
    world: World,
    players: Vec<Option<Player>>,
    ticks: u64,
    items: ItemStore,
    npcs: NpcStore,
    /// Encoded sections, keyed by section coordinates.
    ///
    /// A section is several kilobytes of deflate that costs about 0.13 ms to build, and the same
    /// one goes to every player who joins or walks past. Caching turns that into a memcpy; the
    /// whole cache for a full-size world is well under a megabyte.
    section_cache: HashMap<(i32, i32), Bytes>,
    rng: SmallRng,
    save_path: Option<PathBuf>,
    /// Ticks between autosaves, or `None` when saving is unavailable or disabled.
    autosave_ticks: Option<u64>,
    /// Everything currently in flight.
    projectiles: crate::game::projectile::ProjectileStore,
    /// How many shots have been fired since the server started, for `/npcs`.
    shots_thrown: u64,
    /// The longest tick seen in the current reporting window.
    /// The invasion under way, if any.
    invasion: Option<InvasionState>,
    /// The Old One's Army, which is a siege rather than an invasion and so keeps its own state.
    army: crate::game::army::ArmyState,
    /// The arena's two ends, surveyed once when the crystal goes down and kept until it is gone.
    army_arena: Option<((i32, i32), (i32, i32))>,
    /// Which moon is up, how far through its waves it is, and how far into the current wave.
    ///
    /// A moon is not an invasion: there is nothing to see off, only twenty waves and one night to
    /// get through as many of them as you can. Dawn ends it wherever you got to.
    moon: crate::game::moons::MoonState,
    /// The Lunar Apocalypse: four pillars and what comes after them.
    lunar: crate::game::lunar::LunarState,
    /// The wind and the rain, which several ported routines read.
    weather: crate::game::weather::Weather,
    /// How many syncs in a row each NPC has been withheld from each player.
    npc_skips: HashMap<(u8, u8), u8>,
    /// Whose turn it is to have the ground around them searched for a house.
    housing_turn: usize,
    /// Timers that are switched on, and how long each has left in its window.
    running_timers: HashMap<(i32, i32), i32>,
    /// Trap tiles that have fired recently, and how long until each can fire again.
    ///
    /// The game keeps the same list, capped at 999 entries; this one is a map because looking a
    /// tile up is what it is for.
    mech_cooldown: HashMap<(i32, i32), i32>,
    /// The last pillar shields, invasion progress and Moon Lord countdown that went out.
    ///
    /// All three are recomputed every tick and almost never move, so they are compared before
    /// they are sent. Broadcasting them unconditionally would be three packets a tick to every
    /// client for the whole of an event.
    last_sent_shields: [i32; 4],
    last_sent_countdown: i32,
    last_sent_invasion: (i32, i32),
    /// Which fish the Angler is asking for today, as an index into the quest table.
    angler_quest: u8,
    /// Who has already handed one in since dawn.
    ///
    /// By name rather than by slot, because that is what the game keys it on and because a player
    /// who reconnects into a different slot should not get a second go at the reward.
    angler_finished_today: std::collections::HashSet<String>,
    /// Which tile entity each player currently has open, if any.
    ///
    /// One player per entity, which is the point: without it two people can open the same
    /// mannequin and each take everything off it.
    tile_entity_anchors: HashMap<u8, i32>,
    /// Liquid waiting to settle. Empty unless something has disturbed it.
    liquids: crate::world::liquid::Liquids,
    /// Which of each pair of hardmode ores this world settled on.
    ore_tiers: crate::world::hardmode::OreTiers,
    worst_tick: TickCost,
    /// The longest a tick has been held off the processor this window.
    worst_stall: Duration,
}

impl GameServer {
    pub fn new(config: Config, mut world: World) -> Self {
        // From here on, tile edits invalidate cached sections.
        world.start_tracking_changes();
        let slots = config.max_players;
        let save_path = config.save_target().map(Path::to_path_buf);
        let autosave_ticks = match (save_path.is_some(), config.autosave_secs) {
            (true, secs) if secs > 0 => Some(secs * 60),
            _ => None,
        };
        if save_path.is_none() {
            warn!(
                "no save target: set `save_file`, or pass --save, to keep this world when the \
                 server stops"
            );
        }

        // Timers that were left switched on are picked up here rather than waiting to be hit
        // again. That is a deliberate divergence: the game keeps its list of running timers only
        // in memory, so reopening a world leaves every one of them drawn as on and doing nothing
        // until somebody flips it twice. On a single-player world that is a curiosity; on a
        // server it means every contraption in the world dies silently on a restart.
        let running_timers = running_timers_in(&world);

        // The weather comes off the world it was loaded with, so a reloaded save picks up the
        // shower it was in the middle of rather than starting clear.
        let weather = crate::game::weather::Weather {
            wind: world.wind,
            target: world.wind,
            raining: world.raining,
            rain_time: world.rain_time,
            max_rain: world.max_rain,
            sandstorm: world.sandstorm,
            sandstorm_time: world.sandstorm_time,
            severity: world.sandstorm_severity,
            intended_severity: world.sandstorm_intended_severity,
            ..Default::default()
        };

        let mut server = Self {
            config,
            world,
            players: (0..slots).map(|_| None).collect(),
            ticks: 0,
            items: ItemStore::new(),
            npcs: NpcStore::new(),
            section_cache: HashMap::new(),
            rng: SmallRng::seed_from_u64(0x7e77_a51a),
            projectiles: crate::game::projectile::ProjectileStore::new(),
            shots_thrown: 0,
            invasion: None,
            army: crate::game::army::ArmyState::default(),
            army_arena: None,
            moon: crate::game::moons::MoonState::default(),
            lunar: crate::game::lunar::LunarState::default(),
            weather,
            npc_skips: HashMap::new(),
            housing_turn: 0,
            running_timers,
            mech_cooldown: HashMap::new(),
            tile_entity_anchors: HashMap::new(),
            // Deliberately impossible starting values, so the first tick of each always sends.
            last_sent_shields: [-1; 4],
            last_sent_countdown: -1,
            last_sent_invasion: (-1, -1),
            angler_quest: 0,
            angler_finished_today: std::collections::HashSet::new(),
            liquids: crate::world::liquid::Liquids::default(),
            ore_tiers: crate::world::hardmode::OreTiers::default(),
            worst_tick: TickCost::default(),
            worst_stall: Duration::ZERO,
            save_path,
            autosave_ticks,
        };
        // The Angler wants something from the moment the world opens, not from the first dawn.
        // A server that waited would give the first day's players nothing to do for him.
        server.roll_angler_quest();
        server
    }

    /// Write the world to disk, announcing the outcome in chat.
    ///
    /// Serialisation runs on the game task because it needs exclusive access to the world; it takes
    /// a fraction of a second even for a full-size world, which is why it is not worth the cost of
    /// snapshotting eighty megabytes of tiles to move it off-thread.
    fn save_world(&mut self, reason: &str) {
        let Some(path) = self.save_path.clone() else {
            return;
        };
        let started = Instant::now();
        match wld_save::save(&self.world, &path) {
            Ok(()) => {
                let ms = started.elapsed().as_millis();
                info!(path = %path.display(), reason, elapsed_ms = ms as u64, "world saved");
                self.announce(&format!("World saved ({ms} ms)."));
            }
            Err(e) => {
                error!(path = %path.display(), error = %e, "world save failed");
                self.announce("World save FAILED; see the server log.");
            }
        }
    }

    pub async fn run(mut self, mut events: mpsc::Receiver<ServerEvent>) {
        let mut ticker = interval(TICK);
        // Catching up on missed ticks would fast-forward the world clock after any stall.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                event = events.recv() => match event {
                    Some(event) => self.handle_event(event),
                    None => break,
                },
                _ = ticker.tick() => {
                    let cost = self.tick();
                    self.note_tick_cost(cost);
                }
            }
        }

        // The channel closing is the shutdown signal, so this is the last chance to persist.
        self.save_world("shutdown");
        info!("game loop stopped");
    }

    /// Keep an eye on how much of the sixteen-millisecond budget a tick is actually using.
    ///
    /// A server that is quietly overrunning its budget looks identical to one that is not, right
    /// up until the world starts running slow. Reporting the worst tick in each ten-second window,
    /// and only when it is over half the budget, makes that visible without a line a second.
    ///
    /// Two different problems can push a tick over its budget and they need different answers, so
    /// they get different messages: work that costs too much processor is this server's bug, and a
    /// tick that took a long time without using the processor is the machine being busy elsewhere.
    /// The breakdown comes with the first one, because "a tick took 26 ms" is a mystery and "the
    /// spawn scan took 26 ms" is a bug report.
    fn note_tick_cost(&mut self, cost: TickCost) {
        if cost.cpu > self.worst_tick.cpu {
            self.worst_tick = cost;
        }
        self.worst_stall = self.worst_stall.max(cost.wall.saturating_sub(cost.cpu));
        if !self.ticks.is_multiple_of(TICK_REPORT_EVERY) {
            return;
        }
        let worst = std::mem::take(&mut self.worst_tick);
        let stall = std::mem::take(&mut self.worst_stall);
        debug!(
            cpu_us = worst.cpu.as_micros() as u64,
            wall_us = worst.wall.as_micros() as u64,
            stall_us = stall.as_micros() as u64,
            phase = worst.worst_phase().0,
            npcs = self.npcs.len(),
            "tick window"
        );
        if worst.cpu * 2 > TICK {
            let (phase, phase_cost) = worst.worst_phase();
            warn!(
                worst_us = worst.cpu.as_micros() as u64,
                budget_us = TICK.as_micros() as u64,
                phase,
                phase_us = phase_cost.as_micros() as u64,
                npcs = self.npcs.len(),
                projectiles = self.projectiles.len(),
                "ticks are using a lot of their budget"
            );
        } else if stall > TICK {
            // Not a warning: nothing here is wrong, the machine is just busy. Worth saying,
            // because a player will feel it either way.
            info!(
                stall_us = stall.as_micros() as u64,
                cpu_us = worst.cpu.as_micros() as u64,
                "the game loop was held off the processor; the machine is busy elsewhere"
            );
        }
    }

    fn tick(&mut self) -> TickCost {
        let mut cost = TickCost::default();
        let began = Instant::now();
        let cpu_began = clock::Cpu::now();
        let mut clock = Instant::now();
        let mut lap = |cost: &mut TickCost, phase: Phase| {
            let now = Instant::now();
            cost.phases[phase as usize] += now - clock;
            clock = now;
        };

        self.ticks += 1;
        let was_day = self.world.day_time;
        self.world.tick_time();
        // Dawn puts the moons away and takes the blood moon with them, and rolls for an eclipse.
        if self.world.day_time && !was_day {
            self.stop_moon();
            self.world.blood_moon = false;
            self.roll_dawn_events();
            self.broadcast_world_data();
        }
        // Dusk rolls for a blood moon, which needs somebody with more than a hundred and twenty
        // life to be worth having.
        if !self.world.day_time && was_day {
            self.roll_dusk_events();
        }
        if !self.world.day_time && was_day && self.world.eclipse {
            self.world.eclipse = false;
            self.announce("The solar eclipse is over.");
            self.broadcast_world_data();
        }

        if let Some(every) = self.autosave_ticks
            && self.ticks.is_multiple_of(every)
        {
            self.save_world("autosave");
        }
        self.tick_tile_entities();
        self.tick_liquids();
        self.tick_spread();
        self.tick_weather();
        self.tick_mech_cooldowns();
        self.tick_timers();
        self.tick_lunar();
        lap(&mut cost, Phase::World);

        self.flush_dirty_sections();
        lap(&mut cost, Phase::Sections);
        self.tick_items();
        lap(&mut cost, Phase::Items);
        self.tick_npc_buffs();
        self.tick_npcs();
        lap(&mut cost, Phase::Npcs);
        self.tick_projectiles();
        lap(&mut cost, Phase::Projectiles);
        self.tick_contact_damage();
        lap(&mut cost, Phase::Damage);
        self.tick_spawning();
        lap(&mut cost, Phase::Spawning);
        self.tick_town_npcs();
        self.tick_old_man();
        lap(&mut cost, Phase::Housing);

        if self.ticks.is_multiple_of(TIME_SYNC_TICKS)
            && self.players.iter().flatten().any(Player::is_playing)
        {
            let time = packets::TimeSet {
                day_time: self.world.day_time,
                time: self.world.time,
                sun_mod_y: 0,
                moon_mod_y: 0,
            };
            if let Ok(frame) = time.encode() {
                self.broadcast(frame, None);
            }
        }
        lap(&mut cost, Phase::Sync);

        cost.cpu = clock::Cpu::now().since(cpu_began);
        cost.wall = began.elapsed();
        cost
    }

    /// Age items, settle falling ones, and hand nearby ones to a player who can pick them up.
    fn tick_items(&mut self) {
        for index in self.items.tick() {
            if let Ok(frame) = terrustia_proto::items::item_despawn(index) {
                self.broadcast(frame, None);
            }
        }

        // Falling items need their landing broadcast, but nothing in between: a client draws the
        // arc itself once it knows where the item started.
        let mut landed = Vec::new();
        let world = &self.world;
        for (index, item) in self
            .items
            .iter()
            .filter(|(_, i)| !i.resting)
            .map(|(i, item)| (i, *item))
            .collect::<Vec<_>>()
        {
            let mut item = item;
            items::fall(&mut item, |x, y| world.tile(x, y).is_active());
            let settled = item.resting;
            if let Some(slot) = self.items.get_mut(index) {
                *slot = item;
            }
            if settled {
                landed.push(index);
            }
        }
        for index in landed {
            self.broadcast_item(index);
        }

        // Offer unreserved items to the nearest player in range.
        let positions: Vec<(u8, (f32, f32))> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing())
            .map(|p| (p.slot, p.position))
            .collect();
        if positions.is_empty() {
            return;
        }

        let offers: Vec<(i16, u8, (f32, f32))> = self
            .items
            .iter()
            .filter(|(_, item)| !item.is_reserved())
            .filter_map(|(index, item)| {
                positions
                    .iter()
                    .map(|(slot, pos)| {
                        let (dx, dy) = (pos.0 - item.position.0, pos.1 - item.position.1);
                        (*slot, dx * dx + dy * dy)
                    })
                    .filter(|(_, d2)| *d2 <= ITEM_GRAB_RANGE * ITEM_GRAB_RANGE)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(slot, _)| (index, slot, item.position))
            })
            .collect();

        for (index, owner, position) in offers {
            if let Some(item) = self.items.get_mut(index) {
                item.owner = owner;
                item.reservation = items::RESERVATION_TICKS;
            }
            if let Ok(frame) = ItemOwner::reserve(index, owner, position).encode() {
                self.broadcast(frame, None);
            }
        }
    }

    /// Tell everyone the current state of one item.
    fn broadcast_item(&mut self, index: i16) {
        let Some(item) = self.items.get(index).copied() else {
            return;
        };
        let mut sync = SyncItem::dropped(index, item.position, item.item);
        sync.velocity = item.velocity;
        if let Ok(frame) = sync.encode() {
            self.broadcast(frame, None);
        }
    }

    /// Drop whatever a broken tile yields, if it is a block with a simple drop.
    fn spawn_tile_drop(&mut self, tile: u16, frame_x: i16, frame_y: i16, x: i32, y: i32) {
        let Some(item_id) = drop_of(tile, frame_x, frame_y) else {
            debug!(tile, "nothing known to drop for this tile type");
            return;
        };
        let position = (x as f32 * 16.0, y as f32 * 16.0);
        let Some(index) = self.items.spawn(ItemStack::new(item_id, 1, 0), position) else {
            debug!("item slots are full; the drop was discarded");
            return;
        };
        self.broadcast_item(index);
    }

    /// Put an item into the world at a place, as a thing that can be picked up.
    fn spawn_item(&mut self, item: ItemStack, position: (f32, f32)) {
        if item.is_empty() {
            return;
        }
        let Some(index) = self.items.spawn(item, position) else {
            debug!("item slots are full; the drop was discarded");
            return;
        };
        self.broadcast_item(index);
    }

    /// Packet 21: a client dropping something, or updating an item it holds the reservation on.
    fn on_sync_item(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let sync = SyncItem::decode(payload)?;

        if sync.is_new() {
            let Some(index) = self.items.spawn(sync.item, sync.position) else {
                return Ok(());
            };
            if let Some(item) = self.items.get_mut(index) {
                item.velocity = sync.velocity;
                // A player throwing an item keeps first claim on it.
                item.owner = slot;
                item.reservation = items::RESERVATION_TICKS;
            }
            self.broadcast_item(index);
            return Ok(());
        }

        // Otherwise only the reserving player may move it.
        match self.items.get_mut(sync.index) {
            Some(item) if item.owner == slot => {
                item.item = sync.item;
                item.position = sync.position;
                item.velocity = sync.velocity;
                item.resting = false;
            }
            _ => {
                debug!(
                    slot,
                    index = sync.index,
                    "ignoring item update from a non-owner"
                );
                return Ok(());
            }
        }

        self.broadcast(sync.encode()?, Some(slot));
        Ok(())
    }

    /// Packet 151: a client reporting that it picked an item up.
    fn on_item_despawn(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let index = decode_item_despawn(payload)?;
        match self.items.get(index) {
            Some(item) if item.owner == slot => {}
            _ => {
                debug!(
                    slot,
                    index, "ignoring pickup of an item reserved for someone else"
                );
                return Ok(());
            }
        }
        self.items.remove(index);
        self.broadcast(terrustia_proto::items::item_despawn(index)?, Some(slot));
        Ok(())
    }

    fn handle_event(&mut self, event: ServerEvent) {
        match event {
            ServerEvent::Join { addr, out, slot } => {
                let assigned = self.allocate_slot(addr, out);
                let _ = slot.send(assigned);
            }
            ServerEvent::Packet { slot, frame } => self.handle_packet(slot, frame),
            ServerEvent::Leave { slot } => self.remove_player(slot),
        }
    }

    // ---------------------------------------------------------------- players

    fn allocate_slot(&mut self, addr: SocketAddr, out: mpsc::Sender<Bytes>) -> Option<u8> {
        let slot = self.players.iter().position(Option::is_none)?;
        let slot = u8::try_from(slot).ok()?;
        self.players[slot as usize] = Some(Player::new(slot, addr, out));
        debug!(%addr, slot, "connection accepted into a slot");
        Some(slot)
    }

    fn player(&self, slot: u8) -> Option<&Player> {
        self.players.get(slot as usize)?.as_ref()
    }

    fn player_mut(&mut self, slot: u8) -> Option<&mut Player> {
        self.players.get_mut(slot as usize)?.as_mut()
    }

    fn remove_player(&mut self, slot: u8) {
        let Some(player) = self.players.get_mut(slot as usize).and_then(Option::take) else {
            return;
        };
        info!(slot, name = %player.name, "player disconnected");

        // Whatever they had open is free again. Without this a mannequin somebody was looking at
        // when their connection dropped stays locked for the rest of the world's life.
        if self.tile_entity_anchors.remove(&slot).is_some()
            && let Ok(frame) = {
                let mut w = terrustia_proto::PacketWriter::new(id::REQUEST_TILE_ENTITY_INTERACTION);
                w.i32(NO_TILE_ENTITY).u8(slot);
                w.finish()
            }
        {
            self.broadcast(frame, Some(slot));
        }

        if player.state == ConnState::Playing {
            if let Ok(frame) = packets::player_active(slot, false) {
                self.broadcast(frame, Some(slot));
            }
            self.announce(&format!("{} has left.", player.name));
        }
    }

    /// Queue a frame for one player, dropping the connection if its queue has backed up.
    fn send(&mut self, slot: u8, frame: Vec<u8>) {
        self.send_bytes(slot, Bytes::from(frame));
    }

    fn send_bytes(&mut self, slot: u8, frame: Bytes) {
        let Some(out) = self.player(slot).map(|p| p.out.clone()) else {
            return;
        };
        // A client that cannot keep up would otherwise grow the queue without bound. Dropping it is
        // the same call vanilla makes, and the read task notices the closed channel.
        //
        // A *closed* channel is a different thing and must not be reported as the same: it means
        // the connection has already gone, which happens every time anybody leaves — and every
        // player still on the server is then sent the news, one send per departed connection.
        // Calling that "dropping connection" at warning level sends an operator looking for a
        // network problem that is not there.
        let id = frame.get(2).copied().unwrap_or(255);
        match out.try_send(frame) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => {
                debug!(slot, "sending to a connection that has already gone");
                self.remove_player(slot);
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!(
                    slot,
                    packet = id,
                    name = terrustia_proto::id::name(id),
                    "outbound queue full; dropping a client that cannot keep up"
                );
                self.remove_player(slot);
            }
        }
    }

    /// Send to every player who is in the world, optionally skipping one.
    fn broadcast(&mut self, frame: Vec<u8>, except: Option<u8>) {
        let bytes = Bytes::from(frame);
        // Collect first: sending can remove a player, which would invalidate an in-flight iterator.
        let targets: Vec<u8> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing() && Some(p.slot) != except)
            .map(|p| p.slot)
            .collect();
        for slot in targets {
            self.send_bytes(slot, bytes.clone());
        }
    }

    fn announce(&mut self, text: &str) {
        info!("{text}");
        if let Ok(frame) = net_module::chat_broadcast(
            net_module::SERVER_AUTHOR,
            &NetworkText::literal(text),
            SERVER_CHAT_COLOUR,
        ) {
            self.broadcast(frame, None);
        }
    }

    fn kick(&mut self, slot: u8, reason: &str) {
        debug!(slot, reason, "kicking");
        if let Ok(frame) = packets::kick(&NetworkText::literal(reason)) {
            self.send(slot, frame);
        }
        self.remove_player(slot);
    }

    // ---------------------------------------------------------------- packets

    fn handle_packet(&mut self, slot: u8, frame: Frame) {
        let payload = frame.payload;
        let result = match frame.id {
            id::HELLO => self.on_hello(slot, &payload),
            id::SYNC_PLAYER => self.on_sync_player(slot, &payload),
            id::SYNC_EQUIPMENT => self.on_equipment(slot, &payload),
            id::SPAWN_BOSS_USE_LICENSE_START_EVENT => self.on_summon(slot, &payload),
            id::REQUEST_WORLD_DATA => self.on_request_world_data(slot),
            id::SPAWN_TILE_DATA => self.on_spawn_tile_data(slot, &payload),
            id::PLAYER_SPAWN => self.on_player_spawn(slot, &payload),
            id::PLAYER_CONTROLS => self.on_player_controls(slot, &payload),
            id::PLAYER_LIFE_MANA => self.on_health(slot, &payload),
            id::PLAYER_MANA => self.on_mana(slot, &payload),
            id::CLIENT_UUID => self.on_uuid(slot, &payload),
            id::SEND_PASSWORD => self.on_password(slot, &payload),
            id::TEAM_CHANGE | id::TEAM_CHANGE_FROM_U_I => self.on_team(slot, &payload),
            id::TOGGLE_P_V_P => self.on_pvp(slot, &payload),
            id::PLAYER_BUFFS => self.on_buffs(slot, &payload),
            id::SYNC_PLAYER_ZONE => self.on_zone(slot, &payload),
            // Damage and death are not simulated; relaying keeps every client's view of another
            // player's health and death messages consistent with the client that took the hit.
            id::PLAYER_HURT_V2 | id::PLAYER_DEATH_V2 | id::DEAD_PLAYER => {
                self.relay_player_packet(slot, frame.id, &payload)
            }
            // Everything a player does that only other clients need to be told about: a heal, a
            // mana burst, the angle of the item they are holding, a ninja dodge, their stealth, a
            // flute note, which NPC their minions are on, which tile they are mining, and which
            // loadout they just switched to. Each is the same shape — the owner byte is rewritten
            // to the connection it arrived on, so nobody can act as anybody else — and none of
            // them is something the server has an opinion about.
            id::PLAYER_HEAL
            | id::ITEM_ROTATION_AND_ANIMATION
            | id::MANA_EFFECT
            | id::INSTRUMENT_SOUND
            | id::SYNC_DODGE
            | id::PLAYER_STEALTH
            | id::MINION_ATTACK_TARGET_UPDATE
            | id::SYNC_TILE_PICKING
            | id::SYNC_LOADOUT => self.relay_player_packet(slot, frame.id, &payload),
            id::CRYSTAL_INVASION_START => self.on_crystal_placed(slot, &payload),
            id::SYNC_TILE_PAINT_OR_COATING | id::SYNC_WALL_PAINT_OR_COATING => {
                self.on_paint(slot, frame.id, &payload)
            }
            id::MISC_DATA_SYNC => self.on_misc_data(slot, &payload),
            id::LOCK_AND_UNLOCK => self.on_lock(slot, &payload),
            id::CHEST_UPDATES => self.on_chest_update(slot, &payload),
            id::TILE_ENTITY_PLACEMENT => self.on_tile_entity_placed(slot, &payload),
            id::HIT_SWITCH => self.on_hit_switch(slot, &payload),
            id::BUG_CATCHING => self.on_bug_caught(slot, &payload),
            id::BUG_RELEASING => self.on_bug_released(slot, &payload),
            id::LIQUID_UPDATE => self.on_liquid(slot, &payload),
            // Social chatter and cosmetic effects: nothing to keep, but everyone else has to see
            // it or the world looks different from each side.
            id::SYNC_EMOTE_BUBBLE
            | id::EMOJI
            | id::TOGGLE_PARTY
            | id::PING
            | id::SPECIAL_F_X
            | id::ITEM_USE_SOUND
            | id::MINION_REST_TARGET_UPDATE
            | id::SYNC_PROJECTILE_TRACKERS
            | id::UPDATE_PLAYER_LUCK_FACTORS
            | id::SYNC_REVENGE_MARKER
            | id::REMOVE_REVENGE_MARKER
            | id::LAND_GOLF_BALL_IN_CUP
            | id::COMBAT_TEXT_INT
            | id::COMBAT_TEXT_STRING => {
                if self.player(slot).is_some_and(Player::is_playing)
                    && let Ok(relayed) = packets::verbatim(frame.id, &payload)
                {
                    self.broadcast(relayed, Some(slot));
                }
                Ok(())
            }
            id::PLACE_OBJECT => self.on_place_object(slot, &payload),
            id::TELEPORT_ENTITY => self.on_teleport(slot, &payload),
            // Which town NPC a player is talking to. The owner byte is first, so the ordinary
            // relay handles it, and remembering it is what a shop will need.
            id::SYNC_TALK_N_P_C => self.on_talk_npc(slot, &payload),
            id::REQUEST_SECTION => self.on_request_section(slot, &payload),
            id::AREA_TILE_CHANGE => self.on_tile_square(slot, &payload),
            id::SYNC_ITEM | id::SPAWN_INSTANCED_ITEM => self.on_sync_item(slot, &payload),
            id::SYNC_ITEM_DESPAWN => self.on_item_despawn(slot, &payload),
            id::DAMAGE_N_P_C => self.on_damage_npc(slot, &payload),
            id::TOGGLE_DOOR_STATE => self.on_door(slot, &payload),
            id::REQUEST_CHEST_OPEN => self.on_chest_open(slot, &payload),
            id::SYNC_CHEST_ITEM => self.on_chest_item(slot, &payload),
            id::SYNC_PLAYER_CHEST => self.on_player_chest(slot, &payload),
            id::OPEN_SIGN_REQUEST => self.on_sign_request(slot, &payload),
            id::OPEN_SIGN_RESPONSE => self.on_sign_write(slot, &payload),
            id::TILE_MANIPULATION => self.on_tile_manipulation(slot, &payload),
            id::NET_MODULES => self.on_net_module(slot, &payload),
            id::SYNC_PROJECTILE => self.on_client_projectile(slot, &payload),
            id::KILL_PROJECTILE => self.on_client_projectile_kill(slot, &payload),
            // Four different ids for one message: putting an item into a frame, onto a weapon
            // rack, onto a food platter, or into a display jar.
            id::ITEM_FRAME_TRY_PLACING
            | id::WEAPONS_RACK_TRY_PLACING
            | id::FOOD_PLATTER_TRY_PLACING
            | id::DEAD_CELLS_DISPLAY_JAR_TRY_PLACING => self.on_display_item(slot, &payload),
            id::T_E_LEASHED_ENTITY_ANCHOR_PLACE_ITEM => self.on_anchor_item(slot, &payload),
            id::REQUEST_TELEPORTATION_BY_SERVER => self.on_server_teleport(slot, &payload),
            id::ANGLER_QUEST_FINISHED => self.on_angler_finished(slot),
            id::QUESTS_COUNT_SYNC => self.on_quest_count(slot, &payload),
            id::T_E_DISPLAY_DOLL_DATA_SYNC => self.on_display_doll_slot(slot, &payload),
            id::T_E_HAT_RACK_ITEM_SYNC => self.on_hat_rack_slot(slot, &payload),
            id::REQUEST_TILE_ENTITY_INTERACTION => {
                self.on_tile_entity_interaction(slot, &payload)
            }
            id::ADD_N_P_C_BUFF => self.on_add_npc_buff(slot, &payload),
            id::REQUEST_N_P_C_BUFF_REMOVAL => self.on_remove_npc_buff(slot, &payload),
            id::UNIQUE_TOWN_N_P_C_INFO_SYNC_REQUEST => self.on_town_npc_name_request(slot, &payload),
            other => {
                debug!(slot, id = other, name = id::name(other), "ignoring packet");
                Ok(())
            }
        };

        if let Err(e) = result {
            debug!(slot, id = frame.id, name = id::name(frame.id), error = %e, "malformed packet");
        }
    }

    fn on_hello(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if self.player(slot).map(|p| p.state) != Some(ConnState::Greeting) {
            return Ok(()); // a second hello is not a way to restart the handshake
        }

        let hello = Hello::decode(payload)?;
        if !hello.is_supported() {
            info!(slot, version = %hello.version, "rejecting unsupported client");
            self.kick(
                slot,
                &format!(
                    "This server runs Terraria {} (protocol {}).",
                    "1.4.5.7",
                    id::CUR_RELEASE
                ),
            );
            return Ok(());
        }

        if let Some(player) = self.player_mut(slot) {
            player.greeted = true;
        }

        // With a password set, the slot is withheld until the client proves it knows it.
        if !self.config.password.is_empty() {
            self.send(slot, packets::empty(id::REQUEST_PASSWORD)?);
            return Ok(());
        }

        self.accept_player(slot)
    }

    /// Assign the slot and let the client proceed.
    fn accept_player(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        if let Some(player) = self.player_mut(slot) {
            player.password_ok = true;
            player.advance_to(ConnState::SlotAssigned);
        }
        // The trailing bool is new in 1.4.5; see docs/protocol-notes.md.
        let frame = packets::player_info(slot, false)?;
        self.send(slot, frame);
        Ok(())
    }

    /// Packet 38: the client's answer to a password prompt.
    fn on_password(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        // Only a connection that has already passed the version check may offer a password.
        let ready = self
            .player(slot)
            .is_some_and(|p| p.greeted && p.state == ConnState::Greeting);
        if self.config.password.is_empty() || !ready {
            return Ok(());
        }
        let offered = PacketReader::new(payload).string()?;
        if constant_time_eq(offered.as_bytes(), self.config.password.as_bytes()) {
            self.accept_player(slot)
        } else {
            info!(slot, "wrong password");
            self.kick(slot, "Incorrect password.");
            Ok(())
        }
    }

    /// Relay a packet that describes the sender, stamping our slot over whatever they claimed.
    fn relay_player_packet(
        &mut self,
        slot: u8,
        message: u8,
        payload: &[u8],
    ) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let frame = packets::rewrite_owner(message, payload, slot)?;
        self.broadcast(frame, Some(slot));
        Ok(())
    }

    fn on_sync_player(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        // The name sits after slot, skin variant, voice variant, voice pitch and hair.
        let mut r = PacketReader::new(payload);
        r.bytes(1 + 1 + 1)?;
        r.f32()?;
        r.u8()?;
        let name = r.string()?;

        if let Some(player) = self.player_mut(slot) {
            if !name.trim().is_empty() {
                player.name = name.trim().to_string();
            }
            player.appearance = Some(Bytes::copy_from_slice(payload));
            player.advance_to(ConnState::Identified);
        }

        // Relay live appearance changes; a first-time sync reaches others at spawn instead.
        if self.player(slot).is_some_and(Player::is_playing) {
            let frame = packets::rewrite_owner(id::SYNC_PLAYER, payload, slot)?;
            self.broadcast(frame, Some(slot));
        }
        Ok(())
    }

    fn on_request_world_data(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        let frame = self.world.world_data().encode()?;
        self.send(slot, frame);
        if let Some(player) = self.player_mut(slot) {
            player.advance_to(ConnState::WorldSent);
        }
        Ok(())
    }

    fn on_spawn_tile_data(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let request = SpawnTileData::decode(payload)?;

        // Vanilla re-sends world data here before the tiles; mirroring it keeps the client's
        // loading sequence identical to the one it was written against.
        let world_data = self.world.world_data().encode()?;
        self.send(slot, world_data);

        let sections = self.sections_for(request);
        let status = packets::status_text(
            sections.len() as i32,
            &NetworkText::literal("Receiving tile data"),
            0,
        )?;
        self.send(slot, status);

        for (sx, sy) in sections {
            self.send_section(slot, sx, sy)?;
        }

        // Vanilla sends the live entities after the tiles and before StartPlaying; without this a
        // joining player sees an empty world where everyone else sees dropped loot.
        let existing: Vec<(i16, (f32, f32), ItemStack)> = self
            .items
            .iter()
            .map(|(index, item)| (index, item.position, item.item))
            .collect();
        for (index, position, stack) in existing {
            self.send(slot, SyncItem::dropped(index, position, stack).encode()?);
        }

        self.send_npcs(slot)?;

        if let Some(player) = self.player_mut(slot) {
            player.advance_to(ConnState::TilesSent);
        }
        self.send(slot, packets::empty(id::INITIAL_SPAWN)?);
        Ok(())
    }

    /// Stream one section, unless this client already has it.
    fn send_section(&mut self, slot: u8, sx: i32, sy: i32) -> terrustia_proto::Result<()> {
        if sx < 0 || sy < 0 || sx >= self.world.sections_x() || sy >= self.world.sections_y() {
            return Ok(());
        }
        // `insert` returns false when the client already has this section.
        let is_new = match self.player_mut(slot) {
            Some(player) => player.sent_sections.insert((sx, sy)),
            None => false,
        };
        if !is_new {
            return Ok(());
        }

        let bounds = self.world.section_bounds(sx, sy);
        if bounds.width == 0 || bounds.height == 0 {
            return Ok(());
        }
        self.flush_dirty_sections();
        let frame = match self.section_cache.get(&(sx, sy)) {
            Some(cached) => cached.clone(),
            None => {
                let extras = self.world.extras_for(bounds);
                let encoded = Bytes::from(encode_section_packet(bounds, &extras, |x, y| {
                    self.world.tile(x, y)
                })?);
                self.section_cache.insert((sx, sy), encoded.clone());
                encoded
            }
        };
        self.send_bytes(slot, frame);
        Ok(())
    }

    /// Packet 159: the client asking for one section as it moves.
    ///
    /// New in 1.4.5 — previously the server pushed sections from the player's position. Without
    /// this a player can walk out of the area streamed at spawn and see nothing but sky.
    fn on_request_section(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if self
            .player(slot)
            .is_none_or(|p| p.state < ConnState::TilesSent)
        {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let sx = i32::from(r.u16()?);
        let sy = i32::from(r.u16()?);
        self.send_section(slot, sx, sy)
    }

    /// The sections vanilla streams on a tile request: a block around spawn, plus one around the
    /// requested position when it is a real location.
    fn sections_for(&self, request: SpawnTileData) -> Vec<(i32, i32)> {
        let mut wanted = HashSet::new();
        let (max_x, max_y) = (self.world.sections_x(), self.world.sections_y());

        let mut add_block = |cx: i32, cy: i32, w: i32, h: i32| {
            for sx in (cx - 2)..(cx - 2 + w) {
                for sy in (cy - 1)..(cy - 1 + h) {
                    if sx >= 0 && sy >= 0 && sx < max_x && sy < max_y {
                        wanted.insert((sx, sy));
                    }
                }
            }
        };

        let (spawn_sx, spawn_sy) = self
            .world
            .section_of(i32::from(self.world.spawn_x), i32::from(self.world.spawn_y));
        add_block(spawn_sx, spawn_sy, 5, 3);

        let valid = request.x >= 10
            && request.y >= 10
            && request.x < self.world.width() - 10
            && request.y < self.world.height() - 10;
        if valid {
            let (sx, sy) = self.world.section_of(request.x, request.y);
            add_block(sx, sy, 6, 4);
        }

        let mut sections: Vec<(i32, i32)> = wanted.into_iter().collect();
        // Deterministic order keeps logs and tests reproducible.
        sections.sort_unstable();
        sections
    }

    fn on_player_spawn(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let spawn = PlayerSpawn::decode(payload)?;
        let was_playing = self.player(slot).is_some_and(Player::is_playing);

        if let Some(player) = self.player_mut(slot) {
            if player.state < ConnState::TilesSent {
                return Ok(()); // spawning before the world arrived is not a valid sequence
            }
            player.team = spawn.team;
            // A respawn puts you back on your feet. Without this the server keeps thinking you
            // are dead, and every routine that checks whether anyone is alive ignores you for the
            // rest of the session.
            if player.life <= 0 {
                player.life = player.life_max.max(1);
                player.immune_ticks = 0;
            }
            player.advance_to(ConnState::Playing);
        } else {
            return Ok(());
        }

        // Always relay the spawn so respawns after death are visible.
        let relay = packets::rewrite_owner(id::PLAYER_SPAWN, payload, slot)?;
        self.broadcast(relay, Some(slot));

        if !was_playing {
            self.introduce(slot)?;
            // What everyone already here is wearing. This waits until they are actually playing
            // rather than going out with the world: a client is still working through its
            // handshake when the tiles arrive, and anything sent then is read by the handshake
            // rather than by the game.
            self.send_existing_equipment(slot);
        }
        Ok(())
    }

    /// Exchange presence between a newly spawned player and everyone already in the world.
    fn introduce(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        let others: Vec<u8> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing() && p.slot != slot)
            .map(|p| p.slot)
            .collect();

        // Tell the newcomer about everyone else.
        for other in &other_slots(&others) {
            for frame in self.presence_frames(*other)? {
                self.send(slot, frame);
            }
        }

        // Tell everyone else about the newcomer.
        for frame in self.presence_frames(slot)? {
            self.broadcast(frame, Some(slot));
        }

        self.send(slot, packets::empty(id::FINISHED_CONNECTING_TO_SERVER)?);

        // What the Angler wants today. A client that is never told shows no quest at all, so a
        // player who joins after dawn would find the Angler had nothing to say until midnight.
        {
            let name = self
                .player(slot)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let done = self.angler_finished_today.contains(&name);
            let mut w = terrustia_proto::PacketWriter::new(id::ANGLER_QUEST);
            w.u8(self.angler_quest).bool(done);
            if let Ok(frame) = w.finish() {
                self.send(slot, frame);
            }
        }

        let name = self
            .player(slot)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        self.announce(&format!("{name} has joined."));

        let motd = self.config.motd.clone();
        if !motd.is_empty()
            && let Ok(frame) = net_module::chat_broadcast(
                net_module::SERVER_AUTHOR,
                &NetworkText::literal(&motd),
                SERVER_CHAT_COLOUR,
            )
        {
            self.send(slot, frame);
        }
        Ok(())
    }

    /// Everything another client needs in order to draw a player.
    fn presence_frames(&self, slot: u8) -> terrustia_proto::Result<Vec<Vec<u8>>> {
        let Some(player) = self.player(slot) else {
            return Ok(Vec::new());
        };

        let mut frames = vec![packets::player_active(slot, true)?];
        if let Some(appearance) = &player.appearance {
            frames.push(packets::rewrite_owner(id::SYNC_PLAYER, appearance, slot)?);
        }
        frames.push(
            PlayerHealth {
                player: slot,
                life: player.life,
                life_max: player.life_max,
            }
            .encode()?,
        );
        frames.push(
            PlayerMana {
                player: slot,
                mana: player.mana,
                mana_max: player.mana_max,
            }
            .encode()?,
        );
        if let Some(buffs) = &player.buffs {
            frames.push(packets::rewrite_owner(id::PLAYER_BUFFS, buffs, slot)?);
        }
        if let Some(zone) = &player.zone {
            frames.push(packets::rewrite_owner(id::SYNC_PLAYER_ZONE, zone, slot)?);
        }
        if player.team != 0 {
            let mut w = terrustia_proto::PacketWriter::new(id::TEAM_CHANGE);
            w.u8(slot).u8(player.team);
            frames.push(w.finish()?);
        }
        if player.pvp {
            let mut w = terrustia_proto::PacketWriter::new(id::TOGGLE_P_V_P);
            w.u8(slot).bool(true);
            frames.push(w.finish()?);
        }
        if let Some(controls) = &player.last_controls {
            frames.push(packets::rewrite_owner(id::PLAYER_CONTROLS, controls, slot)?);
        }
        Ok(frames)
    }

    fn on_player_controls(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let controls = PlayerControls::decode(payload)?;

        if let Some(player) = self.player_mut(slot) {
            // Velocity is what actually changed since the last update, not what the client
            // claims: the routines that lead a running player want the real thing.
            player.velocity = (
                controls.position.0 - player.position.0,
                controls.position.1 - player.position.1,
            );
            player.position = controls.position;
            player.last_controls = Some(Bytes::copy_from_slice(payload));
            if !player.is_playing() {
                return Ok(());
            }
        } else {
            return Ok(());
        }

        // Relayed verbatim: the payload has optional trailing blocks the server does not model.
        let frame = packets::rewrite_owner(id::PLAYER_CONTROLS, payload, slot)?;
        self.broadcast(frame, Some(slot));
        Ok(())
    }

    fn on_health(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let health = PlayerHealth::decode(payload)?;
        if let Some(player) = self.player_mut(slot) {
            player.life = health.life;
            player.life_max = health.life_max;
        }
        if self.player(slot).is_some_and(Player::is_playing) {
            let frame = packets::rewrite_owner(id::PLAYER_LIFE_MANA, payload, slot)?;
            self.broadcast(frame, Some(slot));
        }
        Ok(())
    }

    fn on_mana(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mana = PlayerMana::decode(payload)?;
        if let Some(player) = self.player_mut(slot) {
            player.mana = mana.mana;
            player.mana_max = mana.mana_max;
        }
        if self.player(slot).is_some_and(Player::is_playing) {
            let frame = packets::rewrite_owner(id::PLAYER_MANA, payload, slot)?;
            self.broadcast(frame, Some(slot));
        }
        Ok(())
    }

    fn on_team(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        r.u8()?;
        let team = r.u8()?;
        if let Some(player) = self.player_mut(slot) {
            player.team = team;
        }
        self.relay_player_packet(slot, id::TEAM_CHANGE, payload)
    }

    fn on_pvp(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        r.u8()?;
        let hostile = r.bool()?;
        if let Some(player) = self.player_mut(slot) {
            player.pvp = hostile;
        }
        self.relay_player_packet(slot, id::TOGGLE_P_V_P, payload)
    }

    fn on_buffs(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if let Some(player) = self.player_mut(slot) {
            player.buffs = Some(Bytes::copy_from_slice(payload));
        }
        self.relay_player_packet(slot, id::PLAYER_BUFFS, payload)
    }

    fn on_zone(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if let Some(player) = self.player_mut(slot) {
            player.zone = Some(Bytes::copy_from_slice(payload));
        }
        self.relay_player_packet(slot, id::SYNC_PLAYER_ZONE, payload)
    }

    /// A player teleported: a magic mirror, a teleporter, a recall potion.
    ///
    /// The server has to move its own idea of the player as well as relaying, because every
    /// routine that hunts a target reads that position. A teleport the server does not apply
    /// leaves every enemy in the world attacking where the player used to be.
    fn on_teleport(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        let flags = r.u8()?;
        let _claimed = r.i16()?;
        let x = r.f32()?;
        let y = r.f32()?;
        let style = r.u8()?;

        // Bits 0 and 1 together say what is being teleported; only zero — a player — is ours.
        let what = (flags & 1) + ((flags & 2) >> 1) * 2;
        if what != 0 {
            return Ok(());
        }
        // Bit 2 means "where they already are", which is how a client asks for the effect without
        // moving anything.
        let stay = flags & 4 != 0;
        let extra = if flags & 8 != 0 { r.i32()? } else { 0 };

        let at = if stay {
            match self.player(slot) {
                Some(player) => player.position,
                None => return Ok(()),
            }
        } else {
            (x, y)
        };
        if !at.0.is_finite() || !at.1.is_finite() {
            return Ok(());
        }
        if let Some(player) = self.player_mut(slot) {
            player.position = at;
            player.velocity = (0.0, 0.0);
        }

        let mut w = terrustia_proto::PacketWriter::new(id::TELEPORT_ENTITY);
        w.u8(flags);
        w.i16(i16::from(slot));
        w.f32(at.0);
        w.f32(at.1);
        w.u8(style);
        if flags & 8 != 0 {
            w.i32(extra);
        }
        let frame = w.finish()?;
        self.broadcast(frame, Some(slot));
        debug!(slot, x = at.0, y = at.1, "player teleported");
        Ok(())
    }

    /// Packet 73: a client asking the server to move it somewhere.
    ///
    /// Five items work this way, and the reason they are the server's business rather than the
    /// client's is that all five have to *search the world* for somewhere safe to land — which
    /// means seeing tiles the client may not have loaded. None of them was handled, so a
    /// Teleportation Potion, a Magic Conch, a Demon Conch and a Shellphone were all inert.
    ///
    /// A search that finds nowhere leaves the player where they are. That is the game's own
    /// behaviour and the right one: a conch that fails is a wasted item, but a conch that drops
    /// you into a lava lake is a lost character.
    fn on_server_teleport(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use crate::game::teleport::{self, Gates, Wants};
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let kind = r.u8()?;

        let (width, height) = (self.world.width(), self.world.height());
        let gates = Gates {
            downed_plantera: self.world.progress.downed_plantera,
            downed_skeletron: self.world.progress.downed_boss3,
            surface: i32::from(self.world.surface),
            width,
            height,
        };
        let Some(here) = self.player(slot).map(|p| p.position) else {
            return Ok(());
        };

        // The underworld begins here. The game keeps it as a fraction of the world's height.
        let underworld = height - 200;

        let spot = match kind {
            TELEPORT_POTION => {
                // Anywhere at all, above the underworld.
                let tiles = WorldTiles(&self.world);
                teleport::find_spot(
                    &tiles,
                    &mut self.rng,
                    (100, width - 200),
                    (100, underworld - 100),
                    &Wants::default(),
                    &gates,
                )
            }
            MAGIC_CONCH => {
                // The ocean on the far side of the world from wherever you are, which is what
                // makes the conch a way of crossing the map rather than a local shuffle.
                let far_side_is_left = here.0 / crate::game::npc::TILE >= (width / 2) as f32;
                let start = if far_side_is_left {
                    BEACH_MARGIN
                } else {
                    width - BEACH_DISTANCE
                };
                let tiles = WorldTiles(&self.world);
                teleport::find_spot(
                    &tiles,
                    &mut self.rng,
                    (start, BEACH_DISTANCE - BEACH_MARGIN),
                    (100, i32::from(self.world.surface) + 100),
                    &Wants {
                        avoid_any_liquid: true,
                        max_fall: 300,
                        ..Default::default()
                    },
                    &gates,
                )
            }
            DEMON_CONCH => {
                // The underworld, near the middle first and then further out if that fails.
                let middle = width / 2;
                let tiles = WorldTiles(&self.world);
                let wants = Wants {
                    avoid_any_liquid: true,
                    avoid_walls: true,
                    allow_platform_floor: true,
                    ..Default::default()
                };
                teleport::find_spot(
                    &tiles,
                    &mut self.rng,
                    (middle - 50, 100),
                    (underworld + 20, 80),
                    &wants,
                    &gates,
                )
                .or_else(|| {
                    // Failing the middle, anywhere in the underworld at all.
                    teleport::find_spot(
                        &tiles,
                        &mut self.rng,
                        (100, width - 200),
                        (underworld + 20, 80),
                        &wants,
                        &gates,
                    )
                })
            }
            // The Shellphone's spawn setting, and the rescue that fires when a player is crushed
            // with nowhere to stand. Both go to the world's spawn point, which is always valid.
            SHELLPHONE_SPAWN | NO_SPACE_RESCUE => Some((
                f32::from(self.world.spawn_x) * crate::game::npc::TILE - PLAYER_HALF_WIDTH,
                f32::from(self.world.spawn_y) * crate::game::npc::TILE - PLAYER_HEIGHT,
            )),
            _ => return Ok(()),
        };

        let Some(at) = spot else {
            debug!(slot, kind, "no safe landing spot; the player stays put");
            return Ok(());
        };
        if let Some(player) = self.player_mut(slot) {
            player.position = at;
            player.velocity = (0.0, 0.0);
        }

        // Style 2 is the potion's swirl, 11 the phone's. They are only an effect, but a client
        // that is not told which one plays the wrong animation.
        let style = if kind == TELEPORT_POTION { 2u8 } else { 11u8 };
        let mut w = terrustia_proto::PacketWriter::new(id::TELEPORT_ENTITY);
        w.u8(0).i16(i16::from(slot)).f32(at.0).f32(at.1).u8(style);
        let frame = w.finish()?;
        // To everyone including the mover: the client asked to be moved and does not move itself.
        self.broadcast(frame, None);
        debug!(slot, kind, x = at.0, y = at.1, "server-side teleport");
        Ok(())
    }

    /// A player started or stopped talking to a town NPC.
    fn on_talk_npc(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        let _claimed = r.u8()?;
        let npc = r.i16()?;
        if let Some(player) = self.player_mut(slot) {
            player.talking_to = if npc >= 0 { Some(npc as u8) } else { None };
        }
        self.relay_player_packet(slot, id::SYNC_TALK_N_P_C, payload)
    }

    /// A player placed a multi-tile object: a chest, a door, a bed, a workbench.
    ///
    /// This has to happen on the server as well as on the clients. Every other client is told and
    /// places it locally, but if the server does not write the tiles too the object is gone the
    /// moment the world is saved, invisible to anyone who joins later, and does not count toward a
    /// house.
    fn on_place_object(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        let x = i32::from(r.i16()?);
        let y = i32::from(r.i16()?);
        let block = r.i16()?;
        let style = i32::from(r.i16()?);
        let _alternate = r.u8()?;
        let random = i32::from(r.i8()?);
        let _direction = r.bool()?;

        let Ok(block) = u16::try_from(block) else {
            return Ok(());
        };
        let Some(object) = terrustia_proto::tile_object::tile_object(block) else {
            debug!(
                slot,
                block, "ignoring a placement of something that is not an object"
            );
            return Ok(());
        };
        // Ten tiles clear of the world's edge, as the game requires.
        if x < 10 || y < 10 || x >= self.world.width() - 10 || y >= self.world.height() - 10 {
            return Ok(());
        }

        // The packet gives the cursor tile; the object's own origin says where its corner goes.
        let (left, top) = (x - object.origin.0, y - object.origin.1);
        // Nothing is placed over anything already there — the game refuses the whole object
        // rather than filling in the gaps.
        for dx in 0..object.width {
            for dy in 0..object.height {
                if self.world.tile(left + dx, top + dy).is_active() {
                    return Ok(());
                }
            }
        }

        let (frame_x, frame_y) = object.frame_of(style, random);
        for dx in 0..object.width {
            let fx = frame_x + dx * (object.coord_width + object.padding);
            let mut fy = frame_y;
            for dy in 0..object.height {
                // A framed tile, which is what marks it active — setting the block alone leaves an
                // inactive tile that every client draws as empty air.
                let was = self.world.tile(left + dx, top + dy);
                let tile = terrustia_proto::tile::Tile::framed(block, fx as i16, fy as i16)
                    .with_wall(was.wall);
                self.world.set_tile(left + dx, top + dy, tile);
                self.liquids.disturb(left + dx, top + dy);
                fy += object.coord_heights.get(dy as usize).copied().unwrap_or(16) + object.padding;
            }
        }

        // A chest is not only tiles: it needs somewhere to keep what is put in it.
        if block == CHEST_BLOCK {
            let anchor = (left as i16, top as i16);
            if self.world.chest_at(anchor.0, anchor.1).is_none() {
                self.world
                    .add_chest(crate::world::Chest::empty_at(anchor.0, anchor.1));
            }
        }

        // Everyone else places it themselves from the same packet.
        self.broadcast(
            terrustia_proto::packets::place_object(x, y, block, style, random)?,
            Some(slot),
        );
        debug!(slot, block, x, y, "object placed");
        Ok(())
    }

    /// Tell everyone the world itself has changed — an eclipse begun, a blood moon risen.
    fn broadcast_world_data(&mut self) {
        if let Ok(frame) = self.world.world_data().encode() {
            self.broadcast(frame, None);
        }
    }

    /// A player used a summoning item.
    ///
    /// This is the only way a boss enters the world, so it is also the only place a client gets to
    /// name an NPC type. What it may name is the game's own list and nothing else: without that
    /// check a crafted packet could ask for anything in the roster, in any number.
    ///
    /// Negative types are events rather than bosses — a pumpkin moon, an eclipse, an invasion.
    fn on_summon(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        // The player the client claims; ignored in favour of the connection it arrived on.
        let _claimed = r.i16()?;
        let what = r.i16()?;

        if what >= 0 {
            let npc_type = what as u16;
            if !terrustia_proto::npc_params::summonable(npc_type) {
                debug!(
                    slot,
                    npc_type, "refusing to summon something that is not summonable"
                );
                return Ok(());
            }
            // One at a time. A second Eye of Cthulhu is not a thing the game allows.
            if self.npcs.iter().any(|(_, n)| n.npc_type == npc_type) {
                return Ok(());
            }
            self.summon_on_player(slot, npc_type);
            return Ok(());
        }

        match what {
            // A pumpkin or frost moon, which only rise at night.
            -4 | -5 => {
                let moon = if what == -4 {
                    crate::game::moons::Moon::Pumpkin
                } else {
                    crate::game::moons::Moon::Frost
                };
                self.start_moon(moon, slot);
            }
            // A solar eclipse, which only happens by day.
            -6 => {
                if self.world.day_time && !self.world.eclipse {
                    self.world.eclipse = true;
                    self.announce("A solar eclipse is happening!");
                    self.broadcast_world_data();
                }
            }
            -7 => self.start_invasion(Invasion::Martian),
            // A blood moon, which only rises at night and not twice in one night.
            -10 => {
                if !self.world.day_time && !self.world.blood_moon {
                    self.world.blood_moon = true;
                    self.announce("The blood moon is rising...");
                    self.broadcast_world_data();
                }
            }
            // The rest of the negative range is the invasions, numbered from -1.
            other => {
                if let Some(kind) = Invasion::from_id(i32::from(-other)) {
                    self.start_invasion(kind);
                } else {
                    debug!(slot, what = other, "ignoring an unrecognised summon");
                }
            }
        }
        Ok(())
    }

    /// Put a boss somewhere near a player: on the ground, out of arm's reach, or overhead when
    /// there is no ground to be found.
    fn summon_on_player(&mut self, slot: u8, npc_type: u16) {
        use terrustia_proto::npc_params::{
            SUMMON_ABOVE, SUMMON_ATTEMPTS, SUMMON_RANGE_X, SUMMON_RANGE_Y, SUMMON_SAFE_X,
            SUMMON_SAFE_Y,
        };
        // Copied out, because the search below needs the generator and the player at once.
        let Some(at_player) = self.player(slot).map(|p| p.position) else {
            return;
        };
        let (px, py) = (
            (at_player.0 / crate::game::npc::TILE) as i32,
            (at_player.1 / crate::game::npc::TILE) as i32,
        );

        let mut at = None;
        for _ in 0..SUMMON_ATTEMPTS {
            let x = px + rand::Rng::random_range(&mut self.rng, -SUMMON_RANGE_X..=SUMMON_RANGE_X);
            let y = py + rand::Rng::random_range(&mut self.rng, -SUMMON_RANGE_Y..=SUMMON_RANGE_Y);
            // Not right on top of the player.
            if (x - px).abs() < SUMMON_SAFE_X && (y - py).abs() < SUMMON_SAFE_Y {
                continue;
            }
            if self.world.tile(x, y).is_active() {
                continue;
            }
            let Some(ground) = crate::game::spawn::find_ground(&self.world, x, y) else {
                continue;
            };
            at = Some((
                x as f32 * crate::game::npc::TILE,
                (ground - 1) as f32 * crate::game::npc::TILE,
            ));
            break;
        }
        // Nowhere to stand — a Moon Lord, or a player in mid-air. Overhead it is.
        let at = at.unwrap_or((at_player.0, at_player.1 - SUMMON_ABOVE));

        if let Some(index) = self.npcs.spawn(npc_type, at) {
            let name = self
                .npcs
                .get(index)
                .map(|n| n.stats.name)
                .unwrap_or("Something");
            self.announce(&format!("{name} has awoken!"));
            self.broadcast_npc(index);
            info!(slot, npc_type, name, "boss summoned");
        }
    }

    /// One slot of a player's inventory.
    ///
    /// The slot is remembered whatever it is — the server is the authority on what a player is
    /// carrying — but only the public ones are passed on. A player's safe is their own business.
    ///
    /// The owner byte the client sends is not trusted: it is overwritten with the slot the packet
    /// actually arrived on, which is what stops one client rewriting another's inventory.
    fn on_equipment(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut equipment = terrustia_proto::inventory::SyncEquipment::decode(payload)?;
        equipment.player = slot;
        if equipment.slot >= terrustia_proto::inventory::SLOT_COUNT {
            debug!(
                slot,
                requested = equipment.slot,
                "ignoring an out-of-range inventory slot"
            );
            return Ok(());
        }
        if let Some(player) = self.player_mut(slot) {
            player.inventory.insert(equipment.slot, equipment);
        }
        if terrustia_proto::inventory::relayed(equipment.slot) {
            self.broadcast(equipment.encode()?, Some(slot));
        }
        Ok(())
    }

    /// Tell one player what everybody else is carrying.
    ///
    /// Without this a player who joins a running server sees everyone else naked: the equipment
    /// packets went out before they arrived and are never repeated.
    fn send_existing_equipment(&mut self, to: u8) {
        let frames: Vec<Bytes> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.slot != to && p.is_playing())
            .flat_map(|p| p.inventory.values())
            .filter(|e| terrustia_proto::inventory::relayed(e.slot))
            .filter_map(|e| e.encode().ok())
            .map(Bytes::from)
            .collect();
        for frame in frames {
            self.send_bytes(to, frame);
        }
    }

    fn on_uuid(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let uuid = PacketReader::new(payload).string()?;
        if let Some(player) = self.player_mut(slot) {
            player.uuid = Some(uuid);
        }
        Ok(())
    }

    fn on_tile_manipulation(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }

        let edit = TileManipulation::decode(payload)?;
        let (x, y) = (i32::from(edit.x), i32::from(edit.y));
        if !self.world.in_bounds(x, y) {
            return Ok(());
        }

        let mut tile = self.world.tile(x, y);
        let mut changed = true;
        let mut broke = None;

        match edit.kind() {
            TileAction::KillTile | TileAction::KillTileNoItem => {
                // A pickaxe swing that only damages a block also arrives here; only a real break
                // clears the tile.
                if edit.destroyed() {
                    if tile.is_active() && matches!(edit.kind(), TileAction::KillTile) {
                        // The frames go with the block. Everything that decides what a broken
                        // object is worth — which evil an orb belongs to, which chair a chair is
                        // — lives in the frame, and the frame is about to be cleared.
                        broke = Some((tile.block, tile.frame_x, tile.frame_y));
                    }
                    tile.flags.set(TileFlags::ACTIVE, false);
                    tile.block = 0;
                    tile.frame_x = -1;
                    tile.frame_y = -1;
                    tile.slope = 0;
                    tile.flags.set(TileFlags::HALF_BRICK, false);
                } else {
                    changed = false;
                }
            }
            TileAction::PlaceTile => {
                let block = edit.arg.max(0) as u16;
                if frame_important(block) {
                    // Multi-tile objects need placement and framing rules the slice does not
                    // implement. The edit is still relayed so clients agree with each other.
                    debug!(slot, block, "not modelling framed tile placement");
                    changed = false;
                } else {
                    tile.block = block;
                    tile.frame_x = -1;
                    tile.frame_y = -1;
                    tile.flags.set(TileFlags::ACTIVE, true);
                    tile.flags.set(TileFlags::HALF_BRICK, false);
                    tile.slope = 0;
                }
            }
            TileAction::KillWall => {
                if edit.destroyed() {
                    tile.wall = 0;
                    tile.wall_color = 0;
                } else {
                    changed = false;
                }
            }
            TileAction::PlaceWall => tile.wall = edit.arg.max(0) as u16,
            TileAction::PoundTile => {
                // Hammering cycles a block through half-brick and the slopes; the client does the
                // same walk, so mirroring just the half-brick step keeps the common case right.
                tile.slope = 0;
                let half = tile.flags.has(TileFlags::HALF_BRICK);
                tile.flags.set(TileFlags::HALF_BRICK, !half);
            }
            TileAction::SlopeTile => {
                tile.slope = edit.arg.clamp(0, 4) as u8;
                tile.flags.set(TileFlags::HALF_BRICK, false);
            }
            TileAction::PlaceWire => tile.flags.set(TileFlags::WIRE_RED, true),
            TileAction::KillWire => tile.flags.set(TileFlags::WIRE_RED, false),
            TileAction::PlaceWire2 => tile.flags.set(TileFlags::WIRE_BLUE, true),
            TileAction::KillWire2 => tile.flags.set(TileFlags::WIRE_BLUE, false),
            TileAction::PlaceWire3 => tile.flags.set(TileFlags::WIRE_GREEN, true),
            TileAction::KillWire3 => tile.flags.set(TileFlags::WIRE_GREEN, false),
            TileAction::PlaceWire4 => tile.flags.set(TileFlags::WIRE_YELLOW, true),
            TileAction::KillWire4 => tile.flags.set(TileFlags::WIRE_YELLOW, false),
            TileAction::PlaceActuator => tile.flags.set(TileFlags::ACTUATOR, true),
            TileAction::KillActuator => tile.flags.set(TileFlags::ACTUATOR, false),
            TileAction::Other(action) => {
                debug!(slot, action, "unmodelled tile action; relaying only");
                changed = false;
            }
        }

        if changed {
            self.world.set_tile(x, y, tile);
            // Mining a block is the commonest way liquid starts moving.
            self.liquids.disturb(x, y);
        }
        if let Some((block, frame_x, frame_y)) = broke {
            self.spawn_tile_drop(block, frame_x, frame_y, x, y);
            // A demon altar is the only way hardmode ore gets into a world, and it always costs
            // something to break.
            if block == DEMON_ALTAR {
                self.smash_altar(x, y, slot);
            }
            // The handful of tiles that are worth more than the item they leave behind.
            if block == terrustia_proto::orbs::ORB_TILE {
                self.smash_orb(x, y, frame_x);
            }
            // Neither of these has a summon item: breaking the thing *is* the summon.
            if block == crate::world::bulbs::BULB {
                self.wake_from_tile(x, y, PLANTERA);
                // And another grows, so a world cannot be left with no way back to her.
                if !self.world.progress.downed_plantera {
                    self.grow_plantera_bulb();
                }
            }
            if block == BEE_LARVA {
                self.wake_from_tile(x, y, QUEEN_BEE);
            }
        }

        // Relay regardless: even an edit the server does not model must reach other clients, or
        // their view of the world silently diverges from the sender's.
        self.broadcast(edit.encode()?, Some(slot));
        Ok(())
    }

    /// Packet 20: a rectangle of tiles pushed as one unit.
    ///
    /// Clients send this for anything spanning more than a single tile — furniture, trees, a door
    /// swinging open. Applying it is what keeps the server's world in step with multi-tile
    /// operations without reimplementing the game's placement and framing rules.
    fn on_tile_square(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }

        let square = TileSquare::decode(payload)?;
        let (x0, y0) = (i32::from(square.x), i32::from(square.y));

        // A square is at most 255 on a side, so a hostile one cannot cost much; still refuse any
        // that reaches outside the world rather than clamping it into somewhere unintended.
        if !self.world.in_bounds(x0, y0)
            || !self.world.in_bounds(
                x0 + i32::from(square.width) - 1,
                y0 + i32::from(square.height) - 1,
            )
        {
            debug!(
                slot,
                x = square.x,
                y = square.y,
                "tile square out of bounds"
            );
            return Ok(());
        }

        for dx in 0..usize::from(square.width) {
            for dy in 0..usize::from(square.height) {
                if let Some(tile) = square.tile(dx, dy) {
                    let (x, y) = (x0 + dx as i32, y0 + dy as i32);
                    self.world.set_tile(x, y, tile);
                    // Anything a client rewrites might have been holding liquid up.
                    self.liquids.disturb(x, y);
                }
            }
        }

        self.broadcast(square.encode()?, Some(slot));
        Ok(())
    }

    /// Packet 19: a door opening or closing.
    ///
    /// The tile change itself is not modelled — a door swings between a 1x3 closed tile and a 2x3
    /// open one with recomputed frames, which is placement logic this server does not implement.
    /// Relaying keeps every client in agreement with the one that acted; the server's own copy of
    /// those tiles stays as it was until a client pushes a tile square over them.
    fn on_door(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let door = DoorToggle::decode(payload)?;
        if !self.world.in_bounds(i32::from(door.x), i32::from(door.y)) {
            return Ok(());
        }
        self.broadcast(door.encode()?, Some(slot));
        Ok(())
    }

    /// Packet 31: a client asking to open a chest.
    fn on_chest_open(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let request = RequestChestOpen::decode(payload)?;

        let Some((id, chest)) = self.world.chest_at(request.x, request.y) else {
            return Ok(());
        };
        // Vanilla refuses a chest someone else is already inside, so two players cannot both edit
        // the same slots and clobber each other.
        if self
            .players
            .iter()
            .flatten()
            .any(|p| p.slot != slot && p.open_chest == id)
        {
            debug!(slot, chest = id, "chest is already open elsewhere");
            return Ok(());
        }

        let name = chest.name.clone();
        let (x, y, slots) = (chest.x, chest.y, chest.items.len());
        let items: Vec<_> = chest.items.clone();

        self.send(slot, objects::sync_chest_size(id, slots as i16)?);
        for (index, item) in items.iter().enumerate() {
            let frame = SyncChestItem {
                chest: id,
                slot: index as u8,
                item: *item,
            }
            .encode()?;
            self.send(slot, frame);
        }
        self.send(
            slot,
            SyncPlayerChest {
                chest: id,
                x,
                y,
                name: Some(name).filter(|n| !n.is_empty()),
            }
            .encode()?,
        );

        if let Some(player) = self.player_mut(slot) {
            player.open_chest = id;
        }
        Ok(())
    }

    /// Packet 32: a client changing one chest slot.
    fn on_chest_item(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let sync = SyncChestItem::decode(payload)?;

        // Only the player who has the chest open may change it.
        if self.player(slot).map(|p| p.open_chest) != Some(sync.chest) {
            debug!(
                slot,
                chest = sync.chest,
                "rejecting edit to a chest that is not open"
            );
            return Ok(());
        }

        let Some(chest) = self.world.chest_mut(sync.chest) else {
            return Ok(());
        };
        let Some(cell) = chest.items.get_mut(usize::from(sync.slot)) else {
            return Ok(());
        };
        *cell = sync.item;

        self.broadcast(sync.encode()?, Some(slot));
        Ok(())
    }

    /// Packet 33: a client reporting which chest it has open, including closing one.
    fn on_player_chest(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let sync = SyncPlayerChest::decode(payload)?;
        if let Some(player) = self.player_mut(slot) {
            player.open_chest = sync.chest;
        }
        Ok(())
    }

    /// Packet 46: a client asking to read a sign.
    fn on_sign_request(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let request = RequestSign::decode(payload)?;
        let Some((id, sign)) = self.world.sign_at(request.x, request.y) else {
            return Ok(());
        };
        let frame = SignText {
            sign: id,
            x: sign.x,
            y: sign.y,
            text: sign.text.clone(),
            player: slot,
            editing: 0,
        }
        .encode()?;
        self.send(slot, frame);
        Ok(())
    }

    /// Packet 47: a client writing a sign.
    fn on_sign_write(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut update = SignText::decode(payload)?;
        if update.text.len() > MAX_SIGN_TEXT {
            debug!(slot, len = update.text.len(), "sign text too long");
            return Ok(());
        }

        let id = match self.world.sign_at(update.x, update.y) {
            Some((id, _)) => {
                if let Some(sign) = self.world.sign_mut(id) {
                    sign.text = update.text.clone();
                }
                id
            }
            None => {
                let sign = Sign {
                    x: update.x,
                    y: update.y,
                    text: update.text.clone(),
                };
                match self.world.add_sign(sign) {
                    Some(id) => id,
                    None => return Ok(()),
                }
            }
        };

        update.sign = id;
        update.player = slot;
        update.editing = 0;
        self.broadcast(update.encode()?, Some(slot));
        Ok(())
    }

    fn on_net_module(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let Some(chat) = IncomingChat::decode(payload)? else {
            return Ok(());
        };
        if !self.player(slot).is_some_and(Player::is_playing) || !chat.is_say() {
            return Ok(());
        }
        if net_module::validate_chat(&chat.text, self.config.max_chat_len).is_err() {
            debug!(
                slot,
                len = chat.text.len(),
                "dropping out-of-range chat line"
            );
            return Ok(());
        }

        if let Some(command) = chat.text.strip_prefix('/') {
            return self.run_command(slot, command);
        }

        let name = self
            .player(slot)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        info!("<{name}> {}", chat.text);

        let frame = net_module::chat_broadcast(
            slot,
            &NetworkText::literal(format!("<{name}> {}", chat.text)),
            [255, 255, 255],
        )?;
        self.broadcast(frame, None);
        Ok(())
    }

    /// Send a line of server text to one player.
    fn tell(&mut self, slot: u8, text: &str) {
        if let Ok(frame) = net_module::chat_broadcast(
            net_module::SERVER_AUTHOR,
            &NetworkText::literal(text),
            SERVER_CHAT_COLOUR,
        ) {
            self.send(slot, frame);
        }
    }

    /// Handle a chat line beginning with `/`.
    ///
    /// There is no permission model: this is aimed at a server among friends, and every command
    /// here is either read-only or something any player could achieve anyway.
    fn run_command(&mut self, slot: u8, command: &str) -> terrustia_proto::Result<()> {
        let mut parts = command.split_whitespace();
        let name = parts.next().unwrap_or("").to_ascii_lowercase();
        // The whole rest of the line, not the first word of it: `/spawn Eater of Worlds Head` has
        // to reach the resolver intact or it looks up "eater" and finds nothing.
        let argument = parts.collect::<Vec<_>>().join(" ").to_ascii_lowercase();

        match name.as_str() {
            "help" => {
                for line in [
                    "/help            this list",
                    "/players         who is online",
                    "/time <day|noon|night|midnight>",
                    "/save            write the world to disk",
                    "/where           your position and section",
                    "/spawn <npc>     spawn an NPC beside you",
                    "/npcs            what is alive right now",
                    "/butcher         remove every hostile NPC",
                    "/house           is the room you are standing in a valid house?",
                ] {
                    self.tell(slot, line);
                }
            }
            "players" => {
                let names: Vec<String> = self
                    .players
                    .iter()
                    .flatten()
                    .filter(|p| p.is_playing())
                    .map(|p| p.name.clone())
                    .collect();
                let line = format!(
                    "{} of {} online: {}",
                    names.len(),
                    self.config.max_players,
                    if names.is_empty() {
                        "nobody".to_string()
                    } else {
                        names.join(", ")
                    }
                );
                self.tell(slot, &line);
            }
            "time" => {
                let set = match argument.as_str() {
                    "day" => Some((true, 0)),
                    "noon" => Some((true, DAY_LENGTH / 2)),
                    "night" => Some((false, 0)),
                    "midnight" => Some((false, NIGHT_LENGTH / 2)),
                    _ => None,
                };
                match set {
                    Some((day_time, time)) => {
                        self.world.day_time = day_time;
                        self.world.time = time;
                        let frame = packets::TimeSet {
                            day_time,
                            time,
                            sun_mod_y: 0,
                            moon_mod_y: 0,
                        }
                        .encode()?;
                        self.broadcast(frame, None);
                        self.announce(&format!("Time set to {argument}."));
                    }
                    None => self.tell(slot, "usage: /time <day|noon|night|midnight>"),
                }
            }
            "save" => {
                if self.save_path.is_none() {
                    self.tell(
                        slot,
                        "This world has nowhere to be saved: start the server with --save <path>.",
                    );
                } else {
                    self.save_world("command");
                }
            }
            "spawn" => {
                // Accepts an id or an NPCID name, so `/spawn Zombie` and `/spawn 3` both work.
                let Some(npc_type) = resolve_npc(&argument) else {
                    self.tell(slot, "usage: /spawn <npc id or name>, e.g. /spawn Zombie");
                    return Ok(());
                };
                let Some(position) = self.player(slot).map(|p| p.position) else {
                    return Ok(());
                };
                // Drop it a little to the side so it does not appear inside the player.
                let at = (position.0 + 64.0, position.1 - 32.0);
                // Worm heads come with a body: spawning a bare head would be a floating face.
                let spawned = match npc_type {
                    // EaterofWorldsHead, DevourerHead, GiantWormHead, BoneSerpentHead.
                    13 => self.npcs.spawn_worm(13, 14, 15, 20, at),
                    7 => self.npcs.spawn_worm(7, 8, 9, 8, at),
                    10 => self.npcs.spawn_worm(10, 11, 12, 6, at),
                    39 => self.npcs.spawn_worm(39, 40, 41, 12, at),
                    _ => self.npcs.spawn(npc_type, at),
                };
                match spawned {
                    Some(index) => {
                        let name = npc_stats(npc_type).map(|s| s.name).unwrap_or("?");
                        self.broadcast_npc(index);
                        self.tell(slot, &format!("spawned {name} ({npc_type}) as npc {index}"));
                    }
                    None => self.tell(slot, "no free NPC slots"),
                }
            }
            "npcs" => {
                let total = self.npcs.len();
                let mut summary: Vec<String> = Vec::new();
                let mut counts: std::collections::BTreeMap<&str, usize> =
                    std::collections::BTreeMap::new();
                for (_, npc) in self.npcs.iter() {
                    *counts.entry(npc.stats.name).or_default() += 1;
                }
                for (name, n) in counts.iter().take(8) {
                    summary.push(format!("{name} x{n}"));
                }
                let line = format!(
                    "{total} NPCs ({:.1} spawn slots): {}",
                    self.npcs.used_slots(),
                    if summary.is_empty() {
                        "none".to_string()
                    } else {
                        summary.join(", ")
                    }
                );
                self.tell(slot, &line);
                if self.shots_thrown > 0 {
                    self.tell(
                        slot,
                        &format!(
                            "{} in flight, {} thrown since the server started",
                            self.projectiles.len(),
                            self.shots_thrown
                        ),
                    );
                }
            }
            "butcher" => {
                // Clear every hostile NPC, leaving town residents alone.
                let doomed: Vec<u8> = self
                    .npcs
                    .iter()
                    .filter(|(_, npc)| !npc.stats.town_npc)
                    .map(|(index, _)| index)
                    .collect();
                let killed = doomed.len();
                for index in doomed {
                    self.npcs.remove(index);
                    self.broadcast_npc_death(index);
                }
                self.announce(&format!("Butchered {killed} NPCs."));
            }
            "house" => {
                // Report on the room the player is standing in, with the reason if it is no good.
                let Some(position) = self.player(slot).map(|p| p.position) else {
                    return Ok(());
                };
                let (x, y) = ((position.0 / 16.0) as i32, (position.1 / 16.0) as i32);
                let line = match housing::check_room(&self.world, x, y) {
                    Ok(room) => format!(
                        "valid house: {} tiles, {}x{}",
                        room.tiles.len(),
                        room.right - room.left + 1,
                        room.bottom - room.top + 1
                    ),
                    Err(reason) => format!("not a house: {}", reason.describe()),
                };
                self.tell(slot, &line);
            }
            "where" => {
                let line = self.player(slot).map(|p| {
                    let (tx, ty) = ((p.position.0 / 16.0) as i32, (p.position.1 / 16.0) as i32);
                    let (sx, sy) = self.world.section_of(tx, ty);
                    format!("tile ({tx}, {ty}) in section ({sx}, {sy})")
                });
                match line {
                    Some(line) => self.tell(slot, &line),
                    None => self.tell(slot, "unknown position"),
                }
            }
            other => self.tell(slot, &format!("unknown command: /{other}  (try /help)")),
        }
        Ok(())
    }
}

/// Look up an NPC by numeric id or by its `NPCID` name, case-insensitively.
/// What a broken tile gives back.
///
/// Two tables, and the order matters. [`tile_drop`] is the game's own statement of what *mining*
/// a block gives, and it wins wherever it has an answer — mining grass gives dirt even though
/// grass seeds are what placed it. Only where it deliberately has none, which is every framed
/// object, does the style get worked out and the placing item looked up.
fn drop_of(tile: u16, frame_x: i16, frame_y: i16) -> Option<i32> {
    if let Some(item) = tile_drop(tile) {
        return Some(item);
    }
    let object = terrustia_proto::tile_object::tile_object(tile)?;
    // The frame given is of whichever cell the player hit; the style is read off the corner.
    let (dx, dy) = terrustia_proto::tile_object::origin_of(tile, frame_x, frame_y)?;
    let corner_x = frame_x - (dx * (object.coord_width + object.padding)) as i16;
    let corner_y = frame_y
        - object.coord_heights[..dy as usize]
            .iter()
            .map(|h| h + object.padding)
            .sum::<i32>() as i16;
    let style = object.style_of(corner_x, corner_y);
    terrustia_proto::placed_items::placed_item(tile, style)
}

fn resolve_npc(argument: &str) -> Option<u16> {
    if argument.is_empty() {
        return None;
    }
    if let Ok(id) = argument.parse::<u16>() {
        return npc_stats(id).is_some().then_some(id);
    }
    // The names come from `NPCID`, where they are run together: `EaterofWorldsBody`. Nobody types
    // that, so spaces and punctuation are ignored on both sides and the answer is the same
    // whether you write "Eater of Worlds Body", "eaterofworldsbody" or "Eater-of-Worlds-Body".
    let squashed = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let wanted = squashed(argument);
    if wanted.is_empty() {
        return None;
    }
    (0..terrustia_proto::npc_data::NPC_COUNT)
        .find(|id| npc_stats(*id).is_some_and(|s| squashed(s.name) == wanted))
}

/// Compare two byte strings without leaking their contents through timing.
///
/// A game password is hardly a high-value secret, but a length-independent compare costs nothing
/// and avoids having to argue about it.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Borrow helper: `introduce` needs the slot list detached from `self`.
fn other_slots(slots: &[u8]) -> Vec<u8> {
    slots.to_vec()
}

/// Lets the AI read world tiles without borrowing the whole server.
struct WorldTiles<'a>(&'a World);

impl TileView for WorldTiles<'_> {
    fn tile(&self, x: i32, y: i32) -> Tile {
        self.0.tile(x, y)
    }
}

/// How often the biomes are given a chance to creep, and how far from a player they may.
const SPREAD_EVERY: u64 = 10;
const SPREAD_TRIES: usize = 3;
const SPREAD_RANGE: i32 = 120;

/// The demon altar, which is a crafting station before hardmode and an ore mine after it.
const DEMON_ALTAR: u16 = 26;
/// A bee larva in a hive, which is the second way to reach the Queen.
const BEE_LARVA: u16 = 231;
/// The two bosses that have no summon item at all: breaking their tile is the only way in.
const PLANTERA: u16 = 262;
const QUEEN_BEE: u16 = 222;

/// How far a roar carries, and how long what it leaves behind lasts.
const ROAR_REACH: f32 = 800.0;
const ROAR_SLOW_TICKS: i32 = 720;
/// The Slow debuff, which is what a roar leaves.
const BUFF_SLOW: u16 = 32;

/// How far a Dark Mage looks for something worth healing, and for a corpse worth raising.
const HEAL_REACH: (f32, f32) = terrustia_proto::npc_params::DARK_MAGE_HEAL_RANGE;
const RAISE_CHECK_RANGE: f32 = terrustia_proto::npc_params::RAISE_CHECK_RANGE;
const RAISE_MINIMUM: usize = terrustia_proto::npc_params::RAISE_MINIMUM;

/// Coin item ids, smallest first.
const COIN_ITEMS: [i32; 4] = [71, 72, 73, 74];

impl GameServer {
    /// Advance every NPC and tell clients about the ones that changed.
    /// Run every NPC's buffs: count the timers down, work out what they cost, and tell clients.
    ///
    /// Kept apart from [`Self::tick_npcs`] because a debuff can kill, and dying reaches far
    /// outside the NPC table — loot, coins, invasion counts, boss flags — none of which the AI
    /// loop can touch while it holds the table borrowed.
    ///
    /// The whole pass costs nothing when nothing is burning, which is the ordinary case: an NPC
    /// with no buffs is skipped before any of the per-tick work.
    fn tick_npc_buffs(&mut self) {
        // Nothing anywhere is buffed: the common case, and worth one scan to establish.
        if !self.npcs.iter().any(|(_, n)| !n.buffs.is_empty()) {
            return;
        }

        let dryad = self.dryad_bane_rate();
        // Five debuffs are worth however many of a projectile are stuck in the target, so the
        // projectile table is counted once here rather than once per NPC per debuff.
        let stacks = self.stacked_debuff_projectiles();

        let mut changed: Vec<u8> = Vec::new();
        let mut hits: Vec<(u8, i16)> = Vec::new();
        let mut deaths: Vec<(u8, u16, (f32, f32), f32)> = Vec::new();

        for (index, npc) in self.npcs.iter_mut() {
            if npc.buffs.is_empty() {
                if std::mem::take(&mut npc.buffs_dirty) {
                    changed.push(index);
                }
                continue;
            }

            npc.buffs.set_flags(npc.npc_type, npc.ai[1]);
            if npc.buffs.clear_expired() {
                npc.buffs_dirty = true;
            }

            let count = |projectile: u16| {
                stacks
                    .get(&(index, projectile))
                    .copied()
                    .unwrap_or_default()
            };
            let around = crate::game::buffs::Around {
                npc_type: npc.npc_type,
                ai1: npc.ai[1],
                is_segment: npc.follows.is_some() || npc.follows_boss.is_some(),
                get_good: false,
                lava_wet: false,
                daybreaks: count(DAYBREAK_SPEAR),
                javelins: count(JAVELIN),
                tentacles: count(TENTACLE_SPIKE),
                blood_knives: count(BLOOD_BUTCHERER),
                cells: count(STARDUST_CELL),
                dryad_bane_dps: dryad,
            };

            let immortal = is_immortal(npc);
            let toll = npc
                .buffs
                .dots(&around, immortal, npc.stats.dont_take_damage);
            if toll.healed > 0 && npc.life < npc.life_max {
                npc.life = (npc.life + toll.healed).min(npc.life_max);
                npc.dirty = true;
            }
            if toll.hurt > 0 && !immortal {
                // The game reports each crossing separately, so a heavy stack shows as several
                // numbers rather than one large one.
                let per_hit = toll.hurt / toll.hits.max(1);
                for _ in 0..toll.hits {
                    hits.push((index, i16::try_from(per_hit).unwrap_or(i16::MAX)));
                }
                npc.life -= toll.hurt;
                npc.dirty = true;
                // A debuff never lands the killing blow itself: the game drops the NPC to one
                // hit point and then strikes it for everything, which is what makes the death
                // go through the ordinary path rather than leaving a corpse at zero.
                if npc.life <= 0 {
                    npc.life = 0;
                    deaths.push((
                        index,
                        npc.npc_type,
                        npc.center(),
                        if npc.from_statue {
                            0.0
                        } else {
                            npc.stats.value
                        },
                    ));
                }
            }
            if std::mem::take(&mut npc.buffs_dirty) {
                changed.push(index);
            }
        }

        for (index, amount) in hits {
            if let Ok(frame) = packets::npc_debuff_damage(index, amount) {
                self.broadcast(frame, None);
            }
        }
        for index in changed {
            self.broadcast_npc_buffs(index);
        }
        for (index, npc_type, center, value) in deaths {
            self.npc_died(index, npc_type, center, value);
        }
    }

    /// Tell everyone what is currently on an NPC.
    ///
    /// Not optional decoration: a client computes its own armour penetration from the buff list
    /// it believes the target has, so an enemy covered in ichor that nobody was told about takes
    /// fifteen points less from every hit than it should.
    fn broadcast_npc_buffs(&mut self, index: u8) {
        let Some(npc) = self.npcs.get(index) else {
            return;
        };
        let slots: Vec<(u16, i32)> = npc.buffs.active().map(|s| (s.kind, s.time)).collect();
        let at = npc.position;
        if let Ok(frame) = packets::npc_buffs(index, slots) {
            self.broadcast_near(frame, at, index);
        }
    }

    /// How much the Dryad's Bane is worth in this world right now.
    fn dryad_bane_rate(&self) -> i32 {
        let p = &self.world.progress;
        crate::game::buffs::dryad_bane_dps(
            &crate::game::buffs::BossesDowned {
                eye: p.downed_boss1,
                evil: p.downed_boss2,
                skeletron: p.downed_boss3,
                queen_bee: p.downed_queen_bee,
                hard_mode: p.hard_mode,
                queen_slime: p.downed_queen_slime,
                destroyer: p.downed_mech1,
                twins: p.downed_mech2,
                prime: p.downed_mech3,
                plantera: p.downed_plantera,
                golem: p.downed_golem,
                cultist: p.downed_ancient_cultist,
                empress: p.downed_empress_of_light,
                fishron: p.downed_fishron,
                infected_seed: false,
            },
            i32::from(self.world.game_mode),
            false,
        )
    }

    /// How many of each stacking debuff's projectile is lodged in each NPC.
    ///
    /// The game's own test is `ai[0] == 1 && ai[1] == whoAmI` — the first says the projectile has
    /// stuck rather than still flying, the second says what it stuck in.
    fn stacked_debuff_projectiles(&self) -> std::collections::HashMap<(u8, u16), usize> {
        let mut counts = std::collections::HashMap::new();
        for (_, projectile) in self.projectiles.iter() {
            if !matches!(
                projectile.projectile_type,
                DAYBREAK_SPEAR | JAVELIN | TENTACLE_SPIKE | BLOOD_BUTCHERER | STARDUST_CELL
            ) {
                continue;
            }
            if projectile.ai[0] != 1.0 {
                continue;
            }
            let stuck_in = projectile.ai[1];
            if !(0.0..=255.0).contains(&stuck_in) {
                continue;
            }
            *counts
                .entry((stuck_in as u8, projectile.projectile_type))
                .or_insert(0usize) += 1;
        }
        counts
    }

    fn tick_npcs(&mut self) {
        let targets: Vec<Target> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing())
            .map(|p| Target {
                slot: p.slot,
                center: (p.position.0 + 10.0, p.position.1 + 21.0),
                velocity: p.velocity,
                alive: p.life > 0,
            })
            .collect();

        // The AI needs the world and the NPC table at once, so the tile view is built separately
        // rather than borrowing `self` twice.
        // Worm segments trail whatever is in front of them, so leaders are read before anything
        // moves and the whole chain shifts as one.
        let leaders: Vec<(u8, u8, (f32, f32))> = self
            .npcs
            .iter()
            .filter_map(|(index, npc)| npc.follows.map(|ahead| (index, ahead)))
            .filter_map(|(index, ahead)| self.npcs.get(ahead).map(|l| (index, ahead, l.center())))
            .collect();
        for (index, ahead, center) in leaders {
            // A segment whose leader is gone becomes the new head of what remains.
            if self.npcs.get(ahead).is_none() {
                if let Some(npc) = self.npcs.get_mut(index) {
                    npc.follows = None;
                }
                continue;
            }
            if let Some(npc) = self.npcs.get_mut(index) {
                npc_ai::follow_leader(npc, center);
            }
        }

        let mut expired = Vec::new();
        let mut transformed = Vec::new();
        let mut blasts = Vec::new();
        // Life carried home by leeches this tick, delivered once everything has moved.
        let mut healing: Vec<i32> = Vec::new();
        let mut gates: Vec<(i32, i32, bool)> = Vec::new();
        let mut releases: Vec<((f32, f32), bool)> = Vec::new();
        let mut ended: Option<bool> = None;
        let mut close_gates = false;
        let mut raisings: Vec<(f32, f32)> = Vec::new();
        let mut screams = 0usize;
        let mut roars: Vec<(f32, f32)> = Vec::new();
        let mut rituals: Vec<(f32, f32)> = Vec::new();
        let mut auras: Vec<((f32, f32), f32)> = Vec::new();
        // Taken out of the event's own state for the tick so a mage can read it while the table
        // is borrowed, and put back once everything has moved.
        let mut raisable: Vec<(f32, f32)>;
        let mut escaped_probe = false;
        let mut carrying = Vec::new();
        let mut ai_out = npc_ai::AiOutput::default();
        {
            // What the timid critters flee from. Only two styles read it, so the list is only
            // built when one of them is actually about.
            let anything_timid = self
                .npcs
                .iter()
                .any(|(_, n)| matches!(n.stats.ai_style, 26 | 65));
            let hazards: Vec<npc_ai::Hazard> = if anything_timid {
                self.npcs
                    .iter()
                    .filter(|(_, n)| !n.stats.friendly && n.stats.damage > 0)
                    .map(|(_, n)| npc_ai::Hazard {
                        center: n.center(),
                        half: (n.width() / 2.0, n.height() / 2.0),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            // Two styles jostle for space, and they want different lists: a pirate ghost keeps
            // away from other pirate ghosts, a shimmerfly from anything alive at all. Both lists
            // are a scan of the table, so neither is built unless something present reads it.
            let avoid: Vec<(f32, f32)> = {
                use npc_ai::Avoids;
                let wanted: Vec<(u16, Avoids)> = self
                    .npcs
                    .iter()
                    .filter_map(|(_, n)| {
                        npc_ai::avoidance(n.stats.ai_style).map(|a| (n.npc_type, a))
                    })
                    .collect();
                if wanted.is_empty() {
                    Vec::new()
                } else {
                    let own_kind: Vec<u16> = wanted
                        .iter()
                        .filter(|(_, a)| *a == Avoids::OwnKind)
                        .map(|(ty, _)| *ty)
                        .collect();
                    let anything = wanted.iter().any(|(_, a)| *a == Avoids::AnythingAlive);
                    let mut list: Vec<(f32, f32)> = self
                        .npcs
                        .iter()
                        .filter(|(_, n)| {
                            own_kind.contains(&n.npc_type)
                                || (anything && !n.stats.friendly && n.stats.damage > 0)
                        })
                        .map(|(_, n)| n.center())
                        .collect();
                    if anything {
                        list.extend(targets.iter().map(|t| t.center));
                    }
                    list
                }
            };
            // Where Plantera's hooks have bitten, and how many are still on their way somewhere.
            let hook_anchors: Vec<(f32, f32)> = self
                .npcs
                .iter()
                .filter(|(_, n)| n.stats.ai_style == 52)
                .map(|(_, n)| n.center())
                .collect();
            let hooks = if hook_anchors.is_empty() {
                None
            } else {
                let count = hook_anchors.len() as f32;
                Some((
                    hook_anchors.iter().map(|a| a.0).sum::<f32>() / count,
                    hook_anchors.iter().map(|a| a.1).sum::<f32>() / count,
                ))
            };
            let moving_hooks = self
                .npcs
                .iter()
                .filter(|(_, n)| n.stats.ai_style == 52 && n.velocity != (0.0, 0.0))
                .count();

            // Which NPCs are currently riding a player, and whose head each is on.
            let latched: Vec<(u16, u8)> = self
                .npcs
                .iter()
                .filter(|(_, n)| n.stats.ai_style == 85 && n.ai[0] == 5.0)
                .map(|(_, n)| (n.npc_type, n.target as u8))
                .collect();
            let world_size = (self.world.width(), self.world.height());
            // Where the hurt are, and where goblins have fallen: a Dark Mage reads both, and
            // building either inside the loop would mean scanning the table once per mage.
            let hurt: Vec<(f32, f32)> = if self.npcs.iter().any(|(_, n)| n.stats.ai_style == 109) {
                self.npcs
                    .iter()
                    .filter(|(_, n)| n.life != n.life_max)
                    .map(|(_, n)| n.center())
                    .collect()
            } else {
                Vec::new()
            };
            raisable = std::mem::take(&mut self.army.corpses);
            // The event as its own fixtures see it. The arena is surveyed once, when the crystal
            // first asks for its gates, and kept: re-walking it every tick would let a player
            // change where the gates are by building mid-fight.
            let army = crate::game::ai::ArmyView {
                rate: self
                    .army
                    .tier
                    .map_or(0, |tier| tier.lane_spawn_rate(self.army.wave)),
                on_hold: self.army.spawning_on_hold(),
                crystal_alive: self
                    .npcs
                    .iter()
                    .any(|(_, n)| n.npc_type == terrustia_proto::npc_params::DD2_ETERNIA_CRYSTAL),
                arena: self.army_arena,
            };
            // A Moon Lord socket that has been broken open stays in the fight as an empty
            // shell, so counting the parts is not enough to know how far along the fight is.
            let sockets_open = self
                .npcs
                .iter()
                .filter(|(_, n)| matches!(n.stats.ai_style, 78 | 79) && n.ai[0] == -2.0)
                .count();
            // A handful of routines wait on how many of some other type are still alive: the
            // Brain's armour, the Wall's leeches, a pal's escort. One pass counts them all, and
            // only for the types anything actually asks about.
            let census: Vec<(u16, usize)> = crate::game::ai::CENSUS_TYPES
                .iter()
                .map(|&ty| {
                    (
                        ty,
                        self.npcs.iter().filter(|(_, n)| n.npc_type == ty).count(),
                    )
                })
                .filter(|(_, count)| *count > 0)
                .collect();
            // What every NPC that owns parts looks like, read before anything moves so the whole
            // assembly shifts as one. Not only bosses: a Flying Dutchman's cannon and a scutlix's
            // rider hang off ordinary NPCs the same way.
            let parents: std::collections::HashMap<u8, crate::game::ai::boss::skeletron::Parent> =
                self.npcs
                    .iter()
                    .map(|(index, n)| {
                        (
                            index,
                            crate::game::ai::boss::skeletron::Parent {
                                position: n.position,
                                size: (n.width(), n.height()),
                                rotation: n.rotation,
                                scale: n.scale,
                                velocity: n.velocity,
                                direction: n.direction,
                                sprite_direction: n.sprite_direction,
                                time_left: n.time_left,
                                state: n.ai[1],
                                health: n.life as f32 / n.life_max.max(1) as f32,
                            },
                        )
                    })
                    .collect();
            let tiles = WorldTiles(&self.world);
            // The zone scan reads a forty-tile square, so it runs once a tick for the nearest
            // player rather than once per NPC that happens to care.
            let biome = targets
                .first()
                .map_or(crate::game::spawn::Biome::Forest, |t| {
                    crate::game::spawn::biome_at(
                        &self.world,
                        (t.center.0 / crate::game::npc::TILE) as i32,
                        (t.center.1 / crate::game::npc::TILE) as i32,
                    )
                });
            let conditions = crate::game::ai::Conditions {
                blood_moon: self.world.blood_moon,
                day: self.world.day_time,
                eclipse: self.world.eclipse,
                raining: self.world.raining,
                windy: self.weather.windy(),
                crimson: self.world.crimson,
                snow: biome == crate::game::spawn::Biome::Snow,
                jungle: biome == crate::game::spawn::Biome::Jungle,
                wind: self.weather.wind,
                // Worked out once a tick from wherever the nearest player is, rather than per NPC:
                // the zone scan reads a forty-tile square and only the tumbleweed asks.
                desert: biome == crate::game::spawn::Biome::Desert,
                sandstorm: self.weather.sandstorm,
                surface_y: f32::from(self.world.surface) * crate::game::npc::TILE,
                // Game mode 0 is classic; everything above it is expert or harder, and the
                // routines that branch only ask whether it is above classic.
                expert: self.world.game_mode >= 1,
                hardmode: self.world.progress.hard_mode,
                world_size: (self.world.width(), self.world.height()),
            };
            for (index, npc) in self.npcs.iter_mut() {
                // Segments are positioned by their leader, not by a routine of their own.
                if npc.follows.is_some() {
                    continue;
                }
                // A part reads its parent through this; it cannot see the table itself.
                let parent = npc
                    .follows_boss
                    .and_then(|slot| parents.get(&slot).copied());
                let (parent_state, parent_health) =
                    parent.map_or((0.0, 1.0), |p| (p.state, p.health));
                npc_ai::update_with(
                    npc,
                    &tiles,
                    &targets,
                    &mut self.rng,
                    &mut ai_out,
                    npc_ai::Surroundings {
                        conditions,
                        hazards: &hazards,
                        avoid: &avoid,
                        // A headcrab already on this player's head is the only thing that stops
                        // another trying, so it is worked out once per NPC that asks.
                        // Plantera swings from the average of wherever its hooks have bitten.
                        hooks,
                        // A hook holds on while any of its siblings is still travelling.
                        kin_moving: npc.stats.ai_style == 52
                            && moving_hooks > usize::from(npc.velocity != (0.0, 0.0)),
                        target_taken: npc.stats.ai_style == 85
                            && latched.iter().any(|(ty, slot)| {
                                *ty == npc.npc_type
                                    && Some(*slot) == targets.first().map(|t| t.slot)
                            }),
                        census: &census,
                        army,
                        // A fairy hunting for something to show you is the one routine that
                        // wants a survey of the world rather than a look at its neighbours, so
                        // it is done here and only for the two states that ask.
                        treasure: if npc.stats.ai_style == 112 && matches!(npc.ai[2], 2.0 | 6.0) {
                            crate::game::ai::fairy::treasure(
                                &tiles,
                                npc.center(),
                                (world_size.0, world_size.1),
                            )
                        } else {
                            None
                        },
                        // A Dark Mage picks its spell from what is around it: how many of its
                        // side are hurt, and whether there are goblins on the ground to raise.
                        mage: if npc.stats.ai_style == 109 {
                            let here = npc.center();
                            crate::game::ai::army::mage::MageView {
                                wounded: hurt
                                    .iter()
                                    .filter(|(x, y)| {
                                        (x - here.0).abs() <= HEAL_REACH.0
                                            && (y - here.1).abs() <= HEAL_REACH.1
                                    })
                                    .count(),
                                can_raise: raisable
                                    .iter()
                                    .filter(|c| {
                                        (c.0 - here.0).hypot(c.1 - here.1) <= RAISE_CHECK_RANGE
                                    })
                                    .count()
                                    >= RAISE_MINIMUM,
                            }
                        } else {
                            Default::default()
                        },
                        sockets_open,
                        parent,
                        parent_state,
                        parent_health,
                    },
                );
                // A part raised this tick belongs to the NPC that raised it, which only the
                // caller knows the slot of.
                for summon in &mut ai_out.spawn {
                    if summon.parent == Some(npc_ai::Spawn::OWN_PARENT) {
                        summon.parent = Some(index);
                    }
                }
                if let Some(into) = ai_out.transform.take() {
                    transformed.push((index, into, std::mem::take(&mut ai_out.rest_for)));
                }
                // A bomb that went off does its damage through its own hitbox, which the routine
                // has already swollen; what is left is to make sure it is gone afterwards.
                if std::mem::take(&mut ai_out.detonated) {
                    blasts.push(index);
                }
                if std::mem::take(&mut ai_out.called_invasion) {
                    escaped_probe = true;
                }
                if let (Some(at), Some(rider)) = (ai_out.carry.take(), npc.passenger) {
                    carrying.push((rider, at, npc.velocity));
                }
                // Gates the crystal wants raised, enemies a gate wants let out, and the tick the
                // whole thing ends: all decided by a fixture, all carried out by the server.
                gates.extend(std::mem::take(&mut ai_out.gates));
                if let Some(left) = ai_out.release.take() {
                    releases.push((npc.center(), left));
                }
                if let Some(won) = ai_out.army_ended.take() {
                    ended = Some(won);
                }
                if std::mem::take(&mut ai_out.close_gates) {
                    close_gates = true;
                }
                if std::mem::take(&mut ai_out.raising) {
                    raisings.push(npc.center());
                }
                // Betsy's scream also brings wyverns down through the lane portals, which is what
                // makes it a wall of them rather than the one she calls to herself.
                if std::mem::take(&mut ai_out.screamed) {
                    screams += 1;
                }
                // A roar leaves everyone within earshot slowed, which is what makes Deerclops's
                // opening something you have to be somewhere else for.
                if std::mem::take(&mut ai_out.roared) {
                    roars.push(npc.center());
                }
                // A wither beast standing in its aura weakens whoever is standing in it too.
                if let Some(reach) = ai_out.aura.take() {
                    let here = npc.center();
                    auras.push((here, reach));
                }
                // A boss that vanished and wants to come back somewhere else. It is applied here
                // rather than in the routine because the routine cannot see the world's edges.
                if let Some(at) = ai_out.teleport_to.take() {
                    npc.position = (at.0 - npc.width() / 2.0, at.1 - npc.height());
                    npc.velocity = (0.0, 0.0);
                    npc.dirty = true;
                }
                // The tablet finished breaking: the Cultist rises where it stood.
                if std::mem::take(&mut ai_out.ritual_complete) {
                    rituals.push(npc.center());
                }
                // A leech that got home puts its load into whichever part is worst off, which is
                // what makes ignoring them cost you work you have already done.
                if std::mem::take(&mut ai_out.healed) > 0 {
                    healing.push(std::mem::take(&mut ai_out.healed));
                }
                if npc.time_left <= 0 {
                    expired.push(index);
                }
            }
        }

        // Each load goes to the most hurt part still standing.
        for amount in healing {
            let worst = self
                .npcs
                .iter()
                .filter(|(_, n)| matches!(n.stats.ai_style, 77..=79) && n.life < n.life_max)
                .min_by_key(|(_, n)| n.life)
                .map(|(index, _)| index);
            if let Some(index) = worst
                && let Some(npc) = self.npcs.get_mut(index)
            {
                npc.life = (npc.life + amount).min(npc.life_max);
                npc.dirty = true;
            }
        }

        self.army.corpses = std::mem::take(&mut raisable);

        // Skeletons a mage called up out of the ground where goblins fell.
        for spot in raisings {
            let tier = self.army.tier.map_or(0, |t| t as usize);
            let npc_type =
                terrustia_proto::npc_params::DD2_SKELETON_BY_TIER[tier.saturating_sub(1).min(2)];
            for corpse in self.army.take_raisable(spot) {
                let column = (corpse.0 / crate::game::npc::TILE) as i32;
                let from = (corpse.1 / crate::game::npc::TILE) as i32;
                let Some(ground) = spawn::find_ground(&self.world, column, from) else {
                    continue;
                };
                let at = (corpse.0, (ground - 1) as f32 * crate::game::npc::TILE);
                if let Some(index) = self.npcs.spawn(npc_type, at) {
                    self.broadcast_npc(index);
                }
            }
        }

        // A roar is one moment rather than a state, so what it leaves lasts on its own.
        for at in roars {
            let caught: Vec<u8> = self
                .players
                .iter()
                .flatten()
                .filter(|p| p.is_playing() && p.life > 0)
                .filter(|p| {
                    let (x, y) = (p.position.0 + 10.0, p.position.1 + 21.0);
                    (x - at.0).hypot(y - at.1) < ROAR_REACH
                })
                .map(|p| p.slot)
                .collect();
            for slot in caught {
                if let Ok(frame) =
                    terrustia_proto::packets::add_player_buff(slot, BUFF_SLOW, ROAR_SLOW_TICKS)
                {
                    self.broadcast(frame, None);
                }
            }
        }

        // The aura is refreshed every tick it is out, so a short buff is enough: leaving it is
        // what makes it stop, rather than waiting for a timer.
        for (at, reach) in auras {
            let caught: Vec<u8> = self
                .players
                .iter()
                .flatten()
                .filter(|p| p.is_playing() && p.life > 0)
                .filter(|p| {
                    let (x, y) = (p.position.0 + 10.0, p.position.1 + 21.0);
                    (x - at.0).hypot(y - at.1) < reach
                })
                .map(|p| p.slot)
                .collect();
            for slot in caught {
                if let Ok(frame) = terrustia_proto::packets::add_player_buff(
                    slot,
                    terrustia_proto::npc_params::BUFF_WITHERED_ARMOR,
                    3,
                ) {
                    self.broadcast(frame, None);
                }
            }
        }

        for at in rituals {
            if self
                .npcs
                .iter()
                .any(|(_, n)| n.npc_type == terrustia_proto::npc_params::CULTIST)
            {
                continue;
            }
            if let Some(index) = self.npcs.spawn(terrustia_proto::npc_params::CULTIST, at) {
                self.announce("The Lunatic Cultist has awoken!");
                self.broadcast_npc(index);
            }
        }

        for _ in 0..screams {
            let gates: Vec<(f32, f32)> = self
                .npcs
                .iter()
                .filter(|(_, n)| n.npc_type == terrustia_proto::npc_params::DD2_LANE_PORTAL)
                .map(|(_, n)| n.center())
                .collect();
            if gates.is_empty() {
                continue;
            }
            for _ in 0..3 {
                let at = gates[rand::Rng::random_range(&mut self.rng, 0..gates.len())];
                if let Some(index) = self
                    .npcs
                    .spawn(terrustia_proto::npc_params::BETSY_WYVERN, at)
                {
                    self.broadcast_npc(index);
                }
            }
        }

        self.apply_army(gates, releases, ended, close_gates);

        // Doors a fighter finished working at.
        for action in std::mem::take(&mut ai_out.doors) {
            self.apply_door_action(action);
        }

        // Doors a resident opened on its way in, or pulled shut on its way out.
        for action in std::mem::take(&mut ai_out.town_doors) {
            match action {
                crate::game::ai::town::DoorAction::Open { x, y, direction } => {
                    self.apply_door_action(crate::game::ai::fighter::Action::OpenDoor {
                        x,
                        y,
                        direction,
                    });
                }
                crate::game::ai::town::DoorAction::Close { .. }
                | crate::game::ai::town::DoorAction::None => {}
            }
        }

        // Projectiles a routine threw.
        for shot in std::mem::take(&mut ai_out.shots) {
            self.shots_thrown += 1;
            if let Some(index) = self.projectiles.launch(
                shot.projectile,
                shot.position,
                shot.velocity,
                shot.damage,
                i32::from(shot.time_left),
            ) {
                self.broadcast_projectile(index);
            }
        }

        // Minions a boss asked for. Capped so a long fight cannot fill every slot with servants.
        for summon in ai_out.spawn {
            if self.npcs.used_slots() >= MAX_MINION_SLOTS {
                break;
            }
            if let Some(index) = self.npcs.spawn(summon.npc_type, summon.position) {
                if let Some(npc) = self.npcs.get_mut(index) {
                    npc.velocity = summon.velocity;
                    // A boss part is raised with its side in the velocity's sign, and needs to
                    // know which boss it belongs to.
                    if let Some(owner) = summon.parent {
                        npc.follows_boss = Some(owner);
                        npc.ai[0] = summon.velocity.0.signum();
                        npc.velocity = (0.0, 0.0);
                    }
                }
                self.broadcast_npc(index);
            }
        }

        // Whatever is hanging from a balloon goes exactly where the balloon says, at the
        // balloon's own velocity — it is carried, not trailing.
        for (rider, at, velocity) in carrying {
            if let Some(npc) = self.npcs.get_mut(rider) {
                npc.position = (at.0 - npc.width() / 2.0, at.1 - npc.height() / 2.0);
                npc.velocity = velocity;
                npc.dirty = true;
            }
        }

        // A lost girl who has stopped pretending, or a truffle worm that has gone to ground.
        // The slot is kept so clients see one NPC change rather than one vanish and another
        // appear somewhere in the table.
        for (index, into, rest_for) in transformed {
            if let Some(npc) = self.npcs.get_mut(index) {
                npc.become_type(into);
                if rest_for > 0 {
                    npc.ai[1] = rest_for as f32;
                }
            }
            self.broadcast_npc(index);
        }

        // Anything that detonated has already hurt whatever was inside it, through the enlarged
        // hitbox contact damage reads. It only remains to take it off the table.
        for index in blasts {
            self.npcs.remove(index);
            self.broadcast_npc_death(index);
        }

        // A probe that got away with what it saw brings the Martians down on the world.
        if escaped_probe {
            self.start_invasion(Invasion::Martian);
        }

        for index in expired {
            self.npcs.remove(index);
            // A silently vanished NPC would linger on every client, so tell them it is gone.
            self.broadcast_npc_death(index);
        }
        self.resolve_worm_chains();

        // Position updates go out at ten a second rather than sixty: clients interpolate, and a
        // full-rate stream of every NPC would swamp the connection.
        if self.ticks.is_multiple_of(NPC_SYNC_INTERVAL) {
            let dirty: Vec<u8> = self
                .npcs
                .iter()
                .filter(|(_, npc)| npc.dirty)
                .map(|(index, _)| index)
                .collect();
            for index in dirty {
                if let Some(npc) = self.npcs.get_mut(index) {
                    npc.dirty = false;
                }
                self.broadcast_npc(index);
            }
        }
    }

    /// Put the Eater of Worlds back together after something has been cut out of it.
    ///
    /// This is the whole reason the fight is what it is: a severed body segment does not leave a
    /// gap, it becomes two worms. The segment ahead of the wound grows a tail and the one behind it
    /// grows a head, and both keep fighting. A head with nothing behind it, or a tail with nothing
    /// ahead, is a single segment and dies.
    fn resolve_worm_chains(&mut self) {
        use terrustia_proto::npc_params::splitting_worm;

        // Who follows whom, as it stands now.
        let leaders: std::collections::HashMap<u8, u8> = self
            .npcs
            .iter()
            .filter_map(|(index, npc)| npc.follows.map(|leader| (index, leader)))
            .collect();
        let followed: std::collections::HashSet<u8> = leaders.values().copied().collect();

        let mut transformed = Vec::new();
        let mut orphaned = Vec::new();
        for (index, npc) in self.npcs.iter() {
            let Some((head, body, tail)) = splitting_worm(npc.npc_type) else {
                continue;
            };
            let has_leader = npc
                .follows
                .is_some_and(|leader| self.npcs.get(leader).is_some());
            let has_follower = followed.contains(&index);

            if !has_leader && !has_follower {
                // A lone segment is not a worm.
                orphaned.push(index);
            } else if npc.npc_type == body && !has_leader {
                // The wound is ahead of it: it becomes the head of what is left.
                transformed.push((index, head));
            } else if npc.npc_type == body && !has_follower {
                // The wound is behind it: it becomes the tail.
                transformed.push((index, tail));
            } else if (npc.npc_type == head && !has_follower)
                || (npc.npc_type == tail && !has_leader)
            {
                // An end with nothing attached to it is the last of its worm.
                orphaned.push(index);
            }
        }

        for (index, into) in transformed {
            if let Some(npc) = self.npcs.get_mut(index) {
                // The chain link survives the change of type; only what it is changes.
                let follows = npc.follows;
                npc.become_type(into);
                npc.follows = follows;
            }
            self.broadcast_npc(index);
        }
        for index in orphaned {
            self.npcs.remove(index);
            self.broadcast_npc_death(index);
        }
    }

    /// Pass a player's own projectile on to everyone else.
    ///
    /// The server does not simulate player weapons, so a client's arrows are relayed rather than
    /// re-created. Two checks come straight from the game: the projectile has to claim the sender
    /// as its owner, and it must not be a hostile type — otherwise a modified client could conjure
    /// a demon scythe and blame the server for it.
    fn on_client_projectile(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let sync = terrustia_proto::projectile::SyncProjectile::decode(payload)?;
        if sync.key.owner != slot {
            debug!(
                slot,
                owner = sync.key.owner,
                "dropping a mis-owned projectile"
            );
            return Ok(());
        }
        // A client may sync what it fired itself, but never something that would hurt other
        // players: that is the server's decision, not a claim. Vanilla refuses the same thing.
        let hostile =
            terrustia_proto::projectile_data::projectile_stats(sync.projectile_type as u16)
                .is_some_and(|stats| stats.hostile);
        if hostile {
            debug!(
                slot,
                projectile = sync.projectile_type,
                "dropping a hostile projectile from a client"
            );
            return Ok(());
        }
        let frame = sync.encode()?;
        self.broadcast(frame, Some(slot));
        Ok(())
    }

    /// ...and pass on the news that it is gone.
    fn on_client_projectile_kill(
        &mut self,
        slot: u8,
        payload: &[u8],
    ) -> terrustia_proto::Result<()> {
        let kill = terrustia_proto::projectile::KillProjectile::decode(payload)?;
        if kill.key.owner != slot {
            return Ok(());
        }
        let frame = kill.encode()?;
        self.broadcast(frame, Some(slot));
        Ok(())
    }

    /// Packet 53: a client reporting that it has inflicted something on an NPC.
    ///
    /// This is how virtually every weapon debuff in the game arrives. The client works out what
    /// its weapon inflicts — it knows its own accessories, its own flasks, its own imbues — and
    /// the server decides only whether the target is immune. Trusting the client for *what* it
    /// inflicts is the game's own arrangement, not a shortcut: the alternative would be
    /// reimplementing every weapon's on-hit rules on the server, and the client would still
    /// disagree.
    ///
    /// What the server does *not* trust is the outcome. Immunity is checked here, so no client
    /// can poison King Slime or set the Wall of Flesh alight by asserting it has.
    fn on_add_npc_buff(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let request = terrustia_proto::packets::AddNpcBuff::decode(payload)?;
        // A negative duration would be an eternal buff once it is written into the slot, since
        // nothing counts it up. The game reads it as a short and lets `AddBuff` compare it, which
        // means a negative one is refused for already being shorter than what is there.
        if request.ticks <= 0 {
            return Ok(());
        }
        let Some(npc) = self.npcs.get_mut(request.index) else {
            return Ok(());
        };
        if npc
            .buffs
            .add(npc.npc_type, request.buff, i32::from(request.ticks))
        {
            npc.buffs_dirty = true;
            // Sent now rather than on the next tick: the client that landed the hit is about to
            // work out its next one's armour penetration, and a tick of lag there is a hit at
            // the wrong damage.
            self.broadcast_npc_buffs(request.index);
        }
        Ok(())
    }

    /// Packet 137: a client asking that a buff be taken off an NPC.
    ///
    /// Every one of these is refused, and that is the correct behaviour rather than a gap. The
    /// game validates the request against `BuffID.Sets.CanBeRemovedByNetMessage`, which in this
    /// version is empty — so the message exists, is read, and never does anything. Reading it is
    /// still necessary: several packets arrive in one batch, and skipping this one's bytes would
    /// misparse whatever follows it.
    fn on_remove_npc_buff(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let request = terrustia_proto::packets::RemoveNpcBuff::decode(payload)?;
        let Some(npc) = self.npcs.get_mut(request.index) else {
            return Ok(());
        };
        if npc.buffs.remove_by_request(npc.npc_type, request.buff) {
            npc.buffs_dirty = true;
            self.broadcast_npc_buffs(request.index);
        }
        Ok(())
    }

    /// Packet 56: a client asking what a town NPC is called.
    ///
    /// The client sends this with only the slot filled in the moment the NPC comes into view, and
    /// shows the type's name until it is answered. Left unhandled — as it was — every guide in
    /// the world is "Guide", nobody has a name, and the Tax Collector never becomes Andrew.
    ///
    /// The name is rolled here rather than when the NPC spawns. Nothing can tell the difference:
    /// the roll is kept once made, so an NPC's name never changes, and until somebody asks there
    /// is nobody to notice it did not have one.
    fn on_town_npc_name_request(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = terrustia_proto::PacketReader::new(payload);
        let index = r.i16()?;
        let Ok(index) = u8::try_from(index) else {
            return Ok(());
        };
        let Some(npc) = self.npcs.get(index) else {
            return Ok(());
        };
        let npc_type = npc.npc_type;
        if npc.given_name.is_empty() && terrustia_proto::town_names::has_given_name(npc_type) {
            let variation = self.roll_town_variation(npc_type);
            let name = self.roll_town_name(npc_type, variation);
            if let Some(npc) = self.npcs.get_mut(index) {
                npc.town_variation = variation;
                npc.given_name = name;
            }
        }
        let Some(npc) = self.npcs.get(index) else {
            return Ok(());
        };
        let frame = packets::town_npc_name(index, &npc.given_name, npc.town_variation)?;
        // Answered to the asker alone, as the game does: another client that has not seen this
        // NPC yet has no use for its name and will ask when it does.
        self.send(slot, frame);
        Ok(())
    }

    /// Which of a type's looks a newly-arrived one wears.
    ///
    /// One for almost everything; a cat, a dog or a bunny rolls one of six breeds, which decides
    /// both its sprite and which names it can be given.
    fn roll_town_variation(&mut self, npc_type: u16) -> i32 {
        let count = terrustia_proto::town_names::variation_count(npc_type);
        if count <= 1 {
            return 0;
        }
        rand::Rng::random_range(&mut self.rng, 0..count) as i32
    }

    /// Pick a name, preferring one nobody in the world is already using.
    ///
    /// The game does not guarantee uniqueness either, but it does re-roll the duplicates when a
    /// second of a type arrives, and a town with two Andrews reads as a bug. Falling back to a
    /// plain roll when every name is taken is what keeps a world with sixty cats working.
    fn roll_town_name(&mut self, npc_type: u16, variation: i32) -> String {
        let names = terrustia_proto::town_names::names_for_variation(
            npc_type,
            usize::try_from(variation).unwrap_or(0),
        );
        if names.is_empty() {
            return String::new();
        }
        let taken: Vec<&str> = self
            .npcs
            .iter()
            .filter(|(_, n)| n.npc_type == npc_type && !n.given_name.is_empty())
            .map(|(_, n)| n.given_name.as_str())
            .collect();
        let free: Vec<&&str> = names.iter().filter(|n| !taken.contains(n)).collect();
        if free.is_empty() {
            let at = rand::Rng::random_range(&mut self.rng, 0..names.len());
            return names[at].to_string();
        }
        let at = rand::Rng::random_range(&mut self.rng, 0..free.len());
        (*free[at]).to_string()
    }

    /// Move every projectile, and remove the ones that are finished.
    fn tick_projectiles(&mut self) {
        let mut spent = Vec::new();
        let mut emitted = Vec::new();
        {
            let tiles = WorldTiles(&self.world);
            for (index, projectile) in self.projectiles.iter_mut() {
                if crate::game::projectile::step(projectile, &tiles, &mut emitted)
                    == crate::game::projectile::Outcome::Spent
                {
                    spent.push(index);
                }
            }
        }
        for index in spent {
            self.kill_projectile(index);
        }
        // A flamethrower's flames, and anything else a projectile decided to put in the air.
        for emit in emitted {
            if let Some(index) = self.projectiles.launch(
                emit.projectile_type,
                emit.position,
                emit.velocity,
                emit.damage,
                0,
            ) {
                self.broadcast_projectile(index);
            }
        }

        // Clients interpolate between updates, so projectiles go out at the same rate NPCs do.
        if self.ticks.is_multiple_of(NPC_SYNC_INTERVAL) {
            let dirty: Vec<u16> = self
                .projectiles
                .iter()
                .filter(|(_, p)| p.dirty)
                .map(|(index, _)| index)
                .collect();
            for index in dirty {
                if let Some(p) = self.projectiles.get_mut(index) {
                    p.dirty = false;
                }
                self.broadcast_projectile(index);
            }
        }
    }

    /// Hurt anyone standing in an enemy or in something one of them threw.
    ///
    /// The invulnerability window is what makes this survivable: without it a player inside a
    /// zombie would take sixty hits a second rather than one every half second.
    fn tick_contact_damage(&mut self) {
        const IMMUNE_TICKS: i32 = 30;

        for slot in 0..self.players.len() {
            let Some(player) = self.players[slot].as_ref() else {
                continue;
            };
            if !player.is_playing() || player.life <= 0 {
                continue;
            }
            if player.immune_ticks > 0 {
                if let Some(player) = self.players[slot].as_mut() {
                    player.immune_ticks -= 1;
                }
                continue;
            }
            let box_at = player.position;
            let box_size = (
                crate::game::ai::PLAYER_WIDTH as f32,
                crate::game::ai::PLAYER_HEIGHT as f32,
            );

            // An enemy you are standing in.
            let hit = self.npcs.iter().find(|(_, npc)| {
                !npc.stats.friendly
                    && npc.stats.damage > 0
                    && npc.is_alive()
                    && npc.position.0 < box_at.0 + box_size.0
                    && npc.position.0 + npc.width() > box_at.0
                    && npc.position.1 < box_at.1 + box_size.1
                    && npc.position.1 + npc.height() > box_at.1
            });
            if let Some((index, npc)) = hit {
                let damage = npc.stats.damage;
                let direction = if npc.center().0 < box_at.0 { -1 } else { 1 };
                let npc_type = npc.npc_type;
                self.hurt_player(
                    slot,
                    damage,
                    direction,
                    terrustia_proto::hurt::DeathReason::from_npc(i16::from(index)),
                    IMMUNE_TICKS,
                );
                // Over half the roster leaves something behind as well as the damage, and for
                // several of them that is the actual difficulty of the biome they live in.
                self.apply_touch_debuffs(slot as u8, npc_type);
                continue;
            }

            // Or something one of them threw.
            let struck = self
                .projectiles
                .iter()
                .find(|(_, p)| p.damage > 0 && p.overlaps(box_at, box_size))
                .map(|(index, p)| (index, p.damage, p.center().0, p.projectile_type));
            if let Some((index, damage, from_x, projectile_type)) = struck {
                let direction = if from_x < box_at.0 { -1 } else { 1 };
                self.hurt_player(
                    slot,
                    damage,
                    direction,
                    terrustia_proto::hurt::DeathReason::from_projectile(
                        index as i16,
                        projectile_type as i16,
                    ),
                    IMMUNE_TICKS,
                );
                // A projectile with a hit budget spends one, and dies when it runs out.
                let spent = self.projectiles.get_mut(index).is_some_and(|p| {
                    if p.penetrate > 0 {
                        p.penetrate -= 1;
                    }
                    p.penetrate == 0
                });
                if spent {
                    self.kill_projectile(index);
                }
            }
        }

        self.tick_town_casualties();
    }

    /// Enemies hurt the townsfolk too.
    ///
    /// A blood moon or an invasion that walks through a town and leaves it standing is not a
    /// threat, it is scenery. This is also the only thing that makes a townsperson's armour mean
    /// anything, and their armour is most of what the world's history does for them: the guide who
    /// died to a zombie on the first night can hold a doorway by hardmode.
    fn tick_town_casualties(&mut self) {
        /// How long a townsperson is safe for after being hit.
        ///
        /// Counted on the world's clock rather than per NPC: they are all hit on the same tick,
        /// which costs one field fewer on every NPC and is indistinguishable in play.
        const TOWN_IMMUNE_TICKS: u64 = 30;
        if !self.ticks.is_multiple_of(TOWN_IMMUNE_TICKS) {
            return;
        }

        let residents: Vec<u8> = self
            .npcs
            .iter()
            .filter(|(_, n)| n.stats.town_npc && n.is_alive())
            .map(|(index, _)| index)
            .collect();
        if residents.is_empty() {
            return;
        }
        let toughness = self.town_toughness();

        for index in residents {
            // Their armour is the type's plus everything the world has beaten.
            let Some(resident) = self.npcs.get_mut(index) else {
                continue;
            };
            resident.defense = resident.stats.defense + toughness.defense;
            let (at, size) = (resident.position, (resident.width(), resident.height()));

            let attacker = self.npcs.iter().find(|(other, n)| {
                *other != index
                    && !n.stats.friendly
                    && n.stats.damage > 0
                    && n.is_alive()
                    && n.position.0 < at.0 + size.0
                    && n.position.0 + n.width() > at.0
                    && n.position.1 < at.1 + size.1
                    && n.position.1 + n.height() > at.1
            });
            let Some((_, enemy)) = attacker else {
                continue;
            };
            let (damage, from_x) = (enemy.stats.damage, enemy.center().0);
            let Some(resident) = self.npcs.get_mut(index) else {
                continue;
            };
            let taken = damage_taken(damage, resident.defense, false);
            let direction = if from_x < at.0 { -1 } else { 1 };
            let (killed, name) = (
                resident.take_damage(taken, 0.0, direction),
                resident.stats.name,
            );
            if killed {
                self.npcs.remove(index);
                self.broadcast_npc_death(index);
                self.announce(&format!("{name} was slain..."));
                info!(name, "a townsperson was killed");
            } else {
                self.broadcast_npc(index);
            }
        }
    }

    /// How tough this world's townsfolk are, from everything it has beaten.
    fn town_toughness(&self) -> terrustia_proto::npc_params::TownToughness {
        let p = &self.world.progress;
        terrustia_proto::npc_params::town_toughness(
            &[
                p.downed_king_slime,
                p.downed_boss1,
                p.downed_deerclops,
                p.downed_boss2,
                p.downed_boss3,
                p.downed_queen_bee,
                p.hard_mode,
                p.downed_queen_slime,
                p.downed_mech1,
                p.downed_mech2,
                p.downed_mech3,
                p.downed_plantera,
                p.downed_empress_of_light,
                p.downed_fishron,
                p.downed_golem,
            ],
            (p.combat_book, p.combat_book_two),
        )
    }

    /// Take health off a player, tell everyone, and announce a death if it was fatal.
    fn hurt_player(
        &mut self,
        slot: usize,
        damage: i32,
        direction: i8,
        reason: terrustia_proto::hurt::DeathReason,
        immune_ticks: i32,
    ) {
        let Some(player) = self.players[slot].as_mut() else {
            return;
        };
        let taken = damage.max(1) as i16;
        player.life -= taken;
        player.immune_ticks = immune_ticks;
        let died = player.life <= 0;
        if died {
            player.life = 0;
        }
        let index = player.slot;

        if died {
            if let Ok(frame) = (terrustia_proto::hurt::PlayerDeath {
                player: index,
                reason,
                damage: taken,
                direction,
                pvp: false,
            })
            .encode()
            {
                self.broadcast(frame, None);
            }
        } else if let Ok(frame) = (terrustia_proto::hurt::PlayerHurt {
            player: index,
            reason,
            damage: taken,
            direction,
            crit: false,
            pvp: false,
            cooldown: -1,
        })
        .encode()
        {
            self.broadcast(frame, None);
        }
    }

    /// Tell everyone a projectile is gone, and free its slot.
    fn kill_projectile(&mut self, index: u16) {
        let Some(projectile) = self.projectiles.remove(index) else {
            return;
        };
        if let Ok(frame) = (terrustia_proto::projectile::KillProjectile {
            key: projectile.key,
            position: projectile.position,
        })
        .encode()
        {
            self.broadcast(frame, None);
        }
    }

    /// Tell everyone where a projectile is.
    fn broadcast_projectile(&mut self, index: u16) {
        let Some(p) = self.projectiles.get(index) else {
            return;
        };
        let sync = terrustia_proto::projectile::SyncProjectile {
            key: p.key,
            position: p.position,
            velocity: p.velocity,
            projectile_type: p.projectile_type as i16,
            ai: p.ai,
            banner: 0,
            damage: p.damage as i16,
            knockback: p.knockback,
            original_damage: p.damage as i16,
        };
        let at = sync.position;
        if let Ok(frame) = sync.encode() {
            // Same rule as an NPC's, which is the game's: a projectile is only news to the
            // players whose part of the world it is flying through. Unlike an NPC it has no skip
            // cap, because one that has left never needs catching up on — it is gone.
            self.broadcast_to_nearby(frame, at);
        }
    }

    /// Send a frame only to the players near a point.
    fn broadcast_to_nearby(&mut self, frame: Vec<u8>, at: (f32, f32)) {
        let bytes = Bytes::from(frame);
        for slot in self.players_near(at) {
            self.send_bytes(slot, bytes.clone());
        }
    }

    /// The players whose loaded part of the world covers a point.
    fn players_near(&self, at: (f32, f32)) -> Vec<u8> {
        let section = section_of(at);
        self.players
            .iter()
            .flatten()
            .filter(|p| p.is_playing() && near_section(p.position, section))
            .map(|p| p.slot)
            .collect()
    }

    /// Drop cached sections whose tiles have changed.
    ///
    /// This has to run before a section is served, not merely once a tick: an edit and a join can
    /// land in the same batch of events, and a section sent in between would carry stale tiles.
    fn flush_dirty_sections(&mut self) {
        for section in self.world.take_dirty_sections() {
            self.section_cache.remove(&section);
        }
    }

    fn npc_sync(&self, index: u8) -> Option<SyncNpc> {
        let npc = self.npcs.get(index)?;
        Some(SyncNpc {
            index,
            generation: npc.generation,
            position: npc.position,
            velocity: npc.velocity,
            target: npc.target,
            direction: npc.direction,
            direction_y: npc.direction_y,
            sprite_direction: npc.sprite_direction,
            ai: npc.ai,
            net_id: npc.npc_type as i16,
            life: npc.life,
            life_max: npc.life_max,
            release_owner: 255,
        })
    }

    fn broadcast_npc(&mut self, index: u8) {
        let Some(sync) = self.npc_sync(index) else {
            return;
        };
        let Ok(frame) = sync.encode() else {
            return;
        };
        let at = sync.position;
        self.broadcast_near(frame, at, index);
    }

    /// Send an NPC's state only to the players whose part of the world it is in.
    ///
    /// A broadcast to everybody is what a server can least afford: with two hundred NPCs awake and
    /// a sync every six ticks, sending each to every player is thousands of frames a second per
    /// client, and a client that cannot drain that fast is dropped for being slow. The game's own
    /// rule is to skip an NPC for a client whose loaded sections do not cover it — but never more
    /// than four times in a row, so something far away still gets an occasional update rather than
    /// freezing where it was last seen.
    fn broadcast_near(&mut self, frame: Vec<u8>, at: (f32, f32), index: u8) {
        let bytes = Bytes::from(frame);
        let section = section_of(at);
        let targets: Vec<(u8, bool)> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing())
            .map(|p| (p.slot, near_section(p.position, section)))
            .collect();
        for (slot, near) in targets {
            if !near {
                let skipped = self.npc_skips.entry((index, slot)).or_insert(0);
                if *skipped < MAX_NPC_SYNC_SKIPS {
                    *skipped += 1;
                    continue;
                }
            }
            self.npc_skips.remove(&(index, slot));
            self.send_bytes(slot, bytes.clone());
        }
    }

    /// Carry out what a fighter decided to do to a door.
    fn apply_door_action(&mut self, action: crate::game::ai::fighter::Action) {
        use crate::game::ai::fighter::Action;
        match action {
            Action::None => {}
            Action::OpenDoor { x, y, direction } => {
                // Swinging a door open moves the tile and reframes a 2x3 block, which is placement
                // logic this server does not implement. Broadcasting the toggle makes every client
                // open it, which is how vanilla propagates the change anyway.
                let toggle = terrustia_proto::objects::DoorToggle {
                    action: 0,
                    x: x as i16,
                    y: y as i16,
                    direction: if direction > 0 { 1 } else { 0 },
                };
                if let Ok(frame) = toggle.encode() {
                    self.broadcast(frame, None);
                }
            }
            Action::BreakDoor { x, y } => {
                // A broken door really is gone, so the tiles are cleared here and the change is
                // sent as an ordinary tile edit.
                for dy in -1..=1 {
                    let mut tile = self.world.tile(x, y + dy);
                    if !tile.is_active() {
                        continue;
                    }
                    tile.flags.set(TileFlags::ACTIVE, false);
                    tile.block = 0;
                    tile.frame_x = -1;
                    tile.frame_y = -1;
                    self.world.set_tile(x, y + dy, tile);
                    self.liquids.disturb(x, y + dy);

                    let edit = TileManipulation {
                        action: 0,
                        x: x as i16,
                        y: (y + dy) as i16,
                        arg: 0,
                        style: 0,
                    };
                    if let Ok(frame) = edit.encode() {
                        self.broadcast(frame, None);
                    }
                }
                info!(x, y, "a door was broken down");
            }
        }
    }

    /// Tell clients an NPC is gone. Zero health in packet 23 is what removes it.
    fn broadcast_npc_death(&mut self, index: u8) {
        let sync = SyncNpc {
            index,
            generation: 0,
            position: (0.0, 0.0),
            velocity: (0.0, 0.0),
            target: 255,
            direction: 1,
            direction_y: 1,
            sprite_direction: 1,
            ai: [0.0; 4],
            net_id: 0,
            life: 0,
            life_max: 1,
            release_owner: 255,
        };
        if let Ok(frame) = sync.encode() {
            self.broadcast(frame, None);
        }
    }

    /// Packet 28: a client reporting a hit on an NPC.
    fn on_damage_npc(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let hit = DamageNpc::decode(payload)?;

        // Acknowledge first, as vanilla does, so the client stops resending the hit.
        self.send(slot, damage_ack()?);

        let Some(npc) = self.npcs.get_mut(hit.index) else {
            return Ok(());
        };
        // A stale hit aimed at whoever used to hold this slot must not land on its new occupant.
        if npc.generation != hit.generation {
            debug!(
                slot,
                index = hit.index,
                "dropping a hit with a stale generation"
            );
            return Ok(());
        }

        // Live armour, not the type's: a rolling tortoise really is twice as hard to hurt.
        let amount = damage_taken(i32::from(hit.damage), npc.defense, hit.crit);
        let mut killed = npc.take_damage(amount, hit.knockback, hit.direction);
        // A statue's monster is worth nothing: the game zeroes its value on the way out of the
        // statue, which is what stops a wired statue being a coin printer.
        let value = if npc.from_statue {
            0.0
        } else {
            npc.stats.value
        };
        let (npc_type, center) = (npc.npc_type, npc.center());

        // The Eternia Crystal does not die when it runs out of life — it goes into its losing
        // drama, which is what actually ends the event ten seconds later.
        if killed && npc_type == terrustia_proto::npc_params::DD2_ETERNIA_CRYSTAL {
            killed = false;
            npc.ai[1] = 1.0;
            npc.ai[0] = 0.0;
            npc.life = npc.life_max;
            npc.dirty = true;
        }

        self.broadcast(hit.encode()?, Some(slot));

        if killed {
            self.npc_died(hit.index, npc_type, center, value);
            debug!(slot, npc_type, "npc killed");
        } else {
            self.broadcast_npc(hit.index);
        }
        Ok(())
    }

    /// Everything that follows an NPC running out of life.
    ///
    /// Shared rather than inlined at the one hit path, because a hit is not the only way to die:
    /// a debuff can finish something off between two ticks, with no player credited, and every
    /// one of these still has to happen.
    fn npc_died(&mut self, index: u8, npc_type: u16, center: (f32, f32), value: f32) {
        self.npcs.remove(index);
        self.broadcast_npc_death(index);
        self.drop_coins(value, center);
        self.drop_loot(npc_type, center);
        self.note_invasion_kill(npc_type);
        self.army.note_corpse(npc_type, (center.0, center.1 + 16.0));
        self.note_army_kill(npc_type);
        self.note_moon_kill(npc_type);
        self.lunar.note_kill(npc_type);
        self.note_boss_kill(npc_type);
    }

    /// Drop whatever an NPC was carrying.
    ///
    /// Each chain is rolled in order and stops at the first success, which is what keeps a
    /// skeleton's four weapons rare rather than giving it four separate chances at one.
    ///
    /// On top of that come the drops that depend on the world rather than the thing that died: a
    /// treasure bag in expert, a trophy, and the hardmode materials that only exist once the wall
    /// has fallen.
    fn drop_loot(&mut self, npc_type: u16, center: (f32, f32)) {
        let (tx, ty) = (
            (center.0 / crate::game::npc::TILE) as i32,
            (center.1 / crate::game::npc::TILE) as i32,
        );
        let ground = self.world.tile(tx, ty).block;
        let p = &self.world.progress;
        let at = terrustia_proto::conditional_drops::Conditions {
            expert: self.world.game_mode >= 1,
            master: self.world.game_mode >= 2,
            world_is_crimson: self.world.crimson,
            hard_mode: p.hard_mode,
            downed_plantera: p.downed_plantera,
            in_hallow: matches!(
                terrustia_proto::convert::biome_of(ground),
                Some(terrustia_proto::convert::Biome::Hallow)
            ),
            in_corruption: matches!(
                terrustia_proto::convert::biome_of(ground),
                Some(terrustia_proto::convert::Biome::Corruption)
            ),
            in_crimson: matches!(
                terrustia_proto::convert::biome_of(ground),
                Some(terrustia_proto::convert::Biome::Crimson)
            ),
            underground: ty > i32::from(self.world.rock_layer),
        };
        for rule in terrustia_proto::conditional_drops::conditional(npc_type, at) {
            if rule.one_in > 1 && !rand::Rng::random_ratio(&mut self.rng, 1, rule.one_in) {
                continue;
            }
            let stack = if rule.max > rule.min {
                rand::Rng::random_range(&mut self.rng, rule.min..=rule.max)
            } else {
                rule.min
            };
            if let Some(index) = self
                .items
                .spawn(ItemStack::new(i32::from(rule.item), stack, 0), center)
            {
                self.broadcast_item(index);
            }
        }
        self.drop_flat_loot(npc_type, center);
    }

    /// The unconditional table.
    fn drop_flat_loot(&mut self, npc_type: u16, center: (f32, f32)) {
        for chain in terrustia_proto::npc_drops::drops(npc_type) {
            for rule in *chain {
                if !rand::Rng::random_ratio(&mut self.rng, 1, rule.one_in) {
                    continue;
                }
                let stack = if rule.max > rule.min {
                    rand::Rng::random_range(&mut self.rng, rule.min..=rule.max)
                } else {
                    rule.min
                };
                if let Some(index) = self
                    .items
                    .spawn(ItemStack::new(i32::from(rule.item), stack, 0), center)
                {
                    self.broadcast_item(index);
                }
                break;
            }
        }
    }

    /// Scatter an NPC's coin value as item entities.
    ///
    /// This is only the coin half of a death: it is universal and comes straight from the NPC's
    /// `value`. What the thing was actually carrying is [`Self::drop_loot`].
    fn drop_coins(&mut self, value: f32, center: (f32, f32)) {
        let mut copper = value as i64;
        if copper <= 0 {
            return;
        }
        // Split into platinum, gold, silver and copper, largest denomination first.
        for (tier, item) in COIN_ITEMS.iter().enumerate().rev() {
            let unit = 100i64.pow(tier as u32);
            let count = copper / unit;
            if count == 0 {
                continue;
            }
            copper -= count * unit;
            let stack = count.min(i64::from(i16::MAX)) as i16;
            if let Some(index) = self
                .items
                .spawn(ItemStack::new(*item, stack, 0), (center.0, center.1))
            {
                self.broadcast_item(index);
            }
        }
    }

    /// Stream every live NPC to a client that has just finished loading the world.
    fn send_npcs(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        let syncs: Vec<SyncNpc> = self
            .npcs
            .iter()
            .filter_map(|(index, _)| self.npc_sync(index))
            .collect();
        for sync in syncs {
            self.send(slot, sync.encode()?);
        }
        Ok(())
    }

    /// Look for a free house near the players and move a town NPC into it.
    ///
    /// Vanilla gates each resident behind conditions that mostly read the players' inventories,
    /// which this server does not model. The Guide is the exception — it arrives as soon as there
    /// is somewhere to live — so that is the one moved in automatically; the rest are placed with
    /// `/spawn` and will take a house of their own.
    fn tick_town_npcs(&mut self) {
        if !self.ticks.is_multiple_of(HOUSING_SCAN_INTERVAL) {
            return;
        }

        // House any resident that does not have one yet.
        let homeless: Vec<u8> = self
            .npcs
            .iter()
            .filter(|(_, npc)| npc.stats.town_npc && npc.home.is_none())
            .map(|(index, _)| index)
            .collect();

        let guide_present = self.npcs.iter().any(|(_, n)| n.npc_type == GUIDE);
        if homeless.is_empty() && guide_present {
            return;
        }

        let Some(house) = self.find_free_house() else {
            return;
        };
        let (hx, hy) = house;

        if let Some(index) = homeless.first() {
            if let Some(npc) = self.npcs.get_mut(*index) {
                npc.home = Some(house);
                npc.position = (hx as f32 * 16.0, (hy - 3) as f32 * 16.0);
                npc.dirty = true;
            }
            let name = self
                .npcs
                .get(*index)
                .map(|n| n.stats.name)
                .unwrap_or("Someone");
            self.announce(&format!("{name} has moved in."));
            self.broadcast_npc(*index);
            return;
        }

        // Nobody is homeless and there is no Guide, so the Guide moves in.
        if let Some(index) = self
            .npcs
            .spawn(GUIDE, (hx as f32 * 16.0, (hy - 3) as f32 * 16.0))
        {
            if let Some(npc) = self.npcs.get_mut(index) {
                npc.home = Some(house);
            }
            self.announce("The Guide has moved in.");
            self.broadcast_npc(index);
        }
    }

    /// Find a valid house near a player that no town NPC has claimed.
    /// Look for a room somebody could move into, around one player.
    ///
    /// One player, not all of them: the search is four hundred and twenty-five probes and each
    /// promising one is a flood fill, which is a few tenths of a millisecond. That is nothing once
    /// every five seconds — but it is *per player*, and thirty players would put the whole tick
    /// budget into a single tick. Taking them in turn caps the cost at one player's worth however
    /// many are on, and only means a house is found within a few scans rather than the first.
    fn find_free_house(&mut self) -> Option<(i32, i32)> {
        let playing: Vec<u8> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing())
            .map(|p| p.slot)
            .collect();
        if playing.is_empty() {
            return None;
        }
        self.housing_turn = (self.housing_turn + 1) % playing.len();
        let whose = playing[self.housing_turn];

        let taken: Vec<(i32, i32)> = self.npcs.iter().filter_map(|(_, npc)| npc.home).collect();
        for player in self
            .players
            .iter()
            .flatten()
            .filter(|p| p.slot == whose && p.is_playing())
        {
            let (px, py) = (
                (player.position.0 / 16.0) as i32,
                (player.position.1 / 16.0) as i32,
            );
            // Probe a coarse grid around the player rather than every tile: a house is at least
            // sixty tiles, so a five-tile step cannot miss one.
            for dx in (-60..=60).step_by(5) {
                for dy in (-40..=40).step_by(5) {
                    let (x, y) = (px + dx, py + dy);
                    if let Ok(room) = housing::check_room(&self.world, x, y) {
                        let home = room.home_tile();
                        // Two residents never share a room.
                        if taken
                            .iter()
                            .any(|(tx, ty)| (tx - home.0).abs() < 20 && (ty - home.1).abs() < 20)
                        {
                            continue;
                        }
                        return Some(home);
                    }
                }
            }
        }
        None
    }

    /// Packets 63 and 64: a player painted a tile or a wall.
    ///
    /// The paint is kept rather than only relayed, because it goes into the save and into every
    /// section a client asks for afterwards. A tile that is not there cannot be painted, which is
    /// what stops a crafted packet colouring in the empty sky.
    fn on_paint(&mut self, slot: u8, id: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (i32::from(r.i16()?), i32::from(r.i16()?));
        let colour = r.u8()?;
        // The last byte separates paint from coating; a coating is not a colour and is only
        // relayed, because nothing on the server reads it.
        let coating = r.u8().unwrap_or(0) != 0;

        if !self.world.in_bounds(x, y) {
            return Ok(());
        }
        if !coating {
            let mut tile = self.world.tile(x, y);
            let painting_a_wall = id == id::SYNC_WALL_PAINT_OR_COATING;
            let real = if painting_a_wall {
                tile.wall != 0
            } else {
                tile.is_active()
            };
            if !real {
                debug!(slot, x, y, "painting nothing");
                return Ok(());
            }
            if painting_a_wall {
                tile.wall_color = colour;
            } else {
                tile.color = colour;
            }
            self.world.set_tile(x, y, tile);
        }
        self.broadcast(packets::verbatim(id, payload)?, Some(slot));
        Ok(())
    }

    /// Packets 89, 123, 133 and 149: putting an item into a frame, rack, platter or jar.
    ///
    /// All four are the same message with a different id, and all four are the whole point of the
    /// furniture they belong to: a weapon rack that cannot be given a weapon is a wall decoration.
    ///
    /// Whatever was in it already falls out, which is what the game does and what a player
    /// expects — swapping the sword on your rack should not eat the old one.
    fn on_display_item(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use terrustia_proto::tile_entity::EntityData;
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (r.i16()?, r.i16()?);
        let item = ItemStack {
            id: i32::from(r.i16()?),
            prefix: r.u8()?,
            stack: r.i16()?,
        };

        let Some(at) = self
            .world
            .tile_entities
            .iter()
            .position(|e| e.x == x && e.y == y)
        else {
            // Nothing there to put it in, so it lands on the floor rather than vanishing.
            self.spawn_item(item, tile_corner(x, y));
            return Ok(());
        };
        let entity = &mut self.world.tile_entities[at];
        let EntityData::Held(existing) = entity.data else {
            // That kind of furniture does not hold a single item; the packet is for the wrong one.
            return Ok(());
        };
        entity.data = EntityData::Held(item);
        let id = entity.id;
        if !existing.is_empty() {
            self.spawn_item(existing, tile_corner(x, y));
        }
        self.share_tile_entity(id);
        Ok(())
    }

    /// Packet 156: a kite or a critter being clipped onto its anchor.
    fn on_anchor_item(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use terrustia_proto::tile_entity::EntityData;
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (r.i16()?, r.i16()?);
        let item = r.i16()?;
        let Some(entity) = self
            .world
            .tile_entities
            .iter_mut()
            .find(|e| e.x == x && e.y == y)
        else {
            return Ok(());
        };
        let EntityData::Anchor { item: held } = &mut entity.data else {
            return Ok(());
        };
        *held = item;
        let id = entity.id;
        self.share_tile_entity(id);
        Ok(())
    }

    /// Packet 121: one slot of a mannequin.
    ///
    /// The message names a slot and a command rather than sending the whole thing, because a
    /// mannequin has nineteen slots and a player changes one at a time. Command 2 is the pose;
    /// 0, 1 and 3 are the armour, the dyes and the accessory.
    fn on_display_doll_slot(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use terrustia_proto::tile_entity::EntityData;
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let _claimed_player = r.u8()?; // rewritten to the sender, as vanilla does
        let id = r.i32()?;
        let index = usize::from(r.u8()?);
        let command = r.u8()?;

        let Some(entity) = self.world.tile_entities.iter_mut().find(|e| e.id == id) else {
            return Ok(());
        };
        let EntityData::DisplayDoll(doll) = &mut entity.data else {
            return Ok(());
        };
        if command == DOLL_POSE {
            doll.pose = r.u8()?;
        } else {
            let item = ItemStack {
                id: i32::from(r.u16()? as i16),
                stack: r.u16()? as i16,
                prefix: r.u8()?,
            };
            let into = match command {
                DOLL_DYE => doll.dyes.get_mut(index),
                DOLL_MISC => doll.misc.get_mut(index),
                _ => doll.equip.get_mut(index),
            };
            let Some(into) = into else {
                return Ok(()); // a slot number past the end is a crafted packet
            };
            *into = item;
        }
        // Relayed rather than re-serialised: the payload is exactly what every other client
        // needs, and the sender already has it.
        self.broadcast(packets::rewrite_owner(id::T_E_DISPLAY_DOLL_DATA_SYNC, payload, slot)?, Some(slot));
        Ok(())
    }

    /// Packet 124: one slot of a hat rack.
    ///
    /// Two hats and two dyes, with the dye flag folded into the slot number by adding two — which
    /// is why the number has to be split apart again before it is used as an index.
    fn on_hat_rack_slot(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use terrustia_proto::tile_entity::EntityData;
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let _claimed_player = r.u8()?;
        let id = r.i32()?;
        let mut index = usize::from(r.u8()?);
        let dye = index >= 2;
        if dye {
            index -= 2;
        }

        let Some(entity) = self.world.tile_entities.iter_mut().find(|e| e.id == id) else {
            return Ok(());
        };
        let EntityData::HatRack(rack) = &mut entity.data else {
            return Ok(());
        };
        let item = ItemStack {
            id: i32::from(r.u16()? as i16),
            stack: r.u16()? as i16,
            prefix: r.u8()?,
        };
        let into = if dye {
            rack.dyes.get_mut(index)
        } else {
            rack.items.get_mut(index)
        };
        let Some(into) = into else {
            return Ok(());
        };
        *into = item;
        self.broadcast(
            packets::rewrite_owner(id::T_E_HAT_RACK_ITEM_SYNC, payload, slot)?,
            Some(slot),
        );
        Ok(())
    }

    /// Packet 122: which tile entity a player currently has open.
    ///
    /// Two things hang off it. A client cannot edit an entity it has not claimed, and only one
    /// player may hold a given entity at a time — which is what stops two people emptying the
    /// same mannequin into their own inventories at once.
    fn on_tile_entity_interaction(
        &mut self,
        slot: u8,
        payload: &[u8],
    ) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let id = r.i32()?;

        if id == NO_TILE_ENTITY {
            self.tile_entity_anchors.remove(&slot);
        } else {
            if !self.world.tile_entities.iter().any(|e| e.id == id) {
                return Ok(());
            }
            // Somebody else already has it open, so this claim is refused rather than shared.
            if self
                .tile_entity_anchors
                .iter()
                .any(|(&who, &held)| who != slot && held == id)
            {
                return Ok(());
            }
            self.tile_entity_anchors.insert(slot, id);
        }

        // Everyone is told who holds what, so each client can grey the thing out.
        let mut w = terrustia_proto::PacketWriter::new(id::REQUEST_TILE_ENTITY_INTERACTION);
        w.i32(id).u8(slot);
        let frame = w.finish()?;
        self.broadcast(frame, None);
        Ok(())
    }

    /// Packet 87: a client placed a tile entity.
    ///
    /// The tile has to be there and be the right one, and there has to be nothing there already.
    /// Both checks are the server's: without them a crafted packet hangs an item frame in the sky
    /// or stacks a hundred dummies on one tile.
    fn on_tile_entity_placed(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use terrustia_proto::tile_entity::{EntityKind, TileEntity};
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (r.i16()?, r.i16()?);
        let Some(kind) = EntityKind::from_id(r.u8()?) else {
            return Ok(());
        };
        if !self.world.in_bounds(i32::from(x), i32::from(y)) {
            return Ok(());
        }
        if self.world.tile_entities.iter().any(|e| e.x == x && e.y == y) {
            return Ok(());
        }
        // The tile it claims to stand on has to actually be there.
        if let Some(wanted) = kind.tile() {
            let tile = self.world.tile(i32::from(x), i32::from(y));
            if !tile.is_active() || tile.block != wanted {
                debug!(slot, x, y, ?kind, "nothing there to place that on");
                return Ok(());
            }
        }

        let id = self.world.next_tile_entity;
        self.world.next_tile_entity += 1;
        self.world.tile_entities.push(TileEntity::new(id, kind, x, y));
        debug!(slot, x, y, ?kind, id, "tile entity placed");
        // Everyone has to be told, the placer included: the client sends the placement but does
        // not create the entity itself, and the id it will refer to from now on is the server's
        // to hand out.
        self.share_tile_entity(id);
        Ok(())
    }

    /// Tell everyone nearby what a tile entity now holds.
    ///
    /// Until this goes out an entity does not exist as far as a client is concerned. An item
    /// frame hangs empty, a mannequin stands bare, and a pylon is scenery you cannot travel to —
    /// which is what every one of them was before this was sent at all.
    /// To everyone rather than only to those nearby, which is what the game does at every one of
    /// its own call sites. It matters: a client keeps its copy of an entity after it has walked
    /// away, and a section is only re-sent when its *tiles* change — which filling an item frame
    /// does not do. Sending only to those in range would leave that client believing in the
    /// contents it saw last, permanently. There are a few hundred of these in a world and they
    /// change when somebody touches them, so the cost is nothing like an NPC sync's.
    fn share_tile_entity(&mut self, id: i32) {
        let Some(entity) = self.world.tile_entities.iter().find(|e| e.id == id) else {
            return;
        };
        let Ok(frame) = terrustia_proto::tile_entity::share(entity) else {
            return;
        };
        self.broadcast(frame, None);
    }

    /// Tell everyone a tile entity has gone.
    fn unshare_tile_entity(&mut self, id: i32) {
        if let Ok(frame) = terrustia_proto::tile_entity::unshare(id) {
            self.broadcast(frame, None);
        }
    }

    /// One tick of the tile entities.
    ///
    /// Only the training dummy does anything: it puts an NPC out when somebody comes near and
    /// takes it away when they leave, which is the only way that NPC ever exists. The rest are
    /// storage and are only there to be remembered.
    fn tick_tile_entities(&mut self) {
        use terrustia_proto::tile_entity::EntityKind;
        /// How far a dummy will notice you from.
        const DUMMY_REACH: f32 = 1600.0;
        const DUMMY_NPC: u16 = 488;

        if self.world.tile_entities.is_empty() {
            return;
        }
        let watchers: Vec<(f32, f32)> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing())
            .map(|p| p.position)
            .collect();

        // An entity whose tile has gone is gone with it. Without this it is a ghost: it keeps its
        // spot reserved so nothing can be placed there again, and it goes on being ticked forever.
        let mut orphaned = Vec::new();
        for (at, entity) in self.world.tile_entities.iter().enumerate() {
            let Some(wanted) = entity.kind.tile() else {
                continue;
            };
            let tile = self.world.tile(i32::from(entity.x), i32::from(entity.y));
            if !tile.is_active() || tile.block != wanted {
                orphaned.push((at, entity.npc(), entity.id));
            }
        }
        for (at, npc, id) in orphaned.iter().rev() {
            if let Some(index) = npc {
                self.npcs.remove(*index);
                self.broadcast_npc_death(*index);
            }
            self.world.tile_entities.remove(*at);
            // Clients keep their own copy, so one that is not told goes on believing in an item
            // frame nobody can see or take from.
            self.unshare_tile_entity(*id);
        }

        let mut raise = Vec::new();
        let mut lower = Vec::new();
        for (at, entity) in self.world.tile_entities.iter().enumerate() {
            if entity.kind != EntityKind::TrainingDummy {
                continue;
            }
            let here = (
                f32::from(entity.x) * crate::game::npc::TILE,
                f32::from(entity.y) * crate::game::npc::TILE,
            );
            let watched = watchers
                .iter()
                .any(|p| (p.0 - here.0).abs() < DUMMY_REACH && (p.1 - here.1).abs() < DUMMY_REACH);
            // A dummy whose own tile has gone takes its NPC with it.
            let tile = self.world.tile(i32::from(entity.x), i32::from(entity.y));
            let planted = tile.is_active() && Some(tile.block) == entity.kind.tile();
            match entity.npc() {
                Some(index) if !watched || !planted => lower.push((at, index)),
                None if watched && planted => raise.push((at, here)),
                _ => {}
            }
        }

        for (at, index) in lower {
            self.npcs.remove(index);
            self.broadcast_npc_death(index);
            if let Some(entity) = self.world.tile_entities.get_mut(at) {
                entity.set_npc(None);
            }
        }
        for (at, here) in raise {
            // It stands on its own tile, and carries where it was planted in its ai so its routine
            // can tell whether it is still there.
            let Some(index) = self.npcs.spawn(DUMMY_NPC, (here.0 + 16.0, here.1 + 48.0)) else {
                continue;
            };
            if let Some(entity) = self.world.tile_entities.get(at)
                && let Some(dummy) = self.npcs.get_mut(index)
            {
                dummy.ai[0] = f32::from(entity.x);
                dummy.ai[1] = f32::from(entity.y);
            }
            if let Some(entity) = self.world.tile_entities.get_mut(at) {
                entity.set_npc(Some(index));
            }
            self.broadcast_npc(index);
        }
    }

    /// Packet 34: a chest or dresser was placed or broken.
    ///
    /// This is where a chest stops existing. Without it the tile goes but the chest stays
    /// registered — a ghost that still answers when somebody clicks the empty space it used to
    /// occupy, and that goes into the save with no tile to belong to.
    fn on_chest_update(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        /// A dresser is a chest by another name, three tiles wide instead of two.
        const DRESSER_BLOCK: u16 = 88;
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let action = r.u8()?;
        let (x, y) = (r.i16()?, r.i16()?);
        let _style = r.i16()?;
        if !self.world.in_bounds(i32::from(x), i32::from(y)) {
            return Ok(());
        }

        // Odd actions break; even ones place. Placement arrives as a tile square from the client,
        // which the ordinary handler already applies, so only the registration is done here.
        let breaking = action % 2 == 1;
        let block = match action {
            0 | 1 => CHEST_BLOCK,
            2 | 3 => DRESSER_BLOCK,
            4 | 5 => terrustia_proto::locks::CHEST_2,
            _ => return Ok(()),
        };
        if breaking {
            // The client reports whichever corner was clicked; the chest is anchored at the
            // top-left, so walk back to it before looking the chest up.
            let tile = self.world.tile(i32::from(x), i32::from(y));
            let wide = if block == DRESSER_BLOCK { 54 } else { 36 };
            let anchor = (
                x - (tile.frame_x % wide) / 18,
                y - i16::from(tile.frame_y % 36 != 0),
            );
            if let Some((id, chest)) = self.world.chest_at(anchor.0, anchor.1) {
                // A chest with anything in it is not breakable, the same rule the game uses.
                if chest.items.iter().any(|item| item.stack > 0) {
                    debug!(slot, x, y, "refusing to break a chest with things in it");
                    return Ok(());
                }
                self.world.remove_chest(id);
                debug!(slot, x = anchor.0, y = anchor.1, id, "chest removed");
            }
        }
        self.broadcast(packets::verbatim(id::CHEST_UPDATES, payload)?, Some(slot));
        Ok(())
    }

    /// Packet 52: a key turned in a chest or a door.
    ///
    /// Two of the chests are gated on Plantera, and that gate is the server's to hold: the biome
    /// chests and the temple are the whole reward for beating her.
    fn on_lock(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use terrustia_proto::locks::{self, LockAction};
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let action = LockAction::from_id(r.u8()?);
        let (x, y) = (i32::from(r.i16()?), i32::from(r.i16()?));
        let Some(action) = action else {
            return Ok(());
        };
        if !self.world.in_bounds(x, y) || !self.world.in_bounds(x + 1, y + 1) {
            return Ok(());
        }

        let anchor = self.world.tile(x, y);
        let moved = match action {
            LockAction::UnlockDoor => {
                // A door has no frame arithmetic to do here — the client reframes it and pushes a
                // tile square. Relaying is what keeps the other clients in step.
                anchor.is_active() && anchor.block == locks::DOOR_CLOSED
            }
            LockAction::UnlockChest | LockAction::LockChest => {
                let style = i32::from(anchor.frame_x) / 36;
                let shift = if action == LockAction::UnlockChest {
                    locks::unlock_shift(anchor.block, style)
                } else {
                    locks::lock_shift(anchor.block, style)
                };
                let Some((shift, needs_plantera)) = shift else {
                    debug!(slot, x, y, block = anchor.block, style, "not a lock");
                    return Ok(());
                };
                if needs_plantera && !self.world.progress.downed_plantera {
                    debug!(slot, x, y, "that lock waits for Plantera");
                    return Ok(());
                }
                // A chest is two tiles by two, and all four carry the frame.
                let toward = if action == LockAction::UnlockChest {
                    -shift
                } else {
                    shift
                };
                for dx in 0..2 {
                    for dy in 0..2 {
                        let mut tile = self.world.tile(x + dx, y + dy);
                        if tile.block != anchor.block {
                            continue;
                        }
                        tile.frame_x += toward;
                        self.world.set_tile(x + dx, y + dy, tile);
                    }
                }
                true
            }
        };
        if !moved {
            return Ok(());
        }
        self.broadcast(packets::verbatim(id::LOCK_AND_UNLOCK, payload)?, Some(slot));
        Ok(())
    }

    /// Packet 59: a switch, lever or pressure plate was hit.
    ///
    /// The circuit runs here as well as being relayed: an actuator changes what the world *is*,
    /// and a trap has to throw the same dart at everybody rather than one per client.
    fn on_hit_switch(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (i32::from(r.i16()?), i32::from(r.i16()?));
        if !self.world.in_bounds(x, y) {
            return Ok(());
        }

        // The circuit is run here rather than only relayed. An actuator changes what the world
        // *is* — whether a block is solid — so a server that leaves it to the clients has a world
        // where players walk through walls the server thinks are there.
        let fired = {
            let world = &mut self.world;
            crate::world::wiring::hit_switch(world, x, y)
        };
        self.apply_circuit(fired, (x, y));

        self.broadcast(packets::verbatim(id::HIT_SWITCH, payload)?, Some(slot));
        Ok(())
    }

    /// Everything a circuit set in motion, including whatever its logic gates went on to do.
    ///
    /// A gate does not act on the world itself: it works out its new state and then starts a
    /// circuit of its own, which can toggle further lamps and run further gates. That cascade is
    /// what makes wiring a machine rather than a switchboard, and it is run here to a ceiling,
    /// because a ring of gates would otherwise go round for ever.
    fn apply_circuit(&mut self, fired: crate::world::wiring::Fired, from: (i32, i32)) {
        /// How many rounds of gates one circuit is allowed to set off.
        const MAX_CASCADE: usize = 64;

        let mut pending = vec![fired];
        let mut fired_gates: std::collections::HashSet<(i32, i32)> =
            std::collections::HashSet::new();
        let mut rounds = 0;

        while let Some(fired) = pending.pop() {
            if fired.truncated {
                warn!(
                    x = from.0,
                    y = from.1,
                    reached = fired.reached,
                    "circuit cut short"
                );
            }
            for (cx, cy) in fired.changed {
                self.broadcast_tile(cx, cy);
            }
            for (tx, ty) in fired.traps {
                self.fire_trap(tx, ty);
            }
            for (sx, sy) in fired.statues {
                self.run_statue(sx, sy);
            }
            if let [a, b] = fired.teleporters[..] {
                self.run_teleporters(a, b);
            }
            if !fired.pump_in.is_empty() && !fired.pump_out.is_empty() {
                self.run_pumps(&fired.pump_in, &fired.pump_out);
            }
            for (tx, ty) in fired.timers_started {
                self.running_timers.insert((tx, ty), TIMER_WINDOW);
            }
            for (tx, ty) in fired.timers_stopped {
                self.running_timers.remove(&(tx, ty));
            }

            if rounds >= MAX_CASCADE {
                if !fired.lamps.is_empty() {
                    warn!(
                        x = from.0,
                        y = from.1,
                        "logic gates went round too many times"
                    );
                }
                continue;
            }
            rounds += 1;
            for (lx, ly) in fired.lamps {
                self.broadcast_tile(lx, ly);
                let result = {
                    let world = &mut self.world;
                    crate::world::wiring::check_logic_gate(
                        world,
                        lx,
                        ly,
                        &fired_gates,
                        &mut self.rng,
                    )
                };
                let Some(result) = result else { continue };
                self.broadcast_tile(result.at.0, result.at.1);
                if !result.fires {
                    continue;
                }
                fired_gates.insert(result.at);
                let onward = {
                    let world = &mut self.world;
                    crate::world::wiring::trip_wire(world, result.at.0, result.at.1)
                };
                pending.push(onward);
            }
        }
    }

    /// Tell everybody about one tile that changed.
    fn broadcast_tile(&mut self, x: i32, y: i32) {
        let tile = self.world.tile(x, y);
        let square = TileSquare {
            x: x as i16,
            y: y as i16,
            width: 1,
            height: 1,
            change_type: 0,
            tiles: vec![tile],
        };
        if let Ok(frame) = square.encode() {
            self.broadcast(frame, None);
        }
    }

    /// Fire every running timer whose turn it is.
    ///
    /// A timer is the only thing in the wire table that starts a circuit with nobody touching it,
    /// and it is how most contraptions actually run: a farm, a lift, a light that blinks. Each one
    /// counts down from the same window the game uses and fires whenever the count is a multiple
    /// of its own period, which is what keeps two timers of the same kind in step.
    fn tick_timers(&mut self) {
        use crate::world::wiring::{timer_is_running, timer_period};

        if self.running_timers.is_empty() {
            return;
        }
        let mut due = Vec::new();
        let mut gone = Vec::new();
        for (&(x, y), left) in &mut self.running_timers {
            let tile = self.world.tile(x, y);
            if !timer_is_running(tile) {
                gone.push((x, y));
                continue;
            }
            *left -= 1;
            if *left <= 0 || (*left).rem_euclid(timer_period(tile.frame_x)) == 0 {
                *left = TIMER_WINDOW;
                due.push((x, y));
            }
        }
        for at in gone {
            self.running_timers.remove(&at);
        }
        for (x, y) in due {
            let fired = {
                let world = &mut self.world;
                crate::world::wiring::trip_wire(world, x, y)
            };
            self.apply_circuit(fired, (x, y));
        }
    }

    /// Fire one trap the current reached, if it is not still cooling down.
    ///
    /// The cooldown is what separates a trap from a machine gun: a pressure plate a slime is
    /// sitting on is hit every tick, and without this every one of those hits would be a dart.
    fn fire_trap(&mut self, x: i32, y: i32) {
        /// The one trap projectile that is rationed by how many are already out.
        const SPIKY_BALL: u16 = 185;

        let tile = self.world.tile(x, y);
        let Some(shot) = crate::world::wiring::trap_shot(tile, x, y, &mut self.rng) else {
            return;
        };
        if self.mech_cooldown.contains_key(&shot.cools_at) {
            return;
        }
        // Spiky balls are also rationed by how many are already lying about, which is what stops
        // a held-down plate burying a corridor in them.
        if shot.projectile_type == SPIKY_BALL {
            let at = (shot.position.0, shot.position.1);
            let allowed = crate::world::wiring::spiky_ball_allowed(
                self.projectiles
                    .iter()
                    .filter(|(_, p)| p.projectile_type == SPIKY_BALL)
                    .map(|(_, p)| {
                        let c = p.center();
                        ((c.0 - at.0).powi(2) + (c.1 - at.1).powi(2)).sqrt()
                    }),
            );
            if !allowed {
                return;
            }
        }
        // The cooldown is taken whether or not a slot was free: a trap that could not fire for
        // want of a projectile slot has still gone off, and should not retry every tick.
        self.mech_cooldown.insert(shot.cools_at, shot.cooldown);
        if let Some(index) = self.projectiles.launch(
            shot.projectile_type,
            shot.position,
            shot.velocity,
            shot.damage,
            0,
        ) {
            self.broadcast_projectile(index);
        }
    }

    /// Run one statue the current reached.
    ///
    /// The spawn point is the middle of the statue's base, which is why a slime statue on a
    /// platform drops its slime onto the platform rather than into the tile it is standing in.
    fn run_statue(&mut self, x: i32, y: i32) {
        use terrustia_proto::statues::{self, Statue};

        let tile = self.world.tile(x, y);
        let (style, _) = statues::style_at(tile.frame_x, tile.frame_y);
        let Some(what) = statues::statue(style) else {
            return;
        };
        if self.mech_cooldown.contains_key(&(x, y)) {
            return;
        }
        let base = ((x * 16 + 16) as f32, ((y + 3) * 16) as f32);

        match what {
            Statue::Npc {
                types,
                offset,
                needs_room,
            } => {
                let npc_type = types[self.rng.random_range(0..types.len())];
                if !self.statue_spawn_allowed(npc_type, base) {
                    // Still take the cooldown: the statue fired, it simply had nothing to give.
                    self.mech_cooldown.insert((x, y), what.cooldown());
                    return;
                }
                self.mech_cooldown.insert((x, y), what.cooldown());
                // Something wide needs the ground around the statue to be clear, or it would
                // appear inside a wall.
                if needs_room && self.solid_tiles(x - 2, x + 3, y, y + 2) {
                    return;
                }
                let at = (base.0 + offset.0 as f32, base.1 + offset.1 as f32);
                if let Some(index) = self.npcs.spawn(npc_type, at) {
                    // A statue's monster is worth nothing and does not count against the spawn
                    // budget, which is what makes a farm a farm rather than a way to stop the
                    // world spawning anything else.
                    if let Some(npc) = self.npcs.get_mut(index) {
                        npc.from_statue = true;
                    }
                    self.broadcast_npc(index);
                }
            }
            Statue::Item { item, offset_y } => {
                let at = (base.0, base.1 + offset_y as f32);
                let crowded = !statues::item_spawn_allowed(
                    self.items
                        .iter()
                        .filter(|(_, w)| w.item.id == item)
                        .map(|(_, w)| {
                            ((w.position.0 - at.0).powi(2) + (w.position.1 - at.1).powi(2)).sqrt()
                        }),
                );
                self.mech_cooldown.insert((x, y), what.cooldown());
                if crowded {
                    return;
                }
                if let Some(index) = self.items.spawn(ItemStack::new(item, 1, 0), at) {
                    self.broadcast_item(index);
                }
            }
            Statue::Lure { types } => {
                self.mech_cooldown.insert((x, y), what.cooldown());
                let candidates: Vec<u8> = self
                    .npcs
                    .iter()
                    .filter(|(_, n)| types.contains(&n.npc_type) && n.is_alive())
                    .map(|(index, _)| index)
                    .collect();
                if candidates.is_empty() {
                    return;
                }
                let index = candidates[self.rng.random_range(0..candidates.len())];
                if let Some(npc) = self.npcs.get_mut(index) {
                    npc.position = (base.0 - npc.width() / 2.0, base.1 - npc.height() - 1.0);
                    npc.velocity = (0.0, 0.0);
                }
                self.broadcast_npc(index);
            }
            Statue::Becomes { block } => {
                self.mech_cooldown.insert((x, y), what.cooldown());
                for dx in 0..2i32 {
                    for dy in 0..3i32 {
                        let mut tile = self.world.tile(x + dx, y + dy);
                        tile.block = block;
                        tile.frame_x = (dx * 18 + 216) as i16;
                        tile.frame_y = (dy * 18) as i16;
                        self.world.set_tile(x + dx, y + dy, tile);
                    }
                }
                let square = TileSquare {
                    x: x as i16,
                    y: y as i16,
                    width: 2,
                    height: 3,
                    change_type: 0,
                    tiles: (0..6)
                        .map(|i| self.world.tile(x + i % 2, y + i / 2))
                        .collect(),
                };
                if let Ok(frame) = square.encode() {
                    self.broadcast(frame, None);
                }
            }
        }
    }

    /// Swap everything standing on one teleporter with everything standing on the other.
    ///
    /// It is a swap rather than a one-way trip: whatever is on each pad moves by the vector to
    /// the other, so two players on opposite pads change places in one pull.
    fn run_teleporters(&mut self, a: (i32, i32), b: (i32, i32)) {
        use crate::game::ai::{PLAYER_HEIGHT, PLAYER_WIDTH};
        use crate::world::wiring::{TELEPORTER_BOX, teleport_pair_is_useful};

        /// Whether a box overlaps a teleporter's catchment.
        fn overlaps(pad: (f32, f32, f32, f32), at: (f32, f32), size: (f32, f32)) -> bool {
            at.0 < pad.0 + pad.2
                && at.0 + size.0 > pad.0
                && at.1 < pad.1 + pad.3
                && at.1 + size.1 > pad.1
        }

        if !teleport_pair_is_useful(a, b) {
            return;
        }
        // The catchment reaches up from the teleporter's own row, which is why standing on one
        // works and walking past one at head height does not.
        let box_of = |at: (i32, i32)| {
            (
                (at.0 * 16) as f32,
                (at.1 * 16) as f32 - TELEPORTER_BOX,
                TELEPORTER_BOX,
                TELEPORTER_BOX,
            )
        };
        let (pad_a, pad_b) = (box_of(a), box_of(b));
        let hop = (pad_b.0 - pad_a.0, pad_b.1 - pad_a.1);

        // Both directions are worked out before anything moves, so a player who has just arrived
        // on the far pad is not sent straight back.
        let mut moves: Vec<(u8, (f32, f32))> = Vec::new();
        let mut npc_moves: Vec<(u8, (f32, f32))> = Vec::new();
        for (pad, shift) in [(pad_a, hop), (pad_b, (-hop.0, -hop.1))] {
            for slot in 0..self.players.len() {
                let Some(player) = self.players[slot].as_ref() else {
                    continue;
                };
                if !player.is_playing() || player.life <= 0 {
                    continue;
                }
                let slot = player.slot;
                if moves.iter().any(|(s, _)| *s == slot) {
                    continue;
                }
                if overlaps(
                    pad,
                    player.position,
                    (PLAYER_WIDTH as f32, PLAYER_HEIGHT as f32),
                ) {
                    moves.push((
                        slot,
                        (player.position.0 + shift.0, player.position.1 + shift.1),
                    ));
                }
            }
            let riders: Vec<(u8, (f32, f32))> = self
                .npcs
                .iter()
                .filter(|(index, n)| {
                    n.is_alive()
                        && n.life_max > 5
                        && !n.stats.boss
                        && !n.no_tile_collide
                        && !npc_moves.iter().any(|(i, _)| i == index)
                        && overlaps(pad, n.position, (n.width(), n.height()))
                })
                .map(|(index, n)| (index, (n.position.0 + shift.0, n.position.1 + shift.1)))
                .collect();
            npc_moves.extend(riders);
        }

        for (slot, to) in moves {
            if let Some(player) = self.player_mut(slot) {
                player.position = to;
                player.velocity = (0.0, 0.0);
            }
            let mut w = terrustia_proto::PacketWriter::new(id::TELEPORT_ENTITY);
            // Flags zero: a player, moving to a given place, with no extra field.
            w.u8(0).i16(i16::from(slot)).f32(to.0).f32(to.1).u8(0);
            if let Ok(frame) = w.finish() {
                self.broadcast(frame, None);
            }
        }
        for (index, to) in npc_moves {
            if let Some(npc) = self.npcs.get_mut(index) {
                npc.position = to;
                npc.velocity = (0.0, 0.0);
            }
            self.broadcast_npc(index);
        }
    }

    /// Move liquid from the inlet pumps a circuit reached to its outlets.
    fn run_pumps(&mut self, inlets: &[(i32, i32)], outlets: &[(i32, i32)]) {
        let changed = {
            let world = &mut self.world;
            crate::world::wiring::transfer_liquid(world, inlets, outlets)
        };
        for (x, y) in changed {
            // The moved liquid has to settle from where it landed, or it would sit in a column of
            // its own until something else disturbed it.
            self.liquids.disturb(x, y);
            let tile = self.world.tile(x, y);
            let square = TileSquare {
                x: x as i16,
                y: y as i16,
                width: 1,
                height: 1,
                change_type: 0,
                tiles: vec![tile],
            };
            if let Ok(frame) = square.encode() {
                self.broadcast(frame, None);
            }
        }
    }

    /// Whether a statue may add another of this type, given how crowded it already is.
    fn statue_spawn_allowed(&self, npc_type: u16, at: (f32, f32)) -> bool {
        terrustia_proto::statues::spawn_allowed(
            self.npcs
                .iter()
                .filter(|(_, n)| {
                    n.is_alive() && terrustia_proto::statues::same_family(npc_type, n.npc_type)
                })
                .map(|(_, n)| {
                    ((n.position.0 - at.0).powi(2) + (n.position.1 - at.1).powi(2)).sqrt()
                }),
        )
    }

    /// Whether any tile in this rectangle is solid.
    fn solid_tiles(&self, from_x: i32, to_x: i32, from_y: i32, to_y: i32) -> bool {
        (from_x..=to_x).any(|x| {
            (from_y..=to_y).any(|y| {
                let tile = self.world.tile(x, y);
                tile.is_active() && terrustia_proto::tile_solid::solid(tile.block)
            })
        })
    }

    /// Count down every trap that has fired recently, and forget the ones that are ready again.
    fn tick_mech_cooldowns(&mut self) {
        self.mech_cooldown.retain(|_, ticks| {
            *ticks -= 1;
            *ticks > 0
        });
    }

    /// Packet 70: a critter was caught in a net, and is now an item.
    fn on_bug_caught(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let index = r.i16()?;
        let Ok(index) = u8::try_from(index) else {
            return Ok(());
        };
        // Only a critter can be netted. Refusing anything else is what stops a crafted packet
        // deleting a boss.
        let catchable = self
            .npcs
            .get(index)
            .is_some_and(|n| n.stats.friendly && !n.stats.town_npc && !n.stats.boss);
        if !catchable {
            debug!(slot, index, "refusing to net that");
            return Ok(());
        }
        self.npcs.remove(index);
        self.broadcast_npc_death(index);
        Ok(())
    }

    /// Packet 71: a critter was let out of a jar.
    fn on_bug_released(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (r.i32()?, r.i32()?);
        let npc_type = r.i16()?;
        let Ok(npc_type) = u16::try_from(npc_type) else {
            return Ok(());
        };
        // The same rule in reverse: a jar holds critters, so only a critter comes out of one.
        let is_a_critter = terrustia_proto::npc_data::npc_stats(npc_type)
            .is_some_and(|s| s.friendly && !s.town_npc && !s.boss);
        if !is_a_critter {
            debug!(slot, npc_type, "refusing to release that");
            return Ok(());
        }
        if let Some(index) = self.npcs.spawn(npc_type, (x as f32, y as f32)) {
            self.broadcast_npc(index);
        }
        Ok(())
    }

    /// Packet 48: a bucket poured, or a client telling us liquid moved.
    ///
    /// The amount is taken and the tile woken; the settling itself is the server's, so a client
    /// cannot make water flow uphill by saying it did.
    fn on_liquid(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (i32::from(r.i16()?), i32::from(r.i16()?));
        let amount = r.u8()?;
        let kind = r.u8()?;
        if !self.world.in_bounds(x, y) {
            return Ok(());
        }
        let mut tile = self.world.tile(x, y);
        tile.liquid = amount;
        tile.liquid_kind = match kind {
            1 => terrustia_proto::tile::Liquid::Lava,
            2 => terrustia_proto::tile::Liquid::Honey,
            3 => terrustia_proto::tile::Liquid::Shimmer,
            _ => terrustia_proto::tile::Liquid::Water,
        };
        self.world.set_tile(x, y, tile);
        self.liquids.disturb(x, y);
        Ok(())
    }

    /// Packet 113: a player put an Eternia Crystal on its stand.
    ///
    /// This is the only way the Old One's Army begins, and it is refused more often than not: the
    /// stand has to be real, there cannot already be a crystal, and the arena has to be sixty
    /// tiles clear on both sides. That last check is why building a proper arena is part of
    /// preparing for the event rather than a nicety.
    fn on_crystal_placed(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use terrustia_proto::npc_params::DD2_ETERNIA_CRYSTAL;
        /// The Eternia Crystal Stand.
        const STAND: u16 = 466;
        /// How much room the arena needs each side of the stand, in tiles.
        const ARENA_CLEARANCE: i32 = 60;

        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (i32::from(r.i16()?), i32::from(r.i16()?));

        if self
            .npcs
            .iter()
            .any(|(_, n)| n.npc_type == DD2_ETERNIA_CRYSTAL)
        {
            return Ok(());
        }
        let tile = self.world.tile(x, y);
        if !tile.is_active() || tile.block != STAND {
            debug!(slot, x, y, "no crystal stand there");
            return Ok(());
        }
        // The crystal sits at the middle of the stand, not at the tile that was clicked.
        let origin = (
            x - i32::from(tile.frame_x) / 18,
            y - i32::from(tile.frame_y) / 18,
        );
        let (left, right) = crate::game::army::arena_ends(&WorldTiles(&self.world), origin);
        if right.0 - origin.0 < ARENA_CLEARANCE || origin.0 - left.0 < ARENA_CLEARANCE {
            debug!(slot, x, y, "the arena is too small for the army");
            return Ok(());
        }

        let Some(tier) = self.army_tier() else {
            return Ok(());
        };
        self.army.start(tier, origin);
        self.army_arena = Some((left, right));
        // Three hundred ticks before the first wave, so the arena has a moment to settle.
        self.army.hold = 300;

        let at = (
            origin.0 as f32 * crate::game::npc::TILE + 40.0,
            origin.1 as f32 * crate::game::npc::TILE + 64.0,
        );
        if let Some(index) = self.npcs.spawn(DD2_ETERNIA_CRYSTAL, at) {
            self.broadcast_npc(index);
        }
        self.announce("The Old One's Army is approaching!");
        info!(
            slot,
            ?tier,
            x = origin.0,
            y = origin.1,
            "old one's army started"
        );
        Ok(())
    }

    /// One tick of settling liquid.
    ///
    /// Nothing happens unless something has been disturbed, so a world nobody is digging in costs
    /// nothing here. What moves is sent as tile squares, batched by row, because a flowing pool
    /// changes a run of neighbours at once and one packet each would be a flood of its own.
    fn tick_liquids(&mut self) {
        if self.liquids.pending() == 0 {
            return;
        }
        let settled = {
            let world = &mut self.world;
            self.liquids.tick(world)
        };
        let mut touched: Vec<(i32, i32)> = settled.changed;
        touched.extend(settled.reacted.iter().map(|(x, y, _)| (*x, *y)));
        if touched.is_empty() {
            return;
        }
        touched.sort_unstable();
        touched.dedup();
        // Runs of changed tiles in the same column go out as one square. A flowing pool changes a
        // stripe of neighbours every tick, and a packet each would be a flood of its own.
        let mut runs: Vec<(i32, i32, i32)> = Vec::new();
        for (x, y) in touched {
            match runs.last_mut() {
                Some((rx, _, end)) if *rx == x && *end + 1 == y => *end = y,
                _ => runs.push((x, y, y)),
            }
        }
        for (x, top, bottom) in runs {
            let height = (bottom - top + 1).clamp(1, 255) as u8;
            let tiles: Vec<terrustia_proto::Tile> = (0..i32::from(height))
                .map(|dy| self.world.tile(x, top + dy))
                .collect();
            let square = TileSquare {
                x: x as i16,
                y: top as i16,
                width: 1,
                height,
                change_type: 0,
                tiles,
            };
            if let Ok(frame) = square.encode() {
                self.broadcast(frame, None);
            }
        }
    }

    /// One tick of the wind and the rain.
    ///
    /// Both are gated on somebody in the world having found a life crystal, which is the game's
    /// own way of keeping a brand-new world's weather quiet.
    fn tick_weather(&mut self) {
        let strong_enough = self
            .players
            .iter()
            .flatten()
            .any(|p| p.is_playing() && p.life_max >= 120);
        let was_raining = self.weather.raining;
        let hard_mode = self.world.progress.hard_mode;
        self.weather.tick(strong_enough, hard_mode, &mut self.rng);
        // The world carries the weather so it goes into the save with everything else.
        self.world.wind = self.weather.wind;
        self.world.raining = self.weather.raining;
        self.world.rain_time = self.weather.rain_time;
        self.world.max_rain = self.weather.max_rain;
        self.world.sandstorm = self.weather.sandstorm;
        self.world.sandstorm_time = self.weather.sandstorm_time;
        self.world.sandstorm_severity = self.weather.severity;
        self.world.sandstorm_intended_severity = self.weather.intended_severity;
        if was_raining != self.weather.raining {
            self.announce(if self.weather.raining {
                "It has started to rain."
            } else {
                "The rain has stopped."
            });
            self.broadcast_world_data();
        }
    }

    /// One tick of the Lunar Apocalypse: the pillars' shields, and the minute after the last one.
    fn tick_lunar(&mut self) {
        use crate::game::lunar::{MOON_LORD, PILLARS};
        let standing = self
            .npcs
            .iter()
            .filter(|(_, n)| PILLARS.contains(&n.npc_type))
            .count();
        let here = self.npcs.iter().any(|(_, n)| n.npc_type == MOON_LORD);
        let was_up = self.lunar.up;
        if self.lunar.tick(standing, here) {
            self.summon_moon_lord();
        }
        if was_up && !self.lunar.up {
            self.announce("Impending doom approaches...");
            info!("the last pillar has fallen");
        }
        // The world remembers which pillars are standing, so a save mid-apocalypse comes back to
        // the same fight rather than an empty sky.
        let standing_now = |ty: u16| self.npcs.iter().any(|(_, n)| n.npc_type == ty);
        let towers = (
            standing_now(crate::game::lunar::SOLAR),
            standing_now(crate::game::lunar::VORTEX),
            standing_now(crate::game::lunar::NEBULA),
            standing_now(crate::game::lunar::STARDUST),
        );
        let p = &mut self.world.progress;
        p.lunar_apocalypse_up = self.lunar.up;
        // A pillar that was standing and is not any more has been beaten, and that is permanent.
        p.downed_tower_solar |= p.tower_active_solar && !towers.0;
        p.downed_tower_vortex |= p.tower_active_vortex && !towers.1;
        p.downed_tower_nebula |= p.tower_active_nebula && !towers.2;
        p.downed_tower_stardust |= p.tower_active_stardust && !towers.3;
        (
            p.tower_active_solar,
            p.tower_active_vortex,
            p.tower_active_nebula,
            p.tower_active_stardust,
        ) = towers;
        // Each pillar carries its own shield on itself, so its routine can read it without
        // knowing the event exists.
        if self.lunar.up {
            let shields: Vec<(u8, i32)> = self
                .npcs
                .iter()
                .filter(|(_, n)| PILLARS.contains(&n.npc_type))
                .map(|(index, n)| (index, self.lunar.shield_of(n.npc_type)))
                .collect();
            for (index, shield) in shields {
                if let Some(pillar) = self.npcs.get_mut(index)
                    && pillar.shield != shield
                {
                    pillar.shield = shield;
                    pillar.dirty = true;
                }
            }
        }
        self.broadcast_lunar_state();
    }

    /// Tell clients what the four shields read, and how long is left before the Moon Lord.
    ///
    /// Neither was ever sent. The shield is the pillar fight's entire feedback loop — it is what
    /// tells a player their hits are counting and how close the pillar is to becoming killable —
    /// and without it the bar over each pillar sits full while the fight is won underneath it.
    ///
    /// Only when something changed, since these tick every frame and almost never move.
    fn broadcast_lunar_state(&mut self) {
        let shields = self.lunar.shields;
        let countdown = self.lunar.countdown;
        if shields != self.last_sent_shields {
            self.last_sent_shields = shields;
            let mut w = terrustia_proto::PacketWriter::new(id::UPDATE_TOWER_SHIELD_STRENGTHS);
            for shield in shields {
                w.u16(u16::try_from(shield.max(0)).unwrap_or(u16::MAX));
            }
            if let Ok(frame) = w.finish() {
                self.broadcast(frame, None);
            }
        }
        if countdown != self.last_sent_countdown {
            self.last_sent_countdown = countdown;
            let mut w = terrustia_proto::PacketWriter::new(id::MOONLORD_HORROR);
            w.i32(crate::game::lunar::MOON_LORD_COUNTDOWN).i32(countdown);
            if let Ok(frame) = w.finish() {
                self.broadcast(frame, None);
            }
        }
    }

    /// Tell clients how far through an invasion is.
    ///
    /// This is the progress bar. Without it a player has no way to know whether a goblin army is
    /// nearly over or has barely started, which turns a paced event into an indefinite one.
    fn broadcast_invasion_progress(&mut self) {
        let (progress, target, kind, wave) = match &self.invasion {
            Some(invasion) => (
                invasion.started_with - invasion.remaining,
                invasion.started_with,
                invasion.kind as i8,
                0i8,
            ),
            // Zero of zero is how the game says "nothing is happening", which is what takes the
            // bar off the screen.
            None => (0, 0, 0, 0),
        };
        if (progress, target) == self.last_sent_invasion {
            return;
        }
        self.last_sent_invasion = (progress, target);
        let mut w = terrustia_proto::PacketWriter::new(id::INVASION_PROGRESS_REPORT);
        w.i32(progress).i32(target).i8(kind).i8(wave);
        if let Ok(frame) = w.finish() {
            self.broadcast(frame, None);
        }
    }

    /// Tear the sky open. This is what killing the Lunatic Cultist does.
    fn trigger_lunar_apocalypse(&mut self) {
        if self.lunar.up || self.lunar.countdown > 0 {
            return;
        }
        let raised = self.lunar.trigger(
            self.world.width(),
            i32::from(self.world.surface),
            self.world.progress.downed_moon_lord,
            &mut self.rng,
        );
        for (npc_type, x, y) in raised {
            let x = x.clamp(20, self.world.width() - 20);
            let at = (
                x as f32 * crate::game::npc::TILE,
                y as f32 * crate::game::npc::TILE,
            );
            if let Some(index) = self.npcs.spawn(npc_type, at) {
                if let Some(pillar) = self.npcs.get_mut(index) {
                    pillar.shield = self.lunar.shield_of(npc_type);
                }
                self.broadcast_npc(index);
            }
        }
        self.announce("The Lunar Apocalypse is upon us!");
        info!("lunar apocalypse");
    }

    /// He arrives on whoever is nearest the middle of the world, not on whoever killed the last
    /// pillar — which is why standing somewhere sensible during the countdown matters.
    fn summon_moon_lord(&mut self) {
        let middle = (
            self.world.width() as f32 / 2.0 * crate::game::npc::TILE,
            f32::from(self.world.surface) / 2.0 * crate::game::npc::TILE,
        );
        let nearest = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing() && p.life > 0)
            .min_by(|a, b| {
                let reach = |p: &&crate::game::player::Player| {
                    (p.position.0 - middle.0).hypot(p.position.1 - middle.1)
                };
                reach(a).total_cmp(&reach(b))
            })
            .map(|p| p.slot);
        let Some(slot) = nearest else {
            return;
        };
        self.summon_on_player(slot, crate::game::lunar::MOON_LORD);
        self.announce("The Moon Lord has awoken!");
        info!(slot, "moon lord");
    }

    /// The world's own rolls at first light: an eclipse, once a mechanical boss is down.
    fn roll_dawn_events(&mut self) {
        if self.world.progress.hard_mode
            && self.world.progress.downed_mech_any
            && !self.world.eclipse
            && rand::Rng::random_range(&mut self.rng, 0..20) == 0
        {
            self.world.eclipse = true;
            self.announce("A solar eclipse is happening!");
            info!("solar eclipse");
        }
        // A meteor is rolled for every dawn once the evil's boss is down, and again whenever one
        // is owed from a kill. It is where meteorite bars come from, and with them the first
        // weapon that does not run out of ammunition.
        let owed = self.world.progress.spawn_meteor;
        if owed
            || (self.world.progress.downed_boss2
                && rand::Rng::random_range(&mut self.rng, 0..50) == 0)
        {
            self.world.progress.spawn_meteor = false;
            self.land_meteor();
        }
        self.roll_angler_quest();
    }

    /// Pick the fish the Angler wants today, and let everybody try again.
    ///
    /// `Main.AnglerQuestSwap`. It re-rolls until it lands on a fish this world can actually
    /// produce — asking for a hardmode fish in a fresh world would cost the player the whole
    /// day's reward — and clears the list of who has already handed one in.
    fn roll_angler_quest(&mut self) {
        use terrustia_proto::angler;
        let p = &self.world.progress;
        let any_boss = p.downed_boss1
            || p.downed_boss2
            || p.downed_boss3
            || p.hard_mode
            || p.downed_king_slime
            || p.downed_queen_bee;
        let (hardmode, crimson) = (p.hard_mode, self.world.crimson);

        let catchable: Vec<usize> = angler::QUESTS
            .iter()
            .enumerate()
            .filter(|(_, q)| angler::available(q, hardmode, crimson, any_boss))
            .map(|(index, _)| index)
            .collect();
        if catchable.is_empty() {
            return; // cannot happen with the shipped table, but guessing is worse than doing nothing
        }
        let at = rand::Rng::random_range(&mut self.rng, 0..catchable.len());
        self.angler_quest = catchable[at] as u8;
        self.angler_finished_today.clear();
        self.broadcast_angler_quest();
    }

    /// Tell each player what the Angler wants, and whether *they* have already handed one in.
    ///
    /// The second half is per-player, which is why this cannot be one broadcast: the packet
    /// carries "have you finished today", and every client needs its own answer.
    fn broadcast_angler_quest(&mut self) {
        let quest = self.angler_quest;
        let names: Vec<(u8, String)> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing())
            .map(|p| (p.slot, p.name.clone()))
            .collect();
        for (slot, name) in names {
            let done = self.angler_finished_today.contains(&name);
            let mut w = terrustia_proto::PacketWriter::new(id::ANGLER_QUEST);
            w.u8(quest).bool(done);
            if let Ok(frame) = w.finish() {
                self.send(slot, frame);
            }
        }
    }

    /// Packet 75: a player reporting that they have handed the Angler today's fish.
    ///
    /// One a day each, which the server has to be the judge of — a client that could tell itself
    /// it had not yet handed one in could farm the reward all day.
    fn on_angler_finished(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        let Some(name) = self
            .player(slot)
            .filter(|p| p.is_playing())
            .map(|p| p.name.clone())
        else {
            return Ok(());
        };
        if self.angler_finished_today.insert(name) {
            debug!(slot, "angler quest handed in");
        }
        Ok(())
    }

    /// Packet 76: how many quests a player has finished, and their golf score.
    ///
    /// Both live on the character rather than the world, so the server's job is to remember what
    /// it is told and pass it on — that is what makes the Angler's reward tiers work at all,
    /// since they are gated on the count.
    fn on_quest_count(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let _claimed = r.u8()?;
        let quests = r.i32()?;
        let golf = r.i32()?;
        if let Some(player) = self.player_mut(slot) {
            player.angler_quests = quests;
            player.golf_score = golf;
        }
        let mut w = terrustia_proto::PacketWriter::new(id::QUESTS_COUNT_SYNC);
        w.u8(slot).i32(quests).i32(golf);
        let frame = w.finish()?;
        self.broadcast(frame, Some(slot));
        Ok(())
    }

    /// Bring a meteor down somewhere out of the way, and tell everyone it happened.
    fn land_meteor(&mut self) {
        let landed = {
            let world = &mut self.world;
            crate::world::meteor::drop(world, &mut self.rng)
        };
        let Some((x, y)) = landed else {
            debug!("nowhere for a meteor to land");
            return;
        };
        self.announce("A meteorite has landed!");
        info!(x, y, "meteorite landed");
        self.push_region(x, y, METEOR_REACH);
    }

    /// Push a square region of the world at every client.
    ///
    /// A client that already holds a section will not ask for it again, so a change this size —
    /// a crater, a hardmode stripe — has to be *sent* or nobody sees it until they rejoin. It
    /// goes as a grid of tile squares, because one square carries at most 255 tiles a side.
    fn push_region(&mut self, x: i32, y: i32, reach: i32) {
        const CHUNK: i32 = 50;
        let (from_x, to_x) = ((x - reach).max(0), (x + reach).min(self.world.width() - 1));
        let (from_y, to_y) = ((y - reach).max(0), (y + reach).min(self.world.height() - 1));

        let mut at_x = from_x;
        while at_x <= to_x {
            let width = CHUNK.min(to_x - at_x + 1);
            let mut at_y = from_y;
            while at_y <= to_y {
                let height = CHUNK.min(to_y - at_y + 1);
                let square = TileSquare {
                    x: at_x as i16,
                    y: at_y as i16,
                    width: width as u8,
                    height: height as u8,
                    change_type: 0,
                    // Column-major: all of one column, then the next.
                    tiles: (0..width)
                        .flat_map(|dx| (0..height).map(move |dy| (dx, dy)))
                        .map(|(dx, dy)| self.world.tile(at_x + dx, at_y + dy))
                        .collect(),
                };
                if let Ok(frame) = square.encode() {
                    self.broadcast(frame, None);
                }
                at_y += height;
            }
            at_x += width;
        }
    }

    /// ...and at nightfall: a blood moon, which will not rise on a new moon and will not rise for
    /// a party of characters who have not found a life crystal between them.
    fn roll_dusk_events(&mut self) {
        if self.world.blood_moon || self.moon.running() || self.world.moon_phase == 4 {
            return;
        }
        let worth_it = self
            .players
            .iter()
            .flatten()
            .any(|p| p.is_playing() && p.life_max > 120);
        if worth_it && rand::Rng::random_range(&mut self.rng, 0..9) == 0 {
            self.world.blood_moon = true;
            self.announce("The blood moon is rising...");
            info!("blood moon");
            self.broadcast_world_data();
        }
    }

    /// Raise a moon, if it is night.
    ///
    /// The two are exclusive and both cancel a blood moon, but raising one does not *fail* when
    /// the other is up — it replaces it, and the new moon starts again at wave one. Refusing
    /// instead would leave somebody who summoned a frost moon fighting a pumpkin one.
    fn start_moon(&mut self, moon: crate::game::moons::Moon, slot: u8) {
        use crate::game::moons::Moon;
        if self.world.day_time {
            return;
        }
        if let Some(was) = self.moon.moon
            && was != moon
        {
            info!(?was, now = ?moon, "one moon replaces the other");
        }
        self.world.blood_moon = false;
        self.moon.start(moon);
        self.world.pumpkin_moon = moon == Moon::Pumpkin;
        self.world.snow_moon = moon == Moon::Frost;
        self.announce(match moon {
            Moon::Pumpkin => "The pumpkin moon is rising...",
            Moon::Frost => "The frost moon is rising...",
        });
        self.broadcast_world_data();
        info!(slot, ?moon, "moon started");
    }

    /// Put a moon away. Dawn does this, and so does raising the other one.
    fn stop_moon(&mut self) {
        let wave = self.moon.wave;
        let Some(moon) = self.moon.stop() else {
            return;
        };
        info!(?moon, wave, "moon over");
        self.world.pumpkin_moon = false;
        self.world.snow_moon = false;
        self.broadcast_world_data();
    }

    /// Count a kill against whatever moon is up.
    ///
    /// Kills are worth points rather than one each — a Pumpking is a third of a wave by itself and
    /// a scarecrow is nothing — so what you choose to fight decides how far the night gets.
    fn note_moon_kill(&mut self, npc_type: u16) {
        if let Some(wave) = self.moon.note_kill(npc_type, self.world.game_mode) {
            self.announce(&format!("Wave {wave}!"));
        }
    }

    /// Land whatever an enemy leaves behind on the player it just touched.
    fn apply_touch_debuffs(&mut self, slot: u8, npc_type: u16) {
        let expert = self.world.game_mode >= 1;
        for rule in terrustia_proto::touch_debuffs::on_touch(npc_type) {
            if rule.expert_only && !expert {
                continue;
            }
            if rule.one_in > 1 && !rand::Rng::random_ratio(&mut self.rng, 1, rule.one_in) {
                continue;
            }
            let ticks = if rule.ticks.1 > rule.ticks.0 {
                rand::Rng::random_range(&mut self.rng, rule.ticks.0..=rule.ticks.1)
            } else {
                rule.ticks.0
            };
            if let Ok(frame) = terrustia_proto::packets::add_player_buff(slot, rule.buff, ticks) {
                self.broadcast(frame, None);
            }
        }
    }

    /// Record a boss's death against the world's history.
    ///
    /// Nothing in the game reads a boss's death directly — everything reads the flag it sets. A
    /// shop that opens, a spawn pool that widens, an event that becomes possible: all of it hangs
    /// off this, which is why a server that kills bosses without recording them has a world that
    /// never progresses.
    fn note_boss_kill(&mut self, npc_type: u16) {
        use terrustia_proto::npc_params as ids;
        let p = &mut self.world.progress;
        let mut announce: Option<&'static str> = None;
        match npc_type {
            // Pre-hardmode.
            4 => p.downed_boss1 = true,
            13 | 266 => p.downed_boss2 = true,
            35 | 36 => p.downed_boss3 = true,
            50 => p.downed_king_slime = true,
            222 => p.downed_queen_bee = true,
            668 => p.downed_deerclops = true,
            // The wall: the one death that changes the world itself.
            113 => {
                if !p.hard_mode {
                    self.start_hardmode();
                }
                return;
            }
            // The mechanical three. Any one of them is what unlocks the next tier.
            134 => {
                p.downed_mech1 = true;
                p.downed_mech_any = true;
            }
            125 | 126 => {
                // The Twins only count once both eyes are gone.
                if self
                    .npcs
                    .iter()
                    .any(|(_, n)| matches!(n.npc_type, 125 | 126))
                {
                    return;
                }
                p.downed_mech2 = true;
                p.downed_mech_any = true;
            }
            127 => {
                p.downed_mech3 = true;
                p.downed_mech_any = true;
            }
            262 => p.downed_plantera = true,
            245 => p.downed_golem = true,
            370 => p.downed_fishron = true,
            657 => {
                p.downed_queen_slime = true;
                announce = Some("Queen Slime has been defeated!");
            }
            636 => {
                p.downed_empress_of_light = true;
                announce = Some("The Empress of Light has been defeated!");
            }
            // The lunar chain.
            ids::CULTIST => {
                p.downed_ancient_cultist = true;
                self.trigger_lunar_apocalypse();
                return;
            }
            crate::game::lunar::MOON_LORD => {
                self.lunar.stop();
                let p = &mut self.world.progress;
                p.downed_moon_lord = true;
                p.lunar_apocalypse_up = false;
                self.announce("The Moon Lord has been defeated!");
                self.broadcast_world_data();
                return;
            }
            _ => return,
        }
        // All three mechs down is what starts the bulbs growing, and the bulbs are the only way
        // to Plantera. One goes in immediately so the jungle is worth walking into straight away.
        if self.world.progress.downed_mech1
            && self.world.progress.downed_mech2
            && self.world.progress.downed_mech3
        {
            self.grow_plantera_bulb();
        }
        if let Some(text) = announce {
            self.announce(text);
        }
        // The flags reach clients in packet 7 and nowhere else, so every change has to be told.
        self.broadcast_world_data();
    }

    /// The wall has fallen: cut the two stripes through the world and turn hardmode on.
    ///
    /// This is the largest single thing that ever happens to a world, and it happens once. The
    /// stripes are cut immediately rather than in the background — a world of a few million tiles
    /// takes a fraction of a second, and doing it inline means no client can see a half-converted
    /// world.
    fn start_hardmode(&mut self) {
        use crate::world::hardmode;
        use terrustia_proto::convert::Biome;

        if self.world.progress.hard_mode {
            return;
        }
        self.world.progress.hard_mode = true;
        let evil = if self.world.crimson {
            Biome::Crimson
        } else {
            Biome::Corruption
        };
        // The dungeon's side decides which way the stripes lean, so neither lands on it.
        let dungeon_x = self.world.dungeon_x.unwrap_or(self.world.width() / 4);
        let stripes = hardmode::hardmode_stripes(self.world.width(), dungeon_x, &mut self.rng);
        let began = std::time::Instant::now();
        let mut converted = 0usize;
        for ((x, drift), into) in stripes.into_iter().zip([Biome::Hallow, evil]) {
            let changed = {
                let world = &mut self.world;
                hardmode::run_stripe(world, x, drift, into, &mut self.rng)
            };
            converted += changed.len();
        }
        self.announce("The ancient spirits of light and dark have been released!");
        info!(
            converted,
            took_ms = began.elapsed().as_millis(),
            "hardmode began"
        );
        // Every client's view of the world is now wrong: drop the caches so they re-request.
        self.section_cache.clear();
        self.broadcast_world_data();
    }

    /// One tick of the biomes creeping.
    ///
    /// A handful of tiles are picked at random near the players each tick rather than the whole
    /// world being scanned. That is how the game does it too, and it is why an infection creeps
    /// where somebody is standing and sits still where nobody is.
    fn tick_spread(&mut self) {
        use crate::world::hardmode;
        if !self.world.progress.hard_mode || !self.ticks.is_multiple_of(SPREAD_EVERY) {
            return;
        }
        let here: Vec<(i32, i32)> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing())
            .map(|p| {
                (
                    (p.position.0 / crate::game::npc::TILE) as i32,
                    (p.position.1 / crate::game::npc::TILE) as i32,
                )
            })
            .collect();
        if here.is_empty() {
            return;
        }
        let downed_plantera = self.world.progress.downed_plantera;
        let mut changed = Vec::new();
        for (px, py) in here {
            for _ in 0..SPREAD_TRIES {
                let x = px + rand::Rng::random_range(&mut self.rng, -SPREAD_RANGE..=SPREAD_RANGE);
                let y = py + rand::Rng::random_range(&mut self.rng, -SPREAD_RANGE..=SPREAD_RANGE);
                if x < 10 || y < 10 || x >= self.world.width() - 10 || y >= self.world.height() - 10
                {
                    continue;
                }
                let taken = {
                    let world = &mut self.world;
                    hardmode::spread(world, x, y, downed_plantera, &mut self.rng)
                };
                changed.extend(taken);
            }
        }
        for (x, y) in changed {
            let tile = self.world.tile(x, y);
            let square = TileSquare {
                x: x as i16,
                y: y as i16,
                width: 1,
                height: 1,
                change_type: 0,
                tiles: vec![tile],
            };
            if let Ok(frame) = square.encode() {
                self.broadcast(frame, None);
            }
        }
    }

    /// Packet 51: the odd-jobs packet, whose first action is the only way to fight Skeletron.
    ///
    /// A client sends it when the player takes the Old Man up on his offer. There is no summon
    /// item for Skeletron and never has been — the dialogue *is* the summon — so without this the
    /// dungeon stays shut and nothing behind it can be reached.
    fn on_misc_data(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        // The player the client names is ignored in favour of the connection it came in on.
        let _claimed = r.u8()?;
        let action = r.u8()?;
        match action {
            1 => self.summon_skeletron(),
            // A sundial or a moondial skipping to the next morning or evening.
            3 => self.skip_to(true),
            6 => self.skip_to(false),
            other => {
                debug!(slot, action = other, "ignoring a misc-data action");
                self.broadcast(packets::verbatim(id::MISC_DATA_SYNC, payload)?, Some(slot));
            }
        }
        Ok(())
    }

    /// Keep the Old Man standing at the dungeon door until Skeletron is beaten.
    ///
    /// He is not a town NPC and does not move in anywhere: he is a fixture of the dungeon, and he
    /// is the only way to start that fight. If he is missing — a fresh server on a world that
    /// never had him, or one where something killed him — he is put back.
    fn tick_old_man(&mut self) {
        const OLD_MAN: u16 = 37;
        const SKELETRON: u16 = 35;

        if !self.ticks.is_multiple_of(OLD_MAN_CHECK_INTERVAL) {
            return;
        }
        if self.world.progress.downed_boss3 {
            return;
        }
        // Not while the fight is on: he has become it.
        if self.npcs.iter().any(|(_, n)| n.npc_type == SKELETRON) {
            return;
        }
        if self.npcs.iter().any(|(_, n)| n.npc_type == OLD_MAN) {
            return;
        }
        let (Some(x), Some(y)) = (self.world.dungeon_x, self.world.dungeon_y) else {
            return;
        };
        // Only bother once somebody is near enough to see him arrive.
        let watched = self.players.iter().flatten().any(|p| {
            p.is_playing()
                && (p.position.0 / crate::game::npc::TILE - x as f32).abs() < OLD_MAN_NOTICE
        });
        if !watched {
            return;
        }
        let at = (x as f32 * 16.0, (y - 3) as f32 * 16.0);
        if let Some(index) = self.npcs.spawn(OLD_MAN, at) {
            self.broadcast_npc(index);
            debug!(x, y, "the old man is back at the dungeon");
        }
    }

    /// Turn the Old Man into Skeletron.
    ///
    /// He is not killed and Skeletron is not summoned beside him — he *becomes* it, which is why
    /// the dungeon has no guardian afterwards. The Clothier will do instead, because he is the
    /// same man once the curse is off him.
    fn summon_skeletron(&mut self) {
        const OLD_MAN: u16 = 37;
        const CLOTHIER: u16 = 54;
        const SKELETRON: u16 = 35;

        if self.npcs.iter().any(|(_, n)| n.npc_type == SKELETRON) {
            return;
        }
        let cursed = self
            .npcs
            .iter()
            .find(|(_, n)| matches!(n.npc_type, OLD_MAN | CLOTHIER) && n.is_alive())
            .map(|(index, n)| (index, n.center()));
        let Some((index, at)) = cursed else {
            debug!("nobody at the dungeon to become Skeletron");
            return;
        };

        self.npcs.remove(index);
        self.broadcast_npc_death(index);
        if let Some(spawned) = self.npcs.spawn(SKELETRON, at) {
            self.announce("Skeletron has awoken!");
            self.broadcast_npc(spawned);
        }
    }

    /// A sundial or moondial: jump the clock to the next dawn or dusk.
    fn skip_to(&mut self, dawn: bool) {
        if dawn {
            self.world.day_time = true;
            self.world.time = 0;
        } else {
            self.world.day_time = false;
            self.world.time = 0;
        }
        self.broadcast_world_data();
    }

    /// Put one Plantera's bulb somewhere in the underground jungle, and tell everyone.
    ///
    /// Called when the third mechanical boss falls, and again whenever the last one is broken —
    /// the jungle is never without one, which is what keeps Plantera reachable.
    fn grow_plantera_bulb(&mut self) {
        let grown = {
            let world = &mut self.world;
            crate::world::bulbs::grow(world, &mut self.rng)
        };
        let Some((x, y)) = grown else {
            debug!("nowhere in the jungle to grow a bulb");
            return;
        };
        let square = TileSquare {
            x: x as i16,
            y: (y - 1) as i16,
            width: 2,
            height: 2,
            change_type: 0,
            tiles: (0..4)
                .map(|i| self.world.tile(x + i % 2, y - 1 + i / 2))
                .collect(),
        };
        if let Ok(frame) = square.encode() {
            self.broadcast(frame, None);
        }
        debug!(x, y, "a plantera's bulb grew");
    }

    /// Wake a boss that has no summon item, from the tile that was its only door.
    ///
    /// A Plantera's bulb and a bee larva are both "break this and it comes" — there is no crafted
    /// summon for either, so without this neither boss can ever appear.
    fn wake_from_tile(&mut self, x: i32, y: i32, boss: u16) {
        if self.npcs.iter().any(|(_, n)| n.npc_type == boss) {
            return;
        }
        let at = (x as f32 * 16.0, y as f32 * 16.0);
        let nearest = self
            .players
            .iter()
            .flatten()
            .filter(|player| player.is_playing())
            .min_by(|a, b| {
                let d = |p: &Player| (p.position.0 - at.0).abs() + (p.position.1 - at.1).abs();
                d(a).total_cmp(&d(b))
            })
            .map(|p| p.slot);
        if let Some(slot) = nearest {
            self.summon_on_player(slot, boss);
        }
    }

    /// Break a shadow orb or a crimson heart.
    ///
    /// This is the early game's hinge. The first one in a world always gives a gun, which is what
    /// makes the corruption worth going into at all; every third one wakes the evil's boss, which
    /// is the only way to reach it without crafting a summon. Breaking one also makes a meteor
    /// possible, and that is where the next tier of gear comes from.
    fn smash_orb(&mut self, x: i32, y: i32, frame_x: i16) {
        use terrustia_proto::orbs;

        let already = self.world.progress.shadow_orb_smashed;
        let roll = self.rng.random_range(0..5);
        let at = (x as f32 * 16.0, y as f32 * 16.0);
        for reward in orbs::reward(frame_x, already, roll) {
            if let Some(index) = self
                .items
                .spawn(ItemStack::new(reward.item, reward.stack, 0), at)
            {
                self.broadcast_item(index);
            }
        }

        let p = &mut self.world.progress;
        p.shadow_orb_smashed = true;
        p.shadow_orb_count = p.shadow_orb_count.saturating_add(1);

        if p.shadow_orb_count >= orbs::ORBS_PER_BOSS {
            p.shadow_orb_count = 0;
            let boss = orbs::boss_for(frame_x);
            self.broadcast_world_data();
            // One at a time: a third orb broken while the boss is already awake is wasted, which
            // is the game's own rule and stops a stack of orbs summoning a stack of bosses.
            if self.npcs.iter().any(|(_, n)| n.npc_type == boss) {
                return;
            }
            // On the nearest player, which is who it holds responsible.
            let nearest = self
                .players
                .iter()
                .flatten()
                .filter(|player| player.is_playing())
                .min_by(|a, b| {
                    let d = |p: &Player| (p.position.0 - at.0).abs() + (p.position.1 - at.1).abs();
                    d(a).total_cmp(&d(b))
                })
                .map(|p| p.slot);
            if let Some(slot) = nearest {
                self.summon_on_player(slot, boss);
            }
            return;
        }
        let omen = orbs::omen(p.shadow_orb_count);
        self.announce(omen);
        self.broadcast_world_data();
    }

    /// Break an altar: seed a tier, spray the ore, and put a wraith on whoever did it.
    fn smash_altar(&mut self, x: i32, y: i32, slot: u8) {
        use crate::world::hardmode;

        let mut tiers = self.ore_tiers;
        let Some(smashed) = hardmode::smash(
            self.world.progress.altar_count,
            self.world.progress.hard_mode,
            &mut tiers,
            hardmode::WorldShape {
                width: self.world.width(),
                height: self.world.height(),
                surface: i32::from(self.world.surface),
                rock_layer: i32::from(self.world.rock_layer),
            },
            &mut self.rng,
        ) else {
            return;
        };
        self.ore_tiers = tiers;
        self.world.progress.altar_count += 1;

        let mut dug = Vec::new();
        for (vx, vy, strength, steps) in smashed.veins {
            let changed = {
                let world = &mut self.world;
                hardmode::run_vein(world, (vx, vy), strength, steps, smashed.ore, &mut self.rng)
            };
            dug.extend(changed);
        }
        // The ore lands all over the world, so the changed tiles go out as whole sections rather
        // than as thousands of squares. Clients re-request what they are near.
        for (dx, dy) in &dug {
            self.liquids.wake(*dx, *dy);
        }

        self.announce(smashed.announcement);
        info!(
            x,
            y,
            ore = smashed.ore,
            veins = dug.len(),
            altars = self.world.progress.altar_count,
            "altar smashed"
        );
        if smashed.decided_a_tier {
            self.broadcast_world_data();
        }
        for _ in 0..smashed.wraiths {
            self.summon_on_player(slot, hardmode::WRAITH);
        }
    }

    /// Which tier the world has earned. There is no choosing it: it is whatever the progression
    /// allows, and a fresh world only ever gets tier one.
    fn army_tier(&self) -> Option<crate::game::army::Tier> {
        use crate::game::army::Tier;
        let progress = &self.world.progress;
        Some(if progress.hard_mode && progress.downed_golem {
            Tier::Three
        } else if progress.hard_mode && progress.downed_mech_any {
            Tier::Two
        } else {
            Tier::One
        })
    }

    /// Carry out what the event's fixtures decided this tick.
    ///
    /// The crystal and the gates only ever *ask*: they have no way to make an NPC or end an event
    /// themselves. Keeping the decisions in the routines and the consequences here is what lets
    /// both be tested on their own.
    fn apply_army(
        &mut self,
        gates: Vec<(i32, i32, bool)>,
        releases: Vec<((f32, f32), bool)>,
        ended: Option<bool>,
        close_gates: bool,
    ) {
        use terrustia_proto::npc_params::DD2_LANE_PORTAL;

        self.army.tick();

        for (x, y, left) in gates {
            let at = (
                x as f32 * crate::game::npc::TILE,
                (y as f32 + 1.0) * crate::game::npc::TILE,
            );
            if let Some(index) = self.npcs.spawn(DD2_LANE_PORTAL, at)
                && let Some(gate) = self.npcs.get_mut(index)
            {
                // Which side it is on is the one thing a gate cannot work out for itself.
                gate.ai[2] = if left { 0.0 } else { 1.0 };
                gate.position.0 -= gate.width() / 2.0;
                gate.position.1 -= gate.height();
                self.broadcast_npc(index);
            }
        }

        // A gate that has been told to shut goes into its closing phase wherever it is in its
        // cycle, which is what makes the ending look like the whole arena powering down at once.
        if close_gates {
            let closing: Vec<u8> = self
                .npcs
                .iter()
                .filter(|(_, n)| n.npc_type == DD2_LANE_PORTAL && n.ai[1] == 0.0)
                .map(|(index, _)| index)
                .collect();
            for index in closing {
                if let Some(gate) = self.npcs.get_mut(index) {
                    gate.ai[1] = 1.0;
                    gate.ai[0] = 0.0;
                    gate.dirty = true;
                }
            }
        }

        if !releases.is_empty()
            && let Some(tier) = self.army.tier
        {
            let players = self
                .players
                .iter()
                .flatten()
                .filter(|p| p.is_playing() && p.life > 0)
                .count();
            for (bottom, left) in releases {
                let census: Vec<(u16, usize)> = {
                    let mut counts: std::collections::HashMap<u16, usize> =
                        std::collections::HashMap::new();
                    for (_, n) in self.npcs.iter() {
                        *counts.entry(n.npc_type).or_default() += 1;
                    }
                    counts.into_iter().collect()
                };
                let count = |ty: u16| census.iter().find(|(t, _)| *t == ty).map_or(0, |(_, c)| *c);
                let wanted = crate::game::army::from_gate(
                    tier,
                    self.army.wave,
                    left,
                    self.army.kills,
                    &count,
                    players,
                    &mut self.rng,
                );
                for npc_type in wanted {
                    if let Some(index) = self.npcs.spawn(npc_type, bottom)
                        && let Some(spawned) = self.npcs.get_mut(index)
                    {
                        spawned.position.0 -= spawned.width() / 2.0;
                        spawned.position.1 -= spawned.height();
                        self.broadcast_npc(index);
                    }
                }
            }
        }

        if let Some(won) = ended {
            self.announce(if won {
                "The Old One's Army has been defeated!"
            } else {
                "The Eternia Crystal was destroyed!"
            });
            info!(won, wave = self.army.wave, "old one's army over");
            self.army.stop();
            self.army_arena = None;
        }
    }

    /// Count a kill against the Old One's Army, and advance its waves.
    ///
    /// A goblin also leaves its body where it fell, which is the whole reason a Dark Mage is
    /// dangerous: it turns your own progress back into enemies.
    fn note_army_kill(&mut self, npc_type: u16) {
        if !self.army.ongoing() {
            return;
        }
        // Expert and above count double.
        let expert = self.world.game_mode >= 1;
        if let Some(wave) = self.army.note_kill(npc_type, expert) {
            if self.army.won() {
                // Winning does not end the event here: the crystal plays it out first, and the
                // event ends when the drama does.
                let crystals: Vec<u8> = self
                    .npcs
                    .iter()
                    .filter(|(_, n)| n.npc_type == terrustia_proto::npc_params::DD2_ETERNIA_CRYSTAL)
                    .map(|(index, _)| index)
                    .collect();
                for index in crystals {
                    if let Some(crystal) = self.npcs.get_mut(index) {
                        crystal.ai[1] = 2.0;
                        crystal.ai[0] = 0.0;
                        crystal.dirty = true;
                    }
                }
                self.announce("The Old One's Army has been defeated!");
            } else {
                self.announce(&format!("Old One's Army: wave {} complete!", wave));
            }
        }
    }

    /// Send in the next invader, if it is time and there is room.
    ///
    /// Invaders arrive at the invasion's column rather than around a player, which is what makes
    /// one feel like something marching toward you instead of something appearing on top of you.
    fn spawn_invaders(&mut self, state: InvasionState) {
        let active = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing() && p.life > 0)
            .count();
        if active == 0 {
            return;
        }
        // An invasion presses harder than ordinary spawning, and the cap scales with the party.
        let cap = spawn::MAX_SPAWNS * 2.0 * (1.0 + 0.3 * active as f32);
        if self.npcs.used_slots() >= cap {
            return;
        }
        if !self.ticks.is_multiple_of(INVASION_SPAWN_EVERY) {
            return;
        }

        let present: Vec<u16> = self.npcs.iter().map(|(_, n)| n.npc_type).collect();
        let Some(npc_type) =
            state.next_invader(self.world.progress.hard_mode, &present, &mut self.rng)
        else {
            return;
        };

        // They arrive around a player near the front rather than at the front itself, which is
        // what puts an invasion in front of somebody instead of over the horizon.
        let near_front: Vec<i32> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing() && p.life > 0)
            .map(|p| (p.position.0 / crate::game::npc::TILE) as i32)
            .filter(|x| state.reaches(*x))
            .collect();
        let column = match near_front.as_slice() {
            // Nobody near the front: the army waits rather than spawning into an empty ocean.
            [] => return,
            columns => {
                let at = columns[rand::Rng::random_range(&mut self.rng, 0..columns.len())];
                let side = if state.from_x > state.toward_x { 1 } else { -1 };
                // Just off screen on the side the army is coming from.
                (at + side * rand::Rng::random_range(&mut self.rng, 40..80))
                    .clamp(10, self.world.width() - 10)
            }
        };
        let Some(ground) = spawn::find_ground(&self.world, column, i32::from(self.world.spawn_y))
        else {
            return;
        };
        let at = (
            column as f32 * crate::game::npc::TILE,
            (ground - 1) as f32 * crate::game::npc::TILE,
        );
        if let Some(index) = self.npcs.spawn(npc_type, at) {
            self.broadcast_npc(index);
        }
    }

    /// Begin an invasion, unless one is already under way or nobody qualifies to be invaded.
    fn start_invasion(&mut self, kind: Invasion) {
        if self.invasion.is_some() {
            return;
        }
        // A player qualifies at two hundred maximum life; a world of fresh characters cannot be
        // invaded at all.
        let qualifying = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing() && p.life_max >= 200)
            .count();
        let Some(state) = InvasionState::begin(
            kind,
            qualifying,
            i32::from(self.world.spawn_x),
            self.world.width(),
            &mut self.rng,
        ) else {
            return;
        };
        let announcement = if kind == Invasion::Martian {
            kind.arrival().to_string()
        } else {
            format!("{} {}!", kind.arrival(), state.side())
        };
        self.announce(&announcement);
        info!(
            invasion = ?kind,
            size = state.started_with,
            from_x = state.from_x,
            "invasion started"
        );
        self.invasion = Some(state);
        // Put the bar on the screen, full, the moment the event begins.
        self.broadcast_invasion_progress();
    }

    /// Count a kill against whatever invasion is running, and end it when the last one falls.
    fn note_invasion_kill(&mut self, npc_type: u16) {
        let Some(state) = self.invasion.as_mut() else {
            return;
        };
        if !crate::game::spawn::belongs_to(state.kind, npc_type) {
            return;
        }
        state.remaining -= 1;
        if state.beaten() {
            let kind = state.kind;
            self.invasion = None;
            self.announce(kind.defeat());
            info!(invasion = ?kind, "invasion defeated");
        }
        // The bar moves on every kill, so it is told on every kill rather than on a timer.
        self.broadcast_invasion_progress();
    }

    /// Try to spawn new NPCs around the players.
    fn tick_spawning(&mut self) {
        if self.ticks.is_multiple_of(300) {
            let active = self
                .players
                .iter()
                .flatten()
                .filter(|p| p.is_playing() && p.life > 0)
                .count();
            debug!(
                active,
                npcs = self.npcs.len(),
                slots = self.npcs.used_slots(),
                "spawn tick"
            );
        }
        // While an invasion is running its members replace the ordinary pool. The front closes on
        // the town a tile a tick and they arrive around whoever is near it, so an invasion is
        // something that comes to you rather than something waiting at the edge of the map.
        if let Some(state) = self.invasion.as_mut()
            && state.march()
        {
            let kind = state.kind;
            self.announce(&format!("{} {}", kind.arrival(), "have reached the town!"));
            info!(invasion = ?kind, "the invasion has arrived");
        }
        if let Some(state) = self.invasion {
            self.spawn_invaders(state);
            return;
        }

        // A moon or an eclipse takes the surface pool over entirely, so the census the tables
        // need is built here once rather than per candidate tile.
        let census: std::collections::HashMap<u16, usize> =
            if self.moon.running() || self.world.eclipse {
                let mut counts = std::collections::HashMap::new();
                for (_, n) in self.npcs.iter() {
                    *counts.entry(n.npc_type).or_insert(0) += 1;
                }
                counts
            } else {
                std::collections::HashMap::new()
            };
        let count = |ty: u16| census.get(&ty).copied().unwrap_or(0);
        let progress = &self.world.progress;
        let events = spawn::EventSpawns {
            moon: self.moon.moon.map(|m| (m, self.moon.wave)),
            eclipse: self.world.eclipse,
            downed_plantera: progress.downed_plantera,
            hard_mode: progress.hard_mode,
            downed_mech_any: progress.downed_mech_any,
            downed_all_mechs: progress.downed_mech1
                && progress.downed_mech2
                && progress.downed_mech3,
            // Three of an event's heavies at once is as many as it will put out.
            boss_cap: self.npcs.iter().filter(|(_, n)| n.stats.boss).count() >= 3,
            census: &count,
        };
        let spawned = spawn::try_spawn(
            &self.world,
            &self.npcs,
            &self.players,
            &events,
            &mut self.rng,
            self.ticks,
        );
        for (npc_type, position) in spawned {
            if let Some(index) = self.npcs.spawn(npc_type, position) {
                self.broadcast_npc(index);
            }
        }
    }
}

#[cfg(test)]
mod combat_tests {
    use crate::game::projectile::ProjectileStore;

    /// A shot decided by a routine has to become an entity that moves and can hit somebody.
    /// Everything up to this point only produced intentions.
    #[test]
    fn a_launched_shot_flies_and_then_expires() {
        let mut store = ProjectileStore::new();
        let index = store
            .launch(38, (1000.0, 1000.0), (6.0, 0.0), 15, 60)
            .expect("harpy feather");
        let start = store.get(index).unwrap().position;

        struct Sky;
        impl crate::game::npc::TileView for Sky {
            fn tile(&self, _x: i32, _y: i32) -> terrustia_proto::tile::Tile {
                terrustia_proto::tile::Tile::AIR
            }
        }

        let mut ticks = 0;
        loop {
            let done = {
                let p = store.get_mut(index).unwrap();
                crate::game::projectile::step(p, &Sky, &mut Vec::new())
                    == crate::game::projectile::Outcome::Spent
            };
            ticks += 1;
            if done || ticks > 200 {
                break;
            }
        }
        assert_eq!(ticks, 60, "it should live exactly as long as it was given");
        let travelled = store.get(index).unwrap().position.0 - start.0;
        assert!(travelled > 300.0, "and cover ground, got {travelled}");
    }

    #[test]
    fn a_shot_that_hits_a_player_overlaps_their_box() {
        let mut store = ProjectileStore::new();
        let index = store
            .launch(38, (1000.0, 1000.0), (0.0, 0.0), 15, 60)
            .unwrap();
        let p = store.get(index).unwrap();
        let player_box = (
            1000.0 - crate::game::ai::PLAYER_WIDTH as f32 / 2.0,
            1000.0 - crate::game::ai::PLAYER_HEIGHT as f32 / 2.0,
        );
        assert!(p.overlaps(
            player_box,
            (
                crate::game::ai::PLAYER_WIDTH as f32,
                crate::game::ai::PLAYER_HEIGHT as f32
            )
        ));
    }

    #[test]
    fn a_penetrating_shot_survives_more_than_one_hit() {
        let mut store = ProjectileStore::new();
        // The demon scythe passes through everything.
        let scythe = store.launch(44, (0.0, 0.0), (1.0, 0.0), 21, 60).unwrap();
        assert_eq!(store.get(scythe).unwrap().penetrate, -1);
        // The sand ball does too; the eye laser does not.
        let laser = store.launch(83, (0.0, 0.0), (1.0, 0.0), 11, 60).unwrap();
        assert_eq!(store.get(laser).unwrap().penetrate, 3);
    }
}

#[cfg(test)]
mod worm_tests {
    use crate::game::npc::NpcStore;
    use terrustia_proto::npc_params::EATER_OF_WORLDS;

    /// Resolve the chains the way the server does, without needing a running server.
    ///
    /// Kept in step with `GameServer::resolve_worm_chains` by testing the same rules: this is the
    /// decision table, and the server method is that table plus broadcasting.
    fn resolve(store: &mut NpcStore) {
        use terrustia_proto::npc_params::splitting_worm;
        let followed: std::collections::HashSet<u8> =
            store.iter().filter_map(|(_, npc)| npc.follows).collect();
        let mut transformed = Vec::new();
        let mut orphaned = Vec::new();
        for (index, npc) in store.iter() {
            let Some((head, body, tail)) = splitting_worm(npc.npc_type) else {
                continue;
            };
            let has_leader = npc.follows.is_some_and(|l| store.get(l).is_some());
            let has_follower = followed.contains(&index);
            if !has_leader && !has_follower {
                orphaned.push(index);
            } else if npc.npc_type == body && !has_leader {
                transformed.push((index, head));
            } else if npc.npc_type == body && !has_follower {
                transformed.push((index, tail));
            } else if (npc.npc_type == head && !has_follower)
                || (npc.npc_type == tail && !has_leader)
            {
                orphaned.push(index);
            }
        }
        for (index, into) in transformed {
            if let Some(npc) = store.get_mut(index) {
                let follows = npc.follows;
                npc.become_type(into);
                npc.follows = follows;
            }
        }
        for index in orphaned {
            store.remove(index);
        }
    }

    fn eater(store: &mut NpcStore, segments: usize) -> Vec<u8> {
        let (head, body, tail) = EATER_OF_WORLDS;
        store.spawn_worm(head, body, tail, segments, (1000.0, 1000.0));
        store.iter().map(|(index, _)| index).collect()
    }

    /// Cut one in half and you have two worms, not a worm with a hole in it.
    #[test]
    fn cutting_an_eater_of_worlds_makes_two_of_them() {
        let mut store = NpcStore::new();
        let chain = eater(&mut store, 6);
        assert!(chain.len() >= 5);

        // Take a segment out of the middle.
        let cut = chain[3];
        store.remove(cut);
        resolve(&mut store);

        let (head, _, tail) = EATER_OF_WORLDS;
        let heads = store.iter().filter(|(_, n)| n.npc_type == head).count();
        let tails = store.iter().filter(|(_, n)| n.npc_type == tail).count();
        assert_eq!(heads, 2, "the piece behind the wound grows a head");
        assert_eq!(tails, 2, "and the piece ahead of it grows a tail");
    }

    #[test]
    fn a_single_leftover_segment_dies_rather_than_becoming_a_worm() {
        let mut store = NpcStore::new();
        let chain = eater(&mut store, 3);
        // Leave exactly one segment standing.
        for index in chain.iter().skip(1) {
            store.remove(*index);
        }
        resolve(&mut store);
        assert_eq!(store.len(), 0, "one segment is not a worm");
    }

    #[test]
    fn an_intact_worm_is_left_alone() {
        let mut store = NpcStore::new();
        let chain = eater(&mut store, 6);
        let before: Vec<u16> = store.iter().map(|(_, n)| n.npc_type).collect();
        resolve(&mut store);
        let after: Vec<u16> = store.iter().map(|(_, n)| n.npc_type).collect();
        assert_eq!(before, after);
        assert_eq!(store.len(), chain.len());
    }

    /// Only the Eater does this. Cut a giant worm and the pieces simply die.
    #[test]
    fn other_worms_do_not_split() {
        let mut store = NpcStore::new();
        // Giant worm: head 10, body 11, tail 12.
        store.spawn_worm(10, 11, 12, 5, (1000.0, 1000.0));
        let chain: Vec<u8> = store.iter().map(|(index, _)| index).collect();
        store.remove(chain[2]);
        let before = store.len();
        resolve(&mut store);
        assert_eq!(store.len(), before, "nothing should have changed");
        assert!(
            store
                .iter()
                .all(|(_, n)| n.npc_type != 10 || n.follows.is_none())
        );
    }
}
