//! Server-side strings (Homepage widgets, fallback labels). The web page localises itself.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    /// `VR_LANG` (en|fr), else the system `LANG`, else English.
    pub fn from_env() -> Self {
        let v = std::env::var("VR_LANG").or_else(|_| std::env::var("LANG")).unwrap_or_default().to_lowercase();
        if v.starts_with("fr") {
            Lang::Fr
        } else {
            Lang::En
        }
    }
}

pub fn t(lang: Lang, key: &str) -> &'static str {
    use Lang::*;
    match (lang, key) {
        (Fr, "session") => "Session",
        (En, "session") => "Session",
        (Fr, "week") => "Semaine",
        (En, "week") => "Week",
        (Fr, "day") => "Jour",
        (En, "day") => "Day",
        (Fr, "free") => "libre",
        (En, "free") => "free",
        (Fr, "use_now") => "Compte à utiliser maintenant",
        (En, "use_now") => "Account to use now",
        (Fr, "use_now_desc") => "Le compte avec le plus de marge, par outil",
        (En, "use_now_desc") => "The account with the most headroom, per tool",
        (Fr, "account") => "Compte",
        (En, "account") => "Account",
        (Fr, "available") => "Disponible",
        (En, "available") => "Available",
        (Fr, "alerts") => "Alertes",
        (En, "alerts") => "Alerts",
        (Fr, "reset") => "Reset",
        (En, "reset") => "Reset",
        (Fr, "status") => "Statut",
        (En, "status") => "Status",
        (Fr, "st_top") => "★ Top",
        (En, "st_top") => "★ Best",
        (_, "st_ok") => "✓ OK",
        (Fr, "st_cached") => "⚠︎ Cache",
        (En, "st_cached") => "⚠︎ Cached",
        (Fr, "st_stale") => "⚠︎ Ancien",
        (En, "st_stale") => "⚠︎ Stale",
        (Fr, "st_error") => "✕ Erreur",
        (En, "st_error") => "✕ Error",
        (Fr, "st_reauth") => "✕ Reconnexion",
        (En, "st_reauth") => "✕ Sign in",
        (Fr, "st_pending") => "… Chargement",
        (En, "st_pending") => "… Loading",
        (Fr, "min") => "min",
        (En, "min") => "min",
        (Fr, "h") => "h",
        (En, "h") => "h",
        (Fr, "d") => "j",
        (En, "d") => "d",
        _ => "",
    }
}

/// Human label of a limit, e.g. "Session", "Week", "Week · Opus".
pub fn limit_label(lang: Lang, kind: &str, name: Option<&str>, window_ms: Option<i64>) -> String {
    match (kind, name) {
        ("session", _) => {
            let hours = window_ms.map(|ms| ms / 3_600_000).filter(|h| *h > 0);
            match (lang, hours) {
                (Lang::Fr, Some(h)) => format!("Session {h} h"),
                (Lang::En, Some(h)) => format!("{h}-hour session"),
                _ => t(lang, "session").into(),
            }
        }
        ("week", _) => t(lang, "week").into(),
        ("day", None) => t(lang, "day").into(),
        (_, Some(n)) => {
            let weekly = window_ms.is_some_and(|ms| ms >= 6 * 86_400_000);
            if weekly {
                format!("{} · {n}", t(lang, "week"))
            } else {
                n.to_string()
            }
        }
        (k, None) => k.to_string(),
    }
}

pub fn duration(lang: Lang, ms: Option<i64>) -> String {
    let Some(ms) = ms else { return "—".into() };
    let m = (ms.max(0) as f64 / 60_000.0).round() as i64;
    if m < 60 {
        return format!("{m} {}", t(lang, "min"));
    }
    let h = m / 60;
    if h < 48 {
        return format!("{h} {} {:02}", t(lang, "h"), m % 60);
    }
    format!("{} {} {} {}", h / 24, t(lang, "d"), h % 24, t(lang, "h"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(limit_label(Lang::En, "session", None, Some(5 * 3_600_000)), "5-hour session");
        assert_eq!(limit_label(Lang::Fr, "session", None, Some(5 * 3_600_000)), "Session 5 h");
        assert_eq!(limit_label(Lang::En, "model", Some("Opus"), Some(7 * 86_400_000)), "Week · Opus");
        assert_eq!(limit_label(Lang::Fr, "model", Some("gemini-2.5-pro"), Some(86_400_000)), "gemini-2.5-pro");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(Lang::En, Some(45 * 60_000)), "45 min");
        assert_eq!(duration(Lang::Fr, Some((2 * 60 + 5) * 60_000)), "2 h 05");
        assert_eq!(duration(Lang::Fr, Some(3 * 86_400_000 + 4 * 3_600_000)), "3 j 4 h");
    }
}
