//! Persistence: accounts + tokens (JSON, mode 0600, atomic writes) and history (SQLite).

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Mutex;

const RETENTION_MS: i64 = 45 * 86_400_000;
/// A sample is written when the value changes, otherwise at most every 10 min (keeps plateaus).
const HEARTBEAT_MS: i64 = 10 * 60_000;
/// Categorical palette slots (resolved to light/dark colours by the page).
const PALETTE: [&str; 5] = ["acc-1", "acc-2", "acc-3", "acc-4", "acc-5"];

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    pub scope: Option<String>,
    /// Provider-specific extras (e.g. an account id sent as a header).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

fn default_provider() -> String {
    "claude".into()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    #[serde(default = "default_provider")]
    pub provider: String,
    pub label: String,
    pub color: String,
    #[serde(default)]
    pub order: i64,
    pub email: Option<String>,
    pub org_name: Option<String>,
    pub account_uuid: Option<String>,
    pub org_uuid: Option<String>,
    pub profile: Option<Value>,
    pub tokens: Tokens,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize, Default)]
struct AccountsFile {
    version: u32,
    accounts: Vec<Account>,
}

pub struct Store {
    pub dir: PathBuf,
    accounts: Mutex<Vec<Account>>,
    db: Mutex<Connection>,
}

pub struct NewAccount {
    pub provider: String,
    pub label: Option<String>,
    pub email: Option<String>,
    pub org_name: Option<String>,
    pub account_uuid: Option<String>,
    pub org_uuid: Option<String>,
    pub profile: Option<Value>,
    pub tokens: Tokens,
}

fn random_id() -> String {
    use rand::Rng;
    format!("{:08x}", rand::thread_rng().gen::<u32>())
}

