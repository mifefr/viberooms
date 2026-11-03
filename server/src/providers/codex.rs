//! ChatGPT subscriptions (Plus, Pro, Team, Business…) through the limits used by OpenAI Codex.
//!
//! Experimental. Uses Codex's OAuth client and the `wham/usage` endpoint Codex reads; neither is a
//! public API (facts checked in openai/codex sources, codex-rs, Nov 2026). VibeRooms signs in on its own:
//! Codex refresh tokens rotate with reuse detection, so copying `~/.codex/auth.json` would log the
//! user's Codex CLI out.

use super::{round_reset, Identity, Limit, Login, Pkce, Snapshot, TokenSet};
use crate::http::{ApiError, Http};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPES: &str = "openid profile email offline_access";
const AUTH_CLAIMS: &str = "https://api.openai.com/auth";

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
fn auth_base() -> String {
    env_or("VR_CODEX_AUTH_BASE", "https://auth.openai.com")
}
fn usage_url() -> String {
    env_or("VR_CODEX_USAGE_URL", "https://chatgpt.com/backend-api/wham/usage")
}

pub fn authorize_url(p: &Pkce) -> String {
    let mut u = url::Url::parse(&format!("{}/oauth/authorize", auth_base())).expect("VR_CODEX_AUTH_BASE");
    u.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", &p.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", &p.state)
        .append_pair("originator", "codex_cli_rs");
    u.into()
}

/// Claims of a JWT (payload only, not verified: the token comes straight from the issuer over TLS).
pub fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Extracts `code` and `state` from what the user pasted: the full redirect URL, a query string or a bare code.
pub fn parse_pasted(pasted: &str) -> (Option<String>, Option<String>) {
    let p = pasted.trim();
    let query = match url::Url::parse(p) {
        Ok(u) => u.query().unwrap_or("").to_string(),
        Err(_) if p.contains("code=") => p.trim_start_matches('?').to_string(),
        Err(_) => return (Some(p.to_string()).filter(|s| !s.is_empty()), None),
    };
    let mut code = None;
    let mut state = None;
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            _ => {}
        }
    }
    (code, state)
}

/// Token set + metadata (account id, plan, e-mail) from a token response.
fn token_set(r: &Value, previous: Option<&TokenSet>) -> Result<TokenSet, ApiError> {
    let access = r.get("access_token").and_then(Value::as_str).map(str::to_owned).or_else(|| previous.map(|p| p.access_token.clone()));
    let access = access.ok_or_else(|| ApiError::new(500, "token response without access_token"))?;
    let refresh =
        r.get("refresh_token").and_then(Value::as_str).map(str::to_owned).or_else(|| previous.and_then(|p| p.refresh_token.clone()));
    let exp = jwt_claims(&access).and_then(|c| c.get("exp").and_then(Value::as_i64)).map(|s| s * 1000);
    let expires_at = exp
        .or_else(|| r.get("expires_in").and_then(Value::as_i64).map(|s| crate::now_ms() + s * 1000))
        .unwrap_or(crate::now_ms() + 3_600_000);
    let meta = match r.get("id_token").and_then(Value::as_str).and_then(jwt_claims) {
        Some(c) => {
            let auth = c.get(AUTH_CLAIMS).cloned().unwrap_or(Value::Null);
            Some(json!({
                "email": c.get("email"),
                "account_id": auth.get("chatgpt_account_id"),
                "user_id": auth.get("chatgpt_user_id").or_else(|| auth.get("user_id")).or_else(|| c.get("sub")),
                "plan_type": auth.get("chatgpt_plan_type"),
            }))
        }
        None => previous.and_then(|p| p.meta.clone()),
    };
    Ok(TokenSet { access_token: access, refresh_token: refresh, expires_at, scope: Some(SCOPES.into()), meta })
}

