#!/usr/bin/env python3
"""Harvest every vanilla citation in `crates/*/src`, content-key it, and report what is uncited.

    python3 tools/parity_index.py <decompiled-tree>              # check (the `just check-parity` mode)
    python3 tools/parity_index.py <decompiled-tree> --update     # rewrite docs/parity-index.tsv
    python3 tools/parity_index.py <decompiled-tree> --coverage   # the "what have we never read" table

AGENTS.md rule 2 makes every transcription cite its source, so the tree already holds ~1800
`NPC.cs:12345` references. That is raw data, and this derives from it rather than restating it.

Why derived and not written down by hand: this project's signature defect is a hand-maintained
claim about code it does not live in, and when such a claim rots it does not say "unknown", it says
"verified". So each entry carries two hashes - the cited vanilla lines, and our own item's body -
and a claim expires on its own the moment either side moves. `--check` then says *which* side moved:

  * the vanilla side moved -> the citation drifted, or the decompiled tree was regenerated
  * our side moved         -> the transcription was edited after it was checked against the game

WHAT THIS DOES NOT DO. It never judges whether a transcription is *correct*. It answers exactly two
questions: "is this still the code it was checked against" and "what is cited by nothing". Claiming
more is how it becomes another lying document.

The coverage half is the one that cannot lie in the dangerous direction. A citation index can only
overstate by going stale, which the hashes catch; coverage can only understate, because a region we
read and did not cite reads as uncited. "40% of NPC.cs is cited by nothing" is a floor, not a boast.

Scope: `crates/*/src/**/*.rs` only. `tests/` and `examples/` are excluded because their citations
live in assertion *strings* (`assert_eq!(rate, 600, "NPC.cs:6190")`), which are not comments and are
already checked by the test itself. The generated tables are included: `npc_params.rs` (126
citations) and `conditional_drops.rs` (101) are hand-written despite sitting beside the generated
ones, and the truly generated files carry a handful of provenance citations each that are worth the
same expiry as any other. A `just regen` that rewrites one will fail this check, which is correct:
the table changed, so the citations against it need re-reading, not silent carry-over.

Needs the decompiled tree, so like `check_drops.py` this is qualification-time only and can never
run in hosted CI.
"""

import hashlib
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INDEX = ROOT / "docs" / "parity-index.tsv"
COLUMNS = ["rust_file", "item", "vanilla_file", "lines", "vanilla_hash", "our_hash"]

# A citation: an optional namespace directory, a `.cs` file, and one or more lines or ranges.
# `\s*` after the colon is deliberate: seven citations in the tree wrap across a comment line
# between the colon and the number, and comment blocks are joined before this runs.
FILE = re.compile(r"(?:(?P<dir>[A-Za-z0-9_.]+)/)?(?P<file>[A-Za-z_][A-Za-z0-9_.]*\.cs)")
CITE = re.compile(
    FILE.pattern + r":\s*(?P<spans>\d+(?:-\d+)?(?:\s*,\s*\d+(?:-\d+)?)*)"
)
# `(`WorldGen.cs:72937` overground, `:73877` underground)`: a bare span continuing the last file
# named in the same comment block. 161 of these.
CONT = re.compile(r"`:\s*(\d+(?:-\d+)?(?:\s*,\s*\d+(?:-\d+)?)*)`")
# A `.cs:` that neither of the above claimed. Counted and reported, never dropped in silence.
# `(?!:)` lets `Minecart.cs::Initialize` through: naming a method is not citing a line.
LOOSE = re.compile(r"[A-Za-z_][A-Za-z0-9_.]*\.cs:(?!:)")

ITEM = re.compile(
    r"^[ \t]*"
    r"(?:pub(?:\s*\([^)]*\))?\s+)?"
    r"(?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r'(?:extern\s+(?:"[^"]*"\s+)?)?'
    r"(?P<kind>macro_rules!|(?:fn|const|static|struct|enum|union|trait|impl|mod|type)\b)"
    r"(?:\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*))?"
)
CHAR_LIT = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'])'")


