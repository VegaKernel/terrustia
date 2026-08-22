//! Join a server, break a run of blocks, ask it to save, and report what changed.
//!
//! Used to check that edits made over the network actually reach the world file.
//!
//! ```text
//! cargo run --release --example mine_and_save -- 127.0.0.1:7779
//! ```

use std::{env, process::ExitCode, time::Duration};

use terrustia_client::{Client, Event};

#[tokio::main]
async fn main() -> ExitCode {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7779".to_string())
        .parse()
        .expect("a socket address");

    let mut client = match Client::join(addr, "miner").await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("join failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (sx, sy) = client.world().spawn;
    println!("joined \"{}\" at spawn {:?}", client.world().name, (sx, sy));

    // Dig straight down from a little to the right of spawn, where there is certainly ground.
    let dig_x = i32::from(sx) + 20;
    let mut dug = Vec::new();
    for depth in 12..32 {
        let y = i32::from(sy) + depth;
        if client.world().tile(dig_x, y).is_some_and(|t| t.is_active()) {
            if let Err(e) = client.break_tile(dig_x as i16, y as i16).await {
                eprintln!("break failed: {e}");
                return ExitCode::FAILURE;
            }
            dug.push(y);
        }
    }
    println!(
        "asked to break {} blocks at x={dig_x}: {:?}",
        dug.len(),
        dug
    );

    if let Err(e) = client.say("/save").await {
        eprintln!("save command failed: {e}");
        return ExitCode::FAILURE;
    }

    client.set_timeout(Duration::from_secs(30));
    match client
        .wait_for("the save confirmation", |e| {
            matches!(e, Event::Chat { text, .. } if text.contains("World saved") || text.contains("FAILED"))
        })
        .await
    {
        Ok(Event::Chat { text, .. }) => {
            println!("server says: {text}");
            if text.contains("FAILED") {
                return ExitCode::FAILURE;
            }
        }
        Ok(_) => unreachable!(),
        Err(e) => {
            eprintln!("no save confirmation: {e}");
            return ExitCode::FAILURE;
        }
    }

    println!(
        "dug {} blocks; check the saved file for holes at x={dig_x}",
        dug.len()
    );
    for y in &dug {
        print!("{y} ");
    }
    println!();
    ExitCode::SUCCESS
}
