//! Provider connectors.
//!
//! A connector knows how to sign a user in (OAuth PKCE), keep its tokens fresh and fetch the current
//! limits, normalised into a [`Snapshot`]. Everything else (history, pace, projections, dashboard,
//! Homepage, menu bar) is provider-agnostic.

pub mod claude;
pub mod codex;

use crate::http::{ApiError, Http};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// One usage limit, normalised across providers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    /// Stable identifier within the provider (e.g. `five_hour`, `seven_day`, `scoped_Fable`).
    pub key: String,
    /// `session` (short rolling window), `week`, `day`, `model` (per-model cap) or `other`.
    pub kind: String,
    /// Model or scope name for per-model limits (e.g. "Opus", "Fable").
    pub name: Option<String>,
    /// Window length, used to compute pace and projections.
    pub window_ms: Option<i64>,
    /// Percentage used, 0–100.
    pub utilization: f64,
    /// When the window resets (epoch ms), rounded to the minute.
    pub resets_at: Option<i64>,
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
    /// Provider-specific extras (e.g. an account id header value).
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

/// How the user hands the authorization result back to the dashboard.
#[derive(Serialize, Clone, Copy)]
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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub tool: &'static str,
    pub login_mode: LoginMode,
    pub experimental: bool,
}

impl Provider {
    pub const ALL: &'static [Provider] = &[Provider::Claude, Provider::Codex];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|p| p.id() == id)
    }

    pub fn id(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }

    pub fn info(self) -> ProviderInfo {
        match self {
            Provider::Claude => {
                ProviderInfo { id: "claude", name: "Claude", tool: "Claude Code", login_mode: LoginMode::PasteCode, experimental: false }
            }
            Provider::Codex => {
                ProviderInfo { id: "codex", name: "ChatGPT", tool: "Codex", login_mode: LoginMode::PasteRedirectUrl, experimental: true }
            }
        }
    }

    pub fn authorize_url(self, pkce: &Pkce) -> String {
        match self {
            Provider::Claude => claude::authorize_url(pkce),
            Provider::Codex => codex::authorize_url(pkce),
        }
    }

    /// Accepts what the user pasted (a code or a redirect URL) and exchanges it for tokens.
    pub async fn exchange(self, http: &Http, pasted: &str, pkce: &Pkce) -> Result<Login, ApiError> {
        match self {
            Provider::Claude => claude::exchange(http, pasted, pkce).await,
            Provider::Codex => codex::exchange(http, pasted, pkce).await,
        }
    }

    pub async fn refresh(self, http: &Http, tokens: &TokenSet) -> Result<TokenSet, ApiError> {
        match self {
            Provider::Claude => claude::refresh(http, tokens.refresh_token.as_deref().unwrap_or_default()).await,
            Provider::Codex => codex::refresh(http, tokens).await,
        }
    }

    /// Current limits. Returns the normalised snapshot and the raw response (kept for diagnostics).
    pub async fn fetch(self, http: &Http, tokens: &TokenSet) -> Result<(Snapshot, Value), ApiError> {
        match self {
            Provider::Claude => claude::fetch(http, tokens).await,
            Provider::Codex => codex::fetch(http, tokens).await,
        }
    }

    pub async fn profile(self, http: &Http, tokens: &TokenSet) -> Result<Value, ApiError> {
        match self {
            Provider::Claude => claude::profile(http, tokens).await,
            Provider::Codex => Ok(codex::profile(tokens)),
        }
    }

    pub fn identity(self, profile: &Value) -> Identity {
        match self {
            Provider::Claude => claude::identity(profile),
            Provider::Codex => codex::identity(profile),
        }
    }

    pub fn plan_label(self, profile: Option<&Value>) -> String {
        match self {
            Provider::Claude => claude::plan_label(profile),
            Provider::Codex => codex::plan_label(profile),
        }
    }

    /// Rebuilds a snapshot from a stored raw response (data imported from older versions).
    pub fn normalize(self, raw: &Value) -> Snapshot {
        match self {
            Provider::Claude => claude::normalize(raw),
            Provider::Codex => codex::normalize(raw),
        }
    }
}
