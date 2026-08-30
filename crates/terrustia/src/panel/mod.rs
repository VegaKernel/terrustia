//! The web admin panel.
//!
//! An embedded frontend, served through `axum`; a login flow reusing the same account store
//! `/register` and `/login` already use (no second credential system); a live status/console/chat
//! WebSocket; the runtime `panel` console command ([`supervise`]) to toggle this on and off
//! without a restart, and whose real inner task cannot outlive `supervise` itself under any
//! circumstance — see [`PanelHandle`]'s doc comment for the real shutdown bug that guarantee
//! closes; and the full admin feature set: a player list with kick/ban, whitelist
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
//! session and its own permission**, checked by [`authorized`] (or [`authorized_token`] for the two
//! WebSocket routes, which carry the session as a `session` query parameter instead of a bearer
//! header — a browser cannot attach a custom header to a WebSocket upgrade): a token looked up in
//! [`PanelState::sessions`] to find the signed-in account, then a fresh, per-request check —
//! against the game task's live `Admin` store, not anything cached at login — that the account's
//! group grants the specific permission that route needs. `login` itself only requires `panel.view`
//! to issue a session at all; a `default` account (which does not hold it) cannot get a session,
//! and a `moderator` one can sign in but still gets `403` from, say, `/api/console`.
//!
//! Route → permission map (see `admin::group::perm` for what each name actually gates):
//!
//! | route | permission |
//! |---|---|
//! | `/api/status`, `/api/players`, `/api/whitelist` (view), `/api/worlds` (view), `/api/config` (view), `/api/metrics`, `/api/backups`, `/api/worlds/new/status`, `/api/ws`, `/api/ws/world` | `panel.view` |
//! | `/api/console`, `/api/chat` | `panel.console` — a raw line down the same unrestricted channel the sticky console uses |
//! | `/api/players/kick` | `server.kick` |
//! | `/api/players/ban` | `server.ban` |
//! | `/api/players/unban` | `server.unban` |
//! | `/api/players/mute`, `/api/players/unmute` | `server.mute` / `server.unmute` |
//! | `/api/whitelist/add`, `/api/whitelist/remove` | `server.whitelist` |
//! | `/api/config/motd` | `world.motd` |
//! | `/api/worlds/switch` | `world.switch` |
//! | `/api/worlds/new` | `world.new` |
//! | `/api/save` | `world.save` |
//! | `/api/rollback` | `world.rollback` |
//! | `/api/accounts` (view, create, delete) | `admin.accounts` |
//! | `/api/accounts/group` | `admin.accounts`, plus an anti-escalation reach check (see `Admin::group_within_reach`) |
//! | `/api/permissions`, `/api/groups/permissions` | `admin.groups`, the group-permission-set editor — plus, for a grant, the same reach check |
//! | `/api/audit` | `admin.audit` |
//!
//! Kick, ban, whitelist and console/chat commands sent from the panel reuse `run_admin_command`'s
//! own logic, or (for raw console/chat lines) `ServerEvent::Console` itself, the very channel the
//! sticky console sends down.
//!
//! **Sessions** are an in-memory map owned by this module, not the account store: a session is a
//! panel-HTTP concern, not core game state, and doesn't need to survive a panel restart.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::admin::{Account, BanKind, Verdict, perm};
use crate::config::Config;
use crate::game::server::{
    PanelAccountInfo, PanelAuthLookup, PanelBackupEntry, PanelBackups, PanelConfigSnapshot,
    PanelGroupInfo, PanelMetrics, PanelPlayer, PanelStatus, PanelWhitelist, ServerEvent, TileColor,
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
    /// The one background world-generation job, if any has been started this panel lifetime.
    /// Worldgen is slow (seconds, and a lot of memory) and pure — it never touches the game task —
    /// so it runs on its own `spawn_blocking` thread and reports progress through this shared cell
    /// rather than blocking the request that kicked it off. Only one at a time.
    worldgen: Arc<Mutex<WorldGenJob>>,
    /// Per-caller-address `/api/login` backoff. Panel-local rather than shared with the game
    /// task's own `/login` throttle: this surface is loopback-only, a different (much smaller)
    /// trust boundary, and reaching across to the game task's state for every check would cost a
    /// channel round trip this handler has no other reason to pay. See `admin::throttle`'s top doc
    /// for the mechanism itself, which is identical either way.
    ip_throttle: Arc<Mutex<crate::admin::Throttle>>,
    /// Per-account (lowercased, as typed) `/api/login` backoff. See [`Self::ip_throttle`]'s doc
    /// comment for why this is panel-local, and `admin::throttle`'s top doc for why both exist.
    account_throttle: Arc<Mutex<crate::admin::Throttle>>,
}

/// Where a background world generation has got to. Coarse on purpose: `worldgen::generate` is a
/// single blocking call with no progress callback, so there is no honest percentage to report —
/// only which of these states it is in, and how long it has been running.
#[derive(Clone, Default)]
struct WorldGenJob {
    status: GenStatus,
    /// The world's name, echoed back so the panel can label the job it is watching.
    name: String,
    /// The file stem of the finished world, on success, so the panel can offer to switch to it.
    world_file: Option<String>,
    /// A human-readable line: the error on failure, or a short note on success.
    message: String,
    /// When the current (or most recent) job started, for an elapsed-seconds readout.
    #[allow(clippy::struct_field_names)]
    started: Option<Instant>,
    /// How long the finished job took, frozen once it is no longer running.
    elapsed_secs: Option<u64>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum GenStatus {
    /// Nothing has ever been started this panel lifetime.
    #[default]
    Idle,
    Running,
    Done,
    Failed,
}

impl GenStatus {
    fn as_str(self) -> &'static str {
        match self {
            GenStatus::Idle => "idle",
            GenStatus::Running => "running",
            GenStatus::Done => "done",
            GenStatus::Failed => "failed",
        }
    }
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

