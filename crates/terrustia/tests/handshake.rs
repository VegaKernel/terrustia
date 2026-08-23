//! End-to-end handshake and multiplayer sync, driven by a fake client over a real TCP socket.
//!
//! A real Terraria client reports protocol problems by hanging rather than erroring, so these
//! tests assert on the exact packet sequence the client's state machine waits for.

use std::{net::SocketAddr, time::Duration};

use bytes::BytesMut;
use terrustia::{
    config::Config,
    game::{GameServer, ServerEvent},
    net::{Frame, TerrariaCodec, listener},
    world::{World, worldgen},
};
use terrustia_proto::{
    PacketWriter, id,
    net_module::MODULE_TEXT,
    packets::{self, PlayerSpawn},
    reader::PacketReader,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::timeout,
};
use tokio_util::codec::Decoder;

/// Generous enough for a debug build to encode a few sections, short enough to fail a hang fast.
const RECV_TIMEOUT: Duration = Duration::from_secs(10);

fn init_logs() {
    use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt};
    if let Ok(spec) = std::env::var("TERRUSTIA_LOG")
        && let Ok(filter) = spec.parse::<Targets>()
    {
        let _ = tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(filter)
            .try_init();
    }
}

/// Start a server on an ephemeral port and return its address.
async fn start_server() -> SocketAddr {
    start_server_with(|_| {}).await
}

