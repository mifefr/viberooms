//! Provider connectors.
//!
//! Three families:
//! - OAuth connectors written in Rust (`claude`, `codex`): sign-in with PKCE, token refresh, fetch;
//! - API-key presets described in `presets.json` and run by the declarative engine;
//! - the `custom` connector: a declarative spec written by the user in the dashboard.
//!
//! Every connector produces a normalised [`Snapshot`]; everything else (history, pace, projections,
//! dashboard, Homepage, menu bar) is provider-agnostic.

pub mod claude;
pub mod codex;
pub mod declarative;

use crate::http::{ApiError, Http};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

fn yes() -> bool {
    true
}

/// One usage limit, normalised across providers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    /// Stable identifier within the provider (e.g. `five_hour`, `seven_day`, `credits`).
    pub key: String,
    /// `session` (short rolling window), `day`, `week`, `month`, `balance` (prepaid, no reset),
    /// `model` (per-model cap) or `other`.
    pub kind: String,
    /// Display name for per-model or custom limits (e.g. "Opus", "Credits").
    pub name: Option<String>,
    /// Window length, used to compute pace and projections.
    pub window_ms: Option<i64>,
    /// Percentage used, 0–100 (meaningful only when `has_percent`).
    pub utilization: f64,
    /// When the window resets (epoch ms), rounded to the minute.
    pub resets_at: Option<i64>,
    /// Absolute amounts, in `unit`, when the provider reports them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// False for a bare balance (no known ceiling): no percentage, no gauge.
    #[serde(default = "yes")]
    pub has_percent: bool,
}

