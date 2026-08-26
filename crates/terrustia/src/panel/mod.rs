//! The web admin panel.
//!
//! An embedded frontend, served through `axum`; a login flow reusing the same account store
//! `/register` and `/login` already use (no second credential system); a live status/console/chat
//! WebSocket; the runtime `panel` console command ([`supervise`]) to toggle this on and off
//! without a restart; and the full admin feature set: a player list with kick/ban, whitelist
//! management, world switching (a real graceful process restart — see
//! [`crate::game::server::GameServer::pending_world_switch`]'s doc comment for why that is the
//! honest answer rather than a hot-swap), a read-only settings view with the few fields that are
//! genuinely safe to change live, and a stylized, procedural live world view (see
//! `crates/terrustia/web-panel/src/lib/WorldView.svelte` for the rendering side, and this module's
//! `world_ws_upgrade` for the data it draws from — real positions and appearance data, real tile
//! types, never a composited Terraria sprite).
//!
//! **Bundling**: `rust-embed`'s `axum` feature embeds the built frontend
//! (`crates/terrustia/web-panel/dist/`), served with a catch-all SPA route and an `index.html`
//! fallback for anything that isn't a real asset — the same shape `../alchemist`'s own
//! `src/server/mod.rs` uses (see this crate's `Cargo.toml` for the citation). The `embed-web`
//! cargo feature (on by default) toggles between that and reading straight from disk, for
//! iterating on the frontend without a full `cargo build`.
//!
//! **Off the game task, always.** Every handler here runs on the panel's own tokio task. Nothing
//! it does blocks the game's single-writer actor: reading live state goes through a `ServerEvent`
//! (a channel send and an `.await` on a `oneshot` reply — [`ask`] is the one place that shape is
//! written down, and every handler below reuses it) and the one expensive operation — argon2 —
//! runs in a `spawn_blocking` on the panel's own task, the same discipline
//! `admin::store::Admin::account_hash`'s doc comment requires for player logins.
//!
//! **Every endpoint below `/api/` other than `/api/unclaimed` and `/api/login` requires a valid
//! session**, checked the same way `/api/status` already did before this module grew: a bearer
//! token (or, for the two WebSocket routes, a `session` query parameter — a browser cannot attach
//! a custom header to a WebSocket upgrade) looked up in [`PanelState::sessions`]. There is
//! deliberately no *further* permission check layered on top of that: a panel session already
//! requires a real, password-verified account on this server, which is a strictly narrower group
//! than "anyone who can reach the console" — the trust boundary `run_console`'s own doc comment
//! already accepts for every admin command typed there. Kick, ban, whitelist and console/chat
//! commands sent from the panel reuse exactly that: `run_admin_command`'s own logic, or (for raw
//! console/chat lines) `ServerEvent::Console` itself, the very channel the sticky console sends
//! down.
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

use crate::admin::{Account, BanKind};
use crate::config::Config;
use crate::game::server::{
    PanelAuthLookup, PanelConfigSnapshot, PanelPlayer, PanelStatus, PanelWhitelist, ServerEvent,
    TileColor,
};
use crate::term::{ConsoleLine, ConsoleLineKind};

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
        .route("/api/ws/world", get(world_ws_upgrade))
        .route("/api/players", get(players))
        .route("/api/players/kick", post(kick_player))
        .route("/api/players/ban", post(ban_player))
        .route("/api/players/unban", post(unban_player))
        .route("/api/whitelist", get(whitelist))
        .route("/api/whitelist/add", post(whitelist_add))
        .route("/api/whitelist/remove", post(whitelist_remove))
        .route("/api/worlds", get(worlds))
        .route("/api/worlds/switch", post(switch_world))
        .route("/api/config", get(config_snapshot))
        .route("/api/config/motd", post(set_motd))
        .route("/api/console", post(send_console))
        .route("/api/chat", post(send_chat))
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
    ask(state, |reply| ServerEvent::PanelAuthLookup { name, reply }).await
}

