//! Runtime-side Shimmer decisions that are more than a one-to-one item transform.
//!
//! `terrustia-proto::shimmer` owns the generated vanilla transform tables and
//! `terrustia-proto::recipes` owns the generated decrafting database.  This module is the small
//! gameplay layer between those data tables and `GameServer`: it decides how much of a dropped
//! stack may be decrafted and exactly how many ingredients come back.
//!
//! Keeping that arithmetic out of `server.rs` matters for two reasons.  First, batch recipes and
//! alchemy loss are rules in their own right and deserve deterministic tests.  Second, the server
//! still has to decide *where* to put the resulting item entities and how to broadcast them; that
//! transport/orchestration work should not be mixed with recipe arithmetic.

use rand::Rng;
use terrustia_proto::{ItemStack, recipes};

/// One ingredient produced by decrafting.
///
/// The count is wider than `ItemStack::stack` deliberately.  A large input stack can yield more
/// than one legal world-item stack, and splitting it belongs to the caller that owns item slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecraftIngredient {
    pub item: u16,
    pub count: u32,
}

/// The complete result of breaking the whole-number recipe batches out of one dropped stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecraftPlan {
    /// Number of source items consumed.  Always a multiple of the recipe's `makes` count.
    pub consumed: u32,
    /// Anything that did not make a whole recipe batch stays exactly as it was.
    pub remainder: ItemStack,
    /// Ingredients to create.  Entries reduced to zero by an Alchemy Table roll are omitted.
    pub ingredients: Vec<DecraftIngredient>,
}

/// Work out what Shimmer decrafting should do to a stack.
///
/// Returns `None` when the item has no decraftable vanilla recipe, the recipe is progression
/// locked, the stack is empty/invalid, or there are not enough items for one complete recipe
/// batch.  In particular, a recipe that makes three torches does not eat a stack of two and round
/// the ingredients down: those two torches remain two torches.
pub fn plan_decraft(
    item: ItemStack,
    crimson: bool,
    downed_skeletron: bool,
    downed_golem: bool,
    rng: &mut impl Rng,
) -> Option<DecraftPlan> {
    let item_type = u16::try_from(item.id).ok()?;
    let stack = u32::try_from(item.stack).ok()?;
    if stack == 0 {
        return None;
    }

    let recipe = recipes::decraft_recipe(item_type, crimson)?;
    if recipes::decraft_locked(recipe, downed_skeletron, downed_golem) {
        return None;
    }

    let makes = u32::from(recipe.makes);
    let batches = stack / makes;
    if batches == 0 {
        return None;
    }

    let consumed = batches.checked_mul(makes)?;
    let remainder_count = stack - consumed;
    let remainder = if remainder_count == 0 {
        ItemStack::EMPTY
    } else {
        ItemStack {
            id: item.id,
            stack: i16::try_from(remainder_count).ok()?,
            prefix: item.prefix,
        }
    };

    let mut ingredients = Vec::with_capacity(recipe.ingredients().len());
    for &(ingredient, per_batch) in recipe.ingredients() {
        let total = u32::from(per_batch).checked_mul(batches)?;
        let count = returned_units(total, recipe.alchemy, rng);
        if count != 0 {
            ingredients.push(DecraftIngredient {
                item: ingredient,
                count,
            });
        }
    }

    Some(DecraftPlan {
        consumed,
        remainder,
        ingredients,
    })
}

/// Alchemy recipes return each individual ingredient with a two-in-three chance.
///
/// This is deliberately per unit, not `floor(total * 2 / 3)`: vanilla rolls every unit
/// independently, so two otherwise identical potion stacks need not return exactly the same
/// ingredient count.
fn returned_units(total: u32, alchemy: bool, rng: &mut impl Rng) -> u32 {
    if !alchemy {
        return total;
    }
    (0..total)
        .filter(|_| rng.random_range(0..3) != 0)
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::SmallRng};

    #[test]
    fn a_partial_recipe_batch_stays_whole() {
        let mut rng = SmallRng::seed_from_u64(1);
        assert!(
            plan_decraft(ItemStack::new(8, 2, 0), false, false, false, &mut rng).is_none(),
            "a torch recipe makes three at a time, so two cannot be decrafted"
        );
    }

    #[test]
    fn whole_batches_decraft_and_the_remainder_stays() {
        let mut rng = SmallRng::seed_from_u64(2);
        let plan = plan_decraft(ItemStack::new(8, 7, 0), false, false, false, &mut rng)
            .expect("torches have a decraftable recipe");

        assert_eq!(plan.consumed, 6, "two complete batches of three");
        assert_eq!(plan.remainder, ItemStack::new(8, 1, 0));
        assert!(
            plan.ingredients
                .contains(&DecraftIngredient { item: 23, count: 2 }),
            "two batches return two Gel"
        );
        assert!(
            plan.ingredients
                .contains(&DecraftIngredient { item: 9, count: 2 }),
            "two batches return two Wood"
        );
    }

    #[test]
    fn a_raw_material_has_no_decraft_plan() {
        let mut rng = SmallRng::seed_from_u64(3);
        // Item zero is deliberately outside every generated recipe lookup and also exercises the
        // conversion/lookup path without relying on a guessed ordinary material id.
        assert!(
            plan_decraft(ItemStack::new(0, 1, 0), false, false, false, &mut rng).is_none()
        );
    }

    #[test]
    fn alchemy_loss_is_per_unit_and_reproducible() {
        let mut a = SmallRng::seed_from_u64(4);
        let mut b = SmallRng::seed_from_u64(4);
        let returned_a = returned_units(1_000, true, &mut a);
        let returned_b = returned_units(1_000, true, &mut b);

        assert_eq!(returned_a, returned_b, "a seeded gameplay RNG must replay exactly");
        assert!(returned_a > 0 && returned_a < 1_000, "alchemy must sometimes lose units");

        let mut ordinary = SmallRng::seed_from_u64(4);
        assert_eq!(returned_units(1_000, false, &mut ordinary), 1_000);
    }
}
