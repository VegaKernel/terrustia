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

# Pull the generated tables back out of the Rust. `rustfmt` puts a space after every tuple comma
# (`(1, 1419)`, not `(1,1419)`) and lays each `Recipe { .. }` out one field per line rather than on
# a single line — both defeated the old space-free, single-line-only regexes below, which then
# matched nothing at all (0 rows) rather than erroring, so every real recipe looked MISSING. `\s*`
# tolerates a run's actual whitespace either way, single space or newline-plus-indent alike.
ing = [
    (int(a), int(b))
    for a, b in re.findall(
        r"\((\d+),\s*(\d+)\),", GEN.split("static INGREDIENTS")[1].split("];")[0]
    )
]
recipes = [
    (int(m.group(1)), int(m.group(2)), int(m.group(3)), int(m.group(4)))
    for m in re.finditer(
        r"Recipe\s*\{\s*result:\s*(\d+),\s*makes:\s*(\d+),\s*first:\s*(\d+),\s*count:\s*(\d+),",
        GEN,
    )
]
crafted = {
    int(a): int(b)
    for a, b in re.findall(
        r"\((\d+),\s*(\d+)\),", GEN.split("static CRAFTED_BY:")[1].split("];")[0]
    )
}
if not crafted:
    # A parser that silently returns nothing is worse than one that errors: an empty `crafted`
    # makes every single sampled item look MISSING, which is indistinguishable from "everything is
    # actually broken" unless this is caught explicitly. This is exactly the failure mode that let
    # this checker rot unnoticed — see the regex comment above.
    raise SystemExit("parsed 0 CRAFTED_BY rows; recipes.rs's shape changed under this regex")

# Re-parse the source independently: last decraftable recipe per result wins.
body = src[src.index("public static void SetupRecipes()") :]


def resolve_locals(text: str) -> str:
    """Substitute the integer locals and ingredient arrays `SetupRecipes` hoists its materials
    into, so a recipe whose material is a variable is not read as no material at all.

    `Recipe.cs` declares exactly three that matter (`int num = 5; int stack = 2;` for the sofas,
    `int type = 3234;` for the crystal furniture, `int num = 3955;` for the Lesion furniture) and
    passes several of the Lesion ones through an `int[] objN = new int[K] { 0, ... }; objN[0] =
    num;` array. Reading only digits made a sofa give back 1+1 instead of 5+2 and turned the Lesion
    Bed's two ingredients into "one of item 7", the array variable's own trailing digit.

    A member access is never substituted: `requiredItem[0].stack = stack;` has one `stack` that is
    a field and one that is the local, and only the second is a value.
    """
    ints: dict[str, str] = {}
    arrays: dict[str, list[int]] = {}
    out: list[str] = []
    ident = re.compile(r"(?<![\w.])(\w+)")
    for raw in text.splitlines():
        line = raw.strip()
        if m := re.fullmatch(r"int (\w+) = (\d+);", line):
            ints[m.group(1)] = m.group(2)
            out.append(raw)
            continue
        if m := re.fullmatch(r"int\[\] (\w+) = new int\[\d*\] \{([^}]*)\};", line):
            arrays[m.group(1)] = [int(n) for n in re.findall(r"-?\d+", m.group(2))]
            out.append(raw)
            continue
        if re.match(r"(?:currentRecipe\.|\w+\.SetIngredients\(|\w+\[\d+\] = )", line):
            line = ident.sub(lambda m: ints.get(m.group(1), m.group(1)), line)
        # `objN[0] = 3955;` patches the placeholder its declaration left behind.
        if (m := re.fullmatch(r"(\w+)\[(\d+)\] = (-?\d+);", line)) and m.group(1) in arrays:
            values = arrays[m.group(1)]
            if int(m.group(2)) < len(values):
                values[int(m.group(2))] = int(m.group(3))
        # `recipeN.SetIngredients(objN);` becomes the numbers themselves.
        if (m := re.fullmatch(r"(\w+\.SetIngredients\()(\w+)(\);)", line)) and m.group(
            2
        ) in arrays:
            line = m.group(1) + ", ".join(map(str, arrays[m.group(2)])) + m.group(3)
        # The one arithmetic stack in the whole file (`Recipe.cs:6374`); a C# `(int)` cast of a
        # positive float truncates.
        line = re.sub(
            r"\(int\)\(\(float\)(\d+) \* ([\d.]+)f\)",
            lambda m: str(int(float(m.group(1)) * float(m.group(2)))),
            line,
        )
        out.append(line)
    return "\n".join(out)


body = resolve_locals(body)
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
