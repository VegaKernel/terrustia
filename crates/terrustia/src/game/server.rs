//! The single-writer game task.
//!
//! One task owns the world and the player table, so there are no locks on the hot path and packet
//! ordering is deterministic. Connections talk to it over an `mpsc` of [`ServerEvent`]; it talks
//! back through each player's outbound queue.

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
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
///
/// A minute, not a second. Vanilla never sends packet 18 at all — nothing in the game's source
/// calls `SendData(18)` — and keeps clients' clocks right by resending packet 7 whenever something
/// about the world changes, which on a quiet world is a handful of times an hour. A client runs its
/// own clock at a known rate between those.
///
/// Once a second was three and a quarter kilobytes over a five-minute session to say something a
/// client already knew. This keeps a correction, because drifting for an hour on nothing but a
/// client's own arithmetic is a worse failure than a small packet, but at a cadence that costs
/// nothing worth measuring.
const TIME_SYNC_TICKS: u64 = 60 * 60;

/// How often the worst tick in the window is reported, when it is worth reporting.
const TICK_REPORT_EVERY: u64 = 600;

/// The parts of a tick, in the order they run.
///
/// What used to be one `World` phase was thirteen separate systems sharing a lap, so a warning
/// saying `phase=world` narrowed the cause down to "somewhere in most of the tick". A two-hour
/// idle run reported that phase eating half the budget with two NPCs and nobody connected; the
/// cause turned out to be the autosave's world copy, which is now its own entry and would have
/// been obvious from the first warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Copying the world for the background save. Runs on the tick, once every autosave.
    Snapshot,
    Liquids,
    Growth,
    Spread,
    Weather,
    /// The clock, tile entities, wiring timers, lunar events and the biome census.
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
    const NAMES: [&'static str; 14] = [
        "snapshot",
        "liquids",
        "growth",
        "spread",
        "weather",
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

/// Times one phase of a tick, on the same clock the tick's own total uses.
///
/// A named type rather than two lines inline, because those two lines were wrong for months and
/// nothing could see it: phases were timed with `Instant` while the tick total came from
/// `clock::Cpu`, so the warning line compared wall microseconds against CPU microseconds and could
/// report a phase costing more than the whole tick containing it. Every phase figure ever logged
/// was inflated by however long that phase spent descheduled.
///
/// Wrapping it makes the mistake unavailable — there is nowhere here to put an `Instant` — and it
/// makes the property that matters testable on its own, which is the part that counts. Asserting
/// "no phase exceeds its tick" does *not* catch this: on an idle machine the two clocks agree, so
/// that assertion passes against the broken code, which is exactly how it survived so long.
struct PhaseClock(clock::Cpu);

impl PhaseClock {
    fn start() -> Self {
        Self(clock::Cpu::now())
    }

    /// Processor time since the last lap.
    fn lap(&mut self) -> Duration {
        let now = clock::Cpu::now();
        let elapsed = now.since(self.0);
        self.0 = now;
        elapsed
    }
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

/// Pixels of frame each pylon style occupies: three tiles wide, eighteen pixels each.
const PYLON_FRAME_WIDTH: i16 = 54;

/// How far from a pylon a player may stand and still travel, in tiles.
///
/// `TileReachCheckSettings.Pylons` overrides the usual reach with sixty in each direction — far
/// more than an arm's length, so that standing anywhere in the room counts.
const PYLON_REACH: f32 = 60.0;

/// How many townsfolk have to live near a pylon before it will carry anybody.
const PYLON_RESIDENTS_NEEDED: usize = 2;

/// Half the box a pylon looks in for those residents.
///
/// `SceneMetrics.ZoneScanSize` works out to 169 x 124 tiles — a screen at the game's assumed
/// 1920x1200, plus twenty-five tiles of padding on every side — and the box is centred on the
/// pylon.
const PYLON_SCAN_HALF_WIDTH: i32 = 84;
const PYLON_SCAN_HALF_HEIGHT: i32 = 62;

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

/// How many NPC syncs each of the two paths has sent, for the tick-window log.
///
/// Counters rather than a guess. The two paths look interchangeable from the code and are not: a
/// measurement here showed 251 full syncs against 5 streamed ones, which said immediately that the
/// rate limiter was doing all the work and the real problem was upstream in how often an NPC was
/// being marked as having changed. Relaxed atomics on a path that already does a hash lookup.
pub static SYNC_FULL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static SYNC_STREAM: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How much a round of proximity streaming has to accumulate before an update is sent.
///
/// The weights below are chosen against it: eight means a player standing on top of something gets
/// an update every streaming round, and one means they get one every eighth round.
const STREAM_THRESHOLD: u8 = 8;

/// How heavily one player's nearness counts towards streaming an NPC to them.
///
/// From `NPC.StreamUpdatesToNearbyPlayers`: full weight within about fifteen tiles, then halving
/// outward, and nothing at all past ninety-odd tiles — which is roughly a screen and a half.
fn stream_weight(distance: f32) -> u8 {
    match distance {
        d if d < 250.0 => 8,
        d if d < 500.0 => 4,
        d if d < 1000.0 => 2,
        d if d < 1500.0 => 1,
        _ => 0,
    }
}

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

/// How long a finished save took, or that it failed. Sent from the writing thread to the game
/// task, which is the only one that can say anything to anybody.
type SaveOutcome = std::result::Result<u64, ()>;
type SaveReport = std::sync::mpsc::Sender<SaveOutcome>;
type SaveReports = std::sync::mpsc::Receiver<SaveOutcome>;

/// The most password hashes that may be in flight at once, server-wide.
///
/// Per-slot exclusivity alone is not enough: at 255 players, 255 concurrent Argon2 hashes is
/// nineteen megabytes each and every core the machine has. This is the ceiling that makes the
/// cost bounded no matter how many people ask at once.
const MAX_AUTH_IN_FLIGHT: usize = 4;

/// Vanilla's tile-edit spam ceilings and decay rates, from `RemoteClient`.
///
/// Transcribed rather than chosen: these decide how long a burst is tolerated, and picking our own
/// numbers would mean a client vanilla is happy with gets booted here, or the reverse. Placing is
/// the tight one — 100 with 0.3 a tick back means about eighteen a second sustained. Breaking is
/// deliberately loose, because mining legitimately produces a lot of packets very quickly.
const SPAM_PLACE_MAX: f32 = 100.0;
const SPAM_PLACE_DECAY: f32 = 0.3;
const SPAM_BREAK_MAX: f32 = 500.0;
const SPAM_BREAK_DECAY: f32 = 5.0;
const SPAM_LIQUID_MAX: f32 = 50.0;
const SPAM_LIQUID_DECAY: f32 = 0.2;

/// A finished password hash, on its way back to the game task to be applied.
///
/// The work — hashing a new password, or verifying one against a stored hash — happens on a
/// worker thread. Only the game task may touch the admin store, so the answer comes back here and
/// is applied on the next tick.
enum AuthOutcome {
    /// A `/register` whose password has been hashed.
    Registered {
        slot: u8,
        account: Box<std::result::Result<crate::admin::Account, String>>,
        /// Whether this account claims the server, decided before the hash was started.
        first: bool,
    },
    /// A `/login` whose password has been checked.
    SignedIn {
        slot: u8,
        account: String,
        correct: bool,
    },
}

type AuthReport = std::sync::mpsc::Sender<AuthOutcome>;
type AuthReports = std::sync::mpsc::Receiver<AuthOutcome>;

/// The most slots a quick stack may offer at once.
///
/// A player's main inventory is forty slots and the void bag adds another forty. Anything past
/// that is a crafted packet, and reading a claimed count of two billion before checking it is how
/// a server runs out of memory.
const MAX_QUICK_STACK_SLOTS: i32 = 200;

// The two town slimes that are made rather than found, and what packet 140 calls each.
const TRANSFORM_COPPER_SLIME: u8 = 1;
const TRANSFORM_ELDER_SLIME: u8 = 2;
const COPPER_SLIME: u16 = 684;
const OLD_SLIME: u16 = 679;
/// The only slime an Old Slime can be made from.
const OLD_SLIME_SOURCE: u16 = 685;

/// Whether an NPC is one a fishing rod can bring up.
///
/// Six types, all blood-moon catches: the Red Slime that unlocks itself, two hardmode fish and
/// two pre-hardmode ones, and the Eyefish. Named rather than tabled — the set is tiny, has not
/// changed since 1.4.0, and `Projectile.FishingCheck_RollEnemySpawns` lists it in one place.
///
/// The check matters more than its size suggests. Without it, packet 130 is a free spawn of
/// anything in the game at any coordinates a client cares to name.
fn is_fishable(npc_type: u16) -> bool {
    matches!(npc_type, 586 | 587 | 618 | 620 | 621 | 682)
}

/// How often a resting item's true position is re-announced, and how many go out at a time.
///
/// Drift is slow, so this is deliberately lazy: a full sweep every tick would cost more than the
/// problem it fixes.
const ITEM_DRIFT_INTERVAL: u64 = 60 * 5;
const ITEMS_PER_SWEEP: usize = 16;

/// The Travelling Merchant, who is not a resident and has no house.
const TRAVELLING_MERCHANT: u16 = 368;
/// The Old Man at the dungeon door, who is not a resident either.
const OLD_MAN: u16 = 37;
/// ...nor is the Skeleton Merchant. Neither counts towards the two townsfolk the Travelling
/// Merchant wants to see before he will visit.
const SKELETON_MERCHANT: u16 = 453;
/// He only turns up in the first half of the day, and leaves at this hour.
const MERCHANT_ARRIVES_BEFORE: i32 = 27_000;
const MERCHANT_LEAVES_AT: i32 = 48_600;
/// The odds of his arriving on any given tick of the morning. `Main.UpdateTime`'s own figure.
const MERCHANT_ODDS: i32 = 27_000 * 4;
/// How many passes over the offer chain are made to fill his stock.
const MERCHANT_ROLLS: usize = 50;
/// How many slots the stock packet carries, whatever he actually has. `Main.TravelShopMaxSlots`.
const TRAVEL_SHOP_SLOTS: usize = 40;

/// One item that shimmer is about to break apart, and what into.
struct Decraft {
    index: i16,
    at: (f32, f32),
    recipe: &'static terrustia_proto::recipes::Recipe,
    /// How many whole batches came apart. A stack only decrafts in whole batches.
    batches: i32,
    /// What was left over and stays as it was.
    kept: i16,
}

/// The most of one thing a single dropped stack holds.
///
/// A stack is a signed short on the wire and nothing in the game stacks higher, so a decraft that
/// gives back more than this spreads across several piles rather than one impossible one.
const MAX_ITEM_STACK: i16 = 9_999;

// The two things packet 146 can announce: a transmutation's sparkle, and coins becoming luck.
const SHIMMER_EFFECT: u8 = 0;
const SHIMMER_COIN_LUCK: u8 = 1;

/// One world tile in pixels, for the arithmetic that turns a position into a tile.
const TILE_SIZE: f32 = crate::game::npc::TILE;

/// The Portal Gun's two ends, which are projectiles rather than tiles.
const PORTAL_PROJECTILE: u16 = 602;

/// Wire and actuators, as items, which is what a wiring tool spends.
const WIRE_ITEM: i16 = 530;
const ACTUATOR_ITEM: i16 = 849;

/// The longest drag a wiring tool will honour, in tiles.
///
/// Not the game's rule — the game has none, relying on the client's own tool range — which is
/// exactly why one is wanted here. A drag across a large world is a hundred thousand tile edits
/// broadcast to every client, which is a denial of service dressed up as a wiring tool.
const MAX_WIRE_DRAG: i32 = 512;

/// A gem lock, and how tall one frame of its sprite sheet is.
///
/// Whether it is locked lives in the frame rather than in a flag: the lower band of the sheet is
/// the locked form, so toggling one is a matter of moving all nine of its cells between bands.
const GEM_LOCK: u16 = 442;
const GEM_LOCK_FRAME_HEIGHT: i16 = 54;

/// How much of one item a player is carrying, across every slot.
fn count_held(player: &Player, item: i16) -> i32 {
    player
        .inventory
        .values()
        .filter(|slot| slot.item.id == i32::from(item))
        .map(|slot| i32::from(slot.item.stack))
        .sum()
}

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

/// A hostile NPC candidate for a town resident to fight back against: (slot, center, velocity).
type Hostile = (u8, (f32, f32), (f32, f32));

/// The nearest hostile a town resident can actually see, from among the candidates it might fight
/// back against.
///
/// Real vanilla (`AI_007_TownEntities`, `NPC.cs:54029-54101`) filters its own equivalent
/// nearest-candidate scan on `Collision.CanHit` *before* comparing distance at all
/// (`NPC.cs:54033`) — a closer hostile behind a wall never becomes a candidate to begin with, so a
/// farther one actually in view is who ends up selected. Filtering here before `min_by` is what
/// reproduces that: without it, a wall-blocked hostile nearer than an open one would always win the
/// distance comparison and — now that `try_combat` refuses to fire at a target it cannot see —
/// leave the resident with nothing it is willing to fight, for as long as the blocked hostile stays
/// alive and nearest.
fn nearest_visible_hostile(
    tiles: &impl TileView,
    npc: &crate::game::npc::Npc,
    hostiles: &[Hostile],
) -> Option<Target> {
    let here = npc.center();
    hostiles
        .iter()
        .filter(|h| {
            crate::game::ai::can_see(
                tiles,
                npc,
                Target {
                    slot: h.0,
                    center: h.1,
                    velocity: h.2,
                    alive: true,
                },
            )
        })
        .min_by(|a, b| {
            let da = (a.1.0 - here.0).powi(2) + (a.1.1 - here.1).powi(2);
            let db = (b.1.0 - here.0).powi(2) + (b.1.1 - here.1).powi(2);
            da.total_cmp(&db)
        })
        .map(|&(slot, center, velocity)| Target {
            slot,
            center,
            velocity,
            alive: true,
        })
}

#[cfg(test)]
mod nearest_visible_hostile_tests {
    use super::nearest_visible_hostile;
    use crate::game::npc::{Npc, TILE, TileView};
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Ground(HashMap<(i32, i32), Tile>);

    impl TileView for Ground {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn wall_at(tiles: &mut Ground, x: i32) {
        for y in 85..=110 {
            tiles.0.insert((x, y), Tile::block(1));
        }
    }

    fn merchant_at(tile_x: i32) -> Npc {
        let mut n = Npc::new(17, (0.0, 0.0), 1).expect("a real town npc type");
        n.position = (tile_x as f32 * TILE, 100.0 * TILE - n.height());
        n
    }

    /// Before this fix, the scan compared every candidate purely by distance — a wall between the
    /// resident and its nearest hostile never disqualified that hostile from being picked, it just
    /// meant `try_combat` would then refuse to fire at it, leaving a visible, fightable hostile
    /// close by completely ignored.
    #[test]
    fn a_farther_visible_hostile_is_picked_over_a_nearer_hidden_one() {
        let mut tiles = Ground::default();
        wall_at(&mut tiles, 203);
        let merchant = merchant_at(200);
        let hostiles = [
            // Nearer, but behind the wall at tile 203.
            (
                1u8,
                (merchant.center().0 + 60.0, merchant.center().1),
                (0.0, 0.0),
            ),
            // Farther, but with a clear line to it.
            (
                2u8,
                (merchant.center().0 - 150.0, merchant.center().1),
                (0.0, 0.0),
            ),
        ];
        let picked = nearest_visible_hostile(&tiles, &merchant, &hostiles)
            .expect("the farther, visible hostile should still be a valid candidate");
        assert_eq!(
            picked.slot, 2,
            "should have skipped the blocked, nearer one"
        );
    }

    #[test]
    fn the_nearest_hostile_is_picked_when_nothing_blocks_the_view() {
        let tiles = Ground::default();
        let merchant = merchant_at(200);
        let hostiles = [
            (
                1u8,
                (merchant.center().0 + 200.0, merchant.center().1),
                (0.0, 0.0),
            ),
            (
                2u8,
                (merchant.center().0 + 50.0, merchant.center().1),
                (0.0, 0.0),
            ),
        ];
        let picked = nearest_visible_hostile(&tiles, &merchant, &hostiles)
            .expect("an unobstructed hostile should be picked");
        assert_eq!(
            picked.slot, 2,
            "the ordinary unobstructed case: nearest wins"
        );
    }

    #[test]
    fn nothing_is_picked_when_every_candidate_is_hidden() {
        let mut tiles = Ground::default();
        wall_at(&mut tiles, 203);
        let merchant = merchant_at(200);
        let hostiles = [(
            1u8,
            (merchant.center().0 + 60.0, merchant.center().1),
            (0.0, 0.0),
        )];
        assert!(nearest_visible_hostile(&tiles, &merchant, &hostiles).is_none());
    }
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
    /// A line typed at the server's own console.
    ///
    /// Slot 255 is "the server" in chat, and the console is treated the same way: it owns the
    /// place unconditionally, because somebody with the terminal already has the world file.
    Console {
        line: String,
    },
    /// The console asking who could be tab-completed, without typing a command.
    ///
    /// Names only — a snapshot for a completion popup, not something that should ever gate on the
    /// game task being free. The console times its own wait out and falls back to no suggestions
    /// rather than stall a keypress on a busy tick.
    ConsoleContext {
        reply: oneshot::Sender<ConsoleContext>,
    },
    /// The web panel asking what it needs to resolve a login, for one account name.
    ///
    /// Never carries a password: argon2 is deliberately expensive (see
    /// `Admin::account_hash_and_group`'s own doc comment) and must run on the panel's own task,
    /// off the game task's tick — this only hands back what that verification needs.
    PanelAuthLookup {
        name: String,
        reply: oneshot::Sender<PanelAuthLookup>,
    },
    /// The web panel inserting an account it already hashed off the game task (claiming an
    /// unclaimed server). Cheap — no hashing happens here, only the push into the store.
    PanelInsertAccount {
        account: crate::admin::Account,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// The web panel asking for a snapshot to show on its status view.
    PanelStatus {
        reply: oneshot::Sender<PanelStatus>,
    },
    /// The web panel asking who is connected, for the player list and the live world view.
    PanelPlayers {
        reply: oneshot::Sender<Vec<PanelPlayer>>,
    },
    /// The web panel's kick button. Reuses exactly what `/kick` and the console's `kick` already
    /// call ([`Self::kick`]/[`Self::announce`] by way of `run_admin_command`'s own logic) rather
    /// than a second copy of it.
    PanelKick {
        name: String,
        reason: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// The web panel's ban button. Same reasoning as `PanelKick`.
    PanelBan {
        kind: crate::admin::BanKind,
        value: String,
        reason: String,
        reply: oneshot::Sender<()>,
    },
    PanelUnban {
        value: String,
        reply: oneshot::Sender<usize>,
    },
    /// The web panel asking who is on the guest list, and whether it is currently in force.
    PanelWhitelist {
        reply: oneshot::Sender<PanelWhitelist>,
    },
    PanelWhitelistAdd {
        name: String,
        reply: oneshot::Sender<bool>,
    },
    PanelWhitelistRemove {
        name: String,
        reply: oneshot::Sender<bool>,
    },
    /// A coarse, sampled view of the world's tiles, for the panel's live world screen. Sampled
    /// rather than exhaustive — see [`Self::world_tile_sample`]'s own doc comment for why a full
    /// tile-for-tile dump is neither necessary nor safe to send over a websocket on every tick.
    PanelWorldTiles {
        reply: oneshot::Sender<PanelWorldTiles>,
    },
    /// A read-only snapshot of the running configuration, for the panel's settings view.
    PanelConfigSnapshot {
        reply: oneshot::Sender<PanelConfigSnapshot>,
    },
    /// The one setting the panel is allowed to change live: the MOTD is read fresh out of
    /// `Config` every time a player spawns (see `on_player_spawn`), so writing a new one here takes
    /// effect for the very next join with nothing else to coordinate.
    PanelSetMotd {
        motd: String,
        reply: oneshot::Sender<()>,
    },
    /// Ask to restart the process pointed at a different world file. `path` has already been
    /// resolved and validated against `crate::worlds::list()` by the panel — this only checks it
    /// still exists (it could have been removed between the listing and the click) and, if so, sets
    /// `stopping` so the ordinary shutdown save runs before `main` re-execs.
    PanelSwitchWorld {
        path: PathBuf,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// One connected player, as the panel needs to show them: who they are, how they are doing, where
/// they are, and — for the live world view — enough of their real appearance data to draw a
/// stylized avatar rather than a sprite. See `panel/mod.rs`'s module doc for why nothing here is
/// (or ever will be) a composited Terraria asset.
#[derive(Debug, Clone)]
pub struct PanelPlayer {
    pub slot: u8,
    pub name: String,
    pub address: String,
    pub life: i16,
    pub life_max: i16,
    pub mana: i16,
    pub mana_max: i16,
    pub position: (f32, f32),
    pub pvp: bool,
    pub appearance: Option<terrustia_proto::player_info::PlayerAppearance>,
    /// Non-zero item ids currently worn in the armour/accessory slots (`inventory.rs`'s
    /// `SLOT_RUNS` run 2, slots 59..79) — real equipped gear, not decoration invented for the
    /// avatar.
    pub equipped: Vec<i32>,
}

/// What [`ServerEvent::PanelWhitelist`] hands back.
#[derive(Debug, Clone, Default)]
pub struct PanelWhitelist {
    pub on: bool,
    pub names: Vec<String>,
}

/// A coarse sample of the world's tiles for the panel's live world screen: one colour bucket per
/// sample point on a fixed-size grid, regardless of how large the actual world is. See
/// [`GameServer::world_tile_sample`] for how the grid is chosen and why sampling — not a full
/// tile dump — is the honest way to stream this over a websocket.
#[derive(Debug, Clone)]
pub struct PanelWorldTiles {
    pub world_width: i32,
    pub world_height: i32,
    pub sample_cols: u32,
    pub sample_rows: u32,
    /// `sample_cols * sample_rows` colour buckets, row-major, one per sample point.
    pub tiles: Vec<TileColor>,
}

/// A tile's colour bucket, for the panel's stylized (not sprite-accurate) world render. See
/// [`GameServer::tile_color`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileColor {
    /// No active tile — open sky, or an unlit cave; the sample carries no depth information to
    /// tell those apart, so it does not pretend to.
    Empty,
    Dirt,
    Stone,
    Grass,
    Corruption,
    Crimson,
    Sand,
    Snow,
    Ice,
    Jungle,
    Ore,
    Gem,
    Water,
    Lava,
    Honey,
    Ash,
    /// Anything active with no bucket of its own — built structures, furniture, and the many tile
    /// ids this sampler does not have a named constant for.
    Other,
}

/// A read-only snapshot of [`Config`] for the panel's settings view. Never carries the actual
/// server password — only whether one is set.
#[derive(Debug, Clone)]
pub struct PanelConfigSnapshot {
    pub listen: SocketAddr,
    pub max_players: usize,
    pub world_width: i32,
    pub world_height: i32,
    pub motd: String,
    pub password_set: bool,
    pub max_chat_len: usize,
    pub idle_timeout_secs: u64,
    pub autosave_secs: u64,
    pub save_target: Option<String>,
    pub whitelist_on: bool,
    pub whitelist_count: usize,
}

/// What [`ServerEvent::PanelAuthLookup`] hands back.
#[derive(Debug, Clone, Default)]
pub struct PanelAuthLookup {
    pub unclaimed: bool,
    /// The currently-active one-time claim token, if the server is unclaimed and one has been
    /// printed to the console. `None` means either the server is claimed, or nobody has connected
    /// yet to trigger `announce_claim_token`.
    pub claim_token: Option<String>,
    /// `(hash, group)` for the named account, if one exists.
    pub hash_and_group: Option<(String, String)>,
}

/// What [`ServerEvent::PanelStatus`] hands back.
#[derive(Debug, Clone, Default)]
pub struct PanelStatus {
    pub player_count: usize,
    pub max_players: usize,
    pub world_name: String,
    /// The file stem of the world currently being served, if it has one, so the panel's world
    /// list can mark which entry is the running one.
    pub world_file: Option<String>,
}

/// What the console can offer to complete a command's second argument with.
#[derive(Debug, Default, Clone)]
pub struct ConsoleContext {
    pub players: Vec<String>,
    pub groups: Vec<String>,
}

/// How the game loop ended.
///
/// This exists so the process can exit with a code that means something. A panicked game task used
/// to look identical to a clean stop from outside — `main` returned `Ok(())` either way — so a
/// server that had crashed exited 0, and every supervisor configured with
/// `Restart=on-failure` (or a Docker restart policy keyed on exit status) left it dead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Stopped {
    /// Asked to stop, and did.
    Cleanly,
    /// Something panicked. The world was still saved on the way out.
    Panicked,
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
    /// How much of the world is hallow, corruption and crimson. One column is counted per tick,
    /// as the game does, and the figures go out in packet 57 when a sweep completes.
    census: crate::world::census::Census,
    /// Which network each announced pylon belongs to, by position.
    ///
    /// Remembered rather than re-derived, because a pylon's network lives in its *tile's* frame
    /// and mining the tile is what removes the entity — so by the time there is a removal to
    /// announce, the frame is gone. The client matches a removal on position **and** type
    /// (`TeleportPylonInfo.Equals`), so getting the type wrong there does not remove anything: the
    /// pylon stays on every travel map for the rest of the session.
    pylon_kinds: HashMap<(i16, i16), u8>,
    /// How many syncs in a row each NPC has been withheld from each player.
    npc_skips: HashMap<(u8, u8), u8>,
    /// How much nearness each player has accumulated towards the next streamed update of each NPC.
    npc_stream: HashMap<(u8, u8), u8>,
    /// Whose turn it is to have the ground around them searched for a house.
    housing_turn: usize,
    /// Timers that are switched on, and how long each has left in its window.
    running_timers: HashMap<(i32, i32), i32>,
    /// Trap tiles that have fired recently, and how long until each can fire again.
    ///
    /// The game keeps the same list, capped at 999 entries; this one is a map because looking a
    /// tile up is what it is for.
    mech_cooldown: HashMap<(i32, i32), i32>,
    /// A save being written on another thread, if one is.
    ///
    /// Kept so that shutdown can wait for it and so that two never run at once.
    saving: Option<tokio::task::JoinHandle<()>>,
    /// Why the save in flight was started, so the right thing is said when it finishes.
    save_reason: &'static str,
    /// How a finished background save reports back to the game task.
    ///
    /// A channel rather than the join handle because the tick is not async and polling a handle
    /// for its value needs an executor; a try-receive needs nothing.
    save_results: (SaveReport, SaveReports),
    /// Password hashing that finished on a worker thread, waiting to be applied.
    ///
    /// Argon2 costs tens of milliseconds by design, against a tick budget of 16.67. Running it
    /// inline — which `/register` and `/login` used to do, with no permission check and no rate
    /// limit — meant any connected client could freeze the world for everybody simply by sending
    /// `/register` in a loop. The hash now happens off the game task and its answer arrives here,
    /// the same shape the background save already uses.
    auth_results: (AuthReport, AuthReports),
    /// Slots with a hash already running. One at a time, so nobody can queue up work.
    auth_in_flight: std::collections::HashSet<u8>,
    /// A one-time secret for claiming an unclaimed server, printed to the console at startup.
    ///
    /// Without this, the first account made owns the server — so on a fresh public server whoever
    /// connected first became the owner, and *everyone* had every permission until they did.
    /// That is fine among friends and a gift to a stranger.
    ///
    /// Requiring the token means claiming needs someone who can read the server's own terminal,
    /// which is the same trust boundary the console already sits behind: whoever has the terminal
    /// already has the world file. Cleared once the server is claimed.
    claim_token: Option<String>,
    /// A world-sized buffer to copy the next snapshot into, once a save has given one back.
    ///
    /// Held so the copy writes into pages that are already mapped. Empty while a save is running,
    /// and empty on the first save of a session, which is the one that pays for the mapping.
    spare_world: Option<crate::world::World>,
    /// Finished saves handing their buffer back.
    world_returns: (
        std::sync::mpsc::Sender<crate::world::World>,
        std::sync::mpsc::Receiver<crate::world::World>,
    ),
    /// The six cavern enemies this world happens to have.
    ///
    /// Drawn from the world's id rather than from the run's generator, so the same world always
    /// has the same six. Worked out once when the world is opened.
    cavern_monsters: crate::game::cavern_monsters::CavernMonsters,
    /// The last pillar shields, invasion progress and Moon Lord countdown that went out.
    ///
    /// All three are recomputed every tick and almost never move, so they are compared before
    /// they are sent. Broadcasting them unconditionally would be three packets a tick to every
    /// client for the whole of an event.
    last_sent_shields: [i32; 4],
    last_sent_countdown: i32,
    last_sent_invasion: (i32, i32),
    /// What the Travelling Merchant is carrying, if he is here.
    ///
    /// Rolled on arrival and thrown away when he leaves. Not world state: he takes his stock
    /// with him, so a world reloaded mid-visit simply has no merchant.
    travel_shop: Vec<u16>,
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
    /// Who may do what, and who is kept out. Read from disk beside the world.
    admin: crate::admin::Admin,
    /// Set by the console's `stop`, so the loop ends and the world is saved on the way out.
    stopping: bool,
    worst_tick: TickCost,
    /// The longest a tick has been held off the processor this window.
    worst_stall: Duration,
    /// A trailing, time-windowed log of player tile edits, for `/world undo`. See
    /// [`crate::game::tile_log`]'s own module doc for retention and scope.
    tile_log: crate::game::tile_log::TileLog,
    /// The other end of [`crate::panel::supervise`]'s toggle channel, for the console's `panel`
    /// command. `None` in every test that never calls [`Self::with_panel_toggle`] — the console
    /// command still works in that case, it just has nothing to notify and says so.
    panel_toggle: Option<mpsc::UnboundedSender<()>>,
    /// Set by [`ServerEvent::PanelSwitchWorld`], and read by `main` after [`Self::run`] returns.
    ///
    /// Switching worlds is not something a single running process can do to its own in-memory
    /// [`crate::world::World`] safely — every system in this file, from `section_cache` to
    /// `cavern_monsters`, is sized and seeded for the world that was loaded at startup. The honest
    /// version is a full graceful restart pointed at a different save file: this field is how the
    /// request crosses from the game task (which decides *whether* to stop, by setting
    /// `stopping`) to `main` (which is the only thing that can actually replace the process), once
    /// `run` has already saved the current world and returned.
    ///
    /// An `Arc<Mutex<..>>` rather than a channel because `main` needs to read it *after* `self` has
    /// been moved into the spawned task and consumed by `run` — a handle cloned out before that
    /// move is the only way to still have an opinion once the task is gone.
    pending_world_switch: Arc<Mutex<Option<PathBuf>>>,
    /// Journey mode's shared toggles. See [`crate::game::journey`]'s own module doc for which
    /// powers this covers and, just as importantly, which fifteen-minus-eleven it does not yet.
    journey: crate::game::journey::JourneyPowers,
    /// Set by `main`'s background `update::boot_check` task once it finds a newer, signature-
    /// verified release; taken (delivered, then cleared) the first time a recognised admin signs
    /// in afterward, in [`Self::note_finished_auth`]. `None` in every test that never calls
    /// [`Self::with_update_notice`] — the sign-in path still works in that case, it just has
    /// nothing to hand over. An `Arc<Mutex<..>>` for the same reason `pending_world_switch` above
    /// is one: `main` needs to keep writing to it from outside after `self` is moved into the
    /// spawned game task, so a handle cloned out before that move is the only way in.
    update_notice: Option<Arc<Mutex<Option<String>>>>,
    /// The birthday party — see [`crate::game::party`]'s own module doc.
    party: crate::game::party::PartyState,
    /// Slime Rain — see [`crate::game::slime_rain`]'s own module doc.
    slime_rain: crate::game::slime_rain::SlimeRainState,
    /// Lantern Night — see [`crate::game::lantern_night`]'s own module doc.
    lantern_night: crate::game::lantern_night::LanternNightState,
}

impl GameServer {
    pub fn new(config: Config, mut world: World) -> Self {
        // From here on, tile edits invalidate cached sections.
        world.start_tracking_changes();
        // A full snapshot now, while startup has no tick budget to blow, so the *first* real
        // autosave has a baseline to diff against instead of paying for a full copy inside a
        // counted tick. Measured on a real CI soak run before this existed: 14,833 µs — 89% of a
        // single tick's 16,666 µs budget, on the very first save after the server came up. Every
        // save after it was already 150–200 µs, because `refresh_snapshot` had something to
        // compare against; this just moves that same first comparison off the clock entirely.
        let spare_world = Some(world.snapshot());
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

        // Read before the world is moved into the server, since the monsters are drawn from it.
        let world_id = world.id;
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
            census: crate::world::census::Census::new(terrustia_proto::tile_sets::TILE_COUNT),
            pylon_kinds: HashMap::new(),
            npc_skips: HashMap::new(),
            npc_stream: HashMap::new(),
            housing_turn: 0,
            running_timers,
            mech_cooldown: HashMap::new(),
            tile_entity_anchors: HashMap::new(),
            saving: None,
            save_reason: "",
            save_results: std::sync::mpsc::channel(),
            auth_results: std::sync::mpsc::channel(),
            auth_in_flight: std::collections::HashSet::new(),
            claim_token: None,
            spare_world,
            world_returns: std::sync::mpsc::channel(),
            cavern_monsters: crate::game::cavern_monsters::CavernMonsters::for_world(world_id),
            // Deliberately impossible starting values, so the first tick of each always sends.
            last_sent_shields: [-1; 4],
            last_sent_countdown: -1,
            last_sent_invasion: (-1, -1),
            travel_shop: Vec::new(),
            angler_quest: 0,
            angler_finished_today: std::collections::HashSet::new(),
            liquids: crate::world::liquid::Liquids::default(),
            // Beside the world it belongs to. A world with nowhere to save has nowhere to put
            // this either, and should not scatter one into whatever directory it was started in.
            admin: match &save_path {
                Some(path) => crate::admin::Admin::load(&path.with_extension("admin.toml")),
                None => crate::admin::Admin::in_memory(),
            },
            stopping: false,
            worst_tick: TickCost::default(),
            worst_stall: Duration::ZERO,
            save_path,
            autosave_ticks,
            tile_log: crate::game::tile_log::TileLog::default(),
            panel_toggle: None,
            pending_world_switch: Arc::new(Mutex::new(None)),
            journey: crate::game::journey::JourneyPowers::default(),
            update_notice: None,
            party: crate::game::party::PartyState::default(),
            slime_rain: crate::game::slime_rain::SlimeRainState::default(),
            lantern_night: crate::game::lantern_night::LanternNightState::default(),
        };
        // The Angler wants something from the moment the world opens, not from the first dawn.
        // A server that waited would give the first day's players nothing to do for him.
        server.roll_angler_quest();
        // Count the world once up front. The per-tick sweep would otherwise take a full minute of
        // play to produce its first figures, and the Dryad would tell everyone who joined in that
        // minute that their world was nought per cent of everything.
        server.census.sweep(&server.world);
        // Remember every pylon's network up front, rather than waiting for the first join to fill
        // the map in. Otherwise a pylon mined before anybody had connected would be announced with
        // the wrong network and stay on every travel map afterwards.
        for pylon in server.pylons() {
            server.pylon_kinds.insert((pylon.x, pylon.y), pylon.kind);
        }
        server
    }

    /// Wires the console's `panel` command up to a running [`crate::panel::supervise`] task.
    /// Builder-style rather than a `new()` parameter: every existing call site (`main`, and
    /// seventeen call sites across this workspace's tests) would otherwise need a channel most of
    /// them have no use for, just to construct a server at all.
    pub fn with_panel_toggle(mut self, toggle: mpsc::UnboundedSender<()>) -> Self {
        self.panel_toggle = Some(toggle);
        self
    }

    /// Wires up the handle `main`'s background `update::boot_check` task writes a message into
    /// once it finds a newer, signature-verified release. Builder-style for the same reason as
    /// [`Self::with_panel_toggle`] just above — most call sites have no use for one.
    pub fn with_update_notice(mut self, notice: Arc<Mutex<Option<String>>>) -> Self {
        self.update_notice = Some(notice);
        self
    }

    /// A handle to whatever [`ServerEvent::PanelSwitchWorld`] requests, for `main` to read once
    /// [`Self::run`] has returned. Call this before `run` consumes `self` — cloning an `Arc` out of
    /// a value about to be moved is the whole trick.
    pub fn world_switch_handle(&self) -> Arc<Mutex<Option<PathBuf>>> {
        self.pending_world_switch.clone()
    }

    /// Every pylon in the world, as module 8 describes them.
    ///
    /// A pylon keeps nothing of its own — which network it belongs to is the tile's frame — so the
    /// kind is read back off the tile rather than stored beside the entity, which is also what
    /// keeps the two from ever disagreeing.
    fn pylons(&self) -> Vec<net_module::Pylon> {
        use terrustia_proto::tile_entity::EntityKind;

        self.world
            .tile_entities
            .iter()
            .filter(|entity| entity.kind == EntityKind::TeleportationPylon)
            .filter_map(|entity| {
                let tile = self.world.tile(i32::from(entity.x), i32::from(entity.y));
                if !tile.is_active() || tile.block != EntityKind::TeleportationPylon.tile() {
                    // The entity outlived its tile. Not worth announcing: the client would draw a
                    // travel destination that is not there.
                    return None;
                }
                Some(net_module::Pylon {
                    x: entity.x,
                    y: entity.y,
                    // A pylon is three tiles wide, so each style occupies 54 pixels of frame.
                    kind: (tile.frame_x / PYLON_FRAME_WIDTH) as u8,
                })
            })
            .collect()
    }

    /// Tell one player about a pylon appearing or vanishing.
    fn broadcast_pylon(&mut self, message: net_module::PylonMessage, pylon: net_module::Pylon) {
        if let Ok(frame) = net_module::pylon_message(message, pylon) {
            self.broadcast(frame, None);
        }
    }

    /// A client asking to be taken to a pylon.
    ///
    /// The game's rules, in the order it checks them: you have to be standing near a pylon, and the
    /// one you are going to needs two townsfolk living within its scan box — except the Victory
    /// pylon, which needs none. The biome requirement is *not* enforced here, because deciding
    /// whether a stretch of ground counts as a jungle needs `SceneMetrics`, which this server does
    /// not have; the effect is that a pylon planted in the wrong biome still works. That is a
    /// permissive difference rather than a broken one, and it is written down rather than hidden.
    fn on_pylon_teleport(
        &mut self,
        slot: u8,
        pylon: net_module::Pylon,
    ) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let known = self.pylons();
        let Some(&destination) = known.iter().find(|p| p.x == pylon.x && p.y == pylon.y) else {
            debug!(slot, x = pylon.x, y = pylon.y, "no pylon there");
            return Ok(());
        };

        let Some(player) = self.player(slot) else {
            return Ok(());
        };
        let at = (player.position.0 / 16.0, player.position.1 / 16.0);
        if !known.iter().any(|p| {
            (f32::from(p.x) - at.0).abs() <= PYLON_REACH
                && (f32::from(p.y) - at.1).abs() <= PYLON_REACH
        }) {
            self.tell(slot, "You need to be near a pylon to travel.");
            return Ok(());
        }

        if destination.kind != net_module::Pylon::VICTORY
            && self.town_npcs_near(destination.x, destination.y) < PYLON_RESIDENTS_NEEDED
        {
            self.tell(
                slot,
                "That pylon needs two townsfolk living near it before it will work.",
            );
            return Ok(());
        }

        // Land on the pylon's own tile, as the game does.
        let to = (
            f32::from(destination.x) * 16.0,
            f32::from(destination.y) * 16.0,
        );
        if let Some(player) = self.player_mut(slot) {
            player.position = to;
            player.velocity = (0.0, 0.0);
        }
        // Style 9 is the pylon's own animation; the extra value picks the colour by network.
        let mut w = terrustia_proto::PacketWriter::new(id::TELEPORT_ENTITY);
        w.u8(0x08) // the fourth bit says an extra value follows
            .i16(i16::from(slot))
            .f32(to.0)
            .f32(to.1)
            .u8(9)
            .i32(i32::from(destination.kind));
        let frame = w.finish()?;
        self.broadcast(frame, None);
        debug!(slot, x = destination.x, y = destination.y, "pylon travel");
        Ok(())
    }

    /// How many housed town NPCs live within a pylon's scan box.
    fn town_npcs_near(&self, x: i16, y: i16) -> usize {
        self.npcs
            .iter()
            .filter(|(_, npc)| npc.stats.town_npc && npc.is_alive())
            .filter(|(_, npc)| {
                let Some((hx, hy)) = npc.home else {
                    // Homeless townsfolk do not count towards a pylon, in the game or here.
                    return false;
                };
                (hx - i32::from(x)).abs() <= PYLON_SCAN_HALF_WIDTH
                    && (hy - i32::from(y)).abs() <= PYLON_SCAN_HALF_HEIGHT
            })
            .count()
    }

    /// The whole banner kill table, as module 11 message 0.
    fn banner_state_frame(&self) -> terrustia_proto::Result<Vec<u8>> {
        let mut kills = [0u32; net_module::BANNER_SLOTS];
        for (&banner, &count) in &self.world.banner_kills {
            if let Some(slot) = kills.get_mut(usize::from(banner)) {
                *slot = count;
            }
        }
        // Nothing is ever claimable here: a banner is dropped where the kill happened rather than
        // held for collection, so there is never a pending one to report.
        let claimable = [0u16; net_module::BANNER_SLOTS];
        net_module::banners_full_state(&kills, &claimable)
    }

    /// One packet 60 per town NPC, saying where it lives.
    ///
    /// A homeless one still gets a frame — the game sends its home tile anyway and flags it as
    /// homeless, which is how the client knows to draw the "no home" marker rather than nothing.
    fn npc_home_frames(&self) -> Vec<Vec<u8>> {
        self.npcs
            .iter()
            .filter(|(_, npc)| npc.stats.town_npc && npc.is_alive())
            .filter_map(|(index, _)| self.npc_home_frame(index))
            .collect()
    }

    /// One packet 60 for one town NPC.
    fn npc_home_frame(&self, index: u8) -> Option<Vec<u8>> {
        let npc = self.npcs.get(index)?;
        let (home, status) = match npc.home {
            // The game distinguishes "has a home tile" from "has a room the game agrees is
            // habitable"; this server only tracks the first, so a housed NPC reports `Settled`
            // rather than claiming a room record it does not keep.
            Some(home) => (home, packets::HouseholdStatus::Settled),
            // A homeless one still reports a position — wherever it happens to be standing — which
            // is what the game sends for an NPC that has been evicted.
            None => (
                (
                    (npc.position.0 / 16.0) as i32,
                    (npc.position.1 / 16.0) as i32,
                ),
                packets::HouseholdStatus::Homeless,
            ),
        };
        packets::npc_home(u16::from(index), home.0 as i16, home.1 as i16, status).ok()
    }

    /// Tell every client where one town NPC lives, after it moves in or is evicted.
    fn broadcast_npc_home(&mut self, index: u8) {
        if let Some(frame) = self.npc_home_frame(index) {
            self.broadcast(frame, None);
        }
    }

    /// Advance the world census by one column, and publish it when a sweep completes.
    fn tick_census(&mut self) {
        self.census.tick(&self.world);

        if self.census.just_finished
            && let Ok(frame) = packets::world_evil_tally(
                self.census.percent_hallow,
                self.census.percent_corrupt,
                self.census.percent_crimson,
            )
        {
            self.broadcast(frame, None);
        }
    }

    /// Write the world to disk, announcing the outcome in chat.
    ///
    /// Serialisation runs on the game task because it needs exclusive access to the world; it takes
    /// a fraction of a second even for a full-size world, which is why it is not worth the cost of
    /// snapshotting eighty megabytes of tiles to move it off-thread.
    /// Write the world to disk, blocking the game task until it is done.
    ///
    /// Only for shutdown, where there is no next tick to protect and the process must not exit
    /// before the bytes are on disk. Everything else wants [`Self::save_world_in_background`].
    fn save_world(&mut self, reason: &str) {
        let Some(path) = self.save_path.clone() else {
            return;
        };
        self.record_town_npcs();
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

    /// Take a copy of the world and let another thread write it out.
    ///
    /// Serialising a large world costs about fifty-five milliseconds against a tick budget of
    /// sixteen and a half, so an autosave on the game task drops three or four ticks — a visible
    /// stutter every five minutes, and worse the bigger the world. Measured with
    /// `examples/savecost.rs`; the cost is in run-detection and encoding rather than in reading
    /// the tiles, which is why making the reads cache-friendlier did nothing.
    ///
    /// The copy costs the tick a few milliseconds of memcpy and nothing else, and it is atomic
    /// with respect to the tick, so the writer can never catch the world halfway through an edit.
    ///
    /// One save at a time. If a previous one is still going the new request is dropped rather
    /// than queued: two saves racing for the same path is worse than a missed autosave, and a
    /// server whose disk cannot keep up with its autosave interval should not build a backlog of
    /// sixty-megabyte snapshots waiting for it.
    fn save_world_in_background(&mut self, reason: &'static str) {
        let Some(path) = self.save_path.clone() else {
            return;
        };
        if let Some(running) = &self.saving
            && !running.is_finished()
        {
            warn!(reason, "a save is still running; skipping this one");
            return;
        }

        // The roster has to reach the world before the snapshot is taken, or the copy that goes to
        // disk holds whoever lived here when it was loaded rather than who lives here now.
        self.record_town_npcs();

        // Copy into a buffer we already own where we have one. A fresh `snapshot()` asks the
        // allocator for a new forty-megabyte mapping and then faults in every page of it as it
        // writes: measured on a 4200x1200 world, 2.600 ms against 0.989 ms for copying into a
        // buffer whose pages are already mapped. That is the difference between roughly a sixth
        // of the tick budget and a sixteenth, four times worse again on a large world, and it is
        // the single most expensive thing an idle server does.
        // Copying forty megabytes of tiles is the most expensive thing an idle server does, and on
        // a world nobody is digging through, almost none of those tiles have changed since the last
        // save. The buffer a finished save hands back already holds that state, so only the
        // sections that have changed since need copying into it.
        let began = Instant::now();
        let (mut snapshot, sections) = match self.spare_world.take() {
            Some(mut spare) if self.world.snapshot_is_incremental() => {
                let sections = self.world.refresh_snapshot(&mut spare);
                (spare, Some(sections))
            }
            // Either the first save of the session, or a world still being generated or loaded,
            // where nothing has been tracking changes.
            Some(mut spare) => {
                spare.copy_state_from(&self.world);
                (spare, None)
            }
            None => (self.world.snapshot(), None),
        };
        snapshot.shrink_caches();
        let copied = began.elapsed();

        let report = self.save_results.0.clone();
        // The writer hands the buffer back when it is finished with it, so the next save can
        // reuse it instead of asking for another mapping.
        let returned = self.world_returns.0.clone();
        self.saving = Some(tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let outcome = match wld_save::save(&snapshot, &path) {
                Ok(()) => {
                    let ms = started.elapsed().as_millis() as u64;
                    info!(path = %path.display(), reason, elapsed_ms = ms, "world saved");
                    Ok(ms)
                }
                Err(e) => {
                    error!(path = %path.display(), error = %e, "world save failed");
                    Err(())
                }
            };
            // A closed channel means the server is already shutting down, which is not an error.
            let _ = report.send(outcome);
            // Hand the buffer back for the next save to write into. If nobody is listening the
            // server is stopping and it simply drops here.
            let _ = returned.send(snapshot);
        }));
        self.save_reason = reason;
        debug!(
            reason,
            snapshot_us = copied.as_micros() as u64,
            // `None` means the whole world was copied. A number suddenly equal to every section in
            // the world means the incremental path has quietly stopped working.
            sections_copied = sections,
            "world snapshot taken; saving in the background"
        );
    }

    /// Start hashing a new account's password, once everything cheap has been checked.
    ///
    /// `owner` says this account claims the server. It is decided by the caller rather than
    /// re-derived here, because the two callers disagree about what earns it: from chat it takes
    /// the console's claim token, and from the console it takes nothing at all.
    fn begin_registration(&mut self, slot: u8, account: &str, password: &str, owner: bool) {
        // Everything decidable without hashing is decided first, so a bad request costs nothing.
        if self.admin.name_taken(account) {
            self.tell(
                slot,
                &format!("there is already an account called {account}"),
            );
            return;
        }
        if password.len() < 6 {
            self.tell(
                slot,
                "that password is too short; use at least six characters",
            );
            return;
        }
        if !self.start_auth(slot) {
            return;
        }
        let group = if owner { "owner" } else { "default" }.to_string();
        let (account, password) = (account.to_string(), password.to_string());
        let report = self.auth_results.0.clone();
        tokio::task::spawn_blocking(move || {
            let made = crate::admin::Account::new(&account, &password, &group);
            let _ = report.send(AuthOutcome::Registered {
                slot,
                account: Box::new(made),
                first: owner,
            });
        });
    }

    /// List the world backups on disk, newest first.
    fn list_backups(&mut self) {
        let Some(path) = self.save_path.clone() else {
            info!("this world is not being saved, so there is nothing to roll back to");
            return;
        };
        let mut found = 0;
        for n in 1..=crate::world::wld_save::BACKUPS_KEPT {
            let bak = path.with_extension(format!("wld.bak{n}"));
            let Ok(meta) = std::fs::metadata(&bak) else {
                continue;
            };
            let age = meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map_or_else(
                    || "unknown".to_string(),
                    |d| format!("{}m ago", d.as_secs() / 60),
                );
            info!(
                backup = n,
                size_mb = meta.len() / 1_048_576,
                age,
                path = %bak.display(),
                "backup"
            );
            found += 1;
        }
        if found == 0 {
            info!("no backups yet; one is made each time the world is saved");
        } else {
            info!("roll one back with:  rollback <n>   (the server stops afterwards)");
        }
    }

    /// Put a backup back, and stop so it is loaded cleanly on the next start.
    ///
    /// Deliberately *not* a hot swap. Replacing the world under a running server would leave every
    /// connected client holding tiles that no longer exist, the NPC roster pointing at houses that
    /// moved, and the next autosave writing the in-memory world straight back over the backup that
    /// was just restored — undoing the rollback within five minutes. Stopping is honest and takes
    /// one restart.
    fn roll_back(&mut self, which: usize) {
        let Some(path) = self.save_path.clone() else {
            info!("this world is not being saved, so there is nothing to roll back to");
            return;
        };
        if which == 0 || which > crate::world::wld_save::BACKUPS_KEPT {
            info!(
                "there are only {} backups; use `rollback 1` for the most recent",
                crate::world::wld_save::BACKUPS_KEPT
            );
            return;
        }
        let bak = path.with_extension(format!("wld.bak{which}"));
        if !bak.exists() {
            info!(path = %bak.display(), "no such backup");
            return;
        }
        // Check it before trusting it. Restoring an unreadable file over a readable one would turn
        // a rollback into the very thing it exists to undo.
        match std::fs::read(&bak)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                crate::world::wld::parse(&bytes)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }) {
            Ok(()) => {}
            Err(e) => {
                error!(path = %bak.display(), error = %e, "that backup will not load; refusing");
                return;
            }
        }
        // Keep what is being replaced, so a rollback is itself reversible.
        let aside = path.with_extension("wld.before-rollback");
        if path.exists()
            && let Err(e) = std::fs::rename(&path, &aside)
        {
            error!(error = %e, "could not move the current world aside; refusing to roll back");
            return;
        }
        if let Err(e) = std::fs::copy(&bak, &path) {
            error!(error = %e, "could not restore the backup");
            let _ = std::fs::rename(&aside, &path);
            return;
        }
        self.announce("The world is being rolled back; the server is stopping.");
        info!(
            backup = which,
            replaced = %aside.display(),
            "world rolled back; stopping so it loads cleanly on the next start"
        );
        // The in-memory world must not be written over what was just restored.
        self.save_path = None;
        self.stopping = true;
    }

    /// Print the one-time claim token, if this server has not been claimed yet.
    ///
    /// Only ever to the log, which means the terminal or the service journal — never to a player.
    /// The whole point is that claiming requires someone who can see the server's own output.
    fn announce_claim_token(&mut self) {
        if !self.admin.unclaimed() {
            return;
        }
        // Not a password: a short one-time secret that lives for one process. Derived from the
        // clock rather than a CSPRNG dependency, mixed so it is not simply readable back as a
        // timestamp. It only has to be unguessable by someone who cannot see this line.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let mut state = now ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut token = String::new();
        for _ in 0..12 {
            // xorshift64*, which is plenty for a value that is printed and then used once.
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let alphabet = b"abcdefghjkmnpqrstuvwxyz23456789";
            token.push(alphabet[(state % alphabet.len() as u64) as usize] as char);
        }
        warn!(
            "this server has no accounts yet, so everyone connecting has every permission. \
             claim it with:  /register <name> <password> {token}   (or `claim <name> <password>` \
             here at the console)"
        );
        self.claim_token = Some(token);
    }

    /// Claim a slot's one allowed password hash, or explain why not.
    ///
    /// Two limits, and both are needed. Per-slot exclusivity stops one client queueing thousands
    /// of hashes; the server-wide ceiling stops two hundred and fifty-five clients each queueing
    /// one. Without them `/register` — which needs no permission at all — was a way for anybody
    /// who could join to stall the world for everybody else.
    fn start_auth(&mut self, slot: u8) -> bool {
        if self.auth_in_flight.contains(&slot) {
            self.tell(slot, "still checking your last one; wait a moment.");
            return false;
        }
        if self.auth_in_flight.len() >= MAX_AUTH_IN_FLIGHT {
            self.tell(
                slot,
                "the server is busy checking passwords; try again shortly.",
            );
            return false;
        }
        self.auth_in_flight.insert(slot);
        true
    }

    /// Apply any password hashing that finished since the last tick.
    ///
    /// Polled rather than awaited, for the same reason the save report is: the tick is not async
    /// and should not become so for this.
    /// Reclaim the snapshot buffer from a save that has finished with it.
    fn reclaim_snapshot_buffer(&mut self) {
        if let Ok(spare) = self.world_returns.1.try_recv() {
            self.spare_world = Some(spare);
        }
    }

    fn note_finished_auth(&mut self) {
        while let Ok(outcome) = self.auth_results.1.try_recv() {
            match outcome {
                AuthOutcome::Registered {
                    slot,
                    account,
                    first,
                } => {
                    self.auth_in_flight.remove(&slot);
                    match *account {
                        Ok(made) => {
                            let (name, group) = (made.name.clone(), made.group.clone());
                            match self.admin.insert_account(made) {
                                Ok(()) => {
                                    let _ = self.admin.save();
                                    self.tell(slot, &format!("account '{name}' made ({group})."));
                                    if first {
                                        // Spent. A second claim must not be possible.
                                        self.claim_token = None;
                                        self.tell(
                                            slot,
                                            "you are the first account here, so you own it.",
                                        );
                                    }
                                    info!(account = %name, group = %group, "account registered");
                                }
                                // Somebody else took the name while this was hashing.
                                Err(e) => self.tell(slot, &e),
                            }
                        }
                        Err(e) => self.tell(slot, &e),
                    }
                }
                AuthOutcome::SignedIn {
                    slot,
                    account,
                    correct,
                } => {
                    self.auth_in_flight.remove(&slot);
                    if correct {
                        self.admin.complete_sign_in(slot, &account);
                        let group = self.admin.group_of(slot).name.clone();
                        self.tell(slot, &format!("signed in as {account} ({group})."));
                        info!(slot, account, "signed in");
                        self.notify_update_if_pending(slot);
                    } else {
                        // One message for both, so it does not say which accounts exist.
                        self.tell(slot, "that name and password do not go together.");
                    }
                }
            }
        }
    }

    /// Hands `slot` the pending update notice, if there is one and `slot` just signed in with the
    /// `Admin` permission — "the first recognised admin who connects" from `update`'s own module
    /// doc. `.take()` on the shared cell delivers it exactly once, to whoever this turns out to
    /// be; every sign-in after that finds the cell already empty and says nothing.
    fn notify_update_if_pending(&mut self, slot: u8) {
        if !self.admin.may(slot, crate::admin::Permission::Admin) {
            return;
        }
        let Some(handle) = &self.update_notice else {
            return;
        };
        let message = handle.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(message) = message {
            self.tell(slot, &message);
        }
    }

    /// Announce a background save that has finished since the last tick.
    ///
    /// The writer runs on another thread and cannot reach the chat, so it posts its result down a
    /// channel and the game task collects it. Polled rather than awaited because the tick is not
    /// async and should not become so for this.
    ///
    /// Only worth saying out loud for a save somebody asked for: an autosave that works is not
    /// news, and one that fails is already in the log as an error.
    fn note_finished_save(&mut self) {
        let Ok(result) = self.save_results.1.try_recv() else {
            return;
        };
        match result {
            Ok(ms) if self.save_reason == "command" => {
                self.announce(&format!("World saved ({ms} ms)."));
            }
            Err(()) => self.announce("World save FAILED; see the server log."),
            Ok(_) => {}
        }
    }

    pub async fn run(mut self, mut events: mpsc::Receiver<ServerEvent>) -> Stopped {
        // Whoever lived here when the world was last saved lives here again.
        self.restore_town_npcs();
        self.announce_claim_token();

        let mut ticker = interval(TICK);
        // Catching up on missed ticks would fast-forward the world clock after any stall.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut outcome = Stopped::Cleanly;
        loop {
            tokio::select! {
                event = events.recv() => match event {
                    // Wrapped for the same reason the tick below is, and more urgently: this is
                    // the path every byte from an untrusted client travels. It was left bare, so
                    // a panic anywhere under `handle_packet` — or in any of the ~130 AI routines
                    // beneath it — unwound straight out of this loop, past the shutdown save at
                    // the bottom of the function, taking everything since the last autosave.
                    Some(event) => {
                        let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                            || self.handle_event(event),
                        ));
                        if handled.is_err() {
                            error!("handling a packet panicked; saving the world and stopping");
                            outcome = Stopped::Panicked;
                            break;
                        }
                    }
                    None => break,
                },
                _ = ticker.tick() => {
                    // A panic in here would otherwise take the world with it. The game is a
                    // single task and the shutdown save below lives inside it, so an unwind
                    // straight out of the loop loses everything since the last autosave. Catching
                    // it turns that into a clean stop that still writes the world out.
                    //
                    // `AssertUnwindSafe` is the honest choice rather than a safe one: the server's
                    // state may well be inconsistent after a panic. That is exactly why this saves
                    // and stops rather than carrying on.
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.tick())) {
                        Ok(cost) => {
                            self.note_tick_cost(cost);
                            if self.stopping {
                                break;
                            }
                        }
                        Err(_) => {
                            error!("the game loop panicked; saving the world and stopping");
                            outcome = Stopped::Panicked;
                            break;
                        }
                    }
                }
            }
        }

        // The channel closing is the shutdown signal, so this is the last chance to persist.
        //
        // Let a background save finish first if one is in flight. Both write through a temporary
        // file and rename, so neither can leave a half-written world — but the shutdown save has
        // the newer state and must land last, and two renames racing would decide that by
        // scheduling rather than by which is newer.
        if let Some(running) = self.saving.take() {
            let _ = running.await;
        }
        self.save_world("shutdown");
        info!("game loop stopped");
        outcome
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
            sync_full = SYNC_FULL.load(std::sync::atomic::Ordering::Relaxed),
            sync_stream = SYNC_STREAM.load(std::sync::atomic::Ordering::Relaxed),
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
        // Phases are timed on the *same* clock as the tick total, which they were not: the total
        // came from `clock::Cpu` and the laps from `Instant`, so the warning line compared CPU
        // microseconds against wall microseconds and could report a phase costing more than the
        // whole tick that contained it. Every phase figure ever logged was inflated by however
        // long that phase spent descheduled. Nine extra thread-clock reads a tick is nothing —
        // it is a vDSO call — and it makes the phases add up to the total, which is the only way
        // the breakdown means anything.
        let mut clock = PhaseClock::start();
        let mut lap = |cost: &mut TickCost, phase: Phase| {
            cost.phases[phase as usize] += clock.lap();
        };

        self.ticks += 1;
        let was_day = self.world.day_time;
        // Journey mode's `FreezeTime` (`Main.cs:6342` gates the whole day/night update the same
        // way). The clock — and everything below keyed off it turning midnight or dawn — simply
        // does not run this tick; nothing here needs its own separate "and skip that too" branch.
        // `ModifyTimeRate` (`Main.cs:6343`'s own `targetTimeRate`) is the other half of the same
        // gate in source — applied here as the tick count itself rather than a separate branch,
        // since `tick_time`'s own loop already handles more than one day/night flip in one call.
        if !self.journey.freeze_time {
            self.world.tick_time(self.journey.time_rate());
            self.tick_slime_rain();
        }
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
        self.tick_party();

        if let Some(every) = self.autosave_ticks
            && self.ticks.is_multiple_of(every)
        {
            self.save_world_in_background("autosave");
        }
        // Its own phase because it is the single most expensive thing the tick does, and it was
        // hidden inside a bucket of thirteen systems.
        lap(&mut cost, Phase::Snapshot);
        self.note_finished_save();
        self.note_finished_auth();
        self.reclaim_snapshot_buffer();
        self.tick_tile_spam();
        // What the world is worth fighting at, refreshed before anything can spawn. Cheap, and
        // keeping it here means no spawn site has to remember to scale.
        let difficulty = self.effective_difficulty();
        self.npcs.set_scaling(crate::game::npc::Scaling {
            difficulty,
            players: self
                .players
                .iter()
                .flatten()
                .filter(|p| p.is_playing())
                .count() as u32,
        });
        self.projectiles.set_hostile_damage_scale(
            terrustia_proto::difficulty::hostile_projectile_multiplier(difficulty),
        );

        self.tick_liquids();
        lap(&mut cost, Phase::Liquids);
        self.tick_growth();
        lap(&mut cost, Phase::Growth);
        self.tick_spread();
        lap(&mut cost, Phase::Spread);
        self.tick_weather();
        lap(&mut cost, Phase::Weather);
        // Whatever is left: the tile entities, the mech cooldowns, the wiring timers, the lunar
        // event and the biome census. Individually small; kept together so the breakdown does not
        // become a wall of near-zero lines.
        self.tick_tile_entities();
        self.tick_mech_cooldowns();
        self.tick_timers();
        self.tick_lunar();
        self.tick_census();
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
        self.tick_travelling_merchant();
        self.tick_old_man();
        self.tick_cultist_tablet();
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
        self.tick_shimmer();
        self.tick_wall_of_flesh_trigger();
        self.correct_item_drift();

        // Offer unreserved items to the nearest player in range. Range is per-player, not one
        // shared constant: Journey mode's `FarPlacementRange` (a misleading name inherited from
        // source — both of its two real vanilla uses, `Player.cs:35212`/`35440`, are about item
        // *pickup* range, not tile placement at all) adds a flat 240 pixels for whichever players
        // have it on, but — matching source's own `difficulty == 3` guard on both sites — only in
        // a Journey-mode world; the power has no effect at all in an ordinary one, even for a
        // player who somehow has it enabled.
        let journey_world = self.world.game_mode == 3;
        let positions: Vec<(u8, (f32, f32), f32)> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing())
            .map(|p| {
                let range = if journey_world && self.journey.has_far_placement_range(p.slot) {
                    ITEM_GRAB_RANGE + 240.0
                } else {
                    ITEM_GRAB_RANGE
                };
                (p.slot, p.position, range)
            })
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
                    .map(|(slot, pos, range)| {
                        let (dx, dy) = (pos.0 - item.position.0, pos.1 - item.position.1);
                        (*slot, dx * dx + dy * dy, range * range)
                    })
                    .filter(|(_, d2, range2)| *d2 <= *range2)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(slot, ..)| (index, slot, item.position))
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
        if is_tree_tile(tile) {
            self.spawn_tree_drop(frame_x, frame_y, x, y);
            return;
        }
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

    /// A tree tile's own drop: kept apart from [`drop_of`]'s static table because vanilla's real
    /// mechanism (`WorldGen.KillTile_GetTreeDrops`) needs live world state a per-tile lookup
    /// cannot see — which biome the tree is rooted in, found by walking down to the ground.
    ///
    /// Faithful to source with one disclosed simplification: vanilla's own "bonus wood" roll also
    /// scales with the chopping player's currently-equipped axe's power (`genRand.Next(35) <=
    /// axe`), which needs per-slot inventory-content tracking this project does not have yet (the
    /// same missing prerequisite `plan.md`'s `RedHatSkeletron` gap already found and disclosed).
    /// Only the roll's own item-independent term (`Main.rand.Next(3) == 0`, real vanilla data, not
    /// invented) is transcribed here; the axe-scaling half is a real, narrower, disclosed gap.
    fn spawn_tree_drop(&mut self, frame_x: i16, frame_y: i16, x: i32, y: i32) {
        let species = self.tree_species_at(x, y);
        // `WorldGen.cs`'s own literal condition, transcribed as-is rather than redesigned: it is
        // the frame range vanilla actually uses to decide "is this the leafy top", quirks and all.
        let is_top = frame_x >= 22 && frame_y >= 198;

        let mut secondary = None;
        if is_top && rand::Rng::random_range(&mut self.rng, 0..2) == 0 && tree_drops_acorns(species)
        {
            secondary = Some(ACORN);
        }

        let primary = match species {
            TreeSpecies::Corrupt => Some(EBONWOOD),
            TreeSpecies::Crimson => Some(SHADEWOOD),
            TreeSpecies::Jungle => Some(RICH_MAHOGANY),
            TreeSpecies::Hallowed => Some(PEARLWOOD),
            TreeSpecies::Snow => Some(BOREAL_WOOD),
            TreeSpecies::Mushroom => {
                (rand::Rng::random_range(&mut self.rng, 0..2) == 0).then_some(GLOWING_MUSHROOM)
            }
            TreeSpecies::Forest | TreeSpecies::None => Some(WOOD),
        };

        let position = (x as f32 * 16.0, y as f32 * 16.0);
        if let Some(item_id) = primary {
            let mut stack: i16 = 1;
            if rand::Rng::random_range(&mut self.rng, 0..3) == 0 {
                stack += 1;
            }
            if let Some(index) = self
                .items
                .spawn(ItemStack::new(item_id, stack, 0), position)
            {
                self.broadcast_item(index);
            }
        }
        if let Some(secondary_id) = secondary
            && let Some(index) = self
                .items
                .spawn(ItemStack::new(secondary_id, 1, 0), position)
        {
            self.broadcast_item(index);
        }
    }

    /// Which vanilla species a tree tile belongs to, found by walking down to the ground it is
    /// rooted in — `WorldGen.GetTreeBottom` + `GetTreeType`. The broken tile itself is already
    /// cleared by the time this runs, exactly as in source (`KillTile` clears the tile before
    /// computing its drop): the walk tolerates that by treating "not active" the same as "still a
    /// tree tile, keep walking", the same forgiving condition vanilla's own loop uses.
    ///
    /// Only the ground types this generator's own `trees::fit_for_tree` can actually grow a tree
    /// on ever occur here — vanilla's desert-palm and underworld-ash branches are omitted as
    /// genuinely unreachable rather than transcribed dead, since nothing in this project plants a
    /// tree on sand or ash today.
    fn tree_species_at(&self, x: i32, y: i32) -> TreeSpecies {
        let mut y = y;
        loop {
            let here = self.world.tile(x, y);
            if here.is_active() && !is_tree_tile(here.block) {
                break;
            }
            if !self.world.in_bounds(x, y + 1) {
                break;
            }
            y += 1;
        }
        let ground = self.world.tile(x, y);
        if !ground.is_active() {
            return TreeSpecies::None;
        }
        match ground.block {
            2 => TreeSpecies::Forest,
            23 => TreeSpecies::Corrupt,
            60 => TreeSpecies::Jungle,
            70 => TreeSpecies::Mushroom,
            109 => TreeSpecies::Hallowed,
            147 => TreeSpecies::Snow,
            199 => TreeSpecies::Crimson,
            _ => TreeSpecies::None,
        }
    }

    /// The Travelling Merchant: whether he turns up today, and whether he has gone.
    ///
    /// He is not a resident — he has no house and no permanent slot. He arrives at random during
    /// the first half of a day, provided the town already has two other townsfolk, and he leaves
    /// at dusk whether or not anybody bought anything.
    ///
    /// `Main.UpdateTime`'s own arrangement: the odds are one in `27000 / dayRate * 4` per tick,
    /// which over a morning works out at rather better than it sounds.
    fn tick_travelling_merchant(&mut self) {
        let here = self
            .npcs
            .iter()
            .find(|(_, n)| n.npc_type == TRAVELLING_MERCHANT)
            .map(|(index, _)| index);

        // Dusk, or past the hour he leaves at, and he packs up.
        let leaving = !self.world.day_time || self.world.time > MERCHANT_LEAVES_AT;
        if let Some(index) = here {
            if leaving {
                self.npcs.remove(index);
                self.broadcast_npc_death(index);
                self.announce("The Traveling Merchant has departed!");
                info!("the travelling merchant left");
            }
            return;
        }
        if leaving || self.world.time >= MERCHANT_ARRIVES_BEFORE {
            return;
        }
        // Two other townsfolk, not counting the Old Man or the Skeleton Merchant, who are not
        // residents either.
        let townsfolk = self
            .npcs
            .iter()
            .filter(|(_, n)| {
                n.stats.town_npc && n.npc_type != OLD_MAN && n.npc_type != SKELETON_MERCHANT
            })
            .count();
        if townsfolk < 2 {
            return;
        }
        if rand::Rng::random_range(&mut self.rng, 0..MERCHANT_ODDS) != 0 {
            return;
        }

        // He arrives at the world's spawn, since he has no home to arrive at.
        let at = (
            f32::from(self.world.spawn_x) * TILE_SIZE,
            f32::from(self.world.spawn_y) * TILE_SIZE - 48.0,
        );
        let Some(index) = self.npcs.spawn(TRAVELLING_MERCHANT, at) else {
            return;
        };
        self.roll_travel_shop();
        self.broadcast_npc(index);
        self.broadcast_travel_shop();
        self.announce("The Traveling Merchant has arrived!");
        info!("the travelling merchant arrived");
    }

    /// Pick what he is carrying today. `Chest.SetupTravelShop`.
    ///
    /// The stock is a chain of rolls rather than a list: each candidate that comes up overwrites
    /// the last, so the final match wins and rarer things are rarer because their odds are
    /// longer, not because they are drawn from a smaller pool.
    fn roll_travel_shop(&mut self) {
        use terrustia_proto::travel_shop::{Needs, OFFERS, TIER_ODDS};
        let p = &self.world.progress;
        let mut world = 0u16;
        for (yes, flag) in [
            (p.hard_mode, Needs::HARDMODE),
            (p.downed_mech_any, Needs::ANY_MECH),
            (p.downed_mech1, Needs::DESTROYER),
            (p.downed_mech2, Needs::TWINS),
            (p.downed_mech3, Needs::PRIME),
            (p.downed_boss1, Needs::EYE),
            (p.downed_boss3, Needs::SKELETRON),
            (p.shadow_orb_smashed, Needs::ORB_SMASHED),
        ] {
            if yes {
                world |= flag;
            }
        }
        let world = Needs(world);

        // Four to six things, as the game rolls it.
        let wanted = rand::Rng::random_range(&mut self.rng, 4..7);
        self.travel_shop.clear();
        // Fifty passes over the chain is plenty to fill six slots and bounded whatever the odds
        // do; the game's own loop is capped at five thousand for the same reason.
        for _ in 0..MERCHANT_ROLLS {
            if self.travel_shop.len() >= wanted {
                break;
            }
            let mut chosen = None;
            for offer in OFFERS {
                if !offer.needs.met_by(world) {
                    continue;
                }
                let odds = TIER_ODDS
                    .get(offer.tier as usize)
                    .copied()
                    .unwrap_or(1)
                    .max(1);
                if rand::Rng::random_range(&mut self.rng, 0..odds) == 0 {
                    // Later entries overwrite earlier ones, which is what makes the chain work.
                    chosen = Some(offer.item);
                }
            }
            if let Some(item) = chosen
                && !self.travel_shop.contains(&item)
            {
                self.travel_shop.push(item);
            }
        }
        debug!(
            stock = self.travel_shop.len(),
            "travelling merchant's stock"
        );
    }

    /// Tell everyone what he is carrying.
    ///
    /// Forty slots on the wire whatever he actually has, zero-filled — the client reads a fixed
    /// count, so a short packet desynchronises everything after it.
    fn broadcast_travel_shop(&mut self) {
        let mut w = terrustia_proto::PacketWriter::new(id::TRAVEL_MERCHANT_ITEMS);
        for slot in 0..TRAVEL_SHOP_SLOTS {
            w.i16(self.travel_shop.get(slot).copied().unwrap_or(0) as i16);
        }
        if let Ok(frame) = w.finish() {
            self.broadcast(frame, None);
        }
    }

    /// Tell clients where resting items actually are.
    ///
    /// A client simulates a dropped item's fall itself from the moment it is told the item
    /// exists, so the two can end up a few pixels apart — over a slope, in water, or after a
    /// tile is broken out from under it. The gap is invisible until somebody tries to walk over
    /// an item that is not where they see it.
    ///
    /// This is packet 160, which carries a position and nothing else. Sent for a handful of items
    /// per second rather than all of them: the drift is slow, and a full sweep every tick would
    /// cost more than the problem.
    fn correct_item_drift(&mut self) {
        if !self.ticks.is_multiple_of(ITEM_DRIFT_INTERVAL) || self.items.is_empty() {
            return;
        }
        let resting: Vec<(i16, (f32, f32))> = self
            .items
            .iter()
            .filter(|(_, item)| item.resting)
            .map(|(index, item)| (index, item.position))
            .take(ITEMS_PER_SWEEP)
            .collect();
        for (index, at) in resting {
            let mut w = terrustia_proto::PacketWriter::new(id::ITEM_POSITION);
            w.i16(index).f32(at.0).f32(at.1);
            if let Ok(frame) = w.finish() {
                self.broadcast_to_nearby(frame, at);
            }
        }
    }

    /// Sink whatever is lying in shimmer, and transmute what has gone far enough in.
    ///
    /// Shimmer is the 1.4.4 transmutation pool: an item dropped into it becomes another item,
    /// a creature, or — for coins — luck. It does not happen on contact. An item sinks over about
    /// a second and a half and changes at nine tenths, which is what makes the mechanic feel
    /// deliberate rather than punishing: you can pull something back out.
    ///
    /// One branch of the game's shimmer is missing and is a gap rather than a decision: an item
    /// with no transform and no creature falls back to being **decrafted** into its recipe's
    /// ingredients, which needs the whole recipe database. Such an item simply sits in the
    /// shimmer here. See `docs/shimmer.md`.
    fn tick_shimmer(&mut self) {
        use crate::world::items::{Shimmering, shimmer};
        // Almost always nothing is in shimmer, and finding that out should cost one scan of the
        // item table rather than a tile lookup per item.
        if self.items.is_empty() {
            return;
        }

        let mut transmuted: Vec<(i16, ItemStack, (f32, f32))> = Vec::new();
        let mut luck: Vec<((f32, f32), i32)> = Vec::new();
        let mut decrafted: Vec<Decraft> = Vec::new();
        let crimson = self.world.crimson;
        {
            let world = &self.world;
            for (index, item) in self.items.iter_mut() {
                // The game's own test: the tile *above* the item, since it is sinking through a
                // surface rather than standing in a pool.
                let x = (item.position.0 + crate::world::items::ITEM_SIZE / 2.0) / TILE_SIZE;
                let y = item.position.1 / TILE_SIZE - 1.0;
                let tile = world.tile(x as i32, y as i32);
                let in_shimmer =
                    tile.liquid > 0 && tile.liquid_kind == terrustia_proto::Liquid::Shimmer;

                if shimmer(item, in_shimmer) != Shimmering::Transmute {
                    continue;
                }
                let held = item.item;
                let at = item.position;
                let kind = u16::try_from(held.id).unwrap_or(0);

                if terrustia_proto::shimmer::is_coin(kind) {
                    // Coins are not transmuted but spent: they become luck, and are gone.
                    luck.push((
                        at,
                        terrustia_proto::shimmer::coin_luck(kind, i32::from(held.stack)),
                    ));
                    transmuted.push((index, ItemStack::EMPTY, at));
                } else if let Some(into) = terrustia_proto::shimmer::transforms_into(kind) {
                    item.shimmered = true;
                    item.item = ItemStack {
                        id: i32::from(into),
                        stack: held.stack,
                        prefix: 0,
                    };
                    transmuted.push((index, item.item, at));
                } else if let Some(recipe) = terrustia_proto::recipes::decraft_recipe(kind, crimson)
                    && i32::from(held.stack) >= i32::from(recipe.makes)
                {
                    // No transform of its own, so it comes apart into what it was made of. A
                    // stack only decrafts in whole batches: three torches give back one gel, and
                    // the two left over stay torches.
                    let batches = i32::from(held.stack) / i32::from(recipe.makes);
                    let kept = held.stack - (batches * i32::from(recipe.makes)) as i16;
                    item.shimmered = true;
                    item.item.stack = kept;
                    decrafted.push(Decraft {
                        index,
                        at,
                        recipe,
                        batches,
                        kept,
                    });
                } else {
                    // Nothing to become and nothing to come apart into. Mark it done so it stops
                    // asking the same question every tick.
                    item.shimmered = true;
                }
            }
        }

        for (index, now, at) in transmuted {
            if now.is_empty() {
                self.items.remove(index);
                if let Ok(frame) = terrustia_proto::items::item_despawn(index) {
                    self.broadcast(frame, None);
                }
            } else {
                self.broadcast_item(index);
            }
            // The sparkle, which every client draws for itself once it is told where.
            let mut w = terrustia_proto::PacketWriter::new(id::SHIMMER_ACTIONS);
            w.u8(SHIMMER_EFFECT).f32(at.0).f32(at.1);
            if let Ok(frame) = w.finish() {
                self.broadcast(frame, None);
            }
        }
        for (at, amount) in luck {
            let mut w = terrustia_proto::PacketWriter::new(id::SHIMMER_ACTIONS);
            w.u8(SHIMMER_COIN_LUCK).f32(at.0).f32(at.1).i32(amount);
            if let Ok(frame) = w.finish() {
                self.broadcast(frame, None);
            }
            debug!(amount, "coins turned into luck");
        }

        for job in decrafted {
            // Whatever did not make a whole batch stays where it was; the rest comes apart.
            if job.kept > 0 {
                self.broadcast_item(job.index);
            } else {
                self.items.remove(job.index);
                if let Ok(frame) = terrustia_proto::items::item_despawn(job.index) {
                    self.broadcast(frame, None);
                }
            }
            for &(ingredient, per_batch) in job.recipe.ingredients() {
                let mut count = job.batches.saturating_mul(i32::from(per_batch));
                // Alchemy gives back less: each unit has a one-in-three chance of being lost.
                // Without it, potions would be a free material duplicator.
                if job.recipe.alchemy {
                    let mut kept_units = 0;
                    for _ in 0..count {
                        if rand::Rng::random_range(&mut self.rng, 0..3) != 0 {
                            kept_units += 1;
                        }
                    }
                    count = kept_units;
                }
                // Spread across stacks rather than one impossible pile.
                while count > 0 {
                    let stack = count.min(i32::from(MAX_ITEM_STACK));
                    count -= stack;
                    self.spawn_item(
                        ItemStack {
                            id: i32::from(ingredient),
                            stack: stack as i16,
                            prefix: 0,
                        },
                        job.at,
                    );
                }
            }
            let mut w = terrustia_proto::PacketWriter::new(id::SHIMMER_ACTIONS);
            w.u8(SHIMMER_EFFECT).f32(job.at.0).f32(job.at.1);
            if let Ok(frame) = w.finish() {
                self.broadcast(frame, None);
            }
            debug!(
                result = job.recipe.result,
                batches = job.batches,
                "decrafted an item in shimmer"
            );
        }
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
            ServerEvent::Console { line } => self.run_console(&line),
            ServerEvent::ConsoleContext { reply } => {
                let players = self
                    .players
                    .iter()
                    .flatten()
                    .filter(|p| p.is_playing())
                    .map(|p| p.name.clone())
                    .collect();
                let groups = self.admin.groups.iter().map(|g| g.name.clone()).collect();
                let _ = reply.send(ConsoleContext { players, groups });
            }
            ServerEvent::PanelAuthLookup { name, reply } => {
                let _ = reply.send(PanelAuthLookup {
                    unclaimed: self.admin.unclaimed(),
                    claim_token: self.claim_token.clone(),
                    hash_and_group: self.admin.account_hash_and_group(&name),
                });
            }
            ServerEvent::PanelInsertAccount { account, reply } => {
                let _ = reply.send(self.admin.insert_account(account));
            }
            ServerEvent::PanelStatus { reply } => {
                let player_count = self
                    .players
                    .iter()
                    .flatten()
                    .filter(|p| p.is_playing())
                    .count();
                let _ = reply.send(PanelStatus {
                    player_count,
                    max_players: self.config.max_players,
                    world_name: self.world.name.clone(),
                    world_file: self.current_world_file_stem(),
                });
            }
            ServerEvent::PanelPlayers { reply } => {
                let _ = reply.send(self.panel_players());
            }
            ServerEvent::PanelKick {
                name,
                reason,
                reply,
            } => {
                let Some(target) = self.slot_named(&name) else {
                    let _ = reply.send(Err(format!("nobody named {name} is connected")));
                    return;
                };
                let reason = if reason.trim().is_empty() {
                    "kicked from the web panel".to_string()
                } else {
                    reason
                };
                // The exact two calls `run_admin_command`'s own "kick" arm makes.
                self.announce(&format!("{name} was kicked: {reason}"));
                self.kick(target, &reason);
                let _ = reply.send(Ok(()));
            }
            ServerEvent::PanelBan {
                kind,
                value,
                reason,
                reply,
            } => {
                let reason = if reason.trim().is_empty() {
                    "banned from the web panel".to_string()
                } else {
                    reason
                };
                // The exact sequence `run_admin_command`'s own "ban" arm runs.
                self.admin.ban(kind, &value, &reason);
                self.announce(&format!("{value} is banned: {reason}"));
                if let Some(target) = self.slot_named(&value) {
                    self.kick(target, &reason);
                }
                info!(value, reason, "ban added from the web panel");
                let _ = reply.send(());
            }
            ServerEvent::PanelUnban { value, reply } => {
                let _ = reply.send(self.admin.unban(&value));
            }
            ServerEvent::PanelWhitelist { reply } => {
                let _ = reply.send(PanelWhitelist {
                    on: self.admin.whitelist_on(),
                    names: self.admin.whitelist.clone(),
                });
            }
            ServerEvent::PanelWhitelistAdd { name, reply } => {
                let added = self.admin.add_to_whitelist(&name);
                if added {
                    let _ = self.admin.save();
                }
                let _ = reply.send(added);
            }
            ServerEvent::PanelWhitelistRemove { name, reply } => {
                let removed = self.admin.remove_from_whitelist(&name);
                if removed {
                    let _ = self.admin.save();
                    // Take effect now rather than at their next join — mirrors the console's own
                    // `whitelist remove` arm.
                    if let Some(slot) = self.slot_named(&name) {
                        self.kick(slot, "You are no longer on this server's guest list.");
                    }
                }
                let _ = reply.send(removed);
            }
            ServerEvent::PanelWorldTiles { reply } => {
                let _ = reply.send(self.world_tile_sample());
            }
            ServerEvent::PanelConfigSnapshot { reply } => {
                let _ = reply.send(PanelConfigSnapshot {
                    listen: self.config.listen,
                    max_players: self.config.max_players,
                    world_width: self.world.width(),
                    world_height: self.world.height(),
                    motd: self.config.motd.clone(),
                    password_set: !self.config.password.is_empty(),
                    max_chat_len: self.config.max_chat_len,
                    idle_timeout_secs: self.config.idle_timeout_secs,
                    autosave_secs: self.config.autosave_secs,
                    save_target: self.save_path.as_ref().map(|p| p.display().to_string()),
                    whitelist_on: self.admin.whitelist_on(),
                    whitelist_count: self.admin.whitelist.len(),
                });
            }
            ServerEvent::PanelSetMotd { motd, reply } => {
                self.config.motd = motd;
                let _ = reply.send(());
            }
            ServerEvent::PanelSwitchWorld { path, reply } => {
                if !path.exists() {
                    let _ = reply.send(Err(format!("{} no longer exists", path.display())));
                    return;
                }
                self.announce("The server is restarting into a different world.");
                info!(path = %path.display(), "world switch requested from the web panel");
                *self
                    .pending_world_switch
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) = Some(path);
                self.stopping = true;
                let _ = reply.send(Ok(()));
            }
        }
    }

    /// The file stem of the world currently being served, if it has one.
    fn current_world_file_stem(&self) -> Option<String> {
        self.save_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .map(str::to_string)
    }

    /// Every connected player, in the shape the panel needs for the player list and the live world
    /// view. `appearance` decodes `Player::appearance`'s raw bytes on demand rather than caching a
    /// decoded copy on `Player` itself — this is the only consumer, and it is asked for at most a
    /// couple of times a second, not once per tick.
    fn panel_players(&self) -> Vec<PanelPlayer> {
        self.players
            .iter()
            .flatten()
            .filter(|p| p.is_playing())
            .map(|p| PanelPlayer {
                slot: p.slot,
                name: p.name.clone(),
                address: p.addr.ip().to_string(),
                life: p.life,
                life_max: p.life_max,
                mana: p.mana,
                mana_max: p.mana_max,
                position: p.position,
                pvp: p.pvp,
                appearance: p.appearance.as_ref().and_then(|bytes| {
                    terrustia_proto::player_info::PlayerAppearance::decode(bytes).ok()
                }),
                equipped: Self::equipped_items(p),
            })
            .collect()
    }

    /// Non-zero item ids in the armour/accessory slot run — real gear, used to accent the panel's
    /// stylized avatar. See `terrustia_proto::inventory`'s `SLOT_RUNS` for the layout: 58 inventory
    /// slots, then 1 cursor slot, then this 20-slot armour/accessory run.
    fn equipped_items(player: &Player) -> Vec<i32> {
        const ARMOR_SLOTS_START: u16 = 59;
        const ARMOR_SLOTS_END: u16 = 79;
        let mut items: Vec<i32> = player
            .inventory
            .iter()
            .filter(|(slot, _)| (ARMOR_SLOTS_START..ARMOR_SLOTS_END).contains(slot))
            .filter(|(_, equip)| equip.item.id != 0)
            .map(|(_, equip)| equip.item.id)
            .collect();
        items.sort_unstable();
        items
    }

    /// How many sample points the panel's live world view gets along each axis, regardless of how
    /// large the actual world is. A full tile-for-tile dump of even a small 4200x1200 world is
    /// five million tiles — nothing a websocket should re-send every few seconds. This is dense
    /// enough to show real terrain shape at a glance and cheap enough to resample from scratch on
    /// every request: at most `WORLD_SAMPLE_COLS * WORLD_SAMPLE_ROWS` tile reads, each a plain
    /// array index.
    fn world_tile_sample(&self) -> PanelWorldTiles {
        const WORLD_SAMPLE_COLS: u32 = 160;
        const WORLD_SAMPLE_ROWS: u32 = 90;

        let width = self.world.width();
        let height = self.world.height();
        let cols = WORLD_SAMPLE_COLS.min(width.max(1) as u32).max(1);
        let rows = WORLD_SAMPLE_ROWS.min(height.max(1) as u32).max(1);
        let mut tiles = Vec::with_capacity((cols * rows) as usize);
        for row in 0..rows {
            let y = ((row * height.max(1) as u32) / rows).min((height - 1).max(0) as u32) as i32;
            for col in 0..cols {
                let x = ((col * width.max(1) as u32) / cols).min((width - 1).max(0) as u32) as i32;
                tiles.push(Self::tile_color(self.world.tile(x, y)));
            }
        }
        PanelWorldTiles {
            world_width: width,
            world_height: height,
            sample_cols: cols,
            sample_rows: rows,
            tiles,
        }
    }

    /// Bucket a tile into a solid colour, not a sprite. Every id below is transcribed from
    /// `crate::world::worldgen::tiles`, the same table the generator itself is checked against —
    /// nothing here is invented. An id with no bucket falls into [`TileColor::Other`] rather than
    /// guessing.
    fn tile_color(tile: terrustia_proto::Tile) -> TileColor {
        use crate::world::worldgen::tiles as t;

        if tile.liquid > 0 {
            return match tile.liquid_kind {
                terrustia_proto::Liquid::Lava => TileColor::Lava,
                terrustia_proto::Liquid::Honey => TileColor::Honey,
                terrustia_proto::Liquid::Water | terrustia_proto::Liquid::Shimmer => {
                    TileColor::Water
                }
            };
        }
        if !tile.is_active() {
            return TileColor::Empty;
        }
        match tile.block {
            t::GRASS => TileColor::Grass,
            t::CORRUPT_GRASS | t::EBONSTONE | t::DEMON_ALTAR | t::SHADOW_ORB => {
                TileColor::Corruption
            }
            t::CRIMSON_GRASS | t::CRIMSTONE => TileColor::Crimson,
            t::JUNGLE_GRASS | t::MUD | t::HIVE | t::LIHZAHRD_BRICK | t::MUSHROOM_GRASS => {
                TileColor::Jungle
            }
            t::SAND | t::EBONSAND | t::CRIMSAND | t::SANDSTONE | t::HARDENED_SAND | t::SILT => {
                TileColor::Sand
            }
            t::SNOW => TileColor::Snow,
            t::ICE => TileColor::Ice,
            t::STONE | t::MARBLE | t::GRANITE | t::ASH | t::OBSIDIAN | t::CLAY => TileColor::Stone,
            t::IRON | t::COPPER | t::GOLD | t::SILVER | t::DEMONITE | t::CRIMTANE => TileColor::Ore,
            t::SAPPHIRE | t::RUBY | t::EMERALD | t::TOPAZ | t::AMETHYST | t::DIAMOND => {
                TileColor::Gem
            }
            t::DIRT => TileColor::Dirt,
            _ => TileColor::Other,
        }
    }

    /// A line typed at the server's own terminal.
    ///
    /// Whoever has the console already has the world file, so it is not gated: there is nothing a
    /// permission could protect them from. Output goes to the log rather than to chat, because the
    /// person who typed it is looking at the log.
    fn run_console(&mut self, line: &str) {
        // Every `info!` inside this function that names `target: CONSOLE_REPLY` is a command's
        // own reply, not an ordinary log line — `TermLayer` prints those the way a REPL prints
        // its own output: no timestamp, no level tag, no target column. Only the replies
        // textually inside this function are tagged; `save`, `backups`, `rollback` and the admin
        // commands delegate to shared functions used by other call paths too, and keep the
        // ordinary log formatting rather than risk retagging something a non-console caller
        // also relies on.
        use crate::term::CONSOLE_REPLY_TARGET as CONSOLE_REPLY;
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let (name, argument) = line.split_once(char::is_whitespace).unwrap_or((line, ""));

        match name.to_ascii_lowercase().as_str() {
            // Only exists in a test build. There has to be *some* way to make the packet path
            // panic on purpose, or the guard around it is only believed rather than checked —
            // and "we catch panics" is exactly the sort of claim that is never true until tried.
            #[cfg(test)]
            "__panic_probe" => panic!("deliberate panic, to prove the packet path is guarded"),
            // Claiming from the console needs no token: whoever can type here can already read
            // the world file, so there is nothing left to prove.
            "claim" => {
                let mut words = argument.split_whitespace();
                match (words.next(), words.next(), words.next()) {
                    (Some(name), Some(password), None) => {
                        if !self.admin.unclaimed() {
                            info!(target: CONSOLE_REPLY, "this server already has an owner; use /register or /group");
                        } else if password.len() < 6 {
                            info!(target: CONSOLE_REPLY, "that password is too short; use at least six characters");
                        } else {
                            match crate::admin::Account::new(name, password, "owner") {
                                Ok(account) => match self.admin.insert_account(account) {
                                    Ok(()) => {
                                        let _ = self.admin.save();
                                        self.claim_token = None;
                                        info!(target: CONSOLE_REPLY, account = name, "server claimed from the console");
                                    }
                                    Err(e) => info!(target: CONSOLE_REPLY, "{e}"),
                                },
                                Err(e) => info!(target: CONSOLE_REPLY, "{e}"),
                            }
                        }
                    }
                    _ => info!(target: CONSOLE_REPLY, "usage: claim <name> <password>"),
                }
            }
            "help" => info!(
                target: CONSOLE_REPLY,
                "console: say <text> | players | save | backups | rollback <n> | \
                 whitelist add|remove|list [name] | \
                 claim <name> <password> | kick <name> [reason] | \
                 ban <name|ip|uuid> <value> [reason] | unban <value> | group <account> <group> | \
                 world undo <player> <duration> | panel | stop"
            ),
            // Toggles the web panel: starts it if it is not running, stops it if it is.
            // `panel_toggle`'s other end (`crate::panel::supervise`) owns the actual bind/abort and
            // decides which of those this pulse means — this arm only ever sends one and reports
            // whether it could.
            "panel" => match &self.panel_toggle {
                Some(toggle) if toggle.send(()).is_ok() => {
                    info!(target: CONSOLE_REPLY, "panel toggled — see the log line just above for which way");
                }
                Some(_) => info!(target: CONSOLE_REPLY, "the panel supervisor is gone"),
                None => info!(target: CONSOLE_REPLY, "no panel supervisor is wired up in this run"),
            },
            "say" => {
                self.announce(argument);
            }
            "players" => {
                let names: Vec<&str> = self
                    .players
                    .iter()
                    .flatten()
                    .filter(|p| p.is_playing())
                    .map(|p| p.name.as_str())
                    .collect();
                info!(target: CONSOLE_REPLY, online = names.len(), "{}", names.join(", "));
            }
            "save" => self.save_world_in_background("console"),
            "backups" => self.list_backups(),
            "whitelist" => {
                let mut words = argument.split_whitespace();
                match (words.next(), words.next()) {
                    (Some("add"), Some(name)) => {
                        if self.admin.add_to_whitelist(name) {
                            let _ = self.admin.save();
                            info!(target: CONSOLE_REPLY, name, "added to the guest list");
                        } else {
                            info!(target: CONSOLE_REPLY, name, "already on the guest list");
                        }
                    }
                    (Some("remove"), Some(name)) => {
                        if self.admin.remove_from_whitelist(name) {
                            let _ = self.admin.save();
                            info!(target: CONSOLE_REPLY, name, "removed from the guest list");
                            // Take effect now rather than at their next join.
                            if let Some(slot) = self.slot_named(name) {
                                self.kick(slot, "You are no longer on this server's guest list.");
                            }
                        } else {
                            info!(target: CONSOLE_REPLY, name, "was not on the guest list");
                        }
                    }
                    (Some("list"), _) | (None, _) => {
                        if self.admin.whitelist_on() {
                            info!(
                                target: CONSOLE_REPLY,
                                names = %self.admin.whitelist.join(", "),
                                "the guest list is on"
                            );
                        } else {
                            info!(
                                target: CONSOLE_REPLY,
                                "the guest list is empty, so anyone may join. \
                                 `whitelist add <name>` turns it on."
                            );
                        }
                    }
                    _ => info!(target: CONSOLE_REPLY, "usage: whitelist add|remove|list [name]"),
                }
            }
            "rollback" => {
                let which: usize = argument.trim().parse().unwrap_or(1);
                self.roll_back(which);
            }
            "stop" => {
                info!("stopping on console request");
                self.stopping = true;
            }
            // The player-facing ones do the same thing here, reporting to the log. Slot 255 is
            // "the server", which `tell` already knows how to address.
            "kick" | "ban" | "unban" | "group" | "world" => {
                let _ = self.run_admin_command(net_module::SERVER_AUTHOR, name, argument);
            }
            other => {
                info!(target: CONSOLE_REPLY, "console: unknown command {other:?} (try 'help')")
            }
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
        // Before the early return, not after it. A client that disconnects mid-check would
        // otherwise hold one of the server's few hashing slots until its worker happened to
        // finish — and one doing it deliberately, repeatedly, would hold all of them.
        self.auth_in_flight.remove(&slot);

        let Some(player) = self.players.get_mut(slot as usize).and_then(Option::take) else {
            return;
        };
        info!(slot, name = %player.name, "player disconnected");
        // A session is not state: whoever reuses this slot starts as nobody.
        self.admin.sign_out(slot);

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
            // `LegacyMultiplayer.20` is `"{0} has left."`, with the name as its one argument.
            let who = NetworkText::literal(&player.name);
            self.announce_key("LegacyMultiplayer.20", vec![who]);
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
        // This always reaches players as real in-game chat (`chat_broadcast`, just below) — a
        // kick notice, a save confirmation, `say`'s own text — so it is exactly as much "chat" for
        // the panel's live feed as a player's own line is, tagged the same way.
        info!(target: crate::term::CHAT_TARGET, "{text}");
        if let Ok(frame) = net_module::chat_broadcast(
            net_module::SERVER_AUTHOR,
            &NetworkText::literal(text),
            SERVER_CHAT_COLOUR,
        ) {
            self.broadcast(frame, None);
        }
    }

    /// Announce something the *game* says, by its localization key.
    ///
    /// Vanilla does not send these as text. It sends a key and its arguments — `NPC.cs:81383` is
    /// `NetworkText.FromKey("Announcement.HasAwoken", ...)` against
    /// `"HasAwoken": "{0} has awoken!"` — and the client renders it in whatever language that
    /// player is playing in. Sending the English instead was wrong three times over: different
    /// bytes on the wire than the real server, English for every non-English player, and verbatim
    /// game text compiled into this repository.
    ///
    /// Only for lines the game itself has a key for. Anything this server says on its own behalf —
    /// save reports, kick reasons, console replies — stays [`Self::announce`], because there is no
    /// key for it and inventing one would render as nothing at all.
    fn announce_key(&mut self, key: &str, args: Vec<NetworkText>) {
        // The log is for whoever is running the server, so it stays readable English.
        info!(key, "announcing");
        if let Ok(frame) = net_module::chat_broadcast(
            net_module::SERVER_AUTHOR,
            &NetworkText::key(key, args),
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
            id::NPC_HOME => self.on_npc_home(slot, &payload),
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
            // Effects nobody but the sender would otherwise see: a temporary animation, a puff
            // of smoke, a legacy sound, a wired cannon firing, an NPC being interfered with, and
            // the two achievement announcements.
            | id::TEMPORARY_ANIMATION
            | id::POOF_OF_SMOKE
            | id::PLAY_LEGACY_SOUND
            | id::WIRED_CANNON_SHOT
            | id::TAMPER_WITH_N_P_C
            | id::ACHIEVEMENT_MESSAGE_N_P_C_KILLED
            | id::ACHIEVEMENT_MESSAGE_EVENT_HAPPENED
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
            id::QUICK_STACK_CHESTS => self.on_quick_stack(slot, &payload),
            id::FISH_OUT_N_P_C => self.on_fished_out_npc(slot, &payload),
            id::SET_MISC_EVENT_VALUES => self.on_misc_event_value(slot, &payload),
            id::REQUEST_LUCY_POPUP => self.on_lucy_popup(slot, &payload),
            // Chat a client asks the server to put in front of everybody: a sign read aloud, a
            // tombstone's epitaph. Relayed rather than modelled, but relayed *to everybody*,
            // which is the part that was missing.
            id::SMART_TEXT_MESSAGE => {
                if self.player(slot).is_some_and(Player::is_playing)
                    && let Ok(relayed) = packets::verbatim(frame.id, &payload)
                {
                    self.broadcast(relayed, Some(slot));
                }
                Ok(())
            }
            id::RELEASE_ITEM_OWNERSHIP => self.on_release_item(slot, &payload),
            id::MURDER_SOMEONE_ELSES_PORTAL => self.on_close_portal(slot, &payload),
            id::TELEPORT_PLAYER_THROUGH_PORTAL => self.on_portal_teleport(slot, &payload),
            id::NEBULA_LEVELUP_REQUEST => self.on_nebula_booster(slot, &payload),
            id::SYNC_EXTRA_VALUE => self.on_extra_value(slot, &payload),
            id::CRYSTAL_INVASION_REQUESTED_TO_SKIP_WAIT_TIME => self.on_skip_army_wait(slot),
            id::REQUEST_QUEST_EFFECT => self.on_quest_effect(slot),
            id::MASS_WIRE_OPERATION => self.on_mass_wire(slot, &payload),
            id::CHEST_NAME => self.on_chest_name_request(slot, &payload),
            id::GEM_LOCK_TOGGLE => self.on_gem_lock(slot, &payload),
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
            // Name both sides. A refusal that says only what the server speaks leaves the person
            // on the other end guessing which of the two needs updating — and this exact check
            // once refused *every* current client, because it matched the string "Terraria325"
            // while the installed game announced 326.
            info!(slot, version = %hello.version, "rejecting unsupported client");
            self.kick(
                slot,
                &format!(
                    "Your client speaks {}; this server speaks {}. \
                     Whichever is older needs updating.",
                    hello.version,
                    id::SUPPORTED_RELEASES
                        .iter()
                        .map(|r| format!("Terraria{r}"))
                        .collect::<Vec<_>>()
                        .join(" and "),
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

        // Two players sharing a name is not merely confusing. `angler_finished_today` is keyed by
        // name on purpose, so a duplicate shares one daily reward with the original and either can
        // shed the cooldown by renaming. Refuse the collision at the door instead.
        let wanted = name.trim().to_string();
        if !wanted.is_empty()
            && self
                .players
                .iter()
                .flatten()
                .any(|p| p.slot != slot && p.name.eq_ignore_ascii_case(&wanted))
        {
            info!(slot, name = %wanted, "rejecting a duplicate name");
            self.kick(slot, "Someone is already playing under that name.");
            return Ok(());
        }

        if let Some(player) = self.player_mut(slot) {
            if !wanted.is_empty() {
                player.name = wanted;
            }
            player.appearance = Some(Bytes::copy_from_slice(payload));
            player.advance_to(ConnState::Identified);
        }

        // The name is known now, so a name or address ban can be enforced before the world is
        // sent. A UUID ban has to wait for packet 68, which arrives later.
        self.enforce_ban(slot);
        if self.player(slot).is_none() {
            return Ok(());
        }

        // Relay live appearance changes; a first-time sync reaches others at spawn instead.
        if self.player(slot).is_some_and(Player::is_playing) {
            let frame = packets::rewrite_owner(id::SYNC_PLAYER, payload, slot)?;
            self.broadcast(frame, Some(slot));
        }
        Ok(())
    }

    fn on_request_world_data(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        let frame = self.world_data().encode()?;
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
        let world_data = self.world_data().encode()?;
        self.send(slot, world_data);

        let sections = self.sections_for(request);
        // The key rather than the English, which is what vanilla sends here (`Lang.inter[44]`):
        // a literal would put "Receiving tile data" on the loading screen of a client playing in
        // any other language.
        let status = packets::status_text(
            sections.len() as i32,
            &NetworkText::key("LegacyInterface.44", Vec::new()),
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
        // Membership is only *checked* here, and claimed further down once the bytes exist.
        // Claiming it up front meant a section that failed to encode was marked delivered anyway,
        // and every re-request for it was then dropped by this same dedupe — leaving a 200x150
        // hole of sky that no amount of walking back through would fill in.
        if self
            .player(slot)
            .is_none_or(|player| player.sent_sections.contains(&(sx, sy)))
        {
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
                let encoded =
                    match encode_section_packet(bounds, &extras, |x, y| self.world.tile(x, y)) {
                        Ok(bytes) => Bytes::from(bytes),
                        Err(e) => {
                            // Loud, because the symptom is a missing piece of world rather than
                            // anything that looks like an error to whoever is playing.
                            warn!(slot, sx, sy, error = %e, "could not encode a section");
                            return Err(e);
                        }
                    };
                self.section_cache.insert((sx, sy), encoded.clone());
                encoded
            }
        };
        if let Some(player) = self.player_mut(slot) {
            player.sent_sections.insert((sx, sy));
        }
        self.send_bytes(slot, frame);
        self.send_chest_contents_for_section(slot, bounds)?;
        Ok(())
    }

    /// Send the contents of every chest inside a section that has just gone out.
    ///
    /// The section itself only announces each chest's id, position and name — enough to draw it,
    /// and nothing more. The game follows every section with the contents as well
    /// (`NetMessage.SyncChestContentsForSection`), and the client needs them for the things it
    /// does without opening a chest: crafting from what is nearby, quick-stacking into it, and the
    /// item search. Without this a room full of stocked chests looks, to all three of those, like
    /// a room full of empty ones.
    fn send_chest_contents_for_section(
        &mut self,
        slot: u8,
        bounds: terrustia_proto::section::SectionBounds,
    ) -> terrustia_proto::Result<()> {
        let right = bounds.x + i32::from(bounds.width);
        let bottom = bounds.y + i32::from(bounds.height);
        let inside: Vec<(i16, Vec<terrustia_proto::ItemStack>)> = self
            .world
            .chests
            .iter()
            .enumerate()
            // A chest's id is its slot in the table, so the index has to survive the gaps that
            // deleted chests leave behind.
            .filter_map(|(id, slot)| slot.as_ref().map(|chest| (id, chest)))
            .filter(|(_, chest)| {
                let (x, y) = (i32::from(chest.x), i32::from(chest.y));
                x >= bounds.x && x < right && y >= bounds.y && y < bottom
            })
            .map(|(id, chest)| (id as i16, chest.items.clone()))
            .collect();

        for (id, items) in inside {
            self.send(slot, objects::sync_chest_size(id, items.len() as i16)?);
            for (index, item) in items.iter().enumerate() {
                let frame = SyncChestItem {
                    chest: id,
                    slot: index as u8,
                    item: *item,
                }
                .encode()?;
                self.send(slot, frame);
            }
        }
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

        // The block is *slid* inside the world rather than clipped against it. Clipping loses a
        // row or column whenever a player is near an edge — a player who spawns in the topmost
        // section used to get one fewer section beneath them than intended, which left the world
        // simply absent a hundred and fifty tiles below their feet. It only showed up when the
        // generator started putting the surface high enough to reach section zero.
        let mut add_block = |cx: i32, cy: i32, w: i32, h: i32| {
            let first_x = (cx - 2).clamp(0, (max_x - w).max(0));
            let first_y = (cy - 1).clamp(0, (max_y - h).max(0));
            for sx in first_x..(first_x + w).min(max_x) {
                for sy in first_y..(first_y + h).min(max_y) {
                    wanted.insert((sx, sy));
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

        // A player joining on the same machine the server runs on counts as the host, which is
        // exactly the rule the game uses — `DoesPlayerSlotCountAsAHost` asks the socket whether the
        // far end is the loopback address and nothing else. Only sent when true, as the game only
        // sends it when true.
        if self.player(slot).is_some_and(Player::is_local) {
            self.send(slot, packets::counts_as_host(slot, true)?);
        }

        // How much of the world has gone over to each side. The client cannot work this out from
        // the sections it holds, so without it the Dryad reports a world that is nought per cent
        // of everything however far the corruption has spread.
        self.send(
            slot,
            packets::world_evil_tally(
                self.census.percent_hallow,
                self.census.percent_corrupt,
                self.census.percent_crimson,
            )?,
        );

        // Where every town NPC lives. This is what the housing screen draws its banners from; a
        // client never told has an empty housing menu no matter how many villagers it can see.
        for frame in self.npc_home_frames() {
            self.send(slot, frame);
        }

        // Every banner's kill count. The world has been recording these since §26; nothing was
        // ever telling a client about them, so the bestiary showed nought kills for everything.
        self.send(slot, self.banner_state_frame()?);

        // Every pylon. The client keeps its own list and draws the travel map from it, so one it
        // was never told about is scenery: standing beside it opens a map with nowhere to go.
        for pylon in self.pylons() {
            self.pylon_kinds.insert((pylon.x, pylon.y), pylon.kind);
            self.send(
                slot,
                net_module::pylon_message(net_module::PylonMessage::Added, pylon)?,
            );
        }

        self.send(slot, packets::empty(id::FINISHED_CONNECTING_TO_SERVER)?);

        // What the Travelling Merchant is carrying, if he is here. A client that joins mid-visit
        // and is not told finds him with nothing to sell.
        if !self.travel_shop.is_empty() {
            let mut w = terrustia_proto::PacketWriter::new(id::TRAVEL_MERCHANT_ITEMS);
            for at in 0..TRAVEL_SHOP_SLOTS {
                w.i16(self.travel_shop.get(at).copied().unwrap_or(0) as i16);
            }
            if let Ok(frame) = w.finish() {
                self.send(slot, frame);
            }
        }

        // Which six cavern enemies this world has. Fixed for the world's life, so it is sent once
        // on joining rather than kept up to date.
        {
            let mut w = terrustia_proto::PacketWriter::new(id::SYNC_CAVERN_MONSTER_TYPE);
            for kind in self.cavern_monsters.flat() {
                w.u16(kind);
            }
            if let Ok(frame) = w.finish() {
                self.send(slot, frame);
            }
        }

        // Journey mode's four shared toggles this server models — `ASharedTogglePower::
        // OnPlayerJoining`'s own effect. A client never told assumes every power starts off, which
        // is wrong the moment an operator has frozen time or the weather before this player joined.
        for id in [
            net_module::power::FREEZE_TIME,
            net_module::power::FREEZE_RAIN,
            net_module::power::FREEZE_WIND,
            net_module::power::STOP_BIOME_SPREAD,
        ] {
            if let Some(enabled) = self.journey.get(id)
                && let Ok(frame) = net_module::creative_power_toggle(id, enabled)
            {
                self.send(slot, frame);
            }
        }
        // `ModifyTimeRate` and `Difficulty` are the two shared sliders that sync to a joining
        // player (`_syncToJoiningPlayers = true`, the base `ASharedSliderPower` default that
        // neither constructor overrides — `ModifyWind`/`ModifyRain` are both `false` in source,
        // see `journey.rs`'s own module doc for why there is nothing to send for those two here).
        for (id, value) in [
            (
                net_module::power::MODIFY_TIME_RATE,
                self.journey.time_rate_slider,
            ),
            (
                net_module::power::DIFFICULTY,
                self.journey.difficulty_slider,
            ),
        ] {
            if let Ok(frame) = net_module::creative_power_slider(id, value) {
                self.send(slot, frame);
            }
        }
        // `Godmode`/`FarPlacementRange`'s full per-player state — `APerPlayerTogglePower::
        // OnPlayerJoining`'s own `SyncEveryone`, bit-packed. `SpawnRate` sends nothing here on
        // purpose: `APerPlayerSliderPower::OnPlayerJoining` only resets the *new* player's own
        // local cache to the default, no network message at all — another player's slider
        // position was never anyone else's business in the first place (see the slider handler's
        // own comment on why a change to it is never broadcast either).
        for (id, states) in [
            (net_module::power::GODMODE, self.journey.godmode),
            (
                net_module::power::FAR_PLACEMENT_RANGE,
                self.journey.far_placement_range,
            ),
        ] {
            if let Ok(frame) = net_module::creative_power_toggle_full_state(id, &states) {
                self.send(slot, frame);
            }
        }

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
        // `LegacyMultiplayer.19` is `"{0} has joined."`.
        let who = NetworkText::literal(&name);
        self.announce_key("LegacyMultiplayer.19", vec![who]);

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
            // Which way they are looking. Only one thing reads it — a wiring tool's L turns the
            // other way depending on it — but that one thing is visible the moment it is wrong.
            player.facing_right = controls.facing_right();
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

    /// Packet 60 inbound: a player using the housing screen.
    ///
    /// This id is sent both ways. The server announces where each town NPC lives, which it already
    /// did — but the *client* sends the same packet to ask for a change, and that half was falling
    /// through to the ignore arm. So dragging an NPC into a room, or evicting one, did nothing at
    /// all on this server while looking like it had worked locally.
    ///
    /// Vanilla's server half (`MessageBuffer.cs` case 60, the `netMode != 1` branches) is two
    /// cases: a status byte of 1 evicts, anything else assigns the room at the given tile. It also
    /// boots a client whose NPC index is out of range as a cheat attempt; we decline the packet
    /// instead, since the transport is not the place to decide somebody is cheating.
    fn on_npc_home(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let index = r.i16()?;
        let home_x = r.i16()?;
        let home_y = r.i16()?;
        let evicting = r.u8()? == 1;

        let Ok(index) = u8::try_from(index) else {
            debug!(
                slot,
                index, "housing request for an npc slot that cannot exist"
            );
            return Ok(());
        };
        // Only town NPCs have homes; anything else is a client asking for something meaningless.
        let Some(npc) = self.npcs.get(index) else {
            return Ok(());
        };
        if !npc.stats.town_npc || !npc.is_alive() {
            return Ok(());
        }

        if evicting {
            if let Some(npc) = self.npcs.get_mut(index) {
                npc.home = None;
            }
            info!(slot, index, "town npc evicted");
        } else {
            // The room has to be one the game would accept, or a client could house a merchant
            // inside solid rock and the server would agree.
            match crate::game::housing::check_room(
                &self.world,
                i32::from(home_x),
                i32::from(home_y),
            ) {
                Ok(_) => {
                    if let Some(npc) = self.npcs.get_mut(index) {
                        npc.home = Some((i32::from(home_x), i32::from(home_y)));
                    }
                    info!(slot, index, home_x, home_y, "town npc moved in");
                }
                Err(why) => {
                    debug!(slot, index, ?why, "housing request refused");
                    // Tell the asker what it actually is, so their screen stops showing the move.
                    if let Some(frame) = self.npc_home_frame(index) {
                        self.send(slot, frame);
                    }
                    return Ok(());
                }
            }
        }
        // Everyone's housing screen has to agree, including the one that asked.
        self.broadcast_npc_home(index);
        Ok(())
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
        if npc >= 0 {
            self.try_rescue(npc as u8);
        }
        self.relay_player_packet(slot, id::SYNC_TALK_N_P_C, payload)
    }

    /// Talking to somebody tied up frees them.
    ///
    /// Six residents are found rather than earned, and the flag their arrival waits on is only
    /// ever set here. Without this the Mechanic could never appear, and she sells the only wire in
    /// the game — so an entire implemented subsystem sat unreachable behind one missing
    /// interaction.
    fn try_rescue(&mut self, index: u8) {
        let Some(npc) = self.npcs.get(index) else {
            return;
        };
        let Some(rescue) = crate::game::rescues::rescue_for(npc.npc_type) else {
            return;
        };

        if let Some(npc) = self.npcs.get_mut(index) {
            npc.become_type(rescue.freed);
        }
        crate::game::rescues::remember(&mut self.world.progress, rescue.freed);
        self.announce(rescue.announcement);
        self.broadcast_npc(index);
        self.broadcast_world_data();
        info!(freed = rescue.freed, "a bound townsperson was rescued");
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

        // Nor is an item frame, a mannequin, a hat rack or a food platter. These are never asked
        // for by packet — the game's placement request does nothing for them — so placing the
        // tile is the *only* moment they can come into existence. Without this an item frame is
        // scenery: it can be built and never holds anything.
        if let Some(kind) = terrustia_proto::tile_entity::EntityKind::for_tile(block) {
            self.add_tile_entity(kind, left as i16, top as i16);
        }

        // Everyone else places it themselves from the same packet.
        self.broadcast(
            terrustia_proto::packets::place_object(x, y, block, style, random)?,
            Some(slot),
        );
        debug!(slot, block, x, y, "object placed");
        Ok(())
    }

    /// `World::world_data`, with the ambient events that live on `GameServer` rather than `World`
    /// patched in — `PartyIsUp` (`self.party`), the same shape `self.army`'s own tier flags would
    /// need if `ArmyOngoing` were wired up (it is not, a real pre-existing gap this project already
    /// disclosed — `World::world_data`'s own comment on `DownedArmyTier1..3`). Every caller that
    /// sends packet 7 should go through this rather than `self.world.world_data()` directly, or a
    /// joining client learns everything about the world except whether a party is happening in it
    /// right now.
    fn world_data(&self) -> terrustia_proto::packets::WorldData {
        use terrustia_proto::packets::WorldFlag;
        let mut data = self.world.world_data();
        data.flags
            .set_flag(WorldFlag::PartyIsUp, self.party.is_up());
        data.flags
            .set_flag(WorldFlag::SlimeRain, self.slime_rain.is_active());
        data.flags
            .set_flag(WorldFlag::LanternNight, self.lantern_night.is_up());
        data
    }

    /// Tell everyone the world itself has changed — an eclipse begun, a blood moon risen.
    fn broadcast_world_data(&mut self) {
        if let Ok(frame) = self.world_data().encode() {
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
                    self.announce_key("LegacyMisc.20", Vec::new());
                    self.broadcast_world_data();
                }
            }
            -7 => self.start_invasion(Invasion::Martian),
            // A blood moon, which only rises at night and not twice in one night.
            -10 => {
                if !self.world.day_time && !self.world.blood_moon {
                    self.world.blood_moon = true;
                    self.announce_key("LegacyMisc.8", Vec::new());
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

        // A worm head spawned alone is a floating face: this is the same real trigger the evil
        // biome's own third-orb-break uses (`smash_orb`) and the one a real summon item's packet
        // reaches (`on_summon`), so a bodyless Eater of Worlds or Destroyer here is not a cosmetic
        // gap — it is the whole fight missing, since both bosses' own real damage/behaviour depend
        // on having a body at all. `/spawn`'s own admin command already knew to do this for the
        // four ordinary worm monsters; this was the one real path that never did.
        let spawned = match terrustia_proto::npc_params::worm_body(npc_type) {
            Some((body, tail, segments)) => {
                self.npcs.spawn_worm(npc_type, body, tail, segments, at)
            }
            None => self.npcs.spawn(npc_type, at),
        };
        if let Some(index) = spawned {
            let name = self
                .npcs
                .get(index)
                .map(|n| n.stats.name)
                .unwrap_or("Something");
            // `Announcement.HasAwoken` is `"{0} has awoken!"`, and its argument is itself a
            // keyed text — the NPC's name. Our internal names are exactly the game's `NPCName.*`
            // keys (`npc_data.rs` calls it "the NPCID constant name"), so the two line up without
            // a translation table.
            let who = NetworkText::key(format!("NPCName.{name}"), Vec::new());
            self.announce_key("Announcement.HasAwoken", vec![who]);
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
        // The last of the three identities arrives here, so this is the first moment a UUID ban
        // can be enforced. Name and address are checked earlier, at the handshake; a UUID cannot
        // be, because packet 68 comes after the slot is already assigned.
        self.enforce_ban(slot);
        Ok(())
    }

    /// Turn somebody away if any of their three identities is banned.
    ///
    /// Name, address and client UUID. `Player::uuid` was stored by this server and read by nothing
    /// at all until now; this is what it was for.
    fn enforce_ban(&mut self, slot: u8) {
        let Some(player) = self.player(slot) else {
            return;
        };
        let (name, address) = (player.name.clone(), player.addr.ip().to_string());
        let uuid = player.uuid.clone();

        // The guest list first, when there is one. Checked here rather than earlier because it is
        // keyed by name, and the name only arrives with the player's appearance.
        if !self.admin.welcome(&name) {
            info!(slot, %name, %address, "refusing somebody not on the guest list");
            self.kick(slot, "You are not on this server's guest list.");
            return;
        }

        let Some(ban) = self.admin.ban_for(&name, &address, uuid.as_deref()) else {
            return;
        };
        let reason = ban.reason.clone();
        info!(slot, %name, %address, reason, "refusing a banned player");
        self.kick(slot, &format!("You are banned: {reason}"));
    }

    /// Count one tile edit against this player's spam budget, and say whether to stop.
    ///
    /// Vanilla's, transcribed from `RemoteClient`: a counter per kind, bumped once per edit
    /// packet, decayed every tick, and the connection booted past a ceiling. Placing is the
    /// tightest (100, decaying 0.3 a tick, so ~18 a second sustained); breaking is deliberately
    /// loose (500, decaying 5 a tick) because mining is fast and legitimate.
    ///
    /// Not having this at all was a regression *from* vanilla rather than a place where we simply
    /// match how trusting vanilla is — which is why it sits inside "match vanilla's trust model"
    /// rather than being the TShock-style validation that stays deferred.
    fn note_tile_spam(&mut self, slot: u8, kind: TileAction) -> bool {
        let (counter, ceiling, why): (fn(&mut Player) -> &mut f32, f32, &str) = match kind {
            TileAction::KillTile | TileAction::KillTileNoItem | TileAction::KillWall => (
                |p| &mut p.spam_break,
                SPAM_BREAK_MAX,
                "breaking tiles too fast",
            ),
            _ => (
                |p| &mut p.spam_place,
                SPAM_PLACE_MAX,
                "placing tiles too fast",
            ),
        };
        let Some(player) = self.player_mut(slot) else {
            return true;
        };
        let count = counter(player);
        *count += 1.0;
        if *count <= ceiling {
            return false;
        }
        // Vanilla boots with `Net.CheatingTileSpam`; the reason travels as our own text because
        // it is the server talking, not the game.
        info!(slot, why, "disconnecting a client for edit spam");
        self.kick(slot, why);
        true
    }

    /// Let every player's spam budget recover, once a tick.
    fn tick_tile_spam(&mut self) {
        for player in self.players.iter_mut().flatten() {
            player.spam_place = (player.spam_place - SPAM_PLACE_DECAY).max(0.0);
            player.spam_break = (player.spam_break - SPAM_BREAK_DECAY).max(0.0);
            player.spam_liquid = (player.spam_liquid - SPAM_LIQUID_DECAY).max(0.0);
        }
    }

    fn on_tile_manipulation(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }

        let edit = TileManipulation::decode(payload)?;
        // Counted before anything else, so an edit that is refused still costs its budget. A
        // client hammering out-of-bounds coordinates is spamming just as hard as one hammering
        // valid ones.
        if self.note_tile_spam(slot, edit.kind()) {
            return Ok(());
        }
        let (x, y) = (i32::from(edit.x), i32::from(edit.y));
        if !self.world.in_bounds(x, y) {
            return Ok(());
        }

        // Vanilla parity: `MessageBuffer.cs`'s packet-17 handler (`case 17`) starts its own local
        // `flag14` from the client's own claimed failure bit, then forces it to `true` — never back
        // to `false` — the moment the edit's section isn't in `Netplay.Clients[whoAmI].TileSections`
        // (`RemoteClient.cs:31`, the exact state `Player::sent_sections` already mirrors here). That
        // combined flag is what every `WorldGen.KillTile`/`KillWall` call in that packet passes as
        // its own `fail` argument: a client editing a tile it was never sent a section for still
        // gets the swing animation, but the edit itself never actually lands — the same shape the
        // check below reproduces by folding into `changed` rather than dropping the packet outright.
        // Relaying regardless (below, unconditional on `changed`) is also load-bearing and already
        // vanilla-shaped: it is exactly how vanilla's own "this edit failed" state reaches every
        // other client too, not something this check needs to special-case.
        let (sx, sy) = self.world.section_of(x, y);
        let section_owned = self
            .player(slot)
            .is_some_and(|p| p.sent_sections.contains(&(sx, sy)));

        let mut tile = self.world.tile(x, y);
        // Snapshotted before any match arm below touches it, so `/world undo` can put back the
        // tile's whole state — not just the field this particular edit happened to change.
        let before = tile;
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

        // Never turns a rejected edit back into an accepted one — only ever suppresses one the
        // match above already decided should apply, matching `flag14`'s own one-way OR in source.
        changed = changed && section_owned;
        if changed {
            self.world.set_tile(x, y, tile);
            // Mining a block is the commonest way liquid starts moving.
            self.liquids.disturb(x, y);
            if let Some(name) = self.player(slot).map(|p| p.name.clone()) {
                self.tile_log.record(x, y, before, &name);
            }
        }
        // Gated on `section_owned`, not `changed`: a rejected edit still leaves `broke` set to
        // whatever the match arm above decided a real kill would drop, and applying these side
        // effects (a real item drop, an altar smash, waking a boss) without the tile ever actually
        // having been removed is exactly the exploit this whole check exists to close.
        if section_owned && let Some((block, frame_x, frame_y)) = broke {
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

    /// Packet 130: an NPC pulled out of the water on a fishing line.
    ///
    /// Fishing is how three town slimes and a handful of enemies arrive, and the Red Slime is a
    /// permanent unlock — catching one for the first time means the world can spawn them from
    /// then on. Placing the NPC is the server's, since only it owns the roster.
    fn on_fished_out_npc(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (i32::from(r.u16()?), i32::from(r.u16()?));
        let npc_type = r.i16()?;
        let Ok(npc_type) = u16::try_from(npc_type) else {
            return Ok(());
        };
        if !self.world.in_bounds(x, y) {
            return Ok(());
        }
        // Only what a rod can actually bring up. Without this the packet is a free spawn of
        // anything in the game, Moon Lord included.
        if !is_fishable(npc_type) {
            debug!(slot, npc_type, "that is not something you can fish out");
            return Ok(());
        }

        let at = (
            x as f32 * crate::game::npc::TILE,
            y as f32 * crate::game::npc::TILE,
        );
        if let Some(index) = self.npcs.spawn(npc_type, at) {
            self.broadcast_npc(index);
            debug!(slot, npc_type, "fished out an npc");
        }
        Ok(())
    }

    /// Packet 140: the two town slimes that are made rather than found.
    ///
    /// A Copper Slime and an Old Slime are each transformed from another slime, once per world,
    /// and the transformation is a permanent unlock. Both have to be the server's: the unlock is
    /// world state and the transformation is a roster change.
    fn on_misc_event_value(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let what = r.u8()?;
        let value = r.i32()?;
        let Ok(index) = u8::try_from(value) else {
            return Ok(());
        };

        let (wanted, into) = match what {
            TRANSFORM_COPPER_SLIME => (None, COPPER_SLIME),
            TRANSFORM_ELDER_SLIME => (Some(OLD_SLIME_SOURCE), OLD_SLIME),
            // Case 0 is the credits roll's clock, which only ever goes the other way.
            _ => return Ok(()),
        };
        let Some(npc) = self.npcs.get(index) else {
            return Ok(());
        };
        if let Some(wanted) = wanted
            && npc.npc_type != wanted
        {
            return Ok(());
        }
        if let Some(npc) = self.npcs.get_mut(index) {
            npc.become_type(into);
            npc.dirty = true;
        }
        self.broadcast_npc(index);
        Ok(())
    }

    /// Packet 141: Lucy the Axe having something to say.
    ///
    /// Pure flavour, and relayed rather than modelled — but a talking axe only its owner can hear
    /// is a talking axe nobody believes in.
    fn on_lucy_popup(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let frame = packets::verbatim(id::REQUEST_LUCY_POPUP, payload)?;
        self.broadcast(frame, Some(slot));
        Ok(())
    }

    /// Packet 85: quick stack — emptying an armful of loot into the chests it belongs in.
    ///
    /// The client offers a list of its own slots and the server decides where each goes, because
    /// only the server knows what is in chests nobody has opened and only the server can stop two
    /// players both being told the same slot was free.
    ///
    /// The client's word is taken for *which of its slots are eligible* — favourited items and
    /// coins are excluded, and that is the client's own bookkeeping — but never for where
    /// anything lands.
    fn on_quick_stack(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use crate::world::quick_stack::{self, Destination};
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let count = r.i32()?;
        if !(0..=MAX_QUICK_STACK_SLOTS).contains(&count) {
            debug!(slot, count, "refusing an implausible quick stack");
            return Ok(());
        }
        let mut offered = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let which = r.i16()?;
            let Ok(which) = u16::try_from(which) else {
                continue;
            };
            // What the slot holds is the server's own record, not the client's claim.
            if let Some(held) = self
                .player(slot)
                .and_then(|p| p.inventory.get(&which))
                .map(|e| e.item)
                .filter(|i| !i.is_empty())
            {
                offered.push((which, held));
            }
        }
        let smart = r.bool().unwrap_or(false);
        let _ = smart; // the sorting mode; the plain rule is the same either way here

        let Some(from) = self.player(slot).map(|p| p.position) else {
            return Ok(());
        };
        // A chest somebody has open is off limits, which is what stops a quick stack landing in
        // the middle of somebody else's rummaging.
        let open: Vec<i16> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.slot != slot)
            .map(|p| p.open_chest)
            .filter(|id| *id >= 0)
            .collect();
        let mut destinations: Vec<Destination> = self
            .world
            .chests
            .iter()
            .enumerate()
            .filter_map(|(id, c)| c.as_ref().map(|c| (id as i16, c)))
            .map(|(id, c)| Destination {
                id,
                position: (
                    f32::from(c.x) * crate::game::npc::TILE + crate::game::npc::TILE,
                    f32::from(c.y) * crate::game::npc::TILE + crate::game::npc::TILE,
                ),
                items: c.items.clone(),
                blocked: open.contains(&id),
            })
            .collect();

        let outcome = quick_stack::run(from, &offered, &mut destinations);
        if outcome.moves.is_empty() && outcome.blocked.is_empty() {
            return Ok(());
        }

        // Write the results back and tell everybody: the chests to everyone, since anyone may
        // have one open, and the player's own slots to the player.
        for movement in &outcome.moves {
            if let Some(Some(chest)) = self
                .world
                .chests
                .get_mut(usize::try_from(movement.chest).unwrap_or(usize::MAX))
                && let Some(cell) = chest.items.get_mut(movement.chest_slot)
            {
                *cell = movement.chest_now;
            }
            let frame = SyncChestItem {
                chest: movement.chest,
                slot: movement.chest_slot as u8,
                item: movement.chest_now,
            }
            .encode()?;
            self.broadcast(frame, None);

            if let Some(player) = self.player_mut(slot)
                && let Some(held) = player.inventory.get_mut(&movement.from_slot)
            {
                held.item = movement.left_behind;
            }
        }
        // One equipment packet per slot that changed, rather than one per move: a stack split
        // across three chests changed once from the player's point of view.
        let mut told = std::collections::HashSet::new();
        for movement in &outcome.moves {
            if !told.insert(movement.from_slot) {
                continue;
            }
            if let Some(equip) = self
                .player(slot)
                .and_then(|p| p.inventory.get(&movement.from_slot))
                .copied()
                && let Ok(frame) = equip.encode()
            {
                self.broadcast(frame, None);
            }
        }

        // Which chests refused, so the client can mark them.
        if !outcome.blocked.is_empty() {
            let mut w = terrustia_proto::PacketWriter::new(id::QUICK_STACK_CHESTS);
            w.i32(outcome.blocked.len() as i32);
            for chest in &outcome.blocked {
                w.u16(*chest as u16);
            }
            if let Ok(frame) = w.finish() {
                self.send(slot, frame);
            }
        }
        debug!(
            slot,
            moved = outcome.moves.len(),
            blocked = outcome.blocked.len(),
            "quick stack"
        );
        Ok(())
    }

    /// Packet 39: a player giving up their claim on an item.
    ///
    /// A dropped item is reserved for whoever is nearest so two players cannot both grab it. This
    /// is the other half of that: a player whose inventory is full, or who simply walked past,
    /// releases the claim so somebody else can have it. Without it a full player standing over a
    /// pile locks all of it for as long as they stand there.
    fn on_release_item(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let index = r.i16()?;
        let _force_to_server = r.bool()?;

        let Some(item) = self.items.get_mut(index) else {
            return Ok(());
        };
        // Only the holder may release it, or one client could free another's claim from under it.
        if item.owner != slot {
            return Ok(());
        }
        item.owner = items::NO_OWNER;
        item.reservation = 0;
        let position = item.position;
        // Told to everyone: the next tick will offer it to whoever is nearest, and until then no
        // client should believe it is spoken for.
        if let Ok(frame) = ItemOwner::reserve(index, items::NO_OWNER, position).encode() {
            self.broadcast(frame, None);
        }
        Ok(())
    }

    /// Packet 95: closing somebody else's portal.
    ///
    /// The Portal Gun's two ends are projectiles. Firing a third replaces the oldest, and the
    /// client that owns them says which one to close — because it is the one that knows which of
    /// its own pair is which.
    fn on_close_portal(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let owner = r.u16()?;
        let which = f32::from(r.u8()?);
        // The owner named has to be the sender, or one player could close another's portals.
        if usize::from(owner) != usize::from(slot) {
            return Ok(());
        }

        let found = self
            .projectiles
            .iter()
            .find(|(_, p)| {
                p.projectile_type == PORTAL_PROJECTILE && p.key.owner == slot && p.ai[1] == which
            })
            .map(|(index, p)| (index, p.key, p.position));
        let Some((index, key, position)) = found else {
            return Ok(());
        };
        self.projectiles.remove(index);
        let kill = terrustia_proto::projectile::KillProjectile { key, position };
        if let Ok(frame) = kill.encode() {
            self.broadcast(frame, None);
        }
        Ok(())
    }

    /// Packet 96: a player stepping through a portal.
    ///
    /// The client works out where it comes out — it knows where both ends are and how it entered
    /// — and the server's job is to agree and tell everybody else. Refusing would desync the one
    /// client that has already moved.
    fn on_portal_teleport(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let _claimed = r.u8()?;
        let colour = r.i16()?;
        let (x, y) = r.vec2()?;
        let velocity = r.vec2()?;
        if !x.is_finite() || !y.is_finite() || !velocity.0.is_finite() || !velocity.1.is_finite() {
            return Ok(());
        }
        // A portal only reaches as far as its other end, which the server can bound even without
        // knowing where that is: nothing on the map is further than the map.
        if !self.world.in_bounds(
            (x / crate::game::npc::TILE) as i32,
            (y / crate::game::npc::TILE) as i32,
        ) {
            return Ok(());
        }
        if let Some(player) = self.player_mut(slot) {
            player.position = (x, y);
            player.velocity = velocity;
        }
        let mut w = terrustia_proto::PacketWriter::new(id::TELEPORT_PLAYER_THROUGH_PORTAL);
        w.u8(slot)
            .i16(colour)
            .f32(x)
            .f32(y)
            .f32(velocity.0)
            .f32(velocity.1);
        let frame = w.finish()?;
        self.broadcast(frame, Some(slot));
        Ok(())
    }

    /// Packet 102: a Nebula armour booster being picked up.
    ///
    /// Purely a relay — the effect is each client's own — but without it nobody else sees the
    /// burst, and a booster picked up in a group looks to everyone else like nothing happened.
    fn on_nebula_booster(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let _claimed = r.u8()?;
        let kind = r.u16()?;
        let at = r.vec2()?;
        let mut w = terrustia_proto::PacketWriter::new(id::NEBULA_LEVELUP_REQUEST);
        w.u8(slot).u16(kind).f32(at.0).f32(at.1);
        let frame = w.finish()?;
        self.broadcast(frame, None);
        Ok(())
    }

    /// Packet 92: coins an NPC is carrying beyond its own worth.
    ///
    /// This is the Coin Loss revenge system: money dropped on death is remembered against
    /// whatever killed you, and killing that back gives it up. The server accumulates rather than
    /// overwrites, because two players can both feed the same enemy.
    fn on_extra_value(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let index = r.i16()?;
        let extra = r.i32()?;
        let at = r.vec2()?;
        let Ok(index) = u8::try_from(index) else {
            return Ok(());
        };
        let Some(npc) = self.npcs.get_mut(index) else {
            return Ok(());
        };
        npc.extra_value = npc.extra_value.saturating_add(extra);
        let total = npc.extra_value;
        let mut w = terrustia_proto::PacketWriter::new(id::SYNC_EXTRA_VALUE);
        w.i16(i16::from(index)).i32(total).f32(at.0).f32(at.1);
        let frame = w.finish()?;
        self.broadcast(frame, None);
        Ok(())
    }

    /// Packet 143: a player asking the Old One's Army to send the next wave early.
    ///
    /// The gap between waves is generous on purpose, and skipping it is how a group that is
    /// ready gets on with it. Refused unless the event is actually waiting.
    fn on_skip_army_wait(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        if let Some(left) = self.army.skip_wait() {
            self.broadcast_army_wait(left);
            debug!(slot, "the next army wave was called early");
        }
        Ok(())
    }

    /// Tell clients how long is left before the next wave comes through the gates.
    ///
    /// The countdown on screen is this and nothing else. Without it the gap between waves is a
    /// blank pause of unknown length, which is exactly the part of the event a group needs to
    /// plan around.
    fn broadcast_army_wait(&mut self, ticks: i32) {
        let mut w = terrustia_proto::PacketWriter::new(id::CRYSTAL_INVASION_SEND_WAIT_TIME);
        w.i32(ticks);
        if let Ok(frame) = w.finish() {
            self.broadcast(frame, None);
        }
    }

    /// Packet 144: the Dryad's little animation when a quest is handed in.
    ///
    /// Nothing but a flourish, and relayed rather than modelled — but a flourish only one client
    /// can see is worse than none.
    fn on_quest_effect(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let frame = packets::empty(id::REQUEST_QUEST_EFFECT)?;
        self.broadcast(frame, None);
        Ok(())
    }

    /// Packet 109: the Grand Design, run along a path.
    ///
    /// Every wiring tool past the first works this way, and it has to be the server's job: the
    /// client does not know how much wire the player has left, and a run that stops halfway has
    /// to stop at the same tile for everybody or two players see different circuits.
    ///
    /// The reply is packet 110 — how much was actually spent — which is what stops a client
    /// believing it still has wire the server has already used.
    fn on_mass_wire(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use crate::world::mass_wire::{self, Supplies, ToolMode};
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let from = (i32::from(r.i16()?), i32::from(r.i16()?));
        let to = (i32::from(r.i16()?), i32::from(r.i16()?));
        let mode = ToolMode(r.u8()?);
        if !mode.does_anything() {
            return Ok(());
        }

        // A drag across the whole world would be a denial of service dressed as a wiring tool.
        let span = (to.0 - from.0).abs().max((to.1 - from.1).abs());
        if span > MAX_WIRE_DRAG {
            debug!(slot, span, "refusing an implausibly long wire drag");
            return Ok(());
        }

        let Some(player) = self.player(slot) else {
            return Ok(());
        };
        let supplies = Supplies {
            wire: count_held(player, WIRE_ITEM),
            actuators: count_held(player, ACTUATOR_ITEM),
        };
        let facing_right = player.facing_right;

        let outcome = mass_wire::run(&mut self.world, from, to, mode, supplies, facing_right);
        for change in &outcome.changes {
            let edit = TileManipulation {
                action: change.action,
                x: change.x as i16,
                y: change.y as i16,
                arg: 0,
                style: 0,
            };
            if let Ok(frame) = edit.encode() {
                self.broadcast(frame, None);
            }
        }

        // Tell the player what it cost. Both are sent even when zero, as the game sends both.
        for (item, spent) in [
            (WIRE_ITEM, outcome.wire_spent),
            (ACTUATOR_ITEM, outcome.actuators_spent),
        ] {
            let mut w = terrustia_proto::PacketWriter::new(id::MASS_WIRE_OPERATION_PAY);
            w.i16(item).i16(spent as i16).u8(slot);
            if let Ok(frame) = w.finish() {
                self.send(slot, frame);
            }
        }
        debug!(
            slot,
            wire = outcome.wire_spent,
            actuators = outcome.actuators_spent,
            tiles = outcome.changes.len(),
            "mass wire operation"
        );
        Ok(())
    }

    /// Packet 69: a client asking what a chest is called.
    ///
    /// Sent for the map, which shows a chest's name without opening it. Answered to the asker
    /// alone, since another client that has not looked at the chest has no use for it.
    fn on_chest_name_request(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let claimed = r.i16()?;
        let (x, y) = (r.i16()?, r.i16()?);

        // A client may name the chest by id or ask the server to find it by position.
        let found = if claimed == -1 {
            self.world.chest_at(x, y)
        } else {
            self.world
                .chests
                .get(usize::try_from(claimed).unwrap_or(usize::MAX))
                .and_then(|c| c.as_ref())
                .filter(|c| c.x == x && c.y == y)
                .map(|c| (claimed, c))
        };
        let Some((id, chest)) = found else {
            return Ok(());
        };
        let mut w = terrustia_proto::PacketWriter::new(id::CHEST_NAME);
        w.i16(id).i16(x).i16(y).string(&chest.name);
        let frame = w.finish()?;
        self.send(slot, frame);
        Ok(())
    }

    /// Packet 105: locking or unlocking a gem lock.
    ///
    /// A tile toggle rather than a wiring one, but the effect is a circuit's: a locked gem lock
    /// is what a Chlorophyte Extractinator run is wired to.
    fn on_gem_lock(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (i32::from(r.i16()?), i32::from(r.i16()?));
        let lock = r.bool()?;
        if !self.world.in_bounds(x, y) {
            return Ok(());
        }
        let tile = self.world.tile(x, y);
        if !tile.is_active() || tile.block != GEM_LOCK {
            return Ok(());
        }
        // A gem lock is a three-by-three object whose locked state lives in the frame's Y band:
        // the lower half of the sprite sheet is the locked form.
        let origin_y = tile.frame_y % GEM_LOCK_FRAME_HEIGHT;
        let wanted = if lock {
            origin_y + GEM_LOCK_FRAME_HEIGHT
        } else {
            origin_y
        };
        if tile.frame_y == wanted {
            return Ok(());
        }
        let (ox, oy) = (
            x - i32::from(tile.frame_x % 54) / 18,
            y - i32::from(origin_y) / 18,
        );
        for dx in 0..3 {
            for dy in 0..3 {
                let (tx, ty) = (ox + dx, oy + dy);
                let mut cell = self.world.tile(tx, ty);
                if !cell.is_active() || cell.block != GEM_LOCK {
                    continue;
                }
                cell.frame_y = if lock {
                    cell.frame_y % GEM_LOCK_FRAME_HEIGHT + GEM_LOCK_FRAME_HEIGHT
                } else {
                    cell.frame_y % GEM_LOCK_FRAME_HEIGHT
                };
                self.world.set_tile(tx, ty, cell);
            }
        }
        // Two tiles of reach covers the whole three-by-three from its centre.
        self.push_region(ox + 1, oy + 1, 2);
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
        // Module 8: "take me to that pylon". Checked before chat because `IncomingChat::decode`
        // returns `None` for it and the request would otherwise be dropped on the floor.
        if let Some((message, pylon)) = net_module::decode_pylon_message(payload)?
            && message == net_module::PylonMessage::RequestTeleport
        {
            return self.on_pylon_teleport(slot, pylon);
        }

        // Module 4: Journey mode powers. Same reason as module 8 above — neither is chat, and
        // `IncomingChat::decode` would return `None` for it too, so it has to be checked first.
        if let Some(message) = net_module::decode_creative_power(payload)? {
            return self.on_creative_power(slot, message);
        }

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
        // Tagged so the web panel's live feed can tell an in-game chat line apart from an
        // operational one — both are `info!`, and only the target says which is which.
        info!(target: crate::term::CHAT_TARGET, "<{name}> {}", chat.text);

        // The text goes out bare, with the author's slot beside it. The client adds the name
        // itself — `ChatHelper.DisplayMessage` prefixes `Main.player[author].name` whenever the
        // author is a real slot — so a server that helpfully prefixes it too has every line
        // rendered with the speaker's name twice, and puts the tag inside the speech bubble over
        // their head as well. Found by asking a real server to relay a line and comparing: it
        // sends `"provoke: hello"` where this sent `"<provoke-actor> provoke: hello"`.
        //
        // The console line above keeps its own `<name>` because nothing is going to add one there.
        let frame = net_module::chat_broadcast(
            slot,
            &NetworkText::literal(chat.text.clone()),
            [255, 255, 255],
        )?;
        self.broadcast(frame, None);
        Ok(())
    }

    /// The commands about people rather than the world.
    ///
    /// Kept apart from the rest because they are the ones that need the argument's case intact —
    /// a lowercased password is a different password, and `run_command` lowercases everything for
    /// the benefit of NPC-name lookup.
    fn run_admin_command(
        &mut self,
        slot: u8,
        name: &str,
        argument: &str,
    ) -> terrustia_proto::Result<()> {
        use crate::admin::BanKind;

        let words: Vec<&str> = argument.split_whitespace().collect();
        match name {
            // The first account owns the server, so making it needs the token printed at
            // startup — otherwise, on a fresh public server, whoever connected first became the
            // owner. Every account after that is ordinary and needs nothing.
            "register" if self.admin.unclaimed() => match words.as_slice() {
                [account, password, token] => {
                    if self.claim_token.as_deref() != Some(*token) {
                        self.tell(
                            slot,
                            "that is not the claim token from the server's console.",
                        );
                        info!(slot, "refused a claim with the wrong token");
                        return Ok(());
                    }
                    self.begin_registration(slot, account, password, true);
                }
                [_, _] => self.tell(
                    slot,
                    "this server has not been claimed yet, and claiming it needs the claim token \
                     printed in the server's own console: /register <name> <password> <token>",
                ),
                _ => self.tell(slot, "usage: /register <name> <password> <token>"),
            },
            "register" => match words.as_slice() {
                [account, password] => self.begin_registration(slot, account, password, false),
                _ => self.tell(slot, "usage: /register <name> <password>"),
            },
            "login" => match words.as_slice() {
                [account, password] => {
                    // The hash is fetched here and compared on a worker thread. An account that
                    // does not exist still pays a hash, deliberately: answering instantly for an
                    // unknown name and slowly for a known one tells an attacker which is which.
                    let stored = self.admin.account_hash(account);
                    if self.start_auth(slot) {
                        let (account, password) = (account.to_string(), password.to_string());
                        let report = self.auth_results.0.clone();
                        tokio::task::spawn_blocking(move || {
                            let correct = match &stored {
                                Some(hash) => crate::admin::Account::verify_hash(hash, &password),
                                // No account: hash against a throwaway anyway, so the two cases
                                // take the same time.
                                None => {
                                    let _ = crate::admin::Account::new("", &password, "");
                                    false
                                }
                            };
                            let _ = report.send(AuthOutcome::SignedIn {
                                slot,
                                account,
                                correct,
                            });
                        });
                    }
                }
                _ => self.tell(slot, "usage: /login <name> <password>"),
            },
            "logout" => {
                self.admin.sign_out(slot);
                self.tell(slot, "signed out.");
            }
            "whoami" => {
                let who = self
                    .admin
                    .signed_in_as(slot)
                    .unwrap_or("nobody")
                    .to_string();
                let group = self.admin.group_of(slot).name.clone();
                self.tell(slot, &format!("you are {who}, in group '{group}'."));
            }
            "kick" => match words.split_first() {
                Some((who, rest)) => {
                    let reason = if rest.is_empty() {
                        "kicked".to_string()
                    } else {
                        rest.join(" ")
                    };
                    match self.slot_named(who) {
                        Some(target) => {
                            self.announce(&format!("{who} was kicked: {reason}"));
                            self.kick(target, &reason);
                        }
                        None => self.tell(slot, &format!("nobody here is called {who}.")),
                    }
                }
                None => self.tell(slot, "usage: /kick <name> [reason]"),
            },
            "ban" => match words.split_first() {
                Some((kind, rest)) if !rest.is_empty() => {
                    let Some(kind) = BanKind::parse(kind) else {
                        self.tell(slot, "usage: /ban <name|ip|uuid> <value> [reason]");
                        return Ok(());
                    };
                    let value = rest[0].to_string();
                    let reason = if rest.len() > 1 {
                        rest[1..].join(" ")
                    } else {
                        "banned".to_string()
                    };
                    self.admin.ban(kind, &value, &reason);
                    self.announce(&format!("{value} is banned: {reason}"));
                    // And remove them if they are standing here.
                    if let Some(target) = self.slot_named(&value) {
                        self.kick(target, &reason);
                    }
                    info!(value, reason, "ban added");
                }
                _ => self.tell(slot, "usage: /ban <name|ip|uuid> <value> [reason]"),
            },
            "unban" => match words.as_slice() {
                [value] => {
                    let removed = self.admin.unban(value);
                    self.tell(slot, &format!("{removed} ban(s) lifted for {value}."));
                }
                _ => self.tell(slot, "usage: /unban <value>"),
            },
            "group" => match words.as_slice() {
                [account, group] => {
                    if !self.admin.groups.iter().any(|g| &g.name == group) {
                        self.tell(slot, &format!("there is no group called {group}."));
                        return Ok(());
                    }
                    match self
                        .admin
                        .accounts
                        .iter_mut()
                        .find(|a| a.name.eq_ignore_ascii_case(account))
                    {
                        Some(found) => {
                            found.group = (*group).to_string();
                            let _ = self.admin.save();
                            self.tell(slot, &format!("{account} is now in {group}."));
                            info!(account, group, "group changed");
                        }
                        None => self.tell(slot, &format!("there is no account called {account}.")),
                    }
                }
                _ => self.tell(slot, "usage: /group <account> <group>"),
            },
            "world" => match words.as_slice() {
                &["undo", player, duration_text] => {
                    let Some(within) = crate::game::tile_log::parse_duration(duration_text) else {
                        self.tell(
                            slot,
                            "could not parse that duration — try something like 10m, 2h or 1d.",
                        );
                        return Ok(());
                    };
                    let reverted = self.tile_log.take_recent(player, within);
                    let count = reverted.len();
                    for (x, y, before) in reverted {
                        self.world.set_tile(x, y, before);
                        self.liquids.disturb(x, y);
                        self.broadcast_tile(x, y);
                    }
                    self.tell(
                        slot,
                        &format!(
                            "reverted {count} tile edit(s) by {player} from the last {duration_text}."
                        ),
                    );
                    info!(slot, player, duration_text, count, "world undo");
                }
                _ => self.tell(slot, "usage: /world undo <player> <duration>"),
            },
            _ => {}
        }
        Ok(())
    }

    /// The slot of whoever is playing under this name.
    fn slot_named(&self, name: &str) -> Option<u8> {
        self.players
            .iter()
            .flatten()
            .find(|p| p.is_playing() && p.name.eq_ignore_ascii_case(name))
            .map(|p| p.slot)
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
    /// Commands are gated by the permission table below: `time`, `save`, `spawn` and `butcher`
    /// need `World`, `kick`/`ban`/`unban` need `Players`, `group` needs `Admin`, and the rest are
    /// read-only or something any player could do anyway. Until somebody registers, the server is
    /// unclaimed and every check passes — see `Admin::unclaimed`.
    fn run_command(&mut self, slot: u8, command: &str) -> terrustia_proto::Result<()> {
        use crate::admin::Permission;

        let mut parts = command.split_whitespace();
        let name = parts.next().unwrap_or("").to_ascii_lowercase();
        // The whole rest of the line, not the first word of it: `/spawn Eater of Worlds Head` has
        // to reach the resolver intact or it looks up "eater" and finds nothing.
        let argument = parts.collect::<Vec<_>>().join(" ").to_ascii_lowercase();
        // And the same line with its case intact, because a password, an account name and a
        // player's name are all case-sensitive and the lowercased form silently corrupts them.
        let raw_argument = command
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
            .join(" ");

        // What each command costs. Anything absent needs nothing beyond being here.
        let needed = match name.as_str() {
            "time" | "save" | "spawn" | "butcher" => Some(Permission::World),
            "kick" | "ban" | "unban" | "world" => Some(Permission::Players),
            "group" => Some(Permission::Admin),
            _ => None,
        };
        if let Some(permission) = needed
            && !self.admin.may(slot, permission)
        {
            // Named rather than vague: "you may not" invites a second attempt, and the point is to
            // tell somebody how to become allowed.
            self.tell(
                slot,
                &format!(
                    "/{name} needs the '{}' permission. Sign in with /login <name> <password>.",
                    permission.as_str()
                ),
            );
            return Ok(());
        }

        match name.as_str() {
            "register" | "login" | "logout" | "kick" | "ban" | "unban" | "group" | "whoami"
            | "world" => {
                return self.run_admin_command(slot, &name, &raw_argument);
            }
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
                    "/register <name> <password>   make an account",
                    "/login <name> <password>      sign in",
                    "/logout          give up whatever you signed in for",
                    "/whoami          who the server thinks you are",
                    "/kick <name> [reason]",
                    "/ban <name|uuid|ip> <value> [reason]",
                    "/unban <value>",
                    "/group <account> <group>      move somebody between groups",
                    "/world undo <player> <duration>   revert their tile edits from the last",
                    "                                   <duration> (e.g. 10m, 2h) — up to 72h back",
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
                        self.set_time(day_time, time)?;
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
                    self.save_world_in_background("command");
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
                let spawned = match terrustia_proto::npc_params::worm_body(npc_type) {
                    Some((body, tail, segments)) => {
                        self.npcs.spawn_worm(npc_type, body, tail, segments, at)
                    }
                    None => self.npcs.spawn(npc_type, at),
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

/// Ticks between passes of the world growing.
///
/// Slower than the biome spread because grass has nowhere to be: the game runs its own tile
/// updates every tick over the whole world, and a sixth of that around the players is more than
/// enough for a field to green over while somebody is looking at it.
const GROWTH_EVERY: u64 = 10;
const SPREAD_TRIES: usize = 3;
const SPREAD_RANGE: i32 = 120;

/// Tree tile types that share vanilla's own `KillTile_GetTreeDrops` branch (`WorldGen.cs`, `case
/// 5: case 596: case 616: case 634:`). Only 5 (ordinary trees) is ever worldgen-placed by this
/// project today (`world::trees`); the vanity-tree and ash-tree variants are included anyway
/// since it costs nothing and the item->tile placement table already knows their ids.
const TREE_TILES: [u16; 4] = [5, 596, 616, 634];

fn is_tree_tile(block: u16) -> bool {
    TREE_TILES.contains(&block)
}

/// `WorldGen.GetTreeType`'s own enum, narrowed to the species this project's ground types can
/// actually produce (see [`GameServer::tree_species_at`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum TreeSpecies {
    None,
    Forest,
    Corrupt,
    Crimson,
    Jungle,
    Hallowed,
    Snow,
    Mushroom,
}

/// `WorldGen.TreeTypeDropsAcorns`: every species drops an acorn from its canopy except a tree with
/// no resolved species, and the two whose canopy drops something else instead (a mushroom cap, a
/// Rich Mahogany sapling players plant deliberately rather than find as a bonus).
fn tree_drops_acorns(species: TreeSpecies) -> bool {
    !matches!(
        species,
        TreeSpecies::None | TreeSpecies::Mushroom | TreeSpecies::Jungle
    )
}

/// Item ids for tree drops (`ItemID.cs`): plain Wood, the six biome-specific woods, an Acorn, and
/// what a Mushroom-biome tree gives instead of wood.
const WOOD: i32 = 9;
const ACORN: i32 = 27;
const EBONWOOD: i32 = 619;
const RICH_MAHOGANY: i32 = 620;
const PEARLWOOD: i32 = 621;
const SHADEWOOD: i32 = 911;
const GLOWING_MUSHROOM: i32 = 183;
const BOREAL_WOOD: i32 = 2503;

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
            self.effective_difficulty(),
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
        // Things a routine killed, as opposed to things that wandered off.
        let mut slain: Vec<(u8, u16, (f32, f32), f32)> = Vec::new();
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
            // Where hostile NPCs are, for town residents fighting back — built once here rather
            // than scanning the whole table per resident, the same reasoning `hurt` above uses
            // for the Dark Mage.
            let hostiles: Vec<Hostile> = if self.npcs.iter().any(|(_, n)| {
                n.stats.town_npc && crate::game::ai::town_combat::town_combat(n.npc_type).is_some()
            }) {
                self.npcs
                    .iter()
                    .filter(|(_, n)| !n.stats.friendly && !n.stats.town_npc && n.is_alive())
                    .map(|(slot, n)| (slot, n.center(), n.velocity))
                    .collect()
            } else {
                Vec::new()
            };
            // The Brain of Cthulhu's own centre, for its Creepers' `ai[2..3]` (ai_style 55 in
            // `ai/mod.rs`, whose own comment already says this is the server's job: "The Brain's
            // position is threaded in through ai[2..3] by the server, which knows where every NPC
            // is"). Nothing ever did that threading, so every Creeper read `ai[2] == ai[3] ==
            // 0.0` — its own untouched default — on every one of its own ticks and asked to be
            // removed (`creeper::update`'s `BrainGone` branch) from the moment it spawned. It only
            // ever looked alive because `tick_life`'s ordinary despawn timer resets right back
            // over that removal for as long as a player stands nearby, and lets the removal
            // through the instant one does not — indistinguishable, from a client's own tracked
            // view, from a boss whose escort simply never reliably syncs. Scanned only when a
            // Creeper actually exists, the same guard `hurt`/`hostiles` above use.
            let brain_center: Option<(f32, f32)> = self
                .npcs
                .iter()
                .any(|(_, n)| n.npc_type == terrustia_proto::npc_params::CREEPER)
                .then(|| {
                    self.npcs
                        .iter()
                        .find(|(_, n)| n.npc_type == 266)
                        .map(|(_, n)| n.center())
                })
                .flatten();
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
            let conditions = self.ai_conditions(biome);
            for (index, npc) in self.npcs.iter_mut() {
                // Segments are positioned by their leader, not by a routine of their own.
                if npc.follows.is_some() {
                    continue;
                }
                // See `brain_center` above: a Creeper reads its own escort centre from here.
                if npc.npc_type == terrustia_proto::npc_params::CREEPER {
                    let (bx, by) = brain_center.unwrap_or((0.0, 0.0));
                    npc.ai[2] = bx;
                    npc.ai[3] = by;
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
                        // The nearest *visible* hostile a resident might fight back against — see
                        // `nearest_visible_hostile`'s own doc comment for why this is filtered on
                        // line of sight before distance is compared, not merely left for
                        // `try_combat` to refuse later.
                        hostile: if npc.stats.town_npc {
                            nearest_visible_hostile(&tiles, npc, &hostiles)
                        } else {
                            None
                        },
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
                        slot: index,
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
                // A routine that decided this one is dead — a burst spore, an uprooted plant, a
                // fallen lunar pillar, the Moon Lord finishing its ten seconds of coming apart.
                //
                // `effects.died` only sets the life to zero; nothing reaped it, so these lingered
                // at zero health forever and **never dropped anything or recorded the kill**. For
                // the Moon Lord that meant beating the game left no luminite and no flag: the
                // world did not notice you had won.
                if npc.life <= 0 {
                    slain.push((
                        index,
                        npc.npc_type,
                        npc.center(),
                        if npc.from_statue {
                            0.0
                        } else {
                            npc.stats.value
                        },
                    ));
                } else if npc.time_left <= 0 {
                    // Outside the world is a separate reason from running out of time, and it has
                    // to be here or nothing catches it: a flying routine that keeps its vertical
                    // velocity does not turn round at the sky, so a bat or a bird leaves through
                    // the top and carries on for ever. Found in a five-minute capture where one
                    // reached y = -8338 — five hundred tiles above the world — and was still being
                    // simulated and broadcast to every client, five hundred and fifteen times, at
                    // coordinates nothing can draw. The game's own check is the same four-sided
                    // hundred-pixel margin.
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
                crate::game::ai::town::DoorAction::Close { x, y } => self.close_door(x, y),
                crate::game::ai::town::DoorAction::None => {}
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

        // A town resident's melee attack landing on a nearby hostile. Mirrors `on_damage_npc`'s
        // death handling, minus the parts that only make sense for a player-originated hit (an
        // ack, a stale-generation check against a client's own aim, a crit roll).
        for hit in std::mem::take(&mut ai_out.melee_hits) {
            let Some(npc) = self.npcs.get_mut(hit.target) else {
                continue;
            };
            let killed = npc.take_damage(hit.damage, hit.knockback, hit.direction);
            let (npc_type, center) = (npc.npc_type, npc.center());
            let value = if npc.from_statue {
                0.0
            } else {
                npc.stats.value
            };
            if killed {
                self.npc_died(hit.target, npc_type, center, value);
            } else {
                self.broadcast_npc(hit.target);
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

        // Deaths first: `npc_died` drops the loot and records the kill, which is the difference
        // between beating the Moon Lord and merely making it go away.
        for (index, npc_type, center, value) in slain {
            self.npc_died(index, npc_type, center, value);
        }

        for index in expired {
            self.npcs.remove(index);
            // A silently vanished NPC would linger on every client, so tell them it is gone.
            self.broadcast_npc_death(index);
        }
        self.resolve_worm_chains();

        // How often an NPC's full state goes out, and to whom.
        //
        // Ported from `NPC.UpdateNetworkCode` and `NPC.StreamUpdatesToNearbyPlayers`, which are two
        // mechanisms rather than one and only make sense together:
        //
        // * a **token bucket** limits full syncs to one per thirty ticks sustained — five for a
        //   boss — with three allowed back to back on top of that;
        // * **proximity streaming** then tops that up for anything actually moving, weighted by how
        //   near each player is, so a creature you are standing next to updates several times a
        //   second while the same creature across the world does not.
        //
        // This server previously had neither, and sent every changed NPC every six ticks to
        // everyone nearby: twenty times the game's sustained rate, measured at seven times its
        // bandwidth over a five-minute capture against the real server on the same world.
        self.tick_npc_syncs();
    }

    /// One tick of NPC network bookkeeping: the rate-limited full sync, then the proximity stream.
    fn tick_npc_syncs(&mut self) {
        // ---- full syncs, rate limited ------------------------------------------------------
        let ready: Vec<u8> = self
            .npcs
            .iter()
            .filter(|(_, npc)| npc.dirty)
            .map(|(index, _)| index)
            .collect();
        for index in ready {
            let Some(npc) = self.npcs.get_mut(index) else {
                continue;
            };
            let cost = if npc.stats.boss {
                crate::game::npc::NET_SPAM_PER_PACKET_BOSS
            } else {
                crate::game::npc::NET_SPAM_PER_PACKET
            };
            if npc.net_spam > crate::game::npc::NET_SPAM_PACKET_LIMIT * cost {
                // Out of tokens. It stays dirty and is tried again next tick, which is what makes
                // this a delay rather than a dropped update.
                continue;
            }
            npc.net_spam += cost;
            npc.dirty = false;
            SYNC_FULL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // Cleared before sending and put back if anybody was skipped. Clearing it
            // unconditionally silently loses a one-off change to a distant NPC: it is marked dirty,
            // the one broadcast it earns is withheld from every faraway player, and since nothing
            // changes it again it is never sent. A player who was elsewhere at that moment sees
            // that NPC at its old health for the rest of the session.
            if self.broadcast_npc(index)
                && let Some(npc) = self.npcs.get_mut(index)
            {
                npc.dirty = true;
            }
        }

        // The bucket drains a tick at a time, which is what sets the sustained rate.
        for (_, npc) in self.npcs.iter_mut() {
            if npc.net_spam > 0 {
                npc.net_spam -= 1;
            }
        }

        // ---- proximity streaming ------------------------------------------------------------
        //
        // Only for things that are moving: a stationary creature has nothing to interpolate and is
        // already correct on every client that has been told about it once.
        let streaming: Vec<(u8, (f32, f32))> = self
            .npcs
            .iter_mut()
            .filter(|(_, npc)| {
                !npc.stats.town_npc
                    && npc.velocity.0.abs() + npc.velocity.1.abs() > 0.5
                    // The three the game excludes from proximity syncing, via
                    // `NPCID.Sets.UsesMultiplayerProximitySyncing`.
                    && !matches!(npc.npc_type, 396..=398)
            })
            .filter_map(|(index, npc)| {
                npc.net_stream = npc.net_stream.saturating_add(1);
                if npc.net_stream < crate::game::npc::NPC_STREAM_SPEED {
                    return None;
                }
                npc.net_stream = 0;
                Some((index, npc.center()))
            })
            .collect();

        for (index, at) in streaming {
            let watchers: Vec<(u8, (f32, f32))> = self
                .players
                .iter()
                .flatten()
                .filter(|p| p.is_playing())
                .map(|p| (p.slot, p.position))
                .collect();
            for (slot, position) in watchers {
                let distance = ((position.0 - at.0).powi(2) + (position.1 - at.1).powi(2)).sqrt();
                let weight = stream_weight(distance);
                if weight == 0 {
                    continue;
                }
                let counter = self.npc_stream.entry((index, slot)).or_insert(0);
                *counter = counter.saturating_add(weight);
                if *counter < STREAM_THRESHOLD {
                    continue;
                }
                *counter = 0;
                SYNC_STREAM.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if let Some(sync) = self.npc_sync(index)
                    && let Ok(frame) = sync.encode()
                {
                    self.send(slot, frame);
                }
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
    fn on_town_npc_name_request(
        &mut self,
        slot: u8,
        payload: &[u8],
    ) -> terrustia_proto::Result<()> {
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
                // `LegacyMisc.19` is `"{0} was slain..."`. Vanilla carries a structured
                // `PlayerDeathReason` for the richer forms; this is the plain one.
                let who = NetworkText::literal(name);
                self.announce_key("LegacyMisc.19", vec![who]);
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
        // Journey mode's `Godmode`. Real vanilla's own `creativeGodMode` gates apply client-side
        // (`Player.cs:31557`/`38486`/`39107`), since most damage in that game is client-decided —
        // this is the one place *this* server decides damage on a player's behalf at all (NPC
        // contact and NPC-thrown projectiles, this function's only two call sites), so it is the
        // one place this server needs its own gate to match.
        if self.journey.is_godmode(slot as u8) {
            return;
        }
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

    /// Returns whether the frame was withheld from at least one player, so the caller knows this
    /// NPC still owes somebody an update.
    fn broadcast_npc(&mut self, index: u8) -> bool {
        let Some(sync) = self.npc_sync(index) else {
            return false;
        };
        let Ok(frame) = sync.encode() else {
            return false;
        };
        let at = sync.position;
        self.broadcast_near(frame, at, index)
    }

    /// Send an NPC's state only to the players whose part of the world it is in.
    ///
    /// A broadcast to everybody is what a server can least afford: with two hundred NPCs awake and
    /// a sync every six ticks, sending each to every player is thousands of frames a second per
    /// client, and a client that cannot drain that fast is dropped for being slow. The game's own
    /// rule is to skip an NPC for a client whose loaded sections do not cover it — but never more
    /// than four times in a row, so something far away still gets an occasional update rather than
    /// freezing where it was last seen.
    /// Returns whether anybody was skipped, which is the caller's cue to try again next interval.
    fn broadcast_near(&mut self, frame: Vec<u8>, at: (f32, f32), index: u8) -> bool {
        let bytes = Bytes::from(frame);
        let mut withheld = false;
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
                    withheld = true;
                    continue;
                }
            }
            self.npc_skips.remove(&(index, slot));
            self.send_bytes(slot, bytes.clone());
        }
        withheld
    }

    /// Carry out what a fighter decided to do to a door.
    /// Pull a door shut behind a resident who has walked through it.
    ///
    /// The close half of this was being dropped on the floor: a town NPC produced the action, the
    /// server matched it and did nothing, and `ai/town.rs` documented the behaviour as "opened and
    /// then closed behind it". So every door an NPC ever used stayed open on every client, and at
    /// night that is the difference between a sealed house and an invitation.
    ///
    /// `action: 1` is the game's close, against `0` for open (`MessageBuffer.cs:1310`).
    fn close_door(&mut self, x: i32, y: i32) {
        if !self.world.in_bounds(x, y) {
            return;
        }
        if !crate::world::doors::close(&mut self.world, x, y) {
            return;
        }
        let toggle = terrustia_proto::objects::DoorToggle {
            action: 1,
            x: x as i16,
            y: y as i16,
            direction: 0,
        };
        if let Ok(frame) = toggle.encode() {
            self.broadcast(frame, None);
        }
    }

    fn apply_door_action(&mut self, action: crate::game::ai::fighter::Action) {
        use crate::game::ai::fighter::Action;
        match action {
            Action::None => {}
            Action::OpenDoor { x, y, direction } => {
                // Move the tiles, then tell everyone. Broadcasting alone — which this used to do,
                // on the reasoning that every client would open it for itself — left the *server*
                // believing the door was still shut, so the NPC standing at it decided to open it
                // again on its next look, and again, for ever. On a world with a town that came to
                // eighteen thousand door packets in five minutes and half of all traffic.
                if !crate::world::doors::open(&mut self.world, x, y, direction) {
                    return;
                }
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
        // Read before the removal takes it: `Conditions.IsBloodMoonAndNotFromStatue` cares whether
        // *this* NPC came from a statue, not just whether one exists somewhere in the world.
        let from_statue = self.npcs.remove(index).is_some_and(|npc| npc.from_statue);
        self.broadcast_npc_death(index);
        self.drop_coins(value, center);
        self.drop_loot(npc_type, center, from_statue);
        self.note_invasion_kill(npc_type);
        self.army.note_corpse(npc_type, (center.0, center.1 + 16.0));
        self.note_army_kill(npc_type);
        self.note_moon_kill(npc_type);
        self.lunar.note_kill(npc_type);
        self.note_banner_kill(npc_type, center);
        self.note_boss_kill(npc_type);
        self.note_slime_rain_kill(npc_type, center);
    }

    /// `DoDeathEvents_AdvanceSlimeRain`. Advances the kill count while a rain is up and, once the
    /// threshold is reached, summons King Slime at the *closest* player to this kill
    /// (`SpawnOnPlayer(closestPlayer.whoAmI, 50)`, real vanilla's own choice — not a random one).
    fn note_slime_rain_kill(&mut self, npc_type: u16, center: (f32, f32)) {
        let king_slime_present = self
            .npcs
            .iter()
            .any(|(_, n)| n.npc_type == crate::game::slime_rain::KING_SLIME);
        let summon = self.slime_rain.note_kill(
            npc_type,
            king_slime_present,
            self.world.progress.downed_king_slime,
        );
        if !summon {
            return;
        }
        let closest = self
            .players
            .iter()
            .enumerate()
            .filter_map(|(slot, p)| p.as_ref().map(|p| (slot as u8, p)))
            .filter(|(_, p)| p.is_playing())
            .min_by(|(_, a), (_, b)| {
                let da = (a.position.0 - center.0).powi(2) + (a.position.1 - center.1).powi(2);
                let db = (b.position.0 - center.0).powi(2) + (b.position.1 - center.1).powi(2);
                da.total_cmp(&db)
            })
            .map(|(slot, _)| slot);
        if let Some(slot) = closest {
            self.summon_on_player(slot, crate::game::slime_rain::KING_SLIME);
        }
    }

    /// Drop whatever an NPC was carrying.
    ///
    /// Each chain — both the unconditional ones in `drop_flat_loot`, below, and the classic-only
    /// ones `conditional_chains` returns (Queen Bee's Hive Wand/Bee-armor, Skeletron's three
    /// weapons, King Slime's Slime Hook/Slime Gun) — is rolled in order and stops at the first
    /// success, which is what keeps a run of alternatives rare rather than giving independent
    /// chances at every one of them.
    ///
    /// On top of that come the drops that depend on the world rather than the thing that died: a
    /// treasure bag in expert, a trophy, and the hardmode materials that only exist once the wall
    /// has fallen.
    fn drop_loot(&mut self, npc_type: u16, center: (f32, f32), from_statue: bool) {
        let (tx, ty) = (
            (center.0 / crate::game::npc::TILE) as i32,
            (center.1 / crate::game::npc::TILE) as i32,
        );
        let ground = self.world.tile(tx, ty).block;
        let p = &self.world.progress;
        let at = terrustia_proto::conditional_drops::Conditions {
            expert: self.is_expert(),
            master: self.is_master(),
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
            // The sibling has to be gone already, and the one that just died is still in the
            // roster at this point, so it is excluded by index rather than by type.
            other_twin_dead: !self
                .npcs
                .iter()
                .any(|(_, n)| matches!(n.npc_type, 125 | 126) && n.is_alive()),
            blood_moon: self.world.blood_moon,
            npc_from_statue: from_statue,
            eclipse: self.world.eclipse,
            downed_mech_any: p.downed_mech_any,
            downed_all_mech_bosses: p.downed_mech1 && p.downed_mech2 && p.downed_mech3,
            pumpkin_moon_wave: matches!(self.moon.moon, Some(crate::game::moons::Moon::Pumpkin))
                .then_some(self.moon.wave),
        };

        // Pools that give exactly one of their options.
        for pool in terrustia_proto::conditional_drops::one_from(npc_type, at) {
            let pick = pool[rand::Rng::random_range(&mut self.rng, 0..pool.len())];
            if let Some(index) = self
                .items
                .spawn(ItemStack::new(i32::from(pick), 1, 0), center)
            {
                self.broadcast_item(index);
            }
            // Some picks bring a companion item along automatically — Golem's Stynger with its
            // own ammunition (`ItemDropDatabase.cs:654-656`), the only one of these this
            // project's drop tables have found so far. Unconditional once the pick lands: real
            // vanilla's own nested `OnSuccess` has no further gate of its own.
            if let Some((companion, min, max)) =
                terrustia_proto::conditional_drops::bundled_with(pick)
            {
                let stack = if max > min {
                    rand::Rng::random_range(&mut self.rng, min..=max)
                } else {
                    min
                };
                if let Some(index) = self
                    .items
                    .spawn(ItemStack::new(i32::from(companion), stack, 0), center)
                {
                    self.broadcast_item(index);
                }
            }
        }
        // Moon Lord: two *distinct* items drawn from his own ten-weapon pool
        // (`FromOptionsWithoutRepeatsDropRule`) — empty for every other npc and in expert mode, so
        // this is a no-op there. Mirrors the game's own algorithm exactly: pick one index, then
        // pick a second uniformly from what remains, rather than drawing from `one_from`'s
        // independent-per-pool mechanism, which could otherwise repeat the same weapon.
        let moon_lord_pool = terrustia_proto::conditional_drops::moon_lord_weapons(npc_type, at);
        if moon_lord_pool.len() >= 2 {
            let first = rand::Rng::random_range(&mut self.rng, 0..moon_lord_pool.len());
            let mut second = rand::Rng::random_range(&mut self.rng, 0..moon_lord_pool.len() - 1);
            if second >= first {
                second += 1;
            }
            for &item in &[moon_lord_pool[first], moon_lord_pool[second]] {
                if let Some(index) = self
                    .items
                    .spawn(ItemStack::new(i32::from(item), 1, 0), center)
                {
                    self.broadcast_item(index);
                }
            }
        }
        // Chance-gated pools: roll the gate first, and only on success pick which option.
        for pool in terrustia_proto::conditional_drops::chance_pools(npc_type, at) {
            if pool.one_in > 1 && !rand::Rng::random_ratio(&mut self.rng, 1, pool.one_in) {
                continue;
            }
            let pick = pool.options[rand::Rng::random_range(&mut self.rng, 0..pool.options.len())];
            if let Some(index) = self
                .items
                .spawn(ItemStack::new(i32::from(pick), 1, 0), center)
            {
                self.broadcast_item(index);
            }
        }
        for rule in terrustia_proto::conditional_drops::conditional(npc_type, at) {
            // Almost every rule here is a plain 1-in-`one_in` roll, but a handful of real vanilla
            // rules (`CommonDrop`/`ByCondition`'s own `chanceNumerator`) roll `M`-in-`N` instead —
            // `rule.numerator` is `1` for everything but those, so this is exactly the old roll for
            // every rule that never needed the field.
            if rule.one_in > 1
                && !rand::Rng::random_ratio(&mut self.rng, rule.numerator, rule.one_in)
            {
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
        // Fallback chains among the classic-only rolls (Queen Bee's Hive Wand/Bee-armor,
        // Skeletron's three weapons, King Slime's Slime Hook/Slime Gun): stop at the first link
        // that lands, the same break-on-first-success shape `drop_flat_loot` already has below —
        // these three cannot live in the flat table itself, which has no notion of expert/classic
        // mode at all (see `conditional_chains`'s own doc for why).
        for chain in terrustia_proto::conditional_drops::conditional_chains(npc_type, at) {
            for rule in chain {
                if rule.one_in > 1
                    && !rand::Rng::random_ratio(&mut self.rng, rule.numerator, rule.one_in)
                {
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

        // House any resident that does not have one yet — but never the Old Man, the Travelling
        // Merchant or the Skeleton Merchant, none of whom ever seek a house in real vanilla
        // (`WorldGen.FindAnyHomelessTownNPC`'s own exclusion list, `nPC.type != 37 && != 453 &&
        // != 368`). Without this, a real, reproducible bug: the Old Man is a real, already-
        // homeless town NPC by design (he haunts the dungeon entrance, never moves in anywhere),
        // so any tick where he happens to be nearby when a freshly-built house first becomes
        // findable, he claims it ahead of whichever real newcomer that house was built for — found
        // live when `moonlord.rs`'s own Guide-house trigger lost this exact race to the Old Man,
        // 2-for-2, on real full runs.
        let homeless: Vec<u8> = self
            .npcs
            .iter()
            .filter(|(_, npc)| {
                npc.stats.town_npc
                    && npc.home.is_none()
                    && !matches!(
                        npc.npc_type,
                        OLD_MAN | TRAVELLING_MERCHANT | SKELETON_MERCHANT
                    )
            })
            .map(|(index, _)| index)
            .collect();

        // Who would arrive if there were somewhere to put them. Worked out before the house
        // search because that search is the expensive half and there is no point paying for it
        // when nobody is homeless and nobody is waiting.
        let guide_present = self.npcs.iter().any(|(_, n)| n.npc_type == GUIDE);
        let newcomer = if guide_present {
            self.next_arrival()
        } else {
            Some((GUIDE, "Guide"))
        };
        if homeless.is_empty() && newcomer.is_none() {
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
            // Where they now live, so every client's housing screen shows the room as taken
            // rather than as still empty.
            self.broadcast_npc_home(*index);
            return;
        }

        // Nobody is homeless, so the newcomer worked out above can take the house.
        let Some((npc_type, name)) = newcomer else {
            return;
        };

        if let Some(index) = self
            .npcs
            .spawn(npc_type, (hx as f32 * 16.0, (hy - 3) as f32 * 16.0))
        {
            if let Some(npc) = self.npcs.get_mut(index) {
                npc.home = Some(house);
            }
            self.announce(&format!("The {name} has moved in."));
            self.broadcast_npc(index);
            self.broadcast_npc_home(index);
        }
    }

    /// Copy the live townsfolk into the world, so a save records who actually lives here.
    ///
    /// The world file's NPC section used to be carried through untouched, which meant every
    /// resident was a session-long guest: their name was regenerated on the next start and their
    /// house forgotten.
    fn record_town_npcs(&mut self) {
        let residents: Vec<crate::world::objects::TownNpc> = self
            .npcs
            .iter()
            .filter(|(_, npc)| npc.stats.town_npc && npc.is_alive())
            .map(|(_, npc)| {
                let home = npc.home.unwrap_or((0, 0));
                crate::world::objects::TownNpc {
                    net_id: i32::from(npc.npc_type),
                    name: npc.given_name.clone(),
                    position: npc.position,
                    homeless: npc.home.is_none(),
                    home,
                    variation: npc.town_variation,
                    homeless_despawn: false,
                }
            })
            .collect();
        self.world.town_npcs = residents;
    }

    /// Put a loaded world's townsfolk back into the roster.
    ///
    /// Called once at startup. Without it a world with a full town opened here empty, and the
    /// arrival logic would slowly re-invite everyone under new names.
    fn restore_town_npcs(&mut self) {
        let saved = std::mem::take(&mut self.world.town_npcs);
        let mut restored = 0usize;
        for npc in &saved {
            let Ok(npc_type) = u16::try_from(npc.net_id.max(0)) else {
                continue;
            };
            let Some(index) = self.npcs.spawn(npc_type, npc.position) else {
                break; // out of slots
            };
            if let Some(live) = self.npcs.get_mut(index) {
                live.given_name = npc.name.clone();
                live.town_variation = npc.variation;
                live.home = (!npc.homeless).then_some(npc.home);
                live.dirty = true;
            }
            restored += 1;
        }
        self.world.town_npcs = saved;
        if restored > 0 {
            info!(residents = restored, "the town's residents are back");
        }
    }

    /// Who is waiting to move in, given what the world has been through and what people carry.
    ///
    /// Only the Guide ever arrived before this, so a town was one house and one resident forever.
    /// The cost of that was not cosmetic: the Mechanic sells the only wire in the game, and the
    /// entire wiring system was therefore unreachable.
    fn next_arrival(&mut self) -> Option<(u16, &'static str)> {
        use crate::game::arrivals::{Town, ready};

        let mut coins: i64 = 0;
        let mut best_life = 0i32;
        let (mut has_explosives, mut has_gun, mut has_dye_material) = (false, false, false);
        for player in self.players.iter().flatten().filter(|p| p.is_playing()) {
            best_life = best_life.max(i32::from(player.life_max));
            for slot in player.inventory.values() {
                let (kind, stack) = (slot.item.id, i64::from(slot.item.stack));
                coins += match kind {
                    71 => stack,
                    72 => stack * 100,
                    73 => stack * 10_000,
                    74 => stack * 1_000_000,
                    _ => 0,
                };
                // Bombs, dynamite and grenades; the guns the Arms Dealer answers to; and the dye
                // plants the Dye Trader wants. Small named sets rather than a table, because that
                // is what the game uses too.
                has_explosives |= matches!(kind, 166 | 167 | 168 | 235 | 1167 | 3006);
                has_gun |= matches!(kind, 24 | 39 | 43 | 96 | 98 | 99 | 120 | 434 | 1255);
                has_dye_material |= matches!(kind, 1105..=1111);
            }
        }

        let residents = self
            .npcs
            .iter()
            .filter(|(_, n)| n.stats.town_npc && n.is_alive())
            .count();
        let here: std::collections::HashSet<u16> = self
            .npcs
            .iter()
            .filter(|(_, n)| n.is_alive())
            .map(|(_, n)| n.npc_type)
            .collect();

        let town = Town {
            progress: &self.world.progress,
            coins,
            best_life,
            has_explosives,
            has_gun,
            has_dye_material,
            residents,
            hard_mode: self.world.progress.hard_mode,
        };
        ready(town, &|kind| here.contains(&kind))
            .into_iter()
            .next()
            .map(|arrival| (arrival.npc_type, arrival.name))
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
        self.broadcast(
            packets::rewrite_owner(id::T_E_DISPLAY_DOLL_DATA_SYNC, payload, slot)?,
            Some(slot),
        );
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
        // Most kinds cannot be asked for at all. The game's base `NetPlaceEntityAttempt` does
        // nothing, so a request naming an item frame or a mannequin is silently dropped — those
        // come into being when their *tile* goes down. Accepting all eleven, which this server
        // did, lets a crafted packet scatter tile entities through a world; a fuzzer duly found
        // three in a saved world that should have had none.
        if !kind.placeable_by_request() {
            debug!(slot, ?kind, "that kind is not placed by asking");
            return Ok(());
        }
        if self
            .world
            .tile_entities
            .iter()
            .any(|e| e.x == x && e.y == y)
        {
            return Ok(());
        }
        // The tile it claims to stand on has to actually be there.
        let tile = self.world.tile(i32::from(x), i32::from(y));
        if !tile.is_active() || tile.block != kind.tile() {
            debug!(slot, x, y, ?kind, "nothing there to place that on");
            return Ok(());
        }

        let id = self.world.next_tile_entity;
        self.world.next_tile_entity += 1;
        self.world
            .tile_entities
            .push(TileEntity::new(id, kind, x, y));
        debug!(slot, x, y, ?kind, id, "tile entity placed");
        // Everyone has to be told, the placer included: the client sends the placement but does
        // not create the entity itself, and the id it will refer to from now on is the server's
        // to hand out.
        self.share_tile_entity(id);
        Ok(())
    }

    /// Give a piece of furniture the tile entity that makes it work, and tell everybody.
    ///
    /// Called when the tile goes down rather than when a client asks, because for most kinds
    /// that is the only moment there is: the game's placement request does nothing for an item
    /// frame, a mannequin, a hat rack, a food platter, a logic sensor or a display jar.
    fn add_tile_entity(&mut self, kind: terrustia_proto::tile_entity::EntityKind, x: i16, y: i16) {
        if self
            .world
            .tile_entities
            .iter()
            .any(|e| e.x == x && e.y == y)
        {
            return;
        }
        let id = self.world.next_tile_entity;
        self.world.next_tile_entity += 1;
        self.world
            .tile_entities
            .push(terrustia_proto::tile_entity::TileEntity::new(
                id, kind, x, y,
            ));
        self.share_tile_entity(id);
        debug!(x, y, ?kind, id, "tile entity created with its tile");
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
        let is_pylon = entity.kind == terrustia_proto::tile_entity::EntityKind::TeleportationPylon;
        let where_it_is = (entity.x, entity.y);
        let Ok(frame) = terrustia_proto::tile_entity::share(entity) else {
            return;
        };
        self.broadcast(frame, None);

        // A pylon needs a second announcement: the tile-entity message puts it in the world, and
        // module 8 is what puts it on the travel map. Only the second one is what a player sees.
        if is_pylon
            && let Some(pylon) = self
                .pylons()
                .into_iter()
                .find(|p| (p.x, p.y) == where_it_is)
        {
            self.pylon_kinds.insert(where_it_is, pylon.kind);
            self.broadcast_pylon(net_module::PylonMessage::Added, pylon);
        }
    }

    /// Tell everyone a tile entity has gone.
    fn unshare_tile_entity(&mut self, id: i32) {
        // Read before the caller removes it, so a pylon can be taken off the travel map by the
        // same call that takes it out of the world.
        let pylon = self
            .world
            .tile_entities
            .iter()
            .find(|e| {
                e.id == id && e.kind == terrustia_proto::tile_entity::EntityKind::TeleportationPylon
            })
            .map(|e| (e.x, e.y));
        if let Ok(frame) = terrustia_proto::tile_entity::unshare(id) {
            self.broadcast(frame, None);
        }
        if let Some(at) = pylon {
            // The remembered network, not one read off a tile that is already gone.
            let kind = self.pylon_kinds.remove(&at).unwrap_or(0);
            self.broadcast_pylon(
                net_module::PylonMessage::Removed,
                net_module::Pylon {
                    x: at.0,
                    y: at.1,
                    kind,
                },
            );
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
            let tile = self.world.tile(i32::from(entity.x), i32::from(entity.y));
            if !tile.is_active() || tile.block != entity.kind.tile() {
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
            let planted = tile.is_active() && tile.block == entity.kind.tile();
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
        let party_monolith = fired.party_monolith;
        self.apply_circuit(fired, (x, y));

        self.broadcast(packets::verbatim(id::HIT_SWITCH, payload)?, Some(slot));

        // `BirthdayParty::ToggleManualParty` — a direct click or a wire signal reaching a Party
        // Monolith (`wiring.rs`'s own `PARTY_MONOLITH`). Real vanilla has no chat message for
        // this at all, unlike a natural party starting or any party ending at night — only the
        // world-data resync (`NetMessage.SendData(7)`) that lets clients react to it.
        if party_monolith {
            self.party.toggle_manual();
            self.broadcast_world_data();
        }
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
        // Vanilla's third spam counter, and the tightest of the three: 50 with 0.2 a tick back.
        // Liquid is the cheapest thing to spam and the most expensive to simulate.
        if let Some(player) = self.player_mut(slot) {
            player.spam_liquid += 1.0;
            if player.spam_liquid > SPAM_LIQUID_MAX {
                info!(slot, "disconnecting a client for liquid spam");
                self.kick(slot, "moving liquid too fast");
                return Ok(());
            }
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
        self.broadcast_army_wait(self.army.hold);

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

        // Net module 0, not tile squares. This is the message the client expects for water moving,
        // and it costs six bytes a tile against a square's per-tile flag chain plus a header —
        // which matters because a settling pool dirties a whole stripe of neighbours every tick,
        // and this used to be a flood of its own.
        let changes: Vec<net_module::LiquidChange> = touched
            .iter()
            .map(|&(x, y)| {
                let tile = self.world.tile(x, y);
                net_module::LiquidChange {
                    x,
                    y,
                    amount: tile.liquid,
                    kind: tile.liquid_kind.as_type_byte(),
                }
            })
            .collect();

        // Split rather than truncate: the count is a `u16` and the frame has a size limit, so a
        // large enough disturbance has to go out as several frames or the tail is simply lost.
        for batch in changes.chunks(net_module::MAX_LIQUID_CHANGES) {
            if let Ok(frame) = net_module::liquid_changes(batch) {
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
        self.weather.tick(
            strong_enough,
            hard_mode,
            self.journey.freeze_wind,
            self.journey.freeze_rain,
            &mut self.rng,
        );
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
            w.i32(crate::game::lunar::MOON_LORD_COUNTDOWN)
                .i32(countdown);
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
        self.announce_key("LegacyMisc.47", Vec::new());
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
            self.announce_key("LegacyMisc.20", Vec::new());
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
        self.roll_natural_party();
        // `LanternNight::CheckMorning` — a lantern night, genuine or manually forced, never
        // survives past one dawn. No chat announcement in real vanilla either, just the world-flag
        // resync `broadcast_world_data` below already sends.
        if self.lantern_night.end_for_the_morning() {
            self.broadcast_world_data();
        }
    }

    /// `BirthdayParty::NaturalAttempt`, called once at dawn (`Main.UpdateTime_StartDay` calling
    /// `BirthdayParty.CheckMorning`) — see `game/party.rs`'s own module doc for the mechanism.
    fn roll_natural_party(&mut self) {
        use crate::game::party::{PARTY_GIRL, PartyState};
        let party_girl_present = self.npcs.iter().any(|(_, n)| n.npc_type == PARTY_GIRL);
        let eligible: Vec<u8> = self
            .npcs
            .iter()
            .filter(|(_, n)| {
                terrustia_proto::npc_data::npc_stats(n.npc_type)
                    .is_some_and(|s| PartyState::can_party(n.npc_type, s.town_npc, s.ai_style))
            })
            .map(|(index, _)| index)
            .collect();
        let Some(chosen) = self
            .party
            .natural_attempt(party_girl_present, &eligible, &mut self.rng)
        else {
            return;
        };
        let names: Vec<String> = chosen
            .iter()
            .filter_map(|&i| self.npcs.get(i))
            .map(|n| n.given_name.clone())
            .collect();
        // `Game.BirthdayParty_1`/`_2`/`_3` (`Terraria.Localization.Content.en-US.json:825-827`).
        let text = match names.as_slice() {
            [a] => format!("Looks like {a} is throwing a party"),
            [a, b] => format!("Looks like {a} & {b} are throwing a party"),
            [a, b, c, ..] => format!("Looks like {a}, {b}, and {c} are throwing a party"),
            [] => return, // cannot happen: natural_attempt only returns Some with 1-3 names
        };
        self.announce(&text);
        info!(?chosen, "birthday party");
        self.broadcast_world_data();
    }

    /// `BirthdayParty::UpdateTime`'s own per-tick prune: an NPC that stops being eligible mid-day
    /// (killed, evicted, whatever) is dropped from the celebration, and a genuine party with
    /// nobody left to celebrate ends early.
    fn tick_party(&mut self) {
        let npcs = &self.npcs;
        let ended = self.party.prune(|index| {
            npcs.get(index).is_some_and(|n| {
                terrustia_proto::npc_data::npc_stats(n.npc_type).is_some_and(|s| {
                    crate::game::party::PartyState::can_party(n.npc_type, s.town_npc, s.ai_style)
                })
            })
        });
        if ended {
            self.announce("Party time's over!");
            self.broadcast_world_data();
        }
    }

    /// Slime Rain's own per-tick countdown, daily roll, and delayed start/stop announcement — see
    /// `crate::game::slime_rain`'s own module doc. Unlike the birthday party's once-per-dawn roll,
    /// `roll`'s own gate (`day_time && before_noon`) needs to catch the exact moment it becomes
    /// true, so this runs every tick rather than only at dawn — the same reason real vanilla's own
    /// `UpdateTime` checks it unconditionally too.
    fn tick_slime_rain(&mut self) {
        let rate = self.journey.time_rate();
        self.slime_rain.tick(rate, &mut self.rng);

        let other_events_busy = self.world.blood_moon
            || self.world.eclipse
            || self.moon.running()
            || self.invasion.is_some()
            || self.army.ongoing();
        // `AnyPlayerReadyToFightKingSlime`'s own `statDefense > 8` half is not modelled — this
        // server never tracks a player's own defense stat (only NPC/town-resident defense is
        // server-authoritative; a player's is a client-computed value this project never
        // receives), the same narrowing `start_invasion`'s own `life_max >= 200` qualifying check
        // already made for a different event's readiness gate.
        let someone_ready = self
            .players
            .iter()
            .flatten()
            .any(|p| p.is_playing() && p.life_max > 140);
        self.slime_rain.roll(
            self.slime_rain.is_active(),
            self.world.day_time,
            self.world.time < DAY_LENGTH / 2,
            rate,
            other_events_busy,
            self.world.progress.downed_king_slime,
            self.world.progress.hard_mode,
            someone_ready,
            self.is_expert(),
            &mut self.rng,
        );

        if let Some(now_active) = self.slime_rain.tick_warning() {
            self.announce_key(
                if now_active {
                    "LegacyWorldGen.74"
                } else {
                    "LegacyWorldGen.75"
                },
                Vec::new(),
            );
            self.broadcast_world_data();
        }
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
        // `BirthdayParty::CheckNight` — a party, genuine or manually forced, never survives past
        // one day. Called first, matching `Main.UpdateTime_StartNight`'s own order.
        if self.party.end_for_the_night() {
            self.announce("Party time's over!");
            self.broadcast_world_data();
        }
        self.roll_natural_lantern_night();
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
            self.announce_key("LegacyMisc.8", Vec::new());
            info!("blood moon");
            self.broadcast_world_data();
        }
    }

    /// `LanternNight::CheckNight`, called once at dusk — see `game/lantern_night.rs`'s own module
    /// doc for the mechanism. `can_start` is real vanilla's own `LanternsCanStart()`, computed
    /// here from every input this project already tracks: no blood moon, no moon event, no real
    /// invasion, no meteor already owed, and nothing boss-shaped currently up — including the
    /// Eater of Worlds' three segments specifically, `NPCID` 13/14/15, none of which carry the
    /// ordinary `.boss` stat (confirmed directly against `npc_data.rs`, the same reason real
    /// vanilla's own `BossIsActive()` needed the same special case).
    fn roll_natural_lantern_night(&mut self) {
        let boss_active = self.npcs.iter().any(|(_, n)| {
            matches!(n.npc_type, 13..=15)
                || terrustia_proto::npc_data::npc_stats(n.npc_type).is_some_and(|s| s.boss)
        });
        let can_start = !self.world.progress.spawn_meteor
            && !self.world.blood_moon
            && !self.world.pumpkin_moon
            && !self.world.snow_moon
            && self.invasion.is_none()
            && self.lunar.countdown == 0
            && !boss_active;
        let was_up = self.lantern_night.is_up();
        self.lantern_night.natural_attempt(
            can_start,
            self.world.progress.downed_moon_lord,
            &mut self.rng,
        );
        // Real vanilla's own `LanternNight::UpdateTime` only resyncs the world flag here
        // (`NetMessage.SendData(7)`) — no chat announcement at all, unlike blood moon/eclipse/the
        // birthday party, each of which really does broadcast a line. Checked directly against
        // source rather than assumed from the pattern those other events set.
        if self.lantern_night.is_up() != was_up {
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
        self.announce_key(
            match moon {
                Moon::Pumpkin => "LegacyMisc.31",
                Moon::Frost => "LegacyMisc.34",
            },
            Vec::new(),
        );
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
        // Computed before borrowing `self.moon` mutably below — `is_expert`/`is_master` borrow
        // all of `self`, which would otherwise overlap the `&mut self.moon` the call needs.
        let (expert, master) = (self.is_expert(), self.is_master());
        if let Some(wave) = self.moon.note_kill(npc_type, expert, master) {
            self.announce(&format!("Wave {wave}!"));
        }
    }

    /// Land whatever an enemy leaves behind on the player it just touched.
    fn apply_touch_debuffs(&mut self, slot: u8, npc_type: u16) {
        let expert = self.is_expert();
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

    /// Count one more of something towards its banner, and hand the banner over on the threshold.
    ///
    /// Nothing counted kills at all before this, so the reward never arrived and the world file's
    /// banner section was written as two zeroes. The count lives on the world, so it survives a
    /// restart rather than starting again at nought every session.
    fn note_banner_kill(&mut self, npc_type: u16, at: (f32, f32)) {
        use terrustia_proto::banners;

        let Some(banner) = banners::banner_of(npc_type) else {
            return;
        };
        let item = banners::banner_item(banner);
        let needed = banners::kills_needed(item);

        let count = self.world.banner_kills.entry(banner).or_insert(0);
        *count += 1;
        let reached = (*count).is_multiple_of(needed);
        let total = *count;

        // Tell every client the new count, so the bestiary's counter moves while they watch rather
        // than only on their next join.
        if let Ok(frame) = net_module::banner_kill_count(banner, total) {
            self.broadcast(frame, None);
        }

        if !reached {
            return;
        }

        let name = terrustia_proto::npc_data::npc_stats(npc_type).map_or("them", |s| s.name);
        self.announce(&format!("{total} {name} defeated!"));
        if let Some(index) = self.items.spawn(ItemStack::new(i32::from(item), 1, 0), at) {
            self.broadcast_item(index);
        }
    }

    /// Record a boss's death against the world's history, and — real vanilla's own
    /// `OnGameEventClearedForTheFirstTime`, `NPC.cs`'s own `SetEventFlagCleared` calls scattered
    /// through the very dispatcher `note_boss_kill_inner` below already is — guarantee the next
    /// lantern night if any of the flags it sets just flipped false→true for the first time.
    /// Wrapped around the whole existing dispatcher, snapshot-and-diff, rather than touching each
    /// of its match arms individually: every one of vanilla's own real trigger flags this project
    /// tracks at all is covered this way without duplicating that dispatcher's own boss roster by
    /// hand, and any flag `note_boss_kill_inner` gains later is covered automatically too.
    fn note_boss_kill(&mut self, npc_type: u16) {
        let before = self.world.progress;
        self.note_boss_kill_inner(npc_type);
        let p = &self.world.progress;
        let first_time = (!before.downed_boss1 && p.downed_boss1)
            || (!before.downed_boss2 && p.downed_boss2)
            || (!before.downed_boss3 && p.downed_boss3)
            || (!before.downed_king_slime && p.downed_king_slime)
            || (!before.downed_queen_bee && p.downed_queen_bee)
            || (!before.downed_deerclops && p.downed_deerclops)
            || (!before.hard_mode && p.hard_mode)
            || (!before.downed_mech1 && p.downed_mech1)
            || (!before.downed_mech2 && p.downed_mech2)
            || (!before.downed_mech3 && p.downed_mech3)
            || (!before.downed_plantera && p.downed_plantera)
            || (!before.downed_golem && p.downed_golem)
            || (!before.downed_fishron && p.downed_fishron)
            || (!before.downed_queen_slime && p.downed_queen_slime)
            || (!before.downed_empress_of_light && p.downed_empress_of_light)
            || (!before.downed_ancient_cultist && p.downed_ancient_cultist)
            || (!before.downed_moon_lord && p.downed_moon_lord);
        if first_time {
            self.lantern_night.next_night_guaranteed = true;
        }
    }

    /// The actual boss-kill dispatcher — see [`note_boss_kill`](Self::note_boss_kill), its own
    /// thin wrapper, for the lantern-night guarantee this function's own flag transitions feed.
    ///
    /// Nothing in the game reads a boss's death directly — everything reads the flag it sets. A
    /// shop that opens, a spawn pool that widens, an event that becomes possible: all of it hangs
    /// off this, which is why a server that kills bosses without recording them has a world that
    /// never progresses.
    fn note_boss_kill_inner(&mut self, npc_type: u16) {
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
                let who = NetworkText::key("NPCName.MoonLord", Vec::new());
                self.announce_key("Announcement.HasBeenDefeated_Single", vec![who]);
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
        self.announce_key("LegacyMisc.15", Vec::new());
        info!(
            converted,
            took_ms = began.elapsed().as_millis(),
            "hardmode began"
        );
        // Every client's view of the world is now wrong: drop the caches so they re-request.
        self.section_cache.clear();
        self.broadcast_world_data();
    }

    /// One tick of the world growing: grass creeping over bare dirt near whoever is playing.
    ///
    /// Terraria samples random tiles across the whole world every tick. This samples around the
    /// players instead, which costs a fraction as much and changes only the part of the world
    /// anyone can see. The sample count is small deliberately — this runs every tick, and grass
    /// that takes a minute to cross a field is indistinguishable from grass that takes ten
    /// seconds, while a hundred times the sampling is very distinguishable in the tick budget.
    fn tick_growth(&mut self) {
        /// Tiles tried per player per tick.
        const SAMPLES: usize = 3;
        /// How far from a player growth is considered, in tiles.
        const REACH: i32 = 90;

        if !self.ticks.is_multiple_of(GROWTH_EVERY) {
            return;
        }
        let around: Vec<(i32, i32)> = self
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
        if around.is_empty() {
            return;
        }

        let changed = {
            let world = &mut self.world;
            crate::world::growth::tick_growth(world, &around, SAMPLES, REACH, &mut self.rng)
        };
        // A grown tile is a tile change like any other; clients re-request the section.
        for (x, y) in changed {
            self.liquids.wake(x, y);
        }
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

    /// Whether the cultist tablet (and its four attendants) may appear at the dungeon entrance —
    /// the periodic, `tick_old_man`-shaped check the Moon Lord acceptance-test bot
    /// (`examples/moonlord.rs`, task #37) found was entirely missing: nothing anywhere placed npc
    /// 437 (`CULTIST_TABLET`), even though its own AI (`ai/boss/tablet.rs`) is real and complete
    /// once it exists — gather four attendants, wait for all four to die, shatter, raise the
    /// Cultist. Without this, the entire post-Golem game (the Lunatic Cultist, the Lunar
    /// Apocalypse, Moon Lord) was unreachable through ordinary play.
    ///
    /// ## What is confirmed vs. reasoned
    ///
    /// **Confirmed**, read directly this session from `terraria.wiki.gg`'s own "Lunatic Cultist"
    /// and "Cultists" pages (no decompiled source tree exists in this environment — see
    /// `secret_seed.rs`'s own module doc for the same standing disclosure this project already
    /// uses when it has to lean on public documentation instead of source): the tablet and its
    /// four Cultists (two Archers, two Devotees) appear at the dungeon's entrance once Golem has
    /// been defeated; they do **not** appear until Skeletron has also been defeated, because the
    /// Old Man takes spawn priority over that exact spot — the same mutual exclusion
    /// `tick_old_man` above already enforces the other way (it stops the moment `downed_boss3` is
    /// set). Killing all four attendants is what raises the Lunatic Cultist, destroying the
    /// tablet — already real and complete in `ai/boss/tablet.rs` and wired through to
    /// `ai/mod.rs`'s `ritual_complete` handling; this function's only job is to put the tablet
    /// itself on the ground so that machinery has something to run.
    ///
    /// **Reasoned, not independently sourced**: that the tablet stops reappearing once the
    /// Lunatic Cultist has actually been killed (`downed_ancient_cultist`). Nothing read this
    /// session states this explicitly, but it matches this file's own standing pattern of a
    /// one-time boss-history flag permanently retiring its own spawn path — the same shape
    /// `downed_boss3` already uses to retire the Old Man above — and the alternative, a tablet
    /// that keeps reappearing at a dungeon whose Cultist fight is already finished, has no
    /// support in anything read either.
    fn tick_cultist_tablet(&mut self) {
        use terrustia_proto::npc_params::{
            CULTIST, CULTIST_ARCHER, CULTIST_DEVOTE, CULTIST_TABLET,
        };

        if !self.ticks.is_multiple_of(OLD_MAN_CHECK_INTERVAL) {
            return;
        }
        let progress = self.world.progress;
        if !progress.downed_golem || !progress.downed_boss3 || progress.downed_ancient_cultist {
            return;
        }
        // The tablet, its four attendants, and the boss they raise all occupy the same spot the
        // Old Man used to — never more than one of this whole chain on the ground at a time.
        if self.npcs.iter().any(|(_, n)| {
            matches!(
                n.npc_type,
                CULTIST_TABLET | CULTIST_DEVOTE | CULTIST_ARCHER | CULTIST
            )
        }) {
            return;
        }
        let (Some(x), Some(y)) = (self.world.dungeon_x, self.world.dungeon_y) else {
            return;
        };
        // Only bother once somebody is near enough to see it appear — the same reasoning
        // `tick_old_man` above already uses for the same spot.
        let watched = self.players.iter().flatten().any(|p| {
            p.is_playing()
                && (p.position.0 / crate::game::npc::TILE - x as f32).abs() < OLD_MAN_NOTICE
        });
        if !watched {
            return;
        }
        let at = (x as f32 * 16.0, (y - 3) as f32 * 16.0);
        if let Some(index) = self.npcs.spawn(CULTIST_TABLET, at) {
            self.broadcast_npc(index);
            info!(
                x,
                y, "the cultist tablet has appeared at the dungeon entrance"
            );
        }
    }

    /// Turn the Old Man into Skeletron.
    ///
    /// He is not killed and Skeletron is not summoned beside him — he *becomes* it, which is why
    /// the dungeon has no guardian afterwards. The Clothier will do instead, because he is the
    /// same man once the curse is off him.
    fn summon_skeletron(&mut self) {
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

    /// Set the clock to an exact point and tell everyone — the `/time` admin command's own effect,
    /// pulled out so Journey mode's four time-skip buttons (`StartDayImmediately`/
    /// `StartNoonImmediately`/`StartNightImmediately`/`StartMidnightImmediately`) can share it
    /// rather than re-decide what a client needs to hear about a jumped clock.
    fn set_time(&mut self, day_time: bool, time: i32) -> terrustia_proto::Result<()> {
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
        Ok(())
    }

    /// `Main.Difficulty`'s own real shape: real vanilla never reads `Main.GameMode` for anything
    /// difficulty-scaled — every such site reads this one float instead, with `expertMode`/
    /// `masterMode` themselves just `Difficulty >= 2`/`>= 3` (`Main.cs`). It is ordinarily
    /// `world.game_mode`-derived, but in a Journey world (`IsJourneyMode`) the `DifficultySlider`
    /// power overrides it to its own continuous value (`Main.cs`'s
    /// `UpdateCreativeGameModeOverride`) — every call site that used to read `world.game_mode`
    /// directly for combat/drop/event scaling should go through this instead, so the slider
    /// actually reaches it. `journey_world` gates (spawning/Godmode/FarPlacementRange/SpawnRate,
    /// which ask "is this a Journey world" rather than "how hard is it") are a different question
    /// and correctly still read `world.game_mode` directly.
    fn effective_difficulty(&self) -> f32 {
        if self.world.game_mode == 3 {
            self.journey.difficulty_multiplier()
        } else {
            terrustia_proto::difficulty::of_game_mode(self.world.game_mode)
        }
    }

    /// `Main.expertMode`'s own definition: `Difficulty >= GameDifficultyLevel.Expert` (`2.0`).
    fn is_expert(&self) -> bool {
        self.effective_difficulty() >= 2.0
    }

    /// `Main.masterMode`'s own definition: `Difficulty >= GameDifficultyLevel.Master` (`3.0`).
    fn is_master(&self) -> bool {
        self.effective_difficulty() >= 3.0
    }

    /// The per-tick AI context every NPC's own behaviour reads from — pulled out of `tick_npcs`
    /// into its own method so it can be tested directly (a Journey world's `DifficultySlider`
    /// reaching `expert` here, for instance) rather than only indirectly through a full AI tick.
    fn ai_conditions(&self, biome: crate::game::spawn::Biome) -> crate::game::ai::Conditions {
        crate::game::ai::Conditions {
            blood_moon: self.world.blood_moon,
            day: self.world.day_time,
            eclipse: self.world.eclipse,
            raining: self.world.raining,
            windy: self.weather.windy(),
            crimson: self.world.crimson,
            snow: biome == crate::game::spawn::Biome::Snow,
            jungle: biome == crate::game::spawn::Biome::Jungle,
            wind: self.weather.wind,
            desert: biome == crate::game::spawn::Biome::Desert,
            sandstorm: self.weather.sandstorm,
            surface_y: f32::from(self.world.surface) * crate::game::npc::TILE,
            // `Main.expertMode` itself — `Difficulty >= Expert`, not a raw game-mode check, so a
            // Journey world's `DifficultySlider` reaches AI branches that ask this too.
            expert: self.is_expert(),
            hardmode: self.world.progress.hard_mode,
            world_size: (self.world.width(), self.world.height()),
        }
    }

    /// Module 4: Journey mode powers. See [`crate::game::journey`]'s own module doc for exactly
    /// which of vanilla's fifteen this covers.
    ///
    /// `slot` is only used by the three per-player powers, and only ever to *override* whatever
    /// player index the wire carried — `APerPlayerTogglePower`/`APerPlayerSliderPower`'s own
    /// `DeserializeNetMessage` does the identical substitution on a dedicated server
    /// (`Main.netMode == 2`), which is why the proto layer discards that byte entirely rather than
    /// handing it up: a client cannot toggle Godmode for somebody else by lying about which slot
    /// it is. No permission-level check yet for any power (`PowerPermissionLevel` — real vanilla
    /// lets an operator configure who may flip each power, `journeypermission_<name>` in its own
    /// config) — every connected player may use every power this server models, disclosed rather
    /// than silently assumed.
    fn on_creative_power(
        &mut self,
        slot: u8,
        message: net_module::CreativePowerMessage,
    ) -> terrustia_proto::Result<()> {
        use net_module::{CreativePowerMessage, power};

        match message {
            // The four buttons share `set_time` with `/time` — same effect, same values
            // (`DAY_LENGTH/2`/`NIGHT_LENGTH/2` for noon/midnight match `SkipToTime`'s own
            // `27000`/`16200` exactly; see that pair's own doc comment on the constants).
            CreativePowerMessage::Button(id) => {
                let set = match id {
                    power::START_DAY => Some((true, 0)),
                    power::START_NOON => Some((true, DAY_LENGTH / 2)),
                    power::START_NIGHT => Some((false, 0)),
                    power::START_MIDNIGHT => Some((false, NIGHT_LENGTH / 2)),
                    _ => None,
                };
                if let Some((day_time, time)) = set {
                    self.set_time(day_time, time)?;
                }
            }
            CreativePowerMessage::Toggle(id, enabled) => {
                if self.journey.set(id, enabled)
                    && let Ok(frame) = net_module::creative_power_toggle(id, enabled)
                {
                    // A dedicated server broadcasts the accepted state to everyone, the toggling
                    // player included — that player's own client does not apply its request
                    // locally, it waits to be told, the same request/confirm shape `RequestUse`
                    // uses in source.
                    self.broadcast(frame, None);
                }
            }
            // Four real, different effects behind the same wire shape:
            // - `ModifyTimeRate`/`Difficulty`: stored (`journey.time_rate_slider`/
            //   `difficulty_slider`), read every tick (`tick()`'s own `journey.time_rate()` call)
            //   or on demand (`effective_difficulty()`, called wherever this server used to read
            //   `world.game_mode` directly for anything difficulty-scaled).
            // - `ModifyWind`/`ModifyRain`: applied straight to `self.weather` and forgotten —
            //   neither is `_syncToJoiningPlayers` nor `IPersistentPerWorldContent` in source (see
            //   `journey.rs`'s own module doc), so there is nothing to hold onto here at all.
            CreativePowerMessage::Slider(id, value) => {
                let applied = match id {
                    power::MODIFY_TIME_RATE => {
                        self.journey.time_rate_slider = value;
                        true
                    }
                    power::DIFFICULTY => {
                        self.journey.difficulty_slider = value;
                        true
                    }
                    power::MODIFY_WIND => {
                        // `MathHelper.Lerp(-0.8f, 0.8f, value)`, set to both the current wind and
                        // its target at once — `ModifyWindDirectionAndStrength::
                        // UpdateInfoFromSliderValueCache`'s own two assignments.
                        let wind = -0.8 + value.clamp(0.0, 1.0) * 1.6;
                        self.weather.wind = wind;
                        self.weather.target = wind;
                        self.world.wind = wind;
                        true
                    }
                    power::MODIFY_RAIN => {
                        // `Main.StartRain(instant: true, value)`/`Main.StopRain(instant: true)`.
                        // Real vanilla rain set this way has no timer at all; this project's own
                        // rain model is timer-driven (`Weather::tick_rain`'s own countdown), so a
                        // long sentinel approximates "does not expire on its own" rather than
                        // removing the timer concept entirely — disclosed, not a silent gap.
                        if value <= 0.0 {
                            self.weather.stop_rain();
                        } else {
                            self.weather.raining = true;
                            self.weather.max_rain = value.clamp(0.0, 1.0);
                            self.weather.rain_time = i32::MAX;
                        }
                        self.world.raining = self.weather.raining;
                        self.world.rain_time = self.weather.rain_time;
                        self.world.max_rain = self.weather.max_rain;
                        true
                    }
                    _ => false,
                };
                if applied && let Ok(frame) = net_module::creative_power_slider(id, value) {
                    self.broadcast(frame, None);
                }
            }
            // `Godmode`/`FarPlacementRange`. `slot` — the real sender, never the wire's own
            // (discarded) player-index byte — is both what gets toggled and, once accepted, what
            // the confirmation names: exactly `SetEnabledState`'s own
            // `NetManager.Instance.Broadcast` of the same `SyncOnePlayer` shape to everyone,
            // toggling player included (its own client waits to be told, same as the shared
            // toggles above).
            CreativePowerMessage::PerPlayerToggle(id, enabled) => {
                let applied = match id {
                    power::GODMODE => {
                        self.journey.set_godmode(slot, enabled);
                        true
                    }
                    power::FAR_PLACEMENT_RANGE => {
                        self.journey.set_far_placement_range(slot, enabled);
                        true
                    }
                    _ => false,
                };
                if applied
                    && let Ok(frame) =
                        net_module::creative_power_toggle_for_player(id, slot, enabled)
                {
                    self.broadcast(frame, None);
                }
            }
            // `SpawnRate`. Stored for `slot` only — real vanilla's own `DeserializeNetMessage` has
            // no broadcast branch here at all (unlike the toggle shape above): another player's
            // personal spawn-rate preference is never anyone else's business, nothing to relay.
            CreativePowerMessage::PerPlayerSlider(id, value) => {
                if id == power::SPAWN_RATE {
                    self.journey.set_spawn_rate_slider(slot, value);
                }
            }
        }
        Ok(())
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

    /// Real vanilla's Wall of Flesh trigger: a Guide Voodoo Doll destroyed by lava in the
    /// Underworld while the Guide is alive. Called every tick from `tick_items` — see that
    /// function's own call site.
    ///
    /// The Moon Lord acceptance-test bot (`examples/moonlord.rs`, task #37) found this was
    /// entirely missing: npc 113 (Wall of Flesh) is absent from `npc_params::SUMMONABLE` on
    /// purpose (see below), and nothing else in this file ever spawned it either — grepping for
    /// its id, `voodoo`, `Voodoo` found only the *death*-side hardmode-transition flag already in
    /// `note_boss_kill_inner`. Without a trigger, hardmode — and everything after it — was
    /// unreachable through ordinary play.
    ///
    /// ## What is confirmed vs. what this project narrows
    ///
    /// **Confirmed**, read directly this session from `terraria.wiki.gg`'s own "Wall of Flesh" and
    /// "Guide Voodoo Doll" pages (no decompiled source tree exists in this environment — see
    /// `secret_seed.rs`'s own module doc for the same standing disclosure this project already
    /// uses when it has to lean on public documentation instead of source): the doll must be
    /// destroyed by lava while it is in the Underworld; the Guide must be alive beforehand and
    /// dies as a direct result of the doll burning (not the other way around — the doll does not
    /// need the Guide to be nearby, only alive somewhere); at least one player must be in the
    /// Underworld; the boss then spawns off whichever edge of the map is nearer to where the doll
    /// burned and walks inward. This is also *why* npc 113 is deliberately absent from
    /// `SUMMONABLE`: real vanilla never lets the Wall of Flesh be quick-summoned through the
    /// ordinary boss-item packet the way an Eye of Cthulhu or King Slime can be — adding it there
    /// would be a real behavioural addition vanilla does not have, not a fix. The Guide Voodoo
    /// Doll's own internal item id — 267 — is confirmed the same way (the wiki's own infobox
    /// states it directly; cross-checked against a second, independent page,
    /// `terrariachecklist.com/item/267`, whose own URL slug agrees).
    ///
    /// **Narrowed, and disclosed rather than silently approximated**, because this project's own
    /// item-entity physics (`world/items.rs`) has real position and real gravity, but no liquid
    /// awareness at all — `items::fall`'s own `blocked` test only asks whether a tile is solid, so
    /// an item dropped over lava today falls straight through it with no buoyancy or immersion of
    /// any kind. Building a full float-on-liquid simulation just for this one item would be a
    /// large undertaking disproportionate to this fix (this project's own standing preference —
    /// see `plan.md`'s Tier 2 "narrow, purpose-built implementation" notes — is a narrower,
    /// disclosed trigger over either fabricating physics vanilla's own item class has but this
    /// codebase doesn't, or leaving the progression blocker unfixed):
    /// - "Touches lava" is read the tile at the item's own position, sampled every tick — the same
    ///   shape `tick_shimmer` already uses to ask "is this item in shimmer" for its own liquid,
    ///   the closest existing precedent in this codebase, rather than a dedicated buoyancy
    ///   simulation. An item falling *through* a lava tile on its way to the floor beneath it
    ///   (this generator's own items have no buoyancy to stop them floating on the surface) still
    ///   satisfies this on the tick it passes through, which is a real, if narrower, sense of
    ///   "touches lava" than vanilla's own floating-on-the-surface one.
    /// - "In the Underworld" reuses the exact `height() - 200` boundary this file's own
    ///   `on_server_teleport` and `world/bulbs.rs`'s own `UNDERWORLD` constant already use for the
    ///   same question — not a new threshold invented for this fix.
    /// - "At least one player is in the Underworld" is not tracked as an independent, separate
    ///   check on player position — a player had to physically carry the doll there to drop it in
    ///   the first place, so requiring the *doll itself* to be in the Underworld stands in for it.
    ///   Disclosed as an inferred substitute for vanilla's own distinct check, not a re-derivation
    ///   of it: a doll that somehow ended up in underworld lava with no player nearby (for
    ///   instance, swept there by an unrelated mechanic) would trigger this where real vanilla
    ///   would not.
    /// - Vanilla's exact off-screen spawn distance is not reproduced pixel-for-pixel; this spawns
    ///   the boss from the nearer world edge instead (matching the wiki's own "closer to the left
    ///   edge, comes from the left" rule for *which side*) and lets `ai/boss/wall.rs`'s own AI —
    ///   "its opening direction is toward whoever woke it" — pick which way it walks from there,
    ///   since that already does not depend on the exact vanilla spawn offset to behave correctly.
    fn tick_wall_of_flesh_trigger(&mut self) {
        const GUIDE_VOODOO_DOLL: i32 = 267;

        if self.world.progress.hard_mode || self.items.is_empty() {
            return;
        }

        let mut burned: Vec<(i16, (f32, f32))> = Vec::new();
        {
            let world = &self.world;
            let underworld_from = world.height() - 200;
            for (index, item) in self.items.iter() {
                if item.item.id != GUIDE_VOODOO_DOLL {
                    continue;
                }
                let x =
                    ((item.position.0 + crate::world::items::ITEM_SIZE / 2.0) / TILE_SIZE) as i32;
                let y =
                    ((item.position.1 + crate::world::items::ITEM_SIZE / 2.0) / TILE_SIZE) as i32;
                if y < underworld_from {
                    continue;
                }
                let tile = world.tile(x, y);
                if tile.liquid > 0 && tile.liquid_kind == terrustia_proto::Liquid::Lava {
                    burned.push((index, item.position));
                }
            }
        }

        for (index, at) in burned {
            if self.summon_wall_of_flesh(at) {
                self.items.remove(index);
                if let Ok(frame) = terrustia_proto::items::item_despawn(index) {
                    self.broadcast(frame, None);
                }
            }
        }
    }

    /// The Guide dies with the doll, and the Wall rises to replace him — see
    /// `tick_wall_of_flesh_trigger`'s own doc comment for the full disclosure of what is real
    /// vanilla behaviour here versus this project's own narrowing.
    ///
    /// Returns whether it actually happened, so the caller knows whether to consume the doll that
    /// triggered it: real vanilla always destroys a Voodoo Doll that burns in lava, but this
    /// project has no general "items burn in lava" mechanic to fall back on if the trigger did
    /// not actually fire (hardmode already begun, no Guide alive) — so a doll that cannot do
    /// anything is left alone rather than silently vanishing for no visible reason.
    fn summon_wall_of_flesh(&mut self, at: (f32, f32)) -> bool {
        const WALL_OF_FLESH: u16 = 113;

        if self.world.progress.hard_mode {
            return false;
        }
        if self.npcs.iter().any(|(_, n)| n.npc_type == WALL_OF_FLESH) {
            return false;
        }
        let guide = self
            .npcs
            .iter()
            .find(|(_, n)| n.npc_type == GUIDE && n.is_alive())
            .map(|(index, _)| index);
        let Some(guide_index) = guide else {
            debug!("a voodoo doll burned in the underworld, but no guide is alive to die with it");
            return false;
        };

        self.npcs.remove(guide_index);
        self.broadcast_npc_death(guide_index);

        let width = self.world.width() as f32 * TILE_SIZE;
        let spawn_x = if at.0 < width / 2.0 { 0.0 } else { width };
        let spawn_at = (spawn_x, at.1);
        if let Some(index) = self.npcs.spawn(WALL_OF_FLESH, spawn_at) {
            let name = self
                .npcs
                .get(index)
                .map(|n| n.stats.name)
                .unwrap_or("Something");
            let who = NetworkText::key(format!("NPCName.{name}"), Vec::new());
            self.announce_key("Announcement.HasAwoken", vec![who]);
            self.broadcast_npc(index);
            info!(x = spawn_at.0, y = spawn_at.1, "the Wall of Flesh rises");
        }
        true
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

        // The world owns the tiers, so a loaded world that already chose palladium keeps it
        // instead of being re-rolled by the next altar broken here.
        let mut tiers = hardmode::OreTiers::load(&self.world.ore_tiers);
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
        tiers.store(&mut self.world.ore_tiers);
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

        self.announce_key(smashed.announcement, Vec::new());
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
            self.wipe_army_field();
        }
    }

    /// Clear the field when the Old One's Army ends.
    ///
    /// The event leaves behind its own furniture — the lane portals, whatever was still coming
    /// through them, and the players' towers. None of it belongs to the world afterwards, and a
    /// server that leaves it standing leaves a permanent goblin camp where the arena was.
    ///
    /// The packet tells clients to do the same on their side, since they draw the towers.
    fn wipe_army_field(&mut self) {
        let leftovers: Vec<u8> = self
            .npcs
            .iter()
            .filter(|(_, n)| crate::game::army::belongs(n.npc_type))
            .map(|(index, _)| index)
            .collect();
        for index in leftovers {
            self.npcs.remove(index);
            self.broadcast_npc_death(index);
        }
        if let Ok(frame) = packets::empty(id::CRYSTAL_INVASION_WIPE_ALL_THE_THINGSSS) {
            self.broadcast(frame, None);
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
        let expert = self.is_expert();
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
                // The wave ended, so the gap begins: clients need the countdown or the pause
                // reads as the event having stopped.
                let left = self.army.hold;
                self.broadcast_army_wait(left);
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
            cavern_monsters: self.cavern_monsters,
        };
        let spawned = spawn::try_spawn(
            &self.world,
            &self.npcs,
            &self.players,
            &events,
            &self.journey,
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

/// Does a panic on the untrusted-packet path actually get caught, and does the world still reach
/// disk on the way out?
///
/// `catch_unwind` used to wrap only `tick()`. `handle_event` — which is where every byte from
/// every client is decoded — was bare, so a panic under it unwound out of the loop, past the
/// shutdown save at the bottom of `run`, and the process still exited zero.
#[cfg(test)]
mod panic_path {
    use super::*;
    use crate::config::Config;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "panic probe")
    }

    #[tokio::test]
    async fn a_panic_handling_an_event_is_caught_and_reported() {
        let (tx, rx) = mpsc::channel::<ServerEvent>(4);
        let server = GameServer::new(Config::default(), tiny_world());

        tx.send(ServerEvent::Console {
            line: "__panic_probe".into(),
        })
        .await
        .expect("the game task should still be listening");

        // The panic is deliberate; let it not spew a backtrace over the test output.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let outcome = server.run(rx).await;
        std::panic::set_hook(hook);

        assert_eq!(
            outcome,
            Stopped::Panicked,
            "a panic on the packet path must be caught and reported, not unwind the task"
        );
    }

    /// The ordinary case still reports a clean stop, so the exit code stays meaningful.
    #[tokio::test]
    async fn a_normal_shutdown_reports_cleanly() {
        let (tx, rx) = mpsc::channel::<ServerEvent>(4);
        let server = GameServer::new(Config::default(), tiny_world());
        drop(tx);
        assert_eq!(server.run(rx).await, Stopped::Cleanly);
    }
}

/// The console's `panel` command sends exactly one pulse on `panel_toggle` — the other end
/// (`crate::panel::supervise`) decides what that pulse means and actually owns the bind/abort.
/// This only proves the command reaches the channel and never panics without one wired; the real
/// start/stop behaviour is covered end-to-end, over a real socket, in `tests/panel.rs`.
#[cfg(test)]
mod panel_toggle_command {
    use super::*;
    use crate::config::Config;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "panel toggle probe")
    }

    #[tokio::test]
    async fn the_panel_command_sends_one_pulse_when_a_toggle_channel_is_wired() {
        let (tx, rx) = mpsc::channel::<ServerEvent>(4);
        let (toggle_tx, mut toggle_rx) = mpsc::unbounded_channel();
        let server = GameServer::new(Config::default(), tiny_world()).with_panel_toggle(toggle_tx);
        let handle = tokio::spawn(server.run(rx));

        tx.send(ServerEvent::Console {
            line: "panel".into(),
        })
        .await
        .unwrap();
        drop(tx);
        assert_eq!(handle.await.unwrap(), Stopped::Cleanly);

        assert!(
            toggle_rx.try_recv().is_ok(),
            "the console command should have sent exactly one pulse"
        );
        assert!(
            toggle_rx.try_recv().is_err(),
            "and only one — not, say, one per tick"
        );
    }

    /// Every test that constructs a `GameServer` directly (all seventeen call sites, before this
    /// one) never calls `with_panel_toggle` — the command has to stay harmless there, not panic.
    #[tokio::test]
    async fn the_panel_command_does_not_panic_with_no_toggle_channel_wired() {
        let (tx, rx) = mpsc::channel::<ServerEvent>(4);
        let server = GameServer::new(Config::default(), tiny_world());
        let handle = tokio::spawn(server.run(rx));

        tx.send(ServerEvent::Console {
            line: "panel".into(),
        })
        .await
        .unwrap();
        drop(tx);
        assert_eq!(handle.await.unwrap(), Stopped::Cleanly);
    }
}

/// Journey mode's `FreezeTime` actually stops the clock — not just the toggle sticking, the real
/// gameplay effect (`tick()`'s own gate on `self.journey.freeze_time`, mirroring `Main.cs:6342`'s
/// gate on the same power in source).
#[cfg(test)]
mod freeze_time {
    use super::*;
    use crate::config::Config;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "freeze time probe")
    }

    #[test]
    fn frozen_time_does_not_advance_across_many_ticks() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.journey.freeze_time = true;
        let (day_time, time) = (server.world.day_time, server.world.time);

        for _ in 0..500 {
            server.tick();
        }

        assert_eq!(
            (server.world.day_time, server.world.time),
            (day_time, time),
            "the clock should not have moved a single tick while frozen"
        );
    }

    #[test]
    fn unfreezing_lets_it_advance_again() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.journey.freeze_time = true;
        let before = server.world.time;
        for _ in 0..10 {
            server.tick();
        }
        assert_eq!(server.world.time, before, "still frozen so far");

        server.journey.freeze_time = false;
        for _ in 0..10 {
            server.tick();
        }
        assert!(
            server.world.time > before,
            "the clock should have moved once unfrozen, got {} from a start of {before}",
            server.world.time
        );
    }
}

/// Journey mode's `ModifyTimeRate` actually changes how fast the clock runs — `tick()`'s own
/// `self.journey.time_rate()` argument to `tick_time`, mirroring `Main.cs:6343`'s own
/// `targetTimeRate` read in source.
#[cfg(test)]
mod modify_time_rate {
    use super::*;
    use crate::config::Config;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "time rate probe")
    }

    #[test]
    fn the_top_of_the_slider_advances_the_clock_twenty_four_times_as_fast() {
        let mut baseline = GameServer::new(Config::default(), tiny_world());
        let mut sped_up = GameServer::new(Config::default(), tiny_world());
        sped_up.journey.time_rate_slider = 1.0; // the slider's real top: 24x

        // Deltas, not absolute values: `new()`'s own startup work (angler quest roll and friends)
        // can leave `world.time` non-zero before the first real tick, which a bare before/after-one-
        // tick comparison would otherwise fold into the "24x" ratio and make it come out wrong.
        let (before_baseline, before_sped) = (baseline.world.time, sped_up.world.time);
        baseline.tick();
        sped_up.tick();
        let (moved_baseline, moved_sped) = (
            baseline.world.time - before_baseline,
            sped_up.world.time - before_sped,
        );

        assert_eq!(
            moved_baseline, 1,
            "an ordinary tick should move the clock by exactly one"
        );
        assert_eq!(
            moved_sped, 24,
            "one tick at the slider's top should move the clock 24 real ticks' worth"
        );
    }
}

/// Journey mode's `Godmode` actually blocks the one damage path this server decides on a
/// player's behalf — `hurt_player`'s own gate, mirroring the effect (not the client-side
/// mechanism) of `creativeGodMode` in source. Unconditional on the world's own difficulty,
/// deliberately unlike `FarPlacementRange`/`SpawnRate` below: `Player.cs`'s own
/// `creativeGodMode = true;` assignment has no `difficulty == 3` guard around it at all.
#[cfg(test)]
mod godmode {
    use super::*;
    use crate::config::Config;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "godmode probe")
    }

    /// The receiver has to stay alive for as long as the caller keeps using `server` — `broadcast`
    /// (`hurt_player`'s own `PlayerHurt`/`PlayerDeath`) removes a player whose send fails, and a
    /// dropped receiver closes the channel immediately regardless of its buffer size, not merely
    /// once that buffer fills. Returned rather than silently kept alive inside this function,
    /// which would only postpone the drop to *this* function's own return, not the caller's use.
    fn with_one_player(mut server: GameServer) -> (GameServer, mpsc::Receiver<Bytes>) {
        let (out_tx, out_rx) = mpsc::channel(16);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = ConnState::Playing;
        player.life = 100;
        player.life_max = 100;
        server.players[0] = Some(player);
        (server, out_rx)
    }

    #[test]
    fn godmode_takes_no_damage() {
        let (mut server, _rx) = with_one_player(GameServer::new(Config::default(), tiny_world()));
        server.journey.set_godmode(0, true);

        server.hurt_player(
            0,
            9999,
            1,
            terrustia_proto::hurt::DeathReason::from_npc(0),
            0,
        );

        assert_eq!(
            server.players[0].as_ref().unwrap().life,
            100,
            "life should be untouched while godmode is on"
        );
    }

    #[test]
    fn an_ordinary_player_takes_damage_normally() {
        let (mut server, _rx) = with_one_player(GameServer::new(Config::default(), tiny_world()));
        // godmode left off — the control case, so the test above is proving something rather
        // than passing regardless of whether the gate exists at all.

        server.hurt_player(0, 30, 1, terrustia_proto::hurt::DeathReason::from_npc(0), 0);

        assert_eq!(server.players[0].as_ref().unwrap().life, 70);
    }

    #[test]
    fn turning_godmode_off_again_lets_damage_through() {
        let (mut server, _rx) = with_one_player(GameServer::new(Config::default(), tiny_world()));
        server.journey.set_godmode(0, true);
        server.hurt_player(
            0,
            9999,
            1,
            terrustia_proto::hurt::DeathReason::from_npc(0),
            0,
        );
        assert_eq!(server.players[0].as_ref().unwrap().life, 100, "still on");

        server.journey.set_godmode(0, false);
        server.hurt_player(0, 30, 1, terrustia_proto::hurt::DeathReason::from_npc(0), 0);
        assert_eq!(server.players[0].as_ref().unwrap().life, 70);
    }

    #[test]
    fn godmode_for_one_player_does_not_protect_another() {
        let (mut server, _rx) = with_one_player(GameServer::new(Config::default(), tiny_world()));
        let (out_tx, _other_rx) = mpsc::channel(16);
        let mut other = Player::new(1, "127.0.0.1:2".parse().unwrap(), out_tx);
        other.state = ConnState::Playing;
        other.life = 100;
        other.life_max = 100;
        server.players[1] = Some(other);

        server.journey.set_godmode(0, true);
        server.hurt_player(1, 30, 1, terrustia_proto::hurt::DeathReason::from_npc(0), 0);

        assert_eq!(
            server.players[1].as_ref().unwrap().life,
            70,
            "slot 1 was never given godmode"
        );
    }
}

/// Journey mode's `FarPlacementRange` — a misleading name inherited from source; both of its two
/// real vanilla uses (`Player.cs:35212`/`35440`) are about item *pickup* range, not tile placement
/// at all (see `tick_items`'s own comment) — extends how far an item can be reserved for a player,
/// by exactly 240 pixels, and only in a world whose own difficulty is literally Journey
/// (`world.game_mode == 3`), matching source's own `difficulty == 3` guard on both sites.
#[cfg(test)]
mod far_placement_range {
    use super::*;
    use crate::config::Config;
    use terrustia_proto::ItemStack;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "far placement probe")
    }

    /// Same shape as `godmode`'s own `with_one_player` — the receiver has to outlive the tick
    /// call, for the same reason (`broadcast` removes a player whose send fails).
    fn with_one_player_at(
        mut server: GameServer,
        position: (f32, f32),
    ) -> (GameServer, mpsc::Receiver<Bytes>) {
        let (out_tx, out_rx) = mpsc::channel(16);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = ConnState::Playing;
        player.position = position;
        server.players[0] = Some(player);
        (server, out_rx)
    }

    fn a_coin() -> ItemStack {
        ItemStack {
            id: 71, // Copper Coin
            prefix: 0,
            stack: 1,
        }
    }

    #[test]
    fn extends_pickup_range_by_exactly_two_hundred_forty_pixels_in_a_journey_world() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.world.game_mode = 3; // Journey
        let (mut server, _rx) = with_one_player_at(server, (0.0, 0.0));
        server.journey.set_far_placement_range(0, true);
        // Within the boosted range (400 + 240 = 640) but outside the ordinary one — the only
        // distance this test is actually about.
        let index = server.items.spawn(a_coin(), (500.0, 0.0)).unwrap();

        server.tick_items();

        assert!(
            server.items.get(index).unwrap().is_reserved(),
            "should have been reserved for the player once the range was extended"
        );
    }

    #[test]
    fn does_not_extend_range_without_the_power_enabled() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.world.game_mode = 3;
        let (mut server, _rx) = with_one_player_at(server, (0.0, 0.0));
        // far_placement_range left off — the control case, so the test above is proving
        // something rather than passing regardless of whether the extension exists at all.
        let index = server.items.spawn(a_coin(), (500.0, 0.0)).unwrap();

        server.tick_items();

        assert!(
            !server.items.get(index).unwrap().is_reserved(),
            "at ordinary range this item should never have been reserved at all"
        );
    }

    /// The power has no effect at all outside a Journey world — `Player.cs`'s own two real uses
    /// both gate on `difficulty == 3` before ever reading it, so an implementation that skipped
    /// that gate would extend pickup range on every world, not just Journey ones.
    #[test]
    fn has_no_effect_outside_a_journey_world() {
        let server = GameServer::new(Config::default(), tiny_world()); // game_mode 0: ordinary
        let (mut server, _rx) = with_one_player_at(server, (0.0, 0.0));
        server.journey.set_far_placement_range(0, true);
        let index = server.items.spawn(a_coin(), (500.0, 0.0)).unwrap();

        server.tick_items();

        assert!(
            !server.items.get(index).unwrap().is_reserved(),
            "an ordinary-difficulty world should use the plain range regardless of the power"
        );
    }

    #[test]
    fn an_item_beyond_even_the_extended_range_is_still_out_of_reach() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.world.game_mode = 3;
        let (mut server, _rx) = with_one_player_at(server, (0.0, 0.0));
        server.journey.set_far_placement_range(0, true);
        let index = server.items.spawn(a_coin(), (700.0, 0.0)).unwrap(); // past 400 + 240

        server.tick_items();

        assert!(!server.items.get(index).unwrap().is_reserved());
    }
}

/// Journey mode's `Difficulty` — real vanilla's `Main.Difficulty` is the single float every
/// difficulty-scaled system in source actually reads (`effective_difficulty`'s own doc), so this
/// module pins that a Journey world's slider actually reaches every one of the call sites that
/// used to read `world.game_mode` directly, one test per site — not just that the accessor itself
/// computes the right number.
///
/// A genuine side-finding along the way, not something introduced by this change: five of those
/// call sites (`ai_conditions`'s `expert`, `drop_loot`'s `expert`/`master`, `apply_touch_debuffs`,
/// `note_army_kill`, `note_moon_kill`) read `world.game_mode >= 1`/`>= 2` directly — and `3 >= 1`
/// and `3 >= 2` are both true, so a Journey world (`game_mode == 3`) was *already* silently read as
/// full expert-and-master for every one of these before this module existed at all, regardless of
/// the gentler `0.5` difficulty `of_game_mode` correctly gave it for NPC life/damage. Real vanilla
/// never has this inconsistency, because `Main.Difficulty` is the one thing everything reads —
/// `expertMode`/`masterMode` are just `Difficulty >= 2`/`>= 3` on it, and a Journey world's
/// `Difficulty` (0.5 by default, whether or not the slider override is even active — `GameMode ==
/// 3` matches neither of `Main.Difficulty`'s own `GameMode == 1`/`== 2` fallback branches) is below
/// both thresholds. Routing every site through `effective_difficulty`/`is_expert`/`is_master`
/// fixes this as a side effect of giving the slider anywhere to reach at all.
#[cfg(test)]
mod difficulty_slider {
    use super::*;
    use crate::config::Config;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "difficulty slider probe")
    }

    fn journey_at(slider: f32) -> GameServer {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.world.game_mode = 3;
        server.journey.difficulty_slider = slider;
        server
    }

    fn with_one_player(mut server: GameServer) -> (GameServer, mpsc::Receiver<Bytes>) {
        let (out_tx, out_rx) = mpsc::channel(16);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = ConnState::Playing;
        server.players[0] = Some(player);
        (server, out_rx)
    }

    #[test]
    fn outside_a_journey_world_the_slider_is_ignored() {
        for game_mode in [0u8, 1, 2] {
            let mut server = GameServer::new(Config::default(), tiny_world());
            server.world.game_mode = game_mode;
            server.journey.difficulty_slider = 1.0; // set, but should never be read
            assert_eq!(
                server.effective_difficulty(),
                terrustia_proto::difficulty::of_game_mode(game_mode),
                "game_mode {game_mode} must ignore the slider entirely"
            );
        }
    }

    #[test]
    fn an_untouched_journey_world_keeps_journeys_own_old_fixed_difficulty() {
        let server = journey_at(0.0);
        assert_eq!(server.effective_difficulty(), 0.5);
        assert!(!server.is_expert(), "a fresh Journey world is not expert");
        assert!(!server.is_master(), "a fresh Journey world is not master");
    }

    #[test]
    fn moving_the_slider_to_its_top_makes_a_journey_world_read_as_master() {
        let server = journey_at(1.0);
        assert_eq!(server.effective_difficulty(), 3.0);
        assert!(server.is_expert());
        assert!(server.is_master());
    }

    /// The main chokepoint (`tick()`'s own `let difficulty = self.effective_difficulty();`):
    /// NPC life scaling. A zombie's `life_max` should reflect the slider's own continuous value,
    /// not the fixed `0.5` a Journey world always used to be stuck with. `life_multiplier` is a
    /// single linear segment from `(0.5, 0.5)` to `(4.0, 4.0)` — i.e. the identity function on
    /// this range — so 0.5x to 3.0x should be a clean 6x, modulo the `as i32` truncation each
    /// scaling step already applies (real vanilla's own `NPC.ScaleStats` truncates too), which is
    /// why this checks a wide, unambiguous margin rather than an exact ratio.
    #[test]
    fn the_npc_scaling_chokepoint_reflects_a_moved_slider() {
        const ZOMBIE: u16 = 3;
        let mut gentle = journey_at(0.0);
        gentle.tick();
        let index = gentle.npcs.spawn(ZOMBIE, (0.0, 0.0)).expect("a slot");
        let gentle_life = gentle.npcs.get(index).unwrap().life_max;

        let mut fierce = journey_at(1.0);
        fierce.tick();
        let index = fierce.npcs.spawn(ZOMBIE, (0.0, 0.0)).expect("a slot");
        let fierce_life = fierce.npcs.get(index).unwrap().life_max;

        assert!(
            fierce_life >= gentle_life * 5,
            "0.5x to 3.0x should be roughly a 6x jump: gentle={gentle_life}, fierce={fierce_life}"
        );
    }

    /// Dryad's Bane borrows the same difficulty curve as town NPC damage
    /// (`dryad_bane_rate`/`buffs::town_npc_damage_multiplier`) — a separate call site from the NPC
    /// scaling chokepoint above, reached through `self.effective_difficulty()` directly rather
    /// than through `self.npcs.set_scaling`. Comparing classic-equivalent (slider 0.33) against
    /// master (slider 1.0) rather than journey's own default (slider 0.0) against master: the
    /// curve's own real shape (`the_difficulty_curve_hits_its_keys` in `buffs.rs`) peaks at
    /// journey (2.0x) and dips before master (1.75x) — a real, pre-existing, already-pinned curve
    /// shape, not something this change introduced, but the wrong pair to assert "goes up" on.
    #[test]
    fn dryad_banes_rate_reflects_a_moved_slider() {
        let classic_equivalent = journey_at(0.33).dryad_bane_rate();
        let master = journey_at(1.0).dryad_bane_rate();
        assert_eq!(
            classic_equivalent, 4,
            "difficulty 1.0: base 4 * multiplier 1.0"
        );
        assert_eq!(
            master, 7,
            "difficulty 3.0: base 4 * multiplier 1.75, truncated"
        );
        assert!(master > classic_equivalent);
    }

    /// `ai_conditions`'s own `expert` field, which town NPC combat (and every other AI branch that
    /// asks) reads instead of a raw game-mode check.
    #[test]
    fn ai_conditions_expert_reflects_a_moved_slider() {
        let biome = crate::game::spawn::Biome::Forest;
        assert!(!journey_at(0.0).ai_conditions(biome).expert);
        assert!(journey_at(1.0).ai_conditions(biome).expert);
    }

    /// `drop_loot`'s `Conditions.expert` — a King Slime's treasure bag (item 3318) is an
    /// unconditional expert-only drop (`conditional_drops::conditional`'s own `always(bag)`), so
    /// its presence or absence is a clean, RNG-free signal.
    #[test]
    fn drop_loots_expert_condition_reflects_a_moved_slider() {
        const KING_SLIME: u16 = 50;
        const TREASURE_BAG: i32 = 3318;

        let mut gentle = journey_at(0.0);
        gentle.drop_loot(KING_SLIME, (0.0, 0.0), false);
        assert!(
            !gentle
                .items
                .iter()
                .any(|(_, it)| it.item.id == TREASURE_BAG),
            "a fresh Journey world is not expert, so no bag"
        );

        let mut fierce = journey_at(1.0);
        fierce.drop_loot(KING_SLIME, (0.0, 0.0), false);
        assert!(
            fierce
                .items
                .iter()
                .any(|(_, it)| it.item.id == TREASURE_BAG),
            "the slider at its top is expert, so the bag should drop"
        );
    }

    /// `apply_touch_debuffs`'s own `expert` gate — npc 222 (Queen Bee) always (`one_in: 1`) lands
    /// an expert-only Poisoned on touch (`touch_debuffs::POISONED_IN_EXPERT`).
    #[test]
    fn apply_touch_debuffs_expert_gate_reflects_a_moved_slider() {
        const QUEEN_BEE: u16 = 222;

        let (mut gentle, mut gentle_rx) = with_one_player(journey_at(0.0));
        gentle.apply_touch_debuffs(0, QUEEN_BEE);
        assert!(
            gentle_rx.try_recv().is_err(),
            "a fresh Journey world is not expert, so no buff should be sent"
        );

        let (mut fierce, mut fierce_rx) = with_one_player(journey_at(1.0));
        fierce.apply_touch_debuffs(0, QUEEN_BEE);
        assert!(
            fierce_rx.try_recv().is_ok(),
            "the slider at its top is expert, so the buff should be sent"
        );
    }

    /// `note_army_kill`'s own `expert` local — a plain Old One's Army goblin (any id in
    /// `army::belongs`'s range) is worth double the kill points once expert.
    #[test]
    fn note_army_kill_expert_doubling_reflects_a_moved_slider() {
        const A_PLAIN_ARMY_ENEMY: u16 = 552;

        let mut gentle = journey_at(0.0);
        gentle.army.start(crate::game::army::Tier::One, (0, 0));
        gentle.note_army_kill(A_PLAIN_ARMY_ENEMY);
        assert_eq!(gentle.army.kills, 1, "not expert, so one plain kill");

        let mut fierce = journey_at(1.0);
        fierce.army.start(crate::game::army::Tier::One, (0, 0));
        fierce.note_army_kill(A_PLAIN_ARMY_ENEMY);
        assert_eq!(fierce.army.kills, 2, "expert doubles a plain kill");
    }

    /// `note_moon_kill`'s own `is_expert()`/`is_master()` — the top of the slider is master, worth
    /// 2.5x a kill rather than the classic 1x a fresh Journey world reads as.
    #[test]
    fn note_moon_kill_scaling_reflects_a_moved_slider() {
        const A_PUMPKIN_MOON_SCARECROW: u16 = 305; // worth 1 point, from moons.rs's own table

        let mut gentle = journey_at(0.0);
        gentle.moon.start(crate::game::moons::Moon::Pumpkin);
        gentle.note_moon_kill(A_PUMPKIN_MOON_SCARECROW);
        assert_eq!(gentle.moon.points, 1.0, "classic scale is 1x");

        let mut fierce = journey_at(1.0);
        fierce.moon.start(crate::game::moons::Moon::Pumpkin);
        fierce.note_moon_kill(A_PUMPKIN_MOON_SCARECROW);
        assert_eq!(fierce.moon.points, 2.5, "master scale is 2.5x");
    }
}

/// The birthday party — see `game/party.rs`'s own module doc for the real vanilla mechanism this
/// wires up: `roll_dawn_events`'s own natural roll, `roll_dusk_events`'s own end-of-day clear,
/// `tick_party`'s own mid-day prune, and `on_hit_switch`'s own reaction to a Party Monolith.
#[cfg(test)]
mod party {
    use super::*;
    use crate::config::Config;
    use rand::SeedableRng;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "party probe")
    }

    /// Real town NPC types (`npc_data.rs`'s own table) — five ordinary residents plus Party Girl,
    /// which is exactly enough for a natural party to have somewhere to start.
    const A_TOWN: [u16; 5] = [17, 18, 19, 20, 22];

    fn a_town_and_party_girl(server: &mut GameServer) {
        for npc_type in A_TOWN {
            server.npcs.spawn(npc_type, (0.0, 0.0)).expect("a slot");
        }
        server
            .npcs
            .spawn(crate::game::party::PARTY_GIRL, (0.0, 0.0))
            .expect("a slot");
    }

    /// `roll_dawn_events`'s own `roll_natural_party` call, run against a real `NpcStore` rather
    /// than a hand-built eligible list — proves the real `npc_data` lookup and the exclusion
    /// list actually connect to a live server, not just `PartyState`'s own already-tested logic.
    #[test]
    fn a_natural_party_eventually_starts_with_a_real_town_present() {
        for seed in 0..500u64 {
            let mut server = GameServer::new(Config::default(), tiny_world());
            server.rng = SmallRng::seed_from_u64(seed);
            a_town_and_party_girl(&mut server);
            server.roll_natural_party();
            if server.party.genuine {
                assert!(!server.party.celebrating.is_empty());
                assert!(server.party.celebrating.len() <= 3);
                return;
            }
        }
        panic!("a party should have started at least once across 500 seeds");
    }

    /// Without Party Girl having moved in, no amount of trying starts a natural party — real
    /// vanilla's own `NPC.AnyNPCs(208)` gate.
    #[test]
    fn no_party_girl_means_no_natural_party_at_the_server_level() {
        for seed in 0..500u64 {
            let mut server = GameServer::new(Config::default(), tiny_world());
            server.rng = SmallRng::seed_from_u64(seed);
            for npc_type in A_TOWN {
                server.npcs.spawn(npc_type, (0.0, 0.0)).expect("a slot");
            }
            server.roll_natural_party();
            assert!(!server.party.genuine);
        }
    }

    #[test]
    fn a_party_ends_at_dusk() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.party.manual = true;
        server.roll_dusk_events();
        assert!(!server.party.is_up(), "manual parties end at night too");
    }

    /// A celebrating NPC that stops being eligible (evicted, its slot reused by something else)
    /// is pruned on the next tick, and the party ends once none are left.
    #[test]
    fn a_party_ends_early_once_its_last_celebrant_is_gone() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        let index = server
            .npcs
            .spawn(crate::game::party::PARTY_GIRL, (0.0, 0.0))
            .expect("a slot");
        server.party.genuine = true;
        server.party.celebrating = vec![index];

        server.npcs.remove(index);
        server.tick_party();

        assert!(!server.party.genuine, "nobody left to celebrate");
        assert!(server.party.celebrating.is_empty());
    }

    /// A direct click on a Party Monolith toggles the world's manually-forced party and resyncs
    /// world data — `on_hit_switch`'s own reaction to `Fired::party_monolith`.
    #[test]
    fn clicking_a_party_monolith_toggles_the_manual_party() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        let (out_tx, _out_rx) = mpsc::channel(16);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = ConnState::Playing;
        server.players[0] = Some(player);

        server
            .world
            .set_tile(50, 50, terrustia_proto::Tile::framed(455, 0, 0));

        let mut payload = Vec::new();
        payload.extend_from_slice(&50i16.to_le_bytes());
        payload.extend_from_slice(&50i16.to_le_bytes());

        assert!(!server.party.manual);
        server.on_hit_switch(0, &payload).unwrap();
        assert!(server.party.manual, "the click should have toggled it on");

        server.on_hit_switch(0, &payload).unwrap();
        assert!(!server.party.manual, "and off again");
    }
}

#[cfg(test)]
mod slime_rain {
    use super::*;
    use crate::config::Config;
    use rand::SeedableRng;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "slime rain probe")
    }

    /// `world_data`'s own `WorldFlag::SlimeRain` patch — real state, not left unwired the way
    /// `PartyIsUp` sat before the birthday party landed.
    #[test]
    fn world_data_reflects_whether_a_rain_is_active() {
        // `WorldFlag::SlimeRain` is byte 2, bit 2 (`packets.rs`'s own `position()`, private to
        // that module) — read directly rather than via a setter-only API that has no matching
        // getter.
        let has_flag = |server: &GameServer| server.world_data().flags.0[2] & (1 << 2) != 0;
        let mut server = GameServer::new(Config::default(), tiny_world());
        assert!(!has_flag(&server));
        server.slime_rain.timer = 100;
        assert!(has_flag(&server));
    }

    /// `tick_slime_rain`'s own daily-roll wiring, driven through a real server rather than
    /// `SlimeRainState::roll` in isolation — proves `effective_difficulty`/`journey.time_rate`/
    /// the world's own day-time fields actually connect, not just the state machine's own
    /// already-tested logic. Expert mode alone is enough to let it fire (no ready player needed),
    /// matching `slime_rain.rs`'s own `expert_mode_alone_can_still_start_a_rain` test — and
    /// Journey's fastest clock (`time_rate_slider = 1.0`, 24x) keeps the odds (`9375`, per that
    /// same test's own comment) small enough for a real server loop to observe within a test.
    #[test]
    fn the_daily_roll_eventually_starts_a_rain_with_a_real_server() {
        for seed in 0..30u64 {
            let mut server = GameServer::new(Config::default(), tiny_world());
            server.rng = SmallRng::seed_from_u64(seed);
            server.world.game_mode = 2; // expert
            server.journey.time_rate_slider = 1.0;
            server.world.day_time = true;
            server.world.time = 0;
            for _ in 0..50_000 {
                server.tick_slime_rain();
                if server.slime_rain.is_active() {
                    return;
                }
            }
        }
        panic!("a rain should have started at least once across 30 seeds");
    }

    /// A hundred and fifty Blue Slime kills during a rain summons King Slime at the *closest*
    /// player to the last kill — `DoDeathEvents_AdvanceSlimeRain`'s own real choice, not a random
    /// one, and not just "some player" the way a first draft might assume.
    #[test]
    fn one_hundred_and_fifty_blue_slime_kills_summons_king_slime_near_the_closest_player() {
        use crate::game::slime_rain::{BLUE_SLIME, KING_SLIME};
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.slime_rain.timer = 100;

        let (near_tx, _near_rx) = mpsc::channel(16);
        let mut near = Player::new(0, "127.0.0.1:1".parse().unwrap(), near_tx);
        near.state = ConnState::Playing;
        near.position = (10.0, 10.0);
        server.players[0] = Some(near);

        let (far_tx, _far_rx) = mpsc::channel(16);
        let mut far = Player::new(1, "127.0.0.1:2".parse().unwrap(), far_tx);
        far.state = ConnState::Playing;
        far.position = (10_000.0, 10_000.0);
        server.players[1] = Some(far);

        for _ in 0..149 {
            server.note_slime_rain_kill(BLUE_SLIME, (10.0, 10.0));
        }
        assert!(
            !server.npcs.iter().any(|(_, n)| n.npc_type == KING_SLIME),
            "not yet — only 149 kills"
        );

        server.note_slime_rain_kill(BLUE_SLIME, (10.0, 10.0));

        assert!(
            server.npcs.iter().any(|(_, n)| n.npc_type == KING_SLIME),
            "the 150th kill should have summoned him"
        );
    }

    /// A kill while no rain is active does nothing at all — `note_kill`'s own `!is_active()`
    /// guard, proven connected through the real death path rather than assumed from the isolated
    /// state-machine test.
    #[test]
    fn a_blue_slime_kill_with_no_rain_active_summons_nothing() {
        use crate::game::slime_rain::{BLUE_SLIME, KING_SLIME};
        let mut server = GameServer::new(Config::default(), tiny_world());
        let (out_tx, _out_rx) = mpsc::channel(16);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = ConnState::Playing;
        server.players[0] = Some(player);

        for _ in 0..200 {
            server.note_slime_rain_kill(BLUE_SLIME, (10.0, 10.0));
        }
        assert!(!server.npcs.iter().any(|(_, n)| n.npc_type == KING_SLIME));
    }
}

#[cfg(test)]
mod lantern_night {
    use super::*;
    use crate::config::Config;
    use rand::SeedableRng;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "lantern night probe")
    }

    /// `world_data`'s own `WorldFlag::LanternNight` patch — real state, the same wiring
    /// `WorldFlag::SlimeRain`/`PartyIsUp` already got for their own events.
    #[test]
    fn world_data_reflects_whether_a_lantern_night_is_up() {
        // `WorldFlag::LanternNight` is byte 5, bit 1 (`packets.rs`'s own `position()`, private to
        // that module) — read directly, the same way the slime-rain flag test above does.
        let has_flag = |server: &GameServer| server.world_data().flags.0[5] & (1 << 1) != 0;
        let mut server = GameServer::new(Config::default(), tiny_world());
        assert!(!has_flag(&server));
        server.lantern_night.genuine = true;
        assert!(has_flag(&server));
    }

    /// `roll_natural_lantern_night`'s own daily-roll wiring, driven through a real server —
    /// proves `world.progress.downed_moon_lord`/the busy-gate computation actually connect, not
    /// just `LanternNightState::natural_attempt`'s own already-tested logic in isolation.
    #[test]
    fn a_natural_lantern_night_eventually_starts_with_moon_lord_downed() {
        for seed in 0..2000u64 {
            let mut server = GameServer::new(Config::default(), tiny_world());
            server.rng = SmallRng::seed_from_u64(seed);
            server.world.progress.downed_moon_lord = true;
            server.roll_natural_lantern_night();
            if server.lantern_night.genuine {
                return;
            }
        }
        panic!("a lantern night should have started at least once across 2000 seeds");
    }

    /// Without Moon Lord ever downed, no amount of trying starts a natural lantern night — real
    /// vanilla's own one real gate on the roll firing at all.
    #[test]
    fn no_lantern_night_without_moon_lord_downed() {
        for seed in 0..500u64 {
            let mut server = GameServer::new(Config::default(), tiny_world());
            server.rng = SmallRng::seed_from_u64(seed);
            server.roll_natural_lantern_night();
            assert!(!server.lantern_night.genuine);
        }
    }

    /// `note_boss_kill`'s own snapshot-and-diff guarantee wiring: killing King Slime for the
    /// first time (a `downed_king_slime` false→true transition) arms
    /// `lantern_night.next_night_guaranteed`, and the very next `roll_natural_lantern_night` call
    /// fires a lantern night outright — deterministic, since the guarantee bypasses the daily
    /// roll's own odds entirely, unlike the statistical test above.
    #[test]
    fn killing_a_boss_for_the_first_time_guarantees_the_next_lantern_night() {
        const KING_SLIME: u16 = crate::game::slime_rain::KING_SLIME;
        let mut server = GameServer::new(Config::default(), tiny_world());
        assert!(!server.lantern_night.next_night_guaranteed);

        server.note_boss_kill(KING_SLIME);
        assert!(server.world.progress.downed_king_slime);
        assert!(
            server.lantern_night.next_night_guaranteed,
            "a first-time boss kill should have armed the guarantee"
        );

        server.roll_natural_lantern_night();
        assert!(
            server.lantern_night.genuine,
            "the armed guarantee should have fired the very next roll"
        );
    }

    /// Killing the *same* boss again (already downed before this kill) does not re-arm the
    /// guarantee — `note_boss_kill`'s own diff is against the flag's own transition, not the kill
    /// event itself.
    #[test]
    fn killing_an_already_downed_boss_again_does_not_rearm_the_guarantee() {
        const KING_SLIME: u16 = crate::game::slime_rain::KING_SLIME;
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.world.progress.downed_king_slime = true;

        server.note_boss_kill(KING_SLIME);
        assert!(
            !server.lantern_night.next_night_guaranteed,
            "already downed before this kill — no false→true transition to guarantee off of"
        );
    }

    /// `roll_dawn_events`'s own `LanternNight::CheckMorning` hook — a lantern night never
    /// survives past one dawn, genuine or manually forced alike, matching the birthday party's
    /// own analogous dawn-end rule.
    #[test]
    fn a_lantern_night_ends_at_dawn() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.lantern_night.manual = true;
        server.roll_dawn_events();
        assert!(
            !server.lantern_night.is_up(),
            "manual nights end at dawn too"
        );
    }
}

/// Can a connected client stall the world by asking to register?
///
/// It could. `/register` needs no permission — it falls through the `needed` match to `None` — and
/// it used to run Argon2 inline on the game task. Argon2 is deliberately expensive: tens of
/// milliseconds against a 16.67 ms tick, so a client sending `/register` in a loop froze the world
/// for everybody. The hashing now happens on a worker thread, with a per-slot lock and a
/// server-wide ceiling so the queue cannot be grown either.
#[cfg(test)]
mod auth_cost {
    use super::*;
    use crate::config::Config;
    use std::time::{Duration, Instant};

    /// What one hash actually costs, measured rather than assumed, so the margin below is real.
    #[test]
    fn a_single_hash_is_far_more_than_a_tick() {
        let started = Instant::now();
        crate::admin::Account::new("probe", "a long enough password", "default").expect("hashing");
        let cost = started.elapsed();
        assert!(
            cost > Duration::from_millis(1),
            "argon2 finished in {cost:?}; if it is really this cheap the ceiling below is \
             meaningless and this whole guard needs rethinking"
        );
    }

    #[tokio::test]
    async fn a_burst_of_registrations_does_not_cost_the_tick() {
        let mut server = GameServer::new(Config::default(), World::empty(200, 150, "auth"));

        // Far more than the ceiling, from one slot and then from many.
        let started = Instant::now();
        for i in 0..64u8 {
            let _ = server.run_admin_command(
                i % 8,
                "register",
                &format!("account{i} a_long_enough_password"),
            );
        }
        let elapsed = started.elapsed();

        // Inline, sixty-four hashes would be well over a second. Deferred, this is bookkeeping.
        assert!(
            elapsed < Duration::from_millis(100),
            "sixty-four registrations took {elapsed:?} on the game task; they are being hashed \
             inline again, which is a denial of service any connected client can trigger"
        );
    }

    #[tokio::test]
    async fn one_slot_gets_one_hash_at_a_time_and_the_server_has_a_ceiling() {
        let mut server = GameServer::new(Config::default(), World::empty(200, 150, "auth"));

        assert!(server.start_auth(3), "the first is allowed");
        assert!(!server.start_auth(3), "a second from the same slot is not");

        // Fill the rest of the server-wide ceiling from other slots.
        for slot in 0..(MAX_AUTH_IN_FLIGHT as u8 - 1) {
            assert!(server.start_auth(slot), "slot {slot} within the ceiling");
        }
        assert!(
            !server.start_auth(200),
            "past the ceiling, nobody else gets one however many slots are asking"
        );
    }

    /// Leaving mid-check must not hold a hashing slot open.
    #[tokio::test]
    async fn disconnecting_releases_the_hashing_slot() {
        let mut server = GameServer::new(Config::default(), World::empty(200, 150, "auth"));
        assert!(server.start_auth(5));
        server.remove_player(5);
        assert!(
            server.start_auth(5),
            "a slot freed by a disconnect must be usable again, or repeated joins exhaust the pool"
        );
    }
}

/// Do the tick's phases and its total actually describe the same thing?
///
/// They did not. `worst_us` came from `clock::Cpu` and `phase_us` from `Instant`, so the warning
/// line compared CPU microseconds against wall microseconds. A real two-hour run logged three
/// ticks where the phase cost *more than the whole tick containing it* — which is impossible, and
/// meant every phase figure was inflated by however long that phase spent descheduled. All of
/// Stage 2's measurement rests on these numbers, so the invariant is pinned here.
#[cfg(test)]
mod tick_accounting {
    use super::*;
    use crate::config::Config;

    #[tokio::test]
    async fn no_phase_can_cost_more_than_its_own_tick() {
        let mut server = GameServer::new(Config::default(), World::empty(600, 400, "accounting"));

        for _ in 0..20 {
            let cost = server.tick();
            let (name, worst) = cost.worst_phase();
            assert!(
                worst <= cost.cpu,
                "phase {name} cost {worst:?} of a tick that cost {:?} — the two are being \
                 measured on different clocks again",
                cost.cpu
            );

            // And the parts must add up to the whole, not merely each be smaller than it.
            let summed: Duration = cost.phases.iter().sum();
            assert!(
                summed <= cost.cpu,
                "the phases sum to {summed:?} but the tick cost {:?}",
                cost.cpu
            );
        }
    }

    /// Wall clock is still recorded separately, because telling "we are slow" from "the machine
    /// is busy" is the reason this instrumentation exists at all.
    #[tokio::test]
    async fn wall_clock_is_still_measured_apart_from_processor_time() {
        let mut server = GameServer::new(Config::default(), World::empty(300, 200, "accounting"));
        let cost = server.tick();
        assert!(
            cost.wall >= cost.cpu,
            "a tick cannot use more processor than it took: cpu {:?}, wall {:?}",
            cost.cpu,
            cost.wall
        );
    }

    /// Every phase has a name, so a breakdown can never print an index.
    #[test]
    fn every_phase_is_named() {
        assert_eq!(Phase::NAMES.len(), Phase::Sync as usize + 1);
    }

    /// The property the fix actually turns on: time spent off the processor is not phase time.
    ///
    /// This is the test that catches the bug, and the reason the two above do not. On an idle
    /// machine wall clock and CPU clock agree, so "no phase exceeds its tick" passes happily
    /// against the broken code — verified by reverting the fix and watching it stay green.
    /// Sleeping forces the two clocks apart on purpose, which is the only reliable way to tell
    /// them apart without a loaded machine.
    #[test]
    fn a_phase_does_not_charge_for_time_spent_descheduled() {
        let mut clock = PhaseClock::start();
        std::thread::sleep(Duration::from_millis(40));
        let charged = clock.lap();
        assert!(
            charged < Duration::from_millis(5),
            "a phase that slept for 40ms was charged {charged:?}; phases are on the wall clock \
             again, which inflates every figure the breakdown prints"
        );
    }

    /// And it does still charge for work, so the clock is not simply stuck at zero.
    #[test]
    fn a_phase_does_charge_for_work() {
        let mut clock = PhaseClock::start();
        let mut total = 0u64;
        for i in 0..4_000_000u64 {
            total = total.wrapping_add(i * i);
        }
        std::hint::black_box(total);
        assert!(
            clock.lap() > Duration::ZERO,
            "four million multiplies cost nothing?"
        );
    }
}

/// The first autosave used to cost a whole extra world-copy inside a counted tick, because
/// `spare_world` started life empty and had nothing to diff the incremental path against.
///
/// Caught by a real CI soak run, not a unit test: `save_world_in_background`'s incremental path
/// (`refresh_snapshot`) requires a buffer that already holds the world's state as of the moment
/// change-tracking began, and there was no such buffer until the first save built one the
/// expensive way. Measured on that run — 14,833 µs, 89% of a single tick's budget — against every
/// later save's 150–200 µs once a buffer existed to refresh instead of rebuild.
#[cfg(test)]
mod snapshot_baseline {
    use super::*;
    use crate::config::Config;

    /// The property the fix turns on: a buffer exists before any save is ever requested, so the
    /// first one has something to diff against instead of paying for a full copy on the clock.
    #[test]
    fn a_spare_snapshot_buffer_exists_before_the_first_save_is_ever_requested() {
        let server = GameServer::new(Config::default(), World::empty(300, 200, "presnapshot"));
        assert!(
            server.spare_world.is_some(),
            "the first autosave has nothing to refresh against, and pays for a full world copy \
             inside a counted tick instead of the ~150µs an incremental refresh costs"
        );
    }

    /// And it is a genuine, independent copy — not the live world under a second name — or the
    /// "incremental" path would be diffing a buffer against itself.
    #[test]
    fn the_spare_buffer_is_a_real_copy_not_an_alias_of_the_live_world() {
        use crate::world::worldgen::tiles;

        let mut server = GameServer::new(Config::default(), World::empty(300, 200, "alias probe"));
        server
            .world
            .set_tile(10, 10, Tile::framed(tiles::CHEST, 0, 0));
        let spare = server
            .spare_world
            .as_ref()
            .expect("pre-warmed at construction");
        assert!(
            !spare.tile(10, 10).is_active(),
            "editing the live world after construction must not be visible through the spare \
             buffer, or it is an alias rather than a snapshot"
        );
    }
}

/// Does the server announce the game's own lines the way the game does?
///
/// Vanilla sends a localization key and its arguments — `NPC.cs:81383` is
/// `NetworkText.FromKey("Announcement.HasAwoken", ...)` against `"HasAwoken": "{0} has awoken!"` —
/// and the client renders it in whatever language that player is using. Sending the English
/// sentence instead was wrong three ways at once: different bytes on the wire than the real
/// server, English for every non-English player, and the game's own text compiled into this
/// repository. So the thing worth pinning is the *mode*, not the words.
#[cfg(test)]
mod announcements {
    use terrustia_proto::NetworkText;
    use terrustia_proto::net_text::TextMode;

    /// Keys used for the game's own lines, each verified against the decompiled localization
    /// files rather than guessed. A wrong key renders as nothing on the client, which is worse
    /// than English — so anything unverified deliberately stays a literal.
    #[test]
    fn the_keys_we_send_are_the_ones_the_game_defines() {
        for key in [
            // Terraria.Localization.Content.en-US.Legacy.json, "LegacyMisc"
            "LegacyMisc.8",  // The Blood Moon is rising...
            "LegacyMisc.9",  // You feel an evil presence watching you...
            "LegacyMisc.15", // The ancient spirits of light and dark have been released.
            "LegacyMisc.19", // {0} was slain...
            "LegacyMisc.20", // A solar eclipse is happening!
            "LegacyMisc.31", // The Pumpkin Moon is rising...
            "LegacyMisc.34", // The Frost Moon is rising...
            "LegacyMisc.47", // The Moon Lord has awoken!
            // The six hardmode ores
            "LegacyMisc.12",
            "LegacyMisc.13",
            "LegacyMisc.14",
            "LegacyMisc.21",
            "LegacyMisc.22",
            "LegacyMisc.23",
            // "LegacyMultiplayer"
            "LegacyMultiplayer.19", // {0} has joined.
            "LegacyMultiplayer.20", // {0} has left.
            // Terraria.Localization.Content.en-US.Game.json, "Announcement"
            "Announcement.HasAwoken",
            "Announcement.HasBeenDefeated_Single",
        ] {
            let text = NetworkText::key(key, Vec::new());
            assert_eq!(text.mode, TextMode::LocalizationKey);
            assert!(
                key.contains('.') && !key.ends_with('.'),
                "{key} is not a section-qualified key"
            );
        }
    }

    /// The ore announcements carry keys, not sentences.
    ///
    /// `hardmode.rs` holds them in a `&'static str` that used to be English, so this is the field
    /// most likely to quietly revert.
    #[test]
    fn the_ore_announcements_are_keys() {
        use crate::world::hardmode::{OreTiers, WorldShape, smash};
        use rand::{SeedableRng, rngs::SmallRng};

        let mut tiers = OreTiers::default();
        let mut rng = SmallRng::seed_from_u64(7);
        let shape = WorldShape {
            width: 4200,
            height: 1200,
            surface: 400,
            rock_layer: 600,
        };

        let mut seen = 0;
        for altars in 1..=3 {
            if let Some(smashed) = smash(altars, true, &mut tiers, shape, &mut rng) {
                let key = smashed.announcement;
                assert!(
                    key.starts_with("LegacyMisc."),
                    "the ore announcement is {key:?}, a sentence rather than a key"
                );
                seen += 1;
            }
        }
        assert!(seen > 0, "no altar smash produced an announcement to check");
    }

    /// A key with an argument nests, which is how "{0} has awoken!" gets its boss.
    #[test]
    fn an_announcement_can_carry_a_name() {
        let who = NetworkText::key("NPCName.MoonLord", Vec::new());
        let line = NetworkText::key("Announcement.HasAwoken", vec![who]);
        assert_eq!(line.substitutions.len(), 1);
        assert_eq!(line.substitutions[0].mode, TextMode::LocalizationKey);
        assert_eq!(line.substitutions[0].text, "NPCName.MoonLord");
    }
}

/// Does a client hammering tile edits get stopped, the way vanilla stops one?
///
/// Vanilla has this and we did not, which makes it a regression *from* the game rather than a
/// place where we are merely as trusting as it is. `RemoteClient` keeps a counter per kind, bumps
/// it per edit packet, decays it each tick and boots past a ceiling. The numbers are transcribed
/// rather than chosen, so a client vanilla tolerates is tolerated here and vice versa.
#[cfg(test)]
mod tile_spam {
    use super::*;

    /// Placing is the tight one: 100, recovering 0.3 a tick.
    #[test]
    fn the_ceilings_and_decay_match_the_game() {
        assert_eq!(SPAM_PLACE_MAX, 100.0);
        assert_eq!(SPAM_PLACE_DECAY, 0.3);
        assert_eq!(SPAM_BREAK_MAX, 500.0);
        assert_eq!(SPAM_BREAK_DECAY, 5.0);
        assert_eq!(SPAM_LIQUID_MAX, 50.0);
        assert_eq!(SPAM_LIQUID_DECAY, 0.2);
    }

    /// Sustained placing above the decay rate eventually trips; ordinary building does not.
    ///
    /// At 60 ticks a second, 0.3 a tick is eighteen placements a second recovered. A player
    /// building fast is well under that; a script is not.
    #[test]
    fn a_realistic_building_rate_never_trips_the_limit() {
        // Ten placements a second for a solid minute, decaying each tick.
        let mut budget = 0.0f32;
        let mut worst = 0.0f32;
        for tick in 0..3600 {
            if tick % 6 == 0 {
                budget += 1.0;
            }
            budget = (budget - SPAM_PLACE_DECAY).max(0.0);
            worst = worst.max(budget);
        }
        assert!(
            worst < SPAM_PLACE_MAX,
            "ten placements a second reached {worst}, which would boot a player who is just \
             building quickly"
        );
    }

    /// And a client placing as fast as it can trips within a few seconds.
    #[test]
    fn a_flood_of_placements_trips_the_limit() {
        let mut budget = 0.0f32;
        let mut tripped = None;
        for tick in 0..600 {
            // Twenty a tick — a script, not a person.
            budget += 20.0;
            budget = (budget - SPAM_PLACE_DECAY).max(0.0);
            if budget > SPAM_PLACE_MAX && tripped.is_none() {
                tripped = Some(tick);
            }
        }
        let tripped = tripped.expect("a flood has to trip the limit");
        assert!(
            tripped < 60,
            "a flood took {tripped} ticks to be noticed; that is a second of free vandalism"
        );
    }

    /// Breaking is deliberately looser, because mining legitimately produces packets very fast.
    ///
    /// A `const` block, so swapping the two by accident fails the build rather than a test run.
    const _: () = assert!(SPAM_BREAK_MAX > SPAM_PLACE_MAX);
    const _: () = assert!(SPAM_BREAK_DECAY > SPAM_PLACE_DECAY);
}

/// The web panel's kick/ban/whitelist/world-view/world-switch events. Each one is checked
/// directly against `handle_event`, the same entry point the real panel HTTP handlers reach
/// through `ServerEvent` — see `tests/panel.rs` for the same features exercised end to end over a
/// real socket instead.
#[cfg(test)]
mod panel_admin_events {
    use super::*;
    use crate::admin::BanKind;
    use crate::config::Config;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "panel events probe")
    }

    /// A player in `ConnState::Playing`, inserted directly into the slot — the shape every other
    /// test in this file that needs a "connected" player without a real socket already uses.
    fn seat_player(server: &mut GameServer, slot: u8, name: &str) {
        let (tx, _rx) = mpsc::channel(4);
        let mut player = Player::new(slot, "127.0.0.1:4000".parse().unwrap(), tx);
        player.name = name.to_string();
        player.state = ConnState::Playing;
        server.players[slot as usize] = Some(player);
    }

    fn oneshot_reply<T>() -> (oneshot::Sender<T>, oneshot::Receiver<T>) {
        oneshot::channel()
    }

    #[test]
    fn kicking_a_connected_player_removes_them_and_reports_success() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        seat_player(&mut server, 0, "Griefer");

        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelKick {
            name: "griefer".into(), // case-insensitive, matching `/kick`
            reason: "wrecked spawn".into(),
            reply,
        });
        assert!(rx.try_recv().expect("a reply was sent").is_ok());
        assert!(
            server.players[0].is_none(),
            "the kicked player must actually be gone"
        );
    }

    #[test]
    fn kicking_nobody_reports_failure_without_touching_anyone() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelKick {
            name: "nobody-here".into(),
            reason: String::new(),
            reply,
        });
        assert!(rx.try_recv().expect("a reply was sent").is_err());
    }

    #[test]
    fn banning_a_connected_player_bans_and_kicks_them() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        seat_player(&mut server, 0, "Griefer");

        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelBan {
            kind: BanKind::Name,
            value: "Griefer".into(),
            reason: "wrecked spawn".into(),
            reply,
        });
        rx.try_recv().expect("a reply was sent");
        assert!(server.players[0].is_none(), "a banned player is removed");
        assert!(
            server.admin.ban_for("Griefer", "1.2.3.4", None).is_some(),
            "the ban itself must be recorded, not just the kick"
        );
    }

    #[test]
    fn unbanning_lifts_a_real_ban() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.admin.ban(BanKind::Name, "Griefer", "wrecked spawn");
        assert!(server.admin.ban_for("Griefer", "0.0.0.0", None).is_some());

        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelUnban {
            value: "Griefer".into(),
            reply,
        });
        assert_eq!(rx.try_recv().unwrap(), 1);
        assert!(server.admin.ban_for("Griefer", "0.0.0.0", None).is_none());
    }

    #[test]
    fn whitelist_add_and_remove_round_trip_through_the_events() {
        let mut server = GameServer::new(Config::default(), tiny_world());

        let (add_reply, mut add_rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelWhitelistAdd {
            name: "Brooklyn".into(),
            reply: add_reply,
        });
        assert!(add_rx.try_recv().unwrap());

        let (list_reply, mut list_rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelWhitelist { reply: list_reply });
        let list = list_rx.try_recv().unwrap();
        assert!(list.on);
        assert_eq!(list.names, vec!["Brooklyn".to_string()]);

        let (remove_reply, mut remove_rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelWhitelistRemove {
            name: "brooklyn".into(), // case-insensitive, matching the console command
            reply: remove_reply,
        });
        assert!(remove_rx.try_recv().unwrap());

        let (list_reply2, mut list_rx2) = oneshot_reply();
        server.handle_event(ServerEvent::PanelWhitelist { reply: list_reply2 });
        assert!(!list_rx2.try_recv().unwrap().on, "an empty list is off");
    }

    #[test]
    fn a_switch_to_a_real_file_arms_the_pending_switch_and_starts_stopping() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        let handle = server.world_switch_handle();
        let target = std::env::temp_dir().join(format!(
            "terrustia-panel-switch-test-{}.wld",
            std::process::id()
        ));
        std::fs::write(&target, b"not a real world, just needs to exist").unwrap();

        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelSwitchWorld {
            path: target.clone(),
            reply,
        });
        assert!(rx.try_recv().unwrap().is_ok());
        assert!(server.stopping, "a switch is a controlled shutdown");
        assert_eq!(
            handle.lock().unwrap().as_deref(),
            Some(target.as_path()),
            "main has to be able to read this back after `run` returns"
        );
        let _ = std::fs::remove_file(&target);
    }

    #[test]
    fn a_switch_to_a_missing_file_is_refused_and_does_not_stop_the_server() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelSwitchWorld {
            path: PathBuf::from("/no/such/world/anywhere.wld"),
            reply,
        });
        assert!(rx.try_recv().unwrap().is_err());
        assert!(!server.stopping);
    }

    #[test]
    fn tile_colour_buckets_match_the_generators_own_ids() {
        use crate::world::worldgen::tiles as t;

        let dirt = terrustia_proto::Tile {
            block: t::DIRT,
            flags: TileFlags(TileFlags::ACTIVE),
            ..terrustia_proto::Tile::AIR
        };
        assert_eq!(GameServer::tile_color(dirt), TileColor::Dirt);

        let stone = terrustia_proto::Tile {
            block: t::STONE,
            flags: TileFlags(TileFlags::ACTIVE),
            ..terrustia_proto::Tile::AIR
        };
        assert_eq!(GameServer::tile_color(stone), TileColor::Stone);

        assert_eq!(
            GameServer::tile_color(terrustia_proto::Tile::AIR),
            TileColor::Empty,
            "an inactive tile has nothing to colour"
        );

        let mut lava = terrustia_proto::Tile::AIR;
        lava.liquid = 255;
        lava.liquid_kind = terrustia_proto::Liquid::Lava;
        assert_eq!(GameServer::tile_color(lava), TileColor::Lava);
    }

    #[test]
    fn the_world_sample_never_exceeds_the_actual_world_and_never_panics_on_a_tiny_one() {
        let server = GameServer::new(Config::default(), crate::world::World::empty(200, 150, "s"));
        let sample = server.world_tile_sample();
        assert!(sample.sample_cols as i32 <= sample.world_width);
        assert!(sample.sample_rows as i32 <= sample.world_height);
        assert_eq!(
            sample.tiles.len(),
            (sample.sample_cols * sample.sample_rows) as usize
        );
    }

    #[test]
    fn equipped_items_only_reads_the_armour_slot_run_and_ignores_empty_slots() {
        let (tx, _rx) = mpsc::channel(4);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), tx);
        // Slot 10 is ordinary inventory, not gear — must be ignored.
        player.inventory.insert(
            10,
            terrustia_proto::inventory::SyncEquipment {
                player: 0,
                slot: 10,
                item: terrustia_proto::ItemStack {
                    id: 999,
                    stack: 1,
                    prefix: 0,
                },
                favorited: false,
                blocked: false,
            },
        );
        // Slot 60 is inside the armour run and carries a real item.
        player.inventory.insert(
            60,
            terrustia_proto::inventory::SyncEquipment {
                player: 0,
                slot: 60,
                item: terrustia_proto::ItemStack {
                    id: 42,
                    stack: 1,
                    prefix: 0,
                },
                favorited: false,
                blocked: false,
            },
        );
        // Slot 61 is inside the run but empty (item id 0) — must be ignored too.
        player.inventory.insert(
            61,
            terrustia_proto::inventory::SyncEquipment {
                player: 0,
                slot: 61,
                item: terrustia_proto::ItemStack {
                    id: 0,
                    stack: 0,
                    prefix: 0,
                },
                favorited: false,
                blocked: false,
            },
        );

        assert_eq!(GameServer::equipped_items(&player), vec![42]);
    }
}

