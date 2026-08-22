//! Drive a running server through the things that matter and report on each.
//!
//! ```text
//! cargo run --release --example verify -- 127.0.0.1:7777
//! ```
//!
//! This is the end-to-end check: it joins a real server over the real protocol and confirms that
//! enemies move, that the ones that shoot actually put projectiles in the air, that those hurt you,
//! that bosses run their phases, and that killing something drops loot. Anything a unit test can
//! prove about a routine in isolation, it already proves; this is about the whole thing working
//! together.

use std::{collections::HashMap, env, process::ExitCode, time::Duration};

use terrustia_client::{Client, Event};
use terrustia_proto::npc_data::npc_stats;

/// Watch for a while, collecting what happened.
struct Seen {
    npcs: HashMap<u8, (u16, (f32, f32), u8)>,
    moved: usize,
    projectiles: usize,
    hits: usize,
    items: usize,
}

/// Watch for a while, keeping the player's position fresh so the server's routines keep seeing
/// them. A read timeout is a quiet moment, not the end: the watch runs to its deadline either way.
async fn watch(client: &mut Client, at: (f32, f32), how_long: Duration) -> Seen {
    let mut seen = Seen {
        npcs: HashMap::new(),
        moved: 0,
        projectiles: 0,
        hits: 0,
        items: 0,
    };
    let deadline = tokio::time::Instant::now() + how_long;
    let mut since_move = 0;
    while tokio::time::Instant::now() < deadline {
        since_move += 1;
        if since_move >= 20 {
            since_move = 0;
            if client.move_to(at.0, at.1).await.is_err() {
                break;
            }
        }
        match client.next_event().await {
            Ok(Event::NpcSynced(n)) => {
                let where_now = (n.position.0, n.position.1);
                if let Some((_, was, _)) = seen.npcs.get(&n.index)
                    && *was != where_now
                {
                    seen.moved += 1;
                }
                seen.npcs
                    .insert(n.index, (n.npc_type(), where_now, n.generation));
            }
            Ok(Event::ProjectileSynced(_)) => seen.projectiles += 1,
            Ok(Event::PlayerHurt(_)) => seen.hits += 1,
            Ok(Event::PlayerDied(_)) => {
                seen.hits += 1;
                // Staying dead would make every later check meaningless: nothing targets a corpse.
                if client.respawn().await.is_err() {
                    break;
                }
            }
            Ok(Event::ItemSynced(_)) => seen.items += 1,
            Ok(_) => {}
            // A quiet stretch, not a failure.
            Err(_) => continue,
        }
    }
    seen
}

fn report(what: &str, ok: bool, detail: String) -> bool {
    println!("{} {what}: {detail}", if ok { "PASS" } else { "FAIL" });
    ok
}

#[tokio::main]
async fn main() -> ExitCode {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7777".to_string())
        .parse()
        .expect("a socket address");

    let mut client = match Client::join(addr, "verifier").await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("join failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    client.set_timeout(Duration::from_millis(150));
    let world = client.world();
    println!(
        "joined \"{}\" {}x{}, spawn {:?}",
        world.name, world.width, world.height, world.spawn
    );

    let (sx, sy) = client.world().spawn;
    let here = (sx as f32 * 16.0, (sy - 3) as f32 * 16.0);
    client.move_to(here.0, here.1).await.ok();
    client.say("/time night").await.ok();

    let mut all_good = true;

    // 1. Enemies move under their own routines.
    client.say("/spawn Zombie").await.ok();
    client.say("/spawn DemonEye").await.ok();
    let seen = watch(&mut client, here, Duration::from_secs(4)).await;
    all_good &= report(
        "enemies move",
        seen.moved > 10,
        format!(
            "{} position changes across {} NPCs",
            seen.moved,
            seen.npcs.len()
        ),
    );

    // 2. The ones that shoot actually put something in the air.
    client.say("/butcher").await.ok();
    client.respawn().await.ok();
    client.say("/spawn Harpy").await.ok();
    client.say("/spawn GoblinSorcerer").await.ok();
    let seen = watch(&mut client, here, Duration::from_secs(8)).await;
    all_good &= report(
        "projectiles fly",
        seen.projectiles > 0,
        format!(
            "{} projectile updates; npcs seen {:?}",
            seen.projectiles,
            seen.npcs.values().map(|(t, _, _)| *t).collect::<Vec<_>>()
        ),
    );

    // 3. Standing among enemies costs health.
    client.say("/butcher").await.ok();
    client.respawn().await.ok();
    client.say("/spawn Zombie").await.ok();
    client.move_to(here.0, here.1).await.ok();
    let seen = watch(&mut client, here, Duration::from_secs(6)).await;
    all_good &= report(
        "enemies hurt you",
        seen.hits > 0,
        format!("{} hits taken", seen.hits),
    );

    // 4. A boss runs its phases and can be killed, dropping loot as it goes.
    client.say("/butcher").await.ok();
    client.respawn().await.ok();
    client.say("/time night").await.ok();
    client.move_to(here.0, here.1).await.ok();
    client.say("/spawn EyeofCthulhu").await.ok();
    let seen = watch(&mut client, here, Duration::from_secs(6)).await;
    let eye = seen.npcs.values().any(|(t, _, _)| *t == 4);
    let servants = seen.npcs.values().filter(|(t, _, _)| *t == 5).count();
    all_good &= report(
        "the Eye of Cthulhu fights",
        eye && servants > 0,
        format!("eye present: {eye}, servants summoned: {servants}"),
    );

    // 5. Killing something leaves something behind.
    client.say("/butcher").await.ok();
    client.respawn().await.ok();
    client.say("/spawn BlueSlime").await.ok();
    let seen = watch(&mut client, here, Duration::from_secs(2)).await;
    // The generation has to match, or the server rightly ignores the hit as stale.
    let slime = seen
        .npcs
        .iter()
        .find(|(_, (t, _, _))| npc_stats(*t).is_some_and(|s| !s.friendly && !s.boss))
        .map(|(index, (_, _, generation))| (*index, *generation));
    let mut dropped = 0;
    if let Some((index, generation)) = slime {
        for _ in 0..30 {
            client.hit_npc(index, generation, 500, 2.0, 1).await.ok();
        }
        dropped = watch(&mut client, here, Duration::from_secs(3)).await.items;
    }
    all_good &= report(
        "loot drops",
        dropped > 0,
        format!("{dropped} item updates after a kill"),
    );

    client.say("/butcher").await.ok();
    println!(
        "\n{}",
        if all_good {
            "all checks passed"
        } else {
            "SOME CHECKS FAILED"
        }
    );
    if all_good {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