/// Send one `ServerEvent` built around a fresh `oneshot` reply, and wait for the game task to
/// answer it. Every handler below that needs live game state goes through this — a channel send
/// that can only fail because the game task is gone, and an `.await` that can only fail the same
/// way, both reported as the same "the game is not running" the panel has always said for this.
async fn ask<T>(
    state: &PanelState,
    build: impl FnOnce(oneshot::Sender<T>) -> ServerEvent,
) -> Result<T, Response> {
    let (reply, rx) = oneshot::channel();
    state
        .events
        .send(build(reply))
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "the game is not running"))?;
    rx.await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "the game did not answer"))
}

/// The session-check every endpoint other than `/api/unclaimed` and `/api/login` needs, pulled out
/// of `status` (the first handler to need it) so every handler added since shares the exact same
/// check rather than a plausible-looking copy of it.
async fn authorized(state: &PanelState, headers: &axum::http::HeaderMap) -> Result<(), Response> {
    let Some(token) = bearer_token(headers) else {
        return Err(err(StatusCode::UNAUTHORIZED, "missing session"));
    };
    if session_name(state, token).is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "invalid or expired session"));
    }
    Ok(())
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
    world_file: Option<String>,
    version: &'static str,
    unclaimed: bool,
}

async fn build_status(state: &PanelState) -> Result<StatusResponse, Response> {
    let live: PanelStatus = ask(state, |reply| ServerEvent::PanelStatus { reply }).await?;
    let lookup = auth_lookup(state, String::new()).await?;
    Ok(StatusResponse {
        uptime_secs: state.started.elapsed().as_secs(),
        player_count: live.player_count,
        max_players: live.max_players,
        world_name: live.world_name,
        world_file: live.world_file,
        version: env!("CARGO_PKG_VERSION"),
        unclaimed: lookup.unclaimed,
    })
}

async fn status(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
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

/// Every frame this socket sends, tagged by `type` so the frontend's one `onmessage` handler can
/// tell a status refresh apart from a console/chat line — see the module doc's note that this is
/// the same WebSocket the panel has always had, not a second one grown for this.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsMessage {
    Status(StatusResponse),
    Console {
        line_kind: &'static str,
        level: &'static str,
        text: String,
    },
}

