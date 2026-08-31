#!/usr/bin/env python3
"""Independently verify the generated recipe table against the decompiled source.

Written from the source rather than from the generator, so a bug shared by both would have to be
made twice. Picks recipes at random, re-parses their chunk by hand, and compares.
"""
import re
import sys
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
        # `for (int l = 3309; l <= 3314; l++) { ... SetDefaults(l) ... }` writes six recipes for
        # one result, and this file keeps the last decraftable recipe per result, so the surviving
        # one is the final iteration's. Binding the counter to its *last* value reproduces exactly
        # that recipe. Without this the loop counter was not a digit, the ingredient slot it fills
        # never matched, and the recipe came out one ingredient short: item 5547 read as "makes 1
        # from item 3306" when it takes 3306 *and* 3314. That was reported as the table being
        # wrong; the table was right and this checker was blind. `SetupRecipes` has three loops of
        # this counted shape (3665..=3704, 2114..=2118, 3309..=3314, 4327..=4332); the rest count
        # over an array length and write no recipe of their own.
        if m := re.match(r"for \(int (\w+) = ", line):
            counted = re.fullmatch(r"for \(int (\w+) = (\d+); \1 <= (\d+); \1\+\+\)", line)
            if counted:
                ints[counted.group(1)] = counted.group(3)
            else:
                # A loop over an array length rebinds the same short name (`i`, `j`) to something
                # this cannot resolve. Forgetting it is the point: leaving the previous counted
                # loop's last value bound would substitute it into an unrelated line later.
                ints.pop(m.group(1), None)
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

def literal(arg: str) -> int | None:
    """A `SetDefaults(...)` argument as an integer, or `None` when it cannot be read here.

    Two shapes beyond a plain number occur in `SetupRecipes`: `num5 - 4327 + 4334`, which is
    arithmetic on a loop counter `resolve_locals` has already substituted, and
    `ItemID.Sets.TextureCopyLoad[i]`, a lookup into a table that lives in another file.

    Returning `None` for the second is the point. The old code's regex simply did not match it, so
    the ingredient vanished and the recipe was compared *one slot short*: item 3704 was reported
    as a table error when the table was right and this checker could not read the source. A
    checker that cannot read something has to say so, not quietly compare the remainder.
    """
    arg = arg.strip()
    if re.fullmatch(r"-?\d+", arg):
        return int(arg)
    if re.fullmatch(r"-?\d+(?:\s*[-+]\s*\d+)+", arg):
        total, sign = 0, 1
        for token in re.findall(r"[-+]|\d+", arg):
            if token in "-+":
                sign = -1 if token == "-" else 1
            else:
                total += sign * int(token)
                sign = 1
        return total
    return None


truth = {}
unreadable = 0
for result, text in chunks:
    result = int(result)
    if "notDecraftable = true" in text or "DisableDecraft()" in text:
        continue
    slots = {}
    skip = False
    for m in re.finditer(r"requiredItem\[(\d+)\]\.SetDefaults\(([^()]*)\)", text):
        value = literal(m.group(2))
        if value is None:
            skip = True
            break
        if value > 0:
            slots[int(m.group(1))] = [value, 1]
    if skip:
        unreadable += 1
        continue
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

# Every craftable item, not a sample of them.
#
# This used to be `random.seed(20260823); random.sample(sorted(truth), 300)`: 300 of the 2543
# recipes the source defines, drawn against a *fixed* seed, so the same 88% of the table was never
# compared with anything and never would be. `tools/mutate_tables.py` measured exactly that:
# corrupting 40 random `result:` fields in `recipes.rs` was caught 3 times out of 40, and the three
# were the ones that happened to fall inside the sample. The whole run costs a fraction of a second
# either way, so the sampling bought nothing and hid nearly everything.
sample = sorted(truth)
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
print(f"  {unreadable} recipe(s) skipped: an ingredient this checker cannot resolve to a number")
print("  " + ("ALL MATCH" if bad == 0 else f"{bad} DISAGREEMENTS"))
sys.exit(1 if bad else 0)