/// The Lunatic Cultist's tablet (npc 437) — real vanilla places it at the dungeon entrance once
/// Golem is down, the same "periodic server-side check" shape `tick_old_man` already uses to keep
/// Skeletron reachable. Before this fix, nothing anywhere ever called `self.npcs.spawn` with npc
/// 437 at all — the Moon Lord acceptance-test bot's own finding (task #37), confirmed by direct
/// inspection: `CULTIST_TABLET` was a named constant nothing referenced as a spawn trigger. Every
/// test below fails on the unfixed code (no `tick_cultist_tablet` to call at all) and passes once
/// it exists and is wired into the tick loop.
#[cfg(test)]
mod cultist_tablet_trigger {
    use super::*;
    use crate::config::Config;
    use terrustia_proto::npc_params::CULTIST_TABLET;

    fn dungeon_world() -> crate::world::World {
        let mut world = crate::world::World::empty(200, 150, "cultist tablet probe");
        world.dungeon_x = Some(100);
        world.dungeon_y = Some(50);
        world
    }

    /// A playing player standing right at the dungeon entrance — near enough for
    /// `tick_cultist_tablet`'s own "somebody has to be there to see it" check, the same reasoning
    /// `tick_old_man` already uses for the same spot.
    fn seat_player_at_the_dungeon(server: &mut GameServer) {
        let (tx, _rx) = mpsc::channel(4);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), tx);
        player.state = ConnState::Playing;
        player.position = (100.0 * TILE_SIZE, 50.0 * TILE_SIZE);
        server.players[0] = Some(player);
    }

    #[test]
    fn the_tablet_appears_once_golem_and_skeletron_are_both_down() {
        let mut server = GameServer::new(Config::default(), dungeon_world());
        seat_player_at_the_dungeon(&mut server);
        server.world.progress.downed_golem = true;
        server.world.progress.downed_boss3 = true;

        server.tick_cultist_tablet();

        assert!(
            server
                .npcs
                .iter()
                .any(|(_, n)| n.npc_type == CULTIST_TABLET),
            "the tablet should have appeared at the dungeon entrance"
        );
    }

    /// Golem alone is not enough — real vanilla's own documented gate (`terraria.wiki.gg`'s
    /// "Cultists" page: the Old Man takes spawn priority over the same spot until Skeletron is
    /// down), and the same mutual exclusion `tick_old_man` above already enforces the other way.
    #[test]
    fn no_tablet_while_skeletron_is_still_undefeated() {
        let mut server = GameServer::new(Config::default(), dungeon_world());
        seat_player_at_the_dungeon(&mut server);
        server.world.progress.downed_golem = true;
        server.world.progress.downed_boss3 = false;

        server.tick_cultist_tablet();

        assert!(
            !server
                .npcs
                .iter()
                .any(|(_, n)| n.npc_type == CULTIST_TABLET),
            "Golem alone should not be enough"
        );
    }

    #[test]
    fn no_tablet_before_golem_is_down() {
        let mut server = GameServer::new(Config::default(), dungeon_world());
        seat_player_at_the_dungeon(&mut server);
        server.world.progress.downed_boss3 = true;
        server.world.progress.downed_golem = false;

        server.tick_cultist_tablet();

        assert!(
            !server
                .npcs
                .iter()
                .any(|(_, n)| n.npc_type == CULTIST_TABLET)
        );
    }

    /// Once the Lunatic Cultist has actually been beaten, the tablet does not return — this
    /// project's own reasoned assumption (disclosed in `tick_cultist_tablet`'s own doc comment),
    /// mirroring how `downed_boss3` already permanently retires the Old Man above.
    #[test]
    fn no_tablet_once_the_cultist_is_already_downed() {
        let mut server = GameServer::new(Config::default(), dungeon_world());
        seat_player_at_the_dungeon(&mut server);
        server.world.progress.downed_golem = true;
        server.world.progress.downed_boss3 = true;
        server.world.progress.downed_ancient_cultist = true;

        server.tick_cultist_tablet();

        assert!(
            !server
                .npcs
                .iter()
                .any(|(_, n)| n.npc_type == CULTIST_TABLET)
        );
    }

    /// Nobody standing nearby to see it appear — the same reasoning `tick_old_man` already
    /// applies to the Old Man's own arrival.
    #[test]
    fn no_tablet_with_nobody_watching() {
        let mut server = GameServer::new(Config::default(), dungeon_world());
        server.world.progress.downed_golem = true;
        server.world.progress.downed_boss3 = true;

        server.tick_cultist_tablet();

        assert!(
            !server
                .npcs
                .iter()
                .any(|(_, n)| n.npc_type == CULTIST_TABLET)
        );
    }
}

