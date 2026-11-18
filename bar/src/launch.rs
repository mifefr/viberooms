//! Sessions of this Mac, one per account and per coding tool, each opened in a terminal in one click.
//!
//! A session is a config directory of a tool: its default one (`~/.claude`, `~/.codex`…) or
//! `~/.<tool>-<name>`, run with the tool's own variable (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`…). Each tool keeps
//! the sign-in of each directory apart, so several accounts run side by side and nothing is swapped. Nothing
//! here reads or writes a token: the account of a Claude Code session comes from `oauthAccount` in its
//! `.claude.json`; for other tools, whose sign-in files hold tokens, the account is linked in VibeRooms
//! (`links` in config.json). Signing in happens in the tool itself.

use crate::{fr, home, l, notify};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// ---------- Tools ----------

/// How a tool tells which account a session is signed in to.
#[derive(Clone, Debug, PartialEq)]
pub enum Identity {
    /// `oauthAccount` in the session's `.claude.json` (`~/.claude.json` for the default one).
    ClaudeJson,
    /// Linked in VibeRooms (`links` in config.json).
    Linked,
}

/// A coding tool run in the terminal: Claude Code, Codex, or one declared in config.json (`tools`).
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    /// "claude": named sessions live in `~/.claude-<name>`.
    pub id: String,
    /// "Claude Code".
    pub name: String,
    /// Command typed in the terminal ("claude").
    pub command: String,
    /// Variable giving the config directory ("CLAUDE_CONFIG_DIR").
    pub env: String,
    /// Default config directory, used without the variable (`~/.claude`).
    pub dir: PathBuf,
    /// Provider of the server-side accounts it uses ("claude"); None: not tied to accounts.
    pub provider: Option<String>,
    /// Entries of the default directory linked into the sessions VibeRooms creates, so that settings,
    /// instructions and skills stay the same everywhere. History and sign-in stay per session.
    pub shared: Vec<String>,
    pub identity: Identity,
    /// Declared in config.json: listed even before it has a session.
    pub declared: bool,
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

impl Tool {
    pub fn claude(home: &Path) -> Self {
        Self {
            id: "claude".into(),
            name: "Claude Code".into(),
            command: "claude".into(),
            env: "CLAUDE_CONFIG_DIR".into(),
            dir: home.join(".claude"),
            provider: Some("claude".into()),
            shared: names(&[
                "settings.json",
                "CLAUDE.md",
                "agents",
                "commands",
                "skills",
                "plugins",
                "output-styles",
                "hooks",
                "keybindings.json",
            ]),
            identity: Identity::ClaudeJson,
            declared: false,
        }
    }

    pub fn codex(home: &Path) -> Self {
        Self {
            id: "codex".into(),
            name: "Codex".into(),
            command: "codex".into(),
            env: "CODEX_HOME".into(),
            dir: home.join(".codex"),
            provider: Some("codex".into()),
            shared: names(&["config.toml", "AGENTS.md", "prompts", "skills"]),
            identity: Identity::Linked,
            declared: false,
        }
    }

