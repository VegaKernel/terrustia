//! The web admin panel, exercised over real TCP connections — not an in-process mock. Covers the
//! foundation (static asset serving, the unclaimed/claim flow, an ordinary login, that
//! `/api/status` requires a valid session, the console `panel` toggle) and the full admin feature
//! set (see `panel/mod.rs`'s module doc): the player list and kick, against a real connected
//! `terrustia-client` bot; whitelist add/list/remove; the read-only settings view and the one
//! live-editable field (MOTD); world listing and the switch endpoint's validation (the switch
//! itself — a real process restart — is `tests/world_switch.rs`, which needs a real subprocess and
//! does not fit this file's in-process shape); and the live status/console/chat feed and the world
//! view, both read over a hand-rolled minimal WebSocket client for the same reason
//! `reqwest_lite` below is a hand-rolled HTTP client — this workspace has no WebSocket client
//! dependency either.

use terrustia::{
    config::Config,
    game::{GameServer, ServerEvent},
    net::listener,
    panel,
    term::{Palette, TermLayer},
    world::worldgen,
};
use tokio::sync::{mpsc, oneshot};

/// The panel's live console/chat feed (`crate::term::console_feed`) is fed by `TermLayer::on_event`
/// — but `TermLayer` is only ever installed as part of `tracing_subscriber::registry()...init()`
/// in `main()`, which nothing in this integration-test binary ever calls. Without this, every
/// `info!`/`warn!` the game task emits goes to whatever the process's default (no-op) subscriber
/// is, `TermLayer::on_event` never runs, and the feed test below would time out — not because the
/// feature is broken, but because nothing in the test process was watching for it. This installs
/// the exact same layer `main` does, once per process, so the real code path actually runs.
fn install_term_layer_once() {
    use std::sync::Once;
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = tracing_subscriber::registry()
            .with(TermLayer::new(Palette::PLAIN))
            .try_init();
    });
}

#[tokio::test]
async fn static_assets_and_the_unclaimed_flow_work_over_a_real_socket() {
    // Bind the real ephemeral listener ourselves so the address is known before the panel task
    // exists, rather than needing `panel::run` to report back what it chose.
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe); // release the port; the panel binds it again immediately below

    let config = Config {
        world_width: 800,
        world_height: 600,
        motd: String::new(),
        panel_enabled: true,
        panel_listen: addr,
        ..Config::default()
    };

    let world = worldgen::generate(
        config.world_width,
        config.world_height,
        config.world_name.clone(),
        7,
    );
    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    let _panel = panel::run(config, tx.clone())
        .await
        .expect("panel should bind its configured loopback address");

    let base = format!("http://{addr}");
    let client = reqwest_lite::Client::new();

    // The app shell is served for the root path, over real HTTP.
    let index = client.get(&base).await;
    assert!(
        index.contains("<div id=\"app\">") || index.contains("id=\"app\""),
        "expected the built index.html to be served at /, got: {index}"
    );

    // Freshly generated world, no accounts registered yet.
    let unclaimed = client.get(&format!("{base}/api/unclaimed")).await;
    assert!(
        unclaimed.contains("\"unclaimed\":true"),
        "a fresh world has no accounts yet: {unclaimed}"
    );

    // The real claim token, the way an operator would actually get it: read off the game task's
    // own state, the same value `announce_claim_token` printed to the console.
    let (reply, rx) = oneshot::channel();
    tx.send(ServerEvent::PanelAuthLookup {
        name: String::new(),
        reply,
    })
    .await
    .unwrap();
    let lookup = rx.await.unwrap();
    let token = lookup
        .claim_token
        .expect("run() announces a token immediately on start");

    // Wrong token is refused.
    let (status, _body) = client
        .post_json(
            &format!("{base}/api/login"),
            r#"{"name":"admin","password":"correcthorsebatterystaple","claim_token":"wrong"}"#,
        )
        .await;
    assert_eq!(status, 401, "a wrong claim token must not claim the server");

    // The right token claims it and returns a real session.
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            &format!(
                r#"{{"name":"admin","password":"correcthorsebatterystaple","claim_token":"{token}"}}"#
            ),
        )
        .await;
    assert_eq!(status, 200, "the real claim token should succeed: {body}");
    let session = extract_session(&body);

    // /api/status refuses no session at all.
    let (status, _) = client.get_status(&format!("{base}/api/status"), None).await;
    assert_eq!(status, 401, "status must require a session");

    // ...and accepts the one just issued, with real, non-placeholder data.
    let (status, body) = client
        .get_status(&format!("{base}/api/status"), Some(&session))
        .await;
    assert_eq!(
        status, 200,
        "a freshly issued session should be accepted: {body}"
    );
    assert!(body.contains("\"world_name\""));
    assert!(
        body.contains("\"unclaimed\":false"),
        "claiming should have flipped this: {body}"
    );

    // The server is claimed now — signing in again with a claim_token field present is simply
    // ignored, and the real password is what's checked.
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            r#"{"name":"admin","password":"wrongpassword"}"#,
        )
        .await;
    assert_eq!(status, 401, "wrong password must be refused: {body}");

    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            r#"{"name":"admin","password":"correcthorsebatterystaple"}"#,
        )
        .await;
    assert_eq!(status, 200, "the real password should sign in: {body}");
}

