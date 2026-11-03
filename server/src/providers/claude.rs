//! Claude subscriptions (Pro, Max, Team, Enterprise).
//!
//! Uses the OAuth endpoints and the usage endpoint that Claude Code itself uses. They are not a
//! public API: constants were read from Claude Code 2.1.x and may change without notice.
//! VibeRooms requests the `user:profile` scope only (read profile and usage, no inference).

use super::{round_reset, Extra, Identity, Limit, Login, Pkce, Share, Snapshot, TokenSet};
use crate::http::{parse_rfc3339, ApiError, Http};
use serde::Deserialize;
use serde_json::Value;

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const H: i64 = 3_600_000;
const D: i64 = 24 * H;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
fn authorize_base() -> String {
    env_or("VR_CLAUDE_AUTHORIZE_URL", "https://claude.com/cai/oauth/authorize")
}
fn token_url() -> String {
    env_or("VR_CLAUDE_TOKEN_URL", "https://platform.claude.com/v1/oauth/token")
}
fn redirect_uri() -> String {
    env_or("VR_CLAUDE_REDIRECT_URI", "https://platform.claude.com/oauth/code/callback")
}
fn api_base() -> String {
    env_or("VR_CLAUDE_API_BASE", "https://api.anthropic.com")
}
fn scopes() -> String {
    env_or("VR_CLAUDE_SCOPES", "user:profile")
}

pub fn authorize_url(p: &Pkce) -> String {
    let mut u = url::Url::parse(&authorize_base()).expect("VR_CLAUDE_AUTHORIZE_URL");
    u.query_pairs_mut()
        .append_pair("code", "true")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri())
        .append_pair("scope", &scopes())
        .append_pair("code_challenge", &p.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &p.state);
    u.into()
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
    account: Option<Value>,
    organization: Option<Value>,
}

fn token_set(r: &TokenResponse) -> TokenSet {
    TokenSet {
        access_token: r.access_token.clone(),
        refresh_token: r.refresh_token.clone(),
        expires_at: crate::now_ms() + r.expires_in.unwrap_or(3600) * 1000,
        scope: r.scope.clone(),
        meta: None,
    }
}

fn s(v: &Option<Value>, ptr: &str) -> Option<String> {
    v.as_ref().and_then(|x| x.pointer(ptr)).and_then(Value::as_str).map(str::to_owned)
}

pub async fn exchange(http: &Http, pasted: &str, pkce: &Pkce) -> Result<Login, ApiError> {
    // Claude shows `code#state` on its callback page.
    let (code, state) = match pasted.trim().split_once('#') {
        Some((c, st)) => (c.trim(), Some(st.trim())),
        None => (pasted.trim(), None),
    };
    if state.is_some_and(|st| st != pkce.state) {
        return Err(ApiError::new(400, "this code does not belong to the sign-in link that was opened"));
    }
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "state": pkce.state,
        "client_id": CLIENT_ID,
        "redirect_uri": redirect_uri(),
        "code_verifier": pkce.verifier,
    });
    let v = http.json(http.inner.post(token_url()).json(&body)).await?;
    let r: TokenResponse = serde_json::from_value(v).map_err(|e| ApiError::new(500, format!("unexpected token response: {e}")))?;
    let tokens = token_set(&r);
    let profile = profile(http, &tokens).await.ok();
    let mut identity = profile.as_ref().map(identity).unwrap_or_default();
    identity.email = identity.email.or_else(|| s(&r.account, "/email_address"));
    identity.account_uuid = s(&r.account, "/uuid").or(identity.account_uuid);
    identity.org_uuid = s(&r.organization, "/uuid").or(identity.org_uuid);
    identity.org_name = identity.org_name.or_else(|| s(&r.organization, "/name"));
    Ok(Login { tokens, identity, profile })
}

pub async fn refresh(http: &Http, refresh_token: &str) -> Result<TokenSet, ApiError> {
    let body = serde_json::json!({ "grant_type": "refresh_token", "refresh_token": refresh_token, "client_id": CLIENT_ID });
    let v = http.json(http.inner.post(token_url()).json(&body)).await?;
    let r: TokenResponse = serde_json::from_value(v).map_err(|e| ApiError::new(500, format!("unexpected refresh response: {e}")))?;
    let mut t = token_set(&r);
    if t.refresh_token.is_none() {
        t.refresh_token = Some(refresh_token.to_string()); // not rotated: keep the previous one
    }
    Ok(t)
}

async fn get(http: &Http, path: &str, tokens: &TokenSet) -> Result<Value, ApiError> {
    let req = http
        .inner
        .get(format!("{}{}", api_base(), path))
        .bearer_auth(&tokens.access_token)
        .header("anthropic-beta", OAUTH_BETA)
        .header("Content-Type", "application/json");
    http.json(req).await
}

