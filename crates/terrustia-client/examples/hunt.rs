//! Watch NPCs spawn, then attack the first one that comes into range.
//!
//! ```text
//! cargo run --release --example hunt -- 127.0.0.1:7777
//! ```

use std::{collections::HashMap, env, process::ExitCode, time::Duration};

use terrustia_client::{Client, Event};
use terrustia_proto::npc_data::npc_stats;

#[tokio::main]
async fn main() -> ExitCode {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7777".to_string())
        .parse()
        .expect("a socket address");

    let mut client = match Client::join(addr, "hunter").await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("join failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    client.set_timeout(Duration::from_secs(5));
    let (sx, sy) = client.world().spawn;
    println!("joined \"{}\" at spawn {:?}", client.world().name, (sx, sy));

    // Stand at spawn so enemies come to us.
    let _ = client
        .move_to(f32::from(sx) * 16.0, f32::from(sy) * 16.0)
        .await;

    let mut seen: HashMap<u8, (u16, u8)> = HashMap::new();
    let mut killed = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);

    while tokio::time::Instant::now() < deadline {
        // Keep reporting our position so the server keeps spawning around us.
        let _ = client
            .move_to(f32::from(sx) * 16.0, f32::from(sy) * 16.0)
            .await;

        match client.next_event().await {
            Ok(Event::NpcSynced(npc)) => {
                if npc.life == 0 {
                    if let Some((npc_type, _)) = seen.remove(&npc.index) {
                        let name = npc_stats(npc_type).map(|s| s.name).unwrap_or("?");
                        println!("  npc {} ({name}) died", npc.index);
                    }
                    continue;
                }
                let npc_type = npc.npc_type();
                let name = npc_stats(npc_type).map(|s| s.name).unwrap_or("?");
                if seen.insert(npc.index, (npc_type, npc.generation)).is_none() {
                    let tile = (npc.position.0 / 16.0, npc.position.1 / 16.0);
                    println!(
                        "  saw {name} (type {npc_type}) at tile ({:.0}, {:.0})",
                        tile.0, tile.1
                    );
                }

                // Hit anything within reach.
                let dx = npc.position.0 - f32::from(sx) * 16.0;
                let dy = npc.position.1 - f32::from(sy) * 16.0;
                if (dx * dx + dy * dy).sqrt() < 600.0 && killed < 5 {
                    let direction = if dx > 0.0 { 1 } else { -1 };
                    let _ = client
                        .hit_npc(npc.index, npc.generation, 40, 3.0, direction)
                        .await;
                    killed += 1;
                }
            }
            Ok(Event::ItemSynced(item)) => {
                println!("  loot: item {} x{}", item.item.id, item.item.stack);
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }

    println!(
        "\nsaw {} distinct NPCs; sent {killed} attacks",
        seen.len().max(killed)
    );
    if seen.is_empty() && killed == 0 {
        eprintln!("no NPCs ever appeared");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
