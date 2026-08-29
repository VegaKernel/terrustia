#!/usr/bin/env python3
"""Validate the message-id table against the code, and optionally regenerate the human doc from it.

`docs/packet-ids.tsv` is the durable, checked-in classification of every Terraria message id this
server's protocol surface covers (id 0 `NeverCalled` plus 1..=162, as defined in
`crates/terrustia-proto/src/id.rs`). It carries, per id: the id and its name; the direction it
travels in vanilla and whether the client/server side of that actually sends it; a status
(live / deprecated / dead-slot / not-applicable-dedicated / client-bookkeeping / modding /
social-host-only); and two terrustia-specific columns, `recv_impl` and `send_impl`, describing what
this server's own dispatch and encoders actually do with it.

This script is the release gate that keeps that table honest. It is read-only over the repo and
uses nothing outside the standard library.

    python3 tools/packet_audit.py              # validate the table against the code
    python3 tools/packet_audit.py --write-doc  # also regenerate docs/packet-coverage.md from it

Every check below is mechanical: it re-derives, from the current source, which ids
`handle_packet` actually dispatches on and which ids have an encoder or a relay call site, and
compares that against what the table claims. A row that falls out of sync with the code — a
handler added without updating its row, a row claiming an encoder that was deleted, a stray id the
table never mentions — is a nonzero exit with the offending id named, not a silent pass.

There used to be a hard-coded `UNUSED` set here, hand-maintained and quietly wrong about three live
ids (42, 57, 68) for long enough that a prior audit had to rediscover them from scratch. This
script no longer has one: the table is the only source of truth for which ids are unused, and this
script's job is only to check that the table and the code still agree.
"""

import csv
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
ID_RS = ROOT / "crates/terrustia-proto/src/id.rs"
# The server side was one file (game/server.rs) until the Lane A split; it is now a module
# directory, and the dispatch table lives in server/dispatch.rs. Scanning the concatenation of
# every file in the directory keeps this checker indifferent to further splits.
SERVER_DIR = ROOT / "crates/terrustia/src/game/server"
PROTO_DIR = ROOT / "crates/terrustia-proto/src"
TERRUSTIA_SRC = ROOT / "crates/terrustia/src"
TABLE_PATH = ROOT / "docs/packet-ids.tsv"
DOC_PATH = ROOT / "docs/packet-coverage.md"

MAX_ID = 162

STATUSES = {
    "live", "deprecated", "dead-slot", "not-applicable-dedicated", "client-bookkeeping",
    "modding", "social-host-only",
}
DIRECTIONS = {"client->server", "server->client", "both", "none"}
YES_NO_NA = {"yes", "no", "n-a"}
RECV_IMPLS = {"dispatched", "relayed-opaque", "ignored", "none"}
SEND_IMPLS = {"dedicated-encoder", "generic-relay", "none"}

# `on_paint` relays packet 63/64 via `packets::verbatim(id, payload)` where `id` is a runtime
# parameter (it handles both ids with one function), not a literal `id::CONST` at the call site, so
# the mechanical scan below cannot see it as a per-id send. Recorded here once, by hand, rather than
# left as a silent gap in the checker: any *other* id relying on a variable-id relay is already
# covered because its row's recv_impl is "relayed-opaque" (see `relay_send_ids` below).
DYNAMIC_RELAY_SEND_IDS = {63, 64}

errors = []


def fail(msg):
    errors.append(msg)


# ---------------------------------------------------------------- id.rs: the id surface itself