/// The same, letting a test shape the world first.
///
/// A test that needs a particular tile has to put it there. Relying on the generator happening to
/// leave one is a test that fails the next time the generator changes — which is exactly what
/// happened when the world started having caves in it.
async fn start_server_with<F: FnOnce(&mut World)>(prepare: F) -> SocketAddr {
    init_logs();
    let config = Config {
        // Small enough that generation and section encoding stay quick under a debug build.
        world_width: 800,
        world_height: 600,
        seed: 12345,
        motd: String::new(),
        ..Config::default()
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let mut world = worldgen::generate(
        config.world_width,
        config.world_height,
        config.world_name.clone(),
        config.seed,
    );
    prepare(&mut world);

    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    tokio::spawn(listener::run(listener, config, tx));
    addr
}

struct FakeClient {
    stream: TcpStream,
    buf: BytesMut,
    codec: TerrariaCodec,
    /// Everything received between the tile request and `InitialSpawn`.
    ///
    /// The server refuses to resend a section a client already has, exactly as vanilla's
    /// `TileSections` check does, so a test that wants to inspect world content has to read the
    /// sections streamed during the handshake rather than asking again.
    handshake_frames: Vec<Frame>,
}

impl FakeClient {
    async fn connect(addr: SocketAddr) -> Self {
        Self {
            stream: TcpStream::connect(addr).await.unwrap(),
            buf: BytesMut::with_capacity(16 * 1024),
            codec: TerrariaCodec,
            handshake_frames: Vec::new(),
        }
    }

    async fn send(&mut self, frame: Vec<u8>) {
        self.stream.write_all(&frame).await.unwrap();
    }

    async fn recv(&mut self) -> Frame {
        loop {
            if let Some(frame) = self.codec.decode(&mut self.buf).unwrap() {
                return frame;
            }
            let n = timeout(RECV_TIMEOUT, self.stream.read_buf(&mut self.buf))
                .await
                .expect("timed out waiting for a packet")
                .expect("socket read failed");
            assert_ne!(n, 0, "server closed the connection unexpectedly");
        }
    }

    /// Collect packets until one with `id` arrives, returning everything including it.
    async fn recv_until(&mut self, id: u8) -> Vec<Frame> {
        let mut frames = Vec::new();
        loop {
            let frame = self.recv().await;
            let found = frame.id == id;
            frames.push(frame);
            if found {
                return frames;
            }
            assert!(frames.len() < 500, "never saw packet {id}");
        }
    }

    /// Try to read a packet, returning None if none completes before `wait` elapses.
    ///
    /// Must keep reading across partial deliveries: a section packet is several kilobytes and
    /// never arrives in one read, so returning None on a short read would report "nothing sent"
    /// for a packet that is merely still in flight.
    async fn try_recv(&mut self, wait: Duration) -> Option<Frame> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(frame) = self.codec.decode(&mut self.buf).unwrap() {
                return Some(frame);
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match timeout(remaining, self.stream.read_buf(&mut self.buf)).await {
                Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return None,
                Ok(Ok(_)) => {}
            }
        }
    }

    async fn hello(&mut self, version: &str) {
        let mut w = PacketWriter::new(id::HELLO);
        w.string(version);
        self.send(w.finish().unwrap()).await;
    }

    /// Packet 4, with just enough prefix for the server to reach the name.
    async fn sync_player(&mut self, slot: u8, name: &str) {
        let mut w = PacketWriter::new(id::SYNC_PLAYER);
        w.u8(slot) // slot
            .u8(0) // skin variant
            .u8(1) // voice variant
            .f32(0.0) // voice pitch offset
            .u8(0) // hair
            .string(name)
            .u8(0) // hair dye
            .bytes(&[0; 16]); // remaining appearance, unread by the server
        self.send(w.finish().unwrap()).await;
    }

    async fn controls(&mut self, slot: u8, x: f32, y: f32) {
        let mut w = PacketWriter::new(id::PLAYER_CONTROLS);
        w.u8(slot)
            .u8(0x40) // facing right
            .u8(0) // no velocity block
            .u8(0)
            .u8(0)
            .u8(0) // selected item
            .vec2(x, y);
        self.send(w.finish().unwrap()).await;
    }

    async fn say(&mut self, text: &str) {
        let mut w = PacketWriter::new(id::NET_MODULES);
        w.u16(MODULE_TEXT).string("Say").string(text);
        self.send(w.finish().unwrap()).await;
    }

    /// Find one tile in the sections streamed during the handshake.
    fn tile_from_handshake(&self, x: i32, y: i32) -> Option<terrustia_proto::Tile> {
        for frame in self
            .handshake_frames
            .iter()
            .filter(|f| f.id == id::TILE_SECTION)
        {
            let stream = terrustia_proto::section::inflate_section_payload(&frame.payload).unwrap();
            let (bounds, tiles, _) =
                terrustia_proto::section::decode_section_stream(&stream).unwrap();
            if bounds.x <= x
                && x < bounds.x + i32::from(bounds.width)
                && bounds.y <= y
                && y < bounds.y + i32::from(bounds.height)
            {
                let ix = (x - bounds.x) as usize;
                let iy = (y - bounds.y) as usize;
                return Some(tiles[iy * bounds.width as usize + ix]);
            }
        }
        None
    }

    /// Drive the full handshake and return the assigned slot.
    async fn join(&mut self, addr_name: &str) -> u8 {
        self.hello(id::VERSION_STRING).await;

        let frame = self.recv().await;
        assert_eq!(frame.id, id::PLAYER_INFO, "expected a slot assignment");
        let mut r = PacketReader::new(&frame.payload);
        let slot = r.u8().unwrap();
        r.bool().unwrap(); // the 1.4.5 trailing flag

        self.sync_player(slot, addr_name).await;
        self.send(packets::empty(id::REQUEST_WORLD_DATA).unwrap())
            .await;

        let frame = self.recv().await;
        assert_eq!(frame.id, id::WORLD_DATA);

        // Ask for tiles at spawn.
        let mut w = PacketWriter::new(id::SPAWN_TILE_DATA);
        w.i32(-1).i32(-1).u8(0);
        self.send(w.finish().unwrap()).await;

        let frames = self.recv_until(id::INITIAL_SPAWN).await;
        let ids: Vec<u8> = frames.iter().map(|f| f.id).collect();
        self.handshake_frames = frames.clone();
        assert_eq!(ids[0], id::WORLD_DATA, "world data is re-sent before tiles");
        assert_eq!(ids[1], id::STATUS_TEXT_SIZE);
        assert!(
            ids.iter().filter(|i| **i == id::TILE_SECTION).count() > 0,
            "expected tile sections, got {ids:?}"
        );

        // Spawn into the world.
        let spawn = PlayerSpawn {
            player: slot,
            spawn_x: -1,
            spawn_y: -1,
            respawn_timer: 0,
            deaths_pve: 0,
            deaths_pvp: 0,
            team: 0,
            context: PlayerSpawn::CONTEXT_SPAWNING_INTO_WORLD,
        };
        self.send(spawn.encode().unwrap()).await;
        slot
    }
}

#[tokio::test]
async fn a_client_completes_the_handshake_and_reaches_the_world() {
    let addr = start_server().await;
    let mut client = FakeClient::connect(addr).await;

    let slot = client.join("brooklyn").await;
    assert_eq!(slot, 0, "first client takes the first slot");

    let frames = client.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;
    assert!(
        frames
            .iter()
            .any(|f| f.id == id::FINISHED_CONNECTING_TO_SERVER),
        "the client never got the finish signal"
    );
}

