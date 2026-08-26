//! The web admin panel — foundation only.
//!
//! What exists here: an embedded frontend, served through `axum`; a login flow reusing the same
//! account store `/register` and `/login` already use (no second credential system); a status
//! endpoint and a WebSocket that streams it live; and the runtime `panel` console command
//! ([`supervise`]) to toggle this on and off without a restart. What does *not* exist yet,
//! deliberately, per the plan this task came from: the player list, kick/ban, whitelist
//! management, world switching, the live console/chat view, and the world screen with player
//! avatars. Those are follow-up work.
//!
//! **Bundling**: `rust-embed`'s `axum` feature embeds the built frontend
//! (`crates/terrustia/web-panel/dist/`), served with a catch-all SPA route and an `index.html`
//! fallback for anything that isn't a real asset — the same shape `../alchemist`'s own
//! `src/server/mod.rs` uses (see this crate's `Cargo.toml` for the citation). The `embed-web`
//! cargo feature (on by default) toggles between that and reading straight from disk, for
//! iterating on the frontend without a full `cargo build`.
//!
//! **Off the game task, always.** Every handler here runs on the panel's own tokio task. Nothing
//! it does blocks the game's single-writer actor: reading live state goes through
//! [`ServerEvent::PanelStatus`]/[`ServerEvent::PanelAuthLookup`] (a channel send and an `.await` on
//! a `oneshot` reply, the same pattern the console's tab completion already uses), and the one
//! expensive operation — argon2 — runs in a `spawn_blocking` on the panel's own task, the same
//! discipline `admin::store::Admin::account_hash`'s doc comment requires for player logins.
//!
//! **Sessions** are an in-memory map owned by this module, not the account store: a session is a
//! panel-HTTP concern, not core game state, and doesn't need to survive a panel restart.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::admin::Account;
use crate::config::Config;
use crate::game::server::{PanelAuthLookup, PanelStatus, ServerEvent};

#[cfg(feature = "embed-web")]
#[derive(rust_embed::RustEmbed)]
#[folder = "web-panel/dist/"]
struct Assets;

fn load_static_asset(path: &str) -> Option<Vec<u8>> {
    #[cfg(feature = "embed-web")]
    {
        Assets::get(path).map(|f| f.data.into_owned())
    }
    #[cfg(not(feature = "embed-web"))]
    {
        // `path` comes straight from the request URI (`static_handler`), and `Uri::path()` does
        // not reject a literal `..` segment the way a browser's own address bar would — only
        // `embed-web`'s off-by-default disk-serving path (a local frontend-development
        // convenience, never compiled into a normal build) touches the filesystem with it, but a
        // dev build listening on localhost is still a real process on a real machine.
        if path.split('/').any(|segment| segment == "..") {
            return None;
        }
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("web-panel/dist")
                .join(path),
        )
        .ok()
    }
}

fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

#[derive(Clone)]
struct PanelState {
    events: mpsc::Sender<ServerEvent>,
    /// session token -> signed-in account name. In-memory only; a panel restart signs everyone
    /// out, which is fine — nothing here is persisted state.
    sessions: Arc<Mutex<HashMap<String, String>>>,
    started: Instant,
}

impl PanelState {
    /// The session map is a plain in-memory cache the panel can always reconstruct by asking
    /// people to sign in again — a poisoned lock (some other request panicked mid-access) is not
    /// worth losing every other session over, so this recovers the data rather than panicking.
    fn sessions(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Start the panel. Returns once the listener is bound, so a startup failure (the port already
/// in use, most likely) surfaces immediately rather than silently inside the spawned task.
pub async fn run(
    config: Config,
    events: mpsc::Sender<ServerEvent>,
) -> std::io::Result<tokio::task::JoinHandle<()>> {
    let listener = tokio::net::TcpListener::bind(config.panel_listen).await?;
    let addr = listener.local_addr().unwrap_or(config.panel_listen);
    let state = PanelState {
        events,
        sessions: Arc::new(Mutex::new(HashMap::new())),
        started: Instant::now(),
    };
    let router = Router::new()
        .route("/api/unclaimed", get(unclaimed))
        .route("/api/login", post(login))
        .route("/api/status", get(status))
        .route("/api/ws", get(ws_upgrade))
        .fallback(static_handler)
        .with_state(state);

    info!(%addr, "web panel listening (loopback only)");
    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            warn!(error = %e, "web panel stopped");
        }
    });
    Ok(handle)
}

