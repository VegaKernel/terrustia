//! Item entities lying in the world.
//!
//! Terraria addresses these by slot index, and the index travels on the wire, so a removed item
//! leaves a hole rather than shifting its neighbours.

use terrustia_proto::{
    ItemStack,
    items::{MAX_ITEMS, PICKUP_REPLACEMENT_TIME, SLOTS_RESERVED_BEFORE_RECYCLING},
};

// A Terraria world item never expires with age, and neither does one here any more.
//
// This server used to clean an item up after ten minutes. That was its own invention, not the
// game's: `WorldItem.UpdateItem`'s only self-destructs are per-type (a Fallen Star at sunrise, a
// Mana Cloak star after 300 ticks, a Defender Medal outside the Old One's Army) plus lava and
// falling out of the world (`WorldItem.cs:646-714`). What actually keeps vanilla's 400-slot table
// from filling is [`ItemStore::pick_slot`]'s recycling, and that is now transcribed, so the timer
// was standing in for a mechanism this server has.
//
// It was doing real harm in the meantime: drop something valuable, come back a quarter of an hour
// later, and the real game still has it while this server had thrown it away. It also made this
// server send a `151` for a reason no real one ever would.
//
// The obvious worry, an idle server hoarding dropped dirt forever, is bounded exactly as vanilla
// bounds it: the table holds 400 items, and the 401st recycles the oldest.

/// How long a reservation lasts before the item is offered to somebody else.
pub const RESERVATION_TICKS: u32 = 100;

/// Downward acceleration applied to a falling item, in pixels per tick squared.
pub const GRAVITY: f32 = 0.2;

/// Terminal speed, so an item that spawns over a chasm does not tunnel through the floor.
pub const MAX_FALL_SPEED: f32 = 7.0;

/// Items are one tile across for the purpose of resting on ground.
pub const ITEM_SIZE: f32 = 16.0;

/// The owner value meaning "reserved for nobody".
pub const NO_OWNER: u8 = terrustia_proto::items::NO_OWNER;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldItem {
    pub item: ItemStack,
    pub position: (f32, f32),
    pub velocity: (f32, f32),
    pub owner: u8,
    /// Ticks left on the current reservation.
    pub reservation: u32,
    /// Whether this item is instanced to exactly one player and only they may ever take it
    /// (`WorldItem.instanced`/`playerIndexTheItemIsReservedFor`, `WorldItem.cs:36,18`; set through
    /// `WorldItem.MakeInstanced`, `WorldItem.cs:326`).
    ///
    /// An expert or master treasure bag is dropped one per interacting player, each announced only
    /// to its owner over packet `90`. Vanilla's server sends the packet to each qualifying player and
    /// then turns its own copy to air, holding only the item slot, so the bag lives solely on each
    /// client; this server keeps the bag as a real server-side item instead and ties it permanently
    /// to its one owner. Set here, `owner` is a permanent claim rather than the ordinary proximity
    /// reservation: the item stays reserved forever (so the proximity offer loop never hands it to
    /// whoever is nearest, mirroring `WorldItem.FindOwner`'s own `if (instanced ...) return`,
    /// `WorldItem.cs:195`), the owner is never cleared when the reservation lapses, and the pickup
    /// gate still admits only `owner`, so no other client - modified or not - can race a bag meant
    /// for someone else.
    pub instanced: bool,
    /// Ticks this item has existed for.
    pub age: u32,
    /// How far through shimmering this item is, from zero to one.
    ///
    /// Shimmer does not transmute on contact — an item sinks into it over about a second and a
    /// half, and the transformation happens at nine tenths. That delay is the whole feel of the
    /// mechanic: you can pull something back out if you change your mind.
    pub shimmer_time: f32,
    /// Whether it has already been transmuted, so it does not go round again.
    ///
    /// The result of a shimmering usually has a transform of its own — the pairs are symmetric,
    /// so wood becomes stone and stone becomes wood — and without this flag an item dropped in
    /// would flicker between the two forever.
    pub shimmered: bool,
    /// Whether the item has settled on the ground and no longer needs simulating.
    pub resting: bool,
}