    /// Prefix of the named session directories: ".claude-".
    fn prefix(&self) -> String {
        format!(".{}-", self.id)
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn valid_env(env: &str) -> bool {
    env.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && env.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Claude Code and Codex, then the tools of config.json (`tools`). An entry with the id of a built-in tool
/// changes it (e.g. its `command`); a new one needs an `id`, a `command` and an `env`. Invalid entries are
/// skipped.
pub fn tools(home: &Path, declared: &Value) -> Vec<Tool> {
    let mut out = vec![Tool::claude(home), Tool::codex(home)];
    for d in declared.as_array().into_iter().flatten() {
        let g = |k: &str| d.get(k).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
        let Some(id) = g("id").map(|i| i.to_lowercase()).filter(|i| valid_id(i)) else { continue };
        let shared = d.get("shared").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect());
        let existing = out.iter().position(|t| t.id == id);
        let base = match existing {
            Some(i) => out[i].clone(),
            None => {
                let (Some(command), Some(env)) = (g("command"), g("env")) else { continue };
                Tool {
                    name: id.clone(),
                    dir: home.join(format!(".{id}")),
                    id,
                    command,
                    env,
                    provider: None,
                    shared: vec![],
                    identity: Identity::Linked,
                    declared: true,
                }
            }
        };
        let t = Tool {
            name: g("name").unwrap_or(base.name),
            command: g("command").unwrap_or(base.command),
            env: g("env").unwrap_or(base.env),
            dir: g("dir").map(|p| expand_in(home, &p)).unwrap_or(base.dir),
            provider: g("provider").or(base.provider),
            shared: shared.unwrap_or(base.shared),
            declared: true,
            ..base
        };
        if !valid_env(&t.env) {
            continue;
        }
        match existing {
            Some(i) => out[i] = t,
            None => out.push(t),
        }
    }
    out
}

// ---------- Sessions ----------

/// Account a session of a `Linked` tool is tied to.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub email: String,
    pub org: Option<String>,
}

/// `links` of config.json: session directory (`~/.codex-perso`) → account.
pub type Links = BTreeMap<String, Link>;

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub tool: Tool,
    /// Config directory (the tool's own `dir` for the default session).
    pub dir: PathBuf,
    /// The default session runs without the tool's variable.
    pub default: bool,
    /// Account it is signed in to, or linked to (None: unknown).
    pub email: Option<String>,
    pub org_uuid: Option<String>,
}

impl Session {
    pub fn new(tool: &Tool, dir: PathBuf) -> Self {
        Self { tool: tool.clone(), dir, default: false, email: None, org_uuid: None }
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
    expand_in(&home(), p)
}

fn expand_in(home: &Path, p: &str) -> PathBuf {
    match p.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(p),
    }
}

/// The Claude Code session whose config file is `json` (None when Claude Code never ran there).
fn read_claude(tool: &Tool, dir: PathBuf, json: &Path, default: bool) -> Option<Session> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(json).ok()?).ok()?;
    let oa = v.get("oauthAccount");
    let g = |k: &str| oa.and_then(|o| o.get(k)).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    Some(Session { tool: tool.clone(), dir, default, email: g("emailAddress"), org_uuid: g("organizationUuid") })
}

/// True when `dir` holds nothing but the shared entries VibeRooms links (the tool never ran there).
fn untouched(tool: &Tool, dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().all(|e| e.file_name().to_str().is_some_and(|n| n == ".DS_Store" || tool.shared.iter().any(|s| s == n))))
        .unwrap_or(false)
}

/// Session of `tool` at `dir`, None when the tool never ran there. Claude Code rewrites `.claude.json`
/// constantly: a file caught mid-write keeps the previous reading, so the session does not blink out of the
/// menu.
fn session_at(tool: &Tool, dir: PathBuf, default: bool, links: &Links, previous: &[Session]) -> Option<Session> {
    match tool.identity {
        Identity::ClaudeJson => {
            let json = if default { dir.parent().unwrap_or(&dir).join(".claude.json") } else { dir.join(".claude.json") };
            if !json.exists() {
                return None;
            }
            read_claude(tool, dir.clone(), &json, default)
                .or_else(|| previous.iter().find(|p| p.dir == dir && p.tool.id == tool.id).cloned())
        }
        Identity::Linked => {
            if !dir.is_dir() || (!default && untouched(tool, &dir)) {
                return None;
            }
            let link = links.get(&tilde(&dir));
            Some(Session {
                tool: tool.clone(),
                dir,
                default,
                email: link.map(|l| l.email.clone()),
                org_uuid: link.and_then(|l| l.org.clone()),
            })
        }
    }
}