async fn stream_status(mut socket: WebSocket, state: PanelState) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    // Subscribing here, not earlier, is deliberate: a `broadcast` receiver only ever sees frames
    // sent *after* it subscribes, so this socket's console tab starts from "now" rather than
    // trying to catch up on everything since the process started.
    let mut console_rx = crate::term::console_feed().subscribe();
    loop {
        let message = tokio::select! {
            _ = interval.tick() => {
                match build_status(&state).await {
                    Ok(s) => WsMessage::Status(s),
                    Err(_) => break,
                }
            }
            line = console_rx.recv() => {
                match line {
                    Ok(ConsoleLine { kind, level, text }) => WsMessage::Console {
                        line_kind: line_kind_name(kind),
                        level,
                        text,
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    // A slow consumer missed some lines; the next one it does receive is still
                    // real, so keep going rather than tearing the socket down over it.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        };
        if !send_ws(&mut socket, &message).await {
            break;
        }
    }
}

fn line_kind_name(kind: ConsoleLineKind) -> &'static str {
    match kind {
        ConsoleLineKind::Log => "log",
        ConsoleLineKind::Reply => "reply",
        ConsoleLineKind::Chat => "chat",
    }
}

/// Serialize and send one message, reporting whether the socket is still usable. A message that
/// fails to serialize is dropped rather than treated as a dead socket — the connection itself is
/// fine, only that one frame was not worth sending.
async fn send_ws<T: Serialize>(socket: &mut WebSocket, message: &T) -> bool {
    let Ok(payload) = serde_json::to_string(message) else {
        return true;
    };
    socket.send(Message::Text(payload.into())).await.is_ok()
}

// ---- players: list, kick, ban, unban ---------------------------------------------------------

#[derive(Serialize)]
struct AppearanceResponse {
    skin_variant: u8,
    hair_style: u8,
    hair_color: [u8; 3],
    skin_color: [u8; 3],
    eye_color: [u8; 3],
    shirt_color: [u8; 3],
    undershirt_color: [u8; 3],
    pants_color: [u8; 3],
    shoe_color: [u8; 3],
}

impl From<terrustia_proto::player_info::PlayerAppearance> for AppearanceResponse {
    fn from(a: terrustia_proto::player_info::PlayerAppearance) -> Self {
        Self {
            skin_variant: a.skin_variant,
            hair_style: a.hair_style,
            hair_color: a.hair_color,
            skin_color: a.skin_color,
            eye_color: a.eye_color,
            shirt_color: a.shirt_color,
            undershirt_color: a.undershirt_color,
            pants_color: a.pants_color,
            shoe_color: a.shoe_color,
        }
    }
}

/// A connected player, over the wire — real position and appearance data for the world view to
/// draw a stylized avatar from, and enough else (health, mana, address) for the player list.
#[derive(Serialize)]
struct PlayerResponse {
    slot: u8,
    name: String,
    address: String,
    life: i16,
    life_max: i16,
    mana: i16,
    mana_max: i16,
    x: f32,
    y: f32,
    pvp: bool,
    appearance: Option<AppearanceResponse>,
    equipped: Vec<i32>,
}

impl From<PanelPlayer> for PlayerResponse {
    fn from(p: PanelPlayer) -> Self {
        Self {
            slot: p.slot,
            name: p.name,
            address: p.address,
            life: p.life,
            life_max: p.life_max,
            mana: p.mana,
            mana_max: p.mana_max,
            x: p.position.0,
            y: p.position.1,
            pvp: p.pvp,
            appearance: p.appearance.map(AppearanceResponse::from),
            equipped: p.equipped,
        }
    }
}

async fn players(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelPlayers { reply }).await {
        Ok(players) => Json(
            players
                .into_iter()
                .map(PlayerResponse::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
struct KickRequest {
    name: String,
    #[serde(default)]
    reason: String,
}

async fn kick_player(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<KickRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelKick {
        name: req.name,
        reason: req.reason,
        reply,
    })
    .await
    {
        Ok(Ok(())) => StatusCode::OK.into_response(),
        Ok(Err(e)) => err(StatusCode::NOT_FOUND, e),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
struct BanRequest {
    /// `"name"`, `"ip"` (or `"address"`) or `"uuid"` — the same three words `/ban` accepts.
    kind: String,
    value: String,
    #[serde(default)]
    reason: String,
}

async fn ban_player(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<BanRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    let Some(kind) = BanKind::parse(&req.kind) else {
        return err(StatusCode::BAD_REQUEST, "kind must be name, ip or uuid");
    };
    match ask(&state, |reply| ServerEvent::PanelBan {
        kind,
        value: req.value,
        reason: req.reason,
        reply,
    })
    .await
    {
        Ok(()) => StatusCode::OK.into_response(),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
struct UnbanRequest {
    value: String,
}

#[derive(Serialize)]
struct UnbanResponse {
    removed: usize,
}

async fn unban_player(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<UnbanRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelUnban {
        value: req.value,
        reply,
    })
    .await
    {
        Ok(removed) => Json(UnbanResponse { removed }).into_response(),
        Err(resp) => resp,
    }
}

// ---- whitelist ---------------------------------------------------------------------------------

#[derive(Serialize)]
struct WhitelistResponse {
    on: bool,
    names: Vec<String>,
}

impl From<PanelWhitelist> for WhitelistResponse {
    fn from(w: PanelWhitelist) -> Self {
        Self {
            on: w.on,
            names: w.names,
        }
    }
}

async fn whitelist(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelWhitelist { reply }).await {
        Ok(w) => Json(WhitelistResponse::from(w)).into_response(),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
struct NameRequest {
    name: String,
}

#[derive(Serialize)]
struct ChangedResponse {
    changed: bool,
}

async fn whitelist_add(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<NameRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelWhitelistAdd {
        name: req.name,
        reply,
    })
    .await
    {
        Ok(changed) => Json(ChangedResponse { changed }).into_response(),
        Err(resp) => resp,
    }
}

async fn whitelist_remove(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<NameRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelWhitelistRemove {
        name: req.name,
        reply,
    })
    .await
    {
        Ok(changed) => Json(ChangedResponse { changed }).into_response(),
        Err(resp) => resp,
    }
}

// ---- worlds: list and switch --------------------------------------------------------------------

#[derive(Serialize)]
struct WorldEntry {
    name: String,
    size_mb: f64,
    /// Whether this is the world the running process currently has open.
    current: bool,
}

async fn worlds(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    // Listing the world directory is a plain filesystem read that needs no game-task state; only
    // which one is *current* does, so that alone goes through the game task.
    let current_file = match ask(&state, |reply| ServerEvent::PanelStatus { reply }).await {
        Ok(status) => status.world_file,
        Err(resp) => return resp,
    };
    let entries: Vec<WorldEntry> = crate::worlds::list()
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?")
                .to_string();
            let size_mb = std::fs::metadata(&path)
                .map(|m| m.len() as f64 / 1_048_576.0)
                .unwrap_or(0.0);
            let current = current_file.as_deref() == Some(name.as_str());
            WorldEntry {
                name,
                size_mb,
                current,
            }
        })
        .collect();
    Json(entries).into_response()
}

#[derive(Deserialize)]
struct SwitchWorldRequest {
    name: String,
}

async fn switch_world(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SwitchWorldRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    // The panel never accepts a raw path from the client — only a name matched against what
    // `worlds::list()` itself found on disk, so there is no path here a request body could smuggle
    // in that the world directory does not already contain.
    let Some(path) = crate::worlds::list().into_iter().find(|p| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|stem| stem == req.name)
    }) else {
        return err(
            StatusCode::NOT_FOUND,
            format!("no world called {}", req.name),
        );
    };
    match ask(&state, |reply| ServerEvent::PanelSwitchWorld {
        path,
        reply,
    })
    .await
    {
        Ok(Ok(())) => StatusCode::OK.into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_REQUEST, e),
        Err(resp) => resp,
    }
}

// ---- settings ------------------------------------------------------------------------------------

#[derive(Serialize)]
struct ConfigResponse {
    listen: String,
    max_players: usize,
    world_width: i32,
    world_height: i32,
    motd: String,
    password_set: bool,
    max_chat_len: usize,
    idle_timeout_secs: u64,
    autosave_secs: u64,
    save_target: Option<String>,
    whitelist_on: bool,
    whitelist_count: usize,
}

impl From<PanelConfigSnapshot> for ConfigResponse {
    fn from(c: PanelConfigSnapshot) -> Self {
        Self {
            listen: c.listen.to_string(),
            max_players: c.max_players,
            world_width: c.world_width,
            world_height: c.world_height,
            motd: c.motd,
            password_set: c.password_set,
            max_chat_len: c.max_chat_len,
            idle_timeout_secs: c.idle_timeout_secs,
            autosave_secs: c.autosave_secs,
            save_target: c.save_target,
            whitelist_on: c.whitelist_on,
            whitelist_count: c.whitelist_count,
        }
    }
}

async fn config_snapshot(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelConfigSnapshot { reply }).await {
        Ok(c) => Json(ConfigResponse::from(c)).into_response(),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
struct MotdRequest {
    motd: String,
}

async fn set_motd(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<MotdRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelSetMotd {
        motd: req.motd,
        reply,
    })
    .await
    {
        Ok(()) => StatusCode::OK.into_response(),
        Err(resp) => resp,
    }
}

// ---- console / chat --------------------------------------------------------------------------

/// Only the first line of whatever was sent, trimmed. A panel session is already as trusted as the
/// console (see this module's doc comment), so this is not a permission boundary — it exists so a
/// chat message or command containing an embedded newline cannot smuggle a second command in past
/// whatever the operator thought they were sending.
fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("").trim()
}

