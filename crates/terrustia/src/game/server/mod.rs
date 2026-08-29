//! The single-writer game task.
//!
//! One task owns the world and the player table, so there are no locks on the hot path and packet
//! ordering is deterministic. Connections talk to it over an `mpsc` of [`ServerEvent`]; it talks
//! back through each player's outbound queue.

use std::{
    collections::{HashMap, HashSet, VecDeque},
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
use tokio::sync::{mpsc, oneshot};

use tracing::{debug, error, info, warn};

use crate::{
    config::Config,
    game::player::{ConnState, Player},
    game::{
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

mod console;
mod dispatch;
mod panel;
mod systems;
mod tick;

use tick::TickCost;

// The panel's snapshot types are defined beside the code that fills them in, and re-exported here
// so `crate::panel` keeps addressing them as `game::server::PanelStatus` and friends.
pub use panel::{
    PanelAccountInfo, PanelAccounts, PanelAuthLookup, PanelBackupEntry, PanelBackups,
    PanelConfigSnapshot, PanelGroupInfo, PanelMetrics, PanelPlayer, PanelStatus, PanelWhitelist,
    PanelWorldTiles, TileColor,
};

/// Signs hold a page of text at most; anything longer is a client that is not playing fair.
const MAX_SIGN_TEXT: usize = 1000;

/// How far a player can be from an item and still have it reserved for them, in pixels.
///
/// Generous on purpose: the reservation only grants the right to pick the item up, and a client
/// needs to hold it before its own grab animation begins.
const ITEM_GRAB_RANGE: f32 = 400.0;

/// How much of a tick a single player's queued section stream may spend before
/// `drain_section_streams` picks it back up next tick, rather than draining it in one shot.
///
/// Set from the worst single-section encode this project has ever measured (`examples/
/// sectioncost.rs`: ~2,976µs on an 8400×2400 world) with headroom, so this phase's own share of a
/// tick stays bounded and predictable rather than reproducing the exact synchronous-burst problem
/// it exists to fix, just moved from one packet handler into the tick loop. At least one section
/// still goes out every tick a queue is non-empty, even past the budget, so a stream always makes
/// forward progress.
const SECTION_STREAM_BUDGET: Duration = Duration::from_micros(4_000);

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

/// Whether two `(left, top, right, bottom)` boxes overlap — the same open-interval test as
/// `Rectangle.Intersects` (touching edges do not count), used to keep a meteor off any player or
/// NPC.
fn boxes_overlap(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    a.0 < b.2 && a.2 > b.0 && a.1 < b.3 && a.3 > b.1
}

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
    /// A live snapshot of the last tick's cost and the current entity counts, for the panel's
    /// metrics view. Reads [`Self::last_tick`] and a few `len()`s — nothing that costs the tick.
    PanelMetrics {
        reply: oneshot::Sender<PanelMetrics>,
    },
    /// The world backups on disk right now, for the panel's backup/rollback view. The listing
    /// itself is a filesystem read, but which file is *current* — and therefore what the backups
    /// belong to — is game-task state, so it is answered here.
    PanelBackups {
        reply: oneshot::Sender<PanelBackups>,
    },
    /// The panel's "save now" button. Reuses the same background save the console's `save` command
    /// runs, so it never blocks the tick.
    PanelForceSave {
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// The panel's rollback button. Reuses [`Self::roll_back`] exactly — the same destructive,
    /// stop-and-reload operation the console `rollback` command performs — and hands its message
    /// back so the panel can show it.
    PanelRollback {
        which: usize,
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// The groups and accounts, for the panel's accounts admin view.
    PanelAccounts {
        reply: oneshot::Sender<PanelAccounts>,
    },
    /// Move an account into a different group. Same rule the console `group` command enforces (the
    /// group must exist), plus a guard the panel needs and the console does not: it refuses to
    /// strip the last account that can still administer the server, so nobody locks themselves out
    /// through the very screen they would use to fix it.
    PanelSetAccountGroup {
        name: String,
        group: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// The panel inserting an account it already hashed off the game task, into a chosen group.
    /// Distinct from [`Self::PanelInsertAccount`], the claim path, which always uses `owner`:
    /// this one validates the requested group exists first.
    PanelCreateAccount {
        account: crate::admin::Account,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Delete an account. Guarded the same way [`Self::PanelSetAccountGroup`] is: the last account
    /// that can administer the server cannot be removed.
    PanelDeleteAccount {
        name: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
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
    /// The palette the boot chose, so the live status footer can colour itself to match — or emit
    /// nothing when colour is off. Defaults to plain; `main` sets it via [`Self::with_palette`].
    palette: crate::term::Palette,
    /// The most recent tick's cost, surfaced live to the web panel's metrics view. Unlike
    /// [`Self::worst_tick`] — a windowed maximum that is taken and reset every report — this is
    /// simply the last tick that ran, so a graph drawn from it moves every frame. Never persisted.
    last_tick: TickCost,
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
            last_tick: TickCost::default(),
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
            palette: crate::term::Palette::PLAIN,
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

    /// Give the game task the boot's colour palette, so the live status footer it keeps at the
    /// bottom of the terminal can match it. Builder-style for the same reason as the two above —
    /// the many test call sites want a plain default, not a palette they have no terminal for.
    pub fn with_palette(mut self, palette: crate::term::Palette) -> Self {
        self.palette = palette;
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
    ///
    /// Returns the message describing what happened either way, so a caller that is not the console
    /// (the web panel) can report it to whoever asked rather than only to the log. The console arm
    /// logs whatever comes back; on success the same announce/stop side effects happen regardless of
    /// who called.
    fn roll_back(&mut self, which: usize) -> Result<String, String> {
        let Some(path) = self.save_path.clone() else {
            return Err(
                "this world is not being saved, so there is nothing to roll back to".into(),
            );
        };
        if which == 0 || which > crate::world::wld_save::BACKUPS_KEPT {
            return Err(format!(
                "there are only {} backups; use `rollback 1` for the most recent",
                crate::world::wld_save::BACKUPS_KEPT
            ));
        }
        let bak = path.with_extension(format!("wld.bak{which}"));
        if !bak.exists() {
            return Err(format!("there is no backup #{which} to roll back to"));
        }
        // Check it before trusting it. Restoring an unreadable file over a readable one would turn
        // a rollback into the very thing it exists to undo.
        std::fs::read(&bak)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                crate::world::wld::parse(&bytes)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            })
            .map_err(|e| format!("that backup will not load; refusing to roll back to it: {e}"))?;
        // Keep what is being replaced, so a rollback is itself reversible.
        let aside = path.with_extension("wld.before-rollback");
        if path.exists()
            && let Err(e) = std::fs::rename(&path, &aside)
        {
            return Err(format!(
                "could not move the current world aside; refusing to roll back: {e}"
            ));
        }
        if let Err(e) = std::fs::copy(&bak, &path) {
            let _ = std::fs::rename(&aside, &path);
            return Err(format!("could not restore the backup: {e}"));
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
        Ok(format!(
            "rolled back to backup #{which}; the server is stopping so it loads cleanly on the \
             next start"
        ))
    }

    /// Print the one-time claim token, if this server has not been claimed yet.
    ///
    /// Only ever to the log, which means the terminal or the service journal — never to a player.
    /// The whole point is that claiming requires someone who can see the server's own output.
    fn announce_claim_token(&mut self) {
        if !self.admin.unclaimed() {
            return;
        }
        // Not a password: a short one-time secret that lives for one process. It gates ownership on
        // two network-reachable paths (`/register <name> <pw> <token>` and an unclaimed panel
        // login), so it is drawn from a real CSPRNG — the same `rand_core::OsRng` the panel's own
        // session tokens use — rather than the clock+pid it used to be, whose entropy an attacker
        // who can estimate the boot time could search. It only has to be unguessable by someone who
        // cannot see this line.
        use argon2::password_hash::rand_core::{OsRng, RngCore};
        const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789"; // 30 symbols
        let mut token = String::with_capacity(12);
        while token.len() < 12 {
            let mut byte = [0u8; 1];
            OsRng.fill_bytes(&mut byte);
            // Reject the top of the byte range so the map onto 30 symbols is unbiased (240 = 8*30).
            if byte[0] < 240 {
                token.push(ALPHABET[(byte[0] % 30) as usize] as char);
            }
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
    /// Drop into the world every pickup a mass-wire run reported — one wire or actuator at each
    /// tile a cut cleared, at that tile's own pixel centre. Vanilla's `KillWire`/`KillActuator`
    /// spawn these the instant they clear a flag; this is the same, batched by the caller. Without
    /// it a mistake with the Grand Design simply destroyed the wire the player paid for.
    fn spawn_wire_drops(&mut self, drops: &[(i32, i32, u16)]) {
        for &(x, y, item_id) in drops {
            let position = (x as f32 * 16.0 + 8.0, y as f32 * 16.0 + 8.0);
            self.spawn_item(ItemStack::new(i32::from(item_id), 1, 0), position);
        }
    }

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
                let hash_and_group = self.admin.account_hash_and_group(&name);
                let admin = hash_and_group.as_ref().is_some_and(|(_, group)| {
                    self.admin
                        .group_grants(group, crate::admin::Permission::Admin)
                });
                let _ = reply.send(PanelAuthLookup {
                    unclaimed: self.admin.unclaimed(),
                    claim_token: self.claim_token.clone(),
                    hash_and_group,
                    admin,
                });
            }
            ServerEvent::PanelInsertAccount { account, reply } => {
                // The claim path. Persist and retire the one-time token on success, exactly as the
                // console `claim` command does (`run_console`'s `"claim"` arm) — without the save, a
                // server claimed through the panel would forget its owner on the next restart (until
                // some later admin mutation happened to save), and the spent token would linger.
                let result = self.admin.insert_account(account);
                if result.is_ok() {
                    let _ = self.admin.save();
                    self.claim_token = None;
                }
                let _ = reply.send(result);
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
            ServerEvent::PanelMetrics { reply } => {
                let _ = reply.send(self.panel_metrics());
            }
            ServerEvent::PanelBackups { reply } => {
                let _ = reply.send(self.panel_backups());
            }
            ServerEvent::PanelForceSave { reply } => {
                if self.save_path.is_none() {
                    let _ = reply.send(Err(
                        "this world is not being saved, so there is nothing to save".into(),
                    ));
                    return;
                }
                // The same background save the console `save` command runs — off the tick, one at a
                // time. The outcome reaches the operator on the live console feed, the same place
                // the console command's own "World saved" line goes.
                self.save_world_in_background("web panel");
                let _ = reply.send(Ok(()));
            }
            ServerEvent::PanelRollback { which, reply } => {
                let _ = reply.send(self.roll_back(which));
            }
            ServerEvent::PanelAccounts { reply } => {
                let _ = reply.send(self.panel_accounts());
            }
            ServerEvent::PanelSetAccountGroup { name, group, reply } => {
                let _ = reply.send(self.panel_set_account_group(&name, &group));
            }
            ServerEvent::PanelCreateAccount { account, reply } => {
                // The group has to exist — an account pointed at a group that is not there silently
                // falls back to `default`, which would be a confusing thing to have just created.
                if !self.admin.groups.iter().any(|g| g.name == account.group) {
                    let _ = reply.send(Err(format!("there is no group called {}", account.group)));
                    return;
                }
                let result = self.admin.insert_account(account);
                if result.is_ok() {
                    let _ = self.admin.save();
                }
                let _ = reply.send(result);
            }
            ServerEvent::PanelDeleteAccount { name, reply } => {
                let _ = reply.send(self.panel_delete_account(&name));
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
