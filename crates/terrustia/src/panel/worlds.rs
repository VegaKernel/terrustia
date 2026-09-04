//! Listing and switching worlds, generating a brand-new one in the background, and the live
//! world-view WebSocket (`/api/ws/world`) that draws from it.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::admin::perm;
use crate::game::server::{ServerEvent, TileColor};

use super::auth::authorized_token;
use super::players::PlayerResponse;
use super::status::WsQuery;
use super::{PanelState, ask, err};

pub(super) fn router() -> Router<PanelState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/worlds", get(worlds))
        .route("/api/worlds/switch", post(switch_world))
        .route("/api/worlds/new", post(new_world))
        .route("/api/worlds/new/status", get(new_world_status))
        .route("/api/ws/world", get(world_ws_upgrade))
}

/// Where a background world generation has got to. Coarse on purpose: `worldgen::generate` is a
/// single blocking call with no progress callback, so there is no honest percentage to report  -
/// only which of these states it is in, and how long it has been running.
#[derive(Clone, Default)]
pub(super) struct WorldGenJob {
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

#[derive(Serialize)]
struct WorldEntry {
    name: String,
    size_mb: f64,
    /// Whether this is the world the running process currently has open.
    current: bool,
}

async fn worlds(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    if let Err(resp) = super::auth::authorized(&state, &headers, perm::PANEL_VIEW).await {
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
    if let Err(resp) = super::auth::authorized(&state, &headers, perm::WORLD_SWITCH).await {
        return resp;
    }
    // The panel never accepts a raw path from the client  -  only a name matched against what
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

// ---- the live world view --------------------------------------------------------------------

async fn world_ws_upgrade(
    State(state): State<PanelState>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(resp) = authorized_token(&state, &q.session, perm::PANEL_VIEW).await {
        return resp;
    }
    ws.on_upgrade(move |socket| stream_world(socket, state, q.session))
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

/// Players refresh often enough to look live (twice a second); tiles are sampled far less often  -
/// the world's shape barely changes tick to tick, and even the bounded sample in
/// `GameServer::world_tile_sample` is not worth recomputing ten times as often as anyone could see
/// a difference. `tokio::time::interval` fires its first tick immediately, so both kinds of frame
/// reach a freshly connected client right away rather than after their first full period.
///
/// `token` carries the session across ticks the same way `status::stream_status` does, and for
/// the same reason: this socket used to check nothing at all after the initial upgrade, so a
/// session that expired, was signed out, or had its account deleted mid-connection kept receiving
/// every player's live position, health and IP indefinitely. A permission check now rides the
/// player-refresh tick, the more frequent of the two.
async fn stream_world(mut socket: WebSocket, state: PanelState, token: String) {
    let mut player_interval = tokio::time::interval(Duration::from_millis(500));
    let mut tile_interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        let message = tokio::select! {
            _ = player_interval.tick() => {
                if authorized_token(&state, &token, perm::PANEL_VIEW).await.is_err() {
                    break;
                }
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
        if !super::send_ws(&mut socket, &message).await {
            break;
        }
    }
}

// ---- world creation --------------------------------------------------------------------------

#[derive(Deserialize)]
struct NewWorldRequest {
    name: String,
    width: i32,
    height: i32,
    /// Optional seed text  -  a plain number reproduces that numeric seed, free text is hashed into
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

/// A world must be a whole number of sections and within the client's addressable range  -  the same
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
    if let Err(resp) = super::auth::authorized(&state, &headers, perm::PANEL_VIEW).await {
        return resp;
    }
    Json(snapshot_worldgen(&state)).into_response()
}

async fn new_world(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<NewWorldRequest>,
) -> Response {
    if let Err(resp) = super::auth::authorized(&state, &headers, perm::WORLD_NEW).await {
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
    // outlives the request that started it (and even a panel toggled off mid-gen  -  the `Arc` keeps
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
                    "generated {name} ({width} x {height}) in {}s  -  switch to it from the worlds tab",
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

/// The job cell type `PanelState::worldgen` carries, and its accessor's lock-recovery behaviour  -
/// see [`super::PanelState::worldgen`].
pub(super) type WorldGenCell = Arc<Mutex<WorldGenJob>>;