#[tokio::test]
async fn the_status_packet_counts_the_sections_that_follow() {
    let addr = start_server().await;
    let mut client = FakeClient::connect(addr).await;

    client.hello(id::VERSION_STRING).await;
    let frame = client.recv().await;
    let slot = PacketReader::new(&frame.payload).u8().unwrap();

    client.sync_player(slot, "counter").await;
    client
        .send(packets::empty(id::REQUEST_WORLD_DATA).unwrap())
        .await;
    assert_eq!(client.recv().await.id, id::WORLD_DATA);

    let mut w = PacketWriter::new(id::SPAWN_TILE_DATA);
    w.i32(-1).i32(-1).u8(0);
    client.send(w.finish().unwrap()).await;

    let frames = client.recv_until(id::INITIAL_SPAWN).await;
    let status = frames
        .iter()
        .find(|f| f.id == id::STATUS_TEXT_SIZE)
        .expect("no status packet");
    let announced = PacketReader::new(&status.payload).i32().unwrap();
    let sent = frames.iter().filter(|f| f.id == id::TILE_SECTION).count();

    assert_eq!(
        announced as usize, sent,
        "the loading bar would never fill: announced {announced}, sent {sent}"
    );
}

#[tokio::test]
async fn an_outdated_client_is_kicked_with_a_reason() {
    let addr = start_server().await;
    let mut client = FakeClient::connect(addr).await;

    client.hello("Terraria279").await; // 1.4.4.9
    let frame = client.recv().await;
    assert_eq!(frame.id, id::KICK);

    let mut r = PacketReader::new(&frame.payload);
    let text = terrustia_proto::NetworkText::read(&mut r).unwrap();
    assert!(
        text.text.contains("1.4.5.7"),
        "kick should name the expected version, got {:?}",
        text.text
    );
}

#[tokio::test]
async fn packets_sent_before_the_handshake_are_ignored() {
    let addr = start_server().await;
    let mut client = FakeClient::connect(addr).await;

    // Chat and movement before a hello must not crash the server or be relayed.
    client.say("hello?").await;
    client.controls(0, 1.0, 2.0).await;

    // The server should still complete a normal handshake afterwards.
    let slot = client.join("late").await;
    let frames = client.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;
    assert!(!frames.is_empty());
    assert_eq!(slot, 0);
}

#[tokio::test]
async fn two_players_see_each_other_join_move_and_chat() {
    let addr = start_server().await;

    let mut alice = FakeClient::connect(addr).await;
    let alice_slot = alice.join("alice").await;
    alice.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;

    let mut bob = FakeClient::connect(addr).await;
    let bob_slot = bob.join("bob").await;
    assert_ne!(alice_slot, bob_slot, "players must get distinct slots");

    // Bob is told about Alice before he finishes connecting.
    let bob_frames = bob.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;
    let saw_alice_active = bob_frames.iter().any(|f| {
        f.id == id::PLAYER_ACTIVE && f.payload.first() == Some(&alice_slot) && f.payload[1] == 1
    });
    assert!(saw_alice_active, "bob was not told alice exists");
    assert!(
        bob_frames
            .iter()
            .any(|f| f.id == id::SYNC_PLAYER && f.payload.first() == Some(&alice_slot)),
        "bob did not receive alice's appearance"
    );

    // Alice learns about Bob.
    let mut alice_saw_bob = false;
    for _ in 0..40 {
        let Some(frame) = alice.try_recv(Duration::from_secs(2)).await else {
            break;
        };
        if frame.id == id::PLAYER_ACTIVE && frame.payload.first() == Some(&bob_slot) {
            alice_saw_bob = true;
            break;
        }
    }
    assert!(alice_saw_bob, "alice was never told bob joined");

    // Movement is relayed with the sender's real slot.
    bob.controls(bob_slot, 1234.0, 567.0).await;
    let mut relayed = None;
    for _ in 0..40 {
        let Some(frame) = alice.try_recv(Duration::from_secs(2)).await else {
            break;
        };
        if frame.id == id::PLAYER_CONTROLS && frame.payload.first() == Some(&bob_slot) {
            relayed = Some(frame);
            break;
        }
    }
    let relayed = relayed.expect("bob's movement never reached alice");
    let controls = packets::PlayerControls::decode(&relayed.payload).unwrap();
    assert_eq!(controls.position, (1234.0, 567.0));

    // Chat reaches the other player.
    bob.say("hi alice").await;
    let mut chat = None;
    for _ in 0..40 {
        let Some(frame) = alice.try_recv(Duration::from_secs(2)).await else {
            break;
        };
        if frame.id == id::NET_MODULES {
            let mut r = PacketReader::new(&frame.payload);
            if r.u16().unwrap() == MODULE_TEXT {
                let author = r.u8().unwrap();
                let text = terrustia_proto::NetworkText::read(&mut r).unwrap();
                if text.text.contains("hi alice") {
                    chat = Some((author, text.text));
                    break;
                }
            }
        }
    }
    let (author, text) = chat.expect("chat never reached alice");
    assert_eq!(author, bob_slot);
    assert!(
        text.contains("bob"),
        "chat should carry the sender's name: {text}"
    );
}

