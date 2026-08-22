//! Connect to a Terraria server, walk around, and report what the world looks like.
//!
//! Works against this server and against the real `TerrariaServer`, which is the point: if the
//! same client is happy with both, they behave the same.
//!
//! ```text
//! cargo run --release --example bot -- 127.0.0.1:7777
//! ```

use std::{env, process::ExitCode, time::Duration};

use terrustia_client::{Client, Event};

#[tokio::main]
async fn main() -> ExitCode {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7777".to_string());
    let addr = match addr.parse() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("bad address {addr}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut client = match Client::join(addr, "terrustia-bot").await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("could not join: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "joined as slot {} of \"{}\"",
        client.slot(),
        client.world().name
    );
    println!(
        "world is {}x{}, spawn at {:?}, {} sections received during the handshake",
        client.world().width,
        client.world().height,
        client.world().spawn,
        client.world().loaded_sections()
    );

    // Look at the ground under spawn to confirm the tiles decoded into something sensible.
    let (sx, sy) = client.world().spawn;
    let column: Vec<String> = (0..12)
        .map(|d| {
            let y = i32::from(sy) + d;
            match client.world().tile(i32::from(sx), y) {
                Some(t) if t.is_active() => format!("{}", t.block),
                Some(_) => "·".to_string(),
                None => "?".to_string(),
            }
        })
        .collect();
    println!("column below spawn: {}", column.join(" "));

    // Walk east, pulling in sections the way a real client does.
    let target_x = i32::from(sx) + 420;
    if let Err(e) = client.walk_to_tile(target_x, i32::from(sy)).await {
        eprintln!("walk failed: {e}");
        return ExitCode::FAILURE;
    }

    client.set_timeout(Duration::from_secs(8));
    let mut new_sections = 0;
    // Give the server a moment to answer the section requests.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        match client.next_event().await {
            Ok(Event::SectionLoaded {
                section_x,
                section_y,
            }) => {
                new_sections += 1;
                println!("  section ({section_x}, {section_y}) arrived");
            }
            Ok(Event::Chat { author, text }) => println!("  chat [{author}] {text}"),
            Ok(_) => {}
            Err(_) => break,
        }
    }

    println!(
        "after walking east: {} sections held, {new_sections} newly streamed",
        client.world().loaded_sections()
    );
    let far = client.world().tile(target_x, i32::from(sy) + 6);
    println!("tile 420 east and 6 down: {far:?}");

    if far.is_none() {
        eprintln!("the server never sent tiles for where we walked");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
