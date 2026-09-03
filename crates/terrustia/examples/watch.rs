//! Join a running server, stand somewhere, and report what the world does around you.
//!
//! The tests drive the server; this watches it. Natural spawning, town NPCs moving in, the clock
//! turning over, weather — all of it only happens over minutes on a real world, which is longer
//! than a test should take and exactly what an operator wants to see before trusting a server.
//!
//! ```sh
//! cargo run --release --example watch -- 127.0.0.1:7777 120
//! ```

use std::{collections::BTreeMap, env, process::ExitCode, time::Duration};

use terrustia_client::{Client, Event};
use terrustia_proto::npc_data::npc_stats;

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:7777".to_string());
    let seconds: u64 = args.next().and_then(|n| n.parse().ok()).unwrap_or(60);
    let Ok(addr) = addr.parse() else {
        eprintln!("usage: watch [host:port] [seconds]");
        return ExitCode::FAILURE;
    };

    let mut client = match Client::join(addr, "watcher").await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not join {addr}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // An optional `x,y` in tiles to stand at instead of spawn: spawning is biome-driven, so
    // checking a biome's pool means standing in that biome.
    let mut stand = None;
    let rest: Vec<String> = args.collect();
    let mut commands: Vec<String> = Vec::new();
    for arg in rest {
        match arg
            .split_once(',')
            .and_then(|(x, y)| Some((x.trim().parse::<i16>().ok()?, y.trim().parse::<i16>().ok()?)))
        {
            Some(at) => stand = Some(at),
            None => commands.push(arg),
        }
    }
    let spawn = stand.unwrap_or(client.world().spawn);
    println!("watching {addr} for {seconds}s from spawn at {spawn:?}");

    // Any further arguments are chat commands to run first, so an operator can set the scene:
    // `watch 127.0.0.1:7777 60 "/time night"`.
    // A fresh character has a hundred life, and an invasion will not begin for a party who have
    // never found a life crystal. A watcher is here to see things happen, so it says otherwise.
    let _ = client.set_life(400, 400).await;

    // Stand where we were asked to before saying anything, so a `/where` reports the real spot
    // rather than where the client happened to start.
    for _ in 0..4 {
        let _ = client
            .move_to(f32::from(spawn.0) * 16.0, f32::from(spawn.1) * 16.0)
            .await;
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    for command in commands {
        // `summon:-4` sends the boss/event packet rather than chat, which is the only way to ask
        // for an event: there is no chat command for one, because the game has no such command.
        if let Some(what) = command.strip_prefix("summon:")
            && let Ok(what) = what.parse::<i16>()
        {
            let mut payload = Vec::new();
            payload.extend_from_slice(&0i16.to_le_bytes());
            payload.extend_from_slice(&what.to_le_bytes());
            let mut frame = Vec::new();
            frame.extend_from_slice(&((payload.len() + 3) as u16).to_le_bytes());
            frame.push(terrustia_proto::id::SPAWN_BOSS_USE_LICENSE_START_EVENT);
            frame.extend_from_slice(&payload);
            println!("  summoning {what}");
            let _ = client.send(&frame).await;
            continue;
        }
        println!("  sending {command}");
        let _ = client.say(&command).await;
    }

    // Counting syncs would count the whole world: NPCs are broadcast to everyone wherever they
    // are, so a bird on the far surface is synced to somebody standing in a cavern. What matters
    // is what appeared *near here*, so each slot is counted once, the first time it is seen, and
    // only if it turned up within a screen or two.
    let mut seen: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut known: std::collections::HashSet<u8> = std::collections::HashSet::new();
    let here = (f32::from(spawn.0) * 16.0, f32::from(spawn.1) * 16.0);
    const NEARBY: f32 = 2000.0;
    let mut chat = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut nudge = tokio::time::interval(Duration::from_millis(500));

    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            _ = nudge.tick() => {
                // Stand still at spawn: spawning happens around a player, so there has to be one.
                let _ = client
                    .move_to(f32::from(spawn.0) * 16.0, f32::from(spawn.1) * 16.0)
                    .await;
            }
            event = client.next_event() => {
                match event {
                    Ok(Event::NpcSynced(npc)) if npc.life > 0 => {
                        let near = (npc.position.0 - here.0).abs() < NEARBY
                            && (npc.position.1 - here.1).abs() < NEARBY;
                        if near && known.insert(npc.index) {
                            let name = npc_stats(npc.npc_type()).map_or("?", |s| s.name);
                            *seen.entry(name).or_default() += 1;
                        }
                    }
                    Ok(Event::Chat { text, .. }) => chat.push(text),
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("connection lost: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
        }
    }

    println!();
    println!("{} distinct types appeared nearby:", seen.len());
    for (name, times) in seen.iter().take(40) {
        println!("  {name:<28} {times}×");
    }
    if !chat.is_empty() {
        println!();
        println!("{} announcements:", chat.len());
        for line in chat.iter().take(20) {
            println!("  {line}");
        }
    }
    ExitCode::SUCCESS
}
