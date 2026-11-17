//! Claude Code sessions of this Mac, one per account, each opened in a terminal in one click.
//!
//! A session is a Claude Code config directory: `~/.claude` (the default one) or `~/.claude-<name>`, run
//! with `CLAUDE_CONFIG_DIR`. Claude Code keeps the sign-in of each directory apart (its own keychain item),
//! so several accounts run side by side and nothing is swapped. Nothing here reads or writes a token: the
//! account of a session comes from `oauthAccount` in its `.claude.json`, and signing in happens in Claude
//! Code itself.

use crate::{fr, home, l, notify};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Entries of `~/.claude` linked into the sessions VibeRooms creates, so that settings, instructions,
/// skills and plugins stay the same everywhere. History, projects and memory stay per session.
pub const SHARED: [&str; 9] =
    ["settings.json", "CLAUDE.md", "agents", "commands", "skills", "plugins", "output-styles", "hooks", "keybindings.json"];

// ---------- Sessions ----------

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// Config directory (`~/.claude` for the default session).
    pub dir: PathBuf,
    /// The default session runs without `CLAUDE_CONFIG_DIR`; its account is in `~/.claude.json`.
    pub default: bool,
    /// Signed-in account (None: never signed in).
    pub email: Option<String>,
    pub org_uuid: Option<String>,
}

impl Session {
    fn new(dir: PathBuf) -> Self {
        Self { dir, default: false, email: None, org_uuid: None }
    }

    /// "~/.claude-perso".
    pub fn short_dir(&self) -> String {
        tilde(&self.dir)
    }
}

/// "~/…" for a path under the home directory.
pub fn tilde(p: &Path) -> String {
    match p.strip_prefix(home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

/// `~/x` → `$HOME/x`.
pub fn expand(p: &str) -> PathBuf {
    match p.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => home().join(rest.trim_start_matches('/')),
        _ => PathBuf::from(p),
    }
}

/// The session whose Claude Code config file is `json` (None when Claude Code never ran there).
fn read_session(dir: PathBuf, json: &Path, default: bool) -> Option<Session> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(json).ok()?).ok()?;
    let oa = v.get("oauthAccount");
    let g = |k: &str| oa.and_then(|o| o.get(k)).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    Some(Session { dir, default, email: g("emailAddress"), org_uuid: g("organizationUuid") })
}

/// Session at `dir`. Claude Code rewrites `.claude.json` constantly: a file caught mid-write keeps the
/// previous reading, so the session does not blink out of the menu.
fn session_at(dir: PathBuf, json: &Path, default: bool, previous: &[Session]) -> Option<Session> {
    if !json.exists() {
        return None;
    }
    read_session(dir.clone(), json, default).or_else(|| previous.iter().find(|p| p.dir == dir).cloned())
}

/// Sessions of this Mac: `~/.claude` (account in `~/.claude.json`), then every `~/.claude-*` directory
/// where Claude Code has run (it holds a `.claude.json`). Other tools' `~/.claude-*` folders are skipped.
/// `previous` is the last result, used for files that cannot be read right now.
pub fn discover(home: &Path, previous: &[Session]) -> Vec<Session> {
    let mut out: Vec<Session> = session_at(home.join(".claude"), &home.join(".claude.json"), true, previous).into_iter().collect();
    let mut others: Vec<Session> = std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.len() > ".claude-".len() && n.starts_with(".claude-")))
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|p| {
            let json = p.join(".claude.json");
            session_at(p, &json, false, previous)
        })
        .collect();
    others.sort_by(|a, b| a.dir.cmp(&b.dir));
    out.extend(others);
    out
}

// ---------- New sessions ----------

/// What a `~/.claude-<name>` directory holds, before starting a new session there.
#[derive(Debug, PartialEq)]
pub enum Slot {
    /// Missing, or prepared by VibeRooms and never used: a new session can start there.
    Free,
    /// A Claude Code session that never signed in: can be reused.
    Unsigned,
    /// A signed-in Claude Code session.
    Signed,
    /// Anything else (another tool's folder): left alone.
    Foreign,
}