def split_code_and_comments(lines):
    """Per line, return (code, comment) with strings, chars and comments removed from `code`.

    Braces inside a string or a `'{'` char literal must not move the nesting depth, and one wrong
    depth cascades into every item below it, so this walks characters rather than regexing.
    """
    code_out, comment_out = [], []
    block = 0  # `/* */` nesting; Rust's block comments nest
    raw_hashes = None  # inside `r#"..."#`, holding the hash count
    for line in lines:
        code, comment = [], None  # `None` is "no comment"; `""` is an empty one, e.g. a bare `///`
        i, n = 0, len(line)
        while i < n:
            if raw_hashes is not None:
                end = line.find('"' + "#" * raw_hashes, i)
                if end < 0:
                    break
                i, raw_hashes = end + 1 + raw_hashes, None
                continue
            if block:
                end, start = line.find("*/", i), line.find("/*", i)
                if start >= 0 and (end < 0 or start < end):
                    block, i = block + 1, start + 2
                elif end >= 0:
                    block, i = block - 1, end + 2
                else:
                    break
                continue
            c = line[i]
            if c == "/" and line[i + 1 : i + 2] == "/":
                # Drop the doc marker too. Comment blocks are joined before citations are read, and
                # `///` would otherwise wedge a stray `/` into a citation that wrapped across lines.
                comment = line[i + 2 :].lstrip("/!")
                break
            if c == "/" and line[i + 1 : i + 2] == "*":
                block, i = 1, i + 2
                continue
            if c == "r" and line[i + 1 : i + 2] in ('#', '"'):
                j = i + 1
                while j < n and line[j] == "#":
                    j += 1
                if j < n and line[j] == '"':
                    raw_hashes, i = j - i - 1, j + 1
                    continue
            if c == '"':
                i += 1
                while i < n:
                    if line[i] == "\\":
                        i += 2
                    elif line[i] == '"':
                        i += 1
                        break
                    else:
                        i += 1
                continue
            if c == "'":
                m = CHAR_LIT.match(line, i)
                if m:
                    i = m.end()
                    continue
            code.append(c)
            i += 1
        code_out.append("".join(code))
        comment_out.append(comment)
    return code_out, comment_out


def name_of(line):
    """An item's *identity*, which is the index key, so it must not move when the item's body does.

    `const MAX: usize`, not `pub const MAX: usize = 15;`: keying on the header text would re-key the
    entry every time the value changed, and the diff would read "citation gone, new citation" when
    what actually happened is "our side moved", which is the one sentence this file exists to say.
    """
    m = ITEM.match(line)
    if m and m.group("kind") != "impl" and m.group("name"):
        return f"{m.group('kind').rstrip()} {m.group('name')}"
    text = " ".join(line.split()).rstrip("{").rstrip()  # `impl A for B`, and anything unparsed
    return text if len(text) <= 80 else text[:77] + "..."


def find_items(lines, code):
    """Every item in the file as (start, end, path), innermost last. 0-based, inclusive."""
    items = []
    stack = []  # [start, base_depth, path] for items whose body is open
    pending = None  # a header seen, waiting for its `{` or its `;`
    depth = 0
    for i, text in enumerate(code):
        if pending is None and ITEM.match(lines[i]):
            parent = stack[-1][2] + " :: " if stack else ""
            pending = (i, depth, parent + name_of(lines[i]))
        for ch in text:
            if ch == "{":
                depth += 1
                if pending and depth == pending[1] + 1:
                    stack.append([pending[0], pending[1], pending[2]])
                    pending = None
            elif ch == "}":
                depth -= 1
                if stack and depth == stack[-1][1]:
                    start, _, path = stack.pop()
                    items.append((start, i, path))
            elif ch == ";" and pending and depth == pending[1]:
                items.append((pending[0], i, pending[2]))
                pending = None
    return items


def attribute(line, items, starts, lines):
    """The item a comment at `line` belongs to: the one it documents, else the one it sits in."""
    j = line + 1
    while j < len(lines):
        stripped = lines[j].strip()
        if stripped and not stripped.startswith(("//", "#")):
            break
        j += 1
    if j in starts:
        return starts[j]
    inner = None
    for start, end, path in items:
        if start <= line <= end and (inner is None or end - start < inner[1]):
            inner = (path, end - start)
    return inner[0] if inner else "(file)"