impl Limit {
    pub fn percent(key: &str, kind: &str, name: Option<String>, window_ms: Option<i64>, utilization: f64, resets_at: Option<i64>) -> Self {
        Limit {
            key: key.into(),
            kind: kind.into(),
            name,
            window_ms,
            utilization,
            resets_at,
            used: None,
            limit: None,
            remaining: None,
            unit: None,
            has_percent: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Extra {
    pub enabled: bool,
    /// Amounts in major currency units.
    pub used: Option<f64>,
    pub limit: Option<f64>,
    pub utilization: Option<f64>,
    pub currency: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Share {
    pub name: String,
    pub percent: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub limits: Vec<Limit>,
    pub extra: Option<Extra>,
    /// Share of weekly usage per surface (Claude Code, chat…), when the provider reports it.
    pub breakdown: Vec<Share>,
}

#[derive(Clone, Debug, Default)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    pub scope: Option<String>,
    /// Provider-specific extras (account id header, API-key form fields, custom connector spec…).
    pub meta: Option<Value>,
}

#[derive(Clone, Debug, Default)]
pub struct Identity {
    pub email: Option<String>,
    pub account_uuid: Option<String>,
    pub org_uuid: Option<String>,
    pub org_name: Option<String>,
}

pub struct Login {
    pub tokens: TokenSet,
    pub identity: Identity,
    pub profile: Option<Value>,
}

/// How the user hands the OAuth result back to the dashboard.
#[derive(Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub enum LoginMode {
    /// The provider shows a code (`code#state`) to copy.
    PasteCode,
    /// The provider redirects to a localhost URL: the user pastes that URL.
    PasteRedirectUrl,
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

fn random_b64(n: usize) -> String {
    let mut buf = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn new_pkce() -> Pkce {
    let verifier = random_b64(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Pkce { verifier, challenge, state: random_b64(32) }
}

/// The API returns the same reset time with ±1–2 s of jitter between calls; round to the minute so a
/// window keeps a single identity (otherwise every poll looks like a new cycle).
pub fn round_reset(ms: i64) -> i64 {
    (ms + 30_000).div_euclid(60_000) * 60_000
}

/// API keys do not expire on our side: a far-future expiry keeps the refresh logic out of the way.
pub const NEVER: i64 = 4_102_444_800_000; // 2100-01-01

/// Short, non-reversible fingerprint of a secret (to recognise the same key twice).
pub fn fingerprint(secret: &str) -> String {
    Sha256::digest(secret.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
}

// ---------- Presets ----------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct I18n {
    pub en: String,
    pub fr: String,
}

/// A form field of an API-key connector. The first `secret` field is the key.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub id: String,
    pub label: I18n,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub placeholder: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    /// Grouping on the dashboard (e.g. "OpenRouter", "API").
    pub tool: String,
    /// `subscription` or `api`.
    pub category: String,
    #[serde(default)]
    pub experimental: bool,
    pub description: I18n,
    /// Where the user creates the key.
    #[serde(default)]
    pub key_url: Option<String>,
    #[serde(default)]
    pub docs_url: Option<String>,
    pub fields: Vec<Field>,
    pub spec: declarative::Spec,
}

/// Built-in presets, or the file named by `VR_PRESETS_FILE` (to try a new preset without rebuilding).
pub fn presets() -> &'static [Preset] {
    static P: OnceLock<Vec<Preset>> = OnceLock::new();
    P.get_or_init(|| {
        if let Ok(path) = std::env::var("VR_PRESETS_FILE") {
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
            {
                Ok(p) => return p,
                Err(e) => tracing::warn!("VR_PRESETS_FILE {path} ignored: {e}"),
            }
        }
        serde_json::from_str(include_str!("presets.json")).expect("presets.json is invalid")
    })
}

// ---------- Registry ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    Claude,
    Codex,
    /// Index into [`presets()`].
    Preset(usize),
    Custom,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuthKind {
    Oauth,
    ApiKey,
    Custom,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub tool: String,
    pub category: String,
    pub auth: AuthKind,
    pub login_mode: Option<LoginMode>,
    pub experimental: bool,
    pub description: I18n,
    pub key_url: Option<String>,
    pub docs_url: Option<String>,
    pub fields: Vec<Field>,
}

fn i18n(en: &str, fr: &str) -> I18n {
    I18n { en: en.into(), fr: fr.into() }
}

impl Provider {
    pub fn all() -> Vec<Provider> {
        let mut v = vec![Provider::Claude, Provider::Codex];
        v.extend((0..presets().len()).map(Provider::Preset));
        v.push(Provider::Custom);
        v
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "claude" => Some(Provider::Claude),
            "codex" => Some(Provider::Codex),
            "custom" => Some(Provider::Custom),
            p => presets().iter().position(|x| x.id == p).map(Provider::Preset),
        }
    }

    pub fn id(self) -> String {
        match self {
            Provider::Claude => "claude".into(),
            Provider::Codex => "codex".into(),
            Provider::Preset(i) => presets()[i].id.clone(),
            Provider::Custom => "custom".into(),
        }
    }

    pub fn auth(self) -> AuthKind {
        match self {
            Provider::Claude | Provider::Codex => AuthKind::Oauth,
            Provider::Preset(_) => AuthKind::ApiKey,
            Provider::Custom => AuthKind::Custom,
        }
    }

    pub fn info(self) -> ProviderInfo {
        match self {
            Provider::Claude => ProviderInfo {
                id: "claude".into(),
                name: "Claude".into(),
                tool: "Claude Code".into(),
                category: "subscription".into(),
                auth: AuthKind::Oauth,
                login_mode: Some(LoginMode::PasteCode),
                experimental: false,
                description: i18n(
                    "Pro, Max, Team or Enterprise subscription: 5-hour session, week, per-model caps, extra credits.",
                    "Abonnement Pro, Max, Team ou Enterprise : session 5 h, semaine, limites par modèle, crédits supplémentaires.",
                ),
                key_url: None,
                docs_url: None,
                fields: vec![],
            },
            Provider::Codex => ProviderInfo {
                id: "codex".into(),
                name: "ChatGPT".into(),
                tool: "Codex".into(),
                category: "subscription".into(),
                auth: AuthKind::Oauth,
                login_mode: Some(LoginMode::PasteRedirectUrl),
                experimental: true,
                description: i18n(
                    "Plus, Pro, Team or Business subscription, as used by Codex: 5-hour window, week, per-model limits.",
                    "Abonnement Plus, Pro, Team ou Business, tel qu'utilisé par Codex : fenêtre 5 h, semaine, limites par modèle.",
                ),
                key_url: None,
                docs_url: None,
                fields: vec![],
            },
            Provider::Preset(i) => {
                let p = &presets()[i];
                ProviderInfo {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    tool: p.tool.clone(),
                    category: p.category.clone(),
                    auth: AuthKind::ApiKey,
                    login_mode: None,
                    experimental: p.experimental,
                    description: p.description.clone(),
                    key_url: p.key_url.clone(),
                    docs_url: p.docs_url.clone(),
                    fields: p.fields.clone(),
                }
            }
            Provider::Custom => ProviderInfo {
                id: "custom".into(),
                name: "Custom".into(),
                tool: "Custom".into(),
                category: "custom".into(),
                auth: AuthKind::Custom,
                login_mode: None,
                experimental: true,
                description: i18n(
                    "Any service that exposes its quota, credits or balance over HTTP: describe the call and the JSON fields to read.",
                    "Tout service qui expose ses quotas, crédits ou solde en HTTP : décris l'appel et les champs JSON à lire.",
                ),
                key_url: None,
                docs_url: None,
                fields: vec![],
            },
        }
    }

    pub fn authorize_url(self, pkce: &Pkce) -> Option<String> {
        match self {
            Provider::Claude => Some(claude::authorize_url(pkce)),
            Provider::Codex => Some(codex::authorize_url(pkce)),
            _ => None,
        }
    }

    /// Accepts what the user pasted (a code or a redirect URL) and exchanges it for tokens.
    pub async fn exchange(self, http: &Http, pasted: &str, pkce: &Pkce) -> Result<Login, ApiError> {
        match self {
            Provider::Claude => claude::exchange(http, pasted, pkce).await,
            Provider::Codex => codex::exchange(http, pasted, pkce).await,
            _ => Err(ApiError::new(400, "this provider does not use OAuth")),
        }
    }

    pub async fn refresh(self, http: &Http, tokens: &TokenSet) -> Result<TokenSet, ApiError> {
        match self {
            Provider::Claude => claude::refresh(http, tokens.refresh_token.as_deref().unwrap_or_default()).await,
            Provider::Codex => codex::refresh(http, tokens).await,
            // An API key cannot be refreshed: if it is refused, the user has to replace it.
            Provider::Preset(_) | Provider::Custom => Ok(tokens.clone()),
        }
    }

    /// The declarative spec used by a preset or a custom account.
    pub fn spec(self, tokens: &TokenSet) -> Option<declarative::Spec> {
        match self {
            Provider::Preset(i) => Some(presets()[i].spec.clone()),
            Provider::Custom => tokens.meta.as_ref().and_then(|m| m.get("connector")).and_then(|c| serde_json::from_value(c.clone()).ok()),
            _ => None,
        }
    }

    /// Current limits. Returns the normalised snapshot and the raw response (kept for diagnostics).
    pub async fn fetch(self, http: &Http, tokens: &TokenSet) -> Result<(Snapshot, Value), ApiError> {
        match self {
            Provider::Claude => claude::fetch(http, tokens).await,
            Provider::Codex => codex::fetch(http, tokens).await,
            Provider::Preset(_) | Provider::Custom => {
                let spec = self.spec(tokens).ok_or_else(|| ApiError::new(400, "missing connector description"))?;
                declarative::fetch(http, &spec, tokens).await
            }
        }
    }

    pub async fn profile(self, http: &Http, tokens: &TokenSet) -> Result<Value, ApiError> {
        match self {
            Provider::Claude => claude::profile(http, tokens).await,
            Provider::Codex => Ok(codex::profile(tokens)),
            Provider::Preset(_) | Provider::Custom => Ok(tokens.meta.clone().unwrap_or_else(|| json!({}))),
        }
    }

    pub fn identity(self, profile: &Value) -> Identity {
        match self {
            Provider::Claude => claude::identity(profile),
            Provider::Codex => codex::identity(profile),
            Provider::Preset(_) | Provider::Custom => Identity {
                email: profile.get("identity").and_then(Value::as_str).map(str::to_owned),
                account_uuid: profile.get("fingerprint").and_then(Value::as_str).map(str::to_owned),
                org_uuid: None,
                org_name: None,
            },
        }
    }

    pub fn plan_label(self, profile: Option<&Value>) -> String {
        match self {
            Provider::Claude => claude::plan_label(profile),
            Provider::Codex => codex::plan_label(profile),
            // "individual_pro" → "Individual Pro"
            Provider::Preset(_) | Provider::Custom => profile
                .and_then(|p| p.get("plan"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .split(['_', '-', ' '])
                .filter(|w| !w.is_empty())
                .map(|w| {
                    let mut c = w.chars();
                    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// Rebuilds a snapshot from a stored raw response (data imported from older versions).
    pub fn normalize(self, raw: &Value) -> Snapshot {
        match self {
            Provider::Claude => claude::normalize(raw),
            Provider::Codex => codex::normalize(raw),
            Provider::Preset(i) => declarative::extract(&presets()[i].spec, raw, crate::now_ms()),
            Provider::Custom => Snapshot::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_parse_and_have_a_secret_field() {
        assert!(!presets().is_empty());
        for p in presets() {
            assert!(p.fields.iter().any(|f| f.secret), "{} has no secret field", p.id);
            assert!(Provider::from_id(&p.id).is_some());
            assert!(!p.spec.limits.is_empty(), "{} has no limits", p.id);
        }
    }

    /// Extracts a preset against a sample answer taken from the provider's documentation or source.
    fn run(id: &str, raw: Value, fields: Value) -> Vec<Limit> {
        let p = presets().iter().find(|p| p.id == id).unwrap();
        let now = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 11, 3, 12, 0, 0).unwrap().timestamp_millis();
        declarative::extract_with(&p.spec, &raw, now, &fields).limits
    }
    fn get<'a>(ls: &'a [Limit], key: &str) -> &'a Limit {
        ls.iter().find(|l| l.key == key).unwrap_or_else(|| panic!("missing limit {key}"))
    }

    #[test]
    fn preset_openrouter() {
        let ls = run(
            "openrouter",
            json!({"data":{"label":"k","limit":100,"limit_reset":"monthly","limit_remaining":72.5,"usage":27.5,"usage_daily":1.2,"usage_weekly":6.8,"usage_monthly":27.5}}),
            Value::Null,
        );
        let k = get(&ls, "key_limit");
        assert_eq!((k.used, k.limit, k.remaining), (Some(27.5), Some(100.0), Some(72.5)));
        assert!((k.utilization - 27.5).abs() < 1e-9);
        assert_eq!(get(&ls, "today").used, Some(1.2));
        assert!(!get(&ls, "month").has_percent);
        // No cap on the key: no percentage, only the amount used.
        let ls = run("openrouter", json!({"data":{"limit":null,"limit_remaining":null,"usage":3.0}}), Value::Null);
        assert!(!get(&ls, "key_limit").has_percent);
    }

    #[test]
    fn preset_openrouter_credits_and_deepseek_and_moonshot() {
        let c = &run("openrouter-credits", json!({"data":{"total_credits":100.5,"total_usage":25.75}}), Value::Null)[0];
        assert_eq!(c.remaining, Some(74.75));
        let d = &run(
            "deepseek",
            json!({"is_available":true,"balance_infos":[{"currency":"USD","total_balance":"110.00","granted_balance":"10.00","topped_up_balance":"100.00"}]}),
            Value::Null,
        )[0];
        assert_eq!((d.remaining, d.unit.as_deref()), (Some(110.0), Some("USD")));
        let m =
            &run("moonshot", json!({"code":0,"data":{"available_balance":49.58,"voucher_balance":46.58,"cash_balance":3.0}}), Value::Null)
                [0];
        assert_eq!(m.remaining, Some(49.58));
    }

    #[test]
    fn preset_kimi_code() {
        let ls = run(
            "kimi-code",
            json!({"usage":{"limit":"1000","remaining":"640","resetTime":"2026-11-08T00:00:00Z"},"limits":[{"window":{"duration":300,"timeUnit":"TIME_UNIT_MINUTE"},"detail":{"limit":"100","remaining":"25","resetTime":"2026-11-03T14:00:00Z"}}],"totalQuota":{"limit":"5000","remaining":"4000"}}),
            Value::Null,
        );
        assert_eq!(get(&ls, "five_hour").utilization, 75.0);
        assert_eq!(get(&ls, "week").utilization, 36.0);
        assert_eq!(get(&ls, "month").utilization, 20.0);
        assert_eq!(get(&ls, "five_hour").kind, "session");
    }

    #[test]
    fn preset_zai_coding() {
        let ls = run(
            "zai-coding",
            json!({"code":200,"data":{"level":"pro","limits":[
            {"type":"TOKENS_LIMIT","unit":3,"number":5,"usage":1000000,"currentValue":72000,"remaining":928000,"percentage":7,"nextResetTime":1793736000000_i64},
            {"type":"TOKENS_LIMIT","unit":6,"number":7,"percentage":53,"nextResetTime":1794096000000_i64},
            {"type":"TIME_LIMIT","unit":5,"number":1,"usage":1000,"currentValue":42}]}}),
            Value::Null,
        );
        assert_eq!(get(&ls, "five_hour").utilization, 7.0);
        assert_eq!(get(&ls, "five_hour").resets_at, Some(1793736000000));
        assert_eq!(get(&ls, "week").utilization, 53.0);
        assert!((get(&ls, "tools").utilization - 4.2).abs() < 1e-9);
    }

    #[test]
    fn preset_xai_inverted_cents() {
        let x = &run("xai-api", json!({"changes":[],"total":{"val":"-1000"}}), Value::Null)[0];
        assert_eq!(x.remaining, Some(10.0));
    }

    #[test]
    fn preset_admin_apis() {
        let a = &run(
            "anthropic-api",
            json!({"cost":{"data":[{"results":[{"amount":"12345.6","currency":"USD"}]},{"results":[{"amount":"654.4"}]}]}}),
            json!({"budget":"200"}),
        )[0];
        assert_eq!((a.used, a.limit), (Some(130.0), Some(200.0)));
        assert_eq!(a.utilization, 65.0);
        let o = &run(
            "openai-api",
            json!({"costs":{"data":[{"results":[{"amount":{"value":40.0,"currency":"usd"}}]},{"results":[{"amount":{"value":10.0}}]}]},"limit":{"threshold_amount":10000,"currency":"USD"}}),
            Value::Null,
        )[0];
        assert_eq!((o.used, o.limit, o.utilization), (Some(50.0), Some(100.0), 50.0));
        // No spend limit readable: the typed budget is used.
        let o =
            &run("openai-api", json!({"costs":{"data":[{"results":[{"amount":{"value":30.0}}]}]},"limit":null}), json!({"budget":"60"}))[0];
        assert_eq!(o.utilization, 50.0);
    }

    #[test]
    fn preset_copilot() {
        let ls = run(
            "copilot",
            json!({"copilot_plan":"individual_pro","quota_reset_date":"2026-12-01","quota_snapshots":{"premium_interactions":{"entitlement":300,"remaining":93,"percent_remaining":31.17,"unlimited":false}}}),
            Value::Null,
        );
        let p = get(&ls, "premium");
        assert_eq!((p.limit, p.remaining, p.used), (Some(300.0), Some(93.0), Some(207.0)));
        assert_eq!(p.kind, "month");
        assert!(p.resets_at.is_some());
        // Usage-based seat: premium requests are unlimited, AI credits are tracked instead.
        let ls = run(
            "copilot",
            json!({"token_based_billing":true,"quota_reset_date":"2026-12-01","quota_snapshots":{"premium_interactions":{"entitlement":0,"unlimited":true,"credits_used":120}}}),
            json!({"credits":"300"}),
        );
        assert!(ls.iter().all(|l| l.key != "premium"));
        assert_eq!(get(&ls, "credits").utilization, 40.0);
    }

    #[test]
    fn registry_round_trip() {
        for p in Provider::all() {
            assert_eq!(Provider::from_id(&p.id()), Some(p));
        }
    }

    #[test]
    fn old_snapshots_without_amounts_still_deserialize() {
        let l: Limit = serde_json::from_value(
            json!({ "key": "k", "kind": "week", "name": null, "windowMs": 1, "utilization": 5.0, "resetsAt": null }),
        )
        .unwrap();
        assert!(l.has_percent);
        assert_eq!(l.used, None);
    }
}
