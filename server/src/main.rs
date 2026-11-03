//! VibeRooms — self-hosted monitor of AI subscription limits.

mod derive;
mod homepage;
mod http;
mod i18n;
mod poller;
mod providers;
mod store;
mod web;

use poller::Runtime;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tracing::info;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

pub struct Flow {
    pub provider: providers::Provider,
    pub pkce: providers::Pkce,
    pub label: Option<String>,
    pub account_id: Option<String>,
    pub created_at: i64,
}

pub struct Config {
    pub public_url: String,
    pub lang: i18n::Lang,
    pub homepage: Option<homepage::Config>,
    pub hook: Option<String>,
}

pub struct App {
    pub store: store::Store,
    pub http: http::Http,
    pub cfg: Config,
    runtime: Mutex<HashMap<String, Runtime>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    wakers: Mutex<HashMap<String, Arc<Notify>>>,
    tasks: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    pub flows: Mutex<HashMap<String, Flow>>,
}

impl App {
    pub fn with_rt<T>(&self, id: &str, f: impl FnOnce(&mut Runtime) -> T) -> T {
        let mut map = self.runtime.lock().unwrap();
        f(map.entry(id.to_string()).or_default())
    }
    pub fn rt(&self, id: &str) -> Runtime {
        self.runtime.lock().unwrap().get(id).cloned().unwrap_or_default()
    }
    pub fn lock_for(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks.lock().unwrap().entry(id.to_string()).or_default().clone()
    }

    /// Starts (or immediately re-runs) collection for an account.
    pub fn track(self: &Arc<Self>, id: &str, first: Duration) {
        if let Some(w) = self.wakers.lock().unwrap().get(id) {
            self.with_rt(id, |r| {
                r.status = poller::Status::Pending;
                r.failures = 0;
                r.profile_at = None;
            });
            w.notify_one();
            return;
        }
        let waker = Arc::new(Notify::new());
        self.wakers.lock().unwrap().insert(id.to_string(), waker.clone());
        let handle = tokio::spawn(poller::account_loop(self.clone(), id.to_string(), waker, first));
        self.tasks.lock().unwrap().insert(id.to_string(), handle);
    }

    pub fn untrack(&self, id: &str) {
        if let Some(h) = self.tasks.lock().unwrap().remove(id) {
            h.abort();
        }
        self.wakers.lock().unwrap().remove(id);
        self.runtime.lock().unwrap().remove(id);
        self.locks.lock().unwrap().remove(id);
    }

    /// Called whenever the list of accounts changes (add, rename, reorder, remove).
    pub fn accounts_changed(self: &Arc<Self>) {
        let app = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            if let Some(hp) = &app.cfg.homepage {
                match homepage::sync(&app, hp).await {
                    Ok(changed) => info!("homepage: {}", if changed { "updated" } else { "unchanged" }),
                    Err(e) => tracing::warn!("homepage sync failed: {e:#}"),
                }
            }
            if let Some(cmd) = app.cfg.hook.clone() {
                let out = tokio::task::spawn_blocking(move || std::process::Command::new("/bin/sh").arg("-c").arg(&cmd).output()).await;
                match out {
                    Ok(Ok(o)) => info!("hook: {}", String::from_utf8_lossy(if o.status.success() { &o.stdout } else { &o.stderr }).trim()),
                    _ => tracing::warn!("hook could not run"),
                }
            }
        });
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("viberooms {VERSION}");
        return Ok(());
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("VR_LOG").unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();

    let dir = env("VR_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").expect("HOME")).join(".config/viberooms"));
    let port: u16 = env("VR_PORT").and_then(|s| s.parse().ok()).unwrap_or(8790);
    // Comma-separated list, e.g. "127.0.0.1,100.64.0.10" to listen on localhost + a Tailscale address only.
    let hosts = env("VR_HOST").unwrap_or_else(|| "127.0.0.1".into());
    let first_host = hosts.split(',').map(str::trim).find(|h| *h != "127.0.0.1" && *h != "0.0.0.0").unwrap_or("127.0.0.1").to_string();
    let cfg = Config {
        public_url: env("VR_PUBLIC_URL").unwrap_or_else(|| format!("http://{first_host}:{port}")),
        lang: i18n::Lang::from_env(),
        homepage: homepage::Config::from_env(),
        hook: env("VR_HOOK"),
    };

    let app = Arc::new(App {
        store: store::Store::open(dir.clone())?,
        http: http::Http::new(),
        cfg,
        runtime: Default::default(),
        locks: Default::default(),
        wakers: Default::default(),
        tasks: Default::default(),
        flows: Default::default(),
    });

    // Resume: reload the last known measure, then stagger the first collections.
    for (i, acc) in app.store.list().iter().enumerate() {
        if let Some((ts, _)) = app.store.latest(&acc.id) {
            app.with_rt(&acc.id, |r| {
                r.status = poller::Status::Ok;
                r.fetched_at = Some(ts);
            });
        }
        app.track(&acc.id, Duration::from_millis(1_500 + i as u64 * 4_000));
    }
    {
        let app = app.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(6 * 3600));
            loop {
                tick.tick().await;
                app.store.prune();
            }
        });
    }

    let router = web::router(app.clone());
    let mut servers = tokio::task::JoinSet::new();
    for h in hosts.split(',').map(str::trim).filter(|h| !h.is_empty()) {
        let listener = tokio::net::TcpListener::bind((h, port)).await.map_err(|e| anyhow::anyhow!("cannot listen on {h}:{port}: {e}"))?;
        info!("VibeRooms {VERSION} listening on http://{h}:{port}");
        let r = router.clone();
        servers.spawn(async move { axum::serve(listener, r).await });
    }
    info!("data: {} — public URL: {}", dir.display(), app.cfg.public_url);
    app.accounts_changed();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
        Some(res) = servers.join_next() => { res??; }
    }
    Ok(())
}