def body_hash(lines, span):
    """Hash an item's body, ignoring comment-only lines: code drift is the signal, prose is not."""
    if span is None:
        text = [ln.rstrip() for ln in lines if not ln.strip().startswith("//")]
    else:
        start, end = span
        text = [ln.rstrip() for ln in lines[start : end + 1] if not ln.strip().startswith("//")]
    return hashlib.sha256("\n".join(text).encode()).hexdigest()[:12]


def parse_spans(text):
    out = []
    for part in text.split(","):
        part = part.strip()
        a, _, b = part.partition("-")
        out.append((int(a), int(b) if b else int(a)))
    return out


def harvest(tree):
    """Every citation in `crates/*/src`, with both content keys. Returns (rows, stats)."""
    by_base = defaultdict(list)
    for cs in tree.rglob("*.cs"):
        by_base[cs.name].append(cs)
    vanilla_lines = {}

    def read_vanilla(path):
        if path not in vanilla_lines:
            vanilla_lines[path] = path.read_text(errors="replace").splitlines()
        return vanilla_lines[path]

    rows = {}
    covered = defaultdict(set)
    stats = defaultdict(int)
    problems = []
    fileonly = set()

    for rs in sorted(ROOT.glob("crates/*/src/**/*.rs")):
        rel = rs.relative_to(ROOT).as_posix()
        lines = rs.read_text().splitlines()
        code, comments = split_code_and_comments(lines)
        items = find_items(lines, code)
        starts = {}
        for start, end, path in items:
            starts.setdefault(start, path)
        extent = {path: (s, e) for s, e, path in items}

        # Group contiguous whole-line comments into one block: a citation that wraps across a line
        # break, and a `:1234` continuing the file named two lines up, are both only parseable here.
        blocks, i = [], 0
        while i < len(lines):
            whole = comments[i] is not None and not code[i].strip()
            if whole:
                j = i
                while j < len(lines) and comments[j] is not None and not code[j].strip():
                    j += 1
                blocks.append((i, " ".join(comments[i:j])))
                i = j
            else:
                if comments[i]:
                    blocks.append((i, comments[i]))
                i += 1

        for line, text in blocks:
            if ".cs" not in text:
                continue
            claimed, hits, conts = [], [], []
            for m in CITE.finditer(text):
                claimed.append((m.start(), m.end()))
                hits.append((m.start(), m.group("dir"), m.group("file"), m.group("spans")))
            for m in CONT.finditer(text):
                if any(a <= m.start() < b for a, b in claimed):
                    continue  # the tail of a `Foo.cs:1-2` already claimed above
                conts.append((m.start(), m.end(), m.group(1)))
            # A `:1234` continues the nearest preceding *file name*, cited or not. Two rules that
            # look right and are not: "the last citation in the block" sends `NPC.cs:17256`,
            # `:17340` to a `Main.cs` cited further down; "the last citation before it" sends the
            # eight `:937, 962, ...` in codegen/drops.rs to `ItemDropRule.cs`, when the sentence
            # they continue names `ItemDropDatabase.cs` with no line number of its own.
            named = [(m.start(), m.group("dir"), m.group("file")) for m in FILE.finditer(text)]
            for start, end, spans in conts:
                prior = [f for f in named if f[0] < start]
                if not prior:
                    continue
                _, dirname, filename = prior[-1]
                claimed.append((start, end))
                hits.append((start, dirname, filename, spans))
            hits.sort()
            for m in LOOSE.finditer(text):
                if not any(a <= m.start() < b for a, b in claimed):
                    stats["unparsed"] += 1
                    problems.append(f"  unparsed  {rel}:{line + 1}  {text.strip()[:110]}")
            # A comment that names a file and no line ("the names match `ProjectileID.cs`") is a
            # pointer, not a citation. It gets no content key, so it can never go stale and can
            # never expire. Counted so the number is visible rather than mistaken for coverage.
            for start, _, filename in named:
                if not any(a <= start < b for a, b in claimed):
                    stats["fileonly"] += 1
                    fileonly.add(filename)

            item = attribute(line, items, starts, lines) if hits else None
            for _, dirname, filename, spans in hits:
                stats["found"] += 1
                spans = ",".join(s.strip() for s in spans.split(","))
                cands = by_base.get(filename, [])
                if dirname:
                    cands = [c for c in cands if c.as_posix().endswith(f"{dirname}/{filename}")]
                if not cands:
                    key = (rel, item or "?", filename, spans)
                    rows[key] = ("NOFILE", "NOFILE")
                    stats["nofile"] += 1
                    continue
                if len(cands) > 1:
                    # 18 basenames repeat in the tree (`Conditions.cs` lives in both
                    # `Terraria.WorldBuilding` and `Terraria.GameContent.ItemDropRules`). A citation
                    # with no namespace prefix is still unambiguous when only one candidate is long
                    # enough to hold the lines it names.
                    need = max(b for _, b in parse_spans(spans))
                    fits = [c for c in cands if len(read_vanilla(c)) >= need]
                    if len(fits) == 1:
                        cands = fits
                if len(cands) > 1:
                    key = (rel, item or "?", filename, spans)
                    rows[key] = ("AMBIG", "AMBIG")
                    stats["ambig"] += 1
                    continue
                src = cands[0]
                text_lines = read_vanilla(src)
                cited, bad = [], False
                for a, b in parse_spans(spans):
                    if a < 1 or b > len(text_lines) or b < a:
                        bad = True
                        break
                    cited.extend(text_lines[a - 1 : b])
                    covered[src].update(range(a, b + 1))
                key = (rel, item or "?", filename, spans)
                if bad:
                    rows[key] = ("NOLINES", "NOLINES")
                    stats["nolines"] += 1
                    continue
                vhash = hashlib.sha256(
                    "\n".join(ln.rstrip() for ln in cited).encode()
                ).hexdigest()[:12]
                rows[key] = (vhash, body_hash(lines, extent.get(item)))
                stats["resolved"] += 1

    stats["fileonly_distinct"] = len(fileonly)
    return rows, covered, stats, problems, by_base


