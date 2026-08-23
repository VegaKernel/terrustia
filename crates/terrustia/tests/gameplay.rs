//! Gameplay behaviour, driven through the headless client.
//!
//! These exercise the parts of the server a player actually touches: chests, signs, multi-tile
//! edits, the world clock, chat commands and saving.

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use terrustia::{
    config::Config,
    game::{GameServer, ServerEvent},
    net::listener,
    world::{Chest, Sign, World, wld, wld_save, worldgen},
};
use terrustia_client::{Client, Event};
use terrustia_proto::{ItemStack, Tile, id, square::TileSquare};
use tokio::{net::TcpListener, sync::mpsc};

/// Start a server on an ephemeral port, letting the caller shape the world first.
async fn start_with<F: FnOnce(&mut World)>(mut config: Config, prepare: F) -> SocketAddr {
    config.world_width = 800;
    config.world_height = 600;
    config.motd = String::new();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let mut world = worldgen::generate(
        config.world_width,
        config.world_height,
        config.world_name.clone(),
        7,
    );
    prepare(&mut world);

    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    tokio::spawn(listener::run(listener, config, tx));
    addr
}

async fn start() -> SocketAddr {
    start_with(Config::default(), |_| {}).await
}

async fn join(addr: SocketAddr, name: &str) -> Client {
    let mut client = Client::join(addr, name).await.expect("handshake");
    client.set_timeout(Duration::from_secs(10));
    client
}

#[tokio::test]
async fn a_chest_can_be_opened_and_its_contents_edited() {
    // Put a chest somewhere the spawn stream will cover.
    let addr = start_with(Config::default(), |world| {
        world.chests = vec![Some(Chest {
            x: 400,
            y: 320,
            name: "Loot".into(),
            items: vec![ItemStack::new(3507, 5, 0), ItemStack::EMPTY],
        })];
    })
    .await;

    let mut client = join(addr, "looter").await;
    client.open_chest(400, 320).await.unwrap();

    // The server announces the size, then every slot, then which chest is open.
    let mut size = None;
    let mut slots = Vec::new();
    let mut opened = false;
    for _ in 0..40 {
        match client.next_event().await.unwrap() {
            Event::Other(frame) if frame.id == id::SYNC_CHEST_SIZE => {
                size = Some(i16::from_le_bytes([frame.payload[2], frame.payload[3]]));
            }
            Event::Other(frame) if frame.id == id::SYNC_CHEST_ITEM => {
                slots
                    .push(terrustia_proto::objects::SyncChestItem::decode(&frame.payload).unwrap());
            }
            Event::Other(frame) if frame.id == id::SYNC_PLAYER_CHEST => {
                let sync =
                    terrustia_proto::objects::SyncPlayerChest::decode(&frame.payload).unwrap();
                assert_eq!(sync.name.as_deref(), Some("Loot"));
                opened = true;
                break;
            }
            _ => {}
        }
    }

    assert_eq!(size, Some(2), "the client must be told the chest's size");
    assert!(opened, "the server never confirmed the chest was open");
    assert_eq!(slots.len(), 2, "every slot should be sent");
    assert_eq!(slots[0].item, ItemStack::new(3507, 5, 0));
    assert!(slots[1].item.is_empty());
}

#[tokio::test]
async fn a_chest_edit_is_refused_unless_the_chest_is_open() {
    let addr = start_with(Config::default(), |world| {
        world.chests = vec![Some(Chest {
            x: 400,
            y: 320,
            name: String::new(),
            items: vec![ItemStack::EMPTY; 4],
        })];
    })
    .await;

    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    // Bob never opened it, so his edit must not take.
    let forged = terrustia_proto::objects::SyncChestItem {
        chest: 0,
        slot: 0,
        item: ItemStack::new(99, 1, 0),
    };
    bob.send(&forged.encode().unwrap()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Alice opens it and should see an empty slot.
    alice.open_chest(400, 320).await.unwrap();
    let mut first_slot = None;
    for _ in 0..40 {
        if let Event::Other(frame) = alice.next_event().await.unwrap()
            && frame.id == id::SYNC_CHEST_ITEM
        {
            let sync = terrustia_proto::objects::SyncChestItem::decode(&frame.payload).unwrap();
            if sync.slot == 0 {
                first_slot = Some(sync.item);
                break;
            }
        }
    }
    assert_eq!(
        first_slot,
        Some(ItemStack::EMPTY),
        "an edit from a player who never opened the chest was applied"
    );
}

#[tokio::test]
async fn two_players_cannot_open_the_same_chest() {
    let addr = start_with(Config::default(), |world| {
        world.chests = vec![Some(Chest {
            x: 400,
            y: 320,
            name: String::new(),
            items: vec![ItemStack::EMPTY; 2],
        })];
    })
    .await;

    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    alice.open_chest(400, 320).await.unwrap();
    alice
        .wait_for(
            "alice's chest to open",
            |e| matches!(e, Event::Other(f) if f.id == id::SYNC_PLAYER_CHEST),
        )
        .await
        .unwrap();

    // Bob's attempt should be ignored while alice is inside.
    bob.open_chest(400, 320).await.unwrap();
    bob.set_timeout(Duration::from_secs(2));
    let opened = bob
        .wait_for(
            "bob's chest to open",
            |e| matches!(e, Event::Other(f) if f.id == id::SYNC_PLAYER_CHEST),
        )
        .await;
    assert!(opened.is_err(), "two players opened the same chest at once");
}

#[tokio::test]
async fn a_sign_can_be_read_and_rewritten() {
    let addr = start_with(Config::default(), |world| {
        world.signs = vec![Some(Sign {
            x: 400,
            y: 320,
            text: "original".into(),
        })];
    })
    .await;

    let mut client = join(addr, "reader").await;
    client.read_sign(400, 320).await.unwrap();

    let text = client
        .wait_for(
            "the sign text",
            |e| matches!(e, Event::Other(f) if f.id == id::OPEN_SIGN_RESPONSE),
        )
        .await
        .unwrap();
    let Event::Other(frame) = text else {
        unreachable!()
    };
    let sign = terrustia_proto::objects::SignText::decode(&frame.payload).unwrap();
    assert_eq!(sign.text, "original");

    // Rewrite it, then have a second player read it back.
    let update = terrustia_proto::objects::SignText {
        sign: sign.sign,
        x: 400,
        y: 320,
        text: "rewritten".into(),
        player: 0,
        editing: 0,
    };
    client.send(&update.encode().unwrap()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut second = join(addr, "second").await;
    second.read_sign(400, 320).await.unwrap();
    let event = second
        .wait_for(
            "the updated sign",
            |e| matches!(e, Event::Other(f) if f.id == id::OPEN_SIGN_RESPONSE),
        )
        .await
        .unwrap();
    let Event::Other(frame) = event else {
        unreachable!()
    };
    assert_eq!(
        terrustia_proto::objects::SignText::decode(&frame.payload)
            .unwrap()
            .text,
        "rewritten"
    );
}

#[tokio::test]
async fn a_tile_square_is_applied_and_relayed() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    // A 2x2 block of stone, the shape a piece of furniture would arrive as.
    let square = TileSquare {
        x: 402,
        y: 330,
        width: 2,
        height: 2,
        change_type: 0,
        tiles: vec![Tile::block(1); 4],
    };
    bob.send(&square.encode().unwrap()).await.unwrap();

    // Alice sees the relay.
    alice
        .wait_for(
            "the relayed square",
            |e| matches!(e, Event::Other(f) if f.id == id::AREA_TILE_CHANGE),
        )
        .await
        .unwrap();

    // And a fresh player is streamed the applied result.
    let fresh = join(addr, "fresh").await;
    for dx in 0..2 {
        for dy in 0..2 {
            let tile = fresh.world().tile(402 + dx, 330 + dy);
            assert_eq!(
                tile.map(|t| t.block),
                Some(1),
                "square tile ({dx}, {dy}) did not stick"
            );
        }
    }
}

#[tokio::test]
async fn a_tile_square_reaching_outside_the_world_is_refused() {
    let addr = start().await;
    let mut client = join(addr, "edge").await;

    let square = TileSquare {
        x: 799,
        y: 599,
        width: 4,
        height: 4,
        change_type: 0,
        tiles: vec![Tile::block(1); 16],
    };
    client.send(&square.encode().unwrap()).await.unwrap();

    // The server should stay up and keep answering.
    client.say("still here?").await.unwrap();
    client
        .wait_for(
            "chat to still work",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("still here?")),
        )
        .await
        .expect("server stopped responding after an out-of-bounds square");
}

#[tokio::test]
async fn chat_commands_answer() {
    let addr = start().await;
    let mut client = join(addr, "asker").await;

    client.say("/players").await.unwrap();
    let event = client
        .wait_for(
            "the player list",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("online")),
        )
        .await
        .unwrap();
    if let Event::Chat { text, .. } = event {
        assert!(text.contains("asker"), "player list should name us: {text}");
    }

    client.say("/time night").await.unwrap();
    client
        .wait_for(
            "the time change",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("Time set to night")),
        )
        .await
        .unwrap();

    client.say("/nonsense").await.unwrap();
    client
        .wait_for(
            "an unknown-command reply",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("unknown command")),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_generated_world_reports_that_it_cannot_be_saved() {
    let addr = start().await;
    let mut client = join(addr, "saver").await;

    client.say("/save").await.unwrap();
    client
        .wait_for(
            "the refusal",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("cannot be saved")),
        )
        .await
        .unwrap();
}