pub async fn profile(http: &Http, tokens: &TokenSet) -> Result<Value, ApiError> {
    get(http, "/api/oauth/profile", tokens).await
}

pub async fn fetch(http: &Http, tokens: &TokenSet) -> Result<(Snapshot, Value), ApiError> {
    let raw = get(http, "/api/oauth/usage", tokens).await?;
    Ok((normalize(&raw), raw))
}

pub fn identity(p: &Value) -> Identity {
    let g = |ptr: &str| p.pointer(ptr).and_then(Value::as_str).map(str::to_owned);
    Identity {
        email: g("/account/email"),
        account_uuid: g("/account/uuid"),
        org_uuid: g("/organization/uuid"),
        org_name: g("/organization/name"),
    }
}

pub fn plan_label(p: Option<&Value>) -> String {
    let st = |ptr: &str| p.and_then(|v| v.pointer(ptr)).and_then(Value::as_str).unwrap_or("").to_lowercase();
    let b = |ptr: &str| p.and_then(|v| v.pointer(ptr)).and_then(Value::as_bool).unwrap_or(false);
    if let Some(name) = p.and_then(|v| v.pointer("/organization/plan_display_name")).and_then(Value::as_str).filter(|n| !n.is_empty()) {
        return name.to_string();
    }
    let tier = st("/organization/rate_limit_tier");
    let kind = st("/organization/organization_type");
    let mult = tier.split('_').find(|t| t.len() > 1 && t.ends_with('x') && t[..t.len() - 1].chars().all(|c| c.is_ascii_digit()));
    let base = if kind.contains("enterprise") {
        "Enterprise"
    } else if kind.contains("team") {
        "Team"
    } else if kind.contains("max") || tier.contains("max") || b("/account/has_claude_max") {
        "Max"
    } else if kind.contains("pro") || b("/account/has_claude_pro") {
        "Pro"
    } else {
        "Claude"
    };
    match mult {
        Some(m) => format!("{base} {m}"),
        None => base.to_string(),
    }
}

fn window(key: &str, val: &Value) -> Option<Limit> {
    let util = val.get("utilization").and_then(Value::as_f64)?;
    let (kind, name, ms) = match key {
        "five_hour" => ("session", None, 5 * H),
        "seven_day" => ("week", None, 7 * D),
        k => {
            let rest = k.strip_prefix("seven_day_")?;
            let name = match rest {
                "opus" => "Opus".to_string(),
                "sonnet" => "Sonnet".to_string(),
                "oauth_apps" => "OAuth apps".to_string(),
                "cowork" => "Cowork".to_string(),
                other => other.replace('_', " "),
            };
            ("model", Some(name), 7 * D)
        }
    };
    Some(Limit::percent(key, kind, name, Some(ms), util.clamp(0.0, 100.0), val.get("resets_at").and_then(parse_rfc3339).map(round_reset)))
}

fn money(v: Option<&Value>) -> Option<(f64, Option<String>)> {
    let v = v?;
    let minor = v.get("amount_minor")?.as_f64()?;
    let exp = v.get("exponent").and_then(Value::as_i64).unwrap_or(2) as i32;
    Some((minor / 10f64.powi(exp), v.get("currency").and_then(Value::as_str).map(str::to_owned)))
}

fn extra(raw: &Value) -> Option<Extra> {
    // Newer shape: `spend` with typed amounts. Fallback: `extra_usage` (minor units + decimal_places).
    if let Some(sp) = raw.get("spend").filter(|v| v.is_object()) {
        let used = money(sp.get("used"));
        let limit = money(sp.get("limit").filter(|v| !v.is_null())).or_else(|| money(sp.get("cap").and_then(|c| c.get("money"))));
        let currency = limit.as_ref().and_then(|l| l.1.clone()).or_else(|| used.as_ref().and_then(|u| u.1.clone()));
        return Some(Extra {
            enabled: sp.get("enabled").and_then(Value::as_bool).unwrap_or(false),
            used: used.map(|u| u.0),
            limit: limit.map(|l| l.0),
            utilization: sp.get("percent").and_then(Value::as_f64),
            currency: currency.unwrap_or_else(|| "USD".into()),
        });
    }
    let ex = raw.get("extra_usage").filter(|v| v.is_object())?;
    let scale = 10f64.powi(ex.get("decimal_places").and_then(Value::as_i64).unwrap_or(2) as i32);
    Some(Extra {
        enabled: ex.get("is_enabled").and_then(Value::as_bool).unwrap_or(false),
        used: ex.get("used_credits").and_then(Value::as_f64).map(|v| v / scale),
        limit: ex.get("monthly_limit").and_then(Value::as_f64).map(|v| v / scale),
        utilization: ex.get("utilization").and_then(Value::as_f64),
        currency: ex.get("currency").and_then(Value::as_str).unwrap_or("USD").to_string(),
    })
}