/// The Wall of Flesh's real vanilla trigger: a Guide Voodoo Doll destroyed by lava in the
/// Underworld while the Guide is alive. Before this fix, no packet a real client could ever send
/// spawned npc 113 at all — the Moon Lord acceptance-test bot's own finding (task #37), confirmed
/// by direct inspection: npc 113 is (deliberately) absent from `npc_params::SUMMONABLE`, and
/// nothing else in this file spawned it either. Every test below fails on the unfixed code (no
/// `tick_wall_of_flesh_trigger` to call at all) and passes once it exists and is wired into
/// `tick_items`.
#[cfg(test)]
mod wall_of_flesh_trigger {
    use super::*;
    use crate::config::Config;

    /// Real vanilla's Guide Voodoo Doll item id, confirmed via `terraria.wiki.gg`'s own infobox —
    /// see `tick_wall_of_flesh_trigger`'s own doc comment for the full citation.
    const GUIDE_VOODOO_DOLL: i32 = 267;
    const GUIDE: u16 = 22;
    const WALL_OF_FLESH: u16 = 113;

    fn underworld_world() -> crate::world::World {
        crate::world::World::empty(200, 400, "wall of flesh probe")
    }

    /// A tile of lava well within the underworld's own `height() - 200` band — the same threshold
    /// `bulbs.rs`'s own `UNDERWORLD` constant and `on_server_teleport`'s own inline arithmetic
    /// already use for the same question.
    fn put_lava_in_the_underworld(server: &mut GameServer) -> (i32, i32) {
        let x = 50;
        let y = server.world.height() - 50;
        server.world.set_tile(
            x,
            y,
            Tile::AIR.with_liquid(terrustia_proto::Liquid::Lava, 255),
        );
        (x, y)
    }

