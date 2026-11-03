//! HTTP API + web page (assets embedded in the binary).

use crate::derive::{self, Window};
use crate::i18n::{self, t};
use crate::poller::{self, Status};
use crate::providers::{self, Extra, Provider, Share, Snapshot};
use crate::store::{Account, NewAccount, Tokens};
use crate::{now_ms, App, Flow, VERSION};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const STALE_MS: i64 = 15 * 60_000;
const FLOW_TTL_MS: i64 = 15 * 60_000;
const MANUAL_MIN_GAP_MS: i64 = 20_000;

type AppState = State<Arc<App>>;

// ---------- Errors ----------
pub struct ApiErr(StatusCode, String);
impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}
fn err(code: StatusCode, msg: impl Into<String>) -> ApiErr {
    ApiErr(code, msg.into())
}
type R<T> = Result<T, ApiErr>;

// ---------- Views ----------
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WindowView {
    #[serde(flatten)]
    pub w: Window,
    /// Fallback label in the server language (the page localises itself).
    pub label: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub provider: String,
    pub provider_name: &'static str,
    pub tool: &'static str,
    pub label: String,
    pub color: String,
    pub order: i64,
    pub email: Option<String>,
    pub org_name: Option<String>,
    pub org_uuid: Option<String>,
    pub plan: String,
    pub status: &'static str,
    pub last_error: Option<String>,
    pub fetched_at: Option<i64>,
    pub next_poll_at: Option<i64>,
    pub busy: bool,
    pub windows: Vec<WindowView>,
    pub session: Option<WindowView>,
    pub week: Option<WindowView>,
    pub extra: Option<Extra>,
    pub breakdown: Vec<Share>,
    pub headroom: Option<f64>,
    /// Best account of its provider right now (only when the provider has several accounts).
    pub recommended: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateView {
    pub now: i64,
    pub version: &'static str,
    pub public_url: String,
    pub lang: &'static str,
    /// Recommended account id per provider.
    pub recommended: HashMap<String, String>,
    pub accounts: Vec<AccountView>,
}

fn snapshot_of(provider: Option<Provider>, stored: &Value) -> Snapshot {
    // Current format is the normalised snapshot; older data stored the raw provider answer.
    serde_json::from_value::<Snapshot>(stored.clone())
        .ok()
        .filter(|s| stored.get("limits").is_some() || !s.limits.is_empty())
        .unwrap_or_else(|| provider.map(|p| p.normalize(stored)).unwrap_or_default())
}

fn account_view(app: &App, acc: &Account, now: i64) -> AccountView {
    let lang = app.cfg.lang;
    let provider = Provider::from_id(&acc.provider);
    let info = provider.map(|p| p.info());
    let rt = app.rt(&acc.id);
    let latest = app.store.latest(&acc.id);
    let snap = latest.as_ref().map(|(_, v)| snapshot_of(provider, v)).unwrap_or_default();
    let windows: Vec<WindowView> = derive::derive(&app.store, &acc.id, &snap, now)
        .into_iter()
        .map(|w| WindowView { label: i18n::limit_label(lang, &w.kind, w.name.as_deref(), w.window_ms), w })
        .collect();
    let raw_windows: Vec<Window> = windows.iter().map(|w| w.w.clone()).collect();
    let age = latest.as_ref().map(|(ts, _)| now - ts);
    let status = match rt.status {
        Status::Pending => "pending",
        Status::Reauth => "reauth",
        Status::Ok if age.is_some_and(|a| a > STALE_MS) => "stale",
        Status::Ok => "ok",
        Status::Error if age.is_some_and(|a| a <= STALE_MS) => "degraded",
        Status::Error => "error",
    };
    AccountView {
        id: acc.id.clone(),
        provider: acc.provider.clone(),
        provider_name: info.as_ref().map(|i| i.name).unwrap_or("?"),
        tool: info.as_ref().map(|i| i.tool).unwrap_or("?"),
        label: acc.label.clone(),
        color: acc.color.clone(),
        order: acc.order,
        email: acc.email.clone(),
        org_name: acc.org_name.clone(),
        org_uuid: acc.org_uuid.clone(),
        plan: provider.map(|p| p.plan_label(acc.profile.as_ref())).unwrap_or_default(),
        status,
        last_error: rt.last_error.clone(),
        fetched_at: latest.as_ref().map(|(ts, _)| *ts),
        next_poll_at: rt.next_poll_at,
        busy: rt.busy,
        session: windows.iter().find(|w| w.w.kind == "session").cloned(),
        week: windows.iter().find(|w| w.w.kind == "week").cloned(),
        headroom: derive::headroom(&raw_windows),
        windows,
        extra: snap.extra,
        breakdown: snap.breakdown,
        recommended: false,
    }
}

/// Per provider: the usable account with the most headroom; on a tie, the one whose session ends first.
pub fn recommend(accounts: &[AccountView]) -> HashMap<String, String> {
    let mut by_provider: HashMap<&str, Vec<&AccountView>> = HashMap::new();
    for a in accounts {
        by_provider.entry(a.provider.as_str()).or_default().push(a);
    }
    let mut out = HashMap::new();
    for (p, list) in by_provider {
        if list.len() < 2 {
            continue;
        }
        let best = list.iter().filter(|a| a.headroom.is_some() && matches!(a.status, "ok" | "degraded")).max_by(|a, b| {
            a.headroom.partial_cmp(&b.headroom).unwrap_or(std::cmp::Ordering::Equal).then_with(|| {
                let ra = a.session.as_ref().and_then(|w| w.w.resets_at).unwrap_or(i64::MAX);
                let rb = b.session.as_ref().and_then(|w| w.w.resets_at).unwrap_or(i64::MAX);
                rb.cmp(&ra)
            })
        });
        if let Some(b) = best {
            out.insert(p.to_string(), b.id.clone());
        }
    }
    out
}

pub fn state_view(app: &App) -> StateView {
    let now = now_ms();
    let mut accounts: Vec<AccountView> = app.store.list().iter().map(|a| account_view(app, a, now)).collect();
    let recommended = recommend(&accounts);
    for a in &mut accounts {
        a.recommended = recommended.get(&a.provider) == Some(&a.id);
    }
    StateView {
        now,
        version: VERSION,
        public_url: app.cfg.public_url.clone(),
        lang: if app.cfg.lang == i18n::Lang::Fr { "fr" } else { "en" },
        recommended,
        accounts,
    }
}

fn pct(w: &Option<WindowView>) -> String {
    w.as_ref().map(|w| format!("{} %", w.w.utilization.round() as i64)).unwrap_or_else(|| "—".into())
}

fn homepage_view(lang: i18n::Lang, a: &AccountView) -> Value {
    let statut = match a.status {
        "ok" if a.recommended => t(lang, "st_top"),
        "ok" => t(lang, "st_ok"),
        "degraded" => t(lang, "st_cached"),
        "stale" => t(lang, "st_stale"),
        "error" => t(lang, "st_error"),
        "reauth" => t(lang, "st_reauth"),
        _ => t(lang, "st_pending"),
    };
    let session = match &a.session {
        Some(w) if w.w.resets_at.is_none() => format!("0 % ({})", t(lang, "free")),
        s => pct(s),
    };
    json!({
        "label": a.label,
        "provider": a.provider_name,
        "plan": a.plan,
        "session": session,
        "semaine": pct(&a.week),
        "reset": i18n::duration(lang, a.session.as_ref().and_then(|w| w.w.remaining_ms)),
        "reset_semaine": i18n::duration(lang, a.week.as_ref().and_then(|w| w.w.remaining_ms)),
        "dispo": a.headroom.map(|h| format!("{} %", h.round() as i64)).unwrap_or_else(|| "—".into()),
        "statut": statut,
    })
}

// ---------- Handlers ----------
async fn h_state(State(app): AppState) -> Json<StateView> {
    Json(state_view(&app))
}

async fn h_health(State(app): AppState) -> Json<Value> {
    Json(json!({ "ok": true, "version": VERSION, "accounts": app.store.list().len() }))
}

async fn h_providers() -> Json<Value> {
    Json(json!({ "providers": Provider::ALL.iter().map(|p| p.info()).collect::<Vec<_>>() }))
}

#[derive(Deserialize)]
struct HistoryQ {
    account: String,
    key: Option<String>,
    hours: Option<f64>,
}
async fn h_history(State(app): AppState, Query(q): Query<HistoryQ>) -> R<Json<Value>> {
    if app.store.get(&q.account).is_none() {
        return Err(err(StatusCode::NOT_FOUND, "unknown account"));
    }
    let key = q.key.unwrap_or_else(|| "five_hour".into());
    let hours = q.hours.unwrap_or(24.0).clamp(1.0, 24.0 * 45.0);
    let since = now_ms() - (hours * 3_600_000.0) as i64;
    let points: Vec<Value> = app.store.series(&q.account, &key, since).into_iter().map(|(t, u, r)| json!([t, u, r])).collect();
    Ok(Json(json!({ "account": q.account, "key": key, "points": points })))
}

async fn h_homepage(State(app): AppState) -> Json<Value> {
    let s = state_view(&app);
    let lang = app.cfg.lang;
    let recs: Vec<&AccountView> = s.accounts.iter().filter(|a| a.recommended).collect();
    let alerts = s
        .accounts
        .iter()
        .filter(|a| a.windows.iter().any(|w| matches!(w.w.level, "crit" | "full")) || matches!(a.status, "error" | "reauth" | "stale"))
        .count();
    // Single account per provider: it is the one to use.
    let picks: Vec<&AccountView> = if recs.is_empty() { s.accounts.iter().collect() } else { recs };
    Json(json!({
        "recommande": if picks.is_empty() { "—".to_string() } else { picks.iter().map(|a| a.label.clone()).collect::<Vec<_>>().join(" · ") },
        "dispo": picks.iter().filter_map(|a| a.headroom).map(|h| format!("{} %", h.round() as i64)).collect::<Vec<_>>().join(" · "),
        "comptes": s.accounts.iter().map(|a| format!("{} {}", a.label, pct(&a.session))).collect::<Vec<_>>().join(" · "),
        "alertes": alerts.to_string(),
        "lang": if lang == i18n::Lang::Fr { "fr" } else { "en" },
    }))
}

async fn h_homepage_one(State(app): AppState, Path(key): Path<String>) -> R<Json<Value>> {
    let s = state_view(&app);
    let a = s
        .accounts
        .iter()
        .find(|a| a.id == key || a.label.eq_ignore_ascii_case(&key))
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "unknown account"))?;
    Ok(Json(homepage_view(app.cfg.lang, a)))
}

