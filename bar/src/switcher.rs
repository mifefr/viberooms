//! Experimental: switching the Claude Code account of this Mac (opt-in, `"switcher": true`).
//!
//! Claude Code keeps its session in the keychain (`Claude Code-credentials`, a JSON with
//! `claudeAiOauth` + `mcpOAuth`) and the displayed identity in `~/.claude.json` (`oauthAccount`).
//! The vault keeps, per account, `claudeAiOauth` + `oauthAccount` in the keychain; a switch only
//! replaces `claudeAiOauth` (the machine's MCP tokens are preserved).
//!
//! Tokens rotate on every refresh: the active session is re-captured whenever it changes, after its
//! identity has been verified through /api/oauth/profile.

use crate::{fr, home, l, notify, now_ms, support_dir};
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

pub const CC_SERVICE: &str = "Claude Code-credentials";
const VAULT_SERVICE: &str = "VibeRooms account";
const GUARD_MS: i64 = 90_000;
const ENROLL_TIMEOUT_MS: i64 = 15 * 60_000;

// ---------- Keychain (through /usr/bin/security, like Claude Code; never a secret in argv) ----------

fn decode_output(s: &str) -> String {
    let t = s.trim_end_matches('\n');
    // `security -w` prints non-printable secrets in hexadecimal.
    if t.len().is_multiple_of(2) && t.len() > 2 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        let bytes: Option<Vec<u8>> = (0..t.len()).step_by(2).map(|i| u8::from_str_radix(&t[i..i + 2], 16).ok()).collect();
        if let Some(Ok(txt)) = bytes.map(String::from_utf8) {
            if txt.starts_with('{') {
                return txt;
            }
        }
    }
    t.to_string()
}

fn sec_read(service: &str, account: Option<&str>) -> Option<String> {
    let mut c = Command::new("/usr/bin/security");
    c.arg("find-generic-password").arg("-s").arg(service);
    if let Some(a) = account {
        c.arg("-a").arg(a);
    }
    let out = c.arg("-w").stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(decode_output(&String::from_utf8_lossy(&out.stdout)))
}

