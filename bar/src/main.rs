//! VibeRooms menu bar (macOS): gauges per account, native menu, notifications, Claude Code status
//! line and, optionally, switching the Claude Code account of this Mac.

mod draw;
mod switcher;

use chrono::{Datelike, Local, TimeZone};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray_icon::menu::{CheckMenuItem, IconMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{TrayIcon, TrayIconBuilder};

const LABEL: &str = "com.allanbruyere.viberooms";
const DEFAULT_SERVER: &str = "http://127.0.0.1:8790";
const POLL: Duration = Duration::from_secs(30);
const LOCK_PORT: u16 = 47_811;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}
fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
}
fn support_dir() -> PathBuf {
    home().join("Library/Application Support/VibeRooms")
}

// ---------- Language ----------
static FR: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
/// True when the UI language is French.
fn fr() -> bool {
    *FR.get().unwrap_or(&false)
}
/// Picks the French or the English string.
fn l<'a>(fr_s: &'a str, en_s: &'a str) -> &'a str {
    if fr() {
        fr_s
    } else {
        en_s
    }
}

fn init_lang(pref: &str) {
    let is_fr = match pref {
        "fr" => true,
        "en" => false,
        _ => {
            let locale = std::process::Command::new("/usr/bin/defaults")
                .args(["read", "-g", "AppleLocale"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
                .unwrap_or_default();
            let env = std::env::var("LANG").unwrap_or_default().to_lowercase();
            if locale.trim().is_empty() {
                env.starts_with("fr")
            } else {
                locale.starts_with("fr")
            }
        }
    };
    let _ = FR.set(is_fr);
}
fn agent_plist() -> PathBuf {
    home().join(format!("Library/LaunchAgents/{LABEL}.plist"))
}

// ---------- Configuration ----------
#[derive(Clone)]
struct Config {
    server: String,
    notifications: bool,
    /// "auto" (system), "fr" or "en".
    lang: String,
    /// Experimental: switch the Claude Code account of this Mac from the menu.
    switcher: bool,
}

impl Config {
    fn load() -> Self {
        let v: Value = std::fs::read_to_string(support_dir().join("config.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        Self {
            server: v.get("server").and_then(Value::as_str).unwrap_or(DEFAULT_SERVER).trim_end_matches('/').to_string(),
            notifications: v.get("notifications").and_then(Value::as_bool).unwrap_or(true),
            lang: v.get("lang").and_then(Value::as_str).unwrap_or("auto").to_string(),
            switcher: v.get("switcher").and_then(Value::as_bool).unwrap_or(false),
        }
    }
    fn save(&self) {
        let _ = std::fs::create_dir_all(support_dir());
        let v =
            serde_json::json!({ "server": self.server, "notifications": self.notifications, "lang": self.lang, "switcher": self.switcher });
        let _ = std::fs::write(support_dir().join("config.json"), serde_json::to_vec_pretty(&v).unwrap());
    }
}

// ---------- Formatting ----------
fn fmt_dur(ms: i64) -> String {
    let m = (ms.max(0) as f64 / 60_000.0).round() as i64;
    if m < 60 {
        return format!("{m} min");
    }
    let h = m / 60;
    if h < 48 {
        return format!("{h} h {:02}", m % 60);
    }
    format!("{} {} {} h", h / 24, l("j", "d"), h % 24)
}

fn fmt_ago(ts: i64) -> String {
    let s = (now_ms() - ts) / 1000;
    if s < 10 {
        l("à l'instant", "just now").into()
    } else if fr() {
        if s < 60 {
            format!("il y a {s} s")
        } else {
            format!("il y a {}", fmt_dur(s * 1000))
        }
    } else if s < 60 {
        format!("{s} s ago")
    } else {
        format!("{} ago", fmt_dur(s * 1000))
    }
}

fn fmt_clock(ts: i64) -> String {
    let Some(d) = Local.timestamp_millis_opt(ts).single() else { return "—".into() };
    let today = Local::now().date_naive();
    let hm = d.format("%H:%M").to_string();
    let days = (d.date_naive() - today).num_days();
    match days {
        0 => hm,
        1 => format!("{} {hm}", l("demain", "tomorrow")),
        2..=6 => {
            const WD_FR: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];
            const WD_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
            let i = d.weekday().num_days_from_monday() as usize;
            format!("{} {hm}", if fr() { WD_FR[i] } else { WD_EN[i] })
        }
        _ => d.format(if fr() { "%d/%m %H:%M" } else { "%m/%d %H:%M" }).to_string(),
    }
}

fn fmt_money(amount: f64, cur: &str) -> String {
    let sym = match cur {
        "EUR" => "€",
        "USD" => "$",
        "GBP" => "£",
        c => c,
    };
    if fr() {
        format!("{:.2} {sym}", amount).replace('.', ",")
    } else {
        format!("{sym}{:.2}", amount)
    }
}

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}
fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn i(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(Value::as_i64)
}

/// Short name of a limit: Session, Week, Day, or "Wk Opus" for a weekly per-model cap.
fn wname(w: &Value) -> String {
    match s(w, "kind") {
        "session" => "Session".into(),
        "week" => l("Semaine", "Week").into(),
        "day" => l("Jour", "Day").into(),
        _ => {
            let n = s(w, "name");
            let weekly = i(w, "windowMs").is_some_and(|ms| ms >= 6 * 86_400_000);
            if weekly {
                format!("{} {n}", l("Sem.", "Wk"))
            } else {
                n.to_string()
            }
        }
    }
}

fn window_line(w: &Value) -> String {
    let used = f(w, "utilization").unwrap_or(0.0);
    let name = wname(w);
    let Some(reset) = i(w, "resetsAt") else {
        return format!("{name} · {}", l("libre", "free"));
    };
    if used >= 100.0 {
        return format!("{name} · {} {}", l("bloqué jusqu'à", "blocked until"), fmt_clock(reset));
    }
    let mut line = if s(w, "kind") == "session" {
        format!("{name} {used:.0} % · {} {} ({})", l("reset dans", "resets in"), fmt_dur(reset - now_ms()), fmt_clock(reset))
    } else {
        format!("{name} {used:.0} % · reset {}", fmt_clock(reset))
    };
    if w.get("hitsBeforeReset").and_then(Value::as_bool) == Some(true) {
        if let Some(eta) = i(w, "etaFull") {
            line.push_str(&format!(" · ⚠︎ {} ~{}", l("limite", "limit"), fmt_clock(eta)));
        }
    }
    line
}

// ---------- Account used by Claude Code on this Mac ----------
fn local_account() -> Option<(String, Option<String>)> {
    let raw = std::fs::read_to_string(home().join(".claude.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let oa = v.get("oauthAccount")?;
    Some((s(oa, "emailAddress").to_lowercase(), oa.get("organizationUuid").and_then(Value::as_str).map(str::to_owned)))
}

fn is_local(a: &Value, local: &Option<(String, Option<String>)>) -> bool {
    let Some((email, org)) = local else { return false };
    if s(a, "provider") != "claude" {
        return false;
    }
    if s(a, "email").to_lowercase() != *email {
        return false;
    }
    match (a.get("orgUuid").and_then(Value::as_str), org) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

// ---------- Notifications ----------
fn notify(title: &str, body: &str) {
    let esc = |t: &str| t.replace('\\', "\\\\").replace('"', "\\\"");
    let script =
        format!("display notification \"{}\" with title \"VibeRooms\" subtitle \"{}\" sound name \"Glass\"", esc(body), esc(title));
    let _ = std::process::Command::new("/usr/bin/osascript").arg("-e").arg(script).spawn();
}

#[derive(Default)]
struct Notifier {
    primed: bool,
    level: HashMap<String, u8>,      // account:window -> notified threshold (0, 80, 95)
    reset_at: HashMap<String, i64>,  // account:window -> known resetsAt
    peak: HashMap<String, f64>,      // account:window -> max utilisation in the cycle
    status: HashMap<String, String>, // account -> status
}

impl Notifier {
    fn check(&mut self, state: &Value, enabled: bool) {
        let Some(accounts) = state.get("accounts").and_then(Value::as_array) else { return };
        let mut out: Vec<(String, String)> = vec![];
        for a in accounts {
            let id = s(a, "id");
            let label = s(a, "label");
            let status = s(a, "status").to_string();
            if status == "reauth" && self.status.get(id).map(String::as_str) != Some("reauth") {
                out.push((
                    label.into(),
                    l(
                        "Session OAuth expirée : reconnecte le compte depuis le tableau de bord.",
                        "OAuth session expired: reconnect the account from the dashboard.",
                    )
                    .into(),
                ));
            }
            self.status.insert(id.into(), status);
            for w in a.get("windows").and_then(Value::as_array).into_iter().flatten() {
                let kind = s(w, "kind");
                if kind != "session" && kind != "week" {
                    continue;
                }
                let key = format!("{id}:{}", s(w, "key"));
                let used = f(w, "utilization").unwrap_or(0.0);
                let reset = i(w, "resetsAt").unwrap_or(0);
                let what = if kind == "session" { l("session", "session") } else { l("quota de la semaine", "weekly limit") };
                let prev_reset = self.reset_at.insert(key.clone(), reset);
                // A real reset moves the deadline by hours; ignore a few seconds of jitter.
                if prev_reset.is_some_and(|p| p != 0 && reset != 0 && (p - reset).abs() > 300_000) {
                    if self.peak.get(&key).copied().unwrap_or(0.0) >= 80.0 && kind == "session" {
                        out.push((
                            label.into(),
                            l("Nouvelle session : quota de nouveau disponible.", "New session: limit available again.").into(),
                        ));
                    }
                    self.peak.insert(key.clone(), 0.0);
                    self.level.insert(key.clone(), 0);
                }
                let peak = self.peak.entry(key.clone()).or_insert(0.0);
                *peak = peak.max(used);
                let lvl = self.level.entry(key.clone()).or_insert(0);
                let target = if used >= 95.0 {
                    95
                } else if used >= 80.0 {
                    80
                } else {
                    0
                };
                if target > *lvl {
                    if self.primed {
                        let tail = if reset > 0 {
                            format!(" · reset {} ({} {})", fmt_clock(reset), l("dans", "in"), fmt_dur(reset - now_ms()))
                        } else {
                            String::new()
                        };
                        out.push((label.into(), format!("{what} {} {used:.0} %{tail}", l("à", "at"))));
                    }
                    *lvl = target;
                } else if used < 70.0 {
                    *lvl = 0;
                }
            }
        }
        if self.primed && enabled {
            for (t, b) in out {
                notify(&t, &b);
            }
        }
        self.primed = true;
    }
}

// ---------- Collecte ----------
enum UserEvent {
    Data(Result<Value, String>),
    Menu(MenuEvent),
    Tick,
    Switcher(switcher::Snapshot),
    SwitchResult(Result<String, String>),
}

/// Switcher thread: watches the Claude Code session of this Mac and performs switches (off the UI loop).
fn switcher_loop(proxy: EventLoopProxy<UserEvent>, rx: mpsc::Receiver<switcher::SwCmd>) {
    let mut sw = switcher::Switcher::new();
    loop {
        sw.tick();
        if proxy.send_event(UserEvent::Switcher(sw.snapshot())).is_err() {
            return;
        }
        let wait = Duration::from_secs(if sw.enrolling() { 3 } else { 15 });
        match rx.recv_timeout(wait) {
            Ok(switcher::SwCmd::Switch(key)) => {
                let r = sw.switch(&key);
                let _ = proxy.send_event(UserEvent::SwitchResult(r));
            }
            Ok(switcher::SwCmd::Enroll) => sw.start_enroll(),
            Ok(switcher::SwCmd::Forget(key)) => sw.forget(&key),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

enum Cmd {
    Refresh,
}

fn fetcher(server: String, proxy: EventLoopProxy<UserEvent>, rx: mpsc::Receiver<Cmd>) {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(5)).timeout(Duration::from_secs(25)).build();
    let mut manual = false;
    loop {
        let res = if manual {
            agent
                .post(&format!("{server}/api/refresh"))
                .set("X-VibeRooms", "1")
                .send_json(serde_json::json!({}))
                .map_err(|e| e.to_string())
                .and_then(|r| r.into_json::<Value>().map_err(|e| e.to_string()))
                .and_then(|v| v.get("state").cloned().ok_or_else(|| "unexpected response".into()))
        } else {
            agent
                .get(&format!("{server}/api/state"))
                .call()
                .map_err(|e| e.to_string())
                .and_then(|r| r.into_json::<Value>().map_err(|e| e.to_string()))
        };
        if proxy.send_event(UserEvent::Data(res)).is_err() {
            return;
        }
        manual = matches!(rx.recv_timeout(POLL), Ok(Cmd::Refresh));
    }
}

// ---------- Menu ----------
struct Fixed {
    header: MenuItem,
    open: MenuItem,
    refresh: MenuItem,
    notif: CheckMenuItem,
    login: CheckMenuItem,
    updated: MenuItem,
    quit: MenuItem,
}

struct Ui {
    cfg: Config,
    tray: Option<TrayIcon>,
    fixed: Option<Fixed>,
    signature: String,
    lines: Vec<(IconMenuItem, String, Option<String>)>, // (item, account id, window key or None = header)
    extras: Vec<(MenuItem, String)>,
    state: Option<Value>,
    last_ok: Option<i64>,
    last_err: Option<String>,
    notifier: Notifier,
    tx: mpsc::Sender<Cmd>,
    sw_tx: mpsc::Sender<switcher::SwCmd>,
    sw: switcher::Snapshot,
    sw_sub: Option<Submenu>,
    sw_items: Vec<(CheckMenuItem, String)>,
    sw_add: Option<MenuItem>,
    sw_forget: Vec<(MenuItem, String)>,
    sw_shortcut: Option<(MenuItem, String)>,
    sw_info: Option<MenuItem>,
    suggested: Option<String>,
}

fn icon(c: draw::Canvas) -> Option<tray_icon::menu::Icon> {
    tray_icon::menu::Icon::from_rgba(c.px, c.w, c.h).ok()
}

impl Ui {
    fn accounts(&self) -> Vec<Value> {
        self.state.as_ref().and_then(|s| s.get("accounts")).and_then(Value::as_array).cloned().unwrap_or_default()
    }

    fn header_text(&self, accounts: &[Value]) -> String {
        if let (Some(err), true) = (&self.last_err, self.last_ok.is_none_or(|t| now_ms() - t > 90_000)) {
            let since = self.last_ok.map(|t| format!(" ({} {})", l("dernière mesure", "last measure"), fmt_ago(t))).unwrap_or_default();
            return format!(
                "⚠︎ {}{since} — {}",
                l("Serveur VibeRooms injoignable", "VibeRooms server unreachable"),
                err.chars().take(60).collect::<String>()
            );
        }
        if accounts.is_empty() {
            return l("Aucun compte connecté — ajoute-les depuis le tableau de bord", "No account connected — add them from the dashboard")
                .into();
        }
        let recs: Vec<String> = accounts
            .iter()
            .filter(|a| a.get("recommended").and_then(Value::as_bool) == Some(true))
            .map(|r| format!("{} : {} ({:.0} % {})", s(r, "tool"), s(r, "label"), f(r, "headroom").unwrap_or(0.0), l("dispo", "left")))
            .collect();
        if recs.is_empty() {
            "VibeRooms".into()
        } else {
            format!("★ {} — {}", l("À utiliser", "Use now"), recs.join("  ·  "))
        }
    }

    fn account_title(a: &Value, local: bool) -> String {
        let mut t = format!("{} — {} {}", s(a, "label"), s(a, "providerName"), s(a, "plan"));
        match f(a, "headroom") {
            Some(h) => t.push_str(&format!(" · {h:.0} % {}", l("dispo", "left"))),
            None => t.push_str(l(" · en attente", " · pending")),
        }
        match s(a, "status") {
            "reauth" => t.push_str(l(" · ⚠︎ reconnexion requise", " · ⚠︎ sign-in required")),
            "error" => t.push_str(l(" · ⚠︎ erreur", " · ⚠︎ error")),
            "stale" | "degraded" => t.push_str(l(" · données en cache", " · cached data")),
            _ => {}
        }
        if local {
            t.push_str(l("   ● ce Mac", "   ● this Mac"));
        }
        t
    }

    /// Label and headroom of a vault entry, from the matching server-side account.
    fn entry_info(e: &switcher::Entry, accounts: &[Value]) -> (String, Option<f64>) {
        let acc = accounts.iter().find(|a| {
            s(a, "provider") == "claude"
                && s(a, "email").eq_ignore_ascii_case(&e.email)
                && match (a.get("orgUuid").and_then(Value::as_str), e.org_uuid.as_deref()) {
                    (Some(x), Some(y)) => x == y,
                    _ => true,
                }
        });
        match acc {
            Some(a) => (s(a, "label").to_string(), f(a, "headroom")),
            None => (format!("{}{}", e.email, e.org_name.as_ref().map(|o| format!(" ({o})")).unwrap_or_default()), None),
        }
    }

    fn entry_text(e: &switcher::Entry, accounts: &[Value]) -> String {
        let (label, head) = Self::entry_info(e, accounts);
        let mut t = label;
        if e.expired() {
            t.push_str(l(" — session expirée, à rajouter", " — session expired, add it again"));
        } else if let Some(h) = head {
            t.push_str(&format!(" — {h:.0} % {}", l("dispo", "left")));
        }
        t
    }

    fn sessions_text(n: usize) -> String {
        match (n, fr()) {
            (0, true) => "Aucune session claude ouverte".into(),
            (0, false) => "No open claude session".into(),
            (1, true) => "1 session claude ouverte : à relancer après une bascule".into(),
            (1, false) => "1 open claude session: restart it after a switch".into(),
            (n, true) => format!("{n} sessions claude ouvertes : à relancer après une bascule"),
            (n, false) => format!("{n} open claude sessions: restart them after a switch"),
        }
    }

    fn switch_title(&self, accounts: &[Value]) -> String {
        let active = self.sw.active.as_ref().and_then(|k| self.sw.entries.iter().find(|e| &e.key == k));
        let name = match active {
            Some(e) => Self::entry_info(e, accounts).0,
            None => l("compte non identifié", "unidentified account").into(),
        };
        let suffix = if self.sw.enrolling { l(" · ajout en cours…", " · adding…") } else { "" };
        format!("{} {name}{suffix}", l("Claude Code sur ce Mac :", "Claude Code on this Mac:"))
    }

    /// Recommended Claude account present in the vault but not active → switch shortcut.
    fn shortcut_target(&self, accounts: &[Value]) -> Option<(String, String)> {
        if !self.cfg.switcher {
            return None;
        }
        let rec = accounts.iter().find(|a| s(a, "provider") == "claude" && a.get("recommended").and_then(Value::as_bool) == Some(true))?;
        let e = self.sw.entries.iter().find(|e| {
            s(rec, "email").eq_ignore_ascii_case(&e.email)
                && rec.get("orgUuid").and_then(Value::as_str).is_none_or(|o| Some(o) == e.org_uuid.as_deref())
        })?;
        if self.sw.active.as_deref() == Some(e.key.as_str()) || e.expired() {
            return None;
        }
        Some((e.key.clone(), s(rec, "label").to_string()))
    }

    fn full_signature(&self, accounts: &[Value]) -> String {
        format!(
            "{}#{}#{:?}#{}#{:?}",
            Self::signature(accounts),
            self.sw.entries.iter().map(|e| format!("{}:{}", e.key, e.expired())).collect::<Vec<_>>().join(","),
            self.sw.active,
            self.sw.enrolling,
            self.shortcut_target(accounts).map(|t| t.0),
        )
    }

    fn signature(accounts: &[Value]) -> String {
        accounts
            .iter()
            .map(|a| {
                let keys: Vec<&str> = a.get("windows").and_then(Value::as_array).into_iter().flatten().map(|w| s(w, "key")).collect();
                let extra = a.pointer("/extra/enabled").and_then(Value::as_bool).unwrap_or(false);
                format!("{}[{}]{}", s(a, "id"), keys.join(","), extra)
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    fn extra_text(a: &Value) -> String {
        let e = a.get("extra").cloned().unwrap_or(Value::Null);
        let cur = e.get("currency").and_then(Value::as_str).unwrap_or("USD");
        let used = f(&e, "used").map(|v| fmt_money(v, cur)).unwrap_or_else(|| "—".into());
        let limit = f(&e, "limit").map(|v| fmt_money(v, cur)).unwrap_or_else(|| l("illimité", "unlimited").into());
        format!("{} {used} / {limit}", l("Crédits supplémentaires :", "Extra usage credits:"))
    }

    fn rebuild(&mut self) {
        let accounts = self.accounts();
        let local = local_account();
        let menu = Menu::new();
        let fx = Fixed {
            header: MenuItem::new(self.header_text(&accounts), true, None),
            open: MenuItem::new(l("Ouvrir le tableau de bord", "Open the dashboard"), true, None),
            refresh: MenuItem::new(l("Rafraîchir maintenant", "Refresh now"), true, None),
            notif: CheckMenuItem::new(
                l("Notifications (80 % / 95 % / reset)", "Notifications (80 % / 95 % / reset)"),
                true,
                self.cfg.notifications,
                None,
            ),
            login: CheckMenuItem::new(l("Ouvrir à la connexion", "Open at login"), true, agent_plist().exists(), None),
            updated: MenuItem::new("", false, None),
            quit: MenuItem::new(l("Quitter VibeRooms", "Quit VibeRooms"), true, None),
        };
        let _ = menu.append(&fx.header);

        // Switching the Claude Code account of this Mac (experimental, opt-in).
        self.sw_items.clear();
        self.sw_forget.clear();
        self.sw_shortcut = None;
        self.sw_sub = None;
        self.sw_add = None;
        self.sw_info = None;
        if let Some((key, label)) = self.shortcut_target(&accounts) {
            let item = MenuItem::new(
                if fr() {
                    format!("⇄ Passer Claude Code sur {label} (★ recommandé)")
                } else {
                    format!("⇄ Switch Claude Code to {label} (★ best)")
                },
                true,
                None,
            );
            let _ = menu.append(&item);
            self.sw_shortcut = Some((item, key));
        }
        if self.cfg.switcher {
            let sub = Submenu::new(self.switch_title(&accounts), true);
            for e in &self.sw.entries {
                let active = self.sw.active.as_deref() == Some(e.key.as_str());
                let item = CheckMenuItem::new(Self::entry_text(e, &accounts), !e.expired() || active, active, None);
                let _ = sub.append(&item);
                self.sw_items.push((item, e.key.clone()));
            }
            if !self.sw.entries.is_empty() {
                let _ = sub.append(&PredefinedMenuItem::separator());
            }
            let add = MenuItem::new(
                if self.sw.enrolling {
                    l("Ajout en cours (fenêtre Terminal)…", "Adding (Terminal window)…")
                } else {
                    l("Ajouter un compte à ce Mac…", "Add an account to this Mac…")
                },
                !self.sw.enrolling,
                None,
            );
            let _ = sub.append(&add);
            let removable: Vec<&switcher::Entry> =
                self.sw.entries.iter().filter(|e| self.sw.active.as_deref() != Some(e.key.as_str())).collect();
            if !removable.is_empty() {
                let forget = Submenu::new(l("Retirer de la bascule", "Remove from switching"), true);
                for e in removable {
                    let item = MenuItem::new(Self::entry_info(e, &accounts).0, true, None);
                    let _ = forget.append(&item);
                    self.sw_forget.push((item, e.key.clone()));
                }
                let _ = sub.append(&forget);
            }
            let _ = sub.append(&PredefinedMenuItem::separator());
            let info = MenuItem::new(Self::sessions_text(self.sw.open_sessions), false, None);
            let _ = sub.append(&info);
            self.sw_info = Some(info);
            let _ = menu.append(&sub);
            self.sw_sub = Some(sub);
            self.sw_add = Some(add);
        }

        let _ = menu.append(&PredefinedMenuItem::separator());
        self.lines.clear();
        self.extras.clear();
        // Accounts grouped by tool (Claude Code, Codex…), in the server order.
        let mut tools: Vec<&str> = vec![];
        for a in &accounts {
            if !tools.contains(&s(a, "tool")) {
                tools.push(s(a, "tool"));
            }
        }
        let grouped: Vec<&Value> = tools.iter().flat_map(|t| accounts.iter().filter(move |a| s(a, "tool") == *t)).collect();
        let mut current_tool = "";
        for a in grouped {
            if tools.len() > 1 && s(a, "tool") != current_tool {
                current_tool = s(a, "tool");
                let _ = menu.append(&MenuItem::new(current_tool.to_uppercase(), false, None));
            }
            let id = s(a, "id").to_string();
            let color = draw::account_color(s(a, "color"));
            let head = IconMenuItem::new(Self::account_title(a, is_local(a, &local)), true, icon(draw::swatch(color)), None);
            let _ = menu.append(&head);
            self.lines.push((head, id.clone(), None));
            for w in a.get("windows").and_then(Value::as_array).into_iter().flatten() {
                let used = f(w, "utilization").unwrap_or(0.0);
                let item = IconMenuItem::new(window_line(w), true, icon(draw::menu_bar(used, draw::level_color(used))), None);
                let _ = menu.append(&item);
                self.lines.push((item, id.clone(), Some(s(w, "key").to_string())));
            }
            if a.pointer("/extra/enabled").and_then(Value::as_bool) == Some(true) {
                let item = MenuItem::new(Self::extra_text(a), true, None);
                let _ = menu.append(&item);
                self.extras.push((item, id.clone()));
            }
            let _ = menu.append(&PredefinedMenuItem::separator());
        }
        let _ = menu.append(&fx.open);
        let _ = menu.append(&fx.refresh);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&fx.notif);
        let _ = menu.append(&fx.login);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&fx.updated);
        let _ = menu.append(&fx.quit);
        if let Some(tray) = &self.tray {
            tray.set_menu(Some(Box::new(menu)));
        }
        self.fixed = Some(fx);
        self.signature = self.full_signature(&accounts);
        self.refresh_texts();
    }

    /// In-place update (the menu may be open): texts, gauges, icon and title.
    fn refresh_texts(&mut self) {
        let accounts = self.accounts();
        if self.full_signature(&accounts) != self.signature {
            return self.rebuild();
        }
        if let Some(sub) = &self.sw_sub {
            sub.set_text(self.switch_title(&accounts));
        }
        if let Some(info) = &self.sw_info {
            info.set_text(Self::sessions_text(self.sw.open_sessions));
        }
        for (item, key) in &self.sw_items {
            if let Some(e) = self.sw.entries.iter().find(|e| &e.key == key) {
                item.set_text(Self::entry_text(e, &accounts));
            }
            item.set_checked(self.sw.active.as_deref() == Some(key.as_str()));
        }
        let local = local_account();
        let by_id: HashMap<&str, &Value> = accounts.iter().map(|a| (s(a, "id"), a)).collect();
        for (item, id, key) in &self.lines {
            let Some(a) = by_id.get(id.as_str()) else { continue };
            match key {
                None => item.set_text(Self::account_title(a, is_local(a, &local))),
                Some(k) => {
                    if let Some(w) = a.get("windows").and_then(Value::as_array).and_then(|ws| ws.iter().find(|w| s(w, "key") == k)) {
                        let used = f(w, "utilization").unwrap_or(0.0);
                        item.set_text(window_line(w));
                        item.set_icon(icon(draw::menu_bar(used, draw::level_color(used))));
                    }
                }
            }
        }
        for (item, id) in &self.extras {
            if let Some(a) = by_id.get(id.as_str()) {
                item.set_text(Self::extra_text(a));
            }
        }
        if let Some(fx) = &self.fixed {
            fx.header.set_text(self.header_text(&accounts));
            fx.updated.set_text(match self.last_ok {
                Some(t) => {
                    let measured = accounts.iter().filter_map(|a| i(a, "fetchedAt")).max();
                    format!(
                        "{} {}{}",
                        l("Synchro", "Synced"),
                        fmt_ago(t),
                        measured.map(|m| format!(" · {} {}", l("mesure", "measured"), fmt_ago(m))).unwrap_or_default()
                    )
                }
                None => l("Connexion au serveur VibeRooms…", "Connecting to the VibeRooms server…").into(),
            });
        }

        // Icon: one column per account (headroom), coloured by usage.
        let gauges: Vec<(f64, f64, bool)> = accounts
            .iter()
            .map(|a| {
                let used = a
                    .get("windows")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|w| matches!(s(w, "kind"), "session" | "week"))
                    .filter_map(|w| f(w, "utilization"))
                    .fold(0.0, f64::max);
                let err = matches!(s(a, "status"), "reauth" | "error" | "pending") || f(a, "headroom").is_none();
                (f(a, "headroom").unwrap_or(0.0), used, err)
            })
            .collect();
        let unreachable = self.last_err.is_some() && self.last_ok.is_none_or(|t| now_ms() - t > 90_000);
        if let Some(tray) = &self.tray {
            let c = draw::tray_icon(&gauges);
            if let Ok(ic) = tray_icon::Icon::from_rgba(c.px, c.w, c.h) {
                let _ = tray.set_icon(Some(ic));
            }
            let mine = accounts.iter().find(|a| is_local(a, &local));
            let title =
                if unreachable { Some("⚠︎".to_string()) } else { mine.and_then(|a| f(a, "headroom")).map(|h| format!("{:.0}%", h)) };
            tray.set_title(title.as_deref());
            let tip = accounts
                .iter()
                .map(|a| {
                    format!(
                        "{} {}",
                        s(a, "label"),
                        f(a, "headroom").map(|h| format!("{h:.0} % {}", l("dispo", "left"))).unwrap_or_else(|| "—".into())
                    )
                })
                .collect::<Vec<_>>()
                .join(" · ");
            let _ = tray.set_tooltip(Some(format!("VibeRooms — {tip}")));
        }
    }

    fn on_data(&mut self, res: Result<Value, String>) {
        match res {
            Ok(v) => {
                write_cache(&v);
                self.notifier.check(&v, self.cfg.notifications);
                self.suggest_switch(&v);
                self.state = Some(v);
                self.last_ok = Some(now_ms());
                self.last_err = None;
            }
            Err(e) => self.last_err = Some(e),
        }
        self.refresh_texts();
    }

    /// This Mac's account at ≥ 95 % (session or week) while another remembered account has headroom: suggest it.
    fn suggest_switch(&mut self, state: &Value) {
        if !self.cfg.notifications || !self.cfg.switcher {
            return;
        }
        let accounts = state.get("accounts").and_then(Value::as_array).cloned().unwrap_or_default();
        let local = local_account();
        let Some(mine) = accounts.iter().find(|a| is_local(a, &local)) else { return };
        let hot = mine
            .get("windows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|w| matches!(s(w, "kind"), "session" | "week"))
            .find(|w| f(w, "utilization").unwrap_or(0.0) >= 95.0)
            .cloned();
        let Some(w) = hot else {
            if mine.get("headroom").and_then(Value::as_f64).unwrap_or(0.0) > 10.0 {
                self.suggested = None; // back down: may suggest again
            }
            return;
        };
        let cycle = format!("{}:{}:{}", s(mine, "id"), s(&w, "key"), i(&w, "resetsAt").unwrap_or(0));
        if self.suggested.as_deref() == Some(cycle.as_str()) {
            return;
        }
        let best = self
            .sw
            .entries
            .iter()
            .filter(|e| !e.expired() && self.sw.active.as_deref() != Some(e.key.as_str()))
            .filter_map(|e| {
                let (label, head) = Self::entry_info(e, &accounts);
                head.map(|h| (label, h))
            })
            .filter(|(_, h)| *h >= 20.0)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((label, head)) = best {
            let used = f(&w, "utilization").unwrap_or(0.0);
            if fr() {
                notify(
                    &format!("{} à {used:.0} %", s(mine, "label")),
                    &format!("{label} a {head:.0} % dispo : bascule depuis le menu ⇄ (pense à relancer tes sessions)."),
                );
            } else {
                notify(
                    &format!("{} at {used:.0} %", s(mine, "label")),
                    &format!("{label} has {head:.0} % left: switch from the ⇄ menu (restart your sessions)."),
                );
            }
            self.suggested = Some(cycle);
        }
    }

    fn on_menu(&mut self, id: &MenuId) -> bool {
        if let Some((item, key)) = self.sw_items.iter().find(|(i, _)| i.id() == id) {
            item.set_checked(self.sw.active.as_deref() == Some(key.as_str())); // the real state arrives after the switch
            if self.sw.active.as_deref() != Some(key.as_str()) {
                let _ = self.sw_tx.send(switcher::SwCmd::Switch(key.clone()));
            }
            return false;
        }
        if let Some((_, key)) = self.sw_shortcut.as_ref().filter(|(i, _)| i.id() == id) {
            let _ = self.sw_tx.send(switcher::SwCmd::Switch(key.clone()));
            return false;
        }
        if self.sw_add.as_ref().is_some_and(|i| i.id() == id) {
            let _ = self.sw_tx.send(switcher::SwCmd::Enroll);
            return false;
        }
        if let Some((_, key)) = self.sw_forget.iter().find(|(i, _)| i.id() == id) {
            let _ = self.sw_tx.send(switcher::SwCmd::Forget(key.clone()));
            return false;
        }
        let Some(fx) = &self.fixed else { return false };
        if id == fx.quit.id() {
            return true;
        }
        if id == fx.refresh.id() {
            let _ = self.tx.send(Cmd::Refresh);
        } else if id == fx.notif.id() {
            self.cfg.notifications = fx.notif.is_checked();
            self.cfg.save();
        } else if id == fx.login.id() {
            set_login_item(fx.login.is_checked());
            fx.login.set_checked(agent_plist().exists());
        } else {
            // Header, accounts, gauges: open the dashboard.
            let _ = std::process::Command::new("/usr/bin/open").arg(&self.cfg.server).spawn();
        }
        false
    }
}

fn set_login_item(on: bool) {
    let path = agent_plist();
    if !on {
        // No `launchctl bootout`: it would kill the running instance. Takes effect at next login.
        let _ = std::fs::remove_file(path);
        return;
    }
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>{exe}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Interactive</string>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
</dict>
</plist>
"#
    );
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(path, plist);
}

// ---------- Local cache + Claude Code status line ----------
fn cache_path() -> PathBuf {
    support_dir().join("state.json")
}

fn write_cache(state: &Value) {
    let v = serde_json::json!({ "savedAt": now_ms(), "state": state });
    let tmp = support_dir().join("state.json.tmp");
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, cache_path());
    }
}

fn short_dur(ms: i64) -> String {
    let m = (ms.max(0) as f64 / 60_000.0).round() as i64;
    if m < 60 {
        format!("{m}min")
    } else if m < 48 * 60 {
        format!("{}h{:02}", m / 60, m % 60)
    } else {
        format!("{}{}", m / 1440, l("j", "d"))
    }
}

fn ansi_level(u: f64) -> &'static str {
    if u >= 85.0 {
        "31"
    } else if u >= 60.0 {
        "33"
    } else {
        "32"
    }
}

/// Output of an existing status line command (same stdin), 1.5 s max.
fn run_wrapped(cmd: &str, input: &str) -> Option<String> {
    let exe = cmd.split_whitespace().next()?;
    if exe.starts_with('/') && !std::path::Path::new(exe).exists() {
        return None; // uninstalled tool: ignore it
    }
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    {
        use std::io::Write;
        let _ = child.stdin.take()?.write_all(input.as_bytes());
    }
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < Duration::from_millis(1500) => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
    let mut out = String::new();
    use std::io::Read;
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let line = out.lines().next().unwrap_or("").trim_end().to_string();
    (!line.is_empty()).then_some(line)
}

/// `--statusline [--wrap <command>]`: limits line for Claude Code's status line (reads the local cache).
fn statusline(wrap: Option<&str>) {
    use std::io::Read;
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let session: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let mut parts: Vec<String> = vec![];
    match wrap.and_then(|c| run_wrapped(c, &input)) {
        Some(prev) => parts.push(prev),
        None => {
            if let Some(m) = session.pointer("/model/display_name").and_then(Value::as_str) {
                parts.push(format!("\x1b[2m{m}\x1b[0m"));
            }
        }
    }
    let cache: Value = std::fs::read_to_string(cache_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    // The status line belongs to Claude Code: only Claude accounts.
    let accounts: Vec<Value> = cache
        .pointer("/state/accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|a| s(a, "provider") == "claude")
        .cloned()
        .collect();
    if accounts.is_empty() {
        parts.push(format!("\x1b[2m{}\x1b[0m", l("quotas indisponibles", "limits unavailable")));
        println!("{}", parts.join(" │ "));
        return;
    }
    let local = local_account();
    let mine = accounts.iter().find(|a| is_local(a, &local));
    let mut seg: Vec<String> = vec![];
    if let Some(a) = mine {
        let mut t = format!("\x1b[1m{}\x1b[0m", s(a, "label"));
        for w in a.get("windows").and_then(Value::as_array).into_iter().flatten() {
            let u = f(w, "utilization").unwrap_or(0.0);
            match s(w, "kind") {
                "session" => {
                    let reset = i(w, "resetsAt").map(|r| format!(" ↻{}", short_dur(r - now_ms()))).unwrap_or_default();
                    t.push_str(&format!(" \x1b[{}m{u:.0}%\x1b[0m{reset}", ansi_level(u)));
                }
                "week" => t.push_str(&format!(" · {} \x1b[{}m{u:.0}%\x1b[0m", l("sem", "wk"), ansi_level(u))),
                _ => {}
            }
        }
        seg.push(t);
    }
    for a in accounts.iter().filter(|a| mine.is_none_or(|m| s(m, "id") != s(a, "id"))) {
        if let Some(h) = f(a, "headroom") {
            seg.push(format!("{} \x1b[{}m{h:.0}% {}\x1b[0m", s(a, "label"), ansi_level(100.0 - h), l("dispo", "left")));
        }
    }
    // Switch hint when this Mac's account is almost exhausted.
    if let (Some(m), Some(rec)) = (mine, accounts.iter().find(|a| a.get("recommended").and_then(Value::as_bool) == Some(true))) {
        let mh = f(m, "headroom").unwrap_or(100.0);
        let rh = f(rec, "headroom").unwrap_or(0.0);
        if s(rec, "id") != s(m, "id") && mh < 15.0 && rh > mh + 20.0 {
            seg.push(format!("\x1b[33m⇄ {}\x1b[0m", s(rec, "label")));
        }
    }
    let age = now_ms() - cache.get("savedAt").and_then(Value::as_i64).unwrap_or(0);
    let mut line = seg.join(" │ ");
    if age > 5 * 60_000 {
        line.push_str(&format!(" \x1b[2m(⏸ {})\x1b[0m", fmt_ago(now_ms() - age)));
    }
    parts.push(line);
    println!("{}", parts.join(" │ "));
}

/// `viberooms-bar --dump <dir>`: exports icon renders as raw RGBA (visual check).
fn dump(dir: &str) {
    let _ = std::fs::create_dir_all(dir);
    let save = |name: &str, c: draw::Canvas| {
        let _ = std::fs::write(format!("{dir}/{name}-{}x{}.rgba", c.w, c.h), &c.px);
    };
    save("tray", draw::tray_icon(&[(78.0, 22.0, false), (28.0, 72.0, false), (91.0, 9.0, false)]));
    save("tray-crit", draw::tray_icon(&[(5.0, 95.0, false), (0.0, 100.0, false), (40.0, 60.0, true)]));
    save("tray-empty", draw::tray_icon(&[]));
    save("bar-19", draw::menu_bar(19.0, draw::level_color(19.0)));
    save("bar-72", draw::menu_bar(72.0, draw::level_color(72.0)));
    save("bar-96", draw::menu_bar(96.0, draw::level_color(96.0)));
    save("swatch", draw::swatch(draw::account_color("acc-2")));
}

/// Command line: `--statusline`, `--status` (vault accounts) and `--switch <e-mail fragment>`.
fn cli(args: &[String]) -> Option<i32> {
    if args.len() > 1 {
        init_lang(&Config::load().lang);
    }
    match args {
        [_, flag, dir] if flag == "--dump" => {
            dump(dir);
            Some(0)
        }
        [_, flag, rest @ ..] if flag == "--statusline" => {
            let wrap = match rest {
                [w, cmd] if w == "--wrap" => Some(cmd.as_str()),
                _ => None,
            };
            statusline(wrap);
            Some(0)
        }
        [_, flag] if flag == "--status" => {
            let mut sw = switcher::Switcher::new();
            sw.tick();
            let snap = sw.snapshot();
            for e in &snap.entries {
                let mark = if snap.active.as_deref() == Some(e.key.as_str()) { "●" } else { " " };
                let exp = e
                    .refresh_expires_at
                    .map(|t| format!("{} {}", l("expire dans", "expires in"), fmt_dur(t - now_ms())))
                    .unwrap_or_default();
                println!("{mark} {} ({}) {exp}", e.email, e.org_name.as_deref().unwrap_or("-"));
            }
            if snap.active.is_none() {
                println!("{}", l("Session active non identifiée.", "Active session not identified."));
            }
            Some(0)
        }
        [_, flag, who] if flag == "--switch" => {
            let mut sw = switcher::Switcher::new();
            sw.tick();
            let who = who.to_lowercase();
            let matches: Vec<_> = sw
                .snapshot()
                .entries
                .into_iter()
                .filter(|e| e.email.to_lowercase().contains(&who) || e.org_name.as_deref().unwrap_or("").to_lowercase().contains(&who))
                .collect();
            if matches.len() != 1 {
                if fr() {
                    eprintln!("{} compte(s) correspondent à « {who} » ; précise.", matches.len())
                } else {
                    eprintln!("{} account(s) match “{who}”; be more specific.", matches.len())
                }
                return Some(2);
            }
            match sw.switch(&matches[0].key) {
                Ok(email) => {
                    let n = switcher::open_sessions();
                    if fr() {
                        println!(
                            "Claude Code utilise maintenant {email}.{}",
                            if n > 0 { format!(" {n} session(s) claude ouverte(s) à relancer.") } else { String::new() }
                        );
                    } else {
                        println!(
                            "Claude Code now uses {email}.{}",
                            if n > 0 { format!(" {n} open claude session(s) to restart.") } else { String::new() }
                        );
                    }
                    Some(0)
                }
                Err(e) => {
                    eprintln!("{} {e}", l("Bascule impossible :", "Switch failed:"));
                    Some(1)
                }
            }
        }
        _ => None,
    }
}

fn main() {
    if let Some(code) = cli(&std::env::args().collect::<Vec<_>>()) {
        std::process::exit(code);
    }
    // Single instance: the local port acts as a lock.
    let _lock = match std::net::TcpListener::bind(("127.0.0.1", LOCK_PORT)) {
        Ok(lk) => lk,
        Err(_) => {
            eprintln!("VibeRooms bar is already running.");
            return;
        }
    };
    let cfg = Config::load();
    init_lang(&cfg.lang);
    if !support_dir().join("config.json").exists() {
        cfg.save();
    }

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = proxy.send_event(UserEvent::Menu(e));
    }));

    let (sw_tx, sw_rx) = mpsc::channel();
    if cfg.switcher {
        let proxy = event_loop.create_proxy();
        std::thread::spawn(move || switcher_loop(proxy, sw_rx));
    }
    let (tx, rx) = mpsc::channel();
    {
        let proxy = event_loop.create_proxy();
        let server = cfg.server.clone();
        std::thread::spawn(move || fetcher(server, proxy, rx));
    }
    {
        let proxy = event_loop.create_proxy();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(15));
            if proxy.send_event(UserEvent::Tick).is_err() {
                return;
            }
        });
    }

    let mut ui = Ui {
        cfg,
        tray: None,
        fixed: None,
        signature: String::new(),
        lines: vec![],
        extras: vec![],
        state: None,
        last_ok: None,
        last_err: None,
        notifier: Notifier::default(),
        tx,
        sw_tx,
        sw: switcher::Snapshot::default(),
        sw_sub: None,
        sw_items: vec![],
        sw_add: None,
        sw_forget: vec![],
        sw_shortcut: None,
        sw_info: None,
        suggested: None,
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                let c = draw::tray_icon(&[]);
                let tray = TrayIconBuilder::new()
                    .with_icon(tray_icon::Icon::from_rgba(c.px, c.w, c.h).expect("icon"))
                    .with_tooltip("VibeRooms")
                    .build()
                    .expect("creating the menu bar icon");
                ui.tray = Some(tray);
                ui.rebuild();
            }
            Event::UserEvent(UserEvent::Data(res)) => ui.on_data(res),
            Event::UserEvent(UserEvent::Tick) => ui.refresh_texts(),
            Event::UserEvent(UserEvent::Switcher(snap)) => {
                ui.sw = snap;
                ui.refresh_texts();
            }
            Event::UserEvent(UserEvent::SwitchResult(res)) => match res {
                Ok(email) => {
                    let accounts = ui.accounts();
                    let label = ui.sw.entries.iter().find(|e| e.email == email).map(|e| Ui::entry_info(e, &accounts).0).unwrap_or(email);
                    let n = switcher::open_sessions();
                    let tail = match (n, fr()) {
                        (0, true) => "Les prochaines sessions claude l'utiliseront.".to_string(),
                        (0, false) => "New claude sessions will use it.".to_string(),
                        (1, true) => "1 session claude ouverte garde l'ancien compte : relance-la.".to_string(),
                        (1, false) => "1 open claude session keeps the previous account: restart it.".to_string(),
                        (n, true) => format!("{n} sessions claude ouvertes gardent l'ancien compte : relance-les."),
                        (n, false) => format!("{n} open claude sessions keep the previous account: restart them."),
                    };
                    if fr() {
                        notify("Claude Code", &format!("Ce Mac utilise maintenant {label}. {tail}"));
                    } else {
                        notify("Claude Code", &format!("This Mac now uses {label}. {tail}"));
                    }
                    ui.refresh_texts();
                }
                Err(e) => notify(l("Bascule impossible", "Switch failed"), &e),
            },
            Event::UserEvent(UserEvent::Menu(e)) if ui.on_menu(e.id()) => {
                ui.tray.take();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