/// Sessions of this Mac, tool by tool: the default one, then every `~/.<tool>-*` directory where the tool
/// has run (for Claude Code, it holds a `.claude.json`: other tools' `~/.claude-*` folders are skipped).
/// `previous` is the last result, used for files that cannot be read right now.
pub fn discover(home: &Path, tools: &[Tool], links: &Links, previous: &[Session]) -> Vec<Session> {
    let entries: Vec<PathBuf> = std::fs::read_dir(home).into_iter().flatten().flatten().map(|e| e.path()).collect();
    let mut out = vec![];
    for tool in tools {
        out.extend(session_at(tool, tool.dir.clone(), true, links, previous));
        let prefix = tool.prefix();
        let mut named: Vec<Session> = entries
            .iter()
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.len() > prefix.len() && n.starts_with(&prefix)))
            .filter(|p| p.is_dir() && **p != tool.dir)
            .filter_map(|p| session_at(tool, p.clone(), false, links, previous))
            .collect();
        named.sort_by(|a, b| a.dir.cmp(&b.dir));
        out.extend(named);
    }
    out
}

// ---------- New sessions ----------

/// What a `~/.<tool>-<name>` directory holds, before starting a new session there.
#[derive(Debug, PartialEq)]
pub enum Slot {
    /// Missing, or prepared by VibeRooms and never used: a new session can start there.
    Free,
    /// A Claude Code session that never signed in: can be reused.
    Unsigned,
    /// A session in use (signed in, for Claude Code).
    Signed,
    /// Anything else (another tool's folder): left alone.
    Foreign,
}

pub fn slot(tool: &Tool, dir: &Path) -> Slot {
    if dir.symlink_metadata().is_err() {
        return Slot::Free;
    }
    if !dir.is_dir() {
        return Slot::Foreign;
    }
    if tool.identity == Identity::ClaudeJson {
        if let Some(s) = read_claude(tool, dir.to_path_buf(), &dir.join(".claude.json"), false) {
            return if s.email.is_some() { Slot::Signed } else { Slot::Unsigned };
        }
    }
    match (untouched(tool, dir), &tool.identity) {
        (true, _) => Slot::Free,
        (false, Identity::Linked) => Slot::Signed,
        (false, Identity::ClaudeJson) => Slot::Foreign,
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

/// First `~/.<tool>-<base>`, `~/.<tool>-<base>-2`… where a new session can start.
pub fn free_dir(home: &Path, tool: &Tool, base: &str) -> PathBuf {
    let base = if base.is_empty() { "session" } else { base };
    let p = tool.prefix();
    (1..)
        .map(|n| home.join(if n == 1 { format!("{p}{base}") } else { format!("{p}{base}-{n}") }))
        .find(|d| matches!(slot(tool, d), Slot::Free | Slot::Unsigned))
        .expect("an unused directory name")
}

/// Creates the session directory (private) and links the shared entries of the tool's default directory
/// into it.
pub fn prepare(tool: &Tool, dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    if dir.symlink_metadata().is_err() {
        std::fs::DirBuilder::new().mode(0o700).create(dir).map_err(|e| format!("{}: {e}", tilde(dir)))?;
    }
    for name in &tool.shared {
        let (src, dst) = (tool.dir.join(name), dir.join(name));
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
    let t = &s.tool;
    let mut parts = vec![];
    if let Some(w) = workdir {
        parts.push(format!("cd {}", sh_path(w, home)));
    }
    if let Some(t) = intro {
        parts.push(format!("printf '%s\\n\\n' {}", sh_quote(t)));
    }
    parts.push(if s.default { t.command.clone() } else { format!("{}={} {}", t.env, sh_path(&s.dir, home), t.command) });
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
            format!("le dossier de départ {} n'existe pas : choisis-en un autre dans Sessions › Dossier de départ.", tilde(w))
        } else {
            format!("the start folder {} does not exist: pick another one in Sessions › Start folder.", tilde(w))
        });
    }
    if matches!(opts.terminal, Terminal::ITerm | Terminal::ITermTab) && !iterm_installed() {
        return Err(l(
            "iTerm n'est pas installé : choisis un autre terminal dans le menu Sessions › Ouvrir dans.",
            "iTerm is not installed: pick another terminal in Sessions › Open in.",
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
            let title = if fr() { format!("Ouverture de {} impossible", s.tool.name) } else { format!("Could not open {}", s.tool.name) };
            notify(&title, &e);
        }
    });
}

