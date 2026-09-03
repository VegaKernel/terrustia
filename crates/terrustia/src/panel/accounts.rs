//! Accounts, groups, the group-permission editor, and the audit log — everything gated on
//! `admin.accounts`, `admin.groups` or `admin.audit`.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::admin::perm;
use crate::game::server::{PanelAccountInfo, PanelGroupInfo, ServerEvent};

use super::auth::authorized;
use super::{PanelState, ask, err};

pub(super) fn router() -> Router<PanelState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/accounts", get(accounts))
        .route("/api/accounts/group", post(set_account_group))
        .route("/api/accounts/create", post(create_account))
        .route("/api/accounts/delete", post(delete_account))
        .route("/api/permissions", get(known_permissions))
        .route("/api/groups/permissions", post(set_group_permission))
        .route("/api/audit", get(audit_log))
}

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
    let account = match tokio::task::spawn_blocking(move || {
        crate::admin::Account::new(&name, &password, &group)
    })
    .await
    {
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
    let actor = match authorized(&state, &headers, perm::ADMIN_ACCOUNTS).await {
        Ok(actor) => actor,
        Err(resp) => return resp,
    };
    match ask(&state, |reply| ServerEvent::PanelDeleteAccount {
        actor,
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

/// `n` comes off the query string, so it is whatever the caller typed. The frontend asks for 200;
/// this is the ceiling on how much of the file a single request can be made to parse and serialize.
const MAX_AUDIT_N: usize = 1000;

/// The tail of the audit log. Two steps on purpose: the game task is asked only where the log
/// lives (a `PathBuf` clone), and the read and the JSON parse happen on the panel's own blocking
/// pool. Reading it on the game task instead put a whole-file read of a file capped at 8 MB by
/// default against the tick, every five seconds, which is exactly what this module's own
/// "off the game task, always" rule exists to prevent.
async fn audit_log(
    State(state): State<PanelState>,
    Query(q): Query<AuditQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err(resp) = authorized(&state, &headers, perm::ADMIN_AUDIT).await {
        return resp;
    }
    let path = match ask(&state, |reply| ServerEvent::PanelAuditPath { reply }).await {
        Ok(path) => path,
        Err(resp) => return resp,
    };
    // An in-memory log (a world with nowhere to save) has no file and no history to show.
    let Some(path) = path else {
        return Json(Vec::<AuditEntryResponse>::new()).into_response();
    };
    let n = q.n.min(MAX_AUDIT_N);
    let events =
        match tokio::task::spawn_blocking(move || crate::admin::audit::tail_file(&path, n)).await {
            Ok(events) => events,
            Err(e) => {
                warn!(error = %e, "the audit-log read task failed");
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "could not read the audit log",
                );
            }
        };
    Json(
        events
            .into_iter()
            .map(AuditEntryResponse::from)
            .collect::<Vec<_>>(),
    )
    .into_response()
}
