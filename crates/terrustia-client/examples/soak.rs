//! Sit in a world for a while, wandering, so the server's tick cost can be watched under the
//! conditions a real player creates: streamed sections, natural spawns, contact damage.
//!
//! ```text
//! cargo run --release --example soak -- 127.0.0.1:7777 180
//! ```

use std::{env, process::ExitCode, time::Duration};

use terrustia_client::Client;
use tokio::time::{Instant, sleep};

#[tokio::main]
async fn main() -> ExitCode {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7777".to_string());
    let seconds: u64 = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let depth: f32 = env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    // The server refuses duplicate names at the door, so multiple soak clients against one server
    // must each be given their own — otherwise only the first joins and the rest exit failing,
    // silently reducing a "three real players" soak to one. Defaults to "soak" for a lone run.
    let name = env::args().nth(4).unwrap_or_else(|| "soak".to_string());
    let Ok(addr) = addr.parse() else {
        eprintln!("bad address");
        return ExitCode::FAILURE;
    };
    let mut client = match Client::join(addr, &name).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not join as {name:?}: {e}");
            return ExitCode::FAILURE;
        }
    };
    client.set_timeout(Duration::from_millis(50));
    let (sx, sy) = client.position();
    println!("joined at ({sx}, {sy}); soaking for {seconds}s");

    let started = Instant::now();
    let mut step = 0i32;
    while started.elapsed() < Duration::from_secs(seconds) {
        // Wander a few hundred tiles back and forth so sections keep streaming.
        let sweep = ((step % 240) - 120) as f32 * 16.0;
        let _ = client.move_to(sx + sweep, sy + depth * 16.0).await;
        step += 1;
        // Drain whatever arrived so the socket never backs up.
        for _ in 0..64 {
            if client.next_event().await.is_err() {
                break;
            }
        }
        sleep(Duration::from_millis(30)).await;
    }
    println!("done after {:?}", started.elapsed());
    ExitCode::SUCCESS
}