    #[test]
    fn a_guide_voodoo_doll_burning_in_underworld_lava_spawns_the_wall_and_kills_the_guide() {
        let mut server = GameServer::new(Config::default(), underworld_world());
        server.npcs.spawn(GUIDE, (500.0, 500.0)).expect("a slot");
        let (x, y) = put_lava_in_the_underworld(&mut server);
        server
            .items
            .spawn(
                ItemStack::new(GUIDE_VOODOO_DOLL, 1, 0),
                (x as f32 * TILE_SIZE, y as f32 * TILE_SIZE),
            )
            .expect("an item slot");

        server.tick_wall_of_flesh_trigger();

        assert!(
            server.npcs.iter().any(|(_, n)| n.npc_type == WALL_OF_FLESH),
            "the Wall of Flesh should have risen"
        );
        assert!(
            !server
                .npcs
                .iter()
                .any(|(_, n)| n.npc_type == GUIDE && n.is_alive()),
            "the guide should have died with the doll"
        );
        assert!(
            server.items.is_empty(),
            "the doll should have burned up in the process"
        );
    }

    /// The Guide must be alive beforehand — real vanilla's own confirmed requirement. Left
    /// narrowly disclosed here: without a general "items burn in lava" mechanic, a doll that
    /// cannot trigger anything is left alone rather than silently destroyed for no visible
    /// reason — see `summon_wall_of_flesh`'s own doc comment.
    #[test]
    fn nothing_happens_without_a_guide_alive() {
        let mut server = GameServer::new(Config::default(), underworld_world());
        let (x, y) = put_lava_in_the_underworld(&mut server);
        let index = server
            .items
            .spawn(
                ItemStack::new(GUIDE_VOODOO_DOLL, 1, 0),
                (x as f32 * TILE_SIZE, y as f32 * TILE_SIZE),
            )
            .expect("an item slot");

        server.tick_wall_of_flesh_trigger();

        assert!(!server.npcs.iter().any(|(_, n)| n.npc_type == WALL_OF_FLESH));
        assert!(
            server.items.get(index).is_some(),
            "left alone, since it could not trigger anything"
        );
    }