/// Fail-then-pass for the panel's own `/api/login` throttle (Lane F). `admin::throttle`'s own
/// unit tests prove the schedule, the jitter and reset-on-success deterministically with an
/// injected clock; this only has to prove the real HTTP handler is actually wired to it: enough
/// wrong passwords against one account open a window, and the next attempt is refused with `429`
/// and the shared `admin::REFUSAL_MESSAGE` — not the ordinary `401` "wrong name or password" —
/// even when it offers the real one, proving the throttle is checked before the credential is.
#[tokio::test]
async fn repeated_wrong_passwords_back_off_api_login() {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    let config = Config {
        world_width: 800,
        world_height: 600,
        motd: String::new(),
        panel_enabled: true,
        panel_listen: addr,
        ..Config::default()
    };

    let world = worldgen::generate(
        config.world_width,
        config.world_height,
        config.world_name.clone(),
        7,
    );
    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    let _panel = panel::run(config, tx.clone())
        .await
        .expect("panel should bind its configured loopback address");

    let base = format!("http://{addr}");
    let client = reqwest_lite::Client::new();

    let (reply, token_rx) = oneshot::channel();
    tx.send(ServerEvent::PanelAuthLookup {
        name: String::new(),
        reply,
    })
    .await
    .unwrap();
    let token = token_rx.await.unwrap().claim_token.unwrap();

    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            &format!(
                r#"{{"name":"owner","password":"correcthorsebatterystaple","claim_token":"{token}"}}"#
            ),
        )
        .await;
    assert_eq!(status, 200, "claiming the server: {body}");
    let owner_session = extract_session(&body);

    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/accounts/create"),
            r#"{"name":"victim","password":"victims-real-password","group":"moderator"}"#,
            &owner_session,
        )
        .await;
    assert_eq!(status, 200, "{body}");

    for attempt in 0..=terrustia::admin::throttle::FREE_ATTEMPTS {
        let (status, body) = client
            .post_json(
                &format!("{base}/api/login"),
                r#"{"name":"victim","password":"the-wrong-password"}"#,
            )
            .await;
        assert_eq!(
            status, 401,
            "attempt {attempt}: an ordinary wrong password, not yet throttled: {body}"
        );
    }

    // Inside the window now: even the real password is refused, and refused differently — the
    // whole point being that this is decided before the password is ever compared at all.
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            r#"{"name":"victim","password":"victims-real-password"}"#,
        )
        .await;
    assert_eq!(
        status, 429,
        "the window should still be open, refusing even the real password: {body}"
    );
    assert!(
        body.contains(terrustia::admin::REFUSAL_MESSAGE),
        "a throttled refusal should carry the shared generic message, not a fresh one: {body}"
    );
}

/// The console's `panel` command, driven exactly as `main` wires it: `GameServer` holds one end
/// of an unbounded channel, `panel::supervise` owns the other and the actual bind/abort. Starts
/// with the panel off (`panel_enabled: false`, no `initial` handle) — the ordinary case, since the
/// panel is opt-in — and drives it on, then off, entirely through real `ServerEvent::Console`
/// lines over a real TCP port, the same way an operator typing at the sticky console would.
#[tokio::test]
async fn the_console_panel_command_starts_and_stops_a_real_listener() {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    let config = Config {
        world_width: 800,
        world_height: 600,
        motd: String::new(),
        panel_enabled: false,
        panel_listen: addr,
        ..Config::default()
    };
    let world = worldgen::generate(
        config.world_width,
        config.world_height,
        config.world_name.clone(),
        11,
    );
    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    let (toggle_tx, toggle_rx) = mpsc::unbounded_channel();
    tokio::spawn(
        GameServer::new(config.clone(), world)
            .with_panel_toggle(toggle_tx)
            .run(rx),
    );
    tokio::spawn(panel::supervise(
        config.clone(),
        tx.clone(),
        toggle_rx,
        None,
    ));

    assert!(
        !port_answers(addr).await,
        "the panel must not be listening before anyone asks for it"
    );

    tx.send(ServerEvent::Console {
        line: "panel".to_string(),
    })
    .await
    .unwrap();
    assert!(
        wait_until(|| port_answers(addr)).await,
        "the panel should be listening within the deadline after the first toggle"
    );

    // Actually reachable, not just holding the port — the same static-asset path the foundation
    // test exercises.
    let index = reqwest_lite::Client::new()
        .get(&format!("http://{addr}"))
        .await;
    assert!(
        index.contains("id=\"app\""),
        "expected the app shell once toggled on: {index}"
    );

    tx.send(ServerEvent::Console {
        line: "panel".to_string(),
    })
    .await
    .unwrap();
    assert!(
        wait_until(|| async { !port_answers(addr).await }).await,
        "the panel should stop listening within the deadline after the second toggle"
    );
}

