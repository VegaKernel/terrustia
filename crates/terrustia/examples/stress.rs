//! Drive a running server hard and report what it costs it.
//!
//! `load` measures traffic; this measures the *tick*. It fills the world with everything at once
//! — a full roster of enemies, a crowd of projectiles, liquid in motion, wired traps firing on
//! timers — and then reads the server's own per-phase tick report back out of its log.
//!
//! ```sh
//! TERRUSTIA_LOG=terrustia=debug cargo run --release -p terrustia \
//!     --example stress -- 127.0.0.1:7777 60
//! ```
//!
//! Run the server with `TERRUSTIA_LOG=terrustia=debug` so its tick-window lines are emitted, and
//! watch the log while this runs.

use std::time::{Duration, Instant};

use terrustia_client::{Client, Event};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let addr: std::net::SocketAddr = args
        .next()
        .unwrap_or_else(|| "127.0.0.1:7777".into())
        .parse()
        .expect("host:port");
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);

    let mut client = Client::join(addr, "stress").await.expect("handshake");
    client.set_timeout(Duration::from_secs(20));
    println!("piling everything on {addr} for {seconds}s");

    // Stand somewhere with room, so what is spawned stays awake around us.
    let spawn = client.world().spawn;
    let (x, y) = (f32::from(spawn.0) * 16.0, f32::from(spawn.1) * 16.0);
    client.move_to(x, y - 64.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // A full roster, several times over: this is far past what a real world holds at once, which
    // is the point — the tick budget should hold with room to spare.
    let mut spawned = 0usize;
    for round in 0..3 {
        for npc_type in 1..=690u16 {
            if terrustia_proto::npc_data::npc_stats(npc_type).is_none() {
                continue;
            }
            if client.say(&format!("/spawn {npc_type}")).await.is_err() {
                break;
            }
            spawned += 1;
            if spawned.is_multiple_of(40) {
                tokio::time::sleep(Duration::from_millis(20)).await;
                client.move_to(x, y - 64.0).await.ok();
            }
        }
        println!("  round {}: {spawned} spawn requests sent", round + 1);
    }

    // Then sit in it, keeping the connection alive and counting what comes back.
    let started = Instant::now();
    let mut frames = 0u64;
    let mut npc_syncs = 0u64;
    let mut projectiles = 0u64;
    let mut last_move = Instant::now();
    while started.elapsed() < Duration::from_secs(seconds) {
        if last_move.elapsed() > Duration::from_millis(200) {
            client.move_to(x, y - 64.0).await.ok();
            last_move = Instant::now();
        }
        match client.next_event().await {
            Ok(event) => {
                frames += 1;
                match event {
                    Event::NpcSynced(_) => npc_syncs += 1,
                    Event::ProjectileSynced(_) => projectiles += 1,
                    _ => {}
                }
            }
            Err(_) => break,
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    println!();
    println!("held {elapsed:.1}s under load");
    println!(
        "  {frames} frames in          ({:.0}/s)",
        frames as f64 / elapsed
    );
    println!(
        "  {npc_syncs} npc syncs       ({:.0}/s)",
        npc_syncs as f64 / elapsed
    );
    println!(
        "  {projectiles} projectile syncs ({:.0}/s)",
        projectiles as f64 / elapsed
    );
    println!();
    println!("the server's own tick costs are in its log: look for `tick window`");
}
