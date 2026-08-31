//! Handshake probe: connects to a Terraria server, drives a minimal client handshake, and dumps
//! the packet sequence it receives.
//!
//! Point it at the real `TerrariaServer` and at this one to compare them:
//!
//! ```text
//! cargo run --example probe -- 127.0.0.1:7778   # vanilla
//! cargo run --example probe -- 127.0.0.1:7777   # terrustia
//! ```
//!
//! Two environment knobs: `PROBE_DUMP_DIR` writes every tile-section payload to that directory, and
//! `PROBE_LINGER=<seconds>` keeps reading past `129 FinishedConnectingToServer` instead of stopping
//! there. `tools/differential.sh` drives both servers through this.

use std::{env, time::Duration};

use bytes::BytesMut;
use terrustia::net::{Frame, TerrariaCodec};
use terrustia_proto::{PacketWriter, id, packets, reader::PacketReader};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use tokio_util::codec::Decoder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7777".to_string());
    let mut stream = TcpStream::connect(&addr).await?;
    stream.set_nodelay(true)?;
    println!("connected to {addr}");

    let mut w = PacketWriter::new(id::HELLO);
    w.string(id::VERSION_STRING);
    stream.write_all(&w.finish()?).await?;

    let mut buf = BytesMut::with_capacity(64 * 1024);
    let mut codec = TerrariaCodec;
    let mut slot = 0u8;
    let mut sections = 0usize;
    let mut requested_tiles = false;
    // Seconds to keep reading *after* packet 129. Zero, the default, stops there: exactly what a
    // client that treats 129 as the end of the handshake would see, and exactly why our own
    // out-of-order 129 was invisible. Anything a server sends after 129 needs the linger to be
    // observed at all, so the differential asks for one.
    let linger = Duration::from_secs(
        env::var("PROBE_LINGER")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    );
    let mut deadline: Option<tokio::time::Instant> = None;

    loop {
        let frame: Frame = loop {
            if let Some(frame) = codec.decode(&mut buf)? {
                break frame;
            }
            let mut wait = Duration::from_secs(5);
            if let Some(at) = deadline {
                let left = at.saturating_duration_since(tokio::time::Instant::now());
                if left.is_zero() {
                    println!("-- linger elapsed; {sections} sections received");
                    return Ok(());
                }
                wait = wait.min(left);
            }
            match timeout(wait, stream.read_buf(&mut buf)).await {
                Ok(Ok(0)) => {
                    println!("-- server closed the connection");
                    return Ok(());
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => return Err(e.into()),
                // Inside the linger a quiet stretch is not the end of the session; the deadline
                // above is what ends it.
                Err(_) if deadline.is_some() => {}
                Err(_) => {
                    println!("-- idle for 5s; stopping. sections received: {sections}");
                    return Ok(());
                }
            }
        };

        match frame.id {
            id::TILE_SECTION => {
                sections += 1;
                // Capture real section payloads so our decoder can be checked against the
                // encoder that actually ships with the game.
                if let Ok(dir) = env::var("PROBE_DUMP_DIR") {
                    let path = format!("{dir}/section-{sections:03}.deflate");
                    if let Err(e) = std::fs::write(&path, &frame.payload) {
                        eprintln!("could not write {path}: {e}");
                    }
                }
                // Every one of them, not the first two: a section is a packet like any other, and
                // a differential that compares two servers' packet order cannot leave a hole in
                // the middle of the sequence where thirty-odd frames used to be.
                println!(
                    "<- {:3} {:<28} {:6} bytes (deflate)",
                    frame.id,
                    id::name(frame.id),
                    frame.payload.len()
                );
                continue;
            }
            id::WORLD_DATA => {
                println!(
                    "<- {:3} {:<28} {:6} bytes",
                    frame.id,
                    id::name(frame.id),
                    frame.payload.len()
                );
                dump_world_data(&frame.payload);
            }
            _ => println!(
                "<- {:3} {:<28} {:6} bytes",
                frame.id,
                id::name(frame.id),
                frame.payload.len()
            ),
        }

        match frame.id {
            id::PLAYER_INFO => {
                slot = frame.payload[0];
                println!("   assigned slot {slot}");
                stream.write_all(&sync_player(slot, "probe")?).await?;
                stream
                    .write_all(
                        &packets::PlayerHealth {
                            player: slot,
                            life: 100,
                            life_max: 100,
                        }
                        .encode()?,
                    )
                    .await?;
                stream
                    .write_all(
                        &packets::PlayerMana {
                            player: slot,
                            mana: 20,
                            mana_max: 20,
                        }
                        .encode()?,
                    )
                    .await?;
                let mut w = PacketWriter::new(id::CLIENT_UUID);
                w.string("probe-uuid");
                stream.write_all(&w.finish()?).await?;
                stream
                    .write_all(&packets::empty(id::REQUEST_WORLD_DATA)?)
                    .await?;
            }
            id::WORLD_DATA if !requested_tiles => {
                requested_tiles = true;
                let mut w = PacketWriter::new(id::SPAWN_TILE_DATA);
                w.i32(-1).i32(-1).u8(0);
                stream.write_all(&w.finish()?).await?;
            }
            id::INITIAL_SPAWN => {
                println!("   ({sections} sections so far) spawning in");
                let spawn = packets::PlayerSpawn {
                    player: slot,
                    spawn_x: -1,
                    spawn_y: -1,
                    respawn_timer: 0,
                    deaths_pve: 0,
                    deaths_pvp: 0,
                    team: 0,
                    context: packets::PlayerSpawn::CONTEXT_SPAWNING_INTO_WORLD,
                };
                stream.write_all(&spawn.encode()?).await?;
            }
            id::FINISHED_CONNECTING_TO_SERVER => {
                println!("== handshake complete; {sections} sections received");
                // A live server keeps streaming world updates forever, so without a linger this
                // is where a probe stops, same as a client would.
                if linger.is_zero() {
                    return Ok(());
                }
                deadline = Some(tokio::time::Instant::now() + linger);
            }
            id::KICK => {
                let mut r = PacketReader::new(&frame.payload);
                if let Ok(text) = terrustia_proto::NetworkText::read(&mut r) {
                    println!("   kicked: {}", text.text);
                }
                return Ok(());
            }
            _ => {}
        }
    }
}