fn sec_write(service: &str, account: &str, label: Option<&str>, secret: &str) -> Result<(), String> {
    if [service, account, label.unwrap_or("")].iter().any(|s| s.contains('"') || s.contains('\n')) {
        return Err("invalid keychain item name".into());
    }
    let label = label.map(|l| format!(" -l \"{l}\"")).unwrap_or_default();
    let hex: String = secret.bytes().map(|b| format!("{b:02x}")).collect();
    let cmd = format!("add-generic-password -U -a \"{account}\" -s \"{service}\"{label} -X \"{hex}\"\n");
    let mut child = Command::new("/usr/bin/security")
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    child.stdin.take().unwrap().write_all(cmd.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("keychain: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    // Read back: the write only counts if the keychain returns exactly the same content.
    match sec_read(service, Some(account)) {
        Some(back) if back == secret => Ok(()),
        _ => Err("keychain: read-back differs after write".into()),
    }
}

fn sec_delete(service: &str, account: Option<&str>) {
    let mut c = Command::new("/usr/bin/security");
    c.arg("delete-generic-password").arg("-s").arg(service);
    if let Some(a) = account {
        c.arg("-a").arg(a);
    }
    let _ = c.stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Keychain services whose name starts with "Claude Code-credentials" (attributes only).
fn list_cc_services() -> HashSet<String> {
    let out = Command::new("/usr/bin/security").arg("dump-keychain").stderr(Stdio::null()).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("\"svce\"<blob>=\""))
        .filter_map(|l| l.strip_suffix('"'))
        .filter(|s| s.starts_with(CC_SERVICE))
        .map(str::to_owned)
        .collect()
}

fn keychain_account_for_cc() -> String {
    // Account ("acct" attribute) of the existing Claude Code item, else the current user.
    let out = Command::new("/usr/bin/security").args(["find-generic-password", "-s", CC_SERVICE]).stderr(Stdio::null()).output();
    if let Ok(o) = out {
        for l in String::from_utf8_lossy(&o.stdout).lines() {
            if let Some(v) = l.trim().strip_prefix("\"acct\"<blob>=\"").and_then(|v| v.strip_suffix('"')) {
                return v.to_string();
            }
        }
    }
    std::env::var("USER").unwrap_or_default()
}

fn hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

// ---------- Identity ----------

#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    pub email: String,
    pub org_uuid: Option<String>,
    pub org_name: Option<String>,
}

impl Identity {
    pub fn key(&self) -> String {
        format!("{}|{}", self.email.to_lowercase(), self.org_uuid.as_deref().unwrap_or("-"))
    }
    fn matches_oauth_account(&self, oa: &Value) -> bool {
        let email = oa.get("emailAddress").and_then(Value::as_str).unwrap_or("").to_lowercase();
        let org = oa.get("organizationUuid").and_then(Value::as_str);
        email == self.email.to_lowercase() && (self.org_uuid.is_none() || org == self.org_uuid.as_deref())
    }
}

enum ProfileError {
    Auth,
    Other,
}

/// Checks who an access token belongs to (read-only: no refresh, hence no rotation).
fn profile(access_token: &str) -> Result<(Identity, Value), ProfileError> {
    let res = ureq::get("https://api.anthropic.com/api/oauth/profile")
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .call();
    let v: Value = match res {
        Ok(r) => r.into_json().map_err(|_| ProfileError::Other)?,
        Err(ureq::Error::Status(401 | 403, _)) => return Err(ProfileError::Auth),
        Err(_) => return Err(ProfileError::Other),
    };
    let s = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_owned);
    let email = s("/account/email").ok_or(ProfileError::Other)?;
    Ok((Identity { email, org_uuid: s("/organization/uuid"), org_name: s("/organization/name") }, v))
}

/// `oauthAccount` rebuilt from the profile (when no faithful copy could be captured).
fn oauth_account_from_profile(p: &Value) -> Value {
    let g = |ptr: &str| p.pointer(ptr).cloned().unwrap_or(Value::Null);
    json!({
        "accountUuid": g("/account/uuid"),
        "emailAddress": g("/account/email"),
        "organizationUuid": g("/organization/uuid"),
        "displayName": g("/account/display_name"),
        "fullName": g("/account/full_name"),
        "accountCreatedAt": g("/account/created_at"),
        "organizationName": g("/organization/name"),
        "organizationType": g("/organization/organization_type"),
        "organizationRateLimitTier": g("/organization/rate_limit_tier"),
        "billingType": g("/organization/billing_type"),
        "hasExtraUsageEnabled": g("/organization/has_extra_usage_enabled"),
        "seatTier": g("/organization/seat_tier"),
        "subscriptionCreatedAt": g("/organization/subscription_created_at"),
    })
}

// ---------- ~/.claude.json ----------

fn claude_json() -> PathBuf {
    home().join(".claude.json")
}

fn read_json(path: &PathBuf) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub fn current_oauth_account() -> Option<Value> {
    read_json(&claude_json())?.get("oauthAccount").cloned()
}

