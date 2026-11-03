//! Provider-agnostic indicators computed from a normalised snapshot: pace, projection of when a limit
//! will be hit, alert level and headroom.

use crate::providers::{Limit, Snapshot};
use crate::store::Store;
use serde::Serialize;

const H: f64 = 3_600_000.0;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub key: String,
    pub kind: String,
    pub name: Option<String>,
    pub window_ms: Option<i64>,
    pub utilization: f64,
    pub resets_at: Option<i64>,
    pub level: &'static str,
    /// Window not started yet (no request since the last reset).
    pub idle: bool,
    /// Fraction of the window elapsed (0–1).
    pub elapsed: Option<f64>,
    pub remaining_ms: Option<i64>,
    /// Utilisation relative to a linear pace (> 1 means burning faster than linear).
    pub pace: Option<f64>,
    pub rate_per_hour: Option<f64>,
    /// `recent` (regression on recent samples) or `average` (since the window started).
    pub rate_source: Option<&'static str>,
    pub eta_full: Option<i64>,
    pub hits_before_reset: Option<bool>,
    /// Absolute amounts (credits, money, requests) when the provider reports them.
    pub used: Option<f64>,
    pub limit: Option<f64>,
    pub remaining: Option<f64>,
    pub unit: Option<String>,
    /// False for a bare balance: no gauge, no percentage-based headroom.
    pub has_percent: bool,
}

pub fn level(u: f64) -> &'static str {
    if u >= 100.0 {
        "full"
    } else if u >= 85.0 {
        "crit"
    } else if u >= 60.0 {
        "warn"
    } else {
        "ok"
    }
}

/// Least-squares slope (percentage points per ms).
pub fn slope(points: &[(i64, f64)]) -> Option<f64> {
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f64;
    let t0 = points[0].0;
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
    for &(t, u) in points {
        let x = (t - t0) as f64;
        sx += x;
        sy += u;
        sxx += x * x;
        sxy += x * u;
    }
    let den = n * sxx - sx * sx;
    if den <= 0.0 {
        return None;
    }
    Some((n * sxy - sx * sy) / den)
}

/// Pure computation, given the samples of the current cycle (for tests and reuse).
pub fn compute(l: &Limit, cycle_points: &[(i64, f64)], now: i64) -> Window {
    let utilization = l.utilization.clamp(0.0, 100.0);
    let mut w = Window {
        key: l.key.clone(),
        kind: l.kind.clone(),
        name: l.name.clone(),
        window_ms: l.window_ms,
        utilization,
        resets_at: l.resets_at,
        level: if l.has_percent { level(utilization) } else { "none" },
        idle: false,
        elapsed: None,
        remaining_ms: None,
        pace: None,
        rate_per_hour: None,
        rate_source: None,
        eta_full: None,
        hits_before_reset: None,
        used: l.used,
        limit: l.limit,
        remaining: l.remaining,
        unit: l.unit.clone(),
        has_percent: l.has_percent,
    };
    if !l.has_percent {
        return w; // a balance has no pace nor projection
    }
    let (Some(reset), Some(len)) = (l.resets_at, l.window_ms) else {
        w.idle = utilization == 0.0;
        return w;
    };
    let len_f = len as f64;
    let start = reset - len;
    let elapsed_ms = ((now - start) as f64).clamp(0.0, len_f);
    let elapsed = elapsed_ms / len_f;
    w.elapsed = Some(elapsed);
    w.remaining_ms = Some((reset - now).max(0));
    if elapsed > 0.03 {
        w.pace = Some(utilization / (elapsed * 100.0));
    }

    // Recent slope only when the samples span enough time (10 min for a session, 3 h otherwise).
    let min_span = if l.kind == "session" { 10 * 60_000 } else { 3 * 3_600_000 };
    let span = match (cycle_points.first(), cycle_points.last()) {
        (Some(a), Some(b)) => b.0 - a.0,
        _ => 0,
    };
    let mut rate = if span >= min_span { slope(cycle_points).map(|r| (r, "recent")) } else { None };
    if rate.is_none() && elapsed_ms > len_f * 0.05 && utilization > 0.0 {
        rate = Some((utilization / elapsed_ms, "average"));
    }
    if let Some((r, src)) = rate {
        w.rate_source = Some(src);
        if r > 1e-12 {
            w.rate_per_hour = Some(r * H);
            if utilization < 100.0 {
                let eta = now + ((100.0 - utilization) / r) as i64;
                w.eta_full = Some(eta);
                w.hits_before_reset = Some(eta < reset);
            }
        } else {
            w.rate_per_hour = Some(0.0);
        }
    }
    w
}

