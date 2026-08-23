//! Quick stack: emptying an armful of loot into the chests it already belongs in.
//!
//! One of the most-used buttons in the game, and one of the few inventory operations that is the
//! *server's* to carry out rather than the client's. A client cannot do it itself because it does
//! not know what is in chests it has not opened, and two players pressing it at once must not both
//! be told a slot was free.
//!
//! The rule is narrower than it looks, and that narrowness is the point:
//!
//! > An item moves only into a chest that **already holds that type**.
//!
//! Nothing goes into an empty chest, and nothing goes into a chest that has no example of it.
//! That is what makes the button safe to press without looking — it can tidy, but it cannot
//! scatter your things into places you would not have put them.
//!
//! Two chests are refused outright: one somebody else has open, and one that is locked. A refusal
//! is reported rather than silently skipped, because the client marks those chests so the player
//! can see why their inventory did not empty.

use terrustia_proto::ItemStack;

/// How far a chest may be and still count as nearby, in pixels. `NearbyChests`' default.
pub const RANGE: f32 = 600.0;

/// A chest a quick stack may reach.
#[derive(Debug, Clone)]
pub struct Destination {
    pub id: i16,
    /// The chest's centre, which is what the range is measured to.
    pub position: (f32, f32),
    pub items: Vec<ItemStack>,
    /// Whether it is locked or somebody else has it open.
    pub blocked: bool,
}

/// One item that moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Move {
    /// The player's inventory slot it came from.
    pub from_slot: u16,
    pub chest: i16,
    pub chest_slot: usize,
    /// What the player's slot holds afterwards — often not empty, since a stack can part-fit.
    pub left_behind: ItemStack,
    /// ...and what the chest slot holds afterwards.
    pub chest_now: ItemStack,
}

/// What a quick stack did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub moves: Vec<Move>,
    /// Chests that would have taken something but were locked or in use.
    pub blocked: Vec<i16>,
}

/// How many of an item may sit in one slot.
///
/// The game reads this per item from `Item.maxStack`. This server has no item table yet, so it
/// uses the format's own ceiling: a stack is a signed short on the wire, and nothing in the game
/// stacks higher than 9,999. Being generous here is safe — the worst case is that a quick stack
/// merges two stacks the game would have kept apart, which loses nothing.
const MAX_STACK: i16 = 9_999;

