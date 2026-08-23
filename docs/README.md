# Documentation

Notes on how this server works and why it works that way. Each file covers one area and is
written to be read before touching the code it describes.

| File | What it covers |
|---|---|
| [protocol-notes.md](protocol-notes.md) | The wire format: frame layout, the handshake, and the packets whose shape is easy to get wrong |
| [packet-coverage.md](packet-coverage.md) | Which of Terraria's 148 live messages this server handles, which it does not, and why |
| [buffs.md](buffs.md) | Debuffs on NPCs: the twenty slots, damage-over-time, and why armour penetration is the client's job |
| [tile-entities.md](tile-entities.md) | The furniture that remembers something — pylons, item frames, mannequins — and its two serialised forms |
| [teleports.md](teleports.md) | The five items that ask the server to move a player, and how a safe landing spot is found |
| [wiring.md](wiring.md) | Circuits, the Grand Design's L-shaped path, and the limits this server adds that the game does not |
| [performance.md](performance.md) | The tick budget, where the time goes, and the autosave stall that hid behind a wrong guess |
| [world-file.md](world-file.md) | The `.wld` format as this server reads and writes it, including what is preserved verbatim |
| [worldgen-parity.md](worldgen-parity.md) | The plan for vanilla-identical world generation, and the oracle that steers it |
| [generated-tables.md](generated-tables.md) | Which source files are generated, from what, and how to regenerate them |

## The rule the whole codebase follows

**Per-type variation lives in generated tables. Hand-written modules hold algorithms only.**

There are 697 NPC types, 754 tiles, 401 buffs and several thousand items. Any rule that differs
per type is data, and data belongs in a file generated from the game's own tables — because a
hand-written match over 697 cases is wrong the moment the game changes, and wrong invisibly.

The generators live in [`tools/`](../tools) and each one names its source. See
[generated-tables.md](generated-tables.md).

## How to check something is right

In rough order of how much it proves:

```sh
cargo test                  # unit and integration tests
cargo clippy --all-targets  # kept at zero warnings
cargo run --release -p terrustia --example bestiary -- 127.0.0.1:7777    # every NPC type, live
cargo run --release -p terrustia --example fuzz -- 127.0.0.1:7777        # malformed packets
cargo run --release -p terrustia --example crowd -- 127.0.0.1:7777       # many players at once
cargo run --release -p terrustia --example stress -- 127.0.0.1:7777      # the tick budget
cargo run --release -p terrustia --example roundtrip_wld -- in.wld out.wld
cargo run --release -p terrustia --example genparity -- reference.wld
python3 tools/packet_audit.py <decompiled-tree>   # what is still unhandled
```

A tick has 16,666 µs to spend. Anything approaching that is a bug, not a tuning problem.
