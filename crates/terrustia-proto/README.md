# terrustia-proto

The Terraria network wire format, transcribed into Rust: the primitives both ends read and write, the
packet framing, the tile-section coding, and the game's data tables (items, NPCs, projectiles,
recipes, drops, and more).

This crate is deliberately free of I/O. Every packet can be built and parsed in memory, so a wire
round-trip is a unit test rather than a socket. The async server that uses it lives in the
[`terrustia`](https://github.com/bybrooklyn/terrustia) project; this crate is the part any Terraria
tool written in Rust can reuse on its own.

It targets Terraria 1.4.5.8, network protocol 326.

## Install

```toml
[dependencies]
terrustia-proto = "0.0.1"
```

## Example

Build a packet and read it back:

```rust
use terrustia_proto::{PacketReader, PacketWriter};

// A frame is [u16 length][u8 message id][payload]; the length counts the whole frame.
let mut w = PacketWriter::new(4); // message id 4
w.u8(1).string("world").i32(4200);
let frame = w.finish().unwrap();

// Skip the 3-byte header (length + id) to read the body back.
let mut r = PacketReader::new(&frame[3..]);
assert_eq!(r.u8().unwrap(), 1);
assert_eq!(r.string().unwrap(), "world");
assert_eq!(r.i32().unwrap(), 4200);
```

Every integer is little-endian, strings are 7-bit-length-prefixed UTF-8, and the reader's numeric and
string methods mirror the writer's one for one, matching how the game drives
`System.IO.BinaryReader` and `BinaryWriter`.

## What is inside

- Wire primitives: `Writer`, `PacketWriter` and `PacketReader`, plus the `Tile`, `Liquid` and
  section types the world stream is built from.
- The tile-section coding the world-data packets use.
- Transcribed game data tables: items, NPCs and their parameters, projectiles, recipes, drop tables,
  buffs, banners and more, generated from the game's own source and checked in so a build needs
  nothing but Rust.

## Status

Version 0.0.1, and part of a server that is still being built, so the API may still shift. It is
pinned to one game version at a time rather than trying to span several.

## Licence

MIT. The `terrustia` server that uses this crate is AGPL, but this crate is not: it is a description
of a wire format, and that should be free for anyone to build on. See the `LICENSE` file beside this
manifest.
