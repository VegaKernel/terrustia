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
use rand::{SeedableRng, rngs::SmallRng};
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
    /// The furniture that remembers something: dummies, frames, racks, pylons.
    tile_entities: Vec<terrustia_proto::tile_entity::TileEntity>,
    next_tile_entity: i32,
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
        // Saving needs a world that came from a file; a generated one has no header to preserve.
        let can_save = save_path.is_some() && world.preserved.is_some();
        let autosave_ticks = match (can_save, config.autosave_secs) {
            (true, secs) if secs > 0 => Some(secs * 60),
            _ => None,
        };
        if save_path.is_some() && !can_save {
            warn!("saving is unavailable: this world was generated rather than loaded from a file");
        }

        // The weather comes off the world it was loaded with, so a reloaded save picks up the
        // shower it was in the middle of rather than starting clear.
        let weather = crate::game::weather::Weather {
            wind: world.wind,
            target: world.wind,
            raining: world.raining,
            rain_time: world.rain_time,
            max_rain: world.max_rain,
            ..Default::default()
        };

        Self {
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
            tile_entities: Vec::new(),
            next_tile_entity: 0,
            liquids: crate::world::liquid::Liquids::default(),
            ore_tiers: crate::world::hardmode::OreTiers::default(),
            worst_tick: TickCost::default(),
            worst_stall: Duration::ZERO,
            save_path,
            autosave_ticks,
        }
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
        self.tick_lunar();
        lap(&mut cost, Phase::World);

        self.flush_dirty_sections();
        lap(&mut cost, Phase::Sections);
        self.tick_items();
        lap(&mut cost, Phase::Items);
        self.tick_npcs();
        lap(&mut cost, Phase::Npcs);
        self.tick_projectiles();
        lap(&mut cost, Phase::Projectiles);
        self.tick_contact_damage();
        lap(&mut cost, Phase::Damage);
        self.tick_spawning();
        lap(&mut cost, Phase::Spawning);
        self.tick_town_npcs();
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
    fn spawn_tile_drop(&mut self, tile: u16, x: i32, y: i32) {
        let Some(item_id) = tile_drop(tile) else {
            // Framed objects choose their drop from a style, which is not modelled; dropping the
            // wrong item would be worse than dropping none.
            debug!(tile, "no simple drop for this tile type");
            return;
        };
        let position = (x as f32 * 16.0, y as f32 * 16.0);
        let Some(index) = self.items.spawn(ItemStack::new(item_id, 1, 0), position) else {
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
        if out.try_send(frame).is_err() {
            warn!(slot, "outbound queue full or closed; dropping connection");
            self.remove_player(slot);
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
                        broke = Some(tile.block);
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
        if let Some(block) = broke {
            self.spawn_tile_drop(block, x, y);
            // A demon altar is the only way hardmode ore gets into a world, and it always costs
            // something to break.
            if block == DEMON_ALTAR {
                self.smash_altar(x, y, slot);
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
        let argument = parts.next().unwrap_or("").to_ascii_lowercase();

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
                if self.save_path.is_none() || self.world.preserved.is_none() {
                    self.tell(
                        slot,
                        "This world cannot be saved: it was generated, not loaded from a file.",
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
fn resolve_npc(argument: &str) -> Option<u16> {
    if argument.is_empty() {
        return None;
    }
    if let Ok(id) = argument.parse::<u16>() {
        return npc_stats(id).is_some().then_some(id);
    }
    (0..terrustia_proto::npc_data::NPC_COUNT)
        .find(|id| npc_stats(*id).is_some_and(|s| s.name.eq_ignore_ascii_case(argument)))
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

/// How far a Dark Mage looks for something worth healing, and for a corpse worth raising.
const HEAL_REACH: (f32, f32) = terrustia_proto::npc_params::DARK_MAGE_HEAL_RANGE;
const RAISE_CHECK_RANGE: f32 = terrustia_proto::npc_params::RAISE_CHECK_RANGE;
const RAISE_MINIMUM: usize = terrustia_proto::npc_params::RAISE_MINIMUM;

/// Coin item ids, smallest first.
const COIN_ITEMS: [i32; 4] = [71, 72, 73, 74];

impl GameServer {
    /// Advance every NPC and tell clients about the ones that changed.
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
                // Weather is not modelled, so only nightfall sends residents indoors, and the
                // things that need a wind blowing simply wither.
                raining: self.world.raining,
                windy: self.weather.windy(),
                crimson: self.world.crimson,
                snow: biome == crate::game::spawn::Biome::Snow,
                jungle: biome == crate::game::spawn::Biome::Jungle,
                wind: self.weather.wind,
                // Worked out once a tick from wherever the nearest player is, rather than per NPC:
                // the zone scan reads a forty-tile square and only the tumbleweed asks.
                desert: biome == crate::game::spawn::Biome::Desert,
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
        if terrustia_proto::projectile_data::projectile_stats(sync.projectile_type as u16).is_some()
        {
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

    /// Move every projectile, and remove the ones that are finished.
    fn tick_projectiles(&mut self) {
        let mut spent = Vec::new();
        {
            let tiles = WorldTiles(&self.world);
            for (index, projectile) in self.projectiles.iter_mut() {
                if crate::game::projectile::step(projectile, &tiles)
                    == crate::game::projectile::Outcome::Spent
                {
                    spent.push(index);
                }
            }
        }
        for index in spent {
            self.kill_projectile(index);
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
        if let Ok(frame) = sync.encode() {
            self.broadcast(frame, None);
        }
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
        if let Some(sync) = self.npc_sync(index)
            && let Ok(frame) = sync.encode()
        {
            self.broadcast(frame, None);
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
        let (npc_type, value, center) = (npc.npc_type, npc.stats.value, npc.center());

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
            self.npcs.remove(hit.index);
            self.broadcast_npc_death(hit.index);
            self.drop_coins(value, center);
            self.drop_loot(npc_type, center);
            self.note_invasion_kill(npc_type);
            self.army.note_corpse(npc_type, (center.0, center.1 + 16.0));
            self.note_army_kill(npc_type);
            self.note_moon_kill(npc_type);
            self.lunar.note_kill(npc_type);
            self.note_boss_kill(npc_type);
            debug!(slot, npc_type, "npc killed");
        } else {
            self.broadcast_npc(hit.index);
        }
        Ok(())
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
    /// Type-specific loot tables are not modelled — `NPC.NPCLoot` is thousands of lines of
    /// per-type rolls — but the coin drop is universal and comes straight from the NPC's `value`.
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
    fn find_free_house(&self) -> Option<(i32, i32)> {
        let taken: Vec<(i32, i32)> = self.npcs.iter().filter_map(|(_, npc)| npc.home).collect();

        for player in self.players.iter().flatten().filter(|p| p.is_playing()) {
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
        if self.tile_entities.iter().any(|e| e.x == x && e.y == y) {
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

        let id = self.next_tile_entity;
        self.next_tile_entity += 1;
        self.tile_entities.push(TileEntity {
            id,
            kind,
            x,
            y,
            npc: None,
        });
        debug!(slot, x, y, ?kind, id, "tile entity placed");
        Ok(())
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

        if self.tile_entities.is_empty() {
            return;
        }
        let watchers: Vec<(f32, f32)> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing())
            .map(|p| p.position)
            .collect();

        let mut raise = Vec::new();
        let mut lower = Vec::new();
        for (at, entity) in self.tile_entities.iter().enumerate() {
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
            match entity.npc {
                Some(index) if !watched || !planted => lower.push((at, index)),
                None if watched && planted => raise.push((at, here)),
                _ => {}
            }
        }

        for (at, index) in lower {
            self.npcs.remove(index);
            self.broadcast_npc_death(index);
            if let Some(entity) = self.tile_entities.get_mut(at) {
                entity.npc = None;
            }
        }
        for (at, here) in raise {
            // It stands on its own tile, and carries where it was planted in its ai so its routine
            // can tell whether it is still there.
            let Some(index) = self.npcs.spawn(DUMMY_NPC, (here.0 + 16.0, here.1 + 48.0)) else {
                continue;
            };
            if let Some(entity) = self.tile_entities.get(at)
                && let Some(dummy) = self.npcs.get_mut(index)
            {
                dummy.ai[0] = f32::from(entity.x);
                dummy.ai[1] = f32::from(entity.y);
            }
            if let Some(entity) = self.tile_entities.get_mut(at) {
                entity.npc = Some(index);
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
    /// The wiring itself is not simulated — what a circuit does when it fires is a system of its
    /// own — but the hit is relayed so every client runs the same circuit, which is what makes a
    /// door open on everybody's screen rather than only the one who stepped on the plate.
    fn on_hit_switch(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (i32::from(r.i16()?), i32::from(r.i16()?));
        if !self.world.in_bounds(x, y) {
            return Ok(());
        }
        self.broadcast(packets::verbatim(id::HIT_SWITCH, payload)?, Some(slot));
        Ok(())
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
        self.weather.tick(strong_enough, &mut self.rng);
        // The world carries the weather so it goes into the save with everything else.
        self.world.wind = self.weather.wind;
        self.world.raining = self.weather.raining;
        self.world.rain_time = self.weather.rain_time;
        self.world.max_rain = self.weather.max_rain;
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

    /// Raise a moon, if it is night and one is not already up.
    ///
    /// The two are exclusive and both cancel a blood moon: whichever went up last is the one you
    /// are fighting.
    fn start_moon(&mut self, moon: crate::game::moons::Moon, slot: u8) {
        use crate::game::moons::Moon;
        if self.world.day_time || self.moon.running() {
            return;
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

        // They come in at the column the invasion owns, standing on whatever ground is there.
        let column = state.from_x.clamp(10, self.world.width() - 10);
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
        // While an invasion is running its members replace the ordinary pool, and they walk in
        // from the column it is coming from rather than appearing around each player.
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
                crate::game::projectile::step(p, &Sky) == crate::game::projectile::Outcome::Spent
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