/// The rest of the panel's admin feature set, in one flow against one running server — player
/// list and kick (against a real connected `terrustia-client` bot, not a synthetic slot), the
/// whitelist, settings, the worlds list and the switch endpoint's validation, and the live
/// status/console/chat feed and world view over real WebSocket connections.
#[tokio::test]
async fn the_admin_feature_set_works_over_real_sockets() {
    install_term_layer_once();

    let game_probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let game_addr = game_probe.local_addr().unwrap();
    drop(game_probe);
    let panel_probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let panel_addr = panel_probe.local_addr().unwrap();
    drop(panel_probe);

    let config = Config {
        listen: game_addr,
        world_width: 800,
        world_height: 600,
        motd: "hello panel".into(),
        panel_enabled: true,
        panel_listen: panel_addr,
        ..Config::default()
    };
    let world = worldgen::generate(
        config.world_width,
        config.world_height,
        config.world_name.clone(),
        21,
    );
    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    let _panel = panel::run(config.clone(), tx.clone()).await.unwrap();
    let game_listener = tokio::net::TcpListener::bind(game_addr).await.unwrap();
    tokio::spawn(listener::run(
        game_listener,
        config.clone(),
        tx.clone(),
        None,
    ));

    let base = format!("http://{panel_addr}");
    let client = reqwest_lite::Client::new();

    // Claim it, the way `static_assets_and_the_unclaimed_flow_work_over_a_real_socket` does: read
    // the real claim token off the game task rather than a test-only shortcut.
    let (reply, token_rx) = oneshot::channel();
    tx.send(ServerEvent::PanelAuthLookup {
        name: String::new(),
        reply,
    })
    .await
    .unwrap();
    let token = token_rx.await.unwrap().claim_token.unwrap();
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            &format!(
                r#"{{"name":"admin","password":"correcthorsebatterystaple","claim_token":"{token}"}}"#
            ),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let session = extract_session(&body);

    // ---- unauthenticated requests are refused, across a sample of the new endpoints ----
    for path in [
        "/api/players",
        "/api/whitelist",
        "/api/config",
        "/api/worlds",
    ] {
        let (status, _) = client.get_status(&format!("{base}{path}"), None).await;
        assert_eq!(status, 401, "{path} must require a session");
    }

    // ---- a real connected player shows up, with real appearance data ----
    let mut bot = terrustia_client::Client::join(game_addr, "Voyager")
        .await
        .expect("the bot should be able to join");

    let (status, body) = client
        .get_status(&format!("{base}/api/players"), Some(&session))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains("\"name\":\"Voyager\""),
        "the connected bot should be in the list: {body}"
    );
    // `terrustia_client::Client::appearance_packet` sends `hair_color: [215, 90, 55]` — this
    // checks the whole real pipeline end to end: the bot's own packet 4 bytes, stored raw on
    // `Player::appearance`, decoded by `PlayerAppearance::decode`, and serialized into this
    // response, all matching the exact bytes the bot actually sent.
    assert!(
        body.contains("\"hair_color\":[215,90,55]"),
        "expected the bot's real hair colour to survive the whole pipeline: {body}"
    );
    assert!(body.contains("\"life\":100"), "{body}");

    // ---- whitelist: add, list, remove ----
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/whitelist/add"),
            r#"{"name":"Friend"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = client
        .get_status(&format!("{base}/api/whitelist"), Some(&session))
        .await;
    assert_eq!(status, 200);
    assert!(
        body.contains("\"on\":true") && body.contains("Friend"),
        "{body}"
    );
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/whitelist/remove"),
            r#"{"name":"Friend"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, body) = client
        .get_status(&format!("{base}/api/whitelist"), Some(&session))
        .await;
    assert!(
        body.contains("\"on\":false"),
        "an empty list is off again: {body}"
    );

    // ---- settings: read-only fields, plus the one live-editable one ----
    let (status, body) = client
        .get_status(&format!("{base}/api/config"), Some(&session))
        .await;
    assert_eq!(status, 200);
    assert!(body.contains("\"motd\":\"hello panel\""), "{body}");
    assert!(body.contains("\"max_players\":8"), "{body}");
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/config/motd"),
            r#"{"motd":"updated from the panel"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 200);
    let (_, body) = client
        .get_status(&format!("{base}/api/config"), Some(&session))
        .await;
    assert!(
        body.contains("\"motd\":\"updated from the panel\""),
        "the motd should change live: {body}"
    );

    // ---- worlds: an unknown name is refused rather than blindly accepted ----
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/worlds/switch"),
            r#"{"name":"no such world anywhere"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 404);

    // ---- the live status/console/chat feed, over a real WebSocket ----
    let mut feed =
        ws_lite::WsClient::connect(panel_addr, &format!("/api/ws?session={session}")).await;
    // The very first frame this socket ever sends proves `stream_status` has already reached its
    // `console_feed().subscribe()` call — that happens before its first loop iteration, and
    // `tokio::time::interval`'s first tick fires immediately, so a status refresh is always the
    // first thing out. Without waiting for this, a chat line sent right after the WS handshake
    // completes could race the subscription itself and broadcast into a receiver that does not
    // exist yet — a real race, not a flaky-test excuse, and this is how it is actually closed.
    feed.recv_text(std::time::Duration::from_secs(5))
        .await
        .expect("the socket should send a first frame promptly");

    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/chat"),
            r#"{"text":"hello from the panel"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 200);
    let mut saw_chat = false;
    for _ in 0..30 {
        match feed.recv_text(std::time::Duration::from_secs(3)).await {
            Some(text) => {
                if text.contains(r#""type":"console""#)
                    && text.contains(r#""line_kind":"chat""#)
                    && text.contains("hello from the panel")
                {
                    saw_chat = true;
                    break;
                }
            }
            None => break,
        }
    }
    assert!(
        saw_chat,
        "expected the chat line sent over /api/chat to arrive on the live feed"
    );

    // A console command sent the same way, checked the same way.
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/console"),
            r#"{"line":"players"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 200);
    let mut saw_reply = false;
    for _ in 0..30 {
        match feed.recv_text(std::time::Duration::from_secs(3)).await {
            Some(text) => {
                if text.contains(r#""line_kind":"reply""#) {
                    saw_reply = true;
                    break;
                }
            }
            None => break,
        }
    }
    assert!(
        saw_reply,
        "expected the `players` console command's own reply to arrive on the live feed"
    );

    // ---- the live world view: real positions, real tiles ----
    let mut world_feed =
        ws_lite::WsClient::connect(panel_addr, &format!("/api/ws/world?session={session}")).await;
    let (mut saw_players, mut saw_tiles) = (false, false);
    for _ in 0..20 {
        if saw_players && saw_tiles {
            break;
        }
        match world_feed
            .recv_text(std::time::Duration::from_secs(3))
            .await
        {
            Some(text) if text.contains(r#""type":"players""#) => {
                saw_players = true;
                assert!(
                    text.contains("Voyager"),
                    "the connected bot should be in the world view too: {text}"
                );
            }
            Some(text) if text.contains(r#""type":"tiles""#) => {
                saw_tiles = true;
                assert!(
                    text.contains("\"world_width\":800") && text.contains("\"world_height\":600"),
                    "expected the real world's dimensions: {text}"
                );
            }
            Some(_) => {}
            None => break,
        }
    }
    assert!(saw_players, "expected at least one players frame");
    assert!(saw_tiles, "expected at least one tiles frame");

    // ---- kick, last, since it removes the bot the rest of this test still needed connected ----
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/players/kick"),
            r#"{"name":"Voyager","reason":"end of test"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    // The bot has been sitting connected, un-drained, through every assertion above — ordinary
    // per-tick traffic (NPC/projectile syncs and the like) queued up ahead of the kick packet in
    // its inbound stream, so this scans past that backlog rather than assuming the very next frame
    // is the kick itself.
    let mut kicked = false;
    for _ in 0..64 {
        match tokio::time::timeout(std::time::Duration::from_secs(5), bot.next_event()).await {
            Ok(Err(terrustia_client::ClientError::Kicked { .. })) => {
                kicked = true;
                break;
            }
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => panic!("unexpected error waiting for the kick: {e}"),
            Err(_) => break,
        }
    }
    assert!(
        kicked,
        "the bot should see a real kick packet, not just vanish"
    );

    // Kicking somebody who is not connected is reported, not silently accepted.
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/players/kick"),
            r#"{"name":"Voyager","reason":"already gone"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 404);
}

/// The four features added while finishing the panel — a live metrics snapshot, the backup/rollback
/// view and its guards, the groups/accounts admin (including the lock-out guard that stops the last
/// admin account being stripped), and world-creation validation — over real sockets, in one flow.
///
/// `save_file` is set to a unique temp path so `save_path` is real: the backups view reports it is
/// saving, `/api/save` is accepted, and the admin store the accounts endpoints mutate is written
/// beside it rather than anywhere permanent. World *generation* itself is not exercised here — it
/// would write into the real platform world directory and take real time — only its validation and
/// status endpoints, which is what this in-process shape can cover honestly.
#[tokio::test]
async fn the_finishing_features_work_over_real_sockets() {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    // A unique save target so no stale admin file from a previous run starts this server claimed,
    // and so the account mutations below write a throwaway admin file rather than a permanent one.
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let save_file = std::env::temp_dir().join(format!(
        "terrustia-panel-test-{}-{unique}.wld",
        std::process::id()
    ));

    let config = Config {
        world_width: 800,
        world_height: 600,
        motd: String::new(),
        panel_enabled: true,
        panel_listen: addr,
        save_file: Some(save_file.clone()),
        autosave_secs: 0,
        ..Config::default()
    };
    let world = worldgen::generate(config.world_width, config.world_height, "features", 5);
    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    let _panel = panel::run(config, tx.clone()).await.unwrap();

    let base = format!("http://{addr}");
    let client = reqwest_lite::Client::new();

    // Claim as the first account (the `owner` group, which grants Admin).
    let (reply, token_rx) = oneshot::channel();
    tx.send(ServerEvent::PanelAuthLookup {
        name: String::new(),
        reply,
    })
    .await
    .unwrap();
    let token = token_rx.await.unwrap().claim_token.unwrap();
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            &format!(
                r#"{{"name":"owner","password":"correcthorsebatterystaple","claim_token":"{token}"}}"#
            ),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let session = extract_session(&body);

    // ---- every new endpoint refuses a request with no session ----
    for path in [
        "/api/metrics",
        "/api/backups",
        "/api/accounts",
        "/api/worlds/new/status",
    ] {
        let (status, _) = client.get_status(&format!("{base}{path}"), None).await;
        assert_eq!(status, 401, "{path} must require a session");
    }

    // ---- metrics: a real, live snapshot ----
    let (status, body) = client
        .get_status(&format!("{base}/api/metrics"), Some(&session))
        .await;
    assert_eq!(status, 200, "{body}");
    // 16,666,667 ns truncates to 16,666 µs — sixty ticks a second.
    assert!(
        body.contains("\"budget_us\":16666"),
        "the tick budget: {body}"
    );
    assert!(body.contains("\"phases\""), "a per-phase breakdown: {body}");
    assert!(body.contains("\"player_count\":0"), "{body}");

    // ---- backups: this world is being saved, so the view reports it and `save` is accepted ----
    let (status, body) = client
        .get_status(&format!("{base}/api/backups"), Some(&session))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"saving\":true"), "a saved world: {body}");
    assert!(body.contains("\"kept\":3"), "the rotation count: {body}");
    let (status, _) = client
        .post_json_auth(&format!("{base}/api/save"), "{}", &session)
        .await;
    assert_eq!(
        status, 200,
        "a save should be accepted when there is a save target"
    );
    // Rolling back a backup that does not exist is refused rather than doing something destructive.
    let (status, body) = client
        .post_json_auth(&format!("{base}/api/rollback"), r#"{"which":5}"#, &session)
        .await;
    assert_eq!(status, 400, "out-of-range rollback must be refused: {body}");

    // ---- accounts: the claim put `owner` in the admin-capable `owner` group ----
    let (status, body) = client
        .get_status(&format!("{base}/api/accounts"), Some(&session))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains("\"name\":\"owner\"") && body.contains("\"can_admin\":true"),
        "the owner account can administer the server: {body}"
    );
    assert!(
        body.contains("\"name\":\"moderator\"") && body.contains("\"name\":\"default\""),
        "the default groups are listed: {body}"
    );

    // The lock-out guard: the only admin account cannot be demoted or deleted.
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/accounts/group"),
            r#"{"name":"owner","group":"default"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 400, "demoting the only admin must be refused");
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/accounts/delete"),
            r#"{"name":"owner"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 400, "deleting the only admin must be refused");

    // Create a moderator, and prove the guards around it.
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/accounts/create"),
            r#"{"name":"short","password":"abc","group":"moderator"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 400, "a too-short password must be refused");
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/accounts/create"),
            r#"{"name":"mod1","password":"moderator-pass","group":"nope"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 400, "a nonexistent group must be refused");
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/accounts/create"),
            r#"{"name":"mod1","password":"moderator-pass","group":"moderator"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 200, "a valid account should be created: {body}");
    let (_, body) = client
        .get_status(&format!("{base}/api/accounts"), Some(&session))
        .await;
    assert!(
        body.contains("\"name\":\"mod1\""),
        "the new account: {body}"
    );
    // A duplicate is refused.
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/accounts/create"),
            r#"{"name":"mod1","password":"moderator-pass","group":"moderator"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 400, "a duplicate name must be refused");
    // Promote mod1 to owner — now there are two admins, so the owner may be demoted, and mod1 may be
    // deleted (the other admin still remains).
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/accounts/group"),
            r#"{"name":"mod1","group":"owner"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 200, "promoting to a second admin is allowed");
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/accounts/delete"),
            r#"{"name":"mod1"}"#,
            &session,
        )
        .await;
    assert_eq!(status, 200, "deleting one of two admins is allowed");

    // ---- world creation: validation and status, without generating into a real directory ----
    let (status, _) = client
        .get_status(&format!("{base}/api/worlds/new/status"), Some(&session))
        .await;
    assert_eq!(status, 200);
    // An ill-sized world (not a whole number of sections) is refused before any slow work starts.
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/worlds/new"),
            r#"{"name":"bad","width":123,"height":45}"#,
            &session,
        )
        .await;
    assert_eq!(status, 400, "a bad world size must be refused: {body}");

    // Tidy up the throwaway save file and its admin sidecar, best-effort.
    let _ = std::fs::remove_file(&save_file);
    let _ = std::fs::remove_file(save_file.with_extension("admin.toml"));
    let _ = std::fs::remove_file(save_file.with_extension("wld.bak1"));
}