pub fn slot(dir: &Path) -> Slot {
    if dir.symlink_metadata().is_err() {
        return Slot::Free;
    }
    if !dir.is_dir() {
        return Slot::Foreign;
    }
    if let Some(s) = read_session(dir.to_path_buf(), &dir.join(".claude.json"), false) {
        return if s.email.is_some() { Slot::Signed } else { Slot::Unsigned };
    }
    let untouched = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().all(|e| e.file_name().to_str().is_some_and(|n| n == ".DS_Store" || SHARED.contains(&n))))
        .unwrap_or(false);
    if untouched {
        Slot::Free
    } else {
        Slot::Foreign
    }
}

/// "Équipe Client X" → "equipe-client-x" (directory name of a session).
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        let c = match c {
            'à' | 'â' | 'ä' | 'á' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.chars().take(40).collect();
    out.trim_end_matches('-').to_string()
}

/// First `~/.claude-<base>`, `~/.claude-<base>-2`… where a new session can start.
pub fn free_dir(home: &Path, base: &str) -> PathBuf {
    let base = if base.is_empty() { "session" } else { base };
    (1..)
        .map(|n| home.join(if n == 1 { format!(".claude-{base}") } else { format!(".claude-{base}-{n}") }))
        .find(|d| matches!(slot(d), Slot::Free | Slot::Unsigned))
        .expect("an unused directory name")
}

/// Creates the session directory (private) and links the shared entries of `~/.claude` into it.
pub fn prepare(home: &Path, dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    if dir.symlink_metadata().is_err() {
        std::fs::DirBuilder::new().mode(0o700).create(dir).map_err(|e| format!("{}: {e}", tilde(dir)))?;
    }
    let src_root = home.join(".claude");
    for name in SHARED {
        let (src, dst) = (src_root.join(name), dir.join(name));
        if src.exists() && dst.symlink_metadata().is_err() {
            std::os::unix::fs::symlink(&src, &dst).map_err(|e| format!("{}: {e}", tilde(&dst)))?;
        }
    }
    Ok(())
}

// ---------- Terminal ----------

#[derive(Clone, Debug, PartialEq)]
pub enum Terminal {
    /// iTerm, new window (`"terminal": "iterm"`).
    ITerm,
    /// iTerm, new tab in the front window (`"iterm-tab"`).
    ITermTab,
    /// Terminal.app (`"terminal"`).
    App,
    /// Any other terminal (`"custom"`): `terminalCommand`, run by /bin/sh, where `{cmd}` stands for the
    /// quoted command line.
    Custom(String),
}

impl Terminal {
    /// From `config.json`; unset or unknown: iTerm when it is installed, else Terminal.
    pub fn from_config(terminal: &str, command: Option<&str>) -> Self {
        match (terminal, command) {
            ("iterm", _) => Self::ITerm,
            ("iterm-tab", _) => Self::ITermTab,
            ("terminal", _) => Self::App,
            ("custom", Some(c)) if !c.trim().is_empty() => Self::Custom(c.to_string()),
            _ if iterm_installed() => Self::ITerm,
            _ => Self::App,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::ITerm => "iterm",
            Self::ITermTab => "iterm-tab",
            Self::App => "terminal",
            Self::Custom(_) => "custom",
        }
    }
}

pub fn iterm_installed() -> bool {
    [PathBuf::from("/Applications/iTerm.app"), home().join("Applications/iTerm.app")].iter().any(|p| p.exists())
}

#[derive(Clone, Debug)]
pub struct Opts {
    pub terminal: Terminal,
    /// Directory the session starts in (None: the terminal's own default).
    pub workdir: Option<PathBuf>,
    /// Ask for the directory at each opening (`workdir` is then the dialog's starting point).
    pub ask_workdir: bool,
}

/// Single-quoted shell word.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A path on the command line: `"$HOME/.claude-perso"` when it reads well, else single-quoted.
fn sh_path(p: &Path, home: &Path) -> String {
    if let Ok(rest) = p.strip_prefix(home) {
        let r = rest.to_string_lossy();
        if r.is_empty() {
            return "\"$HOME\"".into();
        }
        if r.chars().all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c)) {
            return format!("\"$HOME/{r}\"");
        }
    }
    sh_quote(&p.to_string_lossy())
}