/// Normalises a raw `/api/oauth/usage` response.
pub fn normalize(raw: &Value) -> Snapshot {
    let mut limits: Vec<Limit> = vec![];
    if let Some(obj) = raw.as_object() {
        for (key, val) in obj {
            if (key == "five_hour" || key.starts_with("seven_day")) && val.is_object() {
                if let Some(l) = window(key, val) {
                    limits.push(l);
                }
            }
        }
    }
    // Per-model weekly caps (e.g. "Fable") only appear in `limits`.
    for l in raw.get("limits").and_then(Value::as_array).into_iter().flatten() {
        if l.get("kind").and_then(Value::as_str) != Some("weekly_scoped") {
            continue;
        }
        let Some(name) = l.pointer("/scope/model/display_name").and_then(Value::as_str) else { continue };
        let key = format!("scoped_{}", name.replace(' ', "_"));
        if limits.iter().any(|w| w.key == key || w.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name))) {
            continue;
        }
        limits.push(Limit::percent(
            &key,
            "model",
            Some(name.to_string()),
            Some(7 * D),
            l.get("percent").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 100.0),
            l.get("resets_at").and_then(parse_rfc3339).map(round_reset),
        ));
    }
    let rank = |l: &Limit| match l.kind.as_str() {
        "session" => 0,
        "week" => 1,
        _ => 2,
    };
    limits.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.key.cmp(&b.key)));
    let breakdown = raw
        .pointer("/seven_day_breakdown/rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| Some(Share { name: r.get("display_name")?.as_str()?.to_string(), percent: r.get("percent")?.as_f64()? }))
        .filter(|s| s.percent > 0.0)
        .collect();
    Snapshot { limits, extra: extra(raw), breakdown }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "five_hour": { "utilization": 9.0, "resets_at": "2026-11-02T17:59:59.643692+00:00" },
            "seven_day": { "utilization": 3.0, "resets_at": "2026-11-05T05:59:59.643714+00:00" },
            "seven_day_opus": null,
            "seven_day_sonnet": { "utilization": 12.0, "resets_at": "2026-11-05T06:00:00.987+00:00" },
            "limits": [
                { "kind": "session", "percent": 9, "resets_at": "2026-11-02T17:59:59.643692+00:00" },
                { "kind": "weekly_scoped", "percent": 4, "resets_at": "2026-11-05T06:00:00+00:00",
                  "scope": { "model": { "display_name": "Fable" } } }
            ],
            "spend": {
                "enabled": true, "percent": 0,
                "limit": { "amount_minor": 4000, "currency": "EUR", "exponent": 2 },
                "used": { "amount_minor": 1240, "currency": "EUR", "exponent": 2 }
            },
            "seven_day_breakdown": { "rows": [
                { "display_name": "Claude Code", "percent": 99 },
                { "display_name": "Chats", "percent": 0 },
                { "display_name": "Cowork", "percent": 1 }
            ] }
        })
    }

    #[test]
    fn normalizes_windows_and_rounds_resets() {
        let s = normalize(&sample());
        let keys: Vec<&str> = s.limits.iter().map(|l| l.key.as_str()).collect();
        assert_eq!(keys, ["five_hour", "seven_day", "scoped_Fable", "seven_day_sonnet"]);
        let five = &s.limits[0];
        assert_eq!(five.kind, "session");
        assert_eq!(five.resets_at, Some(chrono::DateTime::parse_from_rfc3339("2026-11-02T18:00:00Z").unwrap().timestamp_millis()));
        assert_eq!(s.limits[3].name.as_deref(), Some("Sonnet"));
    }

    #[test]
    fn spend_is_in_major_units() {
        let e = normalize(&sample()).extra.unwrap();
        assert!(e.enabled);
        assert_eq!(e.limit, Some(40.0));
        assert_eq!(e.used, Some(12.4));
        assert_eq!(e.currency, "EUR");
    }

    #[test]
    fn legacy_extra_usage_uses_decimal_places() {
        let e = normalize(&json!({ "extra_usage": { "is_enabled": true, "monthly_limit": 5000, "used_credits": 250, "decimal_places": 2, "currency": "USD" } }))
            .extra
            .unwrap();
        assert_eq!(e.limit, Some(50.0));
        assert_eq!(e.used, Some(2.5));
    }

    #[test]
    fn breakdown_skips_zero_rows() {
        let b = normalize(&sample()).breakdown;
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].name, "Claude Code");
    }

    #[test]
    fn plan_labels() {
        assert_eq!(plan_label(Some(&json!({ "organization": { "plan_display_name": "Ultra" } }))), "Ultra");
        assert_eq!(
            plan_label(Some(
                &json!({ "organization": { "organization_type": "claude_team", "rate_limit_tier": "default_claude_max_5x" } })
            )),
            "Team 5x"
        );
        assert_eq!(plan_label(Some(&json!({ "account": { "has_claude_pro": true }, "organization": {} }))), "Pro");
    }
}