/// Owns the panel's lifecycle for the life of the process, starting and stopping it each time a
/// toggle arrives on `toggle` — the other end of the console's `panel` command
/// (`GameServer::run_console`'s `"panel"` arm), which only ever sends a pulse and never touches
/// the actual [`tokio::task::JoinHandle`] itself: dropping a `JoinHandle` detaches rather than
/// stopping the task it names, so whatever holds it is the only thing that may [`abort`
/// it](tokio::task::JoinHandle::abort), and that has to be one single owner living for the whole
/// process, not something reconstructed per toggle.
///
/// `initial` is whatever `main` already started at boot (or `None`, the ordinary case — the panel
/// is opt-in). A bind failure *here*, after boot, is reported and left off rather than propagated:
/// unlike the startup path (`run`, called directly with `?` in `main`), a toggle happens against a
/// server already serving real players, and a configuration mistake discovered this way should not
/// take the rest of the server down with it.
pub async fn supervise(
    config: Config,
    events: mpsc::Sender<ServerEvent>,
    mut toggle: mpsc::UnboundedReceiver<()>,
    initial: Option<tokio::task::JoinHandle<()>>,
) {
    let mut handle = initial;
    while toggle.recv().await.is_some() {
        match handle.take() {
            Some(running) => {
                running.abort();
                info!("web panel stopped (console toggle)");
            }
            None => match run(config.clone(), events.clone()).await {
                Ok(started) => handle = Some(started),
                Err(e) => warn!(error = %e, "could not start the web panel"),
            },
        }
    }
}

// ---- static assets -------------------------------------------------------------------------

async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(body) = load_static_asset(path) {
        return ([(header::CONTENT_TYPE, content_type_for(path))], body).into_response();
    }
    // SPA fallback: anything not a real asset is a client-side route, so serve the app shell and
    // let the frontend's own router (once there is one) sort it out.
    if let Some(body) = load_static_asset("index.html") {
        return ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response();
    }
    (StatusCode::NOT_FOUND, "panel assets not built").into_response()
}

// ---- api -------------------------------------------------------------------------------------

#[derive(Serialize)]
struct ApiError {
    error: String,
}

fn err(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ApiError {
            error: message.into(),
        }),
    )
        .into_response()
}

#[derive(Serialize)]
struct UnclaimedResponse {
    unclaimed: bool,
}

async fn unclaimed(State(state): State<PanelState>) -> Response {
    match auth_lookup(&state, String::new()).await {
        Ok(lookup) => Json(UnclaimedResponse {
            unclaimed: lookup.unclaimed,
        })
        .into_response(),
        Err(resp) => resp,
    }
}

async fn auth_lookup(state: &PanelState, name: String) -> Result<PanelAuthLookup, Response> {
    let (reply, rx) = oneshot::channel();
    state
        .events
        .send(ServerEvent::PanelAuthLookup { name, reply })
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "the game is not running"))?;
    rx.await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "the game did not answer"))
}

#[derive(Deserialize)]
struct LoginRequest {
    name: String,
    password: String,
    claim_token: Option<String>,
}

#[derive(Serialize)]
struct LoginResponse {
    session: String,
    name: String,
}