fn write_oauth_account(oa: &Value) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = claude_json();
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("~/.claude.json is not valid: {e}"))?;
    let backup = home().join(".claude.json.claude-quotas.bak");
    if !backup.exists() {
        // One-time backup, private like the original (0600).
        if let Ok(mut b) = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&backup) {
            let _ = b.write_all(text.as_bytes());
        }
    }
    v["oauthAccount"] = oa.clone();
    let mut out = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    if text.ends_with('\n') {
        out.push('\n');
    }
    let tmp = path.with_file_name(".claude.json.claude-quotas.tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(out.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

// ---------- Coffre ----------

#[derive(Clone, Debug)]
pub struct Entry {
    pub key: String,
    pub email: String,
    pub org_uuid: Option<String>,
    pub org_name: Option<String>,
    pub refresh_expires_at: Option<i64>,
    pub saved_at: i64,
}

impl Entry {
    pub fn expired(&self) -> bool {
        self.refresh_expires_at.is_some_and(|t| t < now_ms())
    }
}

fn index_path() -> PathBuf {
    support_dir().join("vault.json")
}

fn load_index() -> Vec<Entry> {
    let Some(v) = read_json(&index_path()) else { return vec![] };
    v.get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let s = |k: &str| e.get(k).and_then(Value::as_str).map(str::to_owned);
            Some(Entry {
                key: s("key")?,
                email: s("email")?,
                org_uuid: s("orgUuid"),
                org_name: s("orgName"),
                refresh_expires_at: e.get("refreshExpiresAt").and_then(Value::as_i64),
                saved_at: e.get("savedAt").and_then(Value::as_i64).unwrap_or(0),
            })
        })
        .collect()
}

fn save_index(entries: &[Entry]) {
    let v = json!({
        "note": "VibeRooms vault index (no secret: tokens live in the keychain, service \"VibeRooms account\").",
        "entries": entries.iter().map(|e| json!({
            "key": e.key, "email": e.email, "orgUuid": e.org_uuid, "orgName": e.org_name,
            "refreshExpiresAt": e.refresh_expires_at, "savedAt": e.saved_at,
        })).collect::<Vec<_>>(),
    });
    let _ = std::fs::create_dir_all(support_dir());
    let _ = std::fs::write(index_path(), serde_json::to_vec_pretty(&v).unwrap());
}

// ---------- Moteur ----------

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub entries: Vec<Entry>,
    pub active: Option<String>,
    pub enrolling: bool,
    /// Running `claude` sessions: they keep the previous account after a switch.
    pub open_sessions: usize,
}