    #[test]
    fn an_ordinary_item_burning_in_the_same_lava_does_nothing() {
        let mut server = GameServer::new(Config::default(), underworld_world());
        server.npcs.spawn(GUIDE, (500.0, 500.0)).expect("a slot");
        let (x, y) = put_lava_in_the_underworld(&mut server);
        server
            .items
            .spawn(
                ItemStack::new(1, 1, 0), // not the doll
                (x as f32 * TILE_SIZE, y as f32 * TILE_SIZE),
            )
            .expect("an item slot");

        server.tick_wall_of_flesh_trigger();

        assert!(!server.npcs.iter().any(|(_, n)| n.npc_type == WALL_OF_FLESH));
    }

    /// The doll has to be in the Underworld itself, not merely in lava somewhere else in the
    /// world — matching real vanilla's own location requirement.
    #[test]
    fn a_doll_burning_outside_the_underworld_does_nothing() {
        let mut server = GameServer::new(Config::default(), underworld_world());
        server.npcs.spawn(GUIDE, (500.0, 500.0)).expect("a slot");
        let (x, y) = (50, 10); // the surface, not the underworld
        server.world.set_tile(
            x,
            y,
            Tile::AIR.with_liquid(terrustia_proto::Liquid::Lava, 255),
        );
        server
            .items
            .spawn(
                ItemStack::new(GUIDE_VOODOO_DOLL, 1, 0),
                (x as f32 * TILE_SIZE, y as f32 * TILE_SIZE),
            )
            .expect("an item slot");

        server.tick_wall_of_flesh_trigger();

        assert!(!server.npcs.iter().any(|(_, n)| n.npc_type == WALL_OF_FLESH));
    }

