#!/usr/bin/env python3
"""Independently verify the generated recipe table against the decompiled source.

Written from the source rather than from the generator, so a bug shared by both would have to be
made twice. Picks recipes at random, re-parses their chunk by hand, and compares.
"""
import re
import sys
import random
import pathlib

D = sys.argv[1]
GEN = pathlib.Path(sys.argv[2]).read_text()
src = pathlib.Path(D + "/Terraria/Recipe.cs").read_text(errors="replace")

# Pull the generated tables back out of the Rust.
ing = [
    (int(a), int(b))
    for a, b in re.findall(r"\((\d+),(\d+)\),", GEN.split("static INGREDIENTS")[1].split("];")[0])
]
recipes = [
    (int(m.group(1)), int(m.group(2)), int(m.group(3)), int(m.group(4)))
    for m in re.finditer(
        r"Recipe \{ result: (\d+), makes: (\d+), first: (\d+), count: (\d+),", GEN
    )
]
crafted = {
    int(a): int(b)
    for a, b in re.findall(r"\((\d+),(\d+)\),", GEN.split("static CRAFTED_BY:")[1].split("];")[0])
}

# Re-parse the source independently: last decraftable recipe per result wins.
body = src[src.index("public static void SetupRecipes()") :]
chunks = re.findall(
    r"currentRecipe\.createItem\.SetDefaults\((\d+)\);(.*?)AddRecipe\(\);", body, re.S
)

truth = {}
for result, text in chunks:
    result = int(result)
    if "notDecraftable = true" in text or "DisableDecraft()" in text:
        continue
    slots = {}
    for m in re.finditer(r"requiredItem\[(\d+)\]\.SetDefaults\((\d+)\)", text):
        if int(m.group(2)) > 0:
            slots[int(m.group(1))] = [int(m.group(2)), 1]
    for m in re.finditer(r"requiredItem\[(\d+)\]\.stack = (\d+)", text):
        if int(m.group(1)) in slots:
            slots[int(m.group(1))][1] = int(m.group(2))
    m = re.search(r"SetIngredients\(([^)]*)\)", text)
    if m:
        nums = [int(x) for x in re.findall(r"-?\d+", m.group(1))]
        at = len(slots)
        for i in range(0, len(nums) - 1, 2):
            if nums[i] > 0:
                slots[at] = [nums[i], max(1, nums[i + 1])]
                at += 1
        if len(nums) % 2 == 1 and nums[-1] > 0:
            slots[at] = [nums[-1], 1]
    if not slots:
        continue
    makes = 1
    sm = re.search(r"createItem\.stack = (\d+)", text)
    if sm:
        makes = int(sm.group(1))
    truth[result] = (makes, [tuple(v) for _, v in sorted(slots.items())])

random.seed(20260823)
sample = random.sample(sorted(truth), min(300, len(truth)))
bad = 0
for item in sample:
    makes, wants = truth[item]
    if item not in crafted:
        print(f"  MISSING: item {item} is craftable in the source but not in the table")
        bad += 1
        continue
    result, gen_makes, first, count = recipes[crafted[item]]
    got = ing[first : first + count]
    if result != item or gen_makes != makes or got != wants:
        print(f"  WRONG: item {item}")
        print(f"    source: makes {makes} from {wants}")
        print(f"    table:  makes {gen_makes} from {got} (result field {result})")
        bad += 1

print(f"checked {len(sample)} recipes against the source independently")
print(f"  {len(truth)} craftable items in the source, {len(crafted)} in the table")
print("  " + ("ALL MATCH" if bad == 0 else f"{bad} DISAGREEMENTS"))
sys.exit(1 if bad else 0)