pub async fn exchange(http: &Http, pasted: &str, pkce: &Pkce) -> Result<Login, ApiError> {
    let (code, state) = parse_pasted(pasted);
    let code = code.ok_or_else(|| ApiError::new(400, "no `code` found in what was pasted"))?;
    if state.as_deref().is_some_and(|s| s != pkce.state) {
        return Err(ApiError::new(400, "this URL does not belong to the sign-in link that was opened"));
    }
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", &code)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("client_id", CLIENT_ID)
        .append_pair("code_verifier", &pkce.verifier)
        .finish();
    let req =
        http.inner.post(format!("{}/oauth/token", auth_base())).header("Content-Type", "application/x-www-form-urlencoded").body(form);
    let r = http.json(req).await?;
    let tokens = token_set(&r, None)?;
    let meta = tokens.meta.clone().unwrap_or(Value::Null);
    if meta.get("account_id").is_none_or(Value::is_null) {
        return Err(ApiError::new(400, "this login has no ChatGPT workspace (is it a ChatGPT account?)"));
    }
    Ok(Login { identity: identity(&meta), profile: Some(meta), tokens })
}

pub async fn refresh(http: &Http, current: &TokenSet) -> Result<TokenSet, ApiError> {
    let rt = current.refresh_token.as_deref().ok_or_else(|| ApiError::new(401, "no refresh token"))?;
    let body = json!({ "client_id": CLIENT_ID, "grant_type": "refresh_token", "refresh_token": rt, "scope": "openid profile email" });
    let r = http.json(http.inner.post(format!("{}/oauth/token", auth_base())).json(&body)).await?;
    token_set(&r, Some(current))
}

pub async fn fetch(http: &Http, tokens: &TokenSet) -> Result<(Snapshot, Value), ApiError> {
    let account = tokens.meta.as_ref().and_then(|m| m.get("account_id")).and_then(Value::as_str).unwrap_or_default();
    let req = http.inner.get(usage_url()).bearer_auth(&tokens.access_token).header("chatgpt-account-id", account);
    let raw = http.json(req).await?;
    Ok((normalize(&raw), raw))
}

/// No profile endpoint is needed: the identity comes from the id_token claims kept at sign-in.
pub fn profile(tokens: &TokenSet) -> Value {
    tokens.meta.clone().unwrap_or_else(|| json!({}))
}

pub fn identity(meta: &Value) -> Identity {
    let g = |k: &str| meta.get(k).and_then(Value::as_str).map(str::to_owned);
    Identity { email: g("email"), account_uuid: g("user_id"), org_uuid: g("account_id"), org_name: None }
}