#[tokio::test]
async fn a_client_cannot_impersonate_another_slot() {
    let addr = start_server().await;

    let mut alice = FakeClient::connect(addr).await;
    let alice_slot = alice.join("alice").await;
    alice.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;

    let mut bob = FakeClient::connect(addr).await;
    let bob_slot = bob.join("bob").await;
    bob.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;

    // Bob claims to be Alice; the relay must re-stamp it with Bob's real slot.
    bob.controls(alice_slot, -99.0, -99.0).await;

    for _ in 0..40 {
        let Some(frame) = alice.try_recv(Duration::from_secs(2)).await else {
            break;
        };
        if frame.id == id::PLAYER_CONTROLS {
            assert_eq!(
                frame.payload.first(),
                Some(&bob_slot),
                "a spoofed slot was relayed unchanged"
            );
            return;
        }
    }
    panic!("no relayed movement arrived at all");
}

#[tokio::test]
async fn the_server_refuses_more_players_than_configured() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = Config {
        world_width: 800,
        world_height: 600,
        max_players: 1,
        motd: String::new(),
        ..Config::default()
    };
    let world = worldgen::generate(800, 600, "t", 1);
    let (tx, rx) = mpsc::channel::<ServerEvent>(64);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    tokio::spawn(listener::run(listener, config, tx));

    let mut first = FakeClient::connect(addr).await;
    first.join("first").await;
    first.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;

    // The second client should be told why rather than dropped silently.
    let mut second = FakeClient::connect(addr).await;
    let frame = second.recv().await;
    assert_eq!(frame.id, id::KICK);
    let mut r = PacketReader::new(&frame.payload);
    assert!(
        terrustia_proto::NetworkText::read(&mut r)
            .unwrap()
            .text
            .contains("full")
    );
}

#[tokio::test]
async fn a_slot_is_reused_after_a_player_leaves() {
    let addr = start_server().await;

    let mut first = FakeClient::connect(addr).await;
    let first_slot = first.join("first").await;
    first.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;
    drop(first);

    // Give the server a moment to notice the closed socket.
    tokio::time::sleep(Duration::from_millis(250)).await;

    let mut second = FakeClient::connect(addr).await;
    let second_slot = second.join("second").await;
    assert_eq!(second_slot, first_slot, "the freed slot should be reused");
}

#[tokio::test]
async fn a_section_is_streamed_on_request_and_not_repeated() {
    let addr = start_server().await;
    let mut client = FakeClient::connect(addr).await;
    client.join("walker").await;
    client.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;

    // Section (3, 3) is outside the block streamed around spawn in an 800x600 world.
    let request = |sx: u16, sy: u16| {
        let mut w = PacketWriter::new(id::REQUEST_SECTION);
        w.u16(sx).u16(sy);
        w.finish().unwrap()
    };

    client.send(request(3, 3)).await;
    let mut got_section = false;
    for _ in 0..40 {
        let Some(frame) = client.try_recv(Duration::from_secs(2)).await else {
            break;
        };
        if frame.id == id::TILE_SECTION {
            got_section = true;
            break;
        }
    }
    assert!(got_section, "requested section was never sent");

    // Asking again must not resend it.
    client.send(request(3, 3)).await;
    for _ in 0..10 {
        let Some(frame) = client.try_recv(Duration::from_millis(400)).await else {
            break;
        };
        assert_ne!(
            frame.id,
            id::TILE_SECTION,
            "a section the client already has was sent again"
        );
    }
}

