//! Join a server and print a vertical run of tiles, pulling in the section first.
//!
//! ```text
//! cargo run --release --example peek -- 127.0.0.1:7780 2122 322 332
//! ```

use std::{env, process::ExitCode, time::Duration};

use terrustia_client::{Client, ClientWorld, Event};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("usage: peek <addr> <x> <y_from> <y_to>");
        return ExitCode::FAILURE;
    }
    let addr = args[0].parse().expect("a socket address");
    let (x, y0, y1) = (
        args[1].parse::<i32>().unwrap_or(0),
        args[2].parse::<i32>().unwrap_or(0),
        args[3].parse::<i32>().unwrap_or(0),
    );

    let mut client = match Client::join(addr, "peek").await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("join failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("joined \"{}\"", client.world().name);

    // Ask for the section holding the column, then wait for it to arrive.
    let (sx, sy) = ClientWorld::section_of(x, y0);
    if !client.world().has_section(sx, sy) {
        if let Err(e) = client.request_section(sx as u16, sy as u16).await {
            eprintln!("request failed: {e}");
            return ExitCode::FAILURE;
        }
        client.set_timeout(Duration::from_secs(15));
        if let Err(e) = client
            .wait_for("the requested section", |e| {
                matches!(e, Event::SectionLoaded { section_x, section_y } if *section_x == sx && *section_y == sy)
            })
            .await
        {
            eprintln!("section never arrived: {e}");
            return ExitCode::FAILURE;
        }
    }

    for y in y0..=y1 {
        match client.world().tile(x, y) {
            Some(t) if t.is_active() => println!("  y={y:<5} block {:<5} wall {}", t.block, t.wall),
            Some(t) => println!("  y={y:<5} EMPTY       wall {}", t.wall),
            None => println!("  y={y:<5} (not loaded)"),
        }
    }
    ExitCode::SUCCESS
}
