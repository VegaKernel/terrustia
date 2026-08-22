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
        }
    }

    pub fn is_reserved(&self) -> bool {
        self.owner != NO_OWNER && self.reservation > 0
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