/// Derives every window of a snapshot, using the stored history of the current cycle.
pub fn derive(store: &Store, account_id: &str, snap: &Snapshot, now: i64) -> Vec<Window> {
    snap.limits
        .iter()
        .map(|l| {
            let points: Vec<(i64, f64)> = match (l.resets_at, l.window_ms) {
                (Some(reset), Some(len)) => {
                    let lookback = if l.kind == "session" { 45 * 60_000 } else { 24 * 3_600_000 };
                    store
                        .series(account_id, &l.key, (reset - len).max(now - lookback))
                        .into_iter()
                        .filter(|p| p.2 == Some(reset))
                        .map(|p| (p.0, p.1))
                        .collect()
                }
                _ => vec![],
            };
            compute(l, &points, now)
        })
        .collect()
}

/// Headroom = what is left before the first session/day/week/month limit is hit. Prepaid balances
/// with a known ceiling count too; bare balances (no percentage) do not.
pub fn headroom(windows: &[Window]) -> Option<f64> {
    windows
        .iter()
        .filter(|w| w.has_percent && matches!(w.kind.as_str(), "session" | "week" | "day" | "month" | "balance"))
        .map(|w| 100.0 - w.utilization)
        .reduce(f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn limit(kind: &str, util: f64, reset: i64, len: i64) -> Limit {
        Limit::percent("k", kind, None, Some(len), util, Some(reset))
    }

    #[test]
    fn levels() {
        assert_eq!(level(10.0), "ok");
        assert_eq!(level(60.0), "warn");
        assert_eq!(level(85.0), "crit");
        assert_eq!(level(100.0), "full");
    }

    #[test]
    fn slope_of_a_line() {
        let pts: Vec<(i64, f64)> = (0..5).map(|i| (i * MIN, 10.0 + i as f64)).collect();
        assert!((slope(&pts).unwrap() * MIN as f64 - 1.0).abs() < 1e-9);
        assert_eq!(slope(&pts[..2]), None);
    }

    #[test]
    fn projects_limit_before_reset_from_recent_trend() {
        // 5 h window, 2 h elapsed, 60 % used, +1 %/min over the last 20 min → full in 40 min.
        let now = 1_000_000_000_000;
        let reset = now + 3 * 60 * MIN;
        let pts: Vec<(i64, f64)> = (0..=20).map(|i| (now - (20 - i) * MIN, 40.0 + i as f64)).collect();
        let w = compute(&limit("session", 60.0, reset, 5 * 60 * MIN), &pts, now);
        assert_eq!(w.rate_source, Some("recent"));
        assert_eq!(w.hits_before_reset, Some(true));
        assert!((w.eta_full.unwrap() - (now + 40 * MIN)).abs() < MIN);
        assert!((w.pace.unwrap() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn short_history_falls_back_to_average() {
        let now = 1_000_000_000_000;
        let reset = now + 3 * 60 * MIN;
        let pts = vec![(now - 2 * MIN, 59.0), (now - MIN, 59.5), (now, 60.0)];
        let w = compute(&limit("session", 60.0, reset, 5 * 60 * MIN), &pts, now);
        assert_eq!(w.rate_source, Some("average"));
        // 60 % in 2 h → 30 %/h → 40 % left needs 80 min, before the reset in 3 h.
        assert_eq!(w.hits_before_reset, Some(true));
    }

    #[test]
    fn idle_window_without_reset() {
        let l = Limit::percent("five_hour", "session", None, Some(5 * 60 * MIN), 0.0, None);
        let w = compute(&l, &[], 0);
        assert!(w.idle);
        assert_eq!(w.pace, None);
    }

    #[test]
    fn headroom_ignores_model_caps() {
        let now = 0;
        let mut ws = vec![compute(&limit("session", 30.0, 1, 1), &[], now), compute(&limit("week", 70.0, 1, 1), &[], now)];
        ws.push(compute(&limit("model", 99.0, 1, 1), &[], now));
        assert_eq!(headroom(&ws), Some(30.0));
    }
}