fn intro(tool: &Tool, dir: &Path, email: Option<&str>) -> String {
    let d = format!("{} {}", tool.name, tilde(dir));
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

fn create_and_open(tool: &Tool, dir: PathBuf, email: Option<&str>, opts: &Opts, on_created: Option<OnCreated>) -> Result<(), String> {
    prepare(tool, &dir)?;
    if let Some(f) = on_created {
        f(dir.clone());
    }
    let text = intro(tool, &dir, email);
    open_now(&Session::new(tool, dir), opts, Some(&text))
}

/// Called with the directory of a session VibeRooms has just created (to link it to its account).
pub type OnCreated = Box<dyn FnOnce(PathBuf) + Send>;

/// "Set up <tool> for this account": a new session named after the account, then its sign-in.
pub fn new_for_account(tool: Tool, label: String, email: String, opts: Opts, on_created: OnCreated) {
    std::thread::spawn(move || {
        let Some(opts) = settle(opts) else { return };
        let dir = free_dir(&home(), &tool, &slug(&label));
        let email = Some(email.as_str()).filter(|e| !e.is_empty());
        if let Err(e) = create_and_open(&tool, dir, email, &opts, Some(on_created)) {
            notify(l("Création de la session impossible", "Could not create the session"), &e);
        }
    });
}

/// "New session…": asks for a name, then opens that session (and its sign-in when it is new).
pub fn new_session(tool: Tool, opts: Opts) {
    std::thread::spawn(move || {
        let Some(name) = ask_name(&tool) else { return };
        let s = slug(&name);
        if s.is_empty() {
            return;
        }
        let Some(opts) = settle(opts) else { return };
        let dir = home().join(format!("{}{s}", tool.prefix()));
        let res = match slot(&tool, &dir) {
            Slot::Free | Slot::Unsigned => create_and_open(&tool, dir, None, &opts, None),
            Slot::Signed => open_now(&Session::new(&tool, dir), &opts, None),
            Slot::Foreign => Err(if fr() {
                format!("{} existe déjà et n'est pas une session {} : choisis un autre nom.", tilde(&dir), tool.name)
            } else {
                format!("{} already exists and is not a {} session: pick another name.", tilde(&dir), tool.name)
            }),
        };
        if let Err(e) = res {
            notify(l("Création de la session impossible", "Could not create the session"), &e);
        }
    });
}

/// Name of a new session, asked in a dialog (None when cancelled).
fn ask_name(tool: &Tool) -> Option<String> {
    let (prompt, ok, cancel) = if fr() {
        (
            format!("Nom de la nouvelle session {} (ex. perso, client-x). Elle sera créée dans ~/{}<nom>.", tool.name, tool.prefix()),
            "Créer",
            "Annuler",
        )
    } else {
        (
            format!("Name of the new {} session (e.g. personal, client-x). It is created in ~/{}<name>.", tool.name, tool.prefix()),
            "Create",
            "Cancel",
        )
    };
    let script = format!(
        "activate\ntext returned of (display dialog {} default answer \"\" with title \"VibeRooms\" buttons {{{}, {}}} default button {} cancel button {})",
        as_string(&prompt),
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
    let prompt = l("Dossier où démarrer la session :", "Folder to start the session in:");
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

    fn claude(h: &TempHome) -> Tool {
        Tool::claude(&h.0)
    }

    #[test]
    fn discovers_default_then_named_sessions() {
        let h = TempHome::new("discover");
        h.write(".claude.json", SIGNED);
        h.write(".claude/settings.json", "{}");
        h.write(".claude-work/.claude.json", r#"{"oauthAccount": {"emailAddress": "work@example.com"}}"#);
        h.write(".claude-fresh/.claude.json", r#"{"numStartups": 1}"#);
        h.write(".claude-code-router/config.json", "{}"); // another tool
        h.write(".claude.json.bak", SIGNED); // a file, not a session
        let tools = [claude(&h)];
        let found = discover(&h.0, &tools, &Links::new(), &[]);
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
        let again = discover(&h.0, &tools, &Links::new(), &found);
        assert_eq!(again.len(), 2);
        assert_eq!(again[1], found[2]);
        assert!(discover(&h.0, &tools, &Links::new(), &[]).iter().all(|s| s.default));
    }

    #[test]
    fn discovers_linked_sessions_of_other_tools() {
        let h = TempHome::new("linked");
        let codex = Tool::codex(&h.0);
        h.write(".codex/config.toml", "model = 'x'");
        h.write(".codex-perso/auth.json", "{}");
        prepare(&codex, &h.0.join(".codex-new")).unwrap(); // prepared, never started
        let mut links = Links::new();
        links.insert(tilde(&h.0.join(".codex-perso")), Link { email: "me@example.com".into(), org: Some("acc-1".into()) });
        let found = discover(&h.0, std::slice::from_ref(&codex), &links, &[]);
        let dirs: Vec<String> = found.iter().map(|s| s.short_dir()).collect();
        assert_eq!(dirs, [tilde(&h.0.join(".codex")), tilde(&h.0.join(".codex-perso"))]);
        assert!(found[0].default && found[0].email.is_none());
        assert_eq!((found[1].email.as_deref(), found[1].org_uuid.as_deref()), (Some("me@example.com"), Some("acc-1")));
        assert_eq!(found[1].tool.id, "codex");
        // Without its default directory, a tool has only its named sessions.
        std::fs::remove_dir_all(h.0.join(".codex")).unwrap();
        assert_eq!(discover(&h.0, &[codex], &links, &[]).len(), 1);
    }

    #[test]
    fn tools_from_config() {
        let home = Path::new("/h");
        let declared = serde_json::json!([
            { "id": "claude", "command": "claude --model opus" },
            { "id": "pi", "name": "pi", "command": "pi", "env": "PI_CODING_AGENT_DIR", "dir": "~/.pi/agent", "provider": "claude",
              "shared": ["settings.json", "AGENTS.md"] },
            { "id": "nope", "command": "x" },
            { "id": "bad env", "command": "x", "env": "X" },
            { "id": "bad", "command": "x", "env": "A B" }
        ]);
        let t = tools(home, &declared);
        let ids: Vec<&str> = t.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["claude", "codex", "pi"]);
        let c = &t[0];
        assert_eq!(
            (c.command.as_str(), c.env.as_str(), &c.identity, c.declared),
            ("claude --model opus", "CLAUDE_CONFIG_DIR", &Identity::ClaudeJson, true)
        );
        assert!(c.shared.contains(&"skills".to_string()));
        let pi = &t[2];
        assert_eq!(pi.dir, home.join(".pi/agent"));
        assert_eq!((pi.provider.as_deref(), &pi.identity, pi.shared.len()), (Some("claude"), &Identity::Linked, 2));
        assert_eq!(tools(home, &Value::Null).len(), 2);
    }

    #[test]
    fn slots_and_free_dirs() {
        let h = TempHome::new("slots");
        let c = claude(&h);
        h.write(".claude/settings.json", "{}");
        assert_eq!(slot(&c, &h.0.join(".claude-new")), Slot::Free);
        h.write(".claude-signed/.claude.json", SIGNED);
        assert_eq!(slot(&c, &h.0.join(".claude-signed")), Slot::Signed);
        h.write(".claude-unsigned/.claude.json", "{}");
        assert_eq!(slot(&c, &h.0.join(".claude-unsigned")), Slot::Unsigned);
        h.write(".claude-other/config.json", "{}");
        assert_eq!(slot(&c, &h.0.join(".claude-other")), Slot::Foreign);
        // Prepared by VibeRooms, never started: still free.
        prepare(&c, &h.0.join(".claude-prepared")).unwrap();
        assert_eq!(slot(&c, &h.0.join(".claude-prepared")), Slot::Free);

        assert_eq!(free_dir(&h.0, &c, "signed"), h.0.join(".claude-signed-2"));
        assert_eq!(free_dir(&h.0, &c, "other"), h.0.join(".claude-other-2"));
        assert_eq!(free_dir(&h.0, &c, "unsigned"), h.0.join(".claude-unsigned"));
        assert_eq!(free_dir(&h.0, &c, ""), h.0.join(".claude-session"));

        // Other tools: any use makes a session.
        let codex = Tool::codex(&h.0);
        h.write(".codex-used/history.jsonl", "");
        assert_eq!(slot(&codex, &h.0.join(".codex-used")), Slot::Signed);
        assert_eq!(free_dir(&h.0, &codex, "used"), h.0.join(".codex-used-2"));
    }

    #[test]
    fn prepare_links_only_existing_shared_entries() {
        let h = TempHome::new("prepare");
        h.write(".claude/settings.json", r#"{"model": "opus"}"#);
        h.write(".claude/skills/a/SKILL.md", "x");
        h.write(".claude/history.jsonl", "private");
        let dir = h.0.join(".claude-perso");
        prepare(&claude(&h), &dir).unwrap();
        prepare(&claude(&h), &dir).unwrap(); // idempotent
        assert_eq!(std::fs::read_link(dir.join("settings.json")).unwrap(), h.0.join(".claude/settings.json"));
        assert!(dir.join("skills/a/SKILL.md").exists());
        assert!(dir.join("agents").symlink_metadata().is_err());
        assert!(dir.join("history.jsonl").symlink_metadata().is_err());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);

        let codex = Tool::codex(&h.0);
        h.write(".codex/config.toml", "");
        h.write(".codex/auth.json", "{}");
        prepare(&codex, &h.0.join(".codex-perso")).unwrap();
        assert!(h.0.join(".codex-perso/config.toml").symlink_metadata().is_ok());
        assert!(h.0.join(".codex-perso/auth.json").symlink_metadata().is_err());
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
        let c = Tool::claude(home);
        let named = Session::new(&c, home.join(".claude-perso"));
        let default = Session { default: true, ..Session::new(&c, home.join(".claude")) };
        assert_eq!(command_line(&default, None, None, home), "claude");
        assert_eq!(command_line(&named, None, None, home), r#"CLAUDE_CONFIG_DIR="$HOME/.claude-perso" claude"#);
        assert_eq!(
            command_line(&named, Some(&home.join("Sites/my app")), Some("It's new"), home),
            r#"cd '/Users/me/Sites/my app' && printf '%s\n\n' 'It'\''s new' && CLAUDE_CONFIG_DIR="$HOME/.claude-perso" claude"#
        );
        assert_eq!(command_line(&default, Some(home), None, home), r#"cd "$HOME" && claude"#);
        let odd = Session::new(&c, home.join(".claude-a b"));
        assert_eq!(command_line(&odd, None, None, home), "CLAUDE_CONFIG_DIR='/Users/me/.claude-a b' claude");
        let codex = Tool::codex(home);
        assert_eq!(
            command_line(&Session::new(&codex, home.join(".codex-perso")), None, None, home),
            r#"CODEX_HOME="$HOME/.codex-perso" codex"#
        );
        assert_eq!(command_line(&Session { default: true, ..Session::new(&codex, home.join(".codex")) }, None, None, home), "codex");
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