def load_index():
    if not INDEX.exists():
        return None
    out = {}
    for line in INDEX.read_text().splitlines():
        if not line.strip() or line.startswith("#") or line.startswith(COLUMNS[0] + "\t"):
            continue
        rust, item, vfile, spans, vhash, ohash = line.split("\t")
        out[(rust, item, vfile, spans)] = (vhash, ohash)
    return out


BANNER = """\
# Generated by tools/parity_index.py. Do not hand-edit: rebuild with `just parity-update` and
# review the diff, the same rule the generated data tables live under.
#
# One row per vanilla citation in `crates/*/src`. `vanilla_hash` keys the cited lines as they exist
# in the decompiled tree; `our_hash` keys the body of the Rust item the citation is attached to,
# with comment-only lines dropped. `just check-parity` recomputes both and fails when either moved,
# naming which: the vanilla side means the citation drifted or the tree was regenerated, our side
# means the transcription was edited after it was checked against the game.
#
# This says whether a transcription is still the code it was checked against. It never says whether
# it is correct. `just parity-coverage` says what is cited by nothing at all, which is the other
# half and the half that cannot overstate.
"""


def write_index(rows):
    body = [BANNER.rstrip(), "\t".join(COLUMNS)]
    for key in sorted(rows):
        body.append("\t".join([*key, *rows[key]]))
    INDEX.write_text("\n".join(body) + "\n")


