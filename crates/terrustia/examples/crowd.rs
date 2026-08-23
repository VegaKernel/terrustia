//! Join a lot of players at once and see what it costs the tick.
//!
//! Everything else measures one client. A server's job is many, and several of the per-tick
//! surveys are per player — housing, spawning, contact damage — so this is where a cost that
//! looks like nothing becomes the whole budget.
//!
//! ```sh
//! cargo run --release -p terrustia --example crowd -- 127.0.0.1:7777 8 30
//! ```

use std::time::Duration;

use terrustia_client::Client;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let addr: std::net::SocketAddr = args
        .next()
        .unwrap_or_else(|| "127.0.0.1:7777".into())
        .parse()
        .expect("host:port");
    let count: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);

    println!("joining {count} players to {addr} for {seconds}s");
    let mut joined = Vec::new();
    for i in 0..count {
        match Client::join(addr, &format!("crowd{i}")).await {
            Ok(mut c) => {
                c.set_timeout(Duration::from_secs(20));
                joined.push(c);
            }
            Err(e) => {
                println!("  player {i} could not join: {e}");
                break;
            }
        }
    }
    println!("{} joined", joined.len());
    if joined.is_empty() {
        return;
    }

    // Spread them out, so the per-player surveys cannot share their work.
    let spawn = joined[0].world().spawn;
    let width = joined[0].world().width;
    let mut tasks = Vec::new();
    for (i, mut c) in joined.into_iter().enumerate() {
        let x = ((i as i32 * 137) % width.max(1)).clamp(200, width - 200) as f32 * 16.0;
        let y = f32::from(spawn.1) * 16.0 - 64.0;
        tasks.push(tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
            let mut frames = 0u64;
            let mut walk = 0.0f32;
            // A real client reports its position once a tick, not as fast as the loop will go.
            // Hammering it faster measures the test rather than the server.
            let mut next_move = tokio::time::Instant::now();
            while tokio::time::Instant::now() < deadline {
                if tokio::time::Instant::now() >= next_move {
                    walk = (walk + 8.0) % 320.0;
                    if c.move_to(x + walk, y).await.is_err() {
                        break;
                    }
                    next_move += Duration::from_millis(16);
                }
                match tokio::time::timeout(Duration::from_millis(5), c.next_event()).await {
                    Ok(Ok(_)) => frames += 1,
                    Ok(Err(_)) => break,
                    Err(_) => {}
                }
            }
            frames
        }));
    }
    let mut total = 0u64;
    for t in tasks {
        total += t.await.unwrap_or(0);
    }
    println!("{total} frames received across the crowd");
    println!("the server's own tick costs are in its log: look for `tick window`");
}
