//! Optional integration with Homepage (https://gethomepage.dev): writes a group of `customapi` widgets
//! into `services.yaml` (between markers, the rest of the file is never touched) and registers the
//! group in `settings.yaml`. Enabled by `VR_HOMEPAGE_DIR`.

use crate::i18n::{t, Lang};
use crate::App;
use anyhow::{Context, Result};
use std::path::PathBuf;

const BEGIN: &str = "# >>> viberooms (generated block, do not edit)";
const END: &str = "# <<< viberooms";

pub struct Config {
    pub dir: PathBuf,
    /// Homepage base URL, used to trigger regeneration of its pre-rendered page.
    pub url: String,
    pub group: String,
    /// Existing group after which ours is inserted in the layout.
    pub after: Option<String>,
    pub columns: u32,
}

impl Config {
    pub fn from_env() -> Option<Self> {
        let dir = std::env::var("VR_HOMEPAGE_DIR").ok().filter(|s| !s.is_empty())?;
        Some(Self {
            dir: PathBuf::from(dir),
            url: std::env::var("VR_HOMEPAGE_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".into()),
            group: std::env::var("VR_HOMEPAGE_GROUP").unwrap_or_else(|_| "AI".into()),
            after: std::env::var("VR_HOMEPAGE_AFTER").ok().filter(|s| !s.is_empty()),
            columns: std::env::var("VR_HOMEPAGE_COLUMNS").ok().and_then(|s| s.parse().ok()).unwrap_or(4),
        })
    }
}

/// YAML double-quoted scalar (JSON strings are valid YAML).
fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

pub struct Entry {
    pub id: String,
    pub label: String,
    pub description: String,
}

pub fn build_block(group: &str, public_url: &str, lang: Lang, entries: &[Entry]) -> String {
    let mut l: Vec<String> = vec![BEGIN.into(), format!("- {}:", q(group))];
    let widget = |l: &mut Vec<String>, url: String, fields: &[(&str, &str)]| {
        l.push("        widget:".into());
        l.push("          type: customapi".into());
        l.push(format!("          url: {}", q(&url)));
        l.push("          refreshInterval: 60000".into());
        l.push("          mappings:".into());
        for (field, label) in fields {
            l.push(format!("            - field: {field}"));
            l.push(format!("              label: {}", q(label)));
        }
    };
    l.push(format!("    - {}:", q(t(lang, "use_now"))));
    l.push(format!("        href: {}", q(public_url)));
    l.push(format!("        description: {}", q(t(lang, "use_now_desc"))));
    l.push("        icon: mdi-star-circle".into());
    l.push(format!("        siteMonitor: {}", q(&format!("{public_url}/api/health"))));
    widget(
        &mut l,
        format!("{public_url}/api/homepage"),
        &[("recommande", t(lang, "account")), ("dispo", t(lang, "available")), ("alertes", t(lang, "alerts"))],
    );
    for e in entries {
        l.push(format!("    - {}:", q(&e.label)));
        l.push(format!("        href: {}", q(public_url)));
        l.push(format!("        description: {}", q(&e.description)));
        l.push("        icon: mdi-account-circle".into());
        widget(
            &mut l,
            format!("{public_url}/api/homepage/{}", e.id),
            &[("session", t(lang, "session")), ("reset", t(lang, "reset")), ("semaine", t(lang, "week")), ("statut", t(lang, "status"))],
        );
    }
    l.push(END.into());
    l.join("\n") + "\n"
}

/// Replaces (or appends) the generated block. Returns the new file content.
pub fn splice(text: &str, block: &str) -> String {
    if let (Some(b), Some(e)) = (text.find(BEGIN), text.find(END)) {
        if b < e {
            let tail = &text[e + END.len()..];
            let tail = tail.strip_prefix('\n').unwrap_or(tail);
            return format!("{}{}{}", &text[..b], block, tail);
        }
    }
    format!("{}\n\n{}", text.trim_end_matches('\n'), block)
}