impl WorldItem {
    pub fn new(item: ItemStack, position: (f32, f32)) -> Self {
        Self {
            item,
            position,
            velocity: (0.0, 0.0),
            owner: NO_OWNER,
            reservation: 0,
            instanced: false,
            age: 0,
            resting: false,
            shimmer_time: 0.0,
            shimmered: false,
        }
    }

    pub fn is_reserved(&self) -> bool {
        // An instanced item is claimed for its one owner forever, not on a lapsing timer.
        self.instanced || (self.owner != NO_OWNER && self.reservation > 0)
    }
}

/// How fast an item sinks into shimmer, per tick. `WorldItem.UpdateShimmer`.
pub const SHIMMER_RATE: f32 = 0.01;
/// ...and how far in it has to be before it transmutes, which is not all the way.
pub const SHIMMER_AT: f32 = 0.9;

/// What one tick of sitting in shimmer does to an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shimmering {
    /// Nothing yet — still sinking, or backing out of it.
    Waiting,
    /// It has gone far enough in, and should be transmuted now.
    Transmute,
}

/// Sink an item further into shimmer, or let it climb back out.
///
/// `in_shimmer` is whether the tile above the item's position holds shimmer, which is the game's
/// own test — the item is *under* the surface it is sinking through, not standing in it.
///
/// An item already transmuted never goes again. Most transform pairs are symmetric, so without
/// that a shimmered item would flicker between its two forms forever.
pub fn shimmer(item: &mut WorldItem, in_shimmer: bool) -> Shimmering {
    if item.shimmered {
        return Shimmering::Waiting;
    }
    if !in_shimmer {
        item.shimmer_time = (item.shimmer_time - SHIMMER_RATE).max(0.0);
        return Shimmering::Waiting;
    }
    item.shimmer_time = (item.shimmer_time + SHIMMER_RATE).min(1.0);
    if item.shimmer_time >= SHIMMER_AT {
        // Held just short of the top, as the game holds it, so the item does not vanish before
        // the transformation is shown.
        item.shimmer_time = SHIMMER_AT;
        return Shimmering::Transmute;
    }
    Shimmering::Waiting
}

#[cfg(test)]
mod shimmer_tests {
    use super::*;

    fn dropped() -> WorldItem {
        WorldItem::new(ItemStack::new(9, 1, 0), (100.0, 100.0))
    }

    /// It takes about a second and a half to sink far enough in, not one tick.
    ///
    /// The delay is the whole feel of the mechanic: you can pull something back out.
    #[test]
    fn an_item_sinks_before_it_transmutes() {
        let mut item = dropped();
        let mut ticks = 0;
        loop {
            ticks += 1;
            if shimmer(&mut item, true) == Shimmering::Transmute {
                break;
            }
            assert!(ticks < 1000, "it should transmute eventually");
        }
        // About ninety ticks — a second and a half. Not exactly ninety: adding 0.01 to a `f32`
        // ninety times lands a hair under 0.9, so it takes one more. The game accumulates the
        // same way and arrives at the same place, so this is faithful rather than sloppy.
        assert!(
            (90..=92).contains(&ticks),
            "should take about a second and a half, took {ticks} ticks"
        );
    }

    /// Taken out part-way, it climbs back out rather than staying half-sunk.
    #[test]
    fn an_item_taken_out_climbs_back() {
        let mut item = dropped();
        for _ in 0..40 {
            shimmer(&mut item, true);
        }
        assert!(item.shimmer_time > 0.0);
        for _ in 0..100 {
            shimmer(&mut item, false);
        }
        assert_eq!(item.shimmer_time, 0.0, "it should be fully back out");
        assert!(!item.shimmered);
    }