def coverage_report(tree, covered, by_base, full):
    """What fraction of each vanilla file anything we wrote has cited, and where the gaps are.

    Ranked by *uncited* lines, not by size: the column that matters is the one nobody has read.
    A region we read and chose not to cite reads as uncited here, so every number is a floor.
    """
    print("── COVERAGE: how much of the game anything we wrote actually cites ──")
    print(f"{'vanilla file':<40} {'lines':>7} {'cited':>7} {'%':>6}  largest uncited region")
    sized = []
    for src, seen in covered.items():
        total = len(src.read_text(errors="replace").splitlines())
        sized.append((total - len(seen), total, src, seen))
    sized.sort(reverse=True, key=lambda t: t[0])
    tree_total = tree_cited = 0
    for _, total, src, seen in sized:
        tree_total += total
        tree_cited += len(seen)
        gaps, run = [], 0
        for n in range(1, total + 1):
            if n in seen:
                if run:
                    gaps.append((run, n - run, n - 1))
                run = 0
            else:
                run += 1
        if run:
            gaps.append((run, total - run + 1, total))
        gaps.sort(reverse=True)
        biggest = f"{gaps[0][1]}-{gaps[0][2]} ({gaps[0][0]} lines)" if gaps else "-"
        print(
            f"{src.name:<40} {total:>7} {len(seen):>7} "
            f"{100 * len(seen) / total:>5.1f}%  {biggest}"
        )
    print(f"\n  cited files: {len(covered)}, {tree_cited}/{tree_total} lines "
          f"({100 * tree_cited / max(tree_total, 1):.1f}%) of what they contain")

    uncited = []
    for paths in by_base.values():
        for p in paths:
            if p not in covered:
                uncited.append((len(p.read_text(errors="replace").splitlines()), p))
    uncited.sort(reverse=True, key=lambda t: t[0])
    total_uncited = sum(n for n, _ in uncited)
    print(f"\n── NEVER CITED: {len(uncited)} files, {total_uncited} lines, cited by nothing ──")
    print("  Much of this is client-only (drawing, UI, input) and a dedicated server will never")
    print("  need it. That judgement is a human's; this only reports what nothing points at.")
    for n, p in uncited[: (60 if full else 12)]:
        print(f"  {n:>6}  {p.relative_to(tree).as_posix()}")
    if not full and len(uncited) > 12:
        print(f"  ... and {len(uncited) - 12} more (--coverage for the long list)")


SELF_TEST = '''\
//! Module doc citing `Main.cs:10-11`.

/// Doc for the const, from `NPC.cs:1`, and a wrapped one at `NPC.cs:
/// 2-3`.
pub const A: u32 = 7;

impl Thing {
    /// `WorldGen.cs:1`, `:2` and `:3` all continue WorldGen.
    fn inner(&self) -> char {
        let brace = '{';                   // a brace in a char literal, `Liquid.cs:1`
        let s = "not a // comment, }}}}";
        let raw = r#"} " #"#;
        brace
    }
}

// `Minecart.cs::Initialize` names a method, not a line.
fn after() {}
'''


def self_test():
    """Prove the parser on the four forms that used to drop data on the floor, silently."""
    lines = SELF_TEST.splitlines()
    code, comments = split_code_and_comments(lines)

    # Braces inside a char literal, a string and a raw string must not move the nesting depth,
    # or every item below them lands in the wrong place.
    assert code[9].count("{") == 0 and code[9].count("}") == 0, code[9]
    assert code[10].count("}") == 0, code[10]
    assert code[11].count("}") == 0, code[11]

    items = {name for _, _, name in find_items(lines, code)}
    assert "const A" in items, items  # not `pub const A: u32 = 7;`: the value must not be the key
    assert "impl Thing :: fn inner" in items, items
    assert "fn after" in items, items

    # `///` and `//!` markers are stripped so a citation can wrap across a comment line, and a
    # bare `:2` binds to the nearest file named before it, cited or not.
    block = " ".join(c for c in comments[2:4] if c is not None)
    assert [(m.group("file"), m.group("spans")) for m in CITE.finditer(block)] == [
        ("NPC.cs", "1"),
        ("NPC.cs", "2-3"),
    ], block
    doc = comments[7]
    assert [m.group("spans") for m in CITE.finditer(doc)] == ["1"], doc
    assert [m.group(1) for m in CONT.finditer(doc)] == ["2", "3"], doc
    assert "Minecart" in comments[16] and not LOOSE.search(comments[16]), comments[16]

    print("self-test ok")
    return 0