    /// The world-generation job cell, recovering a poisoned lock the same way [`Self::sessions`]
    /// does and for the same reason — a background gen thread that panicked mid-write should not
    /// take out every future status read.
    fn worldgen(&self) -> std::sync::MutexGuard<'_, WorldGenJob> {
        self.worldgen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The per-address login-throttle map, recovering a poisoned lock the same way
    /// [`Self::sessions`] does and for the same reason.
    fn ip_throttle(&self) -> std::sync::MutexGuard<'_, crate::admin::Throttle> {
        self.ip_throttle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The per-account login-throttle map, recovering a poisoned lock the same way
    /// [`Self::sessions`] does and for the same reason.
    fn account_throttle(&self) -> std::sync::MutexGuard<'_, crate::admin::Throttle> {
        self.account_throttle
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
    // Defence in depth: the panel never faces the network. `Config::validate` already refuses a
    // non-loopback `panel_listen`, but refusing to bind one here too means no path — a future
    // caller that skips validation, a bug — can ever expose this surface.
    if !config.panel_listen.ip().is_loopback() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "panel_listen must be loopback; refusing to bind {}",
                config.panel_listen
            ),
        ));
    }
    let listener = crate::net::listener::bind(config.panel_listen).await?;
    let addr = listener.local_addr().unwrap_or(config.panel_listen);
    let state = PanelState {
        events,
        sessions: Arc::new(Mutex::new(HashMap::new())),
        started: Instant::now(),
        worldgen: Arc::new(Mutex::new(WorldGenJob::default())),
        ip_throttle: Arc::new(Mutex::new(crate::admin::Throttle::new())),
        account_throttle: Arc::new(Mutex::new(crate::admin::Throttle::new())),
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
        .route("/api/players/mute", post(mute_player))
        .route("/api/players/unmute", post(unmute_player))
        .route("/api/whitelist", get(whitelist))
        .route("/api/whitelist/add", post(whitelist_add))
        .route("/api/whitelist/remove", post(whitelist_remove))
        .route("/api/worlds", get(worlds))
        .route("/api/worlds/switch", post(switch_world))
        .route("/api/worlds/new", post(new_world))
        .route("/api/worlds/new/status", get(new_world_status))
        .route("/api/config", get(config_snapshot))
        .route("/api/config/motd", post(set_motd))
        .route("/api/console", post(send_console))
        .route("/api/chat", post(send_chat))
        .route("/api/metrics", get(metrics))
        .route("/api/backups", get(backups))
        .route("/api/save", post(force_save))
        .route("/api/rollback", post(rollback))
        .route("/api/accounts", get(accounts))
        .route("/api/accounts/group", post(set_account_group))
        .route("/api/accounts/create", post(create_account))
        .route("/api/accounts/delete", post(delete_account))
        .route("/api/permissions", get(known_permissions))
        .route("/api/groups/permissions", post(set_group_permission))
        .route("/api/audit", get(audit_log))
        .fallback(static_handler)
        .with_state(state);

    info!(%addr, "web panel listening (loopback only)");
    let handle = tokio::spawn(async move {
        // `with_connect_info` so `login`'s `ConnectInfo<SocketAddr>` extractor can see the real
        // caller address for its per-address throttle. Every other handler ignores it.
        if let Err(e) = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        {
            warn!(error = %e, "web panel stopped");
        }
    });
    Ok(handle)
}

/// Owns the panel's real inner [`JoinHandle`](tokio::task::JoinHandle) — the axum-serving task that
/// holds the live `TcpListener` and its own clone of `events` — and aborts it whenever this guard
/// is dropped, for *any* reason: the console toggle taking it out and aborting it explicitly (the
/// ordinary case, below), or [`supervise`]'s own task ending some other way, which a plain
/// `Option<JoinHandle<()>>` would not have caught.
///
/// That second case is a real bug this guard exists to close, not a hypothetical: `main`'s shutdown
/// sequence `.abort()`s `supervise`'s *outer* task (alongside `accept`/`console`) so every clone of
/// `events_tx` it might be holding actually gets dropped, which is what lets `GameServer::run`'s
/// `events.recv() => None => break` exit path fire at all. But cancelling `supervise`'s future used
/// to just drop its local `handle` variable on the way out — and dropping a `JoinHandle` detaches
/// rather than stops the task it names (this module's own top doc already says so, for the toggle
/// path; the gap was that `supervise`'s *own* cancellation went through the exact same trap). If the
/// panel was running when a real `SIGTERM` arrived, that left the real inner task — and the live
/// `events` clone captured in its `PanelState` — running forever, detached, so `GameServer::run`
/// never saw its last sender go away and a `SIGTERM` logged "shutting down" and then never actually
/// stopped the server. Wrapping the handle in a type whose `Drop` aborts it makes this structurally
/// impossible regardless of *how* `supervise`'s future stops, rather than depending on every call
/// site that might end it to also remember to reach in and abort the inner handle by hand.
struct PanelHandle(Option<tokio::task::JoinHandle<()>>);