    /// Something already transmuted never goes again.
    ///
    /// Most transform pairs are symmetric — wood becomes stone and stone becomes wood — so
    /// without this an item dropped in would flicker between its two forms forever.
    #[test]
    fn a_transmuted_item_does_not_go_round_again() {
        let mut item = dropped();
        while shimmer(&mut item, true) != Shimmering::Transmute {}
        item.shimmered = true;
        for _ in 0..1000 {
            assert_eq!(
                shimmer(&mut item, true),
                Shimmering::Waiting,
                "a shimmered item should never transmute a second time"
            );
        }
    }

    /// The threshold is held just short of the top, so the transformation is visible.
    #[test]
    fn it_stops_short_of_the_surface() {
        let mut item = dropped();
        while shimmer(&mut item, true) != Shimmering::Transmute {}
        assert_eq!(item.shimmer_time, SHIMMER_AT);
        assert!(item.shimmer_time < 1.0);
    }
}

/// The fixed-size table of item entities.
#[derive(Debug)]
pub struct ItemStore {
    slots: Vec<Option<WorldItem>>,
}

impl Default for ItemStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ItemStore {
    pub fn new() -> Self {
        Self {
            slots: vec![None; MAX_ITEMS],
        }
    }

    /// Place an item in the slot [`ItemStore::pick_slot`] chooses.
    ///
    /// Returns that slot and whether taking it destroyed a live item. A caller that talks to
    /// clients owes them a `151` for the destroyed one *before* it announces the new one, because
    /// the wire addresses both by the same index and a client that only ever heard the `21` would
    /// quietly swap the old item for the new one on its own screen without the pickup ever
    /// happening. That is `Item.NewItem`'s own order (`Item.cs:49725-49730`).
    pub fn spawn(&mut self, item: ItemStack, position: (f32, f32)) -> Option<(i16, bool)> {
        let index = self.pick_slot()?;
        let recycled = self.slots[index].is_some();
        self.slots[index] = Some(WorldItem::new(item, position));
        Some((i16::try_from(index).ok()?, recycled))
    }

    /// Which slot a new item should go in, destroying whatever is in it if it comes to that.
    ///
    /// `Item.PickAnItemSlotToSpawnItemOn` (`Item.cs:49779-49845`), transcribed with its branch
    /// order intact. Terraria's world items never expire on their own, so the 400-slot table is
    /// kept clear by *recycling*: a busy server throws away the least valuable item it can find
    /// rather than refusing to drop the new one. Three tiers, in the game's own order:
    ///
    /// 1. The first free slot below the reserve line, if there is one.
    /// 2. Otherwise the oldest "pickup" (a heart or a mana star) that has been lying around for
    ///    more than [`PICKUP_REPLACEMENT_TIME`] ticks. Note this only considers slots *before* the
    ///    first free one, because vanilla's scan breaks there, and it is preferred even when a free
    ///    slot exists, as long as that free slot is at or past the reserve line.
    /// 3. Otherwise, with the table completely full, the oldest item of any kind.
    ///
    /// Two disclosed narrowings, both of which only ever make this pick a *different* slot, never a
    /// wrong one:
    ///
    /// - Between tiers 2 and 3 vanilla tries `EmergencyStacking.EmergencyStackItemsToMakeSpace`
    ///   (`Item.cs:49807-49810`), which walks the whole table looking for pairs of partial stacks
    ///   of the same type close enough together to merge, ranked by an on-screen/age/distance
    ///   ordering, and frees the slot it emptied. That is a 450-line subsystem of its own
    ///   (`Terraria.GameContent/EmergencyStacking.cs`) with a pending-transfer queue that
    ///   `Item.NewItem` also has to clear (`Item.cs:49731`). It is not built here, so this behaves
    ///   exactly as vanilla does when that call returns false: it falls through to tier 3.
    /// - Vanilla also skips any slot whose `Main.timeItemSlotCannotBeReusedFor` is still counting
    ///   down. That timer is set in exactly one place, `WorldItem.MakeInstanced`
    ///   (`WorldItem.cs:326-341`), where the server hands each player their own private copy of a
    ///   treasure bag over packet `90` and then turns its *own* copy to air, holding the slot empty
    ///   for 54000 ticks so nothing else claims an index the clients still have an item in. This
    ///   server does not do that: `drop_instanced_bag` keeps the bag as a real occupied slot owned
    ///   by its one player (see its own doc comment). There is therefore no such thing here as a
    ///   slot that looks free but is not, the timer would be zero everywhere, and every branch that
    ///   reads it degenerates. That includes vanilla's fourth loop (`Item.cs:49830-49838`), which
    ///   is tier 3's comparison with the timer subtracted from both sides: with the timer always
    ///   zero it is tier 3 exactly, so it is left out rather than transcribed as a dead duplicate.
    fn pick_slot(&self) -> Option<usize> {
        // Vanilla's `num`: the first free slot, or 400 for "there is not one".
        let mut free = MAX_ITEMS;
        // ...its `num2`/`num3`: the oldest pickup worth throwing away, and how old that is.
        let mut recyclable = None;
        let mut oldest_pickup = PICKUP_REPLACEMENT_TIME;
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(item) = slot else {
                free = index;
                break;
            };
            if terrustia_proto::items::is_a_pickup(item.item.id) && item.age > oldest_pickup {
                oldest_pickup = item.age;
                recyclable = Some(index);
            }
        }