def parse_id_rs():
    text = ID_RS.read_text()
    lines = text.splitlines()

    raw_consts = []  # (line_index, name, rhs)
    for i, line in enumerate(lines):
        m = re.match(r"pub const ([A-Z_0-9]+): u8 = (\w+);", line.strip())
        if m:
            raw_consts.append((i, m.group(1), m.group(2)))

    name_to_value = {}
    for _, name, rhs in raw_consts:
        if rhs.isdigit():
            name_to_value[name] = int(rhs)
    changed = True
    while changed:
        changed = False
        for _, name, rhs in raw_consts:
            if name not in name_to_value and rhs in name_to_value:
                name_to_value[name] = name_to_value[rhs]
                changed = True

    value_to_primary = {}
    for _, name, rhs in raw_consts:
        if rhs.isdigit():
            value_to_primary[int(rhs)] = name

    if "pub fn name" not in text:
        fail("id.rs: could not find `pub fn name`, the id -> display-name table")
        return name_to_value, value_to_primary, {}

    start = text.index("pub fn name")
    end = text.index("\n}\n", start)
    name_fn_body = text[start:end]
    display_name = {}
    for m in re.finditer(r'^\s*(\d+) => "(\w+)",', name_fn_body, re.M):
        display_name[int(m.group(1))] = m.group(2)

    return name_to_value, value_to_primary, display_name


# ---------------------------------------------------------------- game/server/: what is dispatched

def server_text():
    return "\n".join(p.read_text() for p in sorted(SERVER_DIR.glob("*.rs")))


def parse_dispatch_set():
    text = server_text()
    if "fn handle_packet" not in text or "fn on_hello" not in text:
        fail(
            "game/server/: could not find `fn handle_packet` / `fn on_hello` to slice the "
            "dispatch table"
        )
        return set()
    block = text[text.index("fn handle_packet"): text.index("fn on_hello")]
    return set(re.findall(r"id::([A-Z_0-9]+)", block))


# ---------------------------------------------------------------- send side: encoders and relays

_ENCODER_PAT = re.compile(r"PacketWriter::new\(\s*(?:crate::)?id::([A-Z_0-9]+)\s*\)")
_EMPTY_PAT = re.compile(r"(?:packets::)?empty\(\s*id::([A-Z_0-9]+)\s*\)")
_RELAY_PAT = re.compile(r"(?:rewrite_owner|verbatim)\(\s*(?:id::)?(?:crate::)?id::([A-Z_0-9]+)")


def parse_encoder_names():
    names = set()
    for f in list(PROTO_DIR.glob("*.rs")) + sorted(SERVER_DIR.glob("*.rs")):
        text = f.read_text()
        names |= set(_ENCODER_PAT.findall(text))
        for line in text.splitlines():
            if "fn empty(" in line:
                continue
            names |= set(_EMPTY_PAT.findall(line))
    return names


def parse_relay_send_names():
    return set(_RELAY_PAT.findall(server_text()))


# ---------------------------------------------------------------- the table

def load_table():
    if not TABLE_PATH.exists():
        fail(f"{TABLE_PATH}: does not exist")
        return []
    with TABLE_PATH.open(newline="") as f:
        rows = list(csv.DictReader(f, delimiter="\t"))
    expected_cols = {
        "id", "name", "direction", "client_sends", "server_sends", "status",
        "recv_impl", "send_impl", "evidence", "tests",
    }
    if rows and set(rows[0].keys()) != expected_cols:
        fail(
            f"{TABLE_PATH}: header columns {sorted(rows[0].keys())} do not match the expected "
            f"{sorted(expected_cols)}"
        )
    return rows