#[derive(Deserialize, Default)]
struct RefreshBody {
    id: Option<String>,
}
async fn h_refresh(State(app): AppState, body: Option<Json<RefreshBody>>) -> Json<Value> {
    let ids: Vec<String> = match body.and_then(|b| b.0.id) {
        Some(id) => vec![id],
        None => app.store.list().into_iter().map(|a| a.id).collect(),
    };
    let mut results = vec![];
    for id in ids {
        let rt = app.rt(&id);
        let r = if rt.busy {
            json!({ "id": id, "skipped": "busy" })
        } else if rt.status == Status::Ok && rt.fetched_at.is_some_and(|t| now_ms() - t < MANUAL_MIN_GAP_MS) {
            json!({ "id": id, "skipped": "recent" })
        } else if rt.status == Status::Reauth {
            json!({ "id": id, "skipped": "sign-in required" })
        } else {
            poller::poll_once(&app, &id).await;
            json!({ "id": id, "ok": true })
        };
        results.push(r);
    }
    Json(json!({ "results": results, "state": state_view(&app) }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginStart {
    provider: Option<String>,
    label: Option<String>,
    account_id: Option<String>,
}
async fn h_login_start(State(app): AppState, Json(b): Json<LoginStart>) -> R<Json<Value>> {
    let provider = match (&b.account_id, &b.provider) {
        (Some(id), _) => {
            let acc = app.store.get(id).ok_or_else(|| err(StatusCode::NOT_FOUND, "unknown account"))?;
            Provider::from_id(&acc.provider)
        }
        (None, Some(p)) => Provider::from_id(p),
        (None, None) => Some(Provider::Claude),
    }
    .ok_or_else(|| err(StatusCode::BAD_REQUEST, "unknown provider"))?;
    let pkce = providers::new_pkce();
    let url = provider.authorize_url(&pkce);
    let flow_id: String = pkce.state.chars().take(16).collect();
    let mut flows = app.flows.lock().unwrap();
    flows.retain(|_, f| now_ms() - f.created_at < FLOW_TTL_MS);
    flows.insert(
        flow_id.clone(),
        Flow {
            provider,
            pkce,
            label: b.label.map(|s| s.trim().chars().take(40).collect::<String>()).filter(|s| !s.is_empty()),
            account_id: b.account_id,
            created_at: now_ms(),
        },
    );
    Ok(Json(json!({ "flowId": flow_id, "url": url, "loginMode": provider.info().login_mode })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginFinish {
    flow_id: String,
    code: String,
}
async fn h_login_finish(State(app): AppState, Json(b): Json<LoginFinish>) -> R<Json<Value>> {
    if b.code.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "missing code"));
    }
    let flow = app
        .flows
        .lock()
        .unwrap()
        .remove(&b.flow_id)
        .filter(|f| now_ms() - f.created_at < FLOW_TTL_MS)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "sign-in session expired: start again"))?;
    let login = match flow.provider.exchange(&app.http, &b.code, &flow.pkce).await {
        Ok(l) => l,
        Err(e) => {
            // Keep the flow so the user can retry with a corrected paste.
            let msg = format!("code exchange refused: {e}");
            app.flows.lock().unwrap().insert(b.flow_id.clone(), flow);
            return Err(err(StatusCode::BAD_GATEWAY, msg));
        }
    };
    let provider = flow.provider;
    let id = login.identity;
    let tokens = Tokens {
        access_token: login.tokens.access_token,
        refresh_token: login.tokens.refresh_token,
        expires_at: login.tokens.expires_at,
        scope: login.tokens.scope,
        meta: login.tokens.meta,
    };

    // Reconnection: explicit account, or same provider + identity (account + organisation).
    let existing = match &flow.account_id {
        Some(aid) => app.store.get(aid),
        None => app.store.list().into_iter().find(|a| {
            a.provider == provider.id()
                && match (&a.account_uuid, &id.account_uuid) {
                    (Some(x), Some(y)) => x == y,
                    _ => a.email.is_some() && a.email == id.email,
                }
                && a.org_uuid == id.org_uuid
        }),
    };
    if let Some(acc) = existing {
        app.store
            .update(&acc.id, |a| {
                a.tokens = tokens;
                if login.profile.is_some() {
                    a.profile = login.profile.clone();
                }
                a.email = id.email.clone().or(a.email.take());
                a.org_name = id.org_name.clone().or(a.org_name.take());
                a.account_uuid = id.account_uuid.clone().or(a.account_uuid.take());
                a.org_uuid = id.org_uuid.clone().or(a.org_uuid.take());
            })
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
        app.track(&acc.id, Duration::from_millis(200));
        app.accounts_changed();
        return Ok(Json(json!({ "id": acc.id, "reconnected": true, "email": id.email })));
    }
    let acc = app
        .store
        .add(NewAccount {
            provider: provider.id().to_string(),
            label: flow.label,
            email: id.email.clone(),
            org_name: id.org_name,
            account_uuid: id.account_uuid,
            org_uuid: id.org_uuid,
            profile: login.profile,
            tokens,
        })
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    app.with_rt(&acc.id, |r| r.profile_at = Some(now_ms()));
    app.track(&acc.id, Duration::from_millis(200));
    app.accounts_changed();
    Ok(Json(json!({ "id": acc.id, "reconnected": false, "email": id.email })))
}

#[derive(Deserialize)]
struct AccountPatch {
    label: Option<String>,
    color: Option<String>,
    order: Option<i64>,
}
async fn h_patch_account(State(app): AppState, Path(id): Path<String>, Json(b): Json<AccountPatch>) -> R<Json<StateView>> {
    let label = b.label.map(|s| s.trim().chars().take(40).collect::<String>()).filter(|s| !s.is_empty());
    let color = b.color.filter(|c| {
        let hex = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit());
        let slot = c.len() == 5 && c.starts_with("acc-") && c.as_bytes()[4].is_ascii_digit();
        hex || slot
    });
    let updated = app
        .store
        .update(&id, |a| {
            if let Some(l) = label {
                a.label = l;
            }
            if let Some(c) = color {
                a.color = c;
            }
            if let Some(o) = b.order {
                a.order = o;
            }
        })
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    if updated.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "unknown account"));
    }
    app.accounts_changed();
    Ok(Json(state_view(&app)))
}

