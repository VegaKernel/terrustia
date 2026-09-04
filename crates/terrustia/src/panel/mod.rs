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
//! `crates/terrustia/web-panel/src/lib/WorldView.svelte` for the rendering side, and
//! [`worlds::world_ws_upgrade`] for the data it draws from  -  real positions and appearance data,
//! real tile types, never a composited Terraria sprite).
//!
//! **Layout**: this coordinator module owns startup/shutdown ([`run`], [`supervise`],
//! [`PanelHandle`]), shared plumbing every route needs ([`PanelState`], [`ask`], [`err`],
//! [`auth_lookup`], [`send_ws`]), and static asset serving. Each resource has its own sibling
//! module  -  [`auth`] (sessions, login, logout), [`status`] (the status/console socket),
//! [`players`] (players, kick/ban/mute, the whitelist), [`worlds`] (list/switch/generate, the
//! world-view socket), [`settings`] (config, motd, console/chat send, metrics, backups/rollback)
//! and [`accounts`] (accounts, groups, the permission editor, the audit log)  -  each exposing a
//! `router()` this module merges into one. This mirrors `game::server`'s own split into
//! `mod.rs`/`console.rs`/`dispatch.rs`/`panel.rs`/`systems.rs`/`tick.rs`: one coordinator, sibling
//! files by resource.
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
//! written down, and every handler below reuses it) and the two expensive operations run in a
//! `spawn_blocking` on the panel's own task: argon2, the same discipline
//! `admin::store::Admin::account_hash`'s doc comment requires for player logins, and the audit
//! log's whole-file read ([`accounts::audit_log`], which asks the game task only where the file
//! is).
//!
//! **Every endpoint below `/api/` other than `/api/unclaimed`, `/api/login` and `/api/logout`
//! requires a valid session and its own permission**, checked by [`auth::authorized`] (or
//! [`auth::authorized_token`] for the two WebSocket routes, which carry the session as a `session`
//! query parameter instead of a bearer header  -  a browser cannot attach a custom header to a
//! WebSocket upgrade): a token looked up in [`PanelState::sessions`] to find the signed-in
//! account, then a fresh, per-request check  -  against the game task's live `Admin` store, not
//! anything cached at login  -  that the account's group grants the specific permission that route
//! needs. `login` itself only requires `panel.view` to issue a session at all; a `default` account
//! (which does not hold it) cannot get a session, and a `moderator` one can sign in but still gets
//! `403` from, say, `/api/console`. A session also idles out on its own
//! ([`auth::SESSION_IDLE_TIMEOUT`]), and `/api/logout` ([`auth::router`]) revokes one outright  -
//! see `auth`'s own module doc for both.
//!
//! The two long-lived WebSockets (`status::stream_status`, `worlds::stream_world`) re-run this
//! same check on a timer rather than only once at connect, for the same reason every ordinary
//! route does: a session that expires, is signed out, or has its permissions changed mid-connection
//! must not go on serving a live feed to a credential that would now be refused.
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
//! **Sessions** are an in-memory map owned by [`auth`], not the account store: a session is a
//! panel-HTTP concern, not core game state, and doesn't need to survive a panel restart.

mod accounts;
mod auth;
mod players;
mod settings;
mod status;
mod worlds;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::config::Config;
use crate::game::server::{PanelAuthLookup, ServerEvent};

#[cfg(feature = "embed-web")]
#[derive(rust_embed::RustEmbed)]
#[folder = "web-panel/dist/"]
struct Assets;

