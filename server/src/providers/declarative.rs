//! Declarative connectors: one HTTP call described in JSON, values picked with JSON paths.
//!
//! Used by the generic "custom" connector (configured from the dashboard) and by the built-in presets
//! of `presets.json` (OpenRouter, DeepSeek…). A spec looks like:
//!
//! ```json
//! { "name": "OpenRouter", "url": "https://openrouter.ai/api/v1/key",
//!   "headers": { "Authorization": "Bearer {{key}}" },
//!   "limits": [ { "key": "credits", "kind": "balance", "used": "/data/usage", "limit": "/data/limit",
//!                 "remaining": "/data/limit_remaining", "unit": "USD" } ] }
//! ```
//!
//! Paths are JSON pointers (`/data/usage`) where a `*` segment matches every array item or object
//! member and a `[k=v,k2=v2]` segment keeps the array items whose fields match; several matches are
//! summed. Numbers given as strings ("12.34") are accepted.
//! Several calls can be combined with `requests` (each with an `id`): paths then start with that id,
//! e.g. `/costs/data/*/results/*/amount`.
//! Templates: `{{key}}`, `{{field.<id>}}`, `{{now}}`, `{{now_unix}}`, `{{today}}`, `{{month_start}}`,
//! `{{month_start_unix}}` (UTC).

use super::{round_reset, Limit, Snapshot, TokenSet};
use crate::http::{parse_rfc3339, ApiError, Http};
use chrono::{Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    pub name: String,
    /// Single call (ignored when `requests` is set).
    #[serde(default)]
    pub url: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// Several calls, combined into `{ "<id>": <answer>, … }`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requests: Vec<Request>,
    pub limits: Vec<LimitSpec>,
    /// Path to a plan or tier name shown on the card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Path to an account label (e-mail, key name…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
}

fn default_method() -> String {
    "GET".into()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub id: String,
    pub url: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// An optional call may fail (e.g. it needs a permission the key lacks): its answer is then null.
    #[serde(default)]
    pub optional: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LimitSpec {
    pub key: String,
    /// Display name, e.g. "Credits", "Premium requests".
    #[serde(default)]
    pub label: Option<String>,
    /// `session`, `day`, `week`, `month`, `balance` (prepaid, no reset), `model` or `other`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub percent: Option<String>,
    #[serde(default)]
    pub percent_remaining: Option<String>,
    #[serde(default)]
    pub used: Option<String>,
    #[serde(default)]
    pub limit: Option<String>,
    #[serde(default)]
    pub remaining: Option<String>,
    /// Path to the reset time (RFC 3339 string, or unix seconds / milliseconds).
    #[serde(default)]
    pub resets_at: Option<String>,
    /// Calendar reset when the API gives none: `monthly` or `daily` (UTC).
    #[serde(default)]
    pub reset: Option<String>,
    /// Unit of used/limit/remaining: "USD", "EUR", "credits", "requests"…
    #[serde(default)]
    pub unit: Option<String>,
    /// Path to the unit when the answer gives it (e.g. a currency code).
    #[serde(default)]
    pub unit_path: Option<String>,
    /// Multiplier applied to used/limit/remaining (e.g. 0.01 to turn cents into dollars).
    #[serde(default)]
    pub scale: Option<f64>,
    /// Per-value multipliers, overriding `scale`.
    #[serde(default)]
    pub used_scale: Option<f64>,
    #[serde(default)]
    pub limit_scale: Option<f64>,
    #[serde(default)]
    pub remaining_scale: Option<f64>,
    /// Template used when no path gives the value, e.g. `{{field.budget}}` typed by the user.
    #[serde(default)]
    pub limit_value: Option<String>,
    /// Path to a flag (e.g. `unlimited`): when it is true, the limit is not shown.
    #[serde(default)]
    pub skip_if: Option<String>,
    #[serde(default)]
    pub window_ms: Option<i64>,
}

// ---------- Templates ----------

pub struct Vars<'a> {
    pub key: &'a str,
    pub fields: &'a Value,
    pub now: i64,
}

fn month_start(now: i64) -> chrono::DateTime<Utc> {
    let d = Utc.timestamp_millis_opt(now).single().unwrap_or_else(Utc::now);
    Utc.with_ymd_and_hms(d.year(), d.month(), 1, 0, 0, 0).single().unwrap()
}

fn next_month_start(now: i64) -> chrono::DateTime<Utc> {
    let m = month_start(now);
    let (y, mo) = if m.month() == 12 { (m.year() + 1, 1) } else { (m.year(), m.month() + 1) };
    Utc.with_ymd_and_hms(y, mo, 1, 0, 0, 0).single().unwrap()
}

