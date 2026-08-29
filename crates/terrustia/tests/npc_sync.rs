//! Regression coverage for NPC sync throttling.
//!
//! These tests deliberately put an observer far enough away that the server's per-player NPC
//! throttling engages. A one-off state change must be delayed, never discarded.

use std::{collections::HashSet, net::SocketAddr, time::Duration};

use terrustia::{
    config::Config,
    game::{GameServer, ServerEvent},
    net::listener,
    world::worldgen,
};
use terrustia_client::{Client, Event};
use tokio::{net::TcpListener, sync::mpsc};

async fn start() -> SocketAddr {
    let config = Config {
        world_width: 800,
        world_height: 600,
        motd: String::new(),
        ..Config::default()
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let world = worldgen::generate(
        config.world_width,
        config.world_height,
        config.world_name.clone(),
        7,
    );

    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    tokio::spawn(listener::run(listener, config, tx, None));
    addr
}

async fn join(addr: SocketAddr, name: &str) -> Client {
    let mut client = Client::join(addr, name).await.expect("handshake");
    client.set_timeout(Duration::from_secs(10));
    client
}

async fn spawn_npc(client: &mut Client, name: &str) -> terrustia_proto::npc::SyncNpc {
    let before: HashSet<(u8, u8)> = client
        .world()
        .npcs()
        .map(|npc| (npc.index, npc.generation))
        .collect();

    client.say(&format!("/spawn {name}")).await.unwrap();
    let event = client
        .wait_for("the spawned npc", |e| {
            matches!(e, Event::NpcSynced(n)
                if n.life != 0 && !before.contains(&(n.index, n.generation)))
        })
        .await
        .expect("npc never arrived");
    match event {
        Event::NpcSynced(npc) => npc,
        _ => unreachable!(),
    }
}

/// A dirty NPC used to lose a one-off state change when every faraway recipient was throttled.
///
/// The dirty bit was cleared before the broadcast. If Bob was far enough away for that broadcast
/// to be withheld, nothing made an inert NPC dirty again, so Bob kept the old health forever. The
/// server now keeps the NPC dirty while at least one recipient still owes an update, and the skip
/// counter guarantees delivery after at most four withheld attempts.
#[tokio::test]
async fn a_one_off_change_to_a_distant_inert_npc_is_eventually_delivered() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    // Start together so Bob definitely receives the spawn state before the throttle is engaged.
    alice.move_to(650.0 * 16.0, 300.0 * 16.0).await.unwrap();
    bob.move_to(650.0 * 16.0, 300.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 105 is the bound Goblin Tinkerer: aiStyle 0, so with zero knockback there is no movement or
    // other periodic state change to accidentally rescue a lost health update.
    let npc = spawn_npc(&mut alice, "105").await;
    assert_eq!(npc.npc_type(), 105);
    let initial_life = npc.life;

    bob.wait_for("the inert npc's initial state", |e| {
        matches!(e, Event::NpcSynced(n)
            if n.index == npc.index
                && n.generation == npc.generation
                && n.life == initial_life)
    })
    .await
    .expect("Bob never received the inert NPC before moving away");

    // Section 0 versus section 3 is outside SECTION_REACH=1, so Bob's next full NPC sync is
    // intentionally withheld while Alice remains beside the NPC.
    bob.move_to(100.0 * 16.0, 300.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    alice
        .hit_npc(npc.index, npc.generation, 1, 0.0, 1)
        .await
        .unwrap();

    bob.set_timeout(Duration::from_secs(3));
    let update = bob
        .wait_for("the delayed health update", |e| {
            matches!(e, Event::NpcSynced(n)
                if n.index == npc.index
                    && n.generation == npc.generation
                    && n.life < initial_life)
        })
        .await
        .expect(
            "the one-off health change was lost instead of being retried after the NPC sync skips",
        );

    let Event::NpcSynced(updated) = update else {
        unreachable!()
    };
    assert_eq!(
        updated.life,
        initial_life - 1,
        "a one-damage hit should reach the distant observer after the throttle delay"
    );
}
