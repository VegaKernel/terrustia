//! Read-only config, the motd, console/chat send, live metrics, and backups/rollback.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::admin::perm;
use crate::game::server::{
    PanelBackupEntry, PanelBackups, PanelConfigSnapshot, PanelMetrics, ServerEvent,
};

use super::auth::authorized;
use super::{PanelState, ask, err};

pub(super) fn router() -> Router<PanelState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/config", get(config_snapshot))
        .route("/api/config/motd", post(set_motd))
        .route("/api/console", post(send_console))
        .route("/api/chat", post(send_chat))
        .route("/api/metrics", get(metrics))
        .route("/api/backups", get(backups))
        .route("/api/save", post(force_save))
        .route("/api/rollback", post(rollback))
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
/// console (see this module's parent doc comment), so this is not a permission boundary — it
/// exists so a chat message or command containing an embedded newline cannot smuggle a second
/// command in past whatever the operator thought they were sending.
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