async fn h_delete_account(State(app): AppState, Path(id): Path<String>) -> R<Json<StateView>> {
    app.untrack(&id);
    match app.store.remove(&id) {
        Ok(true) => {
            app.accounts_changed();
            Ok(Json(state_view(&app)))
        }
        Ok(false) => Err(err(StatusCode::NOT_FOUND, "unknown account")),
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))),
    }
}

// ---------- Assets ----------
macro_rules! asset {
    ($name:literal, $mime:literal) => {
        get(|| async {
            let body: std::borrow::Cow<'static, str> = match std::env::var("VR_WEB_DIR") {
                // Dev mode: read from disk to iterate without rebuilding.
                Ok(dir) => std::fs::read_to_string(format!("{dir}/{}", $name)).unwrap_or_default().into(),
                Err(_) => include_str!(concat!("../web/", $name)).into(),
            };
            ([(header::CONTENT_TYPE, $mime), (header::CACHE_CONTROL, "no-cache")], body)
        })
    };
}

/// Mutations require a custom header: a third-party site cannot send it without a CORS preflight (refused).
async fn guard(req: Request, next: Next) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD && req.headers().get("x-viberooms").map(|v| v.as_bytes()) != Some(b"1") {
        return err(StatusCode::FORBIDDEN, "X-VibeRooms header required").into_response();
    }
    let mut res = next.run(req).await;
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

