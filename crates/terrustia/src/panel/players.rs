//! Player list, kick, ban, unban, mute, unmute, and the whitelist. See this module's parent doc
//! comment for the permission each route needs (`server.kick`, `server.ban`, and so on).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::admin::{BanKind, perm};
use crate::game::server::{PanelPlayer, PanelWhitelist, ServerEvent};

use super::auth::authorized;
use super::{ChangedResponse, PanelState, ask, err};

pub(super) fn router() -> Router<PanelState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/players", get(players))
        .route("/api/players/kick", post(kick_player))
        .route("/api/players/ban", post(ban_player))
        .route("/api/players/unban", post(unban_player))
        .route("/api/players/mute", post(mute_player))
        .route("/api/players/unmute", post(unmute_player))
        .route("/api/whitelist", get(whitelist))
        .route("/api/whitelist/add", post(whitelist_add))
        .route("/api/whitelist/remove", post(whitelist_remove))
}

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

/// A connected player, over the wire  -  real position and appearance data for the world view to
/// draw a stylized avatar from, and enough else (health, mana, address) for the player list.
#[derive(Serialize)]
pub(super) struct PlayerResponse {
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
    /// `"name"`, `"ip"` (or `"address"`) or `"uuid"`  -  the same three words `/ban` accepts.
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
    /// How long, in the console's own `10m`/`2h`/`1d`/`1h30m`/bare-seconds grammar
    /// ([`crate::game::tile_log::parse_duration`]), or absent/empty for a permanent mute.
    ///
    /// Parsed here rather than in the browser on purpose. The panel used to parse it client-side
    /// and send seconds, with a narrower grammar than the console's, and anything that grammar did
    /// not match (`10 min`, `1w`, `1.5h`) became an *omitted* field: a permanent mute, silently,
    /// through the same success path as the intended one. A duration the server cannot read is now
    /// refused with a message naming what it does read, which is the only reading of a typo that
    /// is not the most destructive one available.
    #[serde(default)]
    duration: Option<String>,
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
    let duration_secs = match req
        .duration
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        None => None,
        Some(text) => match crate::game::tile_log::parse_duration(text) {
            Some(d) => Some(d.as_secs()),
            None => {
                return err(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "'{text}' is not a duration: use 10m, 2h, 1d, 1h30m or a plain number of \
                         seconds, and leave it empty for a permanent mute"
                    ),
                );
            }
        },
    };
    match ask(&state, |reply| ServerEvent::PanelMute {
        actor,
        name: req.name,
        reason: req.reason,
        duration_secs,
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

async fn whitelist_add(
    State(state): State<PanelState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<NameRequest>,
) -> Response {
    let actor = match authorized(&state, &headers, perm::SERVER_WHITELIST).await {
        Ok(actor) => actor,
        Err(resp) => return resp,
    };
    match ask(&state, |reply| ServerEvent::PanelWhitelistAdd {
        actor,
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
    let actor = match authorized(&state, &headers, perm::SERVER_WHITELIST).await {
        Ok(actor) => actor,
        Err(resp) => return resp,
    };
    match ask(&state, |reply| ServerEvent::PanelWhitelistRemove {
        actor,
        name: req.name,
        reply,
    })
    .await
    {
        Ok(changed) => Json(ChangedResponse { changed }).into_response(),
        Err(resp) => resp,
    }
}