/// Command line typed in the terminal: `cd <workdir> && CLAUDE_CONFIG_DIR=<dir> claude`.
pub fn command_line(s: &Session, workdir: Option<&Path>, intro: Option<&str>, home: &Path) -> String {
    let mut parts = vec![];
    if let Some(w) = workdir {
        parts.push(format!("cd {}", sh_path(w, home)));
    }
    if let Some(t) = intro {
        parts.push(format!("printf '%s\\n\\n' {}", sh_quote(t)));
    }
    parts.push(if s.default { "claude".into() } else { format!("CLAUDE_CONFIG_DIR={} claude", sh_path(&s.dir, home)) });
    parts.join(" && ")
}

fn as_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// AppleScript typing `cmd` in a new iTerm window (or tab). On a cold start, iTerm's own first window is
/// used instead of opening a second one.
fn iterm_script(cmd: &str, tab: bool) -> String {
    let new = if tab {
        "set w to current window\n\t\ttell w to create tab with default profile"
    } else {
        "set w to (create window with default profile)"
    };
    format!(
        "set wasRunning to application \"iTerm\" is running
tell application \"iTerm\"
\tactivate
\tif (count of windows) is 0 then
\t\tset w to (create window with default profile)
\telse if (not wasRunning) and (count of windows) is 1 and (count of tabs of current window) is 1 then
\t\tset w to current window
\telse
\t\t{new}
\tend if
\ttell current session of w to write text {}
end tell",
        as_string(cmd)
    )
}

/// AppleScript running `cmd` in a new Terminal.app window (its first window on a cold start).
fn terminal_script(cmd: &str) -> String {
    let c = as_string(cmd);
    format!(
        "set wasRunning to application \"Terminal\" is running
tell application \"Terminal\"
\tactivate
\tif (not wasRunning) and (count of windows) is 1 then
\t\tdo script {c} in window 1
\telse
\t\tdo script {c}
\tend if
end tell"
    )
}

/// Custom terminal command: `{cmd}` replaced by the quoted command line (appended when absent).
fn custom_command(template: &str, cmd: &str) -> String {
    if template.contains("{cmd}") {
        template.replace("{cmd}", &sh_quote(cmd))
    } else {
        format!("{template} {}", sh_quote(cmd))
    }
}

/// What opening a session runs.
#[derive(Debug, PartialEq)]
pub enum Plan {
    AppleScript(String),
    Shell(String),
}

pub fn plan(s: &Session, opts: &Opts, intro: Option<&str>) -> Plan {
    let cmd = command_line(s, opts.workdir.as_deref(), intro, &home());
    match &opts.terminal {
        Terminal::ITerm => Plan::AppleScript(iterm_script(&cmd, false)),
        Terminal::ITermTab => Plan::AppleScript(iterm_script(&cmd, true)),
        Terminal::App => Plan::AppleScript(terminal_script(&cmd)),
        Terminal::Custom(t) => Plan::Shell(custom_command(t, &cmd)),
    }
}

/// Opens the session in a new terminal window; returns once the terminal has taken the command.
pub fn open_now(s: &Session, opts: &Opts, intro: Option<&str>) -> Result<(), String> {
    if let Some(w) = opts.workdir.as_deref().filter(|w| !w.is_dir()) {
        return Err(if fr() {
            format!("le dossier de départ {} n'existe pas : choisis-en un autre dans Sessions Claude Code › Dossier de départ.", tilde(w))
        } else {
            format!("the start folder {} does not exist: pick another one in Claude Code sessions › Start folder.", tilde(w))
        });
    }
    if matches!(opts.terminal, Terminal::ITerm | Terminal::ITermTab) && !iterm_installed() {
        return Err(l(
            "iTerm n'est pas installé : choisis un autre terminal dans le menu Sessions Claude Code › Ouvrir dans.",
            "iTerm is not installed: pick another terminal in Claude Code sessions › Open in.",
        )
        .into());
    }
    match plan(s, opts, intro) {
        Plan::AppleScript(script) => {
            let out = Command::new("/usr/bin/osascript").arg("-e").arg(script).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
            if out.status.success() {
                return Ok(());
            }
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if err.contains("-1743") {
                return Err(l(
                    "autorise VibeRooms à piloter le terminal : Réglages Système › Confidentialité et sécurité › Automatisation.",
                    "allow VibeRooms to control the terminal: System Settings › Privacy & Security › Automation.",
                )
                .into());
            }
            Err(err)
        }
        Plan::Shell(cmd) => run_detached(&cmd),
    }
}