pub fn router(app: Arc<App>) -> Router {
    let api = Router::new()
        .route("/api/health", get(h_health))
        .route("/api/providers", get(h_providers))
        .route("/api/state", get(h_state))
        .route("/api/history", get(h_history))
        .route("/api/homepage", get(h_homepage))
        .route("/api/homepage/{key}", get(h_homepage_one))
        .route("/api/refresh", post(h_refresh))
        .route("/api/login/start", post(h_login_start))
        .route("/api/login/finish", post(h_login_finish))
        .route("/api/accounts/{id}", patch(h_patch_account).delete(h_delete_account))
        .layer(middleware::from_fn(guard));
    Router::new()
        .route("/", asset!("index.html", "text/html; charset=utf-8"))
        .route("/app.js", asset!("app.js", "text/javascript; charset=utf-8"))
        .route("/i18n.js", asset!("i18n.js", "text/javascript; charset=utf-8"))
        .route("/app.css", asset!("app.css", "text/css; charset=utf-8"))
        .route("/favicon.svg", asset!("favicon.svg", "image/svg+xml"))
        .merge(api)
        .with_state(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(id: &str, provider: &str, headroom: f64, status: &'static str) -> AccountView {
        AccountView {
            id: id.into(),
            provider: provider.into(),
            provider_name: "",
            tool: "",
            label: id.into(),
            color: "acc-1".into(),
            order: 0,
            email: None,
            org_name: None,
            org_uuid: None,
            plan: String::new(),
            status,
            last_error: None,
            fetched_at: None,
            next_poll_at: None,
            busy: false,
            windows: vec![],
            session: None,
            week: None,
            extra: None,
            breakdown: vec![],
            headroom: Some(headroom),
            recommended: false,
        }
    }

    #[test]
    fn recommends_per_provider_and_skips_single_accounts() {
        let list = vec![
            acc("a", "claude", 40.0, "ok"),
            acc("b", "claude", 70.0, "ok"),
            acc("c", "claude", 90.0, "reauth"),
            acc("d", "codex", 10.0, "ok"),
        ];
        let r = recommend(&list);
        assert_eq!(r.get("claude").map(String::as_str), Some("b"));
        assert_eq!(r.get("codex"), None);
    }
}
