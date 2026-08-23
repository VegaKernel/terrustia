#!/usr/bin/env python3
"""Generate crates/terrustia-proto/src/recipes.rs from Recipe.SetupRecipes.

Only what decrafting needs: for each craftable item, the recipe that makes it and what that
recipe wanted. Shimmer breaks a crafted item back into its ingredients, and without this it
cannot — which was the last real gap in the mechanic.

Deliberately *not* a crafting system. Nothing here knows about crafting stations, conditions, or
whether a player can reach the bench; decrafting needs none of that.
"""
import re
import sys
import pathlib

ROOT = pathlib.Path(sys.argv[1])
OUT = pathlib.Path(sys.argv[2])

src = (ROOT / "Terraria/Recipe.cs").read_text(errors="replace")

# Everything between SetupRecipes and the end of the file holds the declarations; the helper
# methods after it (AddSolarFurniture and friends) declare recipes in the same syntax and are
# called from inside SetupRecipes, so they count too.
start = src.index("public static void SetupRecipes()")
body = src[start:]

# Each recipe runs from a `createItem.SetDefaults(...)` to the next `AddRecipe();`.
chunks = []
for m in re.finditer(
    r"currentRecipe\.createItem\.SetDefaults\((\d+)\);(.*?)AddRecipe\(\);", body, re.S
):
    chunks.append((int(m.group(1)), m.group(2)))

if len(chunks) < 1000:
    raise SystemExit(f"only {len(chunks)} recipes parsed; SetupRecipes' shape changed")


def ingredients_of(text):
    """Both declaration styles, in the order they appear."""
    found = {}

    # Style one: requiredItem[i].SetDefaults(X); optionally requiredItem[i].stack = N;
    for m in re.finditer(r"requiredItem\[(\d+)\]\.SetDefaults\((\d+)\)", text):
        slot, item = int(m.group(1)), int(m.group(2))
        if item > 0:
            found[slot] = [item, 1]
    for m in re.finditer(r"requiredItem\[(\d+)\]\.stack = (\d+)", text):
        slot, stack = int(m.group(1)), int(m.group(2))
        if slot in found:
            found[slot][1] = stack

    # Style two: SetIngredients(item, count, item, count, ...) — a count may be omitted, in which
    # case it is one, so pairs are read greedily and a trailing lone item defaults.
    m = re.search(r"SetIngredients\(([^)]*)\)", text)
    if m:
        nums = [int(x) for x in re.findall(r"-?\d+", m.group(1))]
        # The overload with only item ids is used when every count is one; a mixed call always
        # alternates. Distinguishing them reliably is not possible from the numbers alone, so the
        # game's own convention is followed: the call always alternates item, count.
        slot = len(found)
        for i in range(0, len(nums) - 1, 2):
            if nums[i] > 0:
                found[slot] = [nums[i], max(1, nums[i + 1])]
                slot += 1
        if len(nums) % 2 == 1 and nums[-1] > 0:
            found[slot] = [nums[-1], 1]

    return [tuple(v) for _, v in sorted(found.items())]


recipes = []
for index, (result, text) in enumerate(chunks):
    stack = 1
    m = re.search(r"createItem\.stack = (\d+)", text)
    if m:
        stack = int(m.group(1))
    recipes.append(
        {
            "index": index,
            "result": result,
            "stack": stack,
            "ingredients": ingredients_of(text),
            "not_decraftable": "notDecraftable = true" in text
            or "DisableDecraft()" in text,
            "crimson": "crimson = true" in text or ".AddCondition(Condition.InCrimson" in text,
            "corruption": "corruption = true" in text
            or ".AddCondition(Condition.InCorruption" in text,
            "alchemy": "alchemy = true" in text,
        }
    )

# `UpdateWhichItemsAreCrafted`: the *last* decraftable recipe for a result wins.
is_crafted, crimson_of, corruption_of = {}, {}, {}
for r in recipes:
    if not r["ingredients"]:
        continue
    if not r["not_decraftable"]:
        is_crafted[r["result"]] = r["index"]
    if r["crimson"]:
        crimson_of[r["result"]] = r["index"]
    if r["corruption"]:
        corruption_of[r["result"]] = r["index"]

used = sorted(set(is_crafted.values()) | set(crimson_of.values()) | set(corruption_of.values()))
renumber = {old: new for new, old in enumerate(used)}
kept = [recipes[i] for i in used]