/// Adds the group to `layout:` (full width) if missing. Returns `None` when nothing changes.
pub fn ensure_layout(text: &str, group: &str, after: Option<&str>, columns: u32) -> Option<String> {
    if text.lines().any(|l| l.trim_end() == format!("  {group}:")) {
        return None;
    }
    let entry = format!("  {group}:\n    style: row\n    columns: {columns}\n");
    if let Some(a) = after {
        let anchor = format!("  {a}:\n");
        if let Some(pos) = text.find(&anchor) {
            // Insert after the anchor and its indented children.
            let rest = &text[pos + anchor.len()..];
            let child_len: usize = rest.split_inclusive('\n').take_while(|l| l.starts_with("    ")).map(str::len).sum();
            let at = pos + anchor.len() + child_len;
            return Some(format!("{}{}{}", &text[..at], entry, &text[at..]));
        }
    }
    if let Some(pos) = text.find("layout:\n") {
        let at = pos + "layout:\n".len();
        let rest = &text[at..];
        let len: usize = rest.split_inclusive('\n').take_while(|l| l.starts_with("  ")).map(str::len).sum();
        return Some(format!("{}{}{}", &text[..at + len], entry, &text[at + len..]));
    }
    Some(format!("{}\n\nlayout:\n{}", text.trim_end_matches('\n'), entry))
}

fn write_atomic(path: &PathBuf, text: &str) -> Result<()> {
    let tmp = path.with_extension("yaml.viberooms-tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn backup_once(path: &PathBuf) {
    let date = chrono::Local::now().format("%Y%m%d");
    let dst = path.with_file_name(format!("{}.bak-{date}-viberooms", path.file_name().unwrap().to_string_lossy()));
    if !dst.exists() {
        let _ = std::fs::copy(path, dst);
    }
}

pub async fn sync(app: &App, cfg: &Config) -> Result<bool> {
    let entries: Vec<Entry> = app
        .store
        .list()
        .iter()
        .map(|a| {
            let provider = crate::providers::Provider::from_id(&a.provider);
            let plan = provider.map(|p| p.plan_label(a.profile.as_ref())).unwrap_or_default();
            let name = provider.map(|p| p.info().name).unwrap_or("");
            let desc = [Some(format!("{name} {plan}").trim().to_string()), a.email.clone()]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            Entry { id: a.id.clone(), label: a.label.clone(), description: desc }
        })
        .collect();
    let block = build_block(&cfg.group, &app.cfg.public_url, app.cfg.lang, &entries);

    let services = cfg.dir.join("services.yaml");
    let text = std::fs::read_to_string(&services).with_context(|| format!("reading {}", services.display()))?;
    let new = splice(&text, &block);
    let mut changed = false;
    if new != text {
        if !text.contains(BEGIN) {
            backup_once(&services);
        }
        write_atomic(&services, &new)?;
        changed = true;
    }
    let settings = cfg.dir.join("settings.yaml");
    if let Ok(st) = std::fs::read_to_string(&settings) {
        if let Some(new) = ensure_layout(&st, &cfg.group, cfg.after.as_deref(), cfg.columns) {
            backup_once(&settings);
            write_atomic(&settings, &new)?;
            changed = true;
        }
    }
    if changed {
        // Homepage serves a pre-rendered page: ask it to regenerate so the new config shows up.
        let _ = app.http.inner.get(format!("{}/api/revalidate", cfg.url.trim_end_matches('/'))).send().await;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splice_appends_then_replaces() {
        let base = "- \"Infra\":\n    - \"Kuma\":\n        href: x\n";
        let b1 = build_block("AI", "http://h:8790", Lang::En, &[]);
        let once = splice(base, &b1);
        assert!(once.starts_with(base));
        assert!(once.contains(BEGIN) && once.ends_with(&format!("{END}\n")));
        let b2 = build_block(
            "AI",
            "http://h:8790",
            Lang::En,
            &[Entry { id: "abc".into(), label: "Perso".into(), description: "Claude Max".into() }],
        );
        let twice = splice(&once, &b2);
        assert_eq!(twice.matches(BEGIN).count(), 1);
        assert!(twice.contains("/api/homepage/abc"));
        assert!(twice.starts_with(base));
    }

    #[test]
    fn layout_insertion_after_anchor_with_children() {
        let st = "layout:\n  Infra:\n  Apps:\n    style: row\n  Misc:\n    initiallyCollapsed: true\n";
        let out = ensure_layout(st, "AI", Some("Apps"), 4).unwrap();
        assert_eq!(
            out,
            "layout:\n  Infra:\n  Apps:\n    style: row\n  AI:\n    style: row\n    columns: 4\n  Misc:\n    initiallyCollapsed: true\n"
        );
        assert_eq!(ensure_layout(&out, "AI", Some("Apps"), 4), None);
    }

    #[test]
    fn layout_appended_without_anchor() {
        let out = ensure_layout("layout:\n  Infra:\n", "AI", None, 3).unwrap();
        assert_eq!(out, "layout:\n  Infra:\n  AI:\n    style: row\n    columns: 3\n");
    }
}