#[derive(Deserialize)]
struct ConsoleRequest {
    line: String,
}

async fn send_console(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ConsoleRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    let line = first_line(&req.line);
    if line.is_empty() {
        return err(StatusCode::BAD_REQUEST, "empty command");
    }
    if state
        .events
        .send(ServerEvent::Console {
            line: line.to_string(),
        })
        .await
        .is_err()
    {
        return err(StatusCode::SERVICE_UNAVAILABLE, "the game is not running");
    }
    StatusCode::OK.into_response()
}

#[derive(Deserialize)]
struct ChatRequest {
    text: String,
}

async fn send_chat(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers).await {
        return resp;
    }
    let text = first_line(&req.text);
    if text.is_empty() {
        return err(StatusCode::BAD_REQUEST, "empty message");
    }
    // Exactly what the console's own `say` command does — see `run_console`'s `"say"` arm.
    if state
        .events
        .send(ServerEvent::Console {
            line: format!("say {text}"),
        })
        .await
        .is_err()
    {
        return err(StatusCode::SERVICE_UNAVAILABLE, "the game is not running");
    }
    StatusCode::OK.into_response()
}

// ---- the live world view --------------------------------------------------------------------

async fn world_ws_upgrade(
    State(state): State<PanelState>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if session_name(&state, &q.session).is_none() {
        return err(StatusCode::UNAUTHORIZED, "invalid or expired session");
    }
    ws.on_upgrade(move |socket| stream_world(socket, state))
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WorldWsMessage {
    Players {
        players: Vec<PlayerResponse>,
    },
    Tiles {
        world_width: i32,
        world_height: i32,
        sample_cols: u32,
        sample_rows: u32,
        tiles: Vec<&'static str>,
    },
}

fn tile_color_name(color: TileColor) -> &'static str {
    match color {
        TileColor::Empty => "empty",
        TileColor::Dirt => "dirt",
        TileColor::Stone => "stone",
        TileColor::Grass => "grass",
        TileColor::Corruption => "corruption",
        TileColor::Crimson => "crimson",
        TileColor::Sand => "sand",
        TileColor::Snow => "snow",
        TileColor::Ice => "ice",
        TileColor::Jungle => "jungle",
        TileColor::Ore => "ore",
        TileColor::Gem => "gem",
        TileColor::Water => "water",
        TileColor::Lava => "lava",
        TileColor::Honey => "honey",
        TileColor::Ash => "ash",
        TileColor::Other => "other",
    }
}