    /// Once hardmode has already begun there is nothing left for this trigger to do — matching
    /// `note_boss_kill_inner`'s own `if !p.hard_mode` guard on the death side.
    #[test]
    fn nothing_happens_once_hardmode_has_already_begun() {
        let mut server = GameServer::new(Config::default(), underworld_world());
        server.npcs.spawn(GUIDE, (500.0, 500.0)).expect("a slot");
        server.world.progress.hard_mode = true;
        let (x, y) = put_lava_in_the_underworld(&mut server);
        server
            .items
            .spawn(
                ItemStack::new(GUIDE_VOODOO_DOLL, 1, 0),
                (x as f32 * TILE_SIZE, y as f32 * TILE_SIZE),
            )
            .expect("an item slot");

        server.tick_wall_of_flesh_trigger();

        assert!(!server.npcs.iter().any(|(_, n)| n.npc_type == WALL_OF_FLESH));
    }
}

/// Real server-side coverage for the seven boss-drop-table bugs a parallel audit found this
/// session, cross-referenced against `ItemDropDatabase.cs` and fixed in `conditional_drops.rs`
/// (`conditional_chains`, `moon_lord_weapons`, `bundled_with`, the Creeper npc-id fix, the Queen
/// Slime trophy fix).
///
/// `conditional_drops.rs`'s own test module pins the *data* — the exact item ids, rates, and
/// which npc a rule is wired to. It cannot pin the *algorithms* that actually roll that data,
/// because those live here, in `drop_loot`: break-on-first-success for a chain, draw-without-
/// replacement for Moon Lord's pair, and "spawn a companion item" for Golem's bundle. These tests
/// drive the real consumer end to end instead.
#[cfg(test)]
mod boss_drop_table_fixes {
    use super::*;
    use crate::config::Config;
    use crate::world::items::ItemStore;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "boss drop table probe")
    }

    /// Every `(item id, stack size)` this one kill spawned, in allocation order.
    /// `ItemStore::spawn` always fills the lowest free slot, and nothing here ever removes an
    /// item, so resetting the store immediately before each kill means everything found
    /// afterward belongs to that kill alone — no need to diff against a running total.
    fn kill_and_collect(server: &mut GameServer, npc_type: u16) -> Vec<(i32, i16)> {
        server.items = ItemStore::new();
        server.drop_loot(npc_type, (0.0, 0.0), false);
        let mut items: Vec<(i16, i32, i16)> = server
            .items
            .iter()
            .map(|(index, it)| (index, it.item.id, it.item.stack))
            .collect();
        items.sort_unstable_by_key(|(index, _, _)| *index);
        items
            .into_iter()
            .map(|(_, id, stack)| (id, stack))
            .collect()
    }

    /// Bug #1, driven end to end: Moon Lord must hand back exactly two items from his real
    /// ten-weapon pool, and never the same one twice. The unfixed code had no case for npc 398 at
    /// all, so this pool never dropped anything; a naive fix drawing from `one_from`'s own
    /// independent-per-pool mechanism could still repeat the same weapon (~1-in-10 per kill) —
    /// this pins the actual without-replacement algorithm in `drop_loot`.
    #[test]
    fn moon_lord_always_drops_two_distinct_signature_weapons() {
        const MOON_LORD: u16 = 398;
        const POOL: [i32; 10] = [3063, 3389, 3065, 1553, 3930, 3541, 3570, 3571, 3569, 5480];
        let mut server = GameServer::new(Config::default(), tiny_world());
        for trial in 0..60 {
            let dropped = kill_and_collect(&mut server, MOON_LORD);
            let picked: Vec<i32> = dropped
                .into_iter()
                .map(|(id, _)| id)
                .filter(|id| POOL.contains(id))
                .collect();
            assert_eq!(
                picked.len(),
                2,
                "trial {trial}: exactly two signature weapons, got {picked:?}"
            );
            assert_ne!(
                picked[0], picked[1],
                "trial {trial}: never the same weapon twice"
            );
        }
    }

    /// Bug #1, expert side: expert mode replaces this pool with nothing at all — the treasure bag
    /// carries it instead, same as every other boss's ordinary loot.
    #[test]
    fn moon_lord_gives_none_of_his_signature_weapons_in_expert() {
        const MOON_LORD: u16 = 398;
        const POOL: [i32; 10] = [3063, 3389, 3065, 1553, 3930, 3541, 3570, 3571, 3569, 5480];
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.world.game_mode = 1; // expert
        for trial in 0..10 {
            let dropped = kill_and_collect(&mut server, MOON_LORD);
            assert!(
                !dropped.iter().any(|(id, _)| POOL.contains(id)),
                "trial {trial}: expert should give none of these: {dropped:?}"
            );
        }
    }

    /// Bug #2, driven end to end: Queen Bee must never hand back both the Hive Wand and a piece
    /// of Bee armor in the same kill. The unfixed code rolled 1129 as an independent
    /// `classic_only` entry and spawned an armor piece from `one_from` *unconditionally* — so
    /// both a guaranteed armor piece and a possible wand could land together.
    #[test]
    fn queen_bee_never_gives_the_wand_and_armor_together() {
        const QUEEN_BEE: u16 = 222;
        const BEE_STUFF: [i32; 4] = [1129, 842, 843, 844];
        let mut server = GameServer::new(Config::default(), tiny_world());
        let mut saw_wand = false;
        let mut saw_armor = false;
        for trial in 0..150 {
            let dropped = kill_and_collect(&mut server, QUEEN_BEE);
            let hits: Vec<i32> = dropped
                .into_iter()
                .map(|(id, _)| id)
                .filter(|id| BEE_STUFF.contains(id))
                .collect();
            assert!(
                hits.len() <= 1,
                "trial {trial}: at most one of the wand/armor, got {hits:?}"
            );
            match hits.first() {
                Some(1129) => saw_wand = true,
                Some(_) => saw_armor = true,
                None => {}
            }
        }
        assert!(
            saw_wand,
            "150 trials never landed the wand — check the odds"
        );
        assert!(
            saw_armor,
            "150 trials never landed any armor piece — check the odds"
        );
    }

    /// Bug #3, driven end to end: Skeletron must never hand back more than one of its three
    /// weapons in the same kill — the unfixed code rolled all three as independent `classic_only`
    /// entries, so a single kill could give 0, 1, 2 or all 3.
    #[test]
    fn skeletron_never_gives_more_than_one_weapon_per_kill() {
        const SKELETRON: u16 = 35;
        const WEAPONS: [i32; 3] = [1281, 1273, 1313];
        let mut server = GameServer::new(Config::default(), tiny_world());
        let mut saw_any = false;
        for trial in 0..250 {
            let dropped = kill_and_collect(&mut server, SKELETRON);
            let hits: Vec<i32> = dropped
                .into_iter()
                .map(|(id, _)| id)
                .filter(|id| WEAPONS.contains(id))
                .collect();
            assert!(
                hits.len() <= 1,
                "trial {trial}: at most one weapon, got {hits:?}"
            );
            saw_any |= !hits.is_empty();
        }
        assert!(
            saw_any,
            "250 trials never landed any weapon — check the odds"
        );
    }

    /// Bug #5, driven end to end: King Slime must get exactly one of the Slime Hook or Slime Gun
    /// every single kill — never both, and, critically, never neither. The unfixed code only ever
    /// had the 1/3 Slime Hook roll, so roughly two kills in three gave neither item.
    #[test]
    fn king_slime_always_gets_exactly_one_of_hook_or_gun() {
        const KING_SLIME: u16 = 50;
        let mut server = GameServer::new(Config::default(), tiny_world());
        let mut saw_hook = false;
        let mut saw_gun = false;
        for trial in 0..60 {
            let dropped = kill_and_collect(&mut server, KING_SLIME);
            let hook = dropped.iter().filter(|(id, _)| *id == 2585).count();
            let gun = dropped.iter().filter(|(id, _)| *id == 2610).count();
            assert_eq!(
                hook + gun,
                1,
                "trial {trial}: exactly one of the two, got {dropped:?}"
            );
            saw_hook |= hook == 1;
            saw_gun |= gun == 1;
        }
        assert!(saw_hook, "60 trials never landed the Slime Hook");
        assert!(saw_gun, "60 trials never landed the Slime Gun");
    }

    /// Bug #7, driven end to end: whenever Golem's pool draw is the Stynger, the same kill must
    /// also carry 60-180 of its own Stynger Bolt — and no other pick brings anything extra. The
    /// unfixed code spawned only whatever `one_from` picked, with no notion of a bundled item, so
    /// item 1261 never dropped from anywhere.
    #[test]
    fn golems_stynger_pick_always_brings_its_own_bolts() {
        const GOLEM: u16 = 245;
        let mut server = GameServer::new(Config::default(), tiny_world());
        let mut saw_stynger = false;
        for trial in 0..300 {
            let dropped = kill_and_collect(&mut server, GOLEM);
            let stynger = dropped.iter().filter(|(id, _)| *id == 1258).count();
            let bolts = dropped.iter().find(|(id, _)| *id == 1261);
            assert!(
                stynger <= 1,
                "trial {trial}: the pool draws exactly one item"
            );
            if stynger == 1 {
                saw_stynger = true;
                let (_, stack) = *bolts.unwrap_or_else(|| {
                    panic!("trial {trial}: Stynger without its own bolts: {dropped:?}")
                });
                assert!(
                    (60..=180).contains(&stack),
                    "trial {trial}: bolt stack {stack} out of the real 60-180 range"
                );
            } else {
                assert!(
                    bolts.is_none(),
                    "trial {trial}: bolts without Stynger: {dropped:?}"
                );
            }
        }
        assert!(
            saw_stynger,
            "300 trials never drew the Stynger — check the odds"
        );
    }
}

