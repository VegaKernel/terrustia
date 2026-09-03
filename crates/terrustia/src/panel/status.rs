//! `/api/status` and the status/console WebSocket (`/api/ws`).
//!
//! The socket carries two kinds of frame down one connection: a status refresh every two seconds,
//! and (for a session holding `panel.console`) every line the sticky console's own feed sees —
//! log lines, command replies, and in-game chat. Both permissions are re-checked on every tick
//! (see [`stream_status`]'s doc comment), not just once at connect: a session's own group can
//! change mid-connection, through this very panel, and a socket opened before that change must
//! not go on behaving as if it never happened.

use std::time::Duration;

use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::admin::perm;
use crate::game::server::PanelStatus;
use crate::term::{ConsoleLine, ConsoleLineKind};

use super::auth::{authorized, authorized_token, has_permission};
use super::{PanelState, ServerEvent, ask, auth_lookup};

pub(super) fn router() -> Router<PanelState> {
    use axum::routing::get;
    Router::new()
        .route("/api/status", get(status))
        .route("/api/ws", get(ws_upgrade))
}

#[derive(Serialize)]
pub(super) struct StatusResponse {
    uptime_secs: u64,
    player_count: usize,
    max_players: usize,
    world_name: String,
    world_file: Option<String>,
    version: &'static str,
    unclaimed: bool,
    /// The calling account's own permission strings — a UX convenience so the frontend can choose
    /// which tabs and buttons to show; every route still re-checks its own permission server-side
    /// regardless of what this says. See [`super::PanelAuthLookup::permissions`].
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
pub(super) async fn build_status(
    state: &PanelState,
    account: &str,
) -> Result<StatusResponse, Response> {
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

#[derive(Deserialize)]
pub(super) struct WsQuery {
    pub(super) session: String,
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
    ws.on_upgrade(move |socket| stream_status(socket, state, q.session, account))
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

/// `token` (not a bare account name) is what this socket carries across ticks, so each one can
/// re-run the *whole* session check — not just re-ask whether the account still holds
/// `panel.console`, which is all this used to do. A session that expires (idle timeout, see
/// `auth::SESSION_IDLE_TIMEOUT`), is signed out through `/api/logout`, or belongs to an account
/// that gets deleted mid-connection now has its socket closed on the next tick rather than
/// continuing to serve a live status feed — and, for a `panel.console` holder, the entire server
/// log — to a credential that is no longer good for anything else in the panel.
async fn stream_status(mut socket: WebSocket, state: PanelState, token: String, account: String) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    // Subscribing here, not earlier, is deliberate: a `broadcast` receiver only ever sees frames
    // sent *after* it subscribes, so this socket's console tab starts from "now" rather than
    // trying to catch up on everything since the process started.
    let mut console_rx = crate::term::console_feed().subscribe();
    let mut may_read_console = has_permission(&state, &account, perm::PANEL_CONSOLE).await;
    loop {
        let message = tokio::select! {
            _ = interval.tick() => {
                let account = match authorized_token(&state, &token, perm::PANEL_VIEW).await {
                    Ok(account) => account,
                    // The session is gone, expired, or no longer holds `panel.view` — close the
                    // socket rather than keep serving a feed a fresh request would now refuse.
                    Err(_) => break,
                };
                // `panel.view` is enough to *open* this socket, but the console feed carries the
                // entire server log, and `panel.console` is the permission for that. It used to
                // gate writing only, so a moderator holding just `panel.view` received every log
                // line and was merely not shown a tab to read them in, which is not a gate at
                // all. Re-evaluated on every tick alongside the session check above, so granting
                // or revoking it mid-session takes effect within one refresh.
                may_read_console = has_permission(&state, &account, perm::PANEL_CONSOLE).await;
                match build_status(&state, &account).await {
                    Ok(s) => WsMessage::Status(s),
                    Err(_) => break,
                }
            }
            line = console_rx.recv() => {
                match line {
                    Ok(_) if !may_read_console => continue,
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
        if !super::send_ws(&mut socket, &message).await {
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