pub fn plan_label(profile: Option<&Value>) -> String {
    let plan = profile.and_then(|p| p.get("plan_type")).and_then(Value::as_str).unwrap_or("");
    match plan {
        "" | "unknown" => String::new(),
        "prolite" => "Pro Lite".into(),
        "promax" => "Pro Max".into(),
        p => {
            let mut c = p.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

fn kind_for(secs: i64) -> &'static str {
    const H: i64 = 3600;
    if secs < 20 * H {
        "session"
    } else if secs <= 2 * 24 * H {
        "day"
    } else if secs >= 6 * 24 * H {
        "week"
    } else {
        "other"
    }
}

fn window(key: String, w: &Value, model: Option<&str>, now: i64) -> Option<Limit> {
    let used = w.get("used_percent").and_then(Value::as_f64)?;
    let secs =
        w.get("limit_window_seconds").and_then(Value::as_i64).or_else(|| w.get("window_minutes").and_then(Value::as_i64).map(|m| m * 60));
    let reset =
        w.get("reset_at").or_else(|| w.get("resets_at")).and_then(Value::as_i64).map(|s| s * 1000).or_else(|| {
            w.get("reset_after_seconds").or_else(|| w.get("resets_in_seconds")).and_then(Value::as_i64).map(|s| now + s * 1000)
        });
    let kind = match (model, secs) {
        (Some(_), _) => "model",
        (None, Some(s)) => kind_for(s),
        (None, None) => "other",
    };
    Some(Limit {
        key,
        kind: kind.into(),
        name: model.map(str::to_owned),
        window_ms: secs.map(|s| s * 1000),
        utilization: used.clamp(0.0, 100.0),
        resets_at: reset.map(round_reset),
    })
}

/// Normalises a `wham/usage` response.
pub fn normalize(raw: &Value) -> Snapshot {
    let now = crate::now_ms();
    let mut limits = vec![];
    if let Some(rl) = raw.get("rate_limit") {
        for (key, field) in [("primary", "primary_window"), ("secondary", "secondary_window")] {
            if let Some(l) = rl.get(field).filter(|v| v.is_object()).and_then(|w| window(key.into(), w, None, now)) {
                limits.push(l);
            }
        }
    }
    for extra in raw.get("additional_rate_limits").and_then(Value::as_array).into_iter().flatten() {
        let name = extra
            .get("limit_name")
            .and_then(Value::as_str)
            .or_else(|| extra.get("metered_feature").and_then(Value::as_str))
            .unwrap_or("model");
        let Some(rl) = extra.get("rate_limit") else { continue };
        for (suffix, field) in [("primary", "primary_window"), ("secondary", "secondary_window")] {
            if let Some(l) = rl.get(field).filter(|v| v.is_object()).and_then(|w| window(format!("{name}:{suffix}"), w, Some(name), now)) {
                limits.push(l);
            }
        }
    }
    Snapshot { limits, extra: None, breakdown: vec![] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: Value) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    #[test]
    fn decodes_jwt_claims() {
        let c = jwt_claims(&jwt(json!({ "exp": 1_900_000_000, "email": "a@b.c" }))).unwrap();
        assert_eq!(c["email"], "a@b.c");
        assert_eq!(jwt_claims("not-a-jwt"), None);
    }

    #[test]
    fn parses_pasted_redirects() {
        assert_eq!(parse_pasted("http://localhost:1455/auth/callback?code=abc&state=xyz"), (Some("abc".into()), Some("xyz".into())));
        assert_eq!(parse_pasted("code=abc&state=xyz"), (Some("abc".into()), Some("xyz".into())));
        assert_eq!(parse_pasted("  abc  "), (Some("abc".into()), None));
    }

    #[test]
    fn token_set_reads_account_and_plan() {
        let id = jwt(
            json!({ "email": "me@x.io", AUTH_CLAIMS: { "chatgpt_account_id": "acc-1", "chatgpt_plan_type": "pro", "chatgpt_user_id": "u-1" } }),
        );
        let access = jwt(json!({ "exp": 2_000_000_000 }));
        let t = token_set(&json!({ "access_token": access, "refresh_token": "r1", "id_token": id }), None).unwrap();
        assert_eq!(t.expires_at, 2_000_000_000_000);
        let m = t.meta.unwrap();
        assert_eq!(m["account_id"], "acc-1");
        assert_eq!(plan_label(Some(&m)), "Pro");
        assert_eq!(identity(&m).email.as_deref(), Some("me@x.io"));
    }

    #[test]
    fn refresh_keeps_previous_values_when_omitted() {
        let prev = TokenSet {
            access_token: "a0".into(),
            refresh_token: Some("r0".into()),
            expires_at: 0,
            scope: None,
            meta: Some(json!({ "account_id": "acc" })),
        };
        let t = token_set(&json!({ "access_token": jwt(json!({ "exp": 1 })) }), Some(&prev)).unwrap();
        assert_eq!(t.refresh_token.as_deref(), Some("r0"));
        assert_eq!(t.meta.unwrap()["account_id"], "acc");
    }

    #[test]
    fn normalizes_usage() {
        let raw = json!({
            "plan_type": "plus",
            "rate_limit": {
                "primary_window": { "used_percent": 15, "limit_window_seconds": 18000, "reset_after_seconds": 3600, "reset_at": 1_767_225_600 },
                "secondary_window": { "used_percent": 5, "limit_window_seconds": 604800, "reset_at": 1_767_600_000 }
            },
            "additional_rate_limits": [
                { "limit_name": "GPT-5 Pro", "rate_limit": { "primary_window": { "used_percent": 40, "limit_window_seconds": 604800, "reset_at": 1_767_600_000 } } }
            ]
        });
        let s = normalize(&raw);
        assert_eq!(s.limits.len(), 3);
        assert_eq!((s.limits[0].kind.as_str(), s.limits[0].window_ms), ("session", Some(18_000_000)));
        assert_eq!(s.limits[0].resets_at, Some(1_767_225_600_000));
        assert_eq!(s.limits[1].kind, "week");
        assert_eq!((s.limits[2].kind.as_str(), s.limits[2].name.as_deref()), ("model", Some("GPT-5 Pro")));
    }
}