        // A server keeps the tail of the table in hand, so the last slots stay available for
        // whatever cannot be recycled. Reaching into it, or finding no free slot at all, is what
        // makes recycling preferable to allocating.
        if free >= MAX_ITEMS - SLOTS_RESERVED_BEFORE_RECYCLING
            && let Some(index) = recyclable
        {
            return Some(index);
        }
        if free != MAX_ITEMS {
            return Some(free);
        }

        // Nothing free and no stale pickup: destroy the oldest item in the world. `> oldest`
        // rather than `>=` keeps vanilla's own first-index-wins tie-break.
        let mut oldest = 0;
        let mut pick = None;
        for (index, slot) in self.slots.iter().enumerate() {
            let age = slot.map_or(0, |item| item.age);
            if age > oldest {
                oldest = age;
                pick = Some(index);
            }
        }
        // Vanilla returns slot 400 here, the scratch entry `Main.item` carries past the 400 real
        // ones, which is its way of throwing the drop away: nothing is broadcast for it and the
        // next spawn overwrites it. Only reachable when all 400 slots hold an item that has not
        // been ticked even once, so it is this server's `None` and the same discarded drop.
        pick
    }

    pub fn get(&self, index: i16) -> Option<&WorldItem> {
        self.slots.get(usize::try_from(index).ok()?)?.as_ref()
    }

    pub fn get_mut(&mut self, index: i16) -> Option<&mut WorldItem> {
        self.slots.get_mut(usize::try_from(index).ok()?)?.as_mut()
    }

    pub fn remove(&mut self, index: i16) -> Option<WorldItem> {
        self.slots.get_mut(usize::try_from(index).ok()?)?.take()
    }

    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every live item with its index.
    pub fn iter(&self) -> impl Iterator<Item = (i16, &WorldItem)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_ref().map(|item| (i as i16, item)))
    }

    /// ...and the same, to change them in place.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (i16, &mut WorldItem)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_mut().map(|item| (i as i16, item)))
    }

    /// Age every item by one tick, and lapse any reservation that has run out.
    ///
    /// Nothing is removed here. `age` is vanilla's `timeSinceItemSpawned` (`WorldItem.cs:442-445`,
    /// incremented once per update while the item is active) and its only reader is the slot
    /// picker, which uses it to decide what to recycle when the table is full. See the note at the
    /// top of this file for why the ten-minute cleanup that used to live here is gone.
    pub fn tick(&mut self) {
        for slot in self.slots.iter_mut() {
            let Some(item) = slot else { continue };
            item.age = item.age.saturating_add(1);
            item.reservation = item.reservation.saturating_sub(1);
            // An instanced item stays claimed for its one owner; only an ordinary proximity
            // reservation lapses back to nobody.
            if item.reservation == 0 && !item.instanced {
                item.owner = NO_OWNER;
            }
        }
    }
}

