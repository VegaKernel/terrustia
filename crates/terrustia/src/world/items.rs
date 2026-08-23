//! Item entities lying in the world.
//!
//! Terraria addresses these by slot index, and the index travels on the wire, so a removed item
//! leaves a hole rather than shifting its neighbours.

use terrustia_proto::{ItemStack, items::MAX_ITEMS};

/// How long an unclaimed item survives before it is cleaned up, in ticks at 60 Hz.
pub const DESPAWN_TICKS: u32 = 60 * 60 * 10;

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
            age: 0,
            resting: false,
            shimmer_time: 0.0,
            shimmered: false,
        }
    }

    pub fn is_reserved(&self) -> bool {
        self.owner != NO_OWNER && self.reservation > 0
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

    /// Place an item in the lowest free slot, or return None when the world is full of them.
    pub fn spawn(&mut self, item: ItemStack, position: (f32, f32)) -> Option<i16> {
        let index = self.slots.iter().position(Option::is_none)?;
        self.slots[index] = Some(WorldItem::new(item, position));
        i16::try_from(index).ok()
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

    /// Age every item by one tick, returning the indices that have expired.
    pub fn tick(&mut self) -> Vec<i16> {
        let mut expired = Vec::new();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let Some(item) = slot else { continue };
            item.age = item.age.saturating_add(1);
            item.reservation = item.reservation.saturating_sub(1);
            if item.reservation == 0 {
                item.owner = NO_OWNER;
            }
            if item.age >= DESPAWN_TICKS {
                expired.push(index as i16);
            }
        }
        for index in &expired {
            self.slots[*index as usize] = None;
        }
        expired
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

    #[test]
    fn spawning_uses_the_lowest_free_slot() {
        let mut store = ItemStore::new();
        assert_eq!(store.spawn(stack(), (0.0, 0.0)), Some(0));
        assert_eq!(store.spawn(stack(), (0.0, 0.0)), Some(1));
        store.remove(0);
        assert_eq!(
            store.spawn(stack(), (0.0, 0.0)),
            Some(0),
            "the hole is reused"
        );
    }

    #[test]
    fn removing_leaves_a_hole_rather_than_renumbering() {
        let mut store = ItemStore::new();
        store.spawn(stack(), (0.0, 0.0));
        let second = store.spawn(ItemStack::new(9, 5, 0), (1.0, 1.0)).unwrap();
        store.remove(0);
        assert_eq!(
            store.get(second).unwrap().item.id,
            9,
            "index 1 still holds its item"
        );
    }

    #[test]
    fn a_full_store_refuses_more() {
        let mut store = ItemStore::new();
        for _ in 0..MAX_ITEMS {
            assert!(store.spawn(stack(), (0.0, 0.0)).is_some());
        }
        assert_eq!(store.spawn(stack(), (0.0, 0.0)), None);
        assert_eq!(store.len(), MAX_ITEMS);
    }

    #[test]
    fn reservations_lapse_so_an_item_is_not_locked_forever() {
        let mut store = ItemStore::new();
        let index = store.spawn(stack(), (0.0, 0.0)).unwrap();
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

    #[test]
    fn items_expire_and_are_reported_once() {
        let mut store = ItemStore::new();
        let index = store.spawn(stack(), (0.0, 0.0)).unwrap();
        store.get_mut(index).unwrap().age = DESPAWN_TICKS - 1;

        assert_eq!(store.tick(), vec![index]);
        assert!(store.get(index).is_none());
        assert!(
            store.tick().is_empty(),
            "an expired item is only reported once"
        );
    }
}
