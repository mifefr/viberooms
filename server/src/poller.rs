//! Collection loop: one task per account, OAuth refresh serialised per account, backoff on errors.

use crate::http::ApiError;
use crate::providers::{Provider, TokenSet};
use crate::store::{Account, Tokens};
use crate::{now_ms, App};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{info, warn};

pub fn poll_interval() -> Duration {
    Duration::from_millis(std::env::var("VR_POLL_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(180_000))
}
const PROFILE_EVERY_MS: i64 = 6 * 3_600_000;
const REFRESH_MARGIN_MS: i64 = 5 * 60_000;
const MAX_BACKOFF: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Status {
    #[default]
    Pending,
    Ok,
    Error,
    Reauth,
}

#[derive(Clone, Default, Debug)]
pub struct Runtime {
    pub status: Status,
    pub last_error: Option<String>,
    pub fetched_at: Option<i64>,
    pub next_poll_at: Option<i64>,
    pub last_attempt: Option<i64>,
    pub failures: u32,
    pub profile_at: Option<i64>,
    pub busy: bool,
}

enum PollError {
    Reauth(String),
    Api(ApiError),
}

pub enum Outcome {
    Next(Duration),
    Reauth,
    Gone,
}

fn to_set(t: &Tokens) -> TokenSet {
    TokenSet {
        access_token: t.access_token.clone(),
        refresh_token: t.refresh_token.clone(),
        expires_at: t.expires_at,
        scope: t.scope.clone(),
        meta: t.meta.clone(),
    }
}

fn provider_of(acc: &Account) -> Result<Provider, PollError> {
    Provider::from_id(&acc.provider).ok_or_else(|| PollError::Reauth(format!("unknown provider `{}`", acc.provider)))
}

async fn ensure_fresh(app: &App, id: &str, force: bool) -> Result<(Provider, TokenSet), PollError> {
    let acc = app.store.get(id).ok_or_else(|| PollError::Reauth("account removed".into()))?;
    let provider = provider_of(&acc)?;
    let t = to_set(&acc.tokens);
    if !force && t.expires_at - now_ms() > REFRESH_MARGIN_MS {
        return Ok((provider, t));
    }
    if t.refresh_token.is_none() {
        return Err(PollError::Reauth("no refresh token".into()));
    }
    match provider.refresh(&app.http, &t).await {
        Ok(fresh) => {
            // Persist the rotated refresh token before using the new access token.
            let saved = app.store.update(id, |a| {
                a.tokens.access_token = fresh.access_token.clone();
                a.tokens.refresh_token = fresh.refresh_token.clone();
                a.tokens.expires_at = fresh.expires_at;
                if fresh.scope.is_some() {
                    a.tokens.scope = fresh.scope.clone();
                }
                if fresh.meta.is_some() {
                    a.tokens.meta = fresh.meta.clone();
                }
            });
            if let Err(e) = saved {
                warn!(account = %acc.label, "FAILED to persist refreshed tokens: {e:#}");
            }
            info!(account = %acc.label, "token refreshed (expires in {} min)", (fresh.expires_at - now_ms()) / 60_000);
            Ok((provider, TokenSet { meta: fresh.meta.clone().or(t.meta), ..fresh }))
        }
        Err(e) if e.is_grant_dead() => Err(PollError::Reauth(format!("refresh refused: {e}"))),
        Err(e) => Err(PollError::Api(e)),
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Usage,
    Profile,
}

enum Fetched {
    Usage(crate::providers::Snapshot, Value),
    Profile(Value),
}

async fn call(app: &App, p: Provider, kind: Kind, t: &TokenSet) -> Result<Fetched, ApiError> {
    match kind {
        Kind::Usage => p.fetch(&app.http, t).await.map(|(s, r)| Fetched::Usage(s, r)),
        Kind::Profile => p.profile(&app.http, t).await.map(Fetched::Profile),
    }
}

async fn authed(app: &App, id: &str, kind: Kind) -> Result<Fetched, PollError> {
    let (p, t) = ensure_fresh(app, id, false).await?;
    match call(app, p, kind, &t).await {
        Ok(v) => Ok(v),
        Err(e) if e.is_unauthorized() => {
            let (p, t) = ensure_fresh(app, id, true).await?;
            call(app, p, kind, &t).await.map_err(|e| if e.is_unauthorized() { PollError::Reauth(e.message) } else { PollError::Api(e) })
        }
        Err(e) => Err(PollError::Api(e)),
    }
}

/// One full collection for an account, serialised by the account lock.
pub async fn poll_once(app: &App, id: &str) -> Outcome {
    let lock = app.lock_for(id);
    let _guard = lock.lock().await;
    let Some(acc) = app.store.get(id) else { return Outcome::Gone };
    let profile_due = app.with_rt(id, |r| {
        r.busy = true;
        r.last_attempt = Some(now_ms());
        r.profile_at.is_none_or(|t| now_ms() - t > PROFILE_EVERY_MS)
    });

    let result: Result<(), PollError> = async {
        if profile_due {
            match authed(app, id, Kind::Profile).await {
                Ok(Fetched::Profile(profile)) => {
                    let ident = Provider::from_id(&acc.provider).map(|p| p.identity(&profile)).unwrap_or_default();
                    let _ = app.store.update(id, |a| {
                        if ident.email.is_some() {
                            a.email = ident.email.clone();
                        }
                        if ident.org_name.is_some() {
                            a.org_name = ident.org_name.clone();
                        }
                        a.profile = Some(profile);
                    });
                    app.with_rt(id, |r| r.profile_at = Some(now_ms()));
                }
                Ok(_) => {}
                Err(PollError::Reauth(m)) => return Err(PollError::Reauth(m)),
                Err(PollError::Api(e)) => warn!(account = %acc.label, "profile unavailable: {e}"),
            }
        }
        let Fetched::Usage(snapshot, raw) = authed(app, id, Kind::Usage).await? else { unreachable!() };
        let now = now_ms();
        let points: Vec<_> = snapshot.limits.iter().map(|l| (l.key.clone(), l.utilization, l.resets_at)).collect();
        let snap_json = serde_json::to_value(&snapshot).unwrap_or(Value::Null);
        if let Err(e) = app.store.record_usage(id, now, &snap_json, &raw, &points) {
            warn!(account = %acc.label, "history write failed: {e:#}");
        }
        app.with_rt(id, |r| {
            r.status = Status::Ok;
            r.last_error = None;
            r.fetched_at = Some(now);
            r.failures = 0;
        });
        Ok(())
    }
    .await;

    app.with_rt(id, |r| r.busy = false);
    match result {
        Ok(()) => Outcome::Next(poll_interval()),
        Err(PollError::Reauth(m)) => {
            warn!(account = %acc.label, "sign-in required: {m}");
            app.with_rt(id, |r| {
                r.status = Status::Reauth;
                r.last_error = Some(m);
                r.failures += 1;
            });
            Outcome::Reauth
        }
        Err(PollError::Api(e)) => {
            let failures = app.with_rt(id, |r| {
                r.status = Status::Error;
                r.last_error = Some(e.message.clone());
                r.failures += 1;
                r.failures
            });
            let backoff = e
                .retry_after
                .unwrap_or_else(|| Duration::from_secs(60 * 2u64.pow(failures.min(5))))
                .clamp(Duration::from_secs(30), MAX_BACKOFF);
            warn!(account = %acc.label, "failure #{failures}: {} → retry in {} s", e.message, backoff.as_secs());
            Outcome::Next(backoff)
        }
    }
}

/// Life loop of an account. `waker` forces an immediate collection (after a reconnection).
pub async fn account_loop(app: Arc<App>, id: String, waker: Arc<Notify>, first: Duration) {
    let mut delay = Some(first);
    loop {
        let forced = match delay {
            Some(d) => {
                app.with_rt(&id, |r| r.next_poll_at = Some(now_ms() + d.as_millis() as i64));
                tokio::select! {
                    _ = tokio::time::sleep(d) => false,
                    _ = waker.notified() => true,
                }
            }
            None => {
                app.with_rt(&id, |r| r.next_poll_at = None);
                waker.notified().await;
                true
            }
        };
        if app.store.get(&id).is_none() {
            break;
        }
        // A manual refresh just happened: re-align the timer instead of calling twice.
        if !forced {
            let interval = poll_interval().as_millis() as i64;
            let since = app.with_rt(&id, |r| r.last_attempt.map(|t| now_ms() - t));
            let ok = app.with_rt(&id, |r| r.status == Status::Ok);
            if let (Some(since), true) = (since, ok) {
                if since < interval / 2 {
                    delay = Some(Duration::from_millis((interval - since) as u64));
                    continue;
                }
            }
        }
        delay = match poll_once(&app, &id).await {
            Outcome::Next(d) => Some(d),
            Outcome::Reauth => None,
            Outcome::Gone => break,
        };
    }
}