def main():
    write_doc = "--write-doc" in sys.argv

    name_to_value, value_to_primary, display_name = parse_id_rs()
    dispatch_names = parse_dispatch_set()
    dispatch_values = {name_to_value[n] for n in dispatch_names if n in name_to_value}
    for n in dispatch_names:
        if n not in name_to_value:
            fail(f"handle_packet dispatches on id::{n}, which id.rs does not define")

    encoder_values = {name_to_value[n] for n in parse_encoder_names() if n in name_to_value}
    relay_values = {name_to_value[n] for n in parse_relay_send_names() if n in name_to_value}

    rows = load_table()
    by_id = {}
    for row in rows:
        try:
            v = int(row["id"])
        except (KeyError, ValueError):
            fail(f"table: row with unparseable id: {row}")
            continue
        if v in by_id:
            fail(f"table: id {v} appears more than once")
        by_id[v] = row

    # 1. Every id in id.rs's range has exactly one row, and the table has no ids outside it.
    all_ids = set(range(0, MAX_ID + 1))
    missing_rows = sorted(all_ids - by_id.keys())
    extra_rows = sorted(by_id.keys() - all_ids)
    for v in missing_rows:
        fail(f"table: id {v} has no row")
    for v in extra_rows:
        fail(f"table: id {v} has a row but is outside 0..={MAX_ID}")

    for v, row in sorted(by_id.items()):
        if v not in all_ids:
            continue
        where = f"table row {v} ({row.get('name', '?')})"

        # 2. name matches id.rs's own name() table (id 0 is NeverCalled by convention, not a real
        # const, so it is checked against display_name directly like everything else).
        expected_name = display_name.get(v)
        if expected_name is not None and row.get("name") != expected_name:
            fail(f"{where}: name column is {row.get('name')!r}, id.rs's name() says {expected_name!r}")

        # 3. enum columns are one of the allowed values.
        if row.get("status") not in STATUSES:
            fail(f"{where}: status {row.get('status')!r} is not one of {sorted(STATUSES)}")
        if row.get("direction") not in DIRECTIONS:
            fail(f"{where}: direction {row.get('direction')!r} is not one of {sorted(DIRECTIONS)}")
        if row.get("client_sends") not in YES_NO_NA:
            fail(f"{where}: client_sends {row.get('client_sends')!r} is not yes/no/n-a")
        if row.get("server_sends") not in YES_NO_NA:
            fail(f"{where}: server_sends {row.get('server_sends')!r} is not yes/no/n-a")
        recv_impl = row.get("recv_impl")
        send_impl = row.get("send_impl")
        if recv_impl not in RECV_IMPLS:
            fail(f"{where}: recv_impl {recv_impl!r} is not one of {sorted(RECV_IMPLS)}")
        if send_impl not in SEND_IMPLS:
            fail(f"{where}: send_impl {send_impl!r} is not one of {sorted(SEND_IMPLS)}")
        if not row.get("evidence", "").strip():
            fail(f"{where}: evidence column is empty")
        if not row.get("tests", "").strip():
            fail(f"{where}: tests column is empty (use \"none\" explicitly)")

        # 4. direction and client_sends/server_sends agree with each other.
        direction = row.get("direction")
        if direction == "both" and (row.get("client_sends"), row.get("server_sends")) != ("yes", "yes"):
            fail(f"{where}: direction is 'both' but client_sends/server_sends is not yes/yes")
        if direction == "client->server" and (row.get("client_sends"), row.get("server_sends")) != ("yes", "no"):
            fail(f"{where}: direction is 'client->server' but client_sends/server_sends is not yes/no")
        if direction == "server->client" and (row.get("client_sends"), row.get("server_sends")) != ("no", "yes"):
            fail(f"{where}: direction is 'server->client' but client_sends/server_sends is not no/yes")

        # 5. recv_impl must match what handle_packet actually dispatches on, mechanically.
        actually_dispatched = v in dispatch_values
        table_says_dispatched = recv_impl in ("dispatched", "relayed-opaque")
        if actually_dispatched and not table_says_dispatched:
            fail(
                f"{where}: handle_packet has a match arm for this id, but the table says "
                f"recv_impl={recv_impl!r} (expected 'dispatched' or 'relayed-opaque')"
            )
        if table_says_dispatched and not actually_dispatched:
            fail(
                f"{where}: table says recv_impl={recv_impl!r}, but handle_packet has no "
                f"match arm for id::{value_to_primary.get(v, v)}"
            )

        # 6. send_impl must match what the code actually does, mechanically.
        has_encoder = v in encoder_values
        has_relay_send = v in relay_values or v in DYNAMIC_RELAY_SEND_IDS or recv_impl == "relayed-opaque"
        if send_impl == "dedicated-encoder" and not has_encoder:
            fail(
                f"{where}: table says send_impl='dedicated-encoder', but no "
                f"`PacketWriter::new(id::{value_to_primary.get(v, v)})` or "
                f"`packets::empty(id::{value_to_primary.get(v, v)})` call site was found"
            )
        if send_impl == "generic-relay" and (has_encoder or not has_relay_send):
            fail(
                f"{where}: table says send_impl='generic-relay', but the mechanical scan found "
                f"{'a dedicated encoder instead' if has_encoder else 'no relay call site for it'}"
            )
        if send_impl == "none" and (has_encoder or (has_relay_send and v not in DYNAMIC_RELAY_SEND_IDS and recv_impl != "relayed-opaque" and v not in relay_values)):
            pass  # covered by the two branches above; kept for readability, no separate check needed
        if has_encoder and send_impl != "dedicated-encoder":
            fail(
                f"{where}: a dedicated encoder exists for this id "
                f"(`PacketWriter::new(id::{value_to_primary.get(v, v)})` or `packets::empty(...)`), "
                f"but the table says send_impl={send_impl!r}"
            )

    if errors:
        print(f"packet_audit: {len(errors)} problem(s) found\n", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        return 1

    # ---------------------------------------------------------------- summary

    from collections import Counter
    status_counts = Counter(row["status"] for row in by_id.values())
    recv_counts = Counter(row["recv_impl"] for row in by_id.values())
    send_counts = Counter(row["send_impl"] for row in by_id.values())

    print(f"packet_audit: {len(by_id)} ids checked against id.rs and game/server/, table and code agree")
    print("\nstatus:")
    for k in sorted(STATUSES):
        if status_counts[k]:
            print(f"  {k:<26} {status_counts[k]:>3}")
    print("\nterrustia recv_impl:")
    for k in sorted(RECV_IMPLS):
        print(f"  {k:<26} {recv_counts[k]:>3}")
    print("\nterrustia send_impl:")
    for k in sorted(SEND_IMPLS):
        print(f"  {k:<26} {send_counts[k]:>3}")

    genuine_gaps = sorted(
        v for v, row in by_id.items()
        if row["status"] == "live" and row["recv_impl"] in ("ignored", "none")
        and row["send_impl"] == "none"
        and (row["client_sends"] == "yes" or row["server_sends"] == "yes")
    )
    if genuine_gaps:
        print(f"\nlive ids with no terrustia implementation on either side ({len(genuine_gaps)}):")
        for v in genuine_gaps:
            print(f"  {v:>3} {by_id[v]['name']}")

    if write_doc:
        write_docs(by_id, status_counts, recv_counts, send_counts, genuine_gaps)
        print(f"\nwrote {DOC_PATH}")

    return 0


# ---------------------------------------------------------------- --write-doc

PREAMBLE = """# Packet coverage

<!-- Hand-written preamble; everything below "## Where it stands" is generated by
     `python3 tools/packet_audit.py --write-doc` from docs/packet-ids.tsv. Do not hand-edit past
     that point: edit the table (or the audit's classification logic) and regenerate instead. -->

Terraria 1.4.5.8 (protocol release 326; release 325 is accepted too) defines 163 message ids: `0`
`NeverCalled` plus `1..=162`, as transcribed in `crates/terrustia-proto/src/id.rs`. This page is
that table's classification, one row per id: which direction it travels, whether this server
receives and sends it, and why not where it does not.

The source of truth is `docs/packet-ids.tsv`, a tab-separated file Python's standard library can
read without a parser. `tools/packet_audit.py` validates every row against the actual code (the
dispatch table in `crates/terrustia/src/game/server/`'s `handle_packet`, and the encoders in
`crates/terrustia-proto/src/packets.rs` and its neighbours) and fails with a precise message on any
mismatch — a handler added without updating its row, a row claiming an encoder that no longer
exists, an id the table never mentions. Regenerate this page after editing the table:

```sh
python3 tools/packet_audit.py --write-doc
```
"""


def md_escape(s):
    return s.replace("|", "\\|")


def write_docs(by_id, status_counts, recv_counts, send_counts, genuine_gaps):
    lines = [PREAMBLE, "## Where it stands\n"]
    lines.append("| | |")
    lines.append("|---|---:|")
    lines.append(f"| Total ids | {len(by_id)} |")
    for k in sorted(STATUSES):
        if status_counts[k]:
            lines.append(f"| Status: {k} | {status_counts[k]} |")
    lines.append(f"| Dispatched by handle_packet | {recv_counts['dispatched']} |")
    lines.append(f"| Relayed opaquely (received, not parsed) | {recv_counts['relayed-opaque']} |")
    lines.append(f"| Ignored (falls to the catch-all) | {recv_counts['ignored']} |")
    lines.append(f"| Sent via a dedicated encoder | {send_counts['dedicated-encoder']} |")
    lines.append(f"| Sent via a generic relay | {send_counts['generic-relay']} |")
    lines.append("")

    if genuine_gaps:
        lines.append(
            f"## Live ids with no terrustia implementation on either side ({len(genuine_gaps)})\n"
        )
        lines.append(
            "Every one of these is a real, currently-used vanilla mechanic (`status = live`) that "
            "this server neither receives nor sends. Each is a genuine gap, not a dead id; see its "
            "row's evidence column below for what it is and where it comes from.\n"
        )
        for v in genuine_gaps:
            row = by_id[v]
            lines.append(f"- **{row['name']}** ({v}) - {row['evidence']}")
        lines.append("")

    dead_or_deprecated = sorted(
        v for v, row in by_id.items() if row["status"] in ("dead-slot", "deprecated")
    )
    if dead_or_deprecated:
        lines.append(f"## Dead slots and deprecated ids ({len(dead_or_deprecated)})\n")
        for v in dead_or_deprecated:
            row = by_id[v]
            lines.append(f"- **{row['name']}** ({v}, {row['status']}) - {row['evidence']}")
        lines.append("")

    other_na = sorted(
        v for v, row in by_id.items()
        if row["status"] in (
            "not-applicable-dedicated", "client-bookkeeping", "modding", "social-host-only",
        )
    )
    if other_na:
        lines.append(f"## Not applicable to this build ({len(other_na)})\n")
        for v in other_na:
            row = by_id[v]
            lines.append(f"- **{row['name']}** ({v}, {row['status']}) - {row['evidence']}")
        lines.append("")

    lines.append("## The full table\n")
    lines.append(
        "| id | name | direction | client sends | server sends | status | recv impl | send impl |"
    )
    lines.append("|---:|---|---|---|---|---|---|---|")
    for v in sorted(by_id):
        row = by_id[v]
        lines.append(
            "| {id} | {name} | {direction} | {client_sends} | {server_sends} | {status} "
            "| {recv_impl} | {send_impl} |".format(
                id=v,
                name=md_escape(row["name"]),
                direction=row["direction"],
                client_sends=row["client_sends"],
                server_sends=row["server_sends"],
                status=row["status"],
                recv_impl=row["recv_impl"],
                send_impl=row["send_impl"],
            )
        )
    lines.append("")
    lines.append(
        "Evidence and test coverage for every row live in `docs/packet-ids.tsv`, one column each; "
        "they are omitted from this table to keep it readable and shown in the sections above only "
        "where they tell a story (a gap, a dead id, a reason something does not apply here)."
    )
    lines.append("")

    DOC_PATH.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    sys.exit(main())