pub fn render(template: &str, v: &Vars) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = rest[start + 2..start + end].trim();
        let now = Utc.timestamp_millis_opt(v.now).single().unwrap_or_else(Utc::now);
        let value = match name {
            "key" => v.key.to_string(),
            "now" => now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "now_unix" => now.timestamp().to_string(),
            "today" => now.format("%Y-%m-%d").to_string(),
            "month_start" => month_start(v.now).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "month_start_unix" => month_start(v.now).timestamp().to_string(),
            f if f.starts_with("field.") => {
                v.fields.get(&f[6..]).map(|x| x.as_str().map(str::to_owned).unwrap_or_else(|| x.to_string())).unwrap_or_default()
            }
            _ => String::new(),
        };
        out.push_str(&value);
        rest = &rest[start + end + 2..];
    }
    out.push_str(rest);
    out
}

// ---------- JSON paths ----------

/// Every value matched by a JSON pointer whose `*` segments match all children.
pub fn select<'a>(root: &'a Value, path: &str) -> Vec<&'a Value> {
    let p = path.trim();
    if p.is_empty() || p == "/" {
        return vec![root];
    }
    let mut current = vec![root];
    for seg in p.trim_start_matches('/').split('/') {
        let seg = seg.replace("~1", "/").replace("~0", "~");
        let mut next = vec![];
        let filter = seg.strip_prefix('[').and_then(|x| x.strip_suffix(']')).map(|f| {
            f.split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect::<Vec<_>>()
        });
        for v in current {
            if let (Some(conds), Value::Array(a)) = (&filter, v) {
                let matches = |item: &&Value| {
                    conds.iter().all(|(k, want)| {
                        item.get(k).map(|x| x.as_str().map(str::to_owned).unwrap_or_else(|| x.to_string())) == Some(want.clone())
                    })
                };
                next.extend(a.iter().filter(matches));
                continue;
            }
            match (seg.as_str(), v) {
                ("*", Value::Array(a)) => next.extend(a.iter()),
                ("*", Value::Object(o)) => next.extend(o.values()),
                (s, Value::Array(a)) => {
                    if let Some(x) = s.parse::<usize>().ok().and_then(|i| a.get(i)) {
                        next.push(x)
                    }
                }
                (s, Value::Object(o)) => {
                    if let Some(x) = o.get(s) {
                        next.push(x)
                    }
                }
                _ => {}
            }
        }
        current = next;
    }
    current
}

fn as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().trim_start_matches('$').replace(',', "").parse().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Sum of the numeric values matched by a path (None when nothing numeric matches).
pub fn number(root: &Value, path: &Option<String>) -> Option<f64> {
    let p = path.as_ref()?;
    let nums: Vec<f64> = select(root, p).into_iter().filter_map(as_number).collect();
    (!nums.is_empty()).then(|| nums.iter().sum())
}

fn timestamp(root: &Value, path: &Option<String>) -> Option<i64> {
    let v = select(root, path.as_ref()?).into_iter().next()?;
    if let Some(ms) = parse_rfc3339(v) {
        return Some(ms);
    }
    if let Some(d) = v.as_str().and_then(|s| chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()) {
        return Some(d.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis());
    }
    let n = as_number(v)?;
    // Heuristic: seconds before year 2286, milliseconds after.
    Some(if n < 1e10 { (n * 1000.0) as i64 } else { n as i64 })
}

fn text(root: &Value, path: &Option<String>) -> Option<String> {
    let v = select(root, path.as_ref()?).into_iter().next()?;
    Some(v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())).filter(|s| !s.is_empty() && s != "null")
}

// ---------- Extraction ----------

pub fn extract(spec: &Spec, raw: &Value, now: i64) -> Snapshot {
    extract_with(spec, raw, now, &Value::Null)
}