impl PanelHandle {
    fn take(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        self.0.take()
    }
}

impl Drop for PanelHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

/// Owns the panel's lifecycle for the life of the process, starting and stopping it each time a
/// toggle arrives on `toggle` — the other end of the console's `panel` command
/// (`GameServer::run_console`'s `"panel"` arm), which only ever sends a pulse and never touches
/// the actual [`tokio::task::JoinHandle`] itself: dropping a `JoinHandle` detaches rather than
/// stopping the task it names, so whatever holds it is the only thing that may [`abort`
/// it](tokio::task::JoinHandle::abort), and that has to be one single owner living for the whole
/// process, not something reconstructed per toggle. That owner is [`PanelHandle`] — see its own doc
/// comment for the real bug (a leaked panel task defeating `SIGTERM` shutdown) its `Drop` impl
/// closes, on top of the explicit `.abort()` the toggle path below already did correctly.
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
    let mut handle = PanelHandle(initial);
    while toggle.recv().await.is_some() {
        match handle.take() {
            Some(running) => {
                running.abort();
                info!("web panel stopped (console toggle)");
            }
            None => match run(config.clone(), events.clone()).await {
                Ok(started) => handle = PanelHandle(Some(started)),
                Err(e) => warn!(error = %e, "could not start the web panel"),
            },
        }
    }
    // The loop above only ends if `toggle`'s sender side is ever dropped (it currently never is —
    // `main` holds it for the process's whole life) — but if it ever does, `handle`'s own `Drop`
    // impl aborts whatever real inner task it's still holding on the way out, same as the explicit
    // `.abort()` from outside covers. Nothing to do here by name; this comment exists so a future
    // reader doesn't go looking for a missing cleanup call.
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

/// The session-and-permission check every endpoint other than `/api/unclaimed` and `/api/login`
/// needs: a valid session (a bearer token that resolves to a signed-in account), and that account's
/// group currently granting `permission`. Checked fresh against the game task's live `Admin` store
/// on *every* call rather than cached at login — a group's permissions, or an account's own group,
/// can change mid-session (through this very panel), and a session issued before that change must
/// not go on behaving as if it never happened.
///
/// Returns the account name on success: a handful of handlers (anything that can hand power to
/// someone else — creating an account, moving one between groups, editing a group's own permission
/// set) need to know who is asking, for `Admin::group_within_reach`'s anti-escalation check.
///
/// See this module's own doc comment for the full route-to-permission map.
async fn authorized(
    state: &PanelState,
    headers: &axum::http::HeaderMap,
    permission: crate::admin::Permission,
) -> Result<String, Response> {
    let Some(token) = bearer_token(headers) else {
        return Err(err(StatusCode::UNAUTHORIZED, "missing session"));
    };
    authorized_token(state, token, permission).await
}

/// The token-and-permission half of [`authorized`], factored out so the two WebSocket upgrades
/// ([`ws_upgrade`], [`world_ws_upgrade`]) can reuse the exact same check with a session that
/// arrived as a query parameter rather than a bearer header (a browser cannot attach a custom
/// header to a WebSocket upgrade request).
async fn authorized_token(
    state: &PanelState,
    token: &str,
    permission: crate::admin::Permission,
) -> Result<String, Response> {
    let Some(name) = session_name(state, token) else {
        return Err(err(StatusCode::UNAUTHORIZED, "invalid or expired session"));
    };
    let allowed = ask(state, |reply| ServerEvent::PanelAuthorize {
        name: name.clone(),
        permission: permission.as_str().to_string(),
        reply,
    })
    .await?;
    if !allowed {
        return Err(err(
            StatusCode::FORBIDDEN,
            format!("this account does not have '{}'", permission.as_str()),
        ));
    }
    Ok(name)
}

// `password` and `claim_token` below are never logged, at any level, anywhere in `login` or the
// functions it calls: see `admin::mod`'s own "never logged" convention. `#[derive(Deserialize)]`
// gives this struct no `Debug`/`Display` on purpose: an errant `{req:?}` in a future edit would
// fail to compile instead of quietly printing both.
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