/// Sessions are scoped by permission, not all-or-nothing: a `default` account cannot even sign in
/// (it holds no `panel.view`), and a `moderator` session can reach a route it has the permission
/// for (`server.kick`) while getting a real `403` from one it does not (`panel.console`) — proving
/// the check is per-route, not "any session is fully privileged" the way it used to be before this
/// system existed (see `panel/mod.rs`'s module doc for the full route-to-permission map).
///
/// Fail-then-pass: before `authorized()` took a `Permission` and checked it fresh per route, a
/// `moderator` session hitting `/api/console` here got `200`, not `403` — any valid session could
/// run unrestricted console commands, which is exactly the all-or-nothing hole this closes.
#[tokio::test]
async fn moderator_and_default_sessions_are_scoped_by_permission() {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    let config = Config {
        world_width: 800,
        world_height: 600,
        motd: String::new(),
        panel_enabled: true,
        panel_listen: addr,
        ..Config::default()
    };
    let world = worldgen::generate(config.world_width, config.world_height, "scoped", 9);
    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    let _panel = panel::run(config, tx.clone()).await.unwrap();

    let base = format!("http://{addr}");
    let client = reqwest_lite::Client::new();

    // Claim as owner.
    let (reply, token_rx) = oneshot::channel();
    tx.send(ServerEvent::PanelAuthLookup {
        name: String::new(),
        reply,
    })
    .await
    .unwrap();
    let token = token_rx.await.unwrap().claim_token.unwrap();
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            &format!(
                r#"{{"name":"owner","password":"correcthorsebatterystaple","claim_token":"{token}"}}"#
            ),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let owner_session = extract_session(&body);

    // Create one account in each of the two non-owner default groups.
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/accounts/create"),
            r#"{"name":"modacct","password":"moderator-pass","group":"moderator"}"#,
            &owner_session,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/accounts/create"),
            r#"{"name":"defacct","password":"default-pass","group":"default"}"#,
            &owner_session,
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // A `default` account cannot even sign in to the panel: it does not hold `panel.view`.
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            r#"{"name":"defacct","password":"default-pass"}"#,
        )
        .await;
    assert_eq!(
        status, 403,
        "a default-group account must not be able to open a panel session: {body}"
    );

    // A `moderator` account can sign in — it holds `panel.view`.
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            r#"{"name":"modacct","password":"moderator-pass"}"#,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mod_session = extract_session(&body);

    // `/api/status` carries the session's own permissions, for the frontend's tab filtering.
    let (status, body) = client
        .get_status(&format!("{base}/api/status"), Some(&mod_session))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"server.kick\""), "{body}");
    assert!(
        !body.contains("\"panel.console\""),
        "a moderator must not carry panel.console: {body}"
    );

    // A route it *does* have the permission for: not found (proving the permission check passed
    // and the request reached the handler), never forbidden.
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/players/kick"),
            r#"{"name":"nobody-connected","reason":"test"}"#,
            &mod_session,
        )
        .await;
    assert_eq!(
        status, 404,
        "a moderator has server.kick, so this must reach the handler: {body}"
    );

    // A route it does *not* have the permission for: forbidden, not merely "not found" or "ok".
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/console"),
            r#"{"line":"players"}"#,
            &mod_session,
        )
        .await;
    assert_eq!(
        status, 403,
        "a moderator must not reach the raw console channel: {body}"
    );

    // Same story for accounts administration — a moderator does not hold `admin.accounts`.
    let (status, body) = client
        .get_status(&format!("{base}/api/accounts"), Some(&mod_session))
        .await;
    assert_eq!(status, 403, "{body}");

    // But an ordinary `panel.view` route still works for it.
    let (status, body) = client
        .get_status(&format!("{base}/api/players"), Some(&mod_session))
        .await;
    assert_eq!(status, 200, "{body}");
}