/// Real server-side coverage for the numerator fix in `conditional_drops.rs`: `Conditional` used
/// to have no way to represent a real vanilla `chanceNumerator` other than `1`, so every rule with
/// a real rate of `M`-in-`N` (`M != 1`) was modelled at the wrong, too-low `1`-in-`N` instead. The
/// unit tests in `conditional_drops.rs` pin the exact numerator/denominator this module now
/// carries; these drive the real `drop_loot` consumer over many trials to prove the roll it
/// actually performs lands at the *real* rate rather than the old one — the same lesson the Queen
/// Bee test in `boss_drop_table_fixes` already taught this project: a correct-looking data table
/// can still be wrong if the consumer never reads the field.
///
/// Every trial count below is chosen so the real rate and the old (pre-fix) rate are each roughly
/// ten standard deviations from the threshold — at that separation a false result from ordinary RNG
/// variance is not a realistic concern.
#[cfg(test)]
mod conditional_numerator_fixes {
    use super::*;
    use crate::config::Config;
    use crate::world::items::ItemStore;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "numerator fix probe")
    }

    /// Every item id this one kill spawned, in allocation order — see the identical helper in
    /// `boss_drop_table_fixes` for why resetting the store first makes this exact.
    fn kill_and_collect_ids(server: &mut GameServer, npc_type: u16) -> Vec<i32> {
        server.items = ItemStore::new();
        server.drop_loot(npc_type, (0.0, 0.0), false);
        let mut items: Vec<(i16, i32)> = server
            .items
            .iter()
            .map(|(index, it)| (index, it.item.id))
            .collect();
        items.sort_unstable_by_key(|(index, _)| *index);
        items.into_iter().map(|(_, id)| id).collect()
    }

    /// The Creeper's Tissue Sample (1329) and Crimtane Ore (880): real vanilla rolls both at
    /// 2-in-3 in classic (`ItemDropDatabase.cs:502-503`), not the 1-in-3 this project modelled
    /// before `Conditional` had a numerator field. 300 trials: 2-in-3 has a mean of 200 (sd ~8.2),
    /// 1-in-3 a mean of 100 (sd ~8.2) — the 150 threshold below sits about six standard deviations
    /// from either, so this distinguishes the two rates rather than just checking "something
    /// dropped."
    #[test]
    fn the_creeper_drops_tissue_sample_and_crimtane_at_two_in_three_not_one_in_three() {
        const CREEPER: u16 = 267;
        const TRIALS: usize = 300;
        let mut server = GameServer::new(Config::default(), tiny_world());
        let mut tissue_hits = 0usize;
        let mut crimtane_hits = 0usize;
        for _ in 0..TRIALS {
            let dropped = kill_and_collect_ids(&mut server, CREEPER);
            tissue_hits += dropped.iter().filter(|&&id| id == 1329).count();
            crimtane_hits += dropped.iter().filter(|&&id| id == 880).count();
        }
        assert!(
            tissue_hits > 150,
            "tissue sample landed {tissue_hits}/{TRIALS} — real rate is 2-in-3 (~200), not 1-in-3 (~100)"
        );
        assert!(
            crimtane_hits > 150,
            "crimtane landed {crimtane_hits}/{TRIALS} — real rate is 2-in-3 (~200), not 1-in-3 (~100)"
        );
    }

    /// Queen Bee's own `ByCondition(condition, 1130, 4, 10, 30, 3)` (`ItemDropDatabase.cs:551`):
    /// real rate is 3-in-4 (mean 225 of 300, sd ~7.5), not the 1-in-4 this project modelled before
    /// (mean 75). The 150 threshold sits ten standard deviations from either.
    #[test]
    fn queen_bee_drops_item_1130_at_three_in_four_not_one_in_four() {
        const QUEEN_BEE: u16 = 222;
        const TRIALS: usize = 300;
        let mut server = GameServer::new(Config::default(), tiny_world());
        let mut hits = 0usize;
        for _ in 0..TRIALS {
            let dropped = kill_and_collect_ids(&mut server, QUEEN_BEE);
            hits += dropped.iter().filter(|&&id| id == 1130).count();
        }
        assert!(
            hits > 150,
            "item 1130 landed {hits}/{TRIALS} — real rate is 3-in-4 (~225), not 1-in-4 (~75)"
        );
    }

    /// The hornet family's own `DropBasedOnExpertMode(CommonDrop(209, 3, 1, 1, 2), Common(209))`
    /// (`ItemDropDatabase.cs:1170`): classic's real rate is 2-in-3 (mean 200 of 300, sd ~8.2), not
    /// the 1-in-3 this project modelled before — a gap this numerator audit found fresh, not one of
    /// the two already known when it started. Expert stays unconditional (100%), unaffected by this
    /// fix and checked here as a same-test regression guard.
    #[test]
    fn hornet_stinger_drops_at_two_in_three_in_classic_not_one_in_three() {
        const HORNET: u16 = 42;
        const TRIALS: usize = 300;
        let mut server = GameServer::new(Config::default(), tiny_world());
        let mut hits = 0usize;
        for _ in 0..TRIALS {
            let dropped = kill_and_collect_ids(&mut server, HORNET);
            hits += dropped.iter().filter(|&&id| id == 209).count();
        }
        assert!(
            hits > 150,
            "stinger landed {hits}/{TRIALS} — real classic rate is 2-in-3 (~200), not 1-in-3 (~100)"
        );

        server.world.game_mode = 1; // expert
        for _ in 0..30 {
            let dropped = kill_and_collect_ids(&mut server, HORNET);
            assert!(
                dropped.contains(&209),
                "expert's stinger roll is unconditional (chanceDenominator: 1)"
            );
        }
    }

    /// The Black Recluse's own `DropBasedOnExpertMode(Common(2607, 2, 1, 3), CommonDrop(2607, 10,
    /// 1, 3, 9))` (`ItemDropDatabase.cs:959`): before this fix, every mode gave the same flat
    /// 1-in-2 (mean 150 of 300) because the rule was never mode-branched at all — real expert is
    /// 9-in-10 (mean 270, sd ~5.2), a real, material difference from classic this test proves the
    /// consumer now actually rolls, not just that `conditional_drops.rs`'s own data table has two
    /// different numbers in it.
    #[test]
    fn black_recluse_drops_its_own_item_far_more_often_in_expert_than_classic() {
        const BLACK_RECLUSE: u16 = 163;
        const TRIALS: usize = 300;
        let mut server = GameServer::new(Config::default(), tiny_world());

        let mut classic_hits = 0usize;
        for _ in 0..TRIALS {
            let dropped = kill_and_collect_ids(&mut server, BLACK_RECLUSE);
            classic_hits += dropped.iter().filter(|&&id| id == 2607).count();
        }

        server.world.game_mode = 1; // expert
        let mut expert_hits = 0usize;
        for _ in 0..TRIALS {
            let dropped = kill_and_collect_ids(&mut server, BLACK_RECLUSE);
            expert_hits += dropped.iter().filter(|&&id| id == 2607).count();
        }

        assert!(
            (100..200).contains(&classic_hits),
            "classic landed {classic_hits}/{TRIALS} — real classic rate is 1-in-2 (~150)"
        );
        assert!(
            expert_hits > 230,
            "expert landed {expert_hits}/{TRIALS} — real expert rate is 9-in-10 (~270), not the classic 1-in-2 (~150)"
        );
    }
}

