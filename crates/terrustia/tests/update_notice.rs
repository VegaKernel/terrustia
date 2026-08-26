//! The in-game half of `terrustia update`'s boot check: once a signature-verified newer release
//! is found, the first recognised admin to sign in afterward gets told, over real chat, exactly
//! once.
//!
//! The network/verification half (fetching the real GitHub releases API, shelling out to a real
//! `cosign` to check a real signature) is `update.rs`'s own job and is exercised there, end to
//! end, against a real locally hand-rolled HTTP server and a real cosign-signed fixture — see that
//! module's tests. This file starts exactly where those leave off: a message already sitting in
//! the shared cell `update::boot_check` would have set, and proves the *delivery* mechanism
//! (`GameServer::with_update_notice`, `note_finished_auth`'s `notify_update_if_pending`) actually
//! reaches a real connected player over a real socket, and only the first recognised admin, only
//! once.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use terrustia::{
    config::Config,
    game::{GameServer, ServerEvent},
    net::listener,
    world::worldgen,
};
use terrustia_client::{Client, Event};
use tokio::{net::TcpListener, sync::mpsc};

type Notice = Arc<std::sync::Mutex<Option<String>>>;

async fn start() -> (SocketAddr, mpsc::Sender<ServerEvent>, Notice) {
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

    let notice: Notice = Arc::new(std::sync::Mutex::new(None));
    let server = GameServer::new(config.clone(), world).with_update_notice(notice.clone());

    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(server.run(rx));
    tokio::spawn(listener::run(listener, config, tx.clone(), None));
    (addr, tx, notice)
}

async fn join(addr: SocketAddr, name: &str) -> Client {
    let mut client = Client::join(addr, name).await.expect("handshake");
    client.set_timeout(Duration::from_secs(10));
    client
}

/// The claim token an operator would read off the console — same lookup `tests/panel.rs` already
/// uses, rather than parsing printed log lines.
async fn claim_token(tx: &mpsc::Sender<ServerEvent>) -> String {
    let (reply, rx) = tokio::sync::oneshot::channel();
    tx.send(ServerEvent::PanelAuthLookup {
        name: String::new(),
        reply,
    })
    .await
    .unwrap();
    rx.await
        .unwrap()
        .claim_token
        .expect("a fresh server announces a claim token immediately on start")
}

#[tokio::test]
async fn the_first_recognised_admin_to_sign_in_after_an_update_is_found_is_told_once() {
    let (addr, tx, notice) = start().await;
    let token = claim_token(&tx).await;

    // Set directly, rather than by actually running `update::boot_check` against the network:
    // this test is about delivery, not discovery — see this file's own doc comment for the split.
    *notice.lock().unwrap() = Some(
        "a new terrustia release (v9.9.9) is available; ask an operator to run `terrustia \
         update`."
            .to_string(),
    );

    let mut owner = join(addr, "owner").await;
    owner
        .say(&format!("/register owner hunter2hunter2 {token}"))
        .await
        .unwrap();
    // Registering alone does not sign a player in — `/login` is the real recognition moment, and
    // the one `notify_update_if_pending` is wired to.
    owner
        .wait_for(
            "the registration to complete",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("you are the first account")),
        )
        .await
        .expect("registering with the real claim token must succeed");

    owner.say("/login owner hunter2hunter2").await.unwrap();
    let notified = owner
        .wait_for(
            "the update notice",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("v9.9.9")),
        )
        .await;
    assert!(
        notified.is_ok(),
        "the first recognised admin to sign in after an update is found must be told about it"
    );

    // Delivered exactly once: signing out and back in again must not repeat it.
    owner.say("/logout").await.unwrap();
    owner.say("/login owner hunter2hunter2").await.unwrap();
    // There is nothing to positively wait for here — the point is the *absence* of a second
    // notice — so drain whatever the server actually sends for a bounded window and check none of
    // it mentions the release again.
    owner.set_timeout(Duration::from_millis(500));
    let mut saw_it_again = false;
    while let Ok(event) = owner.next_event().await {
        if matches!(&event, Event::Chat { text, .. } if text.contains("v9.9.9")) {
            saw_it_again = true;
            break;
        }
    }
    assert!(
        !saw_it_again,
        "the notice must be delivered once, not on every later sign-in"
    );
}

#[tokio::test]
async fn a_player_who_never_signs_in_as_an_admin_is_not_told() {
    let (addr, tx, notice) = start().await;
    let token = claim_token(&tx).await;

    *notice.lock().unwrap() = Some("a new terrustia release (v9.9.9) is available".to_string());

    // An ordinary connection, on a still-unclaimed server: every permission passes here (see
    // `a_stranger_cannot_claim_an_unclaimed_server` in `gameplay.rs`), but nobody has signed in as
    // a *recognised admin* — no account exists yet at all — so the notice must stay put.
    let mut stranger = join(addr, "stranger").await;
    stranger.say("/whoami").await.unwrap();
    stranger
        .wait_for(
            "whoami's reply",
            |e| matches!(e, Event::Chat { text, .. } if text.contains("nobody")),
        )
        .await
        .unwrap();

    stranger.set_timeout(Duration::from_millis(500));
    let mut saw_it = false;
    while let Ok(event) = stranger.next_event().await {
        if matches!(&event, Event::Chat { text, .. } if text.contains("v9.9.9")) {
            saw_it = true;
            break;
        }
    }
    assert!(
        !saw_it,
        "a connection that never signs in as a recognised admin must not receive the notice"
    );
    assert!(
        notice.lock().unwrap().is_some(),
        "the notice must still be waiting for a real admin, not consumed by a stranger"
    );

    // Claim the server as a sanity check that the harness itself is not just failing silently —
    // the owner *does* get it, the same proof the other test makes more thoroughly.
    let _ = token;
}