async fn login(
    State(state): State<PanelState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginRequest>,
) -> Response {
    let lookup = match auth_lookup(&state, req.name.clone()).await {
        Ok(l) => l,
        Err(resp) => return resp,
    };

    if lookup.unclaimed {
        // Not throttled: there is no existing password to check here at all (the claim path only
        // ever creates the very first account), the same reasoning that leaves `/register` on a
        // claimed server unthrottled too: see `TODO.md`'s Lane F entry. The token itself is a
        // ~59-bit random secret (`GameServer::announce_claim_token`), not a guessable password;
        // brute-forcing it is already infeasible, and it is spent after one use regardless.
        //
        // Constant-time for the same reason the console's own claim-token compare is
        // (`console::run_admin_command`'s `"register"` arm): it is a secret compared byte for
        // byte, so a plain `!=` would leak it one byte at a time through timing. `claim_token`
        // being `None` (nothing to claim) always refuses, exactly as the old `!=` did.
        let offered = req.claim_token.as_deref().unwrap_or_default();
        let token_ok = lookup.claim_token.as_deref().is_some_and(|expected| {
            crate::admin::constant_time_eq(expected.as_bytes(), offered.as_bytes())
        });
        if !token_ok {
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
        // The first account owns the server, so it goes into the `owner` group — the one
        // `group::defaults` gives the `*` permission. This mirrors the console `claim`/`register`
        // path exactly (`GameServer::run_console`'s `"claim"` arm and `announce_claim_token`'s own
        // owner branch both use `"owner"`). An earlier draft passed `"everything"`, which is not a
        // group any server actually has — it resolves to `default`, silently leaving the very first
        // account unable to administer anything.
        let account = match tokio::task::spawn_blocking(move || {
            Account::new(&name, &password, "owner")
        })
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

    // Checked before the lookup result is even consulted: a throttled attempt must not learn
    // "no such account" vs. "wrong password" any faster or slower than usual, so both are refused
    // the same way, this early, with the one shared `REFUSAL_MESSAGE`. See `admin::throttle`'s
    // top doc and `login_throttled`'s doc comment on the game-task side for the same rule applied
    // to `/login`.
    let ip_key = addr.ip().to_string();
    let account_key = req.name.to_ascii_lowercase();
    let now = Instant::now();
    // Each verdict is read out of its `MutexGuard` into a plain value *before* the `if let` below:
    // a guard borrowed straight in an `if let`'s own condition stays locked for the rest of that
    // block by Rust's temporary-lifetime rules, and `record_throttled`'s `.await` inside it would
    // then be holding a `std::sync::MutexGuard` (not `Send`) across a suspend point.
    let ip_verdict = state.ip_throttle().check(&ip_key, now);
    let account_verdict = state.account_throttle().check(&account_key, now);
    let mut refused = false;
    if let Verdict::Refused { log_summary, .. } = ip_verdict {
        refused = true;
        if let Some(n) = log_summary {
            record_throttled(&state, format!("ip:{ip_key}"), n).await;
        }
    }
    if let Verdict::Refused { log_summary, .. } = account_verdict {
        refused = true;
        if let Some(n) = log_summary {
            record_throttled(&state, format!("account:{account_key}"), n).await;
        }
    }
    if refused {
        return err(StatusCode::TOO_MANY_REQUESTS, crate::admin::REFUSAL_MESSAGE);
    }

    let Some((hash, _group)) = lookup.hash_and_group else {
        return err(StatusCode::UNAUTHORIZED, "no such account");
    };
    let password = req.password.clone();
    let ok = tokio::task::spawn_blocking(move || Account::verify_hash(&hash, &password))
        .await
        .unwrap_or(false);
    if ok {
        state.ip_throttle().record_success(&ip_key);
        state.account_throttle().record_success(&account_key);
    } else {
        state.ip_throttle().record_failure(&ip_key, now);
        state.account_throttle().record_failure(&account_key, now);
        return err(StatusCode::UNAUTHORIZED, "wrong name or password");
    }
    // Authenticating an account is not enough to hold a panel session: the account's group must
    // grant `panel.view`. Every route reached with that session then checks its own permission (see
    // this module's doc comment for the full mapping) — a `default` account, which does not hold
    // `panel.view`, cannot get a session at all; a `moderator` one can sign in but gets `403` from
    // `/api/console` all the same.
    if !lookup.panel_view {
        return err(
            StatusCode::FORBIDDEN,
            "this account is not permitted to use the admin panel",
        );
    }
    issue_session(&state, req.name)
}

/// Sends the game task one summarised audit-log line for a login-throttle refusal. See
/// `admin::throttle::Verdict::Refused`'s own doc comment for why this is called only once per
/// summarised window rather than once per refusal. Fire-and-forget: `login` has already decided
/// to refuse the request regardless of whether this record lands, matching `AuditLog::record`'s
/// own "a write failure must never block the action it is recording" rule: if the game task is
/// gone, there is nobody left to hold a session against anyway.
async fn record_throttled(state: &PanelState, target: String, count: u32) {
    let _ = state
        .events
        .send(ServerEvent::PanelAuditThrottled {
            target,
            detail: format!("{count} refused login attempt(s) backed off"),
        })
        .await;
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
    /// The calling account's own permission strings — a UX convenience so the frontend can choose
    /// which tabs and buttons to show; every route still re-checks its own permission server-side
    /// regardless of what this says. See [`PanelAuthLookup::permissions`].
    permissions: Vec<String>,
    /// How many world saves have failed in a row. `0` is healthy; see
    /// [`PanelStatus::save_failures`]'s own doc comment for what this means and where it comes
    /// from — this is a straight passthrough, not a second copy of the logic.
    save_failures: u32,
}

/// `account` is the signed-in account making the request — its own permissions are what the
/// response's `permissions` field carries, so this must be the real caller, not an empty
/// placeholder (an earlier draft used `String::new()` here, before this field existed, when only
/// the server-wide `unclaimed` flag was needed from `auth_lookup`).
async fn build_status(state: &PanelState, account: &str) -> Result<StatusResponse, Response> {
    let live: PanelStatus = ask(state, |reply| ServerEvent::PanelStatus { reply }).await?;
    let lookup = auth_lookup(state, account.to_string()).await?;
    Ok(StatusResponse {
        uptime_secs: state.started.elapsed().as_secs(),
        player_count: live.player_count,
        max_players: live.max_players,
        world_name: live.world_name,
        world_file: live.world_file,
        version: env!("CARGO_PKG_VERSION"),
        unclaimed: lookup.unclaimed,
        permissions: lookup.permissions,
        save_failures: live.save_failures,
    })
}

async fn status(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    let account = match authorized(&state, &headers, perm::PANEL_VIEW).await {
        Ok(account) => account,
        Err(resp) => return resp,
    };
    match build_status(&state, &account).await {
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
    let account = match authorized_token(&state, &q.session, perm::PANEL_VIEW).await {
        Ok(account) => account,
        Err(resp) => return resp,
    };
    ws.on_upgrade(move |socket| stream_status(socket, state, account))
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

async fn stream_status(mut socket: WebSocket, state: PanelState, account: String) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    // Subscribing here, not earlier, is deliberate: a `broadcast` receiver only ever sees frames
    // sent *after* it subscribes, so this socket's console tab starts from "now" rather than
    // trying to catch up on everything since the process started.
    let mut console_rx = crate::term::console_feed().subscribe();
    loop {
        let message = tokio::select! {
            _ = interval.tick() => {
                match build_status(&state, &account).await {
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
    muted: bool,
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
            muted: p.muted,
        }
    }
}

async fn players(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::PANEL_VIEW).await {
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
    let actor = match authorized(&state, &headers, perm::SERVER_KICK).await {
        Ok(actor) => actor,
        Err(resp) => return resp,
    };
    match ask(&state, |reply| ServerEvent::PanelKick {
        actor,
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
    let actor = match authorized(&state, &headers, perm::SERVER_BAN).await {
        Ok(actor) => actor,
        Err(resp) => return resp,
    };
    let Some(kind) = BanKind::parse(&req.kind) else {
        return err(StatusCode::BAD_REQUEST, "kind must be name, ip or uuid");
    };
    match ask(&state, |reply| ServerEvent::PanelBan {
        actor,
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
    let actor = match authorized(&state, &headers, perm::SERVER_UNBAN).await {
        Ok(actor) => actor,
        Err(resp) => return resp,
    };
    match ask(&state, |reply| ServerEvent::PanelUnban {
        actor,
        value: req.value,
        reply,
    })
    .await
    {
        Ok(removed) => Json(UnbanResponse { removed }).into_response(),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
struct MuteRequest {
    name: String,
    #[serde(default)]
    reason: String,
    /// Seconds, or absent/`null` for a permanent mute. A plain number rather than the console's
    /// `10m`/`2h` duration words, since this is a form field, not a chat line to parse.
    #[serde(default)]
    duration_secs: Option<u64>,
}

async fn mute_player(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<MuteRequest>,
) -> Response {
    let actor = match authorized(&state, &headers, perm::SERVER_MUTE).await {
        Ok(actor) => actor,
        Err(resp) => return resp,
    };
    match ask(&state, |reply| ServerEvent::PanelMute {
        actor,
        name: req.name,
        reason: req.reason,
        duration_secs: req.duration_secs,
        reply,
    })
    .await
    {
        Ok(()) => StatusCode::OK.into_response(),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
struct UnmuteRequest {
    name: String,
}

async fn unmute_player(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<UnmuteRequest>,
) -> Response {
    let actor = match authorized(&state, &headers, perm::SERVER_UNMUTE).await {
        Ok(actor) => actor,
        Err(resp) => return resp,
    };
    match ask(&state, |reply| ServerEvent::PanelUnmute {
        actor,
        name: req.name,
        reply,
    })
    .await
    {
        Ok(removed) => Json(ChangedResponse { changed: removed }).into_response(),
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
    if let Err(resp) = authorized(&state, &headers, perm::PANEL_VIEW).await {
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
    if let Err(resp) = authorized(&state, &headers, perm::SERVER_WHITELIST).await {
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
    if let Err(resp) = authorized(&state, &headers, perm::SERVER_WHITELIST).await {
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
    if let Err(resp) = authorized(&state, &headers, perm::PANEL_VIEW).await {
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
    if let Err(resp) = authorized(&state, &headers, perm::WORLD_SWITCH).await {
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
    if let Err(resp) = authorized(&state, &headers, perm::PANEL_VIEW).await {
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
    if let Err(resp) = authorized(&state, &headers, perm::WORLD_MOTD).await {
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
    if let Err(resp) = authorized(&state, &headers, perm::PANEL_CONSOLE).await {
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
    if let Err(resp) = authorized(&state, &headers, perm::PANEL_CONSOLE).await {
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
    if let Err(resp) = authorized_token(&state, &q.session, perm::PANEL_VIEW).await {
        return resp;
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

// ---- metrics --------------------------------------------------------------------------------

/// The process's current resident set size in bytes, best-effort and platform-specific. `None`
/// where the platform will not report it — the panel's memory graph shows a gap rather than a
/// wrong number. No new dependency: `libc` is already a workspace crate, and this is the one small
/// piece of live state the game task does not (and should not) track for the panel.
///
/// The `unsafe` here is the same shape `game::clock` already carries an allow for: a single
/// read-only libc call filling a stack-allocated record, with the pointer and size the platform's
/// own API dictates. Nothing is retained past the call and no invariant of ours rides on it.
#[allow(unsafe_code)]
fn process_rss_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        // Field two of `/proc/self/statm` is the resident page count.
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let resident_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page <= 0 {
            return None;
        }
        Some(resident_pages * page as u64)
    }
    #[cfg(target_os = "macos")]
    {
        // `proc_pid_rusage`'s v0 record carries `ri_resident_size`, the current resident size in
        // bytes — the honest "how much memory is this process using right now" number, unlike
        // `getrusage`'s `ru_maxrss`, which is a high-water mark that never falls.
        let mut info = std::mem::MaybeUninit::<libc::rusage_info_v0>::zeroed();
        let ret = unsafe {
            libc::proc_pid_rusage(
                std::process::id() as libc::c_int,
                libc::RUSAGE_INFO_V0,
                (&mut info as *mut std::mem::MaybeUninit<libc::rusage_info_v0>)
                    .cast::<libc::rusage_info_t>(),
            )
        };
        if ret != 0 {
            return None;
        }
        // A zero return means the kernel filled the whole record.
        let info = unsafe { info.assume_init() };
        Some(info.ri_resident_size)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

#[derive(Serialize)]
struct PhaseCost {
    name: &'static str,
    us: u64,
}

#[derive(Serialize)]
struct MetricsResponse {
    /// The per-tick budget, in microseconds, so the panel can draw the line a tick is measured
    /// against (16,667 µs — sixty ticks a second).
    budget_us: u64,
    /// The most recent tick's processor cost, and how long it took in wall time.
    cpu_us: u64,
    wall_us: u64,
    /// The worst processor cost seen this reporting window.
    worst_cpu_us: u64,
    phases: Vec<PhaseCost>,
    player_count: usize,
    npc_count: usize,
    projectile_count: usize,
    item_count: usize,
    ticks: u64,
    /// Resident set size in bytes, or `null` where the platform will not report it.
    memory_bytes: Option<u64>,
}

impl MetricsResponse {
    fn build(m: PanelMetrics, memory_bytes: Option<u64>) -> Self {
        Self {
            budget_us: m.budget_us,
            cpu_us: m.last_cpu_us,
            wall_us: m.last_wall_us,
            worst_cpu_us: m.worst_cpu_us,
            phases: m
                .phases
                .into_iter()
                .map(|(name, us)| PhaseCost { name, us })
                .collect(),
            player_count: m.player_count,
            npc_count: m.npc_count,
            projectile_count: m.projectile_count,
            item_count: m.item_count,
            ticks: m.ticks,
            memory_bytes,
        }
    }
}

async fn metrics(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::PANEL_VIEW).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelMetrics { reply }).await {
        Ok(m) => Json(MetricsResponse::build(m, process_rss_bytes())).into_response(),
        Err(resp) => resp,
    }
}

// ---- backups & rollback ---------------------------------------------------------------------

#[derive(Serialize)]
struct BackupEntryResponse {
    index: usize,
    size_mb: f64,
    age_secs: Option<u64>,
}

impl From<PanelBackupEntry> for BackupEntryResponse {
    fn from(b: PanelBackupEntry) -> Self {
        Self {
            index: b.index,
            size_mb: b.size_bytes as f64 / 1_048_576.0,
            age_secs: b.age_secs,
        }
    }
}

#[derive(Serialize)]
struct BackupsResponse {
    saving: bool,
    world_file: Option<String>,
    kept: usize,
    backups: Vec<BackupEntryResponse>,
}

impl From<PanelBackups> for BackupsResponse {
    fn from(b: PanelBackups) -> Self {
        Self {
            saving: b.saving,
            world_file: b.world_file,
            kept: b.kept,
            backups: b
                .backups
                .into_iter()
                .map(BackupEntryResponse::from)
                .collect(),
        }
    }
}

async fn backups(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::PANEL_VIEW).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelBackups { reply }).await {
        Ok(b) => Json(BackupsResponse::from(b)).into_response(),
        Err(resp) => resp,
    }
}

async fn force_save(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::WORLD_SAVE).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelForceSave { reply }).await {
        Ok(Ok(())) => StatusCode::OK.into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_REQUEST, e),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
struct RollbackRequest {
    which: usize,
}

#[derive(Serialize)]
struct RollbackResponse {
    message: String,
}

async fn rollback(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<RollbackRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::WORLD_ROLLBACK).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelRollback {
        which: req.which,
        reply,
    })
    .await
    {
        Ok(Ok(message)) => Json(RollbackResponse { message }).into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_REQUEST, e),
        Err(resp) => resp,
    }
}

// ---- groups & accounts admin ----------------------------------------------------------------

#[derive(Serialize)]
struct GroupResponse {
    name: String,
    permissions: Vec<String>,
    can_admin: bool,
}

impl From<PanelGroupInfo> for GroupResponse {
    fn from(g: PanelGroupInfo) -> Self {
        Self {
            name: g.name,
            permissions: g.permissions,
            can_admin: g.can_admin,
        }
    }
}

#[derive(Serialize)]
struct AccountResponse {
    name: String,
    group: String,
    can_admin: bool,
}

impl From<PanelAccountInfo> for AccountResponse {
    fn from(a: PanelAccountInfo) -> Self {
        Self {
            name: a.name,
            group: a.group,
            can_admin: a.can_admin,
        }
    }
}

#[derive(Serialize)]
struct AccountsResponse {
    groups: Vec<GroupResponse>,
    accounts: Vec<AccountResponse>,
}

async fn accounts(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::ADMIN_ACCOUNTS).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelAccounts { reply }).await {
        Ok(a) => Json(AccountsResponse {
            groups: a.groups.into_iter().map(GroupResponse::from).collect(),
            accounts: a.accounts.into_iter().map(AccountResponse::from).collect(),
        })
        .into_response(),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
struct SetGroupRequest {
    name: String,
    group: String,
}

async fn set_account_group(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SetGroupRequest>,
) -> Response {
    let actor = match authorized(&state, &headers, perm::ADMIN_ACCOUNTS).await {
        Ok(actor) => actor,
        Err(resp) => return resp,
    };
    match ask(&state, |reply| ServerEvent::PanelSetAccountGroup {
        actor,
        name: req.name,
        group: req.group,
        reply,
    })
    .await
    {
        Ok(Ok(())) => StatusCode::OK.into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_REQUEST, e),
        Err(resp) => resp,
    }
}

// `password` is never logged below, at any level: see `admin::mod`'s own "never logged"
// convention, and `LoginRequest`'s own doc comment for why this deliberately has no `Debug` either.
#[derive(Deserialize)]
struct CreateAccountRequest {
    name: String,
    password: String,
    group: String,
}

async fn create_account(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateAccountRequest>,
) -> Response {
    let actor = match authorized(&state, &headers, perm::ADMIN_ACCOUNTS).await {
        Ok(actor) => actor,
        Err(resp) => return resp,
    };
    if req.name.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "an account needs a name");
    }
    // Same length rule the claim path and `Admin::register` both enforce.
    if req.password.len() < 6 {
        return err(
            StatusCode::BAD_REQUEST,
            "that password is too short; use at least six characters",
        );
    }
    // Hash off the game task, exactly as the login/claim path does — argon2 must never run inline.
    let name = req.name.clone();
    let password = req.password.clone();
    let group = req.group.clone();
    let account =
        match tokio::task::spawn_blocking(move || Account::new(&name, &password, &group)).await {
            Ok(Ok(account)) => account,
            Ok(Err(e)) => return err(StatusCode::BAD_REQUEST, e),
            Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "hashing task panicked"),
        };
    match ask(&state, |reply| ServerEvent::PanelCreateAccount {
        actor,
        account,
        reply,
    })
    .await
    {
        Ok(Ok(())) => StatusCode::OK.into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_REQUEST, e),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
struct DeleteAccountRequest {
    name: String,
}

async fn delete_account(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<DeleteAccountRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::ADMIN_ACCOUNTS).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelDeleteAccount {
        name: req.name,
        reply,
    })
    .await
    {
        Ok(Ok(())) => StatusCode::OK.into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_REQUEST, e),
        Err(resp) => resp,
    }
}

// ---- group permission editing ----------------------------------------------------------------

/// Every known permission name, for the group editor's picker. A plain function call rather than a
/// `ServerEvent` round trip: the vocabulary registry (`admin::group::known`) is process-global, not
/// game state, so there is nothing on the game task worth asking.
async fn known_permissions(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::ADMIN_GROUPS).await {
        return resp;
    }
    Json(crate::admin::group::known()).into_response()
}

#[derive(Deserialize)]
struct SetGroupPermissionRequest {
    group: String,
    permission: String,
    grant: bool,
}

/// Add or remove one permission on a group. `actor` (the signed-in account) must already hold the
/// permission being granted — enforced on the game task, in
/// `GameServer::panel_set_group_permission` — so the editor cannot be used to hand out power nobody
/// making the change actually has.
async fn set_group_permission(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SetGroupPermissionRequest>,
) -> Response {
    let actor = match authorized(&state, &headers, perm::ADMIN_GROUPS).await {
        Ok(actor) => actor,
        Err(resp) => return resp,
    };
    match ask(&state, |reply| ServerEvent::PanelSetGroupPermission {
        actor,
        group: req.group,
        permission: req.permission,
        grant: req.grant,
        reply,
    })
    .await
    {
        Ok(Ok(())) => StatusCode::OK.into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_REQUEST, e),
        Err(resp) => resp,
    }
}

// ---- audit log -----------------------------------------------------------------------------

#[derive(Serialize)]
struct AuditEntryResponse {
    when: u64,
    issuer: String,
    action: String,
    target: String,
    detail: String,
}

impl From<crate::admin::audit::AuditEvent> for AuditEntryResponse {
    fn from(e: crate::admin::audit::AuditEvent) -> Self {
        Self {
            when: e.when,
            issuer: e.issuer,
            action: e.action.as_str().to_string(),
            target: e.target,
            detail: e.detail,
        }
    }
}

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default = "default_audit_n")]
    n: usize,
}

fn default_audit_n() -> usize {
    50
}

async fn audit_log(
    State(state): State<PanelState>,
    Query(q): Query<AuditQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::ADMIN_AUDIT).await {
        return resp;
    }
    match ask(&state, |reply| ServerEvent::PanelAuditTail {
        n: q.n,
        reply,
    })
    .await
    {
        Ok(events) => Json(
            events
                .into_iter()
                .map(AuditEntryResponse::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(resp) => resp,
    }
}

// ---- world management: generate a brand-new world -------------------------------------------

#[derive(Deserialize)]
struct NewWorldRequest {
    name: String,
    width: i32,
    height: i32,
    /// Optional seed text — a plain number reproduces that numeric seed, free text is hashed into
    /// one, and either is checked against vanilla's secret-seed strings (see
    /// `worldgen::generate_from_text`). Empty or absent means a fresh random seed.
    #[serde(default)]
    seed: Option<String>,
}

#[derive(Serialize)]
struct NewWorldStatusResponse {
    status: &'static str,
    running: bool,
    name: String,
    world_file: Option<String>,
    message: String,
    elapsed_secs: u64,
}

/// A world must be a whole number of sections and within the client's addressable range — the same
/// rules `Config::validate` applies, checked here so a bad size is refused before a slow generation
/// is started rather than after. Returns the reason as a plain string; the caller turns it into a
/// `400` (a `Result<(), Response>` here would carry axum's large `Response` in its `Err`, which
/// `clippy::result_large_err` rightly flags).
fn validate_world_size(width: i32, height: i32) -> Result<(), String> {
    use terrustia_proto::section::{SECTION_HEIGHT, SECTION_WIDTH};
    if width < 400 || height < 300 {
        return Err("a world must be at least 400 x 300".into());
    }
    if width > i32::from(i16::MAX) || height > i32::from(i16::MAX) {
        return Err(format!("a world may be at most {0} x {0} tiles", i16::MAX));
    }
    if width % SECTION_WIDTH != 0 || height % SECTION_HEIGHT != 0 {
        return Err(format!(
            "world size must be a whole number of {SECTION_WIDTH}x{SECTION_HEIGHT} sections"
        ));
    }
    Ok(())
}

fn snapshot_worldgen(state: &PanelState) -> NewWorldStatusResponse {
    let job = state.worldgen();
    let elapsed_secs = if job.status == GenStatus::Running {
        job.started.map_or(0, |s| s.elapsed().as_secs())
    } else {
        job.elapsed_secs.unwrap_or(0)
    };
    NewWorldStatusResponse {
        status: job.status.as_str(),
        running: job.status == GenStatus::Running,
        name: job.name.clone(),
        world_file: job.world_file.clone(),
        message: job.message.clone(),
        elapsed_secs,
    }
}

async fn new_world_status(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::PANEL_VIEW).await {
        return resp;
    }
    Json(snapshot_worldgen(&state)).into_response()
}

async fn new_world(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<NewWorldRequest>,
) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::WORLD_NEW).await {
        return resp;
    }
    // One at a time. Worldgen is slow and memory-hungry; two at once on the blocking pool is a way
    // to run a loopback machine out of RAM, not a feature.
    if state.worldgen().status == GenStatus::Running {
        return err(
            StatusCode::CONFLICT,
            "a world is already being generated; wait for it to finish",
        );
    }
    if let Err(reason) = validate_world_size(req.width, req.height) {
        return err(StatusCode::BAD_REQUEST, reason);
    }
    // Resolve and validate the destination the same way `--new` does: a plain world name, landed in
    // the server's own worlds/ directory, never a path the request body could smuggle in.
    let path = match crate::worlds::new_world_path(&req.name) {
        Ok(path) => path,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    if path.exists() {
        return err(
            StatusCode::BAD_REQUEST,
            format!(
                "a world called {} already exists on disk; pick another name",
                req.name
            ),
        );
    }
    // Make sure worlds/ exists before the generated world is saved into it.
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "cannot create the world directory {}: {e}",
                parent.display()
            ),
        );
    }

    {
        let mut job = state.worldgen();
        *job = WorldGenJob {
            status: GenStatus::Running,
            name: req.name.clone(),
            started: Some(Instant::now()),
            ..WorldGenJob::default()
        };
    }

    let handle = Arc::clone(&state.worldgen);
    let (name, width, height) = (req.name.clone(), req.width, req.height);
    let seed = req.seed.clone();
    // A real background thread off the request: `worldgen::generate` is a single blocking call, so
    // it runs on the blocking pool and reports back through the shared job cell. It deliberately
    // outlives the request that started it (and even a panel toggled off mid-gen — the `Arc` keeps
    // the cell alive until the thread finishes), so the operator can watch it via `new_world_status`.
    tokio::task::spawn_blocking(move || {
        let began = Instant::now();
        let world = match seed {
            Some(text) if !text.trim().is_empty() => {
                crate::world::worldgen::generate_from_text(width, height, name.clone(), &text)
            }
            _ => {
                use argon2::password_hash::rand_core::{OsRng, RngCore};
                let mut bytes = [0u8; 8];
                OsRng.fill_bytes(&mut bytes);
                crate::world::worldgen::generate(
                    width,
                    height,
                    name.clone(),
                    u64::from_le_bytes(bytes),
                )
            }
        };
        let result = crate::world::wld_save::save(&world, &path);
        let mut job = handle.lock().unwrap_or_else(|p| p.into_inner());
        job.elapsed_secs = Some(began.elapsed().as_secs());
        match result {
            Ok(()) => {
                job.status = GenStatus::Done;
                job.world_file = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string);
                job.message = format!(
                    "generated {name} ({width} x {height}) in {}s — switch to it from the worlds tab",
                    began.elapsed().as_secs()
                );
                info!(world = %name, "new world generated from the web panel");
            }
            Err(e) => {
                job.status = GenStatus::Failed;
                job.message = format!("could not save the generated world: {e}");
                warn!(world = %name, error = %e, "web panel world generation failed");
            }
        }
    });

    (StatusCode::ACCEPTED, Json(snapshot_worldgen(&state))).into_response()
}
