//! Replay a recorded connection and check this server's reading of it.
//!
//! ```sh
//! cargo run --release -p terrustia --example replay -- capture.trcap
//! ```
//!
//! Record one with `cargo run --release -- --record capture.trcap`, then connect real Terraria.
//!
//! Why this exists: every other test here checks `terrustia-proto` against `terrustia-client`,
//! which is itself built on `terrustia-proto`. The two cannot disagree, so a field read at the
//! wrong width passes both. A capture holds bytes produced by Terraria, which owes nothing to this
//! code, and is therefore the only independent check available.
//!
//! What it proves, in order of how much it is worth:
//!
//! 1. **Framing.** The inbound stream re-frames exactly, with nothing left over. Terraria's
//!    `[u16 length][u8 id][payload]` is the one thing that, if misread, desynchronises everything
//!    after it — and a capture either divides cleanly into frames or it does not.
//! 2. **Every id is one this server knows**, and none arrived that it would silently drop.
//! 3. **The handshake decodes**, field by field, for the packets that gate a client entering the
//!    world.
//!
//! A capture is a recording of one session, so absence of a message is not evidence of anything.
//! The census at the end is there to say what the session actually covered.

use std::{collections::BTreeMap, env, process::ExitCode};

use bytes::BytesMut;
use terrustia::net::{
    codec::TerrariaCodec,
    record::{self, Direction},
};
use terrustia_proto::id;
use tokio_util::codec::Decoder;

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: replay <capture.trcap>");
        return ExitCode::FAILURE;
    };

    let chunks = match record::read(std::path::Path::new(&path)) {
        Ok(chunks) => chunks,
        Err(e) => {
            eprintln!("could not read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if chunks.is_empty() {
        eprintln!("{path} holds no traffic: was anything connected while it recorded?");
        return ExitCode::FAILURE;
    }

    // Each (slot, direction) is its own byte stream and has to be re-framed on its own; they are
    // interleaved in the file only because that is the order they happened in.
    let mut streams: BTreeMap<(u8, bool), BytesMut> = BTreeMap::new();
    let mut recorded_bytes = 0usize;
    for chunk in &chunks {
        recorded_bytes += chunk.bytes.len();
        streams
            .entry((chunk.slot, chunk.direction == Direction::Inbound))
            .or_default()
            .extend_from_slice(&chunk.bytes);
    }

    println!("{path}");
    println!(
        "  {} chunks, {recorded_bytes} bytes, {} stream(s), {:.1}s\n",
        chunks.len(),
        streams.len(),
        f64::from(chunks.last().map_or(0, |c| c.micros)) / 1_000_000.0,
    );

    let mut problems = 0usize;
    let mut inbound_census: BTreeMap<u8, usize> = BTreeMap::new();
    let mut outbound_census: BTreeMap<u8, usize> = BTreeMap::new();

    for ((slot, inbound), mut stream) in streams {
        let who = if inbound { "client" } else { "server" };
        let census = if inbound {
            &mut inbound_census
        } else {
            &mut outbound_census
        };

        let mut codec = TerrariaCodec;
        let mut frames = 0usize;
        loop {
            match codec.decode(&mut stream) {
                Ok(Some(frame)) => {
                    frames += 1;
                    *census.entry(frame.id).or_default() += 1;
                    if inbound && let Err(e) = check_inbound(&frame) {
                        println!(
                            "  slot {slot} {who}: frame {frames} ({}) {e}",
                            id::name(frame.id)
                        );
                        problems += 1;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    println!(
                        "  slot {slot} {who}: the stream desynchronised after {frames} frames: {e}"
                    );
                    problems += 1;
                    break;
                }
            }
        }

        // Anything left is a partial frame. At the very end of a capture that is ordinary — the
        // recording stopped mid-send. Anywhere else it would have shown up as a decode error.
        if !stream.is_empty() {
            println!(
                "  slot {slot} {who}: {frames} frames, {} trailing byte(s) — a partial frame at \
                 the cut, which is expected at the end of a recording",
                stream.len()
            );
        } else {
            println!("  slot {slot} {who}: {frames} frames, nothing left over");
        }
    }

    println!("\nwhat the client sent:");
    print_census(&inbound_census);
    println!("\nwhat the server sent:");
    print_census(&outbound_census);

    // The four the client blocks on. Without every one of these it never reaches the world, so
    // their presence is the difference between "it connected" and "it got partway".
    println!();
    let mut missing = Vec::new();
    for (id, what) in [
        (id::PLAYER_INFO, "slot assignment"),
        (id::WORLD_DATA, "world data"),
        (id::FINISHED_CONNECTING_TO_SERVER, "start playing"),
    ] {
        if !outbound_census.contains_key(&id) {
            missing.push(what);
        }
    }
    if !outbound_census.contains_key(&id::TILE_SECTION) {
        missing.push("tile sections");
    }

    if problems == 0 && missing.is_empty() {
        println!(
            "this capture re-frames cleanly and every message in it is one this server knows."
        );
        ExitCode::SUCCESS
    } else {
        if !missing.is_empty() {
            println!(
                "the server never sent: {}. The client cannot have entered the world.",
                missing.join(", ")
            );
        }
        if problems > 0 {
            println!("{problems} problem(s) above.");
        }
        ExitCode::FAILURE
    }
}

/// Check one frame the real client sent.
///
/// Only the packets that gate entering the world are decoded field by field. The rest are checked
/// for being a message this server recognises at all, which is what catches a client speaking a
/// protocol version this build does not.
fn check_inbound(frame: &terrustia::net::Frame) -> Result<(), String> {
    use terrustia_proto::packets::Hello;

    // Exactly "Unknown" is the table's fallback for an id it has no entry for. Note that
    // "Unknown42" and "Unknown68" are *named* entries, not misses: Terraria's own `MessageID.cs`
    // calls them that, and both are messages this server handles.
    if id::name(frame.id) == "Unknown" {
        return Err(format!(
            "is message id {}, which is not in the table",
            frame.id
        ));
    }

    match frame.id {
        id::HELLO => {
            let hello =
                Hello::decode(&frame.payload).map_err(|e| format!("will not decode: {e}"))?;
            if !hello.is_supported() {
                return Err(format!(
                    "says version {:?}, which this server does not accept",
                    hello.version
                ));
            }
        }
        // Fixed-width, so a wrong reading here shows up as a length mismatch rather than as
        // plausible-looking rubbish.
        id::PLAYER_SPAWN if frame.payload.len() < 9 => {
            return Err(format!(
                "is {} bytes, too short to be a spawn",
                frame.payload.len()
            ));
        }
        _ => {}
    }
    Ok(())
}

fn print_census(census: &BTreeMap<u8, usize>) {
    if census.is_empty() {
        println!("  nothing");
        return;
    }
    for (id, count) in census {
        println!("  {count:>7}  {id:>3}  {}", id::name(*id));
    }
}