/// Advance one item's fall by a tick, given a test for whether a tile blocks it.
///
/// Item physics run on the server in Terraria, so a dropped block would otherwise hang in the air
/// exactly where it was mined. This is a deliberately small approximation: gravity, a terminal
/// speed, and resting on the first blocking tile below.
pub fn fall(item: &mut WorldItem, blocked: impl Fn(i32, i32) -> bool) {
    if item.resting {
        return;
    }
    item.velocity.1 = (item.velocity.1 + GRAVITY).min(MAX_FALL_SPEED);
    let next_y = item.position.1 + item.velocity.1;

    // The tile the item's feet would end up in.
    let foot_tile_y = ((next_y + ITEM_SIZE) / 16.0).floor() as i32;
    let tile_x = ((item.position.0 + ITEM_SIZE / 2.0) / 16.0).floor() as i32;

    if blocked(tile_x, foot_tile_y) {
        // Sit exactly on top of the blocking tile rather than inside it.
        item.position.1 = (foot_tile_y as f32) * 16.0 - ITEM_SIZE;
        item.velocity.1 = 0.0;
        item.resting = true;
    } else {
        item.position.1 = next_y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack() -> ItemStack {
        ItemStack::new(3, 1, 0)
    }

    /// A heart, one of the ten types in `ItemID.Sets.IsAPickup`.
    fn a_pickup() -> ItemStack {
        ItemStack::new(58, 1, 0)
    }

    /// The lowest slot a server holds in reserve: 400 - 40 = 360.
    const FIRST_RESERVED_SLOT: i16 = (MAX_ITEMS - SLOTS_RESERVED_BEFORE_RECYCLING) as i16;

    /// Fill every slot, and age each item so the picker has something to order them by.
    fn full_store(item: ItemStack, age: u32) -> ItemStore {
        let mut store = ItemStore::new();
        for _ in 0..MAX_ITEMS {
            let (index, _) = store.spawn(item, (0.0, 0.0)).expect("a slot");
            store.get_mut(index).expect("the item").age = age;
        }
        store
    }

    #[test]
    fn spawning_uses_the_lowest_free_slot() {
        let mut store = ItemStore::new();
        assert_eq!(store.spawn(stack(), (0.0, 0.0)), Some((0, false)));
        assert_eq!(store.spawn(stack(), (0.0, 0.0)), Some((1, false)));
        store.remove(0);
        assert_eq!(
            store.spawn(stack(), (0.0, 0.0)),
            Some((0, false)),
            "the hole is reused"
        );
    }

    #[test]
    fn removing_leaves_a_hole_rather_than_renumbering() {
        let mut store = ItemStore::new();
        store.spawn(stack(), (0.0, 0.0));
        let (second, _) = store.spawn(ItemStack::new(9, 5, 0), (1.0, 1.0)).unwrap();
        store.remove(0);
        assert_eq!(
            store.get(second).unwrap().item.id,
            9,
            "index 1 still holds its item"
        );
    }

    /// The gap this whole picker exists to close: a Terraria world item never expires, so a real
    /// server keeps its 400 slots clear by destroying the least valuable thing in them. Refusing
    /// the drop instead is how loot silently vanished on a busy world.
    #[test]
    fn a_full_store_recycles_rather_than_dropping_the_loot() {
        let mut store = full_store(stack(), 5_000);

        let (index, recycled) = store
            .spawn(ItemStack::new(9, 1, 0), (0.0, 0.0))
            .expect("a full store should still find a slot by recycling one");

        assert!(recycled, "the slot it picked was holding a live item");
        assert_eq!(store.len(), MAX_ITEMS, "still exactly full, not overfull");
        assert_eq!(
            store.get(index).unwrap().item.id,
            9,
            "the new item is the one in that slot now"
        );
    }

    /// With nothing free and no stale pickup, vanilla's third tier takes the oldest item of any
    /// kind (`Item.cs:49818-49827`).
    #[test]
    fn a_full_store_recycles_the_oldest_item() {
        let mut store = full_store(stack(), 5_000);
        store.get_mut(37).unwrap().age = 9_000;

        let (index, _) = store.spawn(ItemStack::new(9, 1, 0), (0.0, 0.0)).unwrap();

        assert_eq!(index, 37);
    }

    /// Tier 2: once the free slots have run down into the reserve, a stale heart is thrown away in
    /// preference to claiming one of them (`Item.cs:49790-49806`). The heart here sits below the
    /// first free slot, because vanilla's scan stops looking the moment it finds one.
    #[test]
    fn a_stale_pickup_is_recycled_before_the_reserve_is_eaten_into() {
        let mut store = full_store(stack(), 0);
        for index in MAX_ITEMS - SLOTS_RESERVED_BEFORE_RECYCLING..MAX_ITEMS {
            store.remove(index as i16);
        }
        store.get_mut(11).unwrap().item = a_pickup();
        store.get_mut(11).unwrap().age = PICKUP_REPLACEMENT_TIME + 1;

        let (index, recycled) = store.spawn(ItemStack::new(9, 1, 0), (0.0, 0.0)).unwrap();

        assert_eq!(index, 11, "the stale heart's slot, not the free one at 360");
        assert!(recycled);
    }

    /// ...but only once the table is that far gone. With room to spare the heart is left alone.
    #[test]
    fn a_stale_pickup_survives_while_there_are_slots_to_spare() {
        let mut store = ItemStore::new();
        let (index, _) = store.spawn(a_pickup(), (0.0, 0.0)).unwrap();
        store.get_mut(index).unwrap().age = PICKUP_REPLACEMENT_TIME + 1;

        assert_eq!(store.spawn(stack(), (0.0, 0.0)), Some((1, false)));
    }

    /// A young heart is not stale enough to be worth destroying, so the picker falls through to
    /// the reserve rather than taking it.
    #[test]
    fn a_fresh_pickup_is_not_recycled() {
        let mut store = full_store(a_pickup(), PICKUP_REPLACEMENT_TIME);
        for index in MAX_ITEMS - SLOTS_RESERVED_BEFORE_RECYCLING..MAX_ITEMS {
            store.remove(index as i16);
        }

        let (index, recycled) = store.spawn(ItemStack::new(9, 1, 0), (0.0, 0.0)).unwrap();

        assert_eq!(index, FIRST_RESERVED_SLOT);
        assert!(!recycled);
    }

    /// Only an ordinary item is protected from the pickup tier: a table full of stale hearts is
    /// exactly what it is meant to clear out.
    #[test]
    fn only_pickups_are_recycled_early() {
        let mut store = full_store(stack(), PICKUP_REPLACEMENT_TIME * 10);
        for index in MAX_ITEMS - SLOTS_RESERVED_BEFORE_RECYCLING..MAX_ITEMS {
            store.remove(index as i16);
        }

        let (index, recycled) = store.spawn(ItemStack::new(9, 1, 0), (0.0, 0.0)).unwrap();

        assert_eq!(
            index, FIRST_RESERVED_SLOT,
            "ancient swords are not pickups; the reserve is what gets used"
        );
        assert!(!recycled);
    }

    /// Vanilla's own last resort is slot 400, the scratch entry past the 400 real ones, which
    /// throws the drop away. Only reachable when every slot holds something that has not been
    /// ticked once, which is this server's `None` and the same discarded drop.
    #[test]
    fn a_store_of_brand_new_items_has_nothing_left_to_recycle() {
        let mut store = full_store(stack(), 0);
        assert_eq!(store.spawn(ItemStack::new(9, 1, 0), (0.0, 0.0)), None);
    }

    /// What the slot picker costs, by how full the table is. Ignored by default, like the other
    /// `measure_` tests here: run it with `--ignored --nocapture --release`.
    ///
    /// It is the same O(400) scan vanilla runs, and it is on the drop path, so it is worth being
    /// able to re-measure. Against the first-free scan it replaces, on an M-series laptop: 1.2 to
    /// 2.7 ns empty, 58 to 180 ns half full, 107 to 469 ns with all 400 slots taken. The worst case
    /// is two full passes rather than one, and a boss dropping ten items pays about 3.6µs of it
    /// against a 16.67ms tick.
    #[test]
    #[ignore]
    fn measure_the_picker() {
        for (name, store) in [
            ("empty", ItemStore::new()),
            ("half full", {
                let mut s = full_store(stack(), 5_000);
                for i in MAX_ITEMS / 2..MAX_ITEMS {
                    s.remove(i as i16);
                }
                s
            }),
            ("into the reserve", {
                let mut s = full_store(stack(), 5_000);
                for i in MAX_ITEMS - 10..MAX_ITEMS {
                    s.remove(i as i16);
                }
                s
            }),
            ("full", full_store(stack(), 5_000)),
        ] {
            let n = 1_000_000;
            let start = std::time::Instant::now();
            let mut sink = 0usize;
            for _ in 0..n {
                sink += store.pick_slot().unwrap_or(0);
            }
            let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
            println!("{name}: {each:.1} ns/pick (sink {sink})");
        }
    }

    #[test]
    fn reservations_lapse_so_an_item_is_not_locked_forever() {
        let mut store = ItemStore::new();
        let (index, _) = store.spawn(stack(), (0.0, 0.0)).unwrap();
        {
            let item = store.get_mut(index).unwrap();
            item.owner = 3;
            item.reservation = 2;
            assert!(item.is_reserved());
        }
        store.tick();
        assert!(store.get(index).unwrap().is_reserved());
        store.tick();
        let item = store.get(index).unwrap();
        assert!(!item.is_reserved());
        assert_eq!(
            item.owner, NO_OWNER,
            "a lapsed reservation clears the owner"
        );
    }

    #[test]
    fn an_item_falls_until_it_lands_on_a_tile() {
        // Ground at tile y = 10, so an item dropped above it should come to rest at y = 144.
        let ground = |_x: i32, y: i32| y >= 10;
        let mut item = WorldItem::new(stack(), (32.0, 0.0));

        for _ in 0..200 {
            fall(&mut item, ground);
            if item.resting {
                break;
            }
        }
        assert!(item.resting, "the item never landed");
        assert_eq!(item.position.1, 10.0 * 16.0 - ITEM_SIZE);
        assert_eq!(item.velocity.1, 0.0);
    }

    #[test]
    fn a_resting_item_stays_put() {
        let mut item = WorldItem::new(stack(), (32.0, 100.0));
        item.resting = true;
        fall(&mut item, |_, _| false);
        assert_eq!(item.position.1, 100.0);
    }

    #[test]
    fn falling_is_capped_so_an_item_cannot_tunnel_through_the_floor() {
        let mut item = WorldItem::new(stack(), (32.0, 0.0));
        for _ in 0..500 {
            fall(&mut item, |_, _| false);
        }
        assert_eq!(item.velocity.1, MAX_FALL_SPEED);
    }

    /// A dropped item waits for whoever dropped it, however long that takes. This server used to
    /// delete it after ten minutes, which the real game never does.
    #[test]
    fn an_item_never_expires_with_age() {
        let mut store = ItemStore::new();
        let (index, _) = store.spawn(stack(), (0.0, 0.0)).unwrap();
        // A whole day of game time, six times the old ten-minute cleanup.
        for _ in 0..(60 * 60 * 60) {
            store.tick();
        }
        assert!(
            store.get(index).is_some(),
            "an hour later it is still lying where it was dropped"
        );
    }

    /// What does age: the picker reads it to decide which slot to recycle when the table is full.
    #[test]
    fn ticking_ages_an_item_so_the_picker_can_rank_it() {
        let mut store = ItemStore::new();
        let (index, _) = store.spawn(stack(), (0.0, 0.0)).unwrap();
        for _ in 0..500 {
            store.tick();
        }
        assert_eq!(store.get(index).expect("still there").age, 500);
    }
}