impl Store {
    pub fn open(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        let file = dir.join("accounts.json");
        let accounts = match fs::read_to_string(&file) {
            Ok(s) => serde_json::from_str::<AccountsFile>(&s).context("accounts.json is not valid")?.accounts,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e.into()),
        };
        let db_path = dir.join("history.sqlite");
        let db = Connection::open(&db_path)?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS samples (
               account_id TEXT NOT NULL, key TEXT NOT NULL, ts INTEGER NOT NULL,
               utilization REAL, resets_at INTEGER);
             CREATE INDEX IF NOT EXISTS samples_idx ON samples(account_id, key, ts);
             CREATE TABLE IF NOT EXISTS latest (
               account_id TEXT PRIMARY KEY, ts INTEGER NOT NULL, usage_json TEXT NOT NULL);",
        )?;
        // `usage_json` holds the normalised snapshot; `raw_json` the provider's raw answer (diagnostics).
        let has_raw: bool = db.prepare("SELECT 1 FROM pragma_table_info('latest') WHERE name = 'raw_json'")?.exists([])?;
        if !has_raw {
            db.execute("ALTER TABLE latest ADD COLUMN raw_json TEXT", [])?;
        }
        // Idempotent migration: reset times rounded to the minute (see providers::round_reset).
        db.execute(
            "UPDATE samples SET resets_at = ((resets_at + 30000) / 60000) * 60000 WHERE resets_at IS NOT NULL AND resets_at % 60000 != 0",
            [],
        )?;
        let _ = fs::set_permissions(&db_path, fs::Permissions::from_mode(0o600));
        Ok(Self { dir, accounts: Mutex::new(accounts), db: Mutex::new(db) })
    }

    fn persist(&self, accounts: &[Account]) -> Result<()> {
        let file = self.dir.join("accounts.json");
        let tmp = self.dir.join(format!("accounts.json.{}.tmp", std::process::id()));
        let body = serde_json::to_vec_pretty(&AccountsFile { version: 1, accounts: accounts.to_vec() })?;
        {
            use std::io::Write;
            let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            f.write_all(&body)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &file)?;
        Ok(())
    }

    pub fn list(&self) -> Vec<Account> {
        let mut v = self.accounts.lock().unwrap().clone();
        v.sort_by_key(|a| a.order);
        v
    }

    pub fn get(&self, id: &str) -> Option<Account> {
        self.accounts.lock().unwrap().iter().find(|a| a.id == id).cloned()
    }

    pub fn add(&self, n: NewAccount) -> Result<Account> {
        let mut accs = self.accounts.lock().unwrap();
        let idx = accs.len();
        let acc = Account {
            id: random_id(),
            provider: n.provider,
            label: n
                .label
                .filter(|s| !s.trim().is_empty())
                .or_else(|| n.email.as_ref().map(|e| e.split('@').next().unwrap_or(e).to_string()))
                .unwrap_or_else(|| format!("Compte {}", idx + 1)),
            color: PALETTE[idx % PALETTE.len()].to_string(),
            order: accs.iter().map(|a| a.order + 1).max().unwrap_or(0),
            email: n.email,
            org_name: n.org_name,
            account_uuid: n.account_uuid,
            org_uuid: n.org_uuid,
            profile: n.profile,
            tokens: n.tokens,
            created_at: crate::now_ms(),
        };
        accs.push(acc.clone());
        self.persist(&accs)?;
        Ok(acc)
    }

    /// Applies a change and persists it immediately (essential after a refresh-token rotation).
    pub fn update<F: FnOnce(&mut Account)>(&self, id: &str, f: F) -> Result<Option<Account>> {
        let mut accs = self.accounts.lock().unwrap();
        let Some(acc) = accs.iter_mut().find(|a| a.id == id) else { return Ok(None) };
        f(acc);
        let out = acc.clone();
        self.persist(&accs)?;
        Ok(Some(out))
    }

    pub fn remove(&self, id: &str) -> Result<bool> {
        let mut accs = self.accounts.lock().unwrap();
        let before = accs.len();
        accs.retain(|a| a.id != id);
        if accs.len() == before {
            return Ok(false);
        }
        self.persist(&accs)?;
        let db = self.db.lock().unwrap();
        db.execute("DELETE FROM samples WHERE account_id = ?1", params![id])?;
        db.execute("DELETE FROM latest WHERE account_id = ?1", params![id])?;
        Ok(true)
    }

    // ---------- Historique ----------

    pub fn record_usage(
        &self,
        account_id: &str,
        ts: i64,
        snapshot: &Value,
        raw: &Value,
        points: &[(String, f64, Option<i64>)],
    ) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO latest (account_id, ts, usage_json, raw_json) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id) DO UPDATE SET ts = excluded.ts, usage_json = excluded.usage_json, raw_json = excluded.raw_json",
            params![account_id, ts, snapshot.to_string(), raw.to_string()],
        )?;
        for (key, util, resets_at) in points {
            let last: Option<(i64, Option<f64>, Option<i64>)> = tx
                .query_row(
                    "SELECT ts, utilization, resets_at FROM samples WHERE account_id = ?1 AND key = ?2 ORDER BY ts DESC LIMIT 1",
                    params![account_id, key],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let write = match last {
                None => true,
                Some((lts, lu, lr)) => lu != Some(*util) || lr != *resets_at || ts - lts >= HEARTBEAT_MS,
            };
            if write {
                tx.execute(
                    "INSERT INTO samples (account_id, key, ts, utilization, resets_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![account_id, key, ts, util, resets_at],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn latest(&self, account_id: &str) -> Option<(i64, Value)> {
        let db = self.db.lock().unwrap();
        db.query_row("SELECT ts, usage_json FROM latest WHERE account_id = ?1", params![account_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .optional()
        .ok()
        .flatten()
        .and_then(|(ts, s)| serde_json::from_str(&s).ok().map(|v| (ts, v)))
    }

    /// Series of (ts, utilisation, resets_at) since `since`.
    pub fn series(&self, account_id: &str, key: &str, since: i64) -> Vec<(i64, f64, Option<i64>)> {
        let db = self.db.lock().unwrap();
        let mut st = match db
            .prepare_cached("SELECT ts, utilization, resets_at FROM samples WHERE account_id = ?1 AND key = ?2 AND ts >= ?3 ORDER BY ts")
        {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        st.query_map(params![account_id, key, since], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<f64>>(1)?.unwrap_or(0.0), r.get::<_, Option<i64>>(2)?))
        })
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }

    pub fn prune(&self) {
        let db = self.db.lock().unwrap();
        let _ = db.execute("DELETE FROM samples WHERE ts < ?1", params![crate::now_ms() - RETENTION_MS]);
    }
}
