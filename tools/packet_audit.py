#!/usr/bin/env python3
"""Report which of Terraria's messages this server neither sends nor receives.

Run with the path to the decompiled game tree to also classify each gap by whether the *client*
ever sends it — a message only the server sends is not a gap on the receiving side.

    python3 tools/packet_audit.py [path-to-decompiled-tree]
"""
import re
import sys
import glob
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent

ids = {}
for line in (ROOT / "crates/terrustia-proto/src/id.rs").read_text().splitlines():
    m = re.match(r"pub const ([A-Z_0-9]+): u8 = (\d+);", line.strip())
    if m:
        ids[m.group(1)] = int(m.group(2))

server = (ROOT / "crates/terrustia/src/game/server.rs").read_text()
dispatch = server[server.index("fn handle_packet") : server.index("fn on_hello")]
inbound = set(re.findall(r"id::([A-Z_0-9]+)", dispatch))

referenced = set()
for f in glob.glob(str(ROOT / "crates/**/*.rs"), recursive=True):
    if f.endswith("id.rs"):
        continue
    referenced |= set(re.findall(r"id::([A-Z_0-9]+)", pathlib.Path(f).read_text()))

# Which message types the client sends, judged by whether any SendData call site for that type
# sits under a `netMode == 1` guard.
client_sends = set()
if len(sys.argv) > 1:
    tree = sys.argv[1]
    out = subprocess.run(
        ["grep", "-rn", "-E", r"SendData\(\s*[0-9]+", tree], capture_output=True, text=True
    ).stdout
    cache = {}
    for line in out.splitlines():
        m = re.match(r"(.*?):(\d+):(.*)", line)
        if not m:
            continue
        path, ln, txt = m.group(1), int(m.group(2)), m.group(3)
        n = int(re.search(r"SendData\(\s*(\d+)", txt).group(1))
        if path not in cache:
            try:
                cache[path] = pathlib.Path(path).read_text(errors="replace").splitlines()
            except OSError:
                cache[path] = []
        ctx = "\n".join(cache[path][max(0, ln - 12) : ln + 2])
        if re.search(r"netMode\s*==\s*1|netMode\s*!=\s*2", ctx):
            client_sends.add(n)

UNUSED = {
    "UNKNOWN15", "UNUSED_MELEE_STRIKE", "UNUSED25", "UNUSED26", "UNKNOWN42", "UNKNOWN44",
    "UNKNOWN57", "UNKNOWN60", "UNKNOWN66", "UNKNOWN67", "UNKNOWN68", "UNUSED83",
    "SYNC_ITEMS_WITH_SHIMMER_DEPRECATED", "SYNC_ITEM_CANNOT_BE_TAKEN_BY_ENEMIES_DEPRECATED",
}

missing = sorted((v, k) for k, v in ids.items() if k not in referenced and k not in UNUSED)
live = len(ids) - len(UNUSED)
print(f"messages: {live} live ({len(UNUSED)} unused/deprecated)")
print(f"  referenced anywhere: {len(referenced & set(ids))}")
print(f"  dispatched inbound:  {len(inbound)}")
print(f"  never touched:       {len(missing)}")
if client_sends:
    hot = [(v, k) for v, k in missing if v in client_sends]
    cold = [(v, k) for v, k in missing if v not in client_sends]
    print(f"\nNEVER TOUCHED, AND THE CLIENT SENDS IT ({len(hot)}):")
    for v, k in hot:
        print(f"  {v:>3} {k}")
    print(f"\nNEVER TOUCHED, SERVER-TO-CLIENT ONLY ({len(cold)}):")
    for v, k in cold:
        print(f"  {v:>3} {k}")
else:
    print("\nNEVER TOUCHED:")
    for v, k in missing:
        print(f"  {v:>3} {k}")