#[tokio::test]
async fn a_garbage_frame_length_closes_the_connection() {
    let addr = start_server().await;
    let mut client = FakeClient::connect(addr).await;

    // A length below the 3-byte minimum means the stream is desynchronised.
    client.send(vec![1, 0, 99]).await;

    let mut buf = [0u8; 8];
    let read = timeout(Duration::from_secs(5), client.stream.read(&mut buf)).await;
    match read {
        Ok(Ok(0)) | Ok(Err(_)) => {}
        Err(_) => panic!("server kept a desynchronised connection open"),
        Ok(Ok(n)) => panic!("server replied with {n} bytes instead of closing"),
    }
}

#[tokio::test]
async fn chat_is_dropped_before_a_player_is_in_the_world() {
    let addr = start_server().await;

    let mut alice = FakeClient::connect(addr).await;
    alice.join("alice").await;
    alice.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;

    // Bob connects but never spawns, then tries to chat.
    let mut bob = FakeClient::connect(addr).await;
    bob.hello(id::VERSION_STRING).await;
    let frame = bob.recv().await;
    let bob_slot = PacketReader::new(&frame.payload).u8().unwrap();
    bob.sync_player(bob_slot, "bob").await;
    bob.say("i should not be heard").await;

    for _ in 0..6 {
        let Some(frame) = alice.try_recv(Duration::from_millis(400)).await else {
            break;
        };
        if frame.id == id::NET_MODULES {
            let mut r = PacketReader::new(&frame.payload);
            if r.u16().unwrap() == MODULE_TEXT {
                r.u8().unwrap();
                let text = terrustia_proto::NetworkText::read(&mut r).unwrap();
                assert!(
                    !text.text.contains("i should not be heard"),
                    "chat from an unspawned client was relayed"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_tile_edit_reaches_other_players_and_sticks_in_the_world() {
    let addr = start_server().await;

    let mut alice = FakeClient::connect(addr).await;
    alice.join("alice").await;
    alice.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;

    let mut bob = FakeClient::connect(addr).await;
    let bob_slot = bob.join("bob").await;
    bob.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;

    // Break a block well inside the ground so there is certainly something there.
    let edit = packets::TileManipulation {
        action: 0,
        x: 400,
        y: 400,
        arg: 0,
        style: 0,
    };
    bob.send(edit.encode().unwrap()).await;

    let mut relayed = None;
    for _ in 0..60 {
        let Some(frame) = alice.try_recv(Duration::from_secs(2)).await else {
            break;
        };
        if frame.id == id::TILE_MANIPULATION {
            relayed = Some(packets::TileManipulation::decode(&frame.payload).unwrap());
            break;
        }
    }
    let relayed = relayed.expect("bob's tile edit never reached alice");
    assert_eq!(relayed, edit, "the edit was altered in transit");

    // A section streamed afterwards must reflect the change, or the edit would vanish on relog.
    let mut fresh = FakeClient::connect(addr).await;
    fresh.join("fresh").await;
    let tile = fresh
        .tile_from_handshake(400, 400)
        .expect("never received the section containing the edit");
    assert!(!tile.is_active(), "the broken block came back");

    // Bob's own edit must not be echoed back to him.
    let mut echoed = false;
    for _ in 0..6 {
        let Some(frame) = bob.try_recv(Duration::from_millis(300)).await else {
            break;
        };
        if frame.id == id::TILE_MANIPULATION {
            echoed = true;
        }
    }
    assert!(!echoed, "the sender was sent its own edit back");
    let _ = bob_slot;
}

#[tokio::test]
async fn a_partially_damaged_block_is_not_removed() {
    // The test needs something solid to damage, so it puts it there rather than hoping the
    // generator did.
    let addr = start_server_with(|world| {
        world.set_tile(401, 401, terrustia_proto::Tile::block(1));
    })
    .await;
    let mut client = FakeClient::connect(addr).await;
    client.join("miner").await;
    client.recv_until(id::FINISHED_CONNECTING_TO_SERVER).await;

    // arg == 1 means "damaged but survived"; treating it as a break would delete blocks on the
    // first pickaxe swing.
    let edit = packets::TileManipulation {
        action: 0,
        x: 401,
        y: 401,
        arg: 1,
        style: 0,
    };
    client.send(edit.encode().unwrap()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut fresh = FakeClient::connect(addr).await;
    fresh.join("fresh").await;
    let tile = fresh
        .tile_from_handshake(401, 401)
        .expect("never received the section containing the tile");
    assert!(tile.is_active(), "a merely damaged block was deleted");
}