/// Run a quick stack from a player's offered slots into the chests around them.
///
/// `offered` is what the client says it is willing to move, by slot and contents. The server does
/// not take that on trust for *what* moves — every destination is its own — but it does take the
/// client's word for which of its own slots are eligible, because favourited and coin slots are
/// the client's own bookkeeping and it is the only one that knows them.
pub fn run(
    from: (f32, f32),
    offered: &[(u16, ItemStack)],
    chests: &mut [Destination],
) -> Outcome {
    let mut out = Outcome::default();

    // Nearest first, so a player standing between two chests fills the one they are looking at.
    let mut order: Vec<usize> = (0..chests.len())
        .filter(|&i| {
            let (dx, dy) = (chests[i].position.0 - from.0, chests[i].position.1 - from.1);
            dx.hypot(dy) <= RANGE
        })
        .collect();
    order.sort_by(|&a, &b| {
        let d = |i: usize| {
            let (dx, dy) = (chests[i].position.0 - from.0, chests[i].position.1 - from.1);
            dx.hypot(dy)
        };
        d(a).total_cmp(&d(b))
    });

    for &(slot, item) in offered {
        let mut carrying = item;
        if carrying.is_empty() {
            continue;
        }
        for &at in &order {
            if carrying.is_empty() {
                break;
            }
            // A chest with no example of this item is not a destination for it, however much room
            // it has. This is the whole rule.
            if !chests[at].items.iter().any(|held| held.id == carrying.id) {
                continue;
            }
            if chests[at].blocked {
                let id = chests[at].id;
                if !out.blocked.contains(&id) {
                    out.blocked.push(id);
                }
                continue;
            }

            // Top up the partial stacks first, then take a free slot.
            let mut targets: Vec<usize> = (0..chests[at].items.len())
                .filter(|&i| {
                    let held = chests[at].items[i];
                    held.id == carrying.id && held.stack < MAX_STACK
                })
                .collect();
            targets.extend(
                (0..chests[at].items.len()).filter(|&i| chests[at].items[i].is_empty()),
            );

            for target in targets {
                if carrying.is_empty() {
                    break;
                }
                let held = chests[at].items[target];
                let room = if held.is_empty() {
                    MAX_STACK
                } else {
                    MAX_STACK - held.stack
                };
                if room <= 0 {
                    continue;
                }
                let moved = carrying.stack.min(room);
                let now = ItemStack {
                    id: carrying.id,
                    stack: if held.is_empty() {
                        moved
                    } else {
                        held.stack + moved
                    },
                    prefix: if held.is_empty() {
                        carrying.prefix
                    } else {
                        held.prefix
                    },
                };
                chests[at].items[target] = now;
                carrying.stack -= moved;
                if carrying.stack <= 0 {
                    carrying = ItemStack::EMPTY;
                }
                out.moves.push(Move {
                    from_slot: slot,
                    chest: chests[at].id,
                    chest_slot: target,
                    left_behind: carrying,
                    chest_now: now,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chest(id: i16, at: (f32, f32), items: &[ItemStack]) -> Destination {
        Destination {
            id,
            position: at,
            items: items.to_vec(),
            blocked: false,
        }
    }

    const WOOD: i32 = 9;
    const STONE: i32 = 3;

    /// The ordinary case: wood goes into the chest that already has wood.
    #[test]
    fn an_item_joins_its_own_kind() {
        let mut chests = vec![chest(
            0,
            (100.0, 100.0),
            &[ItemStack::new(WOOD, 50, 0), ItemStack::EMPTY],
        )];
        let out = run(
            (100.0, 100.0),
            &[(10, ItemStack::new(WOOD, 30, 0))],
            &mut chests,
        );
        assert_eq!(out.moves.len(), 1);
        assert_eq!(chests[0].items[0].stack, 80);
        assert!(out.moves[0].left_behind.is_empty());
    }

    /// Nothing goes into a chest that has no example of it, however much room it has.
    ///
    /// This is what makes the button safe to press without looking.
    #[test]
    fn an_empty_chest_is_not_a_destination() {
        let mut chests = vec![chest(
            0,
            (100.0, 100.0),
            &[ItemStack::EMPTY, ItemStack::EMPTY],
        )];
        let out = run(
            (100.0, 100.0),
            &[(10, ItemStack::new(WOOD, 30, 0))],
            &mut chests,
        );
        assert!(out.moves.is_empty(), "an empty chest takes nothing");
        assert!(chests[0].items.iter().all(|i| i.is_empty()));
    }

    /// ...and neither is a chest full of something else.
    #[test]
    fn a_chest_of_something_else_is_not_a_destination() {
        let mut chests = vec![chest(
            0,
            (100.0, 100.0),
            &[ItemStack::new(STONE, 50, 0), ItemStack::EMPTY],
        )];
        let out = run(
            (100.0, 100.0),
            &[(10, ItemStack::new(WOOD, 30, 0))],
            &mut chests,
        );
        assert!(out.moves.is_empty());
    }

    /// A chest out of reach is not reached.
    #[test]
    fn a_distant_chest_is_out_of_reach() {
        let mut chests = vec![chest(
            0,
            (100.0 + RANGE + 10.0, 100.0),
            &[ItemStack::new(WOOD, 1, 0), ItemStack::EMPTY],
        )];
        let out = run(
            (100.0, 100.0),
            &[(10, ItemStack::new(WOOD, 30, 0))],
            &mut chests,
        );
        assert!(out.moves.is_empty());
    }

    /// A locked chest is reported rather than silently skipped, so the player can see why.
    #[test]
    fn a_locked_chest_is_reported() {
        let mut chests = vec![Destination {
            blocked: true,
            ..chest(7, (100.0, 100.0), &[ItemStack::new(WOOD, 1, 0)])
        }];
        let out = run(
            (100.0, 100.0),
            &[(10, ItemStack::new(WOOD, 30, 0))],
            &mut chests,
        );
        assert!(out.moves.is_empty());
        assert_eq!(out.blocked, vec![7], "the player should be told which");
    }

    /// A stack that does not fit is split, and the remainder stays with the player.
    #[test]
    fn an_overflowing_stack_is_split() {
        let mut chests = vec![chest(
            0,
            (100.0, 100.0),
            &[ItemStack::new(WOOD, MAX_STACK - 10, 0)],
        )];
        let out = run(
            (100.0, 100.0),
            &[(10, ItemStack::new(WOOD, 100, 0))],
            &mut chests,
        );
        assert_eq!(chests[0].items[0].stack, MAX_STACK);
        assert_eq!(
            out.moves.last().unwrap().left_behind.stack,
            90,
            "ninety should stay with the player"
        );
    }

    /// Given two chests that both qualify, the nearer one fills first.
    #[test]
    fn the_nearer_chest_fills_first() {
        let mut chests = vec![
            chest(
                1,
                (500.0, 100.0),
                &[ItemStack::new(WOOD, 1, 0), ItemStack::EMPTY],
            ),
            chest(
                2,
                (150.0, 100.0),
                &[ItemStack::new(WOOD, 1, 0), ItemStack::EMPTY],
            ),
        ];
        let out = run(
            (100.0, 100.0),
            &[(10, ItemStack::new(WOOD, 5, 0))],
            &mut chests,
        );
        assert_eq!(out.moves[0].chest, 2, "the near one, not the far one");
    }

    /// Partial stacks are topped up before a free slot is taken, so a chest does not end up with
    /// two half-stacks of the same thing.
    #[test]
    fn partial_stacks_are_topped_up_first() {
        let mut chests = vec![chest(
            0,
            (100.0, 100.0),
            &[
                ItemStack::EMPTY,
                ItemStack::new(WOOD, 10, 0),
                ItemStack::EMPTY,
            ],
        )];
        let out = run(
            (100.0, 100.0),
            &[(10, ItemStack::new(WOOD, 5, 0))],
            &mut chests,
        );
        assert_eq!(out.moves.len(), 1);
        assert_eq!(out.moves[0].chest_slot, 1, "the partial stack, not slot 0");
        assert_eq!(chests[0].items[1].stack, 15);
        assert!(chests[0].items[0].is_empty());
    }

    /// Nothing offered, nothing done.
    #[test]
    fn an_empty_offer_does_nothing() {
        let mut chests = vec![chest(0, (100.0, 100.0), &[ItemStack::new(WOOD, 1, 0)])];
        let out = run((100.0, 100.0), &[], &mut chests);
        assert_eq!(out, Outcome::default());
    }
}