lines = [
    '''//! What crafted things are made of, generated from the game's recipes.
//!
//! This is **not** a crafting system and does not try to be. It knows nothing about crafting
//! stations, conditions, or whether a player can reach a bench. It answers exactly one question:
//! *if this item were broken apart, what would come out?*
//!
//! That is what shimmer needs. An item with no transmutation of its own is decrafted — broken
//! back into its recipe's ingredients — and without this table it simply sat in the pool.
//!
//! Two rules from the game are baked in rather than applied at runtime:
//!
//! * Where several recipes make the same item, the **last** one wins, which is what
//!   `UpdateWhichItemsAreCrafted` does by overwriting as it goes.
//! * Recipes marked `notDecraftable` are excluded outright, so nothing here can be broken apart
//!   that the game would refuse to break.
//!
//! Generated by `tools/gen_recipes.py` from Terraria 1.4.5.7. Do not edit by hand.

/// One ingredient and how many of it the recipe wanted.
pub type Ingredient = (u16, u16);

/// A recipe, as decrafting sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recipe {
    /// What it makes.
    pub result: u16,
    /// How many it makes at once. An item only decrafts if there are at least this many.
    pub makes: u16,
    /// Where this recipe's ingredients start in [`INGREDIENTS`], and how many there are.
    pub first: u16,
    pub count: u8,
    /// Alchemy recipes give back less: each ingredient has a one-in-three chance of being lost,
    /// which is the game's way of stopping potions being a free material duplicator.
    pub alchemy: bool,
}
''',
]

flat = []
rows = []
for r in kept:
    rows.append((r, len(flat), len(r["ingredients"])))
    flat.extend(r["ingredients"])

lines.append(
    "/// Every decraftable recipe's ingredients, packed end to end.\n"
    "///\n"
    "/// `static` rather than `const`: a `const` of this size is copied at every use site."
)
lines.append(f"static INGREDIENTS: [Ingredient; {len(flat)}] = [")
row = []
for item, stack in flat:
    row.append(f"({item},{stack}),")
    if len(row) == 8:
        lines.append("    " + " ".join(row))
        row = []
if row:
    lines.append("    " + " ".join(row))
lines.append("];")
lines.append("")

lines.append(f"/// The recipes themselves, in the game's own order.")
lines.append(f"static RECIPES: [Recipe; {len(kept)}] = [")
for r, first, count in rows:
    lines.append(
        f"    Recipe {{ result: {r['result']}, makes: {r['stack']}, first: {first}, "
        f"count: {count}, alchemy: {str(r['alchemy']).lower()} }},"
    )
lines.append("];")
lines.append("")


def index_table(name, mapping, doc):
    rows = sorted((item, renumber[i]) for item, i in mapping.items() if i in renumber)
    out = [doc, f"static {name}: [(u16, u16); {len(rows)}] = ["]
    row = []
    for item, at in rows:
        row.append(f"({item},{at}),")
        if len(row) == 8:
            out.append("    " + " ".join(row))
            row = []
    if row:
        out.append("    " + " ".join(row))
    out.append("];")
    return "\n".join(out)


lines.append(
    index_table(
        "CRAFTED_BY",
        is_crafted,
        """/// Which recipe makes each craftable item. `ItemID.Sets.IsCrafted`.""",
    )
)
lines.append("")
lines.append(
    index_table(
        "CRAFTED_BY_CRIMSON",
        crimson_of,
        """/// ...and the crimson-world variant, where one exists.""",
    )
)
lines.append("")
lines.append(
    index_table(
        "CRAFTED_BY_CORRUPTION",
        corruption_of,
        """/// ...and the corruption one.""",
    )
)