/// Like [`extract`], with the user's form fields available to `limitValue` templates.
pub fn extract_with(spec: &Spec, raw: &Value, now: i64, fields: &Value) -> Snapshot {
    let mut limits = vec![];
    for l in &spec.limits {
        if let Some(p) = &l.skip_if {
            if select(raw, p).into_iter().any(|v| v.as_bool() == Some(true) || v.as_str() == Some("true") || v.as_f64() == Some(1.0)) {
                continue;
            }
        }
        let scale = l.scale.unwrap_or(1.0);
        let mut used = number(raw, &l.used).map(|x| x * l.used_scale.unwrap_or(scale));
        let typed = l
            .limit_value
            .as_ref()
            .and_then(|tpl| render(tpl, &Vars { key: "", fields, now }).trim().replace(',', ".").parse::<f64>().ok())
            .filter(|v| *v > 0.0);
        let mut limit = number(raw, &l.limit).map(|x| x * l.limit_scale.unwrap_or(scale)).filter(|v| *v > 0.0).or(typed);
        let mut remaining = number(raw, &l.remaining).map(|x| x * l.remaining_scale.unwrap_or(scale));
        match (used, limit, remaining) {
            (Some(u), Some(li), None) => remaining = Some(li - u),
            (None, Some(li), Some(r)) => used = Some(li - r),
            (Some(u), None, Some(r)) => limit = Some(u + r),
            _ => {}
        }
        let percent =
            number(raw, &l.percent).or_else(|| number(raw, &l.percent_remaining).map(|r| 100.0 - r)).or_else(|| match (used, limit) {
                (Some(u), Some(li)) if li > 0.0 => Some(u / li * 100.0),
                _ => None,
            });
        if percent.is_none() && used.is_none() && remaining.is_none() && limit.is_none() {
            continue; // nothing matched: skip rather than show a fake zero
        }
        let (resets_at, window_ms) = match (timestamp(raw, &l.resets_at), l.reset.as_deref()) {
            (Some(t), _) => (Some(round_reset(t)), l.window_ms),
            (None, Some("monthly")) => {
                let (start, end) = (month_start(now), next_month_start(now));
                (Some(end.timestamp_millis()), Some(end.timestamp_millis() - start.timestamp_millis()))
            }
            (None, Some("daily")) => {
                let d = Utc.timestamp_millis_opt(now).single().unwrap_or_else(Utc::now);
                let next = Utc.with_ymd_and_hms(d.year(), d.month(), d.day(), 0, 0, 0).single().unwrap() + chrono::Duration::days(1);
                (Some(next.timestamp_millis()), Some(86_400_000))
            }
            _ => (None, l.window_ms),
        };
        let kind = l.kind.clone().unwrap_or_else(|| if resets_at.is_some() { "month".into() } else { "balance".into() });
        limits.push(Limit {
            key: l.key.clone(),
            kind,
            name: l.label.clone(),
            window_ms,
            utilization: percent.unwrap_or(0.0).clamp(0.0, 100.0),
            resets_at,
            used,
            limit,
            remaining,
            unit: text(raw, &l.unit_path).or_else(|| l.unit.clone()),
            has_percent: percent.is_some(),
        });
    }
    Snapshot { limits, extra: None, breakdown: vec![] }
}

pub fn plan_of(spec: &Spec, raw: &Value) -> Option<String> {
    text(raw, &spec.plan)
}

pub fn identity_of(spec: &Spec, raw: &Value) -> Option<String> {
    text(raw, &spec.identity)
}

/// Performs the described request and normalises its answer.
pub async fn fetch(http: &Http, spec: &Spec, tokens: &TokenSet) -> Result<(Snapshot, Value), ApiError> {
    let now = crate::now_ms();
    let fields = tokens.meta.as_ref().and_then(|m| m.get("fields")).cloned().unwrap_or(Value::Null);
    let vars = Vars { key: &tokens.access_token, fields: &fields, now };
    let raw = if spec.requests.is_empty() {
        call(http, &spec.url, &spec.method, &spec.headers, &spec.body, &vars).await?
    } else {
        let mut combined = serde_json::Map::new();
        for r in &spec.requests {
            match call(http, &r.url, &r.method, &r.headers, &r.body, &vars).await {
                Ok(v) => {
                    combined.insert(r.id.clone(), v);
                }
                Err(_) if r.optional => {
                    combined.insert(r.id.clone(), Value::Null);
                }
                Err(e) => return Err(e),
            }
        }
        Value::Object(combined)
    };
    Ok((extract_with(spec, &raw, now, &fields), raw))
}