async fn login(State(state): State<PanelState>, Json(req): Json<LoginRequest>) -> Response {
    let lookup = match auth_lookup(&state, req.name.clone()).await {
        Ok(l) => l,
        Err(resp) => return resp,
    };

    if lookup.unclaimed {
        // Mirrors `/register <name> <password> <token>`'s own rules, including the length check
        // `Admin::register` enforces — this path bypasses that function (it hashes inline, which
        // would stall the game task), so the rule has to be repeated here rather than shared.
        if lookup.claim_token.as_deref() != Some(req.claim_token.as_deref().unwrap_or_default()) {
            return err(StatusCode::UNAUTHORIZED, "wrong or missing claim token");
        }
        if req.password.len() < 6 {
            return err(
                StatusCode::BAD_REQUEST,
                "that password is too short; use at least six characters",
            );
        }
        let name = req.name.clone();
        let password = req.password.clone();
        let account =
            match tokio::task::spawn_blocking(move || Account::new(&name, &password, "everything"))
                .await
            {
                Ok(Ok(account)) => account,
                Ok(Err(e)) => return err(StatusCode::BAD_REQUEST, e),
                Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "hashing task panicked"),
            };
        let (reply, rx) = oneshot::channel();
        if state
            .events
            .send(ServerEvent::PanelInsertAccount { account, reply })
            .await
            .is_err()
        {
            return err(StatusCode::SERVICE_UNAVAILABLE, "the game is not running");
        }
        match rx.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return err(StatusCode::BAD_REQUEST, e),
            Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "the game did not answer"),
        }
        return issue_session(&state, req.name);
    }

    let Some((hash, _group)) = lookup.hash_and_group else {
        return err(StatusCode::UNAUTHORIZED, "no such account");
    };
    let password = req.password.clone();
    let ok = tokio::task::spawn_blocking(move || Account::verify_hash(&hash, &password))
        .await
        .unwrap_or(false);
    if !ok {
        return err(StatusCode::UNAUTHORIZED, "wrong name or password");
    }
    issue_session(&state, req.name)
}

fn issue_session(state: &PanelState, name: String) -> Response {
    // This token is not the same threat model as `GameServer::announce_claim_token`: that one
    // never leaves the local console and is spent once. This one *is* sent over the network (the
    // panel's own loopback socket, but still a socket) and, once issued, is the standing
    // credential for full panel control — start/stop, bans, world switching — for as long as the
    // session lives. A fast xorshift seeded from a nanosecond timestamp, the constant process id
    // and a small guessable session count is not unguessable enough for that. Uses the same real
    // CSPRNG `Account::new` already pulls in for password salts (`argon2::password_hash`'s own
    // `rand_core::OsRng` re-export), so this adds no new dependency.
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    state.sessions().insert(token.clone(), name.clone());
    Json(LoginResponse {
        session: token,
        name,
    })
    .into_response()
}

fn session_name(state: &PanelState, token: &str) -> Option<String> {
    state.sessions().get(token).cloned()
}

#[derive(Serialize)]
struct StatusResponse {
    uptime_secs: u64,
    player_count: usize,
    max_players: usize,
    world_name: String,
    version: &'static str,
    unclaimed: bool,
}

async fn build_status(state: &PanelState) -> Result<StatusResponse, Response> {
    let (reply, rx) = oneshot::channel();
    state
        .events
        .send(ServerEvent::PanelStatus { reply })
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "the game is not running"))?;
    let live: PanelStatus = rx
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "the game did not answer"))?;
    let lookup = auth_lookup(state, String::new()).await?;
    Ok(StatusResponse {
        uptime_secs: state.started.elapsed().as_secs(),
        player_count: live.player_count,
        max_players: live.max_players,
        world_name: live.world_name,
        version: env!("CARGO_PKG_VERSION"),
        unclaimed: lookup.unclaimed,
    })
}

async fn status(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return err(StatusCode::UNAUTHORIZED, "missing session");
    };
    if session_name(&state, token).is_none() {
        return err(StatusCode::UNAUTHORIZED, "invalid or expired session");
    }
    match build_status(&state).await {
        Ok(s) => Json(s).into_response(),
        Err(resp) => resp,
    }
}

fn bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

#[derive(Deserialize)]
struct WsQuery {
    session: String,
}

async fn ws_upgrade(
    State(state): State<PanelState>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    // A WebSocket upgrade can't carry a custom Authorization header from a browser, so the session
    // travels as a query parameter here instead — same token, same validation, just a different
    // transport for the one request type that needs it.
    if session_name(&state, &q.session).is_none() {
        return err(StatusCode::UNAUTHORIZED, "invalid or expired session");
    }
    ws.on_upgrade(move |socket| stream_status(socket, state))
}

async fn stream_status(mut socket: WebSocket, state: PanelState) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        interval.tick().await;
        let payload = match build_status(&state).await {
            Ok(s) => match serde_json::to_string(&s) {
                Ok(json) => json,
                Err(_) => continue,
            },
            Err(_) => break,
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            break;
        }
    }
}
