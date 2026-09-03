//! Sessions, login, logout, and the two permission checks every other route in this module reuses
//! ([`authorized`] for an ordinary request, [`authorized_token`] for the two WebSocket upgrades).
//!
//! A session is a bearer token mapped to the account that signed in and when it was last used
//! ([`Session`]). The map lives only in [`super::PanelState`], not the account store: a panel
//! restart signs everyone out, which is fine, nothing here is persisted state.
//!
//! Two things make a captured token stop working on their own, neither of which existed before
//! this file grew them: an idle timeout ([`SESSION_IDLE_TIMEOUT`], checked and refreshed on every
//! lookup in [`session_name`]) and a real `/api/logout` route ([`logout`]) that removes the token
//! from the map, rather than the old "sign out" button, which only cleared the browser's own
//! `localStorage` and left the token itself valid until the process restarted.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::admin::{Account, Verdict};

use super::{PanelState, ServerEvent, ask, auth_lookup, err};

/// One signed-in session: which account it belongs to, and when it was last used.
pub(super) struct Session {
    name: String,
    last_seen: Instant,
}

/// How long a session may sit unused before [`session_name`] treats it as gone. Sliding, not
/// fixed: any authorized request or WebSocket tick refreshes `last_seen`, so an operator actively
/// watching the panel is never signed out mid-session, only a token nobody has used in this long.
///
/// ponytail: expiry is checked lazily, on the next lookup of that exact token, not swept
/// proactively — a session nobody ever looks up again just sits inert in the map until the
/// process restarts. That is a bounded memory cost (one entry per login, on a loopback-only
/// surface with a handful of operators), not a security gap: the token itself already stopped
/// authorizing anything the moment it went idle. Add a periodic sweep if the map's size ever
/// becomes worth watching.
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);

pub(super) fn router() -> Router<PanelState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/unclaimed", get(unclaimed))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
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

/// The account name for a live, non-idle session token, refreshing its `last_seen` on the way out.
/// An expired session is removed from the map right here rather than left for something else to
/// clean up later.
pub(super) fn session_name(state: &PanelState, token: &str) -> Option<String> {
    let mut sessions = state.sessions();
    let session = sessions.get_mut(token)?;
    if session.last_seen.elapsed() > SESSION_IDLE_TIMEOUT {
        sessions.remove(token);
        return None;
    }
    session.last_seen = Instant::now();
    Some(session.name.clone())
}

pub(super) fn issue_session(state: &PanelState, name: String) -> Response {
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
    state.sessions().insert(
        token.clone(),
        Session {
            name: name.clone(),
            last_seen: Instant::now(),
        },
    );
    Json(LoginResponse {
        session: token,
        name,
    })
    .into_response()
}

fn bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// The session-and-permission check every endpoint other than `/api/unclaimed`, `/api/login` and
/// `/api/logout` needs: a valid session (a bearer token that resolves to a signed-in account), and
/// that account's group currently granting `permission`. Checked fresh against the game task's
/// live `Admin` store on *every* call rather than cached at login — a group's permissions, or an
/// account's own group, can change mid-session (through this very panel), and a session issued
/// before that change must not go on behaving as if it never happened.
///
/// Returns the account name on success: a handful of handlers (anything that can hand power to
/// someone else — creating an account, moving one between groups, editing a group's own permission
/// set) need to know who is asking, for `Admin::group_within_reach`'s anti-escalation check.
///
/// See this module's own doc comment for the full route-to-permission map.
pub(super) async fn authorized(
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
/// (`status::ws_upgrade`, `worlds::world_ws_upgrade`) can reuse the exact same check with a
/// session that arrived as a query parameter rather than a bearer header (a browser cannot attach
/// a custom header to a WebSocket upgrade request).
pub(super) async fn authorized_token(
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

/// Whether `account` currently holds `permission`, as a plain yes or no.
///
/// [`authorized_token`] answers the same question for a request, where a "no" has to become a 403.
/// A long-lived socket has no response to fail: it simply withholds what the account may not see,
/// or (see `status::stream_status` and `worlds::stream_world`) drops the connection once the
/// session itself is no longer valid at all.
pub(super) async fn has_permission(
    state: &PanelState,
    account: &str,
    permission: crate::admin::Permission,
) -> bool {
    ask(state, |reply| ServerEvent::PanelAuthorize {
        name: account.to_string(),
        permission: permission.as_str().to_string(),
        reply,
    })
    .await
    .unwrap_or(false)
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

/// A real argon2 hash of a throwaway password, built once and verified against whenever a login
/// names an account that does not exist. Its only job is costing exactly one ordinary
/// verification, so probing account names cannot be told apart from guessing passwords by timing.
fn dummy_login_hash() -> &'static str {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| {
        Account::new("timing-dummy", "timing-dummy-password", "default")
            .map(|account| account.hash)
            .unwrap_or_default()
    })
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

    // A missing account and a wrong password must be indistinguishable from outside: same
    // message, same argon2 cost (verified against a dummy hash when the account does not exist,
    // instead of returning early), and the same throttle failure recorded, so probing names is
    // exactly as slow and exactly as throttled as guessing passwords.
    let (hash, exists) = match lookup.hash_and_group {
        Some((hash, _group)) => (hash, true),
        None => (dummy_login_hash().to_owned(), false),
    };
    let password = req.password.clone();
    let ok = tokio::task::spawn_blocking(move || Account::verify_hash(&hash, &password))
        .await
        .unwrap_or(false)
        && exists;
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

/// Removes the caller's own session token from the map, if any — the "sign out" button's real
/// counterpart. Previously this route did not exist at all: signing out only cleared the token
/// from the browser's own `localStorage` (`api.ts`'s `logout()`), which left the token itself
/// valid until the panel process restarted. Idempotent (a missing or already-removed token is not
/// an error) and never requires a permission: an operator must always be able to sign themselves
/// out, even from a session the panel would otherwise refuse for some other reason.
async fn logout(State(state): State<PanelState>, headers: axum::http::HeaderMap) -> Response {
    if let Some(token) = bearer_token(&headers) {
        state.sessions().remove(token);
    }
    StatusCode::OK.into_response()
}