/// Runs a custom terminal command: an error when it fails within 3 s, else it goes on by itself.
fn run_detached(cmd: &str) -> Result<(), String> {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                let out = child.wait_with_output().map_err(|e| e.to_string())?;
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err(if err.is_empty() { format!("terminalCommand: {status}") } else { err });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
    std::thread::spawn(move || {
        let _ = child.wait_with_output(); // drains stderr and reaps the process
    });
    Ok(())
}

/// The options with the start folder settled: asked now when `ask_workdir` is set (None when cancelled).
fn settle(mut opts: Opts) -> Option<Opts> {
    if opts.ask_workdir {
        opts.workdir = Some(choose_folder(opts.workdir.as_deref())?);
        opts.ask_workdir = false;
    }
    Some(opts)
}

/// Same as `open_now`, off the UI thread; a failure becomes a notification.
pub fn open(s: Session, opts: Opts, intro: Option<String>) {
    std::thread::spawn(move || {
        let Some(opts) = settle(opts) else { return };
        if let Err(e) = open_now(&s, &opts, intro.as_deref()) {
            notify(l("Ouverture de Claude Code impossible", "Could not open Claude Code"), &e);
        }
    });
}

fn intro(dir: &Path, email: Option<&str>) -> String {
    let d = tilde(dir);
    match (email, fr()) {
        (Some(e), true) => format!(
            "VibeRooms : nouvelle session {d}. Connecte-toi avec {e} (dans une fenêtre privée si ton navigateur est sur un autre compte)."
        ),
        (Some(e), false) => {
            format!("VibeRooms: new session {d}. Sign in with {e} (in a private window if your browser is on another account).")
        }
        (None, true) => format!(
            "VibeRooms : nouvelle session {d}. Connecte-toi avec le compte voulu (dans une fenêtre privée si ton navigateur est sur un autre compte)."
        ),
        (None, false) => format!(
            "VibeRooms: new session {d}. Sign in with the account you want (in a private window if your browser is on another account)."
        ),
    }
}

fn create_and_open(dir: PathBuf, email: Option<&str>, opts: &Opts) -> Result<(), String> {
    prepare(&home(), &dir)?;
    let text = intro(&dir, email);
    open_now(&Session::new(dir), opts, Some(&text))
}

/// "Set up Claude Code for this account": a new session named after the account, then its sign-in.
pub fn new_for_account(label: String, email: String, opts: Opts) {
    std::thread::spawn(move || {
        let Some(opts) = settle(opts) else { return };
        let dir = free_dir(&home(), &slug(&label));
        let email = Some(email.as_str()).filter(|e| !e.is_empty());
        if let Err(e) = create_and_open(dir, email, &opts) {
            notify(l("Création de la session impossible", "Could not create the session"), &e);
        }
    });
}

/// "New session…": asks for a name, then opens that session (and its sign-in when it is new).
pub fn new_session(opts: Opts) {
    std::thread::spawn(move || {
        let Some(name) = ask_name() else { return };
        let s = slug(&name);
        if s.is_empty() {
            return;
        }
        let Some(opts) = settle(opts) else { return };
        let dir = home().join(format!(".claude-{s}"));
        let res = match slot(&dir) {
            Slot::Free | Slot::Unsigned => create_and_open(dir, None, &opts),
            Slot::Signed => open_now(&Session::new(dir), &opts, None),
            Slot::Foreign => Err(if fr() {
                format!("{} existe déjà et n'est pas une session Claude Code : choisis un autre nom.", tilde(&dir))
            } else {
                format!("{} already exists and is not a Claude Code session: pick another name.", tilde(&dir))
            }),
        };
        if let Err(e) = res {
            notify(l("Création de la session impossible", "Could not create the session"), &e);
        }
    });
}