/// Number of Claude Code (CLI) processes running on this Mac.
pub fn open_sessions() -> usize {
    Command::new("/usr/bin/pgrep")
        .args(["-x", "claude"])
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

pub enum SwCmd {
    Switch(String),
    Enroll,
    Forget(String),
}

struct Enroll {
    started: i64,
    before: HashSet<String>,
}

pub struct Switcher {
    cc_account: String,
    entries: Vec<Entry>,
    last_hash: Option<u64>,
    active: Option<String>,
    guard: Option<(String, i64)>,
    enroll: Option<Enroll>,
}

fn enroll_dir() -> PathBuf {
    support_dir().join("enroll")
}

impl Switcher {
    pub fn new() -> Self {
        Self { cc_account: keychain_account_for_cc(), entries: load_index(), last_hash: None, active: None, guard: None, enroll: None }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            entries: self.entries.clone(),
            active: self.active.clone(),
            enrolling: self.enroll.is_some(),
            open_sessions: open_sessions(),
        }
    }

    pub fn enrolling(&self) -> bool {
        self.enroll.is_some()
    }

    fn store(&mut self, id: &Identity, oauth: &Value, oauth_account: Value) -> Result<(), String> {
        let key = id.key();
        // Store the claudeAiOauth + oauthAccount pair for this identity.
        let secret = json!({ "v": 1, "claudeAiOauth": oauth, "oauthAccount": oauth_account, "savedAt": now_ms() });
        sec_write(VAULT_SERVICE, &key, Some(&format!("VibeRooms · {}", id.email)), &secret.to_string())?;
        let entry = Entry {
            key: key.clone(),
            email: id.email.clone(),
            org_uuid: id.org_uuid.clone(),
            org_name: id.org_name.clone(),
            refresh_expires_at: oauth.get("refreshTokenExpiresAt").and_then(Value::as_i64),
            saved_at: now_ms(),
        };
        match self.entries.iter_mut().find(|e| e.key == key) {
            Some(e) => *e = entry,
            None => self.entries.push(entry),
        }
        save_index(&self.entries);
        Ok(())
    }

    fn vault(&self, key: &str) -> Option<Value> {
        serde_json::from_str(&sec_read(VAULT_SERVICE, Some(key))?).ok()
    }

    /// Captures the active session when it changed (rotation, /login…), after verifying its identity.
    fn capture_active(&mut self) {
        let Some(raw) = sec_read(CC_SERVICE, Some(&self.cc_account)) else {
            self.active = None;
            self.last_hash = None;
            return;
        };
        let h = hash(&raw);
        if self.last_hash == Some(h) {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(&raw) else { return };
        let Some(oauth) = v.get("claudeAiOauth").filter(|o| o.is_object()) else {
            self.active = None;
            self.last_hash = Some(h);
            return;
        };
        let expires = oauth.get("expiresAt").and_then(Value::as_i64).unwrap_or(0);
        let Some(token) = oauth.get("accessToken").and_then(Value::as_str) else { return };
        if expires < now_ms() + 30_000 {
            return; // expired token: Claude Code will refresh it, we capture afterwards
        }
        match profile(token) {
            Ok((id, prof)) => {
                let oa = match current_oauth_account() {
                    Some(oa) if id.matches_oauth_account(&oa) => oa,
                    _ => self
                        .vault(&id.key())
                        .and_then(|v| v.get("oauthAccount").cloned())
                        .filter(|oa| id.matches_oauth_account(oa))
                        .unwrap_or_else(|| oauth_account_from_profile(&prof)),
                };
                if self.store(&id, oauth, oa).is_ok() {
                    self.active = Some(id.key());
                    self.last_hash = Some(h);
                }
            }
            Err(ProfileError::Auth) => {
                self.active = None;
                self.last_hash = Some(h);
            }
            Err(ProfileError::Other) => {} // network: retry next pass
        }
    }

    /// After a switch, open sessions may write the previous identity back into ~/.claude.json.
    fn enforce_guard(&mut self) {
        let Some((key, until)) = self.guard.clone() else { return };
        if now_ms() > until || self.active.as_deref() != Some(key.as_str()) {
            self.guard = None;
            return;
        }
        let Some(target) = self.vault(&key).and_then(|v| v.get("oauthAccount").cloned()) else { return };
        let same = current_oauth_account().is_some_and(|cur| {
            cur.get("emailAddress") == target.get("emailAddress") && cur.get("organizationUuid") == target.get("organizationUuid")
        });
        if !same {
            let _ = write_oauth_account(&target);
        }
    }

    fn check_enroll(&mut self) {
        let Some(en) = &self.enroll else { return };
        if now_ms() - en.started > ENROLL_TIMEOUT_MS {
            self.finish_enroll(None);
            return;
        }
        let dir = enroll_dir();
        let Some(oa) = read_json(&dir.join(".claude.json")).and_then(|v| v.get("oauthAccount").cloned()) else { return };
        // Tokens: the new keychain item created by the sign-in, or the file fallback.
        let new_services: Vec<String> = list_cc_services().difference(&en.before).cloned().collect();
        let mut found: Option<(Value, Option<String>)> = None;
        for svc in &new_services {
            if let Some(o) =
                sec_read(svc, None).and_then(|r| serde_json::from_str::<Value>(&r).ok()).and_then(|v| v.get("claudeAiOauth").cloned())
            {
                found = Some((o, Some(svc.clone())));
                break;
            }
        }
        if found.is_none() {
            if let Some(o) = read_json(&dir.join(".credentials.json")).and_then(|v| v.get("claudeAiOauth").cloned()) {
                found = Some((o, None));
            }
        }
        let Some((oauth, svc)) = found else { return };
        let Some(token) = oauth.get("accessToken").and_then(Value::as_str) else { return };
        match profile(token) {
            Ok((id, prof)) => {
                let oa = if id.matches_oauth_account(&oa) { oa } else { oauth_account_from_profile(&prof) };
                match self.store(&id, &oauth, oa) {
                    Ok(()) => {
                        if fr() {
                            notify("Compte ajouté", &format!("{} est disponible dans la bascule Claude Code.", id.email));
                        } else {
                            notify("Account added", &format!("{} is available for Claude Code switching.", id.email));
                        }
                        self.finish_enroll(svc);
                    }
                    Err(e) => notify(l("Ajout du compte impossible", "Could not add the account"), &e),
                }
            }
            Err(ProfileError::Auth) => {
                notify(
                    l("Ajout du compte impossible", "Could not add the account"),
                    l("Jeton refusé par Anthropic.", "Token refused by Anthropic."),
                );
                self.finish_enroll(svc);
            }
            Err(ProfileError::Other) => {}
        }
    }

    fn finish_enroll(&mut self, svc: Option<String>) {
        if let Some(s) = svc {
            sec_delete(&s, None);
        }
        let _ = std::fs::remove_dir_all(enroll_dir());
        self.enroll = None;
    }

    pub fn tick(&mut self) {
        self.capture_active();
        self.enforce_guard();
        self.check_enroll();
    }

    pub fn start_enroll(&mut self) {
        let dir = enroll_dir();
        let _ = std::fs::remove_dir_all(&dir);
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        self.enroll = Some(Enroll { started: now_ms(), before: list_cc_services() });
        let d = dir.display().to_string().replace('\'', "");
        let (intro, done) = if fr() {
            (
                "VibeRooms : connecte-toi avec le compte a ajouter. Ta session Claude Code actuelle ne change pas.",
                "OK : compte enregistre, il apparait dans le menu VibeRooms. Tu peux fermer cette fenetre.",
            )
        } else {
            (
                "VibeRooms: sign in with the account to add. Your current Claude Code session does not change.",
                "OK: account saved, it shows up in the VibeRooms menu. You can close this window.",
            )
        };
        let shell = format!("clear; echo {intro}; echo; CLAUDE_CONFIG_DIR='{d}' claude auth login && echo && echo {done}");
        let script = format!(
            "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
            shell.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let _ = Command::new("/usr/bin/osascript").arg("-e").arg(script).spawn();
    }

    pub fn switch(&mut self, key: &str) -> Result<String, String> {
        self.capture_active(); // fresh capture of the account being left
        if self.active.as_deref() == Some(key) {
            return Err(l("ce compte est déjà actif", "this account is already active").into());
        }
        let entry = self.entries.iter().find(|e| e.key == key).cloned().ok_or(l("compte inconnu du coffre", "account not in the vault"))?;
        if entry.expired() {
            return Err(if fr() {
                format!("la session de {} a expiré : ajoute à nouveau ce compte", entry.email)
            } else {
                format!("the session of {} expired: add this account again", entry.email)
            });
        }
        let vault = self.vault(key).ok_or(l("entrée du trousseau introuvable", "keychain item not found"))?;
        let oauth = vault.get("claudeAiOauth").cloned().ok_or(l("entrée du trousseau incomplète", "incomplete keychain item"))?;
        let mut current: Value =
            sec_read(CC_SERVICE, Some(&self.cc_account)).and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_else(|| json!({}));
        current["claudeAiOauth"] = oauth; // mcpOAuth and the rest are preserved
        let raw = current.to_string();
        sec_write(CC_SERVICE, &self.cc_account, None, &raw)?;
        if let Some(oa) = vault.get("oauthAccount").filter(|v| v.is_object()) {
            write_oauth_account(oa)?;
        }
        self.last_hash = Some(hash(&raw));
        self.active = Some(key.to_string());
        self.guard = Some((key.to_string(), now_ms() + GUARD_MS));
        Ok(entry.email)
    }

    pub fn forget(&mut self, key: &str) {
        if self.active.as_deref() == Some(key) {
            return; // never forget the account in use
        }
        sec_delete(VAULT_SERVICE, Some(key));
        self.entries.retain(|e| e.key != key);
        save_index(&self.entries);
    }
}