/// Whether a request path may be resolved against the panel's asset directory at all.
///
/// Every component has to be an ordinary name: that rejects `..`, a leading root (`/etc/passwd`,
/// which `Path::join` does not append but *replaces the whole path with*) and a Windows drive
/// prefix, on the platform's own path-parsing rules rather than on a guess about separators.
///
/// The rule this replaces was `path.split('/').any(|s| s == "..")`, which is only correct where `/`
/// is the only separator. On Windows `Path::join` treats `\` as one too, so `..\..\windows\win.ini`
/// is a single segment to `split('/')`, passes that check, and traverses. `worlds.rs`'s
/// `new_world_path` already guards on the same principle.
fn is_safe_asset_path(path: &str) -> bool {
    std::path::Path::new(path)
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn load_static_asset(path: &str) -> Option<Vec<u8>> {
    // `path` comes straight from the request URI (`static_handler`), and `Uri::path()` does not
    // reject a literal `..` segment the way a browser's own address bar would. Only the
    // disk-serving branch below (a local frontend-development convenience, off by default) touches
    // the filesystem with it, but a dev build listening on localhost is still a real process on a
    // real machine. The check sits ahead of both branches rather than inside that one, so there is
    // a single place to get it right and no build where it is missing.
    if !is_safe_asset_path(path) {
        return None;
    }
    #[cfg(feature = "embed-web")]
    {
        Assets::get(path).map(|f| f.data.into_owned())
    }
    #[cfg(not(feature = "embed-web"))]
    {
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
    /// session token -> the session it belongs to. See `auth`'s own module doc: in-memory only (a
    /// panel restart signs everyone out), idles out on its own, and `/api/logout` can revoke one
    /// outright.
    sessions: Arc<Mutex<HashMap<String, auth::Session>>>,
    started: Instant,
    /// The one background world-generation job, if any has been started this panel lifetime.
    /// Worldgen is slow (seconds, and a lot of memory) and pure — it never touches the game task —
    /// so it runs on its own `spawn_blocking` thread and reports progress through this shared cell
    /// rather than blocking the request that kicked it off. Only one at a time.
    worldgen: worlds::WorldGenCell,
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

impl PanelState {
    /// The session map is a plain in-memory cache the panel can always reconstruct by asking
    /// people to sign in again — a poisoned lock (some other request panicked mid-access) is not
    /// worth losing every other session over, so this recovers the data rather than panicking.
    fn sessions(&self) -> std::sync::MutexGuard<'_, HashMap<String, auth::Session>> {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The world-generation job cell, recovering a poisoned lock the same way [`Self::sessions`]
    /// does and for the same reason — a background gen thread that panicked mid-write should not
    /// take out every future status read.
    fn worldgen(&self) -> std::sync::MutexGuard<'_, worlds::WorldGenJob> {
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
    let listener = crate::net::listener::bind(
        config.panel_listen,
        "panel_listen in the config file, or TERRUSTIA_PANEL_LISTEN",
    )
    .await?;
    let addr = listener.local_addr().unwrap_or(config.panel_listen);
    let state = PanelState {
        events,
        sessions: Arc::new(Mutex::new(HashMap::new())),
        started: Instant::now(),
        worldgen: Arc::new(Mutex::new(worlds::WorldGenJob::default())),
        ip_throttle: Arc::new(Mutex::new(crate::admin::Throttle::new())),
        account_throttle: Arc::new(Mutex::new(crate::admin::Throttle::new())),
    };
    let router = Router::new()
        .merge(auth::router())
        .merge(status::router())
        .merge(players::router())
        .merge(worlds::router())
        .merge(settings::router())
        .merge(accounts::router())
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
/// `live` mirrors whether the panel is actually up, for the one caller that cannot ask: a world
/// switch `exec`s a replacement process (`main`'s `relaunch_into`) and has to pass `--panel` if the
/// panel should survive it. `config.panel_enabled` is the boot-time answer and goes stale the first
/// time somebody types `panel` at the console, so it is written here, where the truth changes.
/// Note it is only set on a *successful* start: a toggle that fails to bind leaves the panel down,
/// and the flag has to say so or the restart would try to bring back something that is not running.
pub async fn supervise(
    config: Config,
    events: mpsc::Sender<ServerEvent>,
    mut toggle: mpsc::UnboundedReceiver<()>,
    initial: Option<tokio::task::JoinHandle<()>>,
    live: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;
    let mut handle = PanelHandle(initial);
    while toggle.recv().await.is_some() {
        match handle.take() {
            Some(running) => {
                running.abort();
                live.store(false, Ordering::Relaxed);
                info!("web panel stopped (console toggle)");
            }
            None => match run(config.clone(), events.clone()).await {
                Ok(started) => {
                    handle = PanelHandle(Some(started));
                    live.store(true, Ordering::Relaxed);
                }
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

// ---- shared plumbing -------------------------------------------------------------------------

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

/// `changed`/`removed`-shaped responses several small mutating routes share (whitelist add/remove,
/// unmute).
#[derive(Serialize)]
struct ChangedResponse {
    changed: bool,
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

/// Serialize and send one message, reporting whether the socket is still usable. A message that
/// fails to serialize is dropped rather than treated as a dead socket — the connection itself is
/// fine, only that one frame was not worth sending. Shared by both long-lived sockets
/// (`status::stream_status`, `worlds::stream_world`).
async fn send_ws<T: Serialize>(socket: &mut WebSocket, message: &T) -> bool {
    let Ok(payload) = serde_json::to_string(message) else {
        return true;
    };
    socket.send(Message::Text(payload.into())).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::is_safe_asset_path;

    /// The traversal guard on the asset path, which is the only thing standing between a request
    /// URI and `fs::read` in a `--no-default-features` (disk-serving) build.
    #[test]
    fn the_asset_path_guard_rejects_everything_that_is_not_an_ordinary_name() {
        // What the panel actually serves.
        assert!(is_safe_asset_path("index.html"));
        assert!(is_safe_asset_path("assets/index-DEADBEEF.js"));
        assert!(is_safe_asset_path("favicon.svg"));

        // Ordinary traversal.
        assert!(!is_safe_asset_path("../etc/passwd"));
        assert!(!is_safe_asset_path("assets/../../etc/passwd"));

        // An absolute path is the case the old `split('/')` rule let through on every platform:
        // no segment equals "..", so it passed, and `Path::join` replaces rather than appends when
        // the argument is absolute, so the read went straight to the named file.
        assert!(!is_safe_asset_path("/etc/passwd"));

        // Percent-encoding is not decoded anywhere on this path, so this stays a literal name and
        // is safe either way. Asserted so that a future decode step has to look at this test.
        assert!(is_safe_asset_path("%2e%2e/etc/passwd"));

        // The Windows case the old rule missed: `\` is a separator to `Path::join` there but never
        // to `split('/')`. It is one ordinary (if odd) file name on unix, so this can only be
        // asserted where it is actually a traversal.
        #[cfg(windows)]
        assert!(!is_safe_asset_path("..\\..\\windows\\win.ini"));
    }
}
