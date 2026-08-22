//! Spawn every pre-hardmode NPC on a live server and check each one arrives and behaves.
//!
//! This is the end-to-end form of the coverage test: it proves the roster is reachable through
//! the real protocol, not just that a table exists.
//!
//! ```text
//! cargo run --release --example bestiary -- 127.0.0.1:7777
//! ```

use std::{collections::HashMap, env, process::ExitCode, time::Duration};

use terrustia_client::{Client, Event};
use terrustia_proto::{npc_data::npc_stats, prehardmode::PRE_HARDMODE};

#[tokio::main]
async fn main() -> ExitCode {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7777".to_string())
        .parse()
        .expect("a socket address");

    let mut client = match Client::join(addr, "bestiary").await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("join failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    client.set_timeout(Duration::from_secs(4));
    let (sx, sy) = client.world().spawn;
    let _ = client
        .move_to(f32::from(sx) * 16.0, f32::from(sy) * 16.0)
        .await;
    println!(
        "joined \"{}\"; testing {} types\n",
        client.world().name,
        PRE_HARDMODE.len()
    );

    let mut arrived = 0usize;
    let mut missing: Vec<&str> = Vec::new();
    let mut moved = 0usize;

    for npc_type in PRE_HARDMODE {
        let name = npc_stats(npc_type).map(|s| s.name).unwrap_or("?");

        // Clear the field first so each type is measured on its own.
        let _ = client.say("/butcher").await;
        let _ = client.say(&format!("/spawn {npc_type}")).await;

        let mut first: Option<(u8, (f32, f32))> = None;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(900);
        while tokio::time::Instant::now() < deadline {
            match client.next_event().await {
                Ok(Event::NpcSynced(n)) if n.npc_type() == npc_type && n.life != 0 => {
                    first = Some((n.index, n.position));
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }

        let Some((index, start)) = first else {
            missing.push(name);
            continue;
        };
        arrived += 1;

        // Watch briefly to see whether it does anything at all.
        let mut seen_move = false;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(700);
        while tokio::time::Instant::now() < deadline && !seen_move {
            let _ = client
                .move_to(f32::from(sx) * 16.0, f32::from(sy) * 16.0)
                .await;
            if let Ok(Event::NpcSynced(n)) = client.next_event().await
                && n.index == index
                && ((n.position.0 - start.0).abs() > 1.0 || (n.position.1 - start.1).abs() > 1.0)
            {
                seen_move = true;
            }
        }
        if seen_move {
            moved += 1;
        }
    }

    let _ = client.say("/butcher").await;

    println!("spawned and synced : {arrived} of {}", PRE_HARDMODE.len());
    println!("observed moving    : {moved}");
    if !missing.is_empty() {
        println!(
            "never arrived      : {} -> {:?}",
            missing.len(),
            &missing[..missing.len().min(20)]
        );
    }

    let mut by_style: HashMap<i32, usize> = HashMap::new();
    for t in PRE_HARDMODE {
        if let Some(s) = npc_stats(t) {
            *by_style.entry(s.ai_style).or_default() += 1;
        }
    }
    let mut styles: Vec<_> = by_style.into_iter().collect();
    styles.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!(
        "ai styles in roster: {}",
        styles
            .iter()
            .take(8)
            .map(|(s, n)| format!("{s}x{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    if missing.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