/// The group-permission editor: an `admin.groups` holder (`owner`) may grant/revoke a permission on
/// a group, but only within its own reach — and the reach guard applies to *itself* too, not just
/// to account/group reassignment. Also proves an `admin.accounts`-only account (no `admin.groups`)
/// gets `403` from the editor routes even though it can see the plain accounts list.
#[tokio::test]
async fn the_group_permission_editor_is_reach_limited() {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    let config = Config {
        world_width: 800,
        world_height: 600,
        motd: String::new(),
        panel_enabled: true,
        panel_listen: addr,
        ..Config::default()
    };
    let world = worldgen::generate(config.world_width, config.world_height, "groupedit", 13);
    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    let _panel = panel::run(config, tx.clone()).await.unwrap();

    let base = format!("http://{addr}");
    let client = reqwest_lite::Client::new();

    let (reply, token_rx) = oneshot::channel();
    tx.send(ServerEvent::PanelAuthLookup {
        name: String::new(),
        reply,
    })
    .await
    .unwrap();
    let token = token_rx.await.unwrap().claim_token.unwrap();
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            &format!(
                r#"{{"name":"owner","password":"correcthorsebatterystaple","claim_token":"{token}"}}"#
            ),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let owner_session = extract_session(&body);

    // The known vocabulary includes both a leaf and a family wildcard.
    let (status, body) = client
        .get_status(&format!("{base}/api/permissions"), Some(&owner_session))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"server.kick\""), "{body}");
    assert!(body.contains("\"server.*\""), "{body}");

    // Owner (holding `*`) may grant `moderator` a new permission it did not have before.
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/groups/permissions"),
            r#"{"group":"moderator","permission":"server.ban","grant":true}"#,
            &owner_session,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, body) = client
        .get_status(&format!("{base}/api/accounts"), Some(&owner_session))
        .await;
    assert!(
        body.contains("\"name\":\"moderator\"") && body.contains("server.ban"),
        "the grant should be visible in the groups list: {body}"
    );

    // And revoke it again.
    let (status, _) = client
        .post_json_auth(
            &format!("{base}/api/groups/permissions"),
            r#"{"group":"moderator","permission":"server.ban","grant":false}"#,
            &owner_session,
        )
        .await;
    assert_eq!(status, 200);

    // An unrecognised permission name is refused rather than silently doing nothing.
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/groups/permissions"),
            r#"{"group":"moderator","permission":"srever.kcik","grant":true}"#,
            &owner_session,
        )
        .await;
    assert_eq!(status, 400, "a typo must be refused: {body}");

    // Now the escalation guard: promote a fresh account to `admin` (which holds `admin.accounts`
    // but deliberately not `admin.groups`), and prove it can see the accounts list but gets `403`
    // from the group-permission editor — the whole point of the ladder in `group::defaults`.
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/accounts/create"),
            r#"{"name":"adminacct","password":"admin-password","group":"admin"}"#,
            &owner_session,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            r#"{"name":"adminacct","password":"admin-password"}"#,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let admin_session = extract_session(&body);

    let (status, body) = client
        .get_status(&format!("{base}/api/accounts"), Some(&admin_session))
        .await;
    assert_eq!(status, 200, "admin.accounts should see the list: {body}");

    let (status, body) = client
        .get_status(&format!("{base}/api/permissions"), Some(&admin_session))
        .await;
    assert_eq!(
        status, 403,
        "admin lacks admin.groups, so the editor's vocabulary route must refuse it: {body}"
    );
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/groups/permissions"),
            r#"{"group":"moderator","permission":"server.ban","grant":true}"#,
            &admin_session,
        )
        .await;
    assert_eq!(
        status, 403,
        "admin must not be able to edit any group's permissions: {body}"
    );

    // And the reach guard bites even for owner: it cannot demote itself into a group that then
    // could not edit permissions at all, because `panel_set_account_group`'s lock-out guard (not
    // the reach guard) refuses stripping the last `admin.groups`-capable account.
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/accounts/group"),
            r#"{"name":"owner","group":"default"}"#,
            &owner_session,
        )
        .await;
    assert_eq!(status, 400, "the last admin.groups-capable account: {body}");
}