/// Print the leading fields of packet 7 so two servers can be compared field by field.
fn dump_world_data(payload: &[u8]) {
    let mut r = PacketReader::new(payload);
    let fields = [
        ("time", r.i32().map(|v| v.to_string())),
        ("day/moon flags", r.u8().map(|v| format!("{v:#010b}"))),
        ("moonPhase", r.u8().map(|v| v.to_string())),
        ("maxTilesX", r.i16().map(|v| v.to_string())),
        ("maxTilesY", r.i16().map(|v| v.to_string())),
        ("spawnTileX", r.i16().map(|v| v.to_string())),
        ("spawnTileY", r.i16().map(|v| v.to_string())),
        ("worldSurface", r.i16().map(|v| v.to_string())),
        ("rockLayer", r.i16().map(|v| v.to_string())),
        ("worldId", r.i32().map(|v| v.to_string())),
        ("worldName", r.string()),
        ("gameMode", r.u8().map(|v| v.to_string())),
    ];
    for (name, value) in fields {
        match value {
            Ok(v) => println!("     {name:<16} {v}"),
            Err(e) => {
                println!("     {name:<16} <error: {e}>");
                return;
            }
        }
    }
    // Skip guid + genVersion + moonType + backgrounds to reach the trailer.
    println!("     ... payload total {} bytes", payload.len());
}

/// A complete packet 4, in the field order the 1.4.5.7 client uses.
fn sync_player(slot: u8, name: &str) -> terrustia_proto::Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::SYNC_PLAYER);
    w.u8(slot)
        .u8(0) // skin variant
        .u8(1) // voice variant
        .f32(0.0) // voice pitch offset
        .u8(0) // hair
        .string(name)
        .u8(0) // hair dye
        .u16(0) // accessory visibility bitfield
        .u8(0) // hideMisc
        .rgb([215, 90, 55]) // hair
        .rgb([255, 125, 90]) // skin
        .rgb([105, 90, 75]) // eye
        .rgb([175, 165, 140]) // shirt
        .rgb([160, 180, 215]) // undershirt
        .rgb([255, 230, 175]) // pants
        .rgb([160, 105, 60]) // shoes
        .u8(0) // difficulty / extra accessory flags
        .u8(0) // torch flags
        .u8(0); // consumable flags
    w.finish()
}