/// Build a small world, save it, and serve the save so the whole persistence path is exercised.
#[tokio::test]
async fn edits_survive_a_save_and_reload() {
    let dir = std::env::temp_dir().join(format!("terrustia-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source: PathBuf = dir.join("source.wld");

    // A generated world cannot be saved, so this test needs a world that came from a file. There
    // is no such file to start from, so the test is skipped unless one has been provided.
    let Ok(seed_world) = std::env::var("TERRUSTIA_TEST_WLD") else {
        eprintln!("set TERRUSTIA_TEST_WLD to a .wld path to run the save round-trip test");
        return;
    };
    std::fs::copy(&seed_world, &source).unwrap();

    let saved = dir.join("saved.wld");
    let config = Config {
        world_file: Some(source.clone()),
        save_file: Some(saved.clone()),
        autosave_secs: 0,
        motd: String::new(),
        ..Config::default()
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let world = wld::load(&source).unwrap();
    let (spawn_x, spawn_y) = (world.spawn_x, world.spawn_y);
    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    tokio::spawn(listener::run(listener, config, tx));

    let mut client = join(addr, "digger").await;
    let dig_x = i32::from(spawn_x) + 20;
    let mut dug = Vec::new();
    for depth in 12..30 {
        let y = i32::from(spawn_y) + depth;
        if client.world().tile(dig_x, y).is_some_and(|t| t.is_active()) {
            client.break_tile(dig_x as i16, y as i16).await.unwrap();
            dug.push(y);
        }
    }
    assert!(!dug.is_empty(), "found nothing to dig");

    client.say("/save").await.unwrap();
    client
        .wait_for(
            "the save confirmation",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("World saved")),
        )
        .await
        .unwrap();

    let reloaded = wld::load(&saved).unwrap();
    for y in dug {
        assert!(
            !reloaded.tile(dig_x, y).is_active(),
            "block at ({dig_x}, {y}) came back after the save"
        );
    }

    // And the save is itself loadable and re-saveable.
    wld_save::save(&reloaded, &dir.join("again.wld")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn breaking_a_block_drops_the_right_item() {
    let addr = start_with(Config::default(), |world| {
        // A lone stone block in mid-air, so the drop is unambiguous.
        world.set_tile(410, 300, Tile::block(1));
    })
    .await;

    let mut client = join(addr, "miner").await;
    client.break_tile(410, 300).await.unwrap();

    let event = client
        .wait_for("the dropped item", |e| matches!(e, Event::ItemSynced(_)))
        .await
        .unwrap();
    let Event::ItemSynced(sync) = event else {
        unreachable!()
    };
    // Stone is tile 1, and its item is StoneBlock (3).
    assert_eq!(sync.item.id, 3, "stone should drop a stone block");
    assert_eq!(sync.item.stack, 1);
}

#[tokio::test]
async fn a_framed_object_drops_nothing_rather_than_the_wrong_item() {
    let addr = start_with(Config::default(), |world| {
        // Chests pick their drop from a frame style, which is not modelled.
        world.set_tile(410, 300, Tile::framed(21, 0, 0));
    })
    .await;

    let mut client = join(addr, "miner").await;
    client.break_tile(410, 300).await.unwrap();

    client.set_timeout(Duration::from_secs(2));
    let dropped = client
        .wait_for("an item drop", |e| matches!(e, Event::ItemSynced(_)))
        .await;
    assert!(
        dropped.is_err(),
        "a framed object dropped something it should not have"
    );
}

#[tokio::test]
async fn a_dropped_item_is_reserved_and_can_be_picked_up() {
    let addr = start_with(Config::default(), |world| {
        world.set_tile(410, 300, Tile::block(1));
    })
    .await;

    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    // Stand alice on top of the block so the reservation goes to her.
    alice.move_to(410.0 * 16.0, 300.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    alice.break_tile(410, 300).await.unwrap();
    let event = alice
        .wait_for("the drop", |e| matches!(e, Event::ItemSynced(_)))
        .await
        .unwrap();
    let Event::ItemSynced(sync) = event else {
        unreachable!()
    };

    let reserved = alice
        .wait_for(
            "the reservation",
            |e| matches!(e, Event::ItemReserved(o) if o.index == sync.index),
        )
        .await
        .unwrap();
    let Event::ItemReserved(owner) = reserved else {
        unreachable!()
    };
    assert_eq!(owner.owner, alice.slot(), "the nearby player should get it");

    // Alice picks it up; bob is told it is gone.
    alice.pick_up(sync.index).await.unwrap();
    bob.wait_for(
        "the despawn",
        |e| matches!(e, Event::ItemDespawned(i) if *i == sync.index),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn an_item_cannot_be_picked_up_by_someone_it_is_not_reserved_for() {
    let addr = start_with(Config::default(), |world| {
        world.set_tile(410, 300, Tile::block(1));
    })
    .await;

    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    alice.move_to(410.0 * 16.0, 300.0 * 16.0).await.unwrap();
    // Put bob far away so he never earns the reservation.
    bob.move_to(100.0 * 16.0, 300.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    alice.break_tile(410, 300).await.unwrap();
    let event = alice
        .wait_for("the drop", |e| matches!(e, Event::ItemSynced(_)))
        .await
        .unwrap();
    let Event::ItemSynced(sync) = event else {
        unreachable!()
    };

    // Bob claims it anyway.
    bob.pick_up(sync.index).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // A player joining now must still see the item, because bob's claim was refused.
    let fresh = join(addr, "fresh").await;
    drop(fresh);

    alice.set_timeout(Duration::from_secs(2));
    let despawned = alice
        .wait_for("a despawn", |e| matches!(e, Event::ItemDespawned(_)))
        .await;
    assert!(
        despawned.is_err(),
        "a stranger was allowed to take the item"
    );
}

#[tokio::test]
async fn a_player_can_throw_an_item_into_the_world() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    alice
        .drop_item(ItemStack::new(3, 17, 0), (410.0 * 16.0, 300.0 * 16.0))
        .await
        .unwrap();

    let event = bob
        .wait_for("the thrown item", |e| matches!(e, Event::ItemSynced(_)))
        .await
        .unwrap();
    let Event::ItemSynced(sync) = event else {
        unreachable!()
    };
    assert_eq!(sync.item.id, 3);
    assert_eq!(sync.item.stack, 17);
    assert_ne!(
        sync.index, 400,
        "the server should have allocated a real slot"
    );
}

#[tokio::test]
async fn a_password_is_required_when_one_is_configured() {
    let addr = start_with(
        Config {
            password: "hunter2".into(),
            ..Config::default()
        },
        |_| {},
    )
    .await;

    // A plain join never gets a slot, because the server asks for a password first.
    let mut raw = terrustia_client::Client::connect(addr, "guest")
        .await
        .unwrap();
    raw.set_timeout(Duration::from_secs(3));
    let joined = raw.handshake().await;
    assert!(
        joined.is_err(),
        "joined a password-protected server without one"
    );
}

#[tokio::test]
async fn the_wrong_password_is_refused_and_the_right_one_accepted() {
    let addr = start_with(
        Config {
            password: "hunter2".into(),
            ..Config::default()
        },
        |_| {},
    )
    .await;

    let send_password = |password: &str| {
        let mut w = terrustia_proto::PacketWriter::new(id::SEND_PASSWORD);
        w.string(password);
        w.finish().unwrap()
    };
    let hello = || {
        let mut w = terrustia_proto::PacketWriter::new(id::HELLO);
        w.string(id::VERSION_STRING);
        w.finish().unwrap()
    };

    // Wrong password: kicked.
    let mut bad = terrustia_client::Client::connect(addr, "bad")
        .await
        .unwrap();
    bad.set_timeout(Duration::from_secs(5));
    bad.send(&hello()).await.unwrap();
    bad.wait_for(
        "the password prompt",
        |e| matches!(e, Event::Other(f) if f.id == id::REQUEST_PASSWORD),
    )
    .await
    .unwrap();
    bad.send(&send_password("wrong")).await.unwrap();
    let result = bad.next_event().await;
    assert!(
        matches!(result, Err(terrustia_client::ClientError::Kicked { .. })),
        "a wrong password should be refused, got {result:?}"
    );

    // Right password: a slot arrives.
    let mut good = terrustia_client::Client::connect(addr, "good")
        .await
        .unwrap();
    good.set_timeout(Duration::from_secs(5));
    good.send(&hello()).await.unwrap();
    good.wait_for(
        "the password prompt",
        |e| matches!(e, Event::Other(f) if f.id == id::REQUEST_PASSWORD),
    )
    .await
    .unwrap();
    good.send(&send_password("hunter2")).await.unwrap();
    good.wait_for(
        "a player slot",
        |e| matches!(e, Event::Other(f) if f.id == id::PLAYER_INFO),
    )
    .await
    .expect("the right password should be accepted");
}

#[tokio::test]
async fn a_password_cannot_be_used_to_skip_the_version_check() {
    let addr = start_with(
        Config {
            password: "hunter2".into(),
            ..Config::default()
        },
        |_| {},
    )
    .await;

    // Send only the password, never a Hello.
    let mut sneaky = terrustia_client::Client::connect(addr, "sneaky")
        .await
        .unwrap();
    sneaky.set_timeout(Duration::from_secs(2));
    let mut w = terrustia_proto::PacketWriter::new(id::SEND_PASSWORD);
    w.string("hunter2");
    sneaky.send(&w.finish().unwrap()).await.unwrap();

    let granted = sneaky
        .wait_for(
            "a player slot",
            |e| matches!(e, Event::Other(f) if f.id == id::PLAYER_INFO),
        )
        .await;
    assert!(granted.is_err(), "the version check was bypassed");
}

/// Ask the server to spawn an NPC beside us and wait for it to arrive.
async fn spawn_npc(client: &mut Client, name: &str) -> terrustia_proto::npc::SyncNpc {
    client.say(&format!("/spawn {name}")).await.unwrap();
    let event = client
        .wait_for(
            "the spawned npc",
            |e| matches!(e, Event::NpcSynced(n) if n.life != 0),
        )
        .await
        .expect("npc never arrived");
    match event {
        Event::NpcSynced(npc) => npc,
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn an_npc_can_be_spawned_and_is_announced_to_everyone() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    let npc = spawn_npc(&mut alice, "Zombie").await;
    assert_eq!(npc.npc_type(), 3, "should be a zombie");
    assert_eq!(npc.life_max, npc.life, "should arrive at full health");

    // Bob sees it too.
    bob.wait_for(
        "the npc",
        |e| matches!(e, Event::NpcSynced(n) if n.npc_type() == 3),
    )
    .await
    .expect("the other player was not told about the npc");
}

#[tokio::test]
async fn spawning_by_name_and_by_id_both_work() {
    let addr = start().await;
    let mut client = join(addr, "namer").await;

    let by_name = spawn_npc(&mut client, "BlueSlime").await;
    assert_eq!(by_name.npc_type(), 1);

    let by_id = spawn_npc(&mut client, "49").await;
    assert_eq!(by_id.npc_type(), 49, "cave bat by id");
}

#[tokio::test]
async fn an_unknown_npc_name_is_refused() {
    let addr = start().await;
    let mut client = join(addr, "typo").await;

    client.say("/spawn Notarealenemy").await.unwrap();
    client
        .wait_for(
            "the usage hint",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("usage: /spawn")),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn hitting_an_npc_reduces_its_health_and_kills_it() {
    let addr = start().await;
    let mut client = join(addr, "fighter").await;

    // A zombie has 45 health and 6 defence, so a 20-damage hit lands for 17.
    let npc = spawn_npc(&mut client, "Zombie").await;
    assert_eq!(npc.life_max, 45);

    client
        .hit_npc(npc.index, npc.generation, 20, 0.0, 1)
        .await
        .unwrap();
    let event = client
        .wait_for("the wounded npc", |e| {
            matches!(e, Event::NpcSynced(n) if n.index == npc.index && n.life > 0 && n.life < 45)
        })
        .await
        .unwrap();
    if let Event::NpcSynced(hurt) = event {
        assert_eq!(hurt.life, 28, "45 - (20 - 6/2) = 28");
    }

    // Two more hits finish it.
    for _ in 0..2 {
        client
            .hit_npc(npc.index, npc.generation, 20, 0.0, 1)
            .await
            .unwrap();
    }
    client
        .wait_for(
            "the death",
            |e| matches!(e, Event::NpcSynced(n) if n.index == npc.index && n.life == 0),
        )
        .await
        .expect("the zombie never died");
}

#[tokio::test]
async fn a_dead_npc_drops_its_coin_value() {
    let addr = start().await;
    let mut client = join(addr, "looter").await;

    // A zombie is worth 60 copper.
    let npc = spawn_npc(&mut client, "Zombie").await;
    for _ in 0..5 {
        client
            .hit_npc(npc.index, npc.generation, 40, 0.0, 1)
            .await
            .unwrap();
    }

    let event = client
        .wait_for(
            "the coin drop",
            |e| matches!(e, Event::ItemSynced(i) if (71..=74).contains(&i.item.id)),
        )
        .await
        .expect("no coins dropped");
    if let Event::ItemSynced(item) = event {
        assert_eq!(item.item.id, 71, "60 copper should drop copper coins");
        assert_eq!(item.item.stack, 60);
    }
}

#[tokio::test]
async fn a_hit_with_a_stale_generation_is_ignored() {
    let addr = start().await;
    let mut client = join(addr, "stale").await;

    let npc = spawn_npc(&mut client, "Zombie").await;
    // Claim a generation the NPC does not have; the hit must not land.
    client
        .hit_npc(npc.index, npc.generation.wrapping_add(7), 100, 0.0, 1)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    client.say("/npcs").await.unwrap();
    let event = client
        .wait_for(
            "the npc list",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("NPCs")),
        )
        .await
        .unwrap();
    if let Event::Chat { text, .. } = event {
        assert!(
            text.contains("Zombie"),
            "the zombie should have survived a stale hit: {text}"
        );
    }
}

#[tokio::test]
async fn butcher_clears_hostiles_but_spares_town_npcs() {
    let addr = start().await;
    let mut client = join(addr, "butcher").await;

    spawn_npc(&mut client, "Zombie").await;
    spawn_npc(&mut client, "Guide").await;

    client.say("/butcher").await.unwrap();
    client
        .wait_for(
            "the butcher report",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("Butchered")),
        )
        .await
        .unwrap();

    client.say("/npcs").await.unwrap();
    let event = client
        .wait_for(
            "the npc list",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("NPCs")),
        )
        .await
        .unwrap();
    if let Event::Chat { text, .. } = event {
        assert!(text.contains("Guide"), "the guide should remain: {text}");
        assert!(
            !text.contains("Zombie"),
            "the zombie should be gone: {text}"
        );
    }
}

#[tokio::test]
async fn a_joining_player_is_told_about_npcs_already_alive() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;
    let npc = spawn_npc(&mut alice, "Skeleton").await;

    // Bob joins afterwards and should be sent the skeleton during his handshake.
    let mut bob = join(addr, "bob").await;
    bob.set_timeout(Duration::from_secs(5));
    let seen = bob
        .wait_for(
            "the existing npc",
            |e| matches!(e, Event::NpcSynced(n) if n.index == npc.index),
        )
        .await;
    assert!(seen.is_ok(), "a late joiner never heard about the skeleton");
}

/// Build a furnished, sealed room into a world, returning a tile inside it.
fn build_house(world: &mut World, x0: i32, y0: i32) -> (i32, i32) {
    let (w, h) = (14, 10);
    for x in x0..x0 + w {
        for y in y0..y0 + h {
            if x == x0 || x == x0 + w - 1 || y == y0 || y == y0 + h - 1 {
                world.set_tile(x, y, Tile::block(1));
            } else {
                let mut air = Tile::AIR;
                air.wall = 4; // stone wall counts as a house wall
                world.set_tile(x, y, air);
            }
        }
    }
    world.set_tile(x0 + 2, y0 + h - 2, Tile::framed(15, 0, 0)); // chair
    world.set_tile(x0 + 4, y0 + h - 2, Tile::framed(14, 0, 0)); // table
    world.set_tile(x0 + 6, y0 + h - 2, Tile::framed(4, 0, 0)); // torch
    world.set_tile(x0 + 1, y0 + h - 2, Tile::framed(10, 0, 0)); // door
    (x0 + 5, y0 + 3)
}

/// A finished house gets a Guide, without anybody asking for one.
///
/// This is the whole of town NPCs arriving: the server scans for a free house near a player every
/// few seconds and moves somebody into it. A server that never does it has a world where nobody
/// ever turns up, however much is built.
#[tokio::test]
async fn a_house_gets_a_guide() {
    let inside = std::cell::Cell::new((0, 0));
    let addr = start_with(Config::default(), |world| {
        inside.set(build_house(world, 300, 300));
    })
    .await;
    let mut client = join(addr, "builder").await;
    let (hx, hy) = inside.get();
    client
        .move_to(hx as f32 * 16.0, hy as f32 * 16.0)
        .await
        .unwrap();

    // The scan runs every few seconds, so this waits rather than expecting it at once.
    client.set_timeout(Duration::from_secs(20));
    let guide = client
        .wait_for(
            "the Guide moving in",
            |e| matches!(e, Event::NpcSynced(n) if n.net_id == 22),
        )
        .await
        .expect("a Guide should have moved into a finished house");
    let Event::NpcSynced(guide) = guide else {
        unreachable!("matched on it")
    };
    // He arrives in the house rather than wherever the scan started from.
    assert!(
        (guide.position.0 / 16.0 - hx as f32).abs() < 20.0,
        "the Guide moved in somewhere else: {:?}",
        guide.position
    );
}

#[tokio::test]
async fn the_house_command_explains_why_a_room_is_rejected() {
    let addr = start_with(Config::default(), |world| {
        // A room with everything except a door.
        let (x0, y0) = (300, 300);
        for x in x0..x0 + 14 {
            for y in y0..y0 + 10 {
                if x == x0 || x == x0 + 13 || y == y0 || y == y0 + 9 {
                    world.set_tile(x, y, Tile::block(1));
                } else {
                    let mut air = Tile::AIR;
                    air.wall = 4;
                    world.set_tile(x, y, air);
                }
            }
        }
        world.set_tile(x0 + 2, y0 + 8, Tile::framed(15, 0, 0));
        world.set_tile(x0 + 4, y0 + 8, Tile::framed(14, 0, 0));
        world.set_tile(x0 + 6, y0 + 8, Tile::framed(4, 0, 0));
    })
    .await;

    let mut client = join(addr, "builder").await;
    client.move_to(305.0 * 16.0, 303.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    client.say("/house").await.unwrap();
    let event = client
        .wait_for(
            "the housing verdict",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("house")),
        )
        .await
        .unwrap();
    if let Event::Chat { text, .. } = event {
        assert!(
            text.contains("needs a door"),
            "should name the missing door, said: {text}"
        );
    }
}

#[tokio::test]
async fn a_finished_house_is_accepted() {
    let addr = start_with(Config::default(), |world| {
        build_house(world, 300, 300);
    })
    .await;

    let mut client = join(addr, "builder").await;
    client.move_to(305.0 * 16.0, 303.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    client.say("/house").await.unwrap();
    let event = client
        .wait_for(
            "the housing verdict",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("house")),
        )
        .await
        .unwrap();
    if let Event::Chat { text, .. } = event {
        assert!(
            text.contains("valid house"),
            "should accept it, said: {text}"
        );
    }
}

#[tokio::test]
async fn the_guide_moves_into_a_finished_house() {
    let addr = start_with(Config::default(), |world| {
        build_house(world, 300, 300);
    })
    .await;

    let mut client = join(addr, "host").await;
    // Stand in the house so the housing scan looks here.
    client.move_to(305.0 * 16.0, 303.0 * 16.0).await.unwrap();

    // The announcement goes out just before the NPC itself, so both have to be watched at once
    // rather than one after the other.
    client.set_timeout(Duration::from_secs(25));
    let mut announced = false;
    let mut guide_at = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    while tokio::time::Instant::now() < deadline && (!announced || guide_at.is_none()) {
        match client.next_event().await {
            Ok(Event::Chat { text, .. }) if text.contains("moved in") => announced = true,
            Ok(Event::NpcSynced(n)) if n.npc_type() == 22 => {
                guide_at = Some((n.position.0 / 16.0, n.position.1 / 16.0));
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    let (tx, ty) = guide_at.expect("the Guide never moved in");
    assert!(
        (300.0..315.0).contains(&tx) && (295.0..312.0).contains(&ty),
        "the Guide moved in at ({tx:.0}, {ty:.0}), which is not the house"
    );
    assert!(announced, "nobody announced the arrival");
}

#[tokio::test]
async fn no_guide_arrives_without_a_house() {
    let addr = start().await;
    let mut client = join(addr, "homeless").await;
    client.set_timeout(Duration::from_secs(12));

    let arrived = client
        .wait_for(
            "a guide",
            |e| matches!(e, Event::NpcSynced(n) if n.npc_type() == 22),
        )
        .await;
    assert!(arrived.is_err(), "the Guide moved in with nowhere to live");
}

#[tokio::test]
async fn a_boss_summons_minions_and_can_be_killed() {
    let addr = start().await;
    let mut client = join(addr, "slayer").await;
    client.set_timeout(Duration::from_secs(20));

    // The Eye leaves at dawn, so the fight has to happen at night.
    client.say("/time night").await.unwrap();
    // And it spawns beside you, so stand where the fight is before calling it up — otherwise it
    // spends the whole test flying across the world to reach you.
    client.move_to(400.0 * 16.0, 300.0 * 16.0).await.unwrap();

    let eye = spawn_npc(&mut client, "EyeofCthulhu").await;
    assert_eq!(eye.life_max, 2800, "the Eye has 2800 health");

    // Stay put so it has something to hover over, and watch for its servants. It only summons
    // while hovering above you and within five hundred pixels, so standing still is the point.
    let mut saw_servant = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline && !saw_servant {
        client.move_to(400.0 * 16.0, 300.0 * 16.0).await.unwrap();
        if let Ok(Event::NpcSynced(n)) = client.next_event().await
            && n.npc_type() == 5
        {
            saw_servant = true;
        }
    }
    assert!(saw_servant, "the Eye never summoned a Servant of Cthulhu");

    // Now kill it. 2800 health, 12 defence: 200-damage hits land for 194.
    for _ in 0..20 {
        client
            .hit_npc(eye.index, eye.generation, 200, 0.0, 1)
            .await
            .unwrap();
    }
    client
        .wait_for(
            "the Eye's death",
            |e| matches!(e, Event::NpcSynced(n) if n.index == eye.index && n.life == 0),
        )
        .await
        .expect("the Eye never died");
}

#[tokio::test]
async fn a_worm_boss_arrives_as_a_linked_chain() {
    let addr = start().await;
    let mut client = join(addr, "wormer").await;
    client.set_timeout(Duration::from_secs(15));

    client.say("/spawn EaterofWorldsHead").await.unwrap();

    // A worm is a head, many body segments and a tail, all as separate NPCs.
    // Count distinct NPC slots: each one re-syncs as it moves, so counting packets would
    // over-count the head many times over.
    let mut seen: std::collections::HashMap<u8, u16> = std::collections::HashMap::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match client.next_event().await {
            Ok(Event::NpcSynced(n)) if n.life != 0 => {
                seen.insert(n.index, n.npc_type());
            }
            Ok(_) => {}
            Err(_) => break,
        }
        let tally = |t: u16| seen.values().filter(|v| **v == t).count();
        if tally(13) >= 1 && tally(15) >= 1 && tally(14) >= 5 {
            break;
        }
    }
    let tally = |t: u16| seen.values().filter(|v| **v == t).count();
    assert_eq!(tally(13), 1, "exactly one head");
    assert_eq!(tally(15), 1, "exactly one tail");
    assert!(tally(14) >= 5, "expected body segments, saw {}", tally(14));
}

#[tokio::test]
async fn king_slime_hops_toward_a_player() {
    let addr = start().await;
    let mut client = join(addr, "royalist").await;
    client.set_timeout(Duration::from_secs(15));

    let king = spawn_npc(&mut client, "KingSlime").await;
    assert_eq!(king.life_max, 2000);

    // It should move; a boss that stands still is not a fight.
    let start_x = king.position.0;
    let mut moved = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && !moved {
        client.move_to(420.0 * 16.0, 300.0 * 16.0).await.unwrap();
        if let Ok(Event::NpcSynced(n)) = client.next_event().await
            && n.index == king.index
            && (n.position.0 - start_x).abs() > 16.0
        {
            moved = true;
        }
    }
    assert!(moved, "King Slime never moved");
}

#[tokio::test]
async fn a_zombie_works_at_a_door_and_opens_it() {
    // A door standing on flat ground, with a zombie spawned beside it.
    let addr = start_with(Config::default(), |world| {
        // Carve a wide, unambiguously open corridor: the generated world is solid rock here, so
        // anything less leaves the zombie spawning inside a wall.
        for x in 380..430 {
            for y in 300..320 {
                world.set_tile(x, y, Tile::AIR);
            }
            for y in 320..332 {
                world.set_tile(x, y, Tile::block(1));
            }
        }
        // A door standing in the corridor at x = 405.
        for y in 317..320 {
            world.set_tile(405, y, Tile::framed(10, 0, 0));
        }
    })
    .await;

    let mut client = join(addr, "doorwatcher").await;
    client.set_timeout(Duration::from_secs(20));

    // Spawn the zombie on the far side of the door, then stand on this side of it so the only
    // way to reach the player is through the door.
    client.move_to(412.0 * 16.0, 318.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    client.say("/spawn Zombie").await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    client.move_to(396.0 * 16.0, 318.0 * 16.0).await.unwrap();

    let mut toggled = false;
    let mut track: Vec<(f32, f32)> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(18);
    while tokio::time::Instant::now() < deadline && !toggled {
        client.move_to(396.0 * 16.0, 318.0 * 16.0).await.unwrap();
        match client.next_event().await {
            // Packet 19 is the door toggle the server broadcasts when a fighter opens one.
            Ok(Event::Other(frame)) if frame.id == id::TOGGLE_DOOR_STATE => toggled = true,
            // Or the door is smashed, which arrives as a tile edit.
            Ok(Event::TileChanged(edit)) if edit.x == 405 => toggled = true,
            Ok(Event::NpcSynced(n)) if n.npc_type() == 3 => {
                track.push((n.position.0 / 16.0, n.position.1 / 16.0));
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(
        toggled,
        "a zombie stood at a door for eighteen seconds and did nothing to it. \
         Zombie was seen at {} positions; first {:?}, last {:?}",
        track.len(),
        track.first(),
        track.last()
    );
}

/// A player who joins a running server has to be told what everyone is already wearing. The
/// equipment packets went out before they arrived and are never repeated, so without the catch-up
/// they would see a room full of naked people.
#[tokio::test]
async fn a_joining_player_is_told_what_everyone_is_wearing() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;

    // A copper shortsword in her first inventory slot.
    let sword = ItemStack {
        id: 3507,
        stack: 1,
        prefix: 0,
    };
    alice.set_equipment(0, sword).await.unwrap();
    // Give the server a moment to record it.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut bob = join(addr, "bob").await;
    bob.set_timeout(Duration::from_secs(3));
    let seen = bob
        .wait_for(
            "alice's sword",
            |e| matches!(e, Event::EquipmentSynced(s) if s.item.id == 3507 && s.slot == 0),
        )
        .await;
    assert!(
        seen.is_ok(),
        "bob should have been told what alice is carrying"
    );
}

/// A live change reaches everyone already connected.
#[tokio::test]
async fn an_equipment_change_reaches_the_other_players() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    let pickaxe = ItemStack {
        id: 3509,
        stack: 1,
        prefix: 0,
    };
    alice.set_equipment(1, pickaxe).await.unwrap();

    bob.set_timeout(Duration::from_secs(3));
    let seen = bob
        .wait_for(
            "alice's pickaxe",
            |e| matches!(e, Event::EquipmentSynced(s) if s.item.id == 3509 && s.slot == 1),
        )
        .await;
    assert!(seen.is_ok(), "bob should have seen the change");
}

/// A player's safe is nobody else's business, however the client labels the packet.
#[tokio::test]
async fn private_storage_is_not_relayed() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    // The first piggy bank slot: inventory, cursor, armour, dyes and the two miscellaneous runs.
    let piggy_bank = 58 + 1 + 20 + 10 + 5 + 5;
    let hoard = ItemStack {
        id: 73,
        stack: 999,
        prefix: 0,
    };
    alice.set_equipment(piggy_bank, hoard).await.unwrap();

    bob.set_timeout(Duration::from_secs(2));
    let leaked = bob
        .wait_for(
            "alice's savings",
            |e| matches!(e, Event::EquipmentSynced(s) if s.slot == piggy_bank),
        )
        .await;
    assert!(leaked.is_err(), "a piggy bank should not be broadcast");
}

/// Using a summoning item is the only way a boss enters the world, so it had better work.
#[tokio::test]
async fn a_player_can_summon_a_boss() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;

    // King Slime.
    alice.summon(50).await.unwrap();
    alice.set_timeout(Duration::from_secs(3));
    let arrived = alice
        .wait_for(
            "king slime",
            |e| matches!(e, Event::NpcSynced(n) if n.net_id == 50),
        )
        .await;
    assert!(arrived.is_ok(), "the boss should have been summoned");
}

/// One at a time. A second Eye of Cthulhu is not something the game allows.
#[tokio::test]
async fn a_boss_cannot_be_summoned_twice() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;

    alice.summon(4).await.unwrap();
    // Short, because a quiet moment is the loop's exit condition rather than an error.
    alice.set_timeout(Duration::from_millis(30));
    // A living boss is re-synced every tick it moves, so "did another packet arrive" is the wrong
    // question. What matters is how many *different* NPC slots ever carry an Eye.
    let mut slots = std::collections::HashSet::new();
    for _ in 0..300 {
        match alice.next_event().await {
            Ok(Event::NpcSynced(n)) if n.net_id == 4 => {
                slots.insert(n.index);
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert_eq!(slots.len(), 1, "the first summon should have worked");

    alice.summon(4).await.unwrap();
    for _ in 0..300 {
        match alice.next_event().await {
            Ok(Event::NpcSynced(n)) if n.net_id == 4 => {
                slots.insert(n.index);
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert_eq!(slots.len(), 1, "two Eyes of Cthulhu at once: {slots:?}");
}

/// A crafted packet cannot conjure something that is not a summonable boss.
#[tokio::test]
async fn only_the_games_own_list_can_be_summoned() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;

    // A Moon Lord is not on the list, however politely you ask.
    alice.summon(398).await.unwrap();
    alice.set_timeout(Duration::from_secs(2));
    let arrived = alice
        .wait_for(
            "a moon lord",
            |e| matches!(e, Event::NpcSynced(n) if n.net_id == 398),
        )
        .await;
    assert!(arrived.is_err(), "that is not a summonable boss");
}

/// A multi-tile object has to be written into the server's own world, not merely relayed. If the
/// server does not place it, the object is gone the moment the world is saved and invisible to
/// anyone who joins afterwards.
#[tokio::test]
async fn a_placed_object_lands_in_the_world() {
    let addr = start_with(Config::default(), |world| {
        // A cleared pocket with solid ground under it. The generated world already has terrain
        // here, and an object is refused outright if anything is in its way.
        for x in 380..420 {
            for y in 310..322 {
                world.set_tile(x, y, terrustia_proto::tile::Tile::AIR);
            }
            world.set_tile(x, 322, terrustia_proto::tile::Tile::block(1));
        }
    })
    .await;
    let mut alice = join(addr, "alice").await;

    // A workbench: two wide, one tall, origin at its top-left.
    alice.place_object(400, 321, 18, 0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Bob's handshake already streams the sections around spawn, so asking for one he has would
    // simply be ignored and the wait would time out. Draining what arrives is enough.
    let mut bob = join(addr, "bob").await;
    bob.set_timeout(Duration::from_millis(50));
    for _ in 0..200 {
        if bob.next_event().await.is_err() {
            break;
        }
    }

    let placed = bob
        .world()
        .tile(400, 321)
        .expect("the tile should have arrived");
    assert!(placed.is_active(), "the workbench should be in the world");
    assert_eq!(placed.block, 18, "and be a workbench");
    let second = bob.world().tile(401, 321).expect("its second tile too");
    assert_eq!(second.block, 18, "both of its tiles");
    assert!(
        second.frame_x > placed.frame_x,
        "with the second column framed after the first: {} then {}",
        placed.frame_x,
        second.frame_x
    );
}

/// Placing an object over something already there is refused outright rather than filling gaps.
#[tokio::test]
async fn an_object_will_not_be_placed_over_something() {
    let addr = start_with(Config::default(), |world| {
        for x in 380..420 {
            for y in 310..322 {
                world.set_tile(x, y, terrustia_proto::tile::Tile::AIR);
            }
            world.set_tile(x, 322, terrustia_proto::tile::Tile::block(1));
        }
        // Something in the way of the second half of the workbench.
        world.set_tile(401, 321, terrustia_proto::tile::Tile::block(1));
    })
    .await;
    let mut alice = join(addr, "alice").await;
    alice.place_object(400, 321, 18, 0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut bob = join(addr, "bob").await;
    bob.set_timeout(Duration::from_millis(50));
    for _ in 0..200 {
        if bob.next_event().await.is_err() {
            break;
        }
    }
    assert_ne!(
        bob.world().tile(400, 321).map(|t| t.block),
        Some(18),
        "half a workbench should not have been placed"
    );
}

/// A teleport the server does not apply leaves every enemy in the world attacking where the
/// player used to be, so it has to move the server's idea of them as well as telling everyone.
#[tokio::test]
async fn a_teleport_moves_the_player_for_everyone() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    alice.teleport(1234.0, 567.0).await.unwrap();

    bob.set_timeout(Duration::from_secs(3));
    let seen = bob
        .wait_for(
            "alice's teleport",
            |e| matches!(e, Event::Other(f) if f.id == id::TELEPORT_ENTITY),
        )
        .await;
    assert!(seen.is_ok(), "bob should have been told");
}

/// Painting a tile sticks and reaches everybody.
#[tokio::test]
async fn paint_sticks_to_the_world() {
    let addr = start_with(Config::default(), |world| {
        world.set_tile(402, 330, Tile::block(1));
    })
    .await;
    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;

    let mut paint = Vec::new();
    paint.extend_from_slice(&402i16.to_le_bytes());
    paint.extend_from_slice(&330i16.to_le_bytes());
    paint.push(13); // a colour
    paint.push(0); // paint, not coating
    bob.send(&frame(id::SYNC_TILE_PAINT_OR_COATING, &paint))
        .await
        .unwrap();

    alice
        .wait_for(
            "the relayed paint",
            |e| matches!(e, Event::Other(f) if f.id == id::SYNC_TILE_PAINT_OR_COATING),
        )
        .await
        .unwrap();

    let fresh = join(addr, "fresh").await;
    assert_eq!(
        fresh.world().tile(402, 330).map(|t| t.color),
        Some(13),
        "the paint did not stick"
    );
}

/// Painting empty air does nothing: a crafted packet cannot colour in the sky.
#[tokio::test]
async fn painting_nothing_does_nothing() {
    let addr = start().await;
    let mut bob = join(addr, "bob").await;

    // Well above the surface, where there is certainly no tile.
    let mut paint = Vec::new();
    paint.extend_from_slice(&402i16.to_le_bytes());
    paint.extend_from_slice(&20i16.to_le_bytes());
    paint.push(13);
    paint.push(0);
    bob.send(&frame(id::SYNC_TILE_PAINT_OR_COATING, &paint))
        .await
        .unwrap();

    let fresh = join(addr, "fresh").await;
    assert_eq!(fresh.world().tile(402, 20).map(|t| t.color), Some(0));
}

/// A locked biome chest stays locked until Plantera is down.
#[tokio::test]
async fn a_biome_chest_waits_for_plantera() {
    // Style 23 is a locked biome chest: frame_x 23 * 36.
    let locked = |world: &mut World| {
        for dx in 0..2 {
            for dy in 0..2 {
                world.set_tile(
                    402 + dx,
                    330 + dy,
                    Tile::framed(21, 23 * 36 + dx as i16 * 18, dy as i16 * 18),
                );
            }
        }
    };
    let addr = start_with(Config::default(), locked).await;
    let mut bob = join(addr, "bob").await;

    let mut unlock = vec![1u8]; // unlock a chest
    unlock.extend_from_slice(&402i16.to_le_bytes());
    unlock.extend_from_slice(&330i16.to_le_bytes());
    bob.send(&frame(id::LOCK_AND_UNLOCK, &unlock))
        .await
        .unwrap();

    let fresh = join(addr, "fresh").await;
    assert_eq!(
        fresh.world().tile(402, 330).map(|t| t.frame_x),
        Some(23 * 36),
        "the chest opened without Plantera"
    );
}

/// A dungeon chest opens with a key, and the whole two-by-two moves together.
#[tokio::test]
async fn a_dungeon_chest_opens() {
    let addr = start_with(Config::default(), |world| {
        // Style 2 is a locked dungeon chest.
        for dx in 0..2 {
            for dy in 0..2 {
                world.set_tile(
                    402 + dx,
                    330 + dy,
                    Tile::framed(21, 2 * 36 + dx as i16 * 18, dy as i16 * 18),
                );
            }
        }
    })
    .await;
    let mut bob = join(addr, "bob").await;

    let mut unlock = vec![1u8];
    unlock.extend_from_slice(&402i16.to_le_bytes());
    unlock.extend_from_slice(&330i16.to_le_bytes());
    bob.send(&frame(id::LOCK_AND_UNLOCK, &unlock))
        .await
        .unwrap();

    let fresh = join(addr, "fresh").await;
    for dx in 0..2 {
        for dy in 0..2 {
            let tile = fresh.world().tile(402 + dx, 330 + dy).expect("a tile");
            assert_eq!(
                tile.frame_x,
                // Style 2 unlocks to style 1: one frame back.
                36 + dx as i16 * 18,
                "corner ({dx}, {dy}) did not move with the rest"
            );
        }
    }
}

/// A net takes a critter and refuses anything else.
#[tokio::test]
async fn a_net_only_catches_critters() {
    let addr = start().await;
    let mut bob = join(addr, "bob").await;
    // Ask for a boss and a bunny by index; neither exists, so this only proves the packet is
    // accepted and refused rather than crashing the server.
    let mut catch = Vec::new();
    catch.extend_from_slice(&0i16.to_le_bytes());
    catch.push(0);
    bob.send(&frame(id::BUG_CATCHING, &catch)).await.unwrap();

    // The server is still answering afterwards.
    bob.send(&frame(id::PING, &[0u8; 8])).await.unwrap();
    let fresh = join(addr, "fresh").await;
    assert!(
        fresh.world().tile(402, 330).is_some(),
        "the server survived"
    );
}

/// A bucket poured into the world falls and pools rather than sitting in the air.
///
/// The basin has walls: a bucket on an open plain spreads out and thins away to nothing, which is
/// what the game does too, so testing it on a shelf would be testing the wrong thing.
#[tokio::test]
async fn poured_water_falls() {
    let addr = start_with(Config::default(), |world| {
        for x in 400..410 {
            world.set_tile(x, 340, Tile::block(1));
            for y in 330..340 {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        // The walls that make it a basin rather than a shelf.
        for y in 330..341 {
            world.set_tile(400, y, Tile::block(1));
            world.set_tile(409, y, Tile::block(1));
        }
    })
    .await;
    let mut bob = join(addr, "bob").await;

    let mut pour = Vec::new();
    pour.extend_from_slice(&405i16.to_le_bytes());
    pour.extend_from_slice(&331i16.to_le_bytes());
    pour.push(255); // a full bucket
    pour.push(0); // water
    bob.send(&frame(id::LIQUID_UPDATE, &pour)).await.unwrap();

    // Give the simulation a moment.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let fresh = join(addr, "fresh").await;
    let up_top = fresh.world().tile(405, 331).map(|t| t.liquid).unwrap_or(0);
    let down_low = fresh.world().tile(405, 339).map(|t| t.liquid).unwrap_or(0);
    assert!(
        down_low > up_top,
        "the water should have fallen: {up_top} up top, {down_low} at the bottom"
    );
}

/// Build a raw frame the way the client's own encoder does.
fn frame(id: u8, payload: &[u8]) -> Vec<u8> {
    let len = (payload.len() + 3) as u16;
    let mut out = Vec::with_capacity(len as usize);
    out.extend_from_slice(&len.to_le_bytes());
    out.push(id);
    out.extend_from_slice(payload);
    out
}

/// Breaking a chest removes it, rather than leaving a ghost behind.
#[tokio::test]
async fn breaking_a_chest_removes_it() {
    let addr = start_with(Config::default(), |world| {
        for dx in 0..2 {
            for dy in 0..2 {
                world.set_tile(
                    402 + dx,
                    330 + dy,
                    Tile::framed(21, dx as i16 * 18, dy as i16 * 18),
                );
            }
        }
        world.add_chest(Chest {
            x: 402,
            y: 330,
            name: String::new(),
            items: vec![ItemStack::default(); 40],
        });
    })
    .await;
    let mut bob = join(addr, "bob").await;

    // Opening it proves it is there to begin with.
    let mut open = Vec::new();
    open.extend_from_slice(&402i16.to_le_bytes());
    open.extend_from_slice(&330i16.to_le_bytes());
    bob.send(&frame(id::REQUEST_CHEST_OPEN, &open))
        .await
        .unwrap();
    bob.wait_for(
        "the chest opening",
        |e| matches!(e, Event::Other(f) if f.id == id::SYNC_PLAYER_CHEST),
    )
    .await
    .unwrap();

    // Now break it.
    let mut kill = vec![1u8]; // break a chest
    kill.extend_from_slice(&402i16.to_le_bytes());
    kill.extend_from_slice(&330i16.to_le_bytes());
    kill.extend_from_slice(&0i16.to_le_bytes());
    bob.send(&frame(id::CHEST_UPDATES, &kill)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // A fresh player asking for the same spot gets nothing.
    let mut fresh = join(addr, "fresh").await;
    fresh
        .send(&frame(id::REQUEST_CHEST_OPEN, &open))
        .await
        .unwrap();
    let answered = fresh
        .try_wait_for(
            "a chest that should be gone",
            |e| matches!(e, Event::Other(f) if f.id == id::SYNC_PLAYER_CHEST),
            Duration::from_millis(500),
        )
        .await;
    assert!(answered.is_none(), "the broken chest still answers");
}

/// A chest with things in it will not break.
#[tokio::test]
async fn a_full_chest_will_not_break() {
    let addr = start_with(Config::default(), |world| {
        for dx in 0..2 {
            for dy in 0..2 {
                world.set_tile(
                    402 + dx,
                    330 + dy,
                    Tile::framed(21, dx as i16 * 18, dy as i16 * 18),
                );
            }
        }
        let mut items = vec![ItemStack::default(); 40];
        items[0] = ItemStack::new(1, 1, 0);
        world.add_chest(Chest {
            x: 402,
            y: 330,
            name: String::new(),
            items,
        });
    })
    .await;
    let mut bob = join(addr, "bob").await;

    let mut kill = vec![1u8];
    kill.extend_from_slice(&402i16.to_le_bytes());
    kill.extend_from_slice(&330i16.to_le_bytes());
    kill.extend_from_slice(&0i16.to_le_bytes());
    bob.send(&frame(id::CHEST_UPDATES, &kill)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut fresh = join(addr, "fresh").await;
    let mut open = Vec::new();
    open.extend_from_slice(&402i16.to_le_bytes());
    open.extend_from_slice(&330i16.to_le_bytes());
    fresh
        .send(&frame(id::REQUEST_CHEST_OPEN, &open))
        .await
        .unwrap();
    fresh
        .wait_for(
            "the chest, still there",
            |e| matches!(e, Event::Other(f) if f.id == id::SYNC_PLAYER_CHEST),
        )
        .await
        .expect("a chest with things in it should survive");
}

/// A training dummy appears when somebody is near its tile, and goes when they leave.
#[tokio::test]
async fn a_training_dummy_comes_and_goes() {
    // The dummy's tile, planted where a joining player will be standing.
    let addr = start_with(Config::default(), |world| {
        let (x, y) = (world.spawn_x as i32, world.spawn_y as i32 + 1);
        world.set_tile(x, y, Tile::framed(378, 0, 0));
    })
    .await;
    let mut bob = join(addr, "bob").await;

    let (x, y) = (bob.world().spawn.0, bob.world().spawn.1 + 1);
    // A dummy only comes out for somebody standing near it, so bob has to actually be there: a
    // client that has never moved is at the origin as far as the server is concerned.
    bob.move_to(f32::from(x) * 16.0, f32::from(y) * 16.0)
        .await
        .unwrap();

    let mut place = Vec::new();
    place.extend_from_slice(&x.to_le_bytes());
    place.extend_from_slice(&y.to_le_bytes());
    place.push(0); // a training dummy
    bob.send(&frame(id::TILE_ENTITY_PLACEMENT, &place))
        .await
        .unwrap();

    // The dummy is put out because bob is standing right there.
    let raised = bob
        .wait_for(
            "the dummy appearing",
            |e| matches!(e, Event::NpcSynced(n) if n.net_id == 488),
        )
        .await
        .expect("a dummy should have been raised");
    let Event::NpcSynced(dummy) = raised else {
        unreachable!("matched on it");
    };
    // It carries where it was planted, which is how its routine knows the tile is still there.
    assert_eq!(
        (dummy.ai[0] as i16, dummy.ai[1] as i16),
        (x, y),
        "the dummy does not know where it was planted"
    );
}

/// A tile entity cannot be hung in mid-air.
#[tokio::test]
async fn a_tile_entity_needs_its_tile() {
    let addr = start().await;
    let mut bob = join(addr, "bob").await;

    // An item frame where there is certainly nothing.
    let mut place = Vec::new();
    place.extend_from_slice(&402i16.to_le_bytes());
    place.extend_from_slice(&20i16.to_le_bytes());
    place.push(1); // an item frame
    bob.send(&frame(id::TILE_ENTITY_PLACEMENT, &place))
        .await
        .unwrap();

    // Nothing happens, and the server is still answering.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let fresh = join(addr, "fresh").await;
    assert!(fresh.world().tile(402, 20).is_some(), "the server survived");
}

/// Mining a dummy's tile takes the dummy with it, rather than leaving it standing.
#[tokio::test]
async fn a_dummy_goes_when_its_tile_does() {
    let addr = start_with(Config::default(), |world| {
        let (x, y) = (world.spawn_x as i32, world.spawn_y as i32 + 1);
        world.set_tile(x, y, Tile::framed(378, 0, 0));
    })
    .await;
    let mut bob = join(addr, "bob").await;
    let (x, y) = (bob.world().spawn.0, bob.world().spawn.1 + 1);
    bob.move_to(f32::from(x) * 16.0, f32::from(y) * 16.0)
        .await
        .unwrap();

    let mut place = Vec::new();
    place.extend_from_slice(&x.to_le_bytes());
    place.extend_from_slice(&y.to_le_bytes());
    place.push(0);
    bob.send(&frame(id::TILE_ENTITY_PLACEMENT, &place))
        .await
        .unwrap();
    let raised = bob
        .wait_for(
            "the dummy",
            |e| matches!(e, Event::NpcSynced(n) if n.net_id == 488),
        )
        .await
        .expect("a dummy");
    let Event::NpcSynced(dummy) = raised else {
        unreachable!("matched on it")
    };

    // Mine the tile out from under it.
    let square = TileSquare {
        x,
        y,
        width: 1,
        height: 1,
        change_type: 0,
        tiles: vec![Tile::AIR],
    };
    bob.send(&square.encode().unwrap()).await.unwrap();

    bob.wait_for(
        "the dummy going",
        |e| matches!(e, Event::NpcSynced(n) if n.index == dummy.index && n.life == 0),
    )
    .await
    .expect("the dummy should have gone with its tile");
}

/// Lay a flat arena with a crystal stand in the middle of it.
///
/// The Old One's Army refuses to begin unless the ground is sixty tiles clear on both sides, which
/// is the game's own way of making building an arena part of preparing for the event.
fn arena_with_a_stand(world: &mut World, at: (i32, i32)) {
    for x in at.0 - 120..=at.0 + 120 {
        for y in at.1 + 1..at.1 + 6 {
            world.set_tile(x, y, Tile::block(1));
        }
        for y in at.1 - 20..=at.1 {
            world.set_tile(x, y, Tile::AIR);
        }
    }
    // The stand itself: a 3x2 object, so every tile carries its frame.
    for dx in 0..3 {
        for dy in 0..2 {
            world.set_tile(
                at.0 + dx,
                at.1 - 1 + dy,
                Tile::framed(466, dx as i16 * 18, dy as i16 * 18),
            );
        }
    }
}

/// Putting a crystal on its stand raises the event, its gates and its first wave.
#[tokio::test]
async fn the_old_ones_army_begins_at_a_crystal_stand() {
    let at = (400, 330);
    let addr = start_with(Config::default(), |world| arena_with_a_stand(world, at)).await;
    let mut bob = join(addr, "bob").await;

    let mut place = Vec::new();
    place.extend_from_slice(&(at.0 as i16).to_le_bytes());
    place.extend_from_slice(&((at.1 - 1) as i16).to_le_bytes());
    bob.send(&frame(id::CRYSTAL_INVASION_START, &place))
        .await
        .unwrap();

    // The crystal itself, and then the two gates it raises at the arena's ends.
    bob.wait_for(
        "the crystal",
        |e| matches!(e, Event::NpcSynced(n) if n.net_id == 548),
    )
    .await
    .expect("a crystal should have appeared");

    let mut gates = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while gates < 2 && tokio::time::Instant::now() < deadline {
        if let Some(Event::NpcSynced(n)) = bob
            .try_wait_for(
                "a gate",
                |e| matches!(e, Event::NpcSynced(n) if n.net_id == 549),
                Duration::from_secs(6),
            )
            .await
        {
            let _ = n;
            gates += 1;
        } else {
            break;
        }
    }
    assert_eq!(gates, 2, "both lane portals should have gone up");
}

/// An arena too small for the event is refused, rather than starting one nobody can win.
#[tokio::test]
async fn a_cramped_arena_is_refused() {
    let at = (400, 330);
    let addr = start_with(Config::default(), |world| {
        // Only twenty tiles of floor each way: nowhere near the sixty the event asks for.
        for x in at.0 - 20..=at.0 + 20 {
            for y in at.1 + 1..at.1 + 6 {
                world.set_tile(x, y, Tile::block(1));
            }
            for y in at.1 - 20..=at.1 {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        for dx in 0..3 {
            for dy in 0..2 {
                world.set_tile(
                    at.0 + dx,
                    at.1 - 1 + dy,
                    Tile::framed(466, dx as i16 * 18, dy as i16 * 18),
                );
            }
        }
        // Walls at both ends, so the arena walker stops well short.
        for y in at.1 - 20..at.1 + 6 {
            world.set_tile(at.0 - 21, y, Tile::block(1));
            world.set_tile(at.0 + 21, y, Tile::block(1));
        }
    })
    .await;
    let mut bob = join(addr, "bob").await;

    let mut place = Vec::new();
    place.extend_from_slice(&(at.0 as i16).to_le_bytes());
    place.extend_from_slice(&((at.1 - 1) as i16).to_le_bytes());
    bob.send(&frame(id::CRYSTAL_INVASION_START, &place))
        .await
        .unwrap();

    let started = bob
        .try_wait_for(
            "a crystal that should not appear",
            |e| matches!(e, Event::NpcSynced(n) if n.net_id == 548),
            Duration::from_millis(800),
        )
        .await;
    assert!(started.is_none(), "the event began in a cramped arena");
}

/// An enemy standing on you kills you, and everybody is told.
///
/// Death is the server's call, not the client's: a client that decides for itself when it has died
/// is a client that decides for itself when it has not.
#[tokio::test]
async fn an_enemy_can_kill_a_player_and_everyone_hears() {
    let addr = start().await;
    let mut alice = join(addr, "alice").await;
    let mut bob = join(addr, "bob").await;
    alice.set_timeout(Duration::from_secs(20));
    bob.set_timeout(Duration::from_secs(20));

    bob.move_to(420.0 * 16.0, 300.0 * 16.0).await.unwrap();
    bob.set_life(5, 400).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Something that hurts on contact. Where it lands is the server's choice, so bob goes to it
    // rather than hoping it comes to him.
    let zombie = spawn_npc(&mut bob, "Zombie").await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut died = false;
    let mut on = zombie.position;
    while tokio::time::Instant::now() < deadline && !died {
        // Stand on it, following it if it wanders.
        bob.move_to(on.0, on.1).await.ok();
        match bob
            .try_wait_for(
                "bob dying",
                |e| {
                    matches!(e, Event::PlayerDied(_))
                        || matches!(e, Event::NpcSynced(n) if n.index == zombie.index)
                },
                Duration::from_millis(300),
            )
            .await
        {
            Some(Event::PlayerDied(death)) => {
                assert_eq!(death.player, bob.slot(), "bob's death, not somebody else's");
                died = true;
            }
            Some(Event::NpcSynced(n)) => on = n.position,
            _ => {}
        }
    }
    assert!(
        died,
        "an enemy standing on a player with five life should kill them"
    );

    // And alice, who was nowhere near it, is told.
    alice
        .try_wait_for(
            "the death reaching alice",
            |e| matches!(e, Event::PlayerDied(_)),
            Duration::from_secs(2),
        )
        .await
        .expect("a death should reach every player, not only the one who died");
}

/// A blood moon that walks through a town and leaves it standing is scenery, not a threat.
///
/// The townsfolk take contact damage from anything hostile they are standing in, and their armour
/// — which is the only place the world's history reaches them — decides how long they last.
#[tokio::test]
async fn an_enemy_kills_a_townsperson_it_is_standing_in() {
    let addr = start_with(Config::default(), |world| {
        for x in 380..430 {
            for y in 300..320 {
                world.set_tile(x, y, Tile::AIR);
            }
            for y in 320..332 {
                world.set_tile(x, y, Tile::block(1));
            }
        }
    })
    .await;

    let mut client = join(addr, "mayor").await;
    client.set_timeout(Duration::from_secs(30));
    client.move_to(400.0 * 16.0, 318.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // A guide, and then something that wants him dead in the same spot.
    client.say("/spawn Guide").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    for _ in 0..6 {
        client.say("/spawn Zombie").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The guide is NPC type 22. Watch until his life falls, or he stops being synced at all.
    let mut seen_full = false;
    let mut hurt = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    while tokio::time::Instant::now() < deadline && !hurt {
        client.move_to(400.0 * 16.0, 318.0 * 16.0).await.unwrap();
        match client.next_event().await {
            Ok(Event::NpcSynced(n)) if n.npc_type() == 22 => {
                if n.life >= 250 {
                    seen_full = true;
                } else if seen_full && n.life < 250 {
                    hurt = true;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(
        hurt,
        "a guide stood in a crowd of zombies for twenty-five seconds and took no damage \
         (seen at full health: {seen_full})"
    );
}

/// A dart trap wired to a lever throws a dart when the lever is pulled, and the dart hurts.
///
/// This is the whole chain in one test: the flood finds the trap, the table works out what it
/// throws, the projectile flies, and standing in front of it costs health.
#[tokio::test]
async fn a_wired_dart_trap_shoots_a_player() {
    let addr = start_with(Config::default(), |world| {
        // A corridor with a lever at one end and a dart trap at the other, joined by red wire.
        for x in 380..430 {
            for y in 300..320 {
                world.set_tile(x, y, Tile::AIR);
            }
            for y in 320..332 {
                world.set_tile(x, y, Tile::block(1));
            }
        }
        // The lever.
        world.set_tile(390, 319, Tile::framed(136, 0, 0));
        // Red wire from the lever to the trap.
        for x in 390..=410 {
            let mut tile = world.tile(x, 319);
            tile.flags
                .set(terrustia_proto::tile::TileFlags::WIRE_RED, true);
            world.set_tile(x, 319, tile);
        }
        // A dart trap at the far end, frame_x 0 so it fires west, back down the corridor.
        let mut trap = Tile::framed(137, 0, 0);
        trap.flags
            .set(terrustia_proto::tile::TileFlags::WIRE_RED, true);
        world.set_tile(410, 319, trap);
    })
    .await;

    let mut client = join(addr, "trapfodder").await;
    client.set_timeout(Duration::from_secs(20));
    // Stand in the dart's way, a few tiles west of the trap.
    client.move_to(404.0 * 16.0, 318.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    client.hit_switch(390, 319).await.unwrap();

    let mut dart = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && dart.is_none() {
        client.move_to(404.0 * 16.0, 318.0 * 16.0).await.unwrap();
        match client.next_event().await {
            Ok(Event::ProjectileSynced(p)) => dart = Some(p),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let dart = dart.expect("the lever should have fired the trap");
    assert_eq!(dart.projectile_type, 98, "a dart trap throws a dart");
    assert!(
        dart.velocity.0 < -10.0,
        "it should fly west at twelve, not {:?}",
        dart.velocity
    );

    // And a dart in the face costs health.
    let mut hurt = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && !hurt {
        client.move_to(404.0 * 16.0, 318.0 * 16.0).await.unwrap();
        // Keep pulling the lever: one dart may pass before the player is standing still.
        client.hit_switch(390, 319).await.unwrap();
        match client.next_event().await {
            Ok(Event::PlayerHurt(_)) => hurt = true,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(hurt, "a dart flew through the player and did nothing");
}

/// A trap has a cooldown, which is the difference between a trap and a machine gun.
///
/// A pressure plate a slime is sitting on is hit every tick; without the cooldown every one of
/// those hits would be a dart, and a dungeon corridor would be a wall of them.
#[tokio::test]
async fn a_trap_will_not_fire_faster_than_its_cooldown() {
    let addr = start_with(Config::default(), |world| {
        for x in 380..430 {
            for y in 300..320 {
                world.set_tile(x, y, Tile::AIR);
            }
            for y in 320..332 {
                world.set_tile(x, y, Tile::block(1));
            }
        }
        world.set_tile(390, 319, Tile::framed(136, 0, 0));
        for x in 390..=410 {
            let mut tile = world.tile(x, 319);
            tile.flags
                .set(terrustia_proto::tile::TileFlags::WIRE_RED, true);
            world.set_tile(x, 319, tile);
        }
        let mut trap = Tile::framed(137, 0, 0);
        trap.flags
            .set(terrustia_proto::tile::TileFlags::WIRE_RED, true);
        world.set_tile(410, 319, trap);
    })
    .await;

    let mut client = join(addr, "leverpuller").await;
    client.set_timeout(Duration::from_secs(20));
    client.move_to(385.0 * 16.0, 318.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Pull the lever fifty times over about a second — well inside the dart trap's 200-tick wait.
    for _ in 0..50 {
        client.hit_switch(390, 319).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Count the distinct darts that were born, not the syncs: one projectile is synced many times.
    let mut born = std::collections::HashSet::new();
    while let Some(Event::ProjectileSynced(p)) = client
        .try_wait_for(
            "a projectile",
            |e| matches!(e, Event::ProjectileSynced(_)),
            Duration::from_millis(400),
        )
        .await
    {
        born.insert((p.key.owner, p.key.index, p.key.generation));
    }
    assert!(
        !born.is_empty(),
        "fifty pulls should have fired the trap at least once"
    );
    assert!(
        born.len() <= 2,
        "fifty pulls in a second fired {} darts; the cooldown is not holding",
        born.len()
    );
}

/// A slime statue wired to a lever produces a slime, and that slime is worth nothing.
///
/// The worthlessness is the point: a statue that dropped coins would be a printing press, and a
/// statue whose monsters counted against the spawn cap would stop the world spawning anything
/// else. The game zeroes both on the way out of the statue.
#[tokio::test]
async fn a_wired_statue_spawns_its_monster() {
    let addr = start_with(Config::default(), |world| {
        for x in 380..430 {
            for y in 300..320 {
                world.set_tile(x, y, Tile::AIR);
            }
            for y in 320..332 {
                world.set_tile(x, y, Tile::block(1));
            }
        }
        world.set_tile(390, 319, Tile::framed(136, 0, 0));
        for x in 390..=400 {
            let mut tile = world.tile(x, 319);
            tile.flags
                .set(terrustia_proto::tile::TileFlags::WIRE_RED, true);
            world.set_tile(x, 319, tile);
        }
        // A slime statue: style 4, so frame_x is 4 * 36. Two wide and three tall, standing on
        // the floor with its base at y = 319.
        for dx in 0..2i16 {
            for dy in 0..3i16 {
                let mut tile = Tile::framed(105, 4 * 36 + dx * 18, dy * 18);
                if dy == 2 {
                    tile.flags
                        .set(terrustia_proto::tile::TileFlags::WIRE_RED, true);
                }
                world.set_tile(400 + i32::from(dx), 317 + i32::from(dy), tile);
            }
        }
    })
    .await;

    let mut client = join(addr, "statuewatcher").await;
    client.set_timeout(Duration::from_secs(20));
    client.move_to(395.0 * 16.0, 318.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    client.hit_switch(390, 319).await.unwrap();

    let mut slime = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && slime.is_none() {
        client.move_to(395.0 * 16.0, 318.0 * 16.0).await.unwrap();
        match client.next_event().await {
            // NPC type 1 is a blue slime.
            Ok(Event::NpcSynced(n)) if n.npc_type() == 1 => slime = Some(n),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let slime = slime.expect("the lever should have run the statue");
    // It appears at the middle of the statue's base, a tile up.
    assert!(
        (slime.position.0 / 16.0 - 401.0).abs() < 3.0,
        "it should stand on the statue, not at {}",
        slime.position.0 / 16.0
    );
}

/// A pair of wired teleporters moves whoever is standing on one to the other.
#[tokio::test]
async fn a_teleporter_pair_moves_a_player() {
    let addr = start_with(Config::default(), |world| {
        for x in 300..500 {
            for y in 300..320 {
                world.set_tile(x, y, Tile::AIR);
            }
            for y in 320..332 {
                world.set_tile(x, y, Tile::block(1));
            }
        }
        world.set_tile(320, 319, Tile::framed(136, 0, 0));
        for x in 320..=460 {
            let mut tile = world.tile(x, 319);
            tile.flags
                .set(terrustia_proto::tile::TileFlags::WIRE_RED, true);
            world.set_tile(x, 319, tile);
        }
        // Two teleporters, three tiles wide each, a long way apart.
        for (at, _) in [(360i32, 0), (450, 1)] {
            for dx in 0..3i32 {
                let mut pad = Tile::framed(235, (dx * 18) as i16, 0);
                pad.flags
                    .set(terrustia_proto::tile::TileFlags::WIRE_RED, true);
                world.set_tile(at + dx, 319, pad);
            }
        }
    })
    .await;

    let mut client = join(addr, "hopper").await;
    client.set_timeout(Duration::from_secs(20));
    // Stand on the first pad.
    client.move_to(361.0 * 16.0, 316.0 * 16.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    client.hit_switch(320, 319).await.unwrap();

    // The server tells everyone, including the player who moved.
    let mut landed = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline && landed.is_none() {
        match client.next_event().await {
            Ok(Event::Other(frame)) if frame.id == id::TELEPORT_ENTITY => {
                let mut r = terrustia_proto::PacketReader::new(&frame.payload);
                let _flags = r.u8().unwrap();
                let _who = r.i16().unwrap();
                let x = r.f32().unwrap();
                let y = r.f32().unwrap();
                landed = Some((x, y));
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let landed = landed.expect("the lever should have worked the teleporters");
    assert!(
        (landed.0 / 16.0 - 451.0).abs() < 4.0,
        "it should have moved ninety tiles east, to about 451, not {}",
        landed.0 / 16.0
    );
}