/// Name of a new session, asked in a dialog (None when cancelled).
fn ask_name() -> Option<String> {
    let (prompt, ok, cancel) = if fr() {
        ("Nom de la nouvelle session Claude Code (ex. perso, client-x). Elle sera créée dans ~/.claude-<nom>.", "Créer", "Annuler")
    } else {
        ("Name of the new Claude Code session (e.g. personal, client-x). It is created in ~/.claude-<name>.", "Create", "Cancel")
    };
    let script = format!(
        "activate\ntext returned of (display dialog {} default answer \"\" with title \"VibeRooms\" buttons {{{}, {}}} default button {} cancel button {})",
        as_string(prompt),
        as_string(cancel),
        as_string(ok),
        as_string(ok),
        as_string(cancel)
    );
    let out = Command::new("/usr/bin/osascript").arg("-e").arg(script).stdin(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// A folder picked in the system dialog, starting from `from` (None when cancelled).
pub fn choose_folder(from: Option<&Path>) -> Option<PathBuf> {
    let prompt = l("Dossier où démarrer Claude Code :", "Folder to start Claude Code in:");
    let at = from.filter(|p| p.is_dir()).map(|p| format!(" default location (POSIX file {})", as_string(&p.to_string_lossy())));
    let script = format!("activate\nPOSIX path of (choose folder with prompt {}{})", as_string(prompt), at.unwrap_or_default());
    let out = Command::new("/usr/bin/osascript").arg("-e").arg(script).stdin(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let path = if path.len() > 1 { path.trim_end_matches('/') } else { &path };
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempHome(PathBuf);
    impl TempHome {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("vr-launch-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn write(&self, rel: &str, text: &str) {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
    }
    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const SIGNED: &str = r#"{"numStartups": 3, "oauthAccount": {"emailAddress": "me@example.com", "organizationUuid": "org-1"}}"#;

    #[test]
    fn discovers_default_then_named_sessions() {
        let h = TempHome::new("discover");
        h.write(".claude.json", SIGNED);
        h.write(".claude/settings.json", "{}");
        h.write(".claude-work/.claude.json", r#"{"oauthAccount": {"emailAddress": "work@example.com"}}"#);
        h.write(".claude-fresh/.claude.json", r#"{"numStartups": 1}"#);
        h.write(".claude-code-router/config.json", "{}"); // another tool
        h.write(".claude.json.bak", SIGNED); // a file, not a session
        let found = discover(&h.0, &[]);
        let dirs: Vec<String> = found.iter().map(|s| s.dir.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(dirs, [".claude", ".claude-fresh", ".claude-work"]);
        assert!(found[0].default);
        assert_eq!(found[0].email.as_deref(), Some("me@example.com"));
        assert_eq!(found[0].org_uuid.as_deref(), Some("org-1"));
        assert_eq!(found[1].email, None);
        assert_eq!(found[2].email.as_deref(), Some("work@example.com"));
        assert!(!found[2].default);

        // A file caught mid-write keeps the previous reading; a deleted one drops the session.
        h.write(".claude-work/.claude.json", r#"{"oauthAccount": {"emailAd"#);
        std::fs::remove_file(h.0.join(".claude-fresh/.claude.json")).unwrap();
        let again = discover(&h.0, &found);
        assert_eq!(again.len(), 2);
        assert_eq!(again[1], found[2]);
        assert!(discover(&h.0, &[]).iter().all(|s| s.default));
    }

    #[test]
    fn slots_and_free_dirs() {
        let h = TempHome::new("slots");
        h.write(".claude/settings.json", "{}");
        assert_eq!(slot(&h.0.join(".claude-new")), Slot::Free);
        h.write(".claude-signed/.claude.json", SIGNED);
        assert_eq!(slot(&h.0.join(".claude-signed")), Slot::Signed);
        h.write(".claude-unsigned/.claude.json", "{}");
        assert_eq!(slot(&h.0.join(".claude-unsigned")), Slot::Unsigned);
        h.write(".claude-other/config.json", "{}");
        assert_eq!(slot(&h.0.join(".claude-other")), Slot::Foreign);
        // Prepared by VibeRooms, never started: still free.
        prepare(&h.0, &h.0.join(".claude-prepared")).unwrap();
        assert_eq!(slot(&h.0.join(".claude-prepared")), Slot::Free);

        assert_eq!(free_dir(&h.0, "signed"), h.0.join(".claude-signed-2"));
        assert_eq!(free_dir(&h.0, "other"), h.0.join(".claude-other-2"));
        assert_eq!(free_dir(&h.0, "unsigned"), h.0.join(".claude-unsigned"));
        assert_eq!(free_dir(&h.0, ""), h.0.join(".claude-session"));
    }

    #[test]
    fn prepare_links_only_existing_shared_entries() {
        let h = TempHome::new("prepare");
        h.write(".claude/settings.json", r#"{"model": "opus"}"#);
        h.write(".claude/skills/a/SKILL.md", "x");
        h.write(".claude/history.jsonl", "private");
        let dir = h.0.join(".claude-perso");
        prepare(&h.0, &dir).unwrap();
        prepare(&h.0, &dir).unwrap(); // idempotent
        assert_eq!(std::fs::read_link(dir.join("settings.json")).unwrap(), h.0.join(".claude/settings.json"));
        assert!(dir.join("skills/a/SKILL.md").exists());
        assert!(dir.join("agents").symlink_metadata().is_err());
        assert!(dir.join("history.jsonl").symlink_metadata().is_err());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("MSDS-PERSO2"), "msds-perso2");
        assert_eq!(slug("  Équipe Client X  "), "equipe-client-x");
        assert_eq!(slug("a / b"), "a-b");
        assert_eq!(slug("!!!"), "");
        assert_eq!(slug(&"x".repeat(60)).len(), 40);
    }

    #[test]
    fn command_lines() {
        let home = Path::new("/Users/me");
        let named = Session::new(home.join(".claude-perso"));
        let default = Session { default: true, ..Session::new(home.join(".claude")) };
        assert_eq!(command_line(&default, None, None, home), "claude");
        assert_eq!(command_line(&named, None, None, home), r#"CLAUDE_CONFIG_DIR="$HOME/.claude-perso" claude"#);
        assert_eq!(
            command_line(&named, Some(&home.join("Sites/my app")), Some("It's new"), home),
            r#"cd '/Users/me/Sites/my app' && printf '%s\n\n' 'It'\''s new' && CLAUDE_CONFIG_DIR="$HOME/.claude-perso" claude"#
        );
        assert_eq!(command_line(&default, Some(home), None, home), r#"cd "$HOME" && claude"#);
        let odd = Session::new(home.join(".claude-a b"));
        assert_eq!(command_line(&odd, None, None, home), "CLAUDE_CONFIG_DIR='/Users/me/.claude-a b' claude");
    }

    #[test]
    fn scripts_escape_the_command() {
        let cmd = r#"cd "$HOME" && claude"#;
        let s = iterm_script(cmd, false);
        assert!(s.contains(r#"write text "cd \"$HOME\" && claude""#));
        assert!(s.contains("create window with default profile"));
        assert!(!s.contains("create tab"));
        assert!(iterm_script(cmd, true).contains("create tab with default profile"));
        assert!(terminal_script(r"a\b").contains(r#"do script "a\\b""#));
        assert_eq!(custom_command("wezterm start -- zsh -lic {cmd}", "claude"), "wezterm start -- zsh -lic 'claude'");
        assert_eq!(custom_command("my-term -e", "claude"), "my-term -e 'claude'");
    }

    #[test]
    fn terminal_from_config() {
        assert_eq!(Terminal::from_config("iterm-tab", None), Terminal::ITermTab);
        assert_eq!(Terminal::from_config("terminal", Some("x")), Terminal::App);
        assert_eq!(Terminal::from_config("custom", Some("x {cmd}")), Terminal::Custom("x {cmd}".into()));
        // "custom" without a command falls back to the default terminal.
        assert_ne!(Terminal::from_config("custom", None).id(), "custom");
    }
}