async fn call(
    http: &Http,
    url: &str,
    method: &str,
    headers: &BTreeMap<String, String>,
    body: &Option<Value>,
    vars: &Vars<'_>,
) -> Result<Value, ApiError> {
    let url = render(url, vars);
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(ApiError::new(400, "the connector URL must start with https:// or http://"));
    }
    let method = reqwest::Method::from_bytes(method.to_uppercase().as_bytes()).map_err(|_| ApiError::new(400, "invalid HTTP method"))?;
    let mut req = http.inner.request(method, &url);
    for (k, v) in headers {
        let value = render(v, vars);
        if !value.trim().is_empty() {
            req = req.header(k.as_str(), value);
        }
    }
    if let Some(body) = body {
        req = req.header("Content-Type", "application/json").body(render(&body.to_string(), vars));
    }
    http.json(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(limits: Vec<LimitSpec>) -> Spec {
        Spec {
            name: "t".into(),
            url: "https://x".into(),
            method: "GET".into(),
            headers: Default::default(),
            body: None,
            requests: vec![],
            limits,
            plan: None,
            identity: None,
        }
    }

    #[test]
    fn array_filters_and_date_only_resets() {
        let v = json!({ "limits": [ { "type": "A", "unit": 3, "p": 7 }, { "type": "A", "unit": 6, "p": 53 }, { "type": "B", "unit": 3, "p": 1 } ], "d": "2026-12-01" });
        assert_eq!(number(&v, &Some("/limits/[type=A,unit=6]/p".into())), Some(53.0));
        assert_eq!(number(&v, &Some("/limits/[unit=3]/p".into())), Some(8.0));
        assert_eq!(timestamp(&v, &Some("/d".into())), Some(Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap().timestamp_millis()));
    }

    #[test]
    fn per_field_scales_typed_limit_and_unit_path() {
        let raw = json!({ "costs": { "data": [ { "results": [ { "amount": "1234" } ] } ] }, "cur": "EUR" });
        let l = LimitSpec {
            key: "m".into(),
            used: Some("/costs/data/*/results/*/amount".into()),
            used_scale: Some(0.01),
            limit_value: Some("{{field.budget}}".into()),
            unit_path: Some("/cur".into()),
            ..Default::default()
        };
        let s = extract_with(&spec(vec![l]), &raw, 0, &json!({ "budget": "50" }));
        let x = &s.limits[0];
        assert_eq!((x.used, x.limit, x.unit.as_deref()), (Some(12.34), Some(50.0), Some("EUR")));
        assert!((x.utilization - 24.68).abs() < 1e-9);
    }

    #[test]
    fn select_with_wildcards_and_indexes() {
        let v = json!({ "data": [ { "results": [ { "amount": "1.5" }, { "amount": 2 } ] }, { "results": [ { "amount": 3 } ] } ] });
        assert_eq!(number(&v, &Some("/data/*/results/*/amount".into())), Some(6.5));
        assert_eq!(number(&v, &Some("/data/1/results/0/amount".into())), Some(3.0));
        assert_eq!(number(&v, &Some("/missing".into())), None);
    }

    #[test]
    fn derives_missing_value_and_percent() {
        let raw = json!({ "data": { "limit": 50, "usage": 12.5 } });
        let s = extract(
            &spec(vec![LimitSpec {
                key: "c".into(),
                used: Some("/data/usage".into()),
                limit: Some("/data/limit".into()),
                unit: Some("USD".into()),
                ..Default::default()
            }]),
            &raw,
            0,
        );
        let l = &s.limits[0];
        assert_eq!((l.used, l.limit, l.remaining), (Some(12.5), Some(50.0), Some(37.5)));
        assert_eq!(l.utilization, 25.0);
        assert!(l.has_percent);
        assert_eq!(l.kind, "balance");
    }

    #[test]
    fn balance_without_limit_has_no_percent() {
        let raw = json!({ "balance_infos": [ { "currency": "CNY", "total_balance": "23.10" } ] });
        let s = extract(
            &spec(vec![LimitSpec { key: "b".into(), remaining: Some("/balance_infos/0/total_balance".into()), ..Default::default() }]),
            &raw,
            0,
        );
        assert_eq!(s.limits[0].remaining, Some(23.1));
        assert!(!s.limits[0].has_percent);
    }

    #[test]
    fn skips_limits_that_match_nothing() {
        let s = extract(&spec(vec![LimitSpec { key: "x".into(), used: Some("/nope".into()), ..Default::default() }]), &json!({}), 0);
        assert!(s.limits.is_empty());
    }

    #[test]
    fn monthly_reset_and_templates() {
        let now = Utc.with_ymd_and_hms(2026, 11, 3, 10, 0, 0).unwrap().timestamp_millis();
        let raw = json!({ "pct": 40 });
        let s = extract(
            &spec(vec![LimitSpec { key: "m".into(), percent: Some("/pct".into()), reset: Some("monthly".into()), ..Default::default() }]),
            &raw,
            now,
        );
        assert_eq!(s.limits[0].resets_at, Some(Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap().timestamp_millis()));
        assert_eq!(s.limits[0].kind, "month");
        let f = json!({ "org": "acme" });
        let v = Vars { key: "sk-1", fields: &f, now };
        assert_eq!(
            render("Bearer {{key}} {{field.org}} {{month_start}} {{today}}", &v),
            "Bearer sk-1 acme 2026-11-01T00:00:00Z 2026-11-03"
        );
    }

    #[test]
    fn reset_timestamps_in_seconds_or_rfc3339() {
        let raw = json!({ "a": 1_767_225_600, "b": "2026-12-01T00:00:00Z" });
        assert_eq!(timestamp(&raw, &Some("/a".into())), Some(1_767_225_600_000));
        assert_eq!(timestamp(&raw, &Some("/b".into())), Some(Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap().timestamp_millis()));
    }
}