/// Wood, closed as a real gap this session: a freshly generated world had trees but no drop
/// mapping for tile 5 at all — chopping one gave nothing, silently, the first material every
/// crafting recipe in the game starts from. `moonlord.rs`'s own doc comment first found and
/// disclosed this live. Fixed by [`GameServer::spawn_tree_drop`], transcribed from
/// `WorldGen.KillTile_GetTreeDrops`.
#[cfg(test)]
mod wood_from_trees {
    use super::*;
    use crate::config::Config;
    use crate::world::items::ItemStore;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "tree drop probe")
    }

    /// Plants a one-tile trunk on top of `ground_block` at `(10, 100)` and returns a server ready
    /// to break it — the trunk's own position, `(10, 99)`, is what `spawn_tile_drop` is called
    /// with in each test below.
    fn planted(ground_block: u16) -> GameServer {
        let mut world = tiny_world();
        world.set_tile(10, 100, Tile::block(ground_block));
        world.set_tile(10, 99, Tile::framed(5, 0, 0));
        GameServer::new(Config::default(), world)
    }

    fn broken_ids(server: &mut GameServer, frame_x: i16, frame_y: i16) -> Vec<i32> {
        server.items = ItemStore::new();
        server.spawn_tile_drop(5, frame_x, frame_y, 10, 99);
        let mut items: Vec<(i16, i32)> = server
            .items
            .iter()
            .map(|(index, it)| (index, it.item.id))
            .collect();
        items.sort_unstable_by_key(|(index, _)| *index);
        items.into_iter().map(|(_, id)| id).collect()
    }

    /// Ordinary forest grass (block 2, `GetTreeType`'s default) gives plain Wood (item 9), not
    /// nothing — the exact gap this test closes.
    #[test]
    fn a_tree_rooted_in_forest_grass_drops_wood() {
        let mut server = planted(2);
        let dropped = broken_ids(&mut server, 0, 0);
        assert!(
            dropped.contains(&9),
            "expected Wood (9) from a plain trunk segment, got {dropped:?}"
        );
    }

    /// The five other real biome ground types each give their own named wood
    /// (`WorldGen.GetTreeType`'s switch), not the plain forest item.
    #[test]
    fn each_biomes_ground_gives_that_biomes_own_wood() {
        for (ground, expected, name) in [
            (23u16, 619i32, "Ebonwood from Corruption grass"),
            (199, 911, "Shadewood from Crimson grass"),
            (60, 620, "Rich Mahogany from Jungle grass"),
            (109, 621, "Pearlwood from Hallowed grass"),
            (147, 2503, "Boreal Wood from Snow"),
        ] {
            let mut server = planted(ground);
            let dropped = broken_ids(&mut server, 0, 0);
            assert!(
                dropped.contains(&expected),
                "{name}: expected item {expected}, got {dropped:?}"
            );
        }
    }

    /// A tree with no resolvable ground under it (nothing planted below the trunk at all) still
    /// gives plain Wood — vanilla's own fallback (`GetTreeType`'s `default: return TreeTypes.None`
    /// still reaches `KillTile_GetTreeDrops`'s unconditional `dropItem = 9` before the species
    /// switch), not a silently discarded drop.
    #[test]
    fn a_tree_with_unresolvable_ground_still_drops_plain_wood() {
        let mut world = tiny_world();
        // No ground tile placed at all under the trunk.
        world.set_tile(10, 99, Tile::framed(5, 0, 0));
        let mut server = GameServer::new(Config::default(), world);
        let dropped = broken_ids(&mut server, 0, 0);
        assert!(
            dropped.contains(&9),
            "expected the plain-Wood fallback, got {dropped:?}"
        );
    }

    /// A Mushroom-grass-rooted tree gives a Glowing Mushroom about half the time and nothing the
    /// other half, never wood (`KillTile_GetTreeDrops`'s `TreeTypes.Mushroom` arm: `dropItem =
    /// (genRand.Next(2)==0) ? 183 : 0`). 300 trials, mean 150 (sd ~8.7) if the roll is real.
    #[test]
    fn a_tree_rooted_in_mushroom_grass_sometimes_gives_a_glowing_mushroom_never_wood() {
        const TRIALS: usize = 300;
        let mut server = planted(70);
        let mut hits = 0usize;
        for _ in 0..TRIALS {
            let dropped = broken_ids(&mut server, 0, 0);
            assert!(
                !dropped.contains(&9),
                "a Mushroom-biome tree should never give plain Wood, got {dropped:?}"
            );
            hits += dropped.iter().filter(|&&id| id == 183).count();
        }
        assert!(
            (100..200).contains(&hits),
            "glowing mushroom landed {hits}/{TRIALS} — real rate is 1-in-2 (~150)"
        );
    }

    /// Breaking the canopy-top frame (`frameX >= 22 && frameY >= 198`, vanilla's own literal
    /// condition) on acorn-capable ground gives an Acorn about half the time, alongside the wood —
    /// not instead of it. 300 trials, mean 150 (sd ~8.7).
    #[test]
    fn the_canopy_top_sometimes_also_drops_an_acorn() {
        const TRIALS: usize = 300;
        let mut server = planted(2);
        let mut acorns = 0usize;
        for _ in 0..TRIALS {
            let dropped = broken_ids(&mut server, 22, 198);
            assert!(
                dropped.contains(&9),
                "the canopy should still give Wood alongside any acorn, got {dropped:?}"
            );
            acorns += dropped.iter().filter(|&&id| id == 27).count();
        }
        assert!(
            (100..200).contains(&acorns),
            "acorn landed {acorns}/{TRIALS} — real rate off the canopy top is 1-in-2 (~150)"
        );
    }

    /// Jungle trees never give an acorn even off the canopy top — `TreeTypeDropsAcorns` excludes
    /// Jungle by name, since Rich Mahogany propagates by a sapling players plant, not a bonus item.
    #[test]
    fn jungle_trees_never_drop_an_acorn_even_off_the_canopy() {
        let mut server = planted(60);
        for _ in 0..40 {
            let dropped = broken_ids(&mut server, 22, 198);
            assert!(
                !dropped.contains(&27),
                "a Jungle tree's canopy should never give an acorn, got {dropped:?}"
            );
        }
    }

    /// A non-canopy frame never gives an acorn, on any ground — only the leafy top can.
    #[test]
    fn a_trunk_segment_never_drops_an_acorn() {
        let mut server = planted(2);
        for _ in 0..40 {
            let dropped = broken_ids(&mut server, 0, 0);
            assert!(
                !dropped.contains(&27),
                "a plain trunk segment should never give an acorn, got {dropped:?}"
            );
        }
    }

    /// "Bonus wood" (a second Wood in the same stack) lands about a third of the time — the one
    /// real, item-independent term in vanilla's own roll (`Main.rand.Next(3) == 0`) this fix
    /// transcribes; the axe-power-scaled term is a disclosed, separate gap (see
    /// `spawn_tree_drop`'s own doc comment). 300 trials, mean 100 (sd ~8.2).
    #[test]
    fn bonus_wood_lands_about_a_third_of_the_time() {
        const TRIALS: usize = 300;
        let mut server = planted(2);
        let mut bonus = 0usize;
        for _ in 0..TRIALS {
            server.items = ItemStore::new();
            server.spawn_tile_drop(5, 0, 0, 10, 99);
            let wood_stack: i16 = server
                .items
                .iter()
                .filter(|(_, it)| it.item.id == 9)
                .map(|(_, it)| it.item.stack)
                .sum();
            assert!(wood_stack == 1 || wood_stack == 2, "got stack {wood_stack}");
            if wood_stack == 2 {
                bonus += 1;
            }
        }
        assert!(
            (70..130).contains(&bonus),
            "bonus wood landed {bonus}/{TRIALS} — real item-independent rate is 1-in-3 (~100)"
        );
    }
}