/// Players refresh often enough to look live (twice a second); tiles are sampled far less often —
/// the world's shape barely changes tick to tick, and even the bounded sample in
/// `GameServer::world_tile_sample` is not worth recomputing ten times as often as anyone could see
/// a difference. `tokio::time::interval` fires its first tick immediately, so both kinds of frame
/// reach a freshly connected client right away rather than after their first full period.
async fn stream_world(mut socket: WebSocket, state: PanelState) {
    let mut player_interval = tokio::time::interval(Duration::from_millis(500));
    let mut tile_interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        let message = tokio::select! {
            _ = player_interval.tick() => {
                match ask(&state, |reply| ServerEvent::PanelPlayers { reply }).await {
                    Ok(players) => WorldWsMessage::Players {
                        players: players.into_iter().map(PlayerResponse::from).collect(),
                    },
                    Err(_) => break,
                }
            }
            _ = tile_interval.tick() => {
                match ask(&state, |reply| ServerEvent::PanelWorldTiles { reply }).await {
                    Ok(t) => WorldWsMessage::Tiles {
                        world_width: t.world_width,
                        world_height: t.world_height,
                        sample_cols: t.sample_cols,
                        sample_rows: t.sample_rows,
                        tiles: t.tiles.into_iter().map(tile_color_name).collect(),
                    },
                    Err(_) => break,
                }
            }
        };
        if !send_ws(&mut socket, &message).await {
            break;
        }
    }
}
