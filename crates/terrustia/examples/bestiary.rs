//! Spawn every NPC the game has on a running server and check each one arrives.
//!
//! This is the roster's end-to-end proof. The unit tests drive the routines directly and the
//! integration tests drive a handful through the protocol; this drives *all* of them through a
//! real socket, so a type that cannot be spawned, cannot be synced, or panics on its first tick
//! shows up as a name in a list rather than as a crash months later.
//!
//! ```sh
//! cargo run --release -- --world some.wld &
//! cargo run --release --example bestiary
//! ```

use std::{collections::BTreeSet, env, process::ExitCode, time::Duration};

use terrustia_client::{Client, Event};
use terrustia_proto::npc_data::{NPC_COUNT, npc_stats};

#[tokio::main]
async fn main() -> ExitCode {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7777".to_string());
    let Ok(addr) = addr.parse() else {
        eprintln!("usage: bestiary [host:port]");
        return ExitCode::FAILURE;
    };

    let mut client = match Client::join(addr, "bestiary").await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not join {addr}: {e}");
            return ExitCode::FAILURE;
        }
    };
    client.set_timeout(Duration::from_secs(5));

    // Everything the game defines, in order.
    let roster: Vec<(u16, &'static str)> = (0..NPC_COUNT)
        .filter_map(|npc_type| npc_stats(npc_type).map(|s| (npc_type, s.name)))
        .collect();
    println!("spawning {} types on {addr}", roster.len());

    let mut arrived = 0usize;
    let mut missing: Vec<(u16, &str)> = Vec::new();
    let mut styles_seen = BTreeSet::new();

    for (npc_type, name) in &roster {
        // Clear the field first, so a previous boss's minions cannot be mistaken for this one.
        client.say("/butcher").await.ok();
        let _ = client
            .try_wait_for(
                "the butcher",
                |e| matches!(e, Event::Chat { .. }),
                Duration::from_millis(200),
            )
            .await;

        client.say(&format!("/spawn {npc_type}")).await.ok();
        let seen = client
            .try_wait_for(
                name,
                |e| matches!(e, Event::NpcSynced(n) if n.net_id == *npc_type as i16),
                Duration::from_millis(700),
            )
            .await;
        if seen.is_some() {
            arrived += 1;
            if let Some(stats) = npc_stats(*npc_type) {
                styles_seen.insert(stats.ai_style);
            }
        } else {
            missing.push((*npc_type, name));
        }
    }

    println!();
    println!("{arrived} of {} arrived and synced", roster.len());
    println!("{} distinct AI styles exercised", styles_seen.len());
    if missing.is_empty() {
        println!("every type in the build spawned");
        return ExitCode::SUCCESS;
    }
    println!("{} did not arrive:", missing.len());
    for (npc_type, name) in &missing {
        println!("  {npc_type:>3} {name}");
    }
    ExitCode::FAILURE
}
