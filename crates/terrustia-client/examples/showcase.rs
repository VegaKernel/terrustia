//! Exercise a server the way a player would: mine, watch the drop, open a chest, read a sign.
//!
//! ```text
//! cargo run --release --example showcase -- 127.0.0.1:7777
//! ```

use std::{env, process::ExitCode, time::Duration};

use terrustia_client::{Client, Event};
use terrustia_proto::{id, objects::SyncChestItem};

#[tokio::main]
async fn main() -> ExitCode {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7777".to_string())
        .parse()
        .expect("a socket address");

    let mut client = match Client::join(addr, "showcase").await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("join failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    client.set_timeout(Duration::from_secs(10));
    let (sx, sy) = client.world().spawn;
    println!("joined \"{}\" at spawn {:?}", client.world().name, (sx, sy));

    // --- mining ---------------------------------------------------------------------------
    let mut mined = None;
    'search: for depth in 6..40 {
        for dx in -30..30 {
            let (x, y) = (i32::from(sx) + dx, i32::from(sy) + depth);
            if client.world().tile(x, y).is_some_and(|t| t.is_active()) {
                mined = Some((x, y));
                break 'search;
            }
        }
    }
    let Some((x, y)) = mined else {
        eprintln!("found no block to mine");
        return ExitCode::FAILURE;
    };

    let before = client.world().tile(x, y).unwrap();
    client
        .move_to(x as f32 * 16.0, y as f32 * 16.0)
        .await
        .unwrap();
    client.break_tile(x as i16, y as i16).await.unwrap();
    println!("mined tile {} at ({x}, {y})", before.block);

    match client
        .wait_for("the drop", |e| matches!(e, Event::ItemSynced(_)))
        .await
    {
        Ok(Event::ItemSynced(sync)) => {
            println!("  dropped item {} x{}", sync.item.id, sync.item.stack);
            match client
                .wait_for(
                    "the reservation",
                    |e| matches!(e, Event::ItemReserved(o) if o.index == sync.index),
                )
                .await
            {
                Ok(Event::ItemReserved(owner)) => {
                    println!("  reserved for slot {}", owner.owner);
                    client.pick_up(sync.index).await.unwrap();
                    println!("  picked it up");
                }
                _ => println!("  (no reservation arrived)"),
            }
        }
        _ => println!("  (no drop; this tile type has no simple drop)"),
    }

    // --- chests ---------------------------------------------------------------------------
    match client.world().nearest_chest(i32::from(sx), i32::from(sy)) {
        Some(chest) => {
            let (cx, cy, name) = (chest.x, chest.y, chest.name.clone());
            println!("opening chest at ({cx}, {cy}) {name:?}");
            client.open_chest(cx, cy).await.unwrap();

            let mut contents = Vec::new();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                match client.next_event().await {
                    Ok(Event::Other(frame)) if frame.id == id::SYNC_CHEST_ITEM => {
                        let sync = SyncChestItem::decode(&frame.payload).unwrap();
                        if !sync.item.is_empty() {
                            contents.push(format!("{}x item {}", sync.item.stack, sync.item.id));
                        }
                    }
                    Ok(Event::Other(frame)) if frame.id == id::SYNC_PLAYER_CHEST => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            if contents.is_empty() {
                println!("  the chest is empty");
            } else {
                println!("  contains: {}", contents.join(", "));
            }
        }
        None => println!("no chest in the sections we were sent"),
    }

    // --- signs ----------------------------------------------------------------------------
    let sign = client.world().signs().next().map(|s| (s.x, s.y));
    match sign {
        Some((sx, sy)) => {
            client.read_sign(sx, sy).await.unwrap();
            match client
                .wait_for(
                    "the sign text",
                    |e| matches!(e, Event::Other(f) if f.id == id::OPEN_SIGN_RESPONSE),
                )
                .await
            {
                Ok(Event::Other(frame)) => {
                    let text = terrustia_proto::objects::SignText::decode(&frame.payload).unwrap();
                    println!("sign at ({sx}, {sy}): {:?}", text.text);
                }
                _ => println!("sign never answered"),
            }
        }
        None => println!("no sign in the sections we were sent"),
    }

    ExitCode::SUCCESS
}