def main():
    args = [a for a in sys.argv[1:]]
    if "--self-test" in args:
        return self_test()
    flags = {a for a in args if a.startswith("--")}
    positional = [a for a in args if not a.startswith("--")]
    if len(positional) != 1:
        print(__doc__.splitlines()[0])
        print("\nusage: python3 tools/parity_index.py <decompiled-tree> [--update|--coverage]")
        return 2
    tree = Path(positional[0])
    if not tree.is_dir():
        print(f"error: no decompiled tree at {tree}")
        return 2

    rows, covered, stats, problems, by_base = harvest(tree)

    print(f"citations found: {stats['found']}   "
          f"resolved: {stats['resolved']}   "
          f"unparsed `.cs:`: {stats['unparsed']}   "
          f"file not in tree: {stats['nofile']}   "
          f"ambiguous name: {stats['ambig']}   "
          f"line out of range: {stats['nolines']}")
    print(f"a file named with no line, so no content key and nothing that can expire: "
          f"{stats['fileonly']} mentions of {stats['fileonly_distinct']} files")

    broken = sorted(k for k, v in rows.items() if v[0] in ("NOFILE", "AMBIG", "NOLINES"))
    if broken or problems:
        print("\n── CITATIONS THAT DO NOT RESOLVE ──")
        for k in broken:
            print(f"  {rows[k][0]:<8}  {k[0]} :: {k[1]}  ->  {k[2]}:{k[3]}")
        for p in problems:
            print(p)

    if "--coverage" in flags:
        print()
        coverage_report(tree, covered, by_base, full=True)
        return 0

    if "--update" in flags:
        write_index(rows)
        print(f"\nwrote {len(rows)} entries to {INDEX.relative_to(ROOT)}")
        return 1 if broken else 0

    old = load_index()
    if old is None:
        print(f"\nerror: no {INDEX.relative_to(ROOT)}. Build it with --update.")
        return 1

    moved_vanilla, moved_ours, moved_both, added, removed = [], [], [], [], []
    for key, (vhash, ohash) in sorted(rows.items()):
        if key not in old:
            added.append(key)
            continue
        was_v, was_o = old[key]
        if vhash != was_v and ohash != was_o:
            moved_both.append(key)
        elif vhash != was_v:
            moved_vanilla.append(key)
        elif ohash != was_o:
            moved_ours.append(key)
    removed = sorted(k for k in old if k not in rows)

    print()
    coverage_report(tree, covered, by_base, full=False)
    print()

    # A regenerated decompiled tree shifts every line number at once. Saying so in one sentence is
    # the difference between a legible failure and 1800 lines of noise that get scrolled past.
    if rows and len(moved_vanilla) + len(moved_both) > len(rows) // 4:
        print(f"── THE DECOMPILED TREE MOVED ──")
        print(f"  {len(moved_vanilla) + len(moved_both)} of {len(rows)} citations point at "
              f"different vanilla lines than the index was built from.")
        print("  That is a regenerated or different tree, not 1800 individual drifts. Confirm the")
        print("  tree is the version this project targets, then rebuild the index with --update")
        print("  and review the diff: every entry in it is now unverified.")
        return 1

    def show(title, keys, note):
        if not keys:
            return
        print(f"── {title} ({len(keys)}) ──")
        print(f"  {note}")
        for k in keys[:40]:
            print(f"  {k[0]} :: {k[1]}\n      -> {k[2]}:{k[3]}")
        if len(keys) > 40:
            print(f"  ... and {len(keys) - 40} more")
        print()

    show("THE VANILLA SIDE MOVED", moved_vanilla,
         "The cited lines are not the lines this was checked against: the citation drifted.")
    show("OUR SIDE MOVED", moved_ours,
         "The transcription was edited after it was checked against the game. Re-read the citation.")
    show("BOTH SIDES MOVED", moved_both,
         "Neither end is what it was. This entry verifies nothing until someone re-reads it.")
    show("NEW CITATIONS", added, "Not in the index yet. Add them with --update.")
    show("CITATIONS GONE", removed, "In the index, not in the source. Drop them with --update.")

    if broken:
        print(f"{len(broken)} citation(s) do not resolve at all - see the list above. Those are")
        print("bugs in the citations themselves, not drift, and --update will not fix them.")
    if broken or moved_vanilla or moved_ours or moved_both or added or removed:
        return 1
    print(f"all {len(rows)} citations still point at the code they were checked against.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