/// A ban placed through the panel shows up in the audit log, attributed to the real signed-in
/// account, and `/api/audit` is itself gated on `admin.audit` — a `moderator` session (no
/// `admin.audit`) is refused even though it can place the ban it is trying to look up.
#[tokio::test]
async fn banning_through_the_panel_appears_in_the_audit_log() {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    // A unique save target: the audit log lives beside the world file, and a fresh path means a
    // fresh (empty) log to read back rather than one left over from an earlier run.
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let save_file = std::env::temp_dir().join(format!(
        "terrustia-audit-test-{}-{unique}.wld",
        std::process::id()
    ));

    let config = Config {
        world_width: 800,
        world_height: 600,
        motd: String::new(),
        panel_enabled: true,
        panel_listen: addr,
        save_file: Some(save_file.clone()),
        autosave_secs: 0,
        ..Config::default()
    };
    let world = worldgen::generate(config.world_width, config.world_height, "audit", 17);
    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    let _panel = panel::run(config, tx.clone()).await.unwrap();

    let base = format!("http://{addr}");
    let client = reqwest_lite::Client::new();

    let (reply, token_rx) = oneshot::channel();
    tx.send(ServerEvent::PanelAuthLookup {
        name: String::new(),
        reply,
    })
    .await
    .unwrap();
    let token = token_rx.await.unwrap().claim_token.unwrap();
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            &format!(
                r#"{{"name":"owner","password":"correcthorsebatterystaple","claim_token":"{token}"}}"#
            ),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let owner_session = extract_session(&body);

    // Before a ban happens, the log holds only the claim itself (claiming is audited too).
    let (status, body) = client
        .get_status(&format!("{base}/api/audit"), Some(&owner_session))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains(r#""action":"claim""#), "{body}");
    assert!(
        !body.contains(r#""action":"ban""#),
        "no ban has happened yet: {body}"
    );

    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/players/ban"),
            r#"{"kind":"name","value":"griefer","reason":"wrecked spawn"}"#,
            &owner_session,
        )
        .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = client
        .get_status(&format!("{base}/api/audit"), Some(&owner_session))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains(r#""action":"ban""#), "{body}");
    assert!(body.contains(r#""issuer":"owner""#), "{body}");
    assert!(body.contains(r#""target":"griefer""#), "{body}");

    // A moderator (no admin.audit) can place a ban — it holds server.ban — but cannot read the log
    // it just wrote a line into.
    let (status, body) = client
        .post_json_auth(
            &format!("{base}/api/accounts/create"),
            r#"{"name":"modacct","password":"moderator-pass","group":"moderator"}"#,
            &owner_session,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            r#"{"name":"modacct","password":"moderator-pass"}"#,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mod_session = extract_session(&body);

    let (status, body) = client
        .get_status(&format!("{base}/api/audit"), Some(&mod_session))
        .await;
    assert_eq!(
        status, 403,
        "a moderator lacks admin.audit, so the log route must refuse it: {body}"
    );

    // Tidy up the throwaway save file and its admin/audit sidecars, best-effort.
    let _ = std::fs::remove_file(&save_file);
    let _ = std::fs::remove_file(save_file.with_extension("admin.toml"));
    let _ = std::fs::remove_file(save_file.with_extension("audit.jsonl"));
    let _ = std::fs::remove_file(save_file.with_extension("wld.bak1"));
}

/// Whether *something* is accepting connections on `addr` right now — a closed port refuses
/// immediately rather than hanging, so no timeout is needed here.
async fn port_answers(addr: std::net::SocketAddr) -> bool {
    tokio::net::TcpStream::connect(addr).await.is_ok()
}

/// Poll `check` until it is true or five seconds pass — the toggle is asynchronous (a channel
/// send, then a supervisor task waking up and, on the way on, a real `bind`), so the effect is
/// never visible at the instant the console line is sent.
async fn wait_until<F, Fut>(mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if check().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    false
}

fn extract_session(body: &str) -> String {
    let key = "\"session\":\"";
    let start = body
        .find(key)
        .expect("login response should carry a session")
        + key.len();
    let end = body[start..].find('"').unwrap() + start;
    body[start..end].to_string()
}

/// A minimal HTTP/1.1 client over a raw TCP socket — this workspace has no HTTP client dependency
/// (the game protocol has nothing to do with HTTP), and pulling one in just for three test
/// requests would be a heavier dependency than the thing it is testing.
mod reqwest_lite {
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    pub struct Client;

    impl Client {
        pub fn new() -> Self {
            Client
        }

        pub async fn get(&self, url: &str) -> String {
            let (_status, body) = self.request("GET", url, None, None).await;
            body
        }

        pub async fn get_status(&self, url: &str, session: Option<&str>) -> (u16, String) {
            self.request("GET", url, None, session).await
        }

        pub async fn post_json(&self, url: &str, json: &str) -> (u16, String) {
            self.request("POST", url, Some(json), None).await
        }

        pub async fn post_json_auth(&self, url: &str, json: &str, session: &str) -> (u16, String) {
            self.request("POST", url, Some(json), Some(session)).await
        }

        async fn request(
            &self,
            method: &str,
            url: &str,
            json_body: Option<&str>,
            session: Option<&str>,
        ) -> (u16, String) {
            let (host_port, path) = split_url(url);
            let mut stream =
                tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&host_port))
                    .await
                    .expect("connect timed out")
                    .expect("connect failed");

            let mut request =
                format!("{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n");
            if let Some(session) = session {
                request.push_str(&format!("Authorization: Bearer {session}\r\n"));
            }
            match json_body {
                Some(json) => {
                    request.push_str("Content-Type: application/json\r\n");
                    request.push_str(&format!("Content-Length: {}\r\n\r\n{json}", json.len()));
                }
                None => request.push_str("\r\n"),
            }

            stream
                .write_all(request.as_bytes())
                .await
                .expect("write failed");
            let mut raw = Vec::new();
            tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut raw))
                .await
                .expect("read timed out")
                .expect("read failed");
            let raw = String::from_utf8_lossy(&raw);

            let mut parts = raw.splitn(2, "\r\n\r\n");
            let head = parts.next().unwrap_or_default();
            let body = parts.next().unwrap_or_default().to_string();
            let status = head
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|code| code.parse().ok())
                .unwrap_or(0);
            (status, body)
        }
    }

    fn split_url(url: &str) -> (String, String) {
        let rest = url.strip_prefix("http://").unwrap_or(url);
        match rest.find('/') {
            Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
            None => (rest.to_string(), "/".to_string()),
        }
    }
}

/// A minimal WebSocket client over a raw TCP socket — the same reasoning as `reqwest_lite` above:
/// this workspace has no WebSocket client dependency (axum pulls `tokio-tungstenite` in
/// transitively for the *server* side, but a transitive dependency cannot be named from test
/// code), and this only ever needs to perform the client handshake and read text frames the
/// server sends, never write one back.
mod ws_lite {
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    pub struct WsClient {
        stream: TcpStream,
    }

    impl WsClient {
        /// `path_and_query` starts with `/`, e.g. `/api/ws?session=...`. The handshake's own
        /// `Sec-WebSocket-Key` does not need to be random for this: the server only has to see a
        /// well-formed value to compute an accept hash this client never bothers checking — a 101
        /// response is proof enough that the upgrade actually happened.
        pub async fn connect(addr: SocketAddr, path_and_query: &str) -> Self {
            let mut stream = TcpStream::connect(addr)
                .await
                .expect("connect for the websocket handshake");
            let request = format!(
                "GET {path_and_query} HTTP/1.1\r\n\
                 Host: {addr}\r\n\
                 Upgrade: websocket\r\n\
                 Connection: Upgrade\r\n\
                 Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                 Sec-WebSocket-Version: 13\r\n\r\n"
            );
            stream
                .write_all(request.as_bytes())
                .await
                .expect("write the upgrade request");

            let mut head = Vec::new();
            loop {
                let mut byte = [0u8; 1];
                stream
                    .read_exact(&mut byte)
                    .await
                    .expect("read the upgrade response");
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&head);
            assert!(
                head.starts_with("HTTP/1.1 101"),
                "expected a websocket upgrade, got: {head}"
            );
            Self { stream }
        }

        /// The next text frame's payload, or `None` if nothing arrived within `timeout`. Anything
        /// that is not a text frame (a ping, most likely) is read and discarded rather than
        /// returned, so a caller never has to know this protocol detail exists.
        pub async fn recv_text(&mut self, timeout: Duration) -> Option<String> {
            tokio::time::timeout(timeout, self.read_one_text_frame())
                .await
                .ok()
        }

        async fn read_one_text_frame(&mut self) -> String {
            loop {
                let mut header = [0u8; 2];
                self.stream
                    .read_exact(&mut header)
                    .await
                    .expect("read a frame header");
                let opcode = header[0] & 0x0F;
                let masked = header[1] & 0x80 != 0;
                let mut len = u64::from(header[1] & 0x7F);
                if len == 126 {
                    let mut ext = [0u8; 2];
                    self.stream
                        .read_exact(&mut ext)
                        .await
                        .expect("extended length");
                    len = u64::from(u16::from_be_bytes(ext));
                } else if len == 127 {
                    let mut ext = [0u8; 8];
                    self.stream
                        .read_exact(&mut ext)
                        .await
                        .expect("extended length");
                    len = u64::from_be_bytes(ext);
                }
                let mask = if masked {
                    let mut key = [0u8; 4];
                    self.stream.read_exact(&mut key).await.expect("mask key");
                    Some(key)
                } else {
                    None
                };
                let mut payload = vec![0u8; len as usize];
                self.stream
                    .read_exact(&mut payload)
                    .await
                    .expect("frame payload");
                if let Some(key) = mask {
                    for (i, byte) in payload.iter_mut().enumerate() {
                        *byte ^= key[i % 4];
                    }
                }
                // 0x1 = text. Everything else (ping/pong/binary/close) is not what this test reads
                // for, so it is consumed and the loop tries again.
                if opcode == 0x1 {
                    return String::from_utf8_lossy(&payload).to_string();
                }
                if opcode == 0x8 {
                    // A close frame: nothing more will ever arrive.
                    return String::new();
                }
            }
        }
    }
}
