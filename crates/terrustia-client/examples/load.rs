//! Fill the world with enemies and see whether the server keeps up.
//!
//! ```text
//! cargo run --release --example load -- 127.0.0.1:7777 60
//! ```
use std::{env, time::Duration};
use terrustia_client::{Client, Event};

#[tokio::main]
async fn main() {
    let addr: std::net::SocketAddr = env::args().nth(1).unwrap().parse().unwrap();
    let secs: u64 = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let mut c = Client::join(addr, "load").await.unwrap();
    c.set_timeout(Duration::from_millis(100));
    let (sx, sy) = c.world().spawn;
    let here = (sx as f32 * 16.0, (sy - 3) as f32 * 16.0);
    c.move_to(here.0, here.1).await.unwrap();
    c.say("/time night").await.unwrap();

    // A crowd of everything that shoots, walks, flies and burrows.
    for what in [
        "Zombie",
        "DemonEye",
        "Harpy",
        "Demon",
        "CaveBat",
        "Hornet",
        "EaterofSouls",
        "GoblinSorcerer",
        "Skeleton",
        "GiantWormHead",
        "Bunny",
        "Squirrel",
        "Vulture",
        "BlueJellyfish",
        "CursedSkull",
        "SnowBalla",
        "Butterfly",
        "Tumbleweed",
    ] {
        for _ in 0..6 {
            c.say(&format!("/spawn {what}")).await.ok();
        }
    }

    let mut events = 0u64;
    let mut projectiles = 0u64;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut i = 0u64;
    while tokio::time::Instant::now() < deadline {
        i += 1;
        if i.is_multiple_of(30) {
            c.move_to(here.0, here.1).await.ok();
        }
        match c.next_event().await {
            Ok(Event::ProjectileSynced(_)) => {
                projectiles += 1;
                events += 1;
            }
            Ok(Event::PlayerDied(_)) => {
                c.respawn().await.ok();
                events += 1;
            }
            Ok(_) => events += 1,
            Err(_) => continue,
        }
    }
    println!("{events} events over {secs}s ({projectiles} projectile updates)");
    c.say("/npcs").await.ok();
    for _ in 0..40 {
        if let Ok(Event::Chat { text, .. }) = c.next_event().await {
            println!("  {text}");
        }
    }
}