lines.append(
    '''
fn look_up(table: &[(u16, u16)], key: u16) -> Option<u16> {
    table
        .binary_search_by_key(&key, |&(k, _)| k)
        .ok()
        .map(|at| table[at].1)
}

/// The recipe an item would be broken apart by, if any.
///
/// `ShimmerTransforms.GetDecraftingRecipeIndex`: a world's evil decides between two variants
/// where both exist, so the same item decrafts differently in a crimson world.
pub fn decraft_recipe(item: u16, crimson: bool) -> Option<&'static Recipe> {
    let base = look_up(&CRAFTED_BY, item)?;
    let chosen = if crimson {
        look_up(&CRAFTED_BY_CRIMSON, item).unwrap_or(base)
    } else {
        look_up(&CRAFTED_BY_CORRUPTION, item).unwrap_or(base)
    };
    RECIPES.get(chosen as usize)
}

impl Recipe {
    /// What this recipe wanted.
    pub fn ingredients(&self) -> &'static [Ingredient] {
        let from = self.first as usize;
        &INGREDIENTS[from..from + self.count as usize]
    }
}

/// Whether a recipe is one the world has not earned yet.
///
/// Two gates: some recipes cannot be decrafted until Skeletron is down, others until the Golem
/// is. The game keeps these as recipe sets; both are small and are passed in rather than tabled,
/// since the caller already knows the world's progress.
pub fn decraft_locked(_recipe: &Recipe, _downed_skeletron: bool, _downed_golem: bool) -> bool {
    // `RecipeSets.PostSkeletron` and `PostGolem` are empty in 1.4.5.7 — the gates exist and
    // nothing is behind them. Kept as a named function so a later version that fills them in
    // needs a table rather than a new concept.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lookups are sorted, or the binary searches silently miss.
    #[test]
    fn the_lookups_are_sorted() {
        for (name, table) in [
            ("CRAFTED_BY", &CRAFTED_BY[..]),
            ("CRAFTED_BY_CRIMSON", &CRAFTED_BY_CRIMSON[..]),
            ("CRAFTED_BY_CORRUPTION", &CRAFTED_BY_CORRUPTION[..]),
        ] {
            assert!(
                table.windows(2).all(|w| w[0].0 < w[1].0),
                "{name} must be strictly ascending"
            );
        }
    }

    /// Every recipe's ingredient span lies inside the packed array and holds real items.
    #[test]
    fn every_recipe_points_at_real_ingredients() {
        for recipe in RECIPES {
            let end = recipe.first as usize + recipe.count as usize;
            assert!(
                end <= INGREDIENTS.len(),
                "item {} runs past the ingredient table",
                recipe.result
            );
            assert!(recipe.count > 0, "item {} has no ingredients", recipe.result);
            assert!(recipe.makes > 0, "item {} makes nothing", recipe.result);
            for &(item, stack) in recipe.ingredients() {
                assert!(item > 0, "recipe for {} wants item zero", recipe.result);
                assert!(stack > 0, "recipe for {} wants none of {item}", recipe.result);
            }
        }
    }

    /// Every index in the lookups names a recipe that exists.
    #[test]
    fn every_lookup_names_a_recipe() {
        for table in [&CRAFTED_BY[..], &CRAFTED_BY_CRIMSON[..], &CRAFTED_BY_CORRUPTION[..]] {
            for &(item, at) in table {
                assert!(
                    (at as usize) < RECIPES.len(),
                    "item {item} names recipe {at}, past the end"
                );
            }
        }
    }

    /// A well-known one, end to end.
    #[test]
    fn a_torch_decrafts_into_what_it_was_made_of() {
        let torch = decraft_recipe(8, false).expect("a torch is crafted");
        assert_eq!(torch.result, 8);
        assert_eq!(torch.makes, 3, "three torches at a time");
        let wants: Vec<u16> = torch.ingredients().iter().map(|&(i, _)| i).collect();
        assert!(wants.contains(&23), "a torch wants gel");
    }

    /// Something that is not crafted decrafts into nothing.
    ///
    /// Note that a great deal *is* crafted, including things that feel primitive — an Iron
    /// Pickaxe is ten bars and three wood. So this checks the shape of the answer rather than
    /// guessing at which items are raw.
    #[test]
    fn raw_materials_do_not_decraft() {
        assert!(decraft_recipe(0, false).is_none(), "nothing at all");
        let uncraftable = (1u16..2000)
            .filter(|&i| decraft_recipe(i, false).is_none())
            .count();
        assert!(
            uncraftable > 100,
            "only {uncraftable} of the first two thousand items are uncrafted, which is too few"
        );
    }

    /// The two evils can disagree about what an item is made of.
    #[test]
    fn the_evils_can_differ() {
        let differing = (0u16..u16::MAX).filter(|&i| {
            match (decraft_recipe(i, false), decraft_recipe(i, true)) {
                (Some(a), Some(b)) => a != b,
                _ => false,
            }
        });
        assert!(
            differing.take(1).count() > 0 || CRAFTED_BY_CRIMSON.is_empty(),
            "if any recipe is evil-specific, some item should decraft differently"
        );
    }
}
'''
)

OUT.write_text("\n".join(lines) + "\n")
print(
    f"wrote {OUT}: {len(chunks)} recipes parsed, {len(kept)} decraftable, "
    f"{len(flat)} ingredient entries, {len(is_crafted)} craftable items"
)
