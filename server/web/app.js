'use strict';
/* global t, applyI18n, setLang, LANG */

// =====================================================================
// Utilities
// =====================================================================
const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const H = 3600e3;
let state = null;
let providers = [];
let clockOffset = 0;
const now = () => Date.now() + clockOffset;

async function api(path, opts = {}) {
  const init = { method: opts.method || 'GET', headers: {} };
  if (init.method !== 'GET') init.headers['X-VibeRooms'] = '1';
  if (opts.body !== undefined) {
    init.headers['Content-Type'] = 'application/json';
    init.body = JSON.stringify(opts.body);
  }
  const res = await fetch(path, init);
  const json = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(json.error || `HTTP ${res.status}`);
  return json;
}

const ICON = {
  refresh: '<path d="M20 12a8 8 0 1 1-2.34-5.66M20 4v5h-5"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>',
  close: '<path d="M6 6l12 12M18 6L6 18"/>',
  back: '<path d="M15 18l-6-6 6-6"/>',
  more: '<circle cx="5" cy="12" r="1.2"/><circle cx="12" cy="12" r="1.2"/><circle cx="19" cy="12" r="1.2"/>',
  alert: '<path d="M12 3l9.5 17h-19zM12 10v4M12 17.5v.01"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
  ext: '<path d="M14 4h6v6M20 4l-9 9M19 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h5"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8v.01"/>',
  star: '<path d="M12 2.8l2.8 5.9 6.4.8-4.7 4.4 1.2 6.4L12 17.2l-5.7 3.1 1.2-6.4-4.7-4.4 6.4-.8z"/>',
};
const ic = (name, cls = 'ic') => `<svg class="${cls}" viewBox="0 0 24 24" aria-hidden="true">${ICON[name]}</svg>`;

function toast(msg) {
  const el = $('#toast');
  el.textContent = msg;
  el.classList.add('show');
  clearTimeout(toast.t);
  toast.t = setTimeout(() => el.classList.remove('show'), 2600);
}

// ---------- Formatting ----------
const locale = () => (LANG === 'fr' ? 'fr-FR' : 'en-GB');
const pct = (u) => `${Math.round(u)}`;
function fmtDur(ms) {
  if (ms == null) return '—';
  const m = Math.max(0, Math.round(ms / 60e3));
  if (m < 60) return `${m} ${t('min')}`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h} ${t('h')} ${String(m % 60).padStart(2, '0')}`;
  return `${Math.floor(h / 24)} ${t('d')} ${h % 24} ${t('h')}`;
}
function fmtAgo(ts) {
  if (!ts) return t('never');
  const s = Math.round((now() - ts) / 1000);
  if (s < 10) return t('just_now');
  if (s < 60) return t('ago_s', { n: s });
  if (s < 3600) return t('ago_min', { n: Math.round(s / 60) });
  return t('ago_dur', { d: fmtDur(s * 1000) });
}
const dayKey = (d) => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
const fmtTime = (d) => new Intl.DateTimeFormat(locale(), { hour: '2-digit', minute: '2-digit' }).format(d);
const fmtWeekday = (d) => new Intl.DateTimeFormat(locale(), { weekday: 'short' }).format(d);
const fmtDate = (d) => new Intl.DateTimeFormat(locale(), { day: 'numeric', month: 'short' }).format(d);
function fmtClock(ts) {
  const d = new Date(ts);
  if (dayKey(d) === dayKey(new Date(now()))) return fmtTime(d);
  if (dayKey(d) === dayKey(new Date(now() + 24 * H))) return `${t('tomorrow')} ${fmtTime(d)}`;
  if (ts - now() < 6 * 24 * H) return `${fmtWeekday(d)} ${fmtTime(d)}`;
  return fmtDate(d);
}
/** Short reset label for a tile row: a countdown under two days, a date beyond. */
const fmtResetShort = (ts) => (ts - now() < 48 * H ? fmtDur(ts - now()) : fmtClock(ts));
const CURRENCIES = ['USD', 'EUR', 'GBP', 'CNY', 'JPY', 'CHF', 'CAD'];
function fmtAmount(v, unit) {
  if (v == null) return '—';
  const u = (unit || '').toUpperCase();
  if (CURRENCIES.includes(u)) {
    try { return new Intl.NumberFormat(locale(), { style: 'currency', currency: u, currencyDisplay: 'narrowSymbol', maximumFractionDigits: Math.abs(v) >= 100 ? 0 : 2 }).format(v); } catch {}
  }
  const n = new Intl.NumberFormat(locale(), { maximumFractionDigits: Math.abs(v) >= 100 ? 0 : 2 }).format(v);
  return unit ? `${n} ${unit}` : n;
}
/** "90 / 300 req", "12,46 / 50 $": the unit once, on the ceiling. */
function usedOfLimit(w) {
  const isMoney = CURRENCIES.includes((w.unit || '').toUpperCase());
  const used = isMoney ? fmtAmount(w.used, w.unit) : new Intl.NumberFormat(locale(), { maximumFractionDigits: 1 }).format(w.used);
  return `${used} / ${fmtAmount(w.limit, w.unit)}`;
}
const accColor = (c) => (/^acc-\d$/.test(c || '') ? `var(--${c})` : c || 'var(--acc-1)');
function resolveColor(c) {
  if (!c.startsWith('var(')) return c;
  return getComputedStyle(document.documentElement).getPropertyValue(c.slice(4, -1)).trim() || '#888';
}
const levelOf = (used) => (used >= 100 ? 'full' : used >= 85 ? 'crit' : used >= 60 ? 'warn' : 'ok');

/** Preset names are bilingual: "Today|Aujourd'hui". */
const loc = (s) => { if (!s || !s.includes('|')) return s; const [en, fr] = s.split('|'); return LANG === 'fr' ? (fr || en) : en; };
function limitLabel(w0) {
  const w = { ...w0, name: loc(w0.name) };
  if (w.kind === 'session') {
    const h = w.windowMs ? Math.round(w.windowMs / H) : null;
    return h ? t('session_h', { h }) : t('session');
  }
  if (w.kind === 'week') return w.name || t('week');
  if (w.kind === 'day') return w.name || t('day');
  if (w.kind === 'month') return w.name || t('month');
  if (w.kind === 'balance') return w.name || t('balance');
  if (w.name) return w.windowMs && w.windowMs >= 6 * 24 * H ? t('week_of', { name: w.name }) : w.name;
  return w.key;
}
function shortLabel(w) {
  if (w.kind === 'session') return t('session');
  if (w.kind === 'model') return loc(w.name) || w.key;
  return limitLabel(w);
}
const ORDER = { session: 0, day: 1, week: 2, month: 3, balance: 4, model: 5, other: 6 };
const sortWindows = (ws) => [...ws].sort((a, b) => (b.hasPercent - a.hasPercent) || ((ORDER[a.kind] ?? 9) - (ORDER[b.kind] ?? 9)));

// =====================================================================
// Tiles
// =====================================================================
function ringHtml(headroom, size = 84) {
  const r = 34, c = 2 * Math.PI * r;
  const used = 100 - headroom;
  const lvl = levelOf(used);
  const dash = Math.max(0, Math.min(1, headroom / 100)) * c;
  return `<div class="ring lv-${lvl}" style="width:${size}px;height:${size}px" role="img" aria-label="${esc(t('aria_headroom', { p: pct(headroom) }))}">
    <svg viewBox="0 0 84 84" style="width:${size}px;height:${size}px"><circle class="track" cx="42" cy="42" r="${r}" fill="none" stroke-width="8"/>
    <circle class="val" cx="42" cy="42" r="${r}" fill="none" stroke-width="8" stroke-linecap="round" stroke-dasharray="${dash.toFixed(1)} ${c.toFixed(1)}"/></svg>
    <div class="ring-txt"><b class="num">${pct(headroom)}<span style="font-size:13px">%</span></b><small>${t('left')}</small></div></div>`;
}

/** The headline of a card: the headroom ring, or the main balance for prepaid accounts. */
function heroHtml(a, size) {
  if (a.headroom != null) return ringHtml(a.headroom, size);
  const bal = a.windows.find((w) => w.remaining != null) || a.windows[0];
  if (bal && bal.remaining != null) return `<div class="value-block"><b class="num">${esc(fmtAmount(bal.remaining, bal.unit))}</b><small>${t('remaining')}</small></div>`;
  return `<div class="value-block"><b>—</b><small>${t('waiting_first')}</small></div>`;
}

function limRow(w) {
  const lvl = w.hasPercent ? w.level : 'none';
  if (!w.hasPercent) {
    const val = w.remaining != null ? fmtAmount(w.remaining, w.unit) : fmtAmount(w.used, w.unit);
    return `<li class="amount lv-none"><span class="ln">${esc(shortLabel(w))}</span><span class="lv num">${esc(val)}</span><span class="lr">${w.remaining != null ? t('left') : t('used')}</span></li>`;
  }
  const value = w.limit != null && w.used != null ? usedOfLimit(w) : `${pct(w.utilization)} %`;
  const reset = w.resetsAt ? `↻ ${fmtResetShort(w.resetsAt)}` : (w.kind === 'session' ? t('free') : '');
  const pace = w.elapsed != null ? `<b style="left:${(w.elapsed * 100).toFixed(1)}%"></b>` : '';
  return `<li class="lv-${lvl}"><span class="ln">${esc(shortLabel(w))}</span><span class="lv num">${esc(value)}</span><span class="lr num">${esc(reset)}</span>
    <span class="bar"><i style="width:${w.utilization}%"></i>${pace}</span></li>`;
}

/** The single most useful warning of an account, if any. */
function tileNote(a) {
  if (a.status === 'reauth') {
    return `<div class="note crit">${ic('alert')}<span>${t(a.auth === 'oauth' ? 'oauth_expired' : 'key_refused')}</span><button class="btn sm" data-act="reconnect">${t('reconnect')}</button></div>`;
  }
  if (a.status === 'error') return `<div class="note crit">${ic('alert')}<span>${t('st_error')}</span></div>`;
  const full = a.windows.find((w) => w.hasPercent && w.level === 'full' && w.resetsAt);
  if (full) return `<div class="note crit">${ic('alert')}<span>${esc(t('blocked_until', { what: shortLabel(full), when: fmtClock(full.resetsAt) }))}</span></div>`;
  const hit = sortWindows(a.windows).find((w) => w.hasPercent && w.hitsBeforeReset && w.etaFull && ['session', 'day', 'week', 'month'].includes(w.kind));
  if (hit) return `<div class="note warn">${ic('alert')}<span>${esc(t('limit_around', { what: shortLabel(hit), when: fmtClock(hit.etaFull) }))}</span></div>`;
  if (a.status === 'stale' || a.status === 'degraded') return `<div class="note warn">${ic('info')}<span>${t('st_' + a.status)}</span></div>`;
  return '';
}

const STATUS_DOT = { ok: 'st-ok', degraded: 'st-warn', stale: 'st-warn', error: 'st-crit', reauth: 'st-crit', pending: 'st-pending' };

function tileHtml(a) {
  const ws = sortWindows(a.windows);
  const shown = ws.slice(0, 4);
  const more = ws.length - shown.length;
  const initial = (a.label || '?').trim().charAt(0).toUpperCase();
  return `<article class="tile ${a.recommended ? 'best' : ''}" data-id="${esc(a.id)}" style="--acc:${accColor(a.color)}" tabindex="0" aria-label="${esc(a.label)}">
    <div class="tile-head">
      <span class="avatar" aria-hidden="true">${esc(initial)}</span>
      <div class="who">
        <div class="name"><span class="t">${esc(a.label)}</span>${a.plan ? `<span class="plan">${esc(a.plan)}</span>` : ''}</div>
        <div class="mail">${esc(a.email || a.providerName)}</div>
      </div>
      <span class="dot ${STATUS_DOT[a.status] || ''}" title="${esc(t('st_' + a.status))}"></span>
    </div>
    <div class="tile-body">
      ${heroHtml(a)}
      <ul class="lims">${shown.map(limRow).join('') || `<li><span class="ln muted">${t('waiting_first')}</span></li>`}</ul>
    </div>
    ${more > 0 ? `<div class="more">${t('more_limits', { n: more })}</div>` : ''}
    ${tileNote(a)}
  </article>`;
}

function groups() {
  const map = new Map();
  for (const a of state.accounts) {
    if (!map.has(a.provider)) map.set(a.provider, { provider: a.provider, tool: a.tool, accounts: [] });
    map.get(a.provider).accounts.push(a);
  }
  return [...map.values()];
}

function groupHtml(g) {
  const recId = state.recommended?.[g.provider];
  const rec = g.accounts.find((a) => a.id === recId);
  const pill = rec ? `<span class="reco-pill">${ic('star')}${esc(t('use_now', { label: rec.label, p: pct(rec.headroom) }))}</span>` : '';
  const span = Math.min(3, g.accounts.length);
  return `<section class="group" style="--span:${span}"><div class="group-head"><h2>${esc(g.tool)}</h2><span class="count">${t('n_accounts', { n: g.accounts.length })}</span>${pill}</div>
    <div class="tiles">${g.accounts.map(tileHtml).join('')}</div></section>`;
}

// =====================================================================
// Main render
// =====================================================================
function renderMain() {
  const el = $('#main');
  if (!state.accounts.length) {
    el.innerHTML = `<div class="welcome"><h2>${t('welcome_title')}</h2><p>${t('welcome_text')}</p><div class="gallery-host"></div></div>`;
    renderGallery($('.gallery-host', el), (p) => openAdd(p));
    return;
  }
  el.innerHTML = `<div class="groups">${groups().map(groupHtml).join('')}</div>`;
}

function renderTable() {
  $('#tablePanel').hidden = !state.accounts.length;
  const head = `<tr><th>${t('th_account')}</th><th>${t('th_tool')}</th><th>${t('th_plan')}</th><th>${t('th_available')}</th><th>${t('th_limit')}</th><th>${t('th_value')}</th><th>${t('th_reset')}</th><th>${t('th_status')}</th></tr>`;
  const rows = state.accounts.flatMap((a) => (a.windows.length ? sortWindows(a.windows) : [null]).map((w, i) => `<tr>
    <td>${i ? '' : esc(a.label)}</td><td>${i ? '' : esc(a.tool)}</td><td>${i ? '' : esc(a.plan)}</td><td>${i || a.headroom == null ? '' : pct(a.headroom) + ' %'}</td>
    <td>${w ? esc(limitLabel(w)) : '—'}</td><td>${w ? esc(w.hasPercent ? pct(w.utilization) + ' %' : fmtAmount(w.remaining ?? w.used, w.unit)) : '—'}</td>
    <td>${w?.resetsAt ? esc(fmtClock(w.resetsAt)) : '—'}</td><td>${i ? '' : t('st_' + a.status)}</td></tr>`)).join('');
  $('#table').innerHTML = `<thead>${head}</thead><tbody>${rows}</tbody>`;
}

function renderMeta() {
  const last = Math.max(0, ...(state?.accounts || []).map((a) => a.fetchedAt || 0));
  $('#updated').textContent = last ? t('updated', { ago: fmtAgo(last) }) : '';
  $('#foot').innerHTML = `VibeRooms ${esc(state?.version || '')} · ${t('footer_poll')} · <a href="https://github.com/mifefr/viberooms" target="_blank" rel="noopener">GitHub</a>`;
}

let lastStamp = '';
function render() {
  if (!state) return;
  applyI18n();
  renderMain();
  renderTable();
  renderMeta();
  if (drawerId) renderDrawer(false);
}

async function loadState() {
  try {
    const s = await api('/api/state');
    clockOffset = s.now - Date.now();
    state = s;
    const stamp = JSON.stringify(s.accounts.map((a) => [a.id, a.fetchedAt, a.status, a.label, a.color, a.order]));
    if (stamp !== lastStamp || !$('#main').children.length) { lastStamp = stamp; render(); } else renderMeta();
  } catch (e) {
    $('#updated').textContent = t('unreachable', { msg: e.message });
  }
}

async function refresh(id) {
  const btn = $('#refreshAll');
  btn.classList.add('spin');
  btn.disabled = true;
  try {
    const r = await api('/api/refresh', { method: 'POST', body: id ? { id } : {} });
    state = r.state;
    clockOffset = state.now - Date.now();
    lastStamp = '';
    render();
  } catch (e) { toast(t('refresh_failed', { msg: e.message })); } finally { btn.classList.remove('spin'); btn.disabled = false; }
}

// =====================================================================
// Settings popover
// =====================================================================
function closePops() { $$('.pop').forEach((p) => p.remove()); }
function openSettings() {
  closePops();
  const theme = document.documentElement.dataset.theme || 'auto';
  const pop = document.createElement('div');
  pop.className = 'pop';
  pop.style.right = '0';
  pop.style.top = '40px';
  pop.innerHTML = `
    <div class="row"><span>${t('language')}</span><span class="seg" data-k="lang"><button data-v="fr" class="${LANG === 'fr' ? 'on' : ''}">FR</button><button data-v="en" class="${LANG === 'en' ? 'on' : ''}">EN</button></span></div>
    <div class="row"><span>${t('theme')}</span><span class="seg" data-k="theme"><button data-v="auto" class="${theme === 'auto' ? 'on' : ''}">${t('theme_auto')}</button><button data-v="light" class="${theme === 'light' ? 'on' : ''}">${t('theme_light')}</button><button data-v="dark" class="${theme === 'dark' ? 'on' : ''}">${t('theme_dark')}</button></span></div>
    <hr>
    <a class="item" href="https://github.com/mifefr/viberooms#readme" target="_blank" rel="noopener">${ic('info')}${t('docs')}</a>
    <div class="row muted small"><span>${t('version')}</span><span>${esc(state?.version || '')}</span></div>`;
  $('#settingsBtn').parentElement.appendChild(pop);
  pop.addEventListener('click', (e) => {
    e.stopPropagation();
    const b = e.target.closest('.seg button');
    if (!b) return;
    const k = b.parentElement.dataset.k;
    if (k === 'lang') setLang(b.dataset.v);
    if (k === 'theme') setTheme(b.dataset.v);
    openSettings();
    render();
  });
}
function setTheme(v) {
  if (v === 'auto') delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = v;
  try { localStorage.setItem('vr-theme', v); } catch {}
}
(function initTheme() {
  let v = 'auto';
  try { v = localStorage.getItem('vr-theme') || 'auto'; } catch {}
  if (v !== 'auto') document.documentElement.dataset.theme = v;
})();

// =====================================================================
// Drawer (account details)
// =====================================================================
let drawerId = null;
let drawerChart = { kind: 'session', hours: 24 };

function projection(w) {
  if (!w.hasPercent) return { txt: '', cls: '' };
  if (!w.resetsAt) return { txt: t('proj_free'), cls: '' };
  if (w.level === 'full') return { txt: t('proj_blocked', { when: fmtClock(w.resetsAt) }), cls: 'bad' };
  if (w.hitsBeforeReset && w.etaFull) return { txt: t('proj_hit', { when: fmtClock(w.etaFull) }), cls: 'bad' };
  if (w.ratePerHour > 0) return { txt: t('proj_holds'), cls: '' };
  if (w.ratePerHour === 0) return { txt: w.kind === 'session' ? t('proj_idle_session') : t('proj_idle'), cls: '' };
  return { txt: '', cls: '' };
}

function detailLimit(w) {
  const lvl = w.hasPercent ? w.level : 'none';
  const p = projection(w);
  const value = w.hasPercent ? `${pct(w.utilization)} %` : fmtAmount(w.remaining ?? w.used, w.unit);
  const amounts = w.used != null && w.limit != null
    ? t('amounts', { used: fmtAmount(w.used, w.unit), limit: fmtAmount(w.limit, w.unit), left: fmtAmount(w.remaining ?? w.limit - w.used, w.unit) })
    : w.remaining != null && !w.hasPercent ? t('remaining') : '';
  const reset = w.resetsAt ? t('reset_at', { when: fmtClock(w.resetsAt), dur: fmtDur(w.resetsAt - now()) }) : (w.kind === 'session' ? t('no_session') : w.kind === 'balance' ? t('no_reset') : '');
  const pace = w.elapsed != null ? `<b style="left:${(w.elapsed * 100).toFixed(1)}%" title="${esc(t('pace_tip', { p: Math.round(w.elapsed * 100) }))}"></b>` : '';
  return `<div class="d-lim lv-${lvl}">
    <div class="top"><span>${esc(limitLabel(w))}</span><b class="num">${esc(value)}</b></div>
    ${w.hasPercent ? `<div class="bar" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(w.utilization)}" aria-label="${esc(limitLabel(w))}"><i style="width:${w.utilization}%"></i>${pace}</div>` : ''}
    <div class="sub"><span>${esc([reset, amounts].filter(Boolean).join(' · '))}</span><span class="${p.cls}">${esc(p.txt)}</span></div>
  </div>`;
}

function openDrawer(id) {
  drawerId = id;
  const a = state.accounts.find((x) => x.id === id);
  drawerChart = { kind: a?.windows.find((w) => w.kind === 'session') ? 'session' : (a?.windows[0]?.kind || 'session'), hours: 24 };
  $('#scrim').hidden = false;
  requestAnimationFrame(() => { $('#scrim').classList.add('open'); $('#drawer').classList.add('open'); });
  $('#drawer').setAttribute('aria-hidden', 'false');
  renderDrawer(true);
}
function closeDrawer() {
  drawerId = null;
  $('#scrim').classList.remove('open');
  $('#drawer').classList.remove('open');
  $('#drawer').setAttribute('aria-hidden', 'true');
  setTimeout(() => { if (!drawerId) $('#scrim').hidden = true; }, 220);
}

function renderDrawer(focus) {
  const a = state.accounts.find((x) => x.id === drawerId);
  const el = $('#drawer');
  if (!a) { closeDrawer(); return; }
  const sum = a.headroom != null
    ? t('summary_headroom', { p: pct(a.headroom) })
    : (a.windows.find((w) => w.remaining != null) ? t('summary_balance') : t('waiting_first'));
  const chartKinds = [...new Set(sortWindows(a.windows).filter((w) => w.hasPercent || w.remaining != null).map((w) => w.kind))].slice(0, 3);
  const colors = ['acc-1', 'acc-2', 'acc-3', 'acc-4', 'acc-5'];
  el.innerHTML = `<header>
      <span class="avatar" style="--acc:${accColor(a.color)}">${esc((a.label || '?').charAt(0).toUpperCase())}</span>
      <div class="who"><div class="name"><span class="t">${esc(a.label)}</span>${a.plan ? `<span class="plan">${esc(a.plan)}</span>` : ''}</div>
      <div class="mail">${esc([a.providerName, a.email].filter(Boolean).join(' · '))}</div></div>
      <button class="btn ghost icon" data-d="close" aria-label="${t('close')}">${ic('close')}</button>
    </header>
    <div class="body">
      ${tileNote(a)}
      <div class="summary">${heroHtml(a, 76)}<p>${sum}${a.recommended ? `<br><b>${t('is_best')}</b>` : ''}</p></div>
      <section><h3>${t('limits')}</h3><div class="detail">${sortWindows(a.windows).map(detailLimit).join('') || `<p class="muted">${t('waiting_first')}</p>`}</div></section>
      ${a.extra?.enabled ? `<section><h3>${t('extra_credits')}</h3><p class="num">${esc(fmtAmount(a.extra.used, a.extra.currency))} / ${a.extra.limit != null ? esc(fmtAmount(a.extra.limit, a.extra.currency)) : t('unlimited')}</p></section>` : ''}
      ${a.breakdown?.length ? `<section><h3>${t('split')}</h3><p>${a.breakdown.map((b) => `${esc(b.name)} <b class="num">${pct(b.percent)} %</b>`).join(' · ')}</p></section>` : ''}
      ${chartKinds.length ? `<section><div class="chart-head"><h3 style="margin:0">${t('history')}</h3>
        <span class="seg" data-d="kind">${chartKinds.map((k) => `<button data-v="${k}" class="${k === drawerChart.kind ? 'on' : ''}">${esc(t('k_' + k))}</button>`).join('')}</span>
        <span class="seg" data-d="hours">${[24, 168, 720].map((h) => `<button data-v="${h}" class="${h === drawerChart.hours ? 'on' : ''}">${h >= 48 ? `${h / 24} ${t('d')}` : `${h} ${t('h')}`}</button>`).join('')}</span></div>
        <div class="chart" id="dchart"></div></section>` : ''}
      <section><h3>${t('account')}</h3>
        <dl class="kv">
          <dt>${t('th_tool')}</dt><dd>${esc(a.tool)}</dd>
          <dt>${t('status')}</dt><dd>${t('st_' + a.status)}${a.lastError ? ` — <span class="muted">${esc(a.lastError)}</span>` : ''}</dd>
          <dt>${t('measured')}</dt><dd>${esc(fmtAgo(a.fetchedAt))}</dd>
          <dt>${t('next_poll')}</dt><dd>${a.nextPollAt ? esc(a.nextPollAt - now() > 60e3 ? fmtDur(a.nextPollAt - now()) : t('soon')) : '—'}</dd>
        </dl>
      </section>
      <section><h3>${t('customize')}</h3>
        <div class="field"><label class="f" for="dname">${t('display_name')}</label>
          <div style="display:flex;gap:8px"><input class="in" id="dname" maxlength="40" value="${esc(a.label)}"><button class="btn" data-d="rename">${t('save')}</button></div></div>
        <div class="field"><label class="f">${t('color')}</label><div class="swatches">${colors.map((c) => `<button data-d="color" data-v="${c}" class="${a.color === c ? 'on' : ''}" style="--c:var(--${c})" aria-label="${c}"></button>`).join('')}</div></div>
      </section>
      <div class="danger-zone">
        <button class="btn" data-d="refresh">${ic('refresh')}${t('m_refresh')}</button>
        <button class="btn" data-d="reconnect">${t(a.auth === 'oauth' ? 'm_reconnect' : 'm_replace_key')}</button>
        <button class="btn danger" data-d="delete">${t('m_remove')}</button>
      </div>
    </div>`;
  if (chartKinds.length) loadDrawerChart(a);
  if (focus) $('[data-d="close"]', el).focus();
}

el('drawer').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-d], .seg button');
  if (!b) return;
  const a = state.accounts.find((x) => x.id === drawerId);
  if (!a) return;
  const seg = b.closest('.seg');
  try {
    if (seg?.dataset.d === 'kind') { drawerChart.kind = b.dataset.v; renderDrawer(false); return; }
    if (seg?.dataset.d === 'hours') { drawerChart.hours = Number(b.dataset.v); renderDrawer(false); return; }
    switch (b.dataset.d) {
      case 'close': closeDrawer(); break;
      case 'refresh': await refresh(a.id); break;
      case 'rename': {
        const label = $('#dname').value.trim();
        if (label) { state = await api(`/api/accounts/${a.id}`, { method: 'PATCH', body: { label } }); lastStamp = ''; render(); toast(t('saved')); }
        break;
      }
      case 'color': state = await api(`/api/accounts/${a.id}`, { method: 'PATCH', body: { color: b.dataset.v } }); lastStamp = ''; render(); break;
      case 'reconnect': openAdd(providers.find((p) => p.id === a.provider), a); break;
      case 'delete':
        if (confirm(t('confirm_remove', { label: a.label }))) { state = await api(`/api/accounts/${a.id}`, { method: 'DELETE' }); closeDrawer(); lastStamp = ''; render(); toast(t('removed')); }
        break;
      default:
    }
  } catch (err) { toast(err.message); }
});
function el(id) { return document.getElementById(id); }

async function loadDrawerChart(a) {
  const w = a.windows.find((x) => x.kind === drawerChart.kind);
  const box = $('#dchart');
  if (!w || !box) return;
  try {
    const r = await api(`/api/history?account=${encodeURIComponent(a.id)}&key=${encodeURIComponent(w.key)}&hours=${drawerChart.hours}`);
    drawChart(box, r.points, resolveColor(accColor(a.color)), w, drawerChart.hours);
  } catch { box.innerHTML = ''; }
}

function niceTicks(t0, t1, hours) {
  const step = hours <= 1.5 ? 15 * 60e3 : hours <= 3 ? 30 * 60e3 : hours <= 8 ? H : hours <= 16 ? 2 * H : hours <= 30 ? 6 * H : hours <= 80 ? 12 * H : hours <= 200 ? 24 * H : 7 * 24 * H;
  const d = new Date(t0);
  d.setSeconds(0, 0);
  if (step >= 24 * H) d.setHours(0, 0);
  else if (step >= H) d.setHours(Math.floor(d.getHours() / (step / H)) * (step / H), 0);
  else d.setMinutes(Math.floor(d.getMinutes() / (step / 60e3)) * (step / 60e3));
  const out = [];
  for (let x = d.getTime(); x <= t1; x += step) if (x >= t0) out.push(x);
  return out;
}

/** Single-series history chart. Percent windows plot utilisation; balances plot the remaining amount. */
function drawChart(box, points, color, w, hours) {
  const usePct = w.hasPercent;
  const val = (p) => p[1];
  const W = Math.max(260, box.clientWidth), Hh = box.clientHeight || 170;
  const m = { l: 36, r: 10, t: 8, b: 22 };
  const t1 = now();
  const first = points.length ? points[0][0] : null;
  const minSpan = hours <= 24 ? 2 * H : 6 * H;
  const t0 = first == null ? t1 - hours * H : Math.max(t1 - hours * H, Math.min(first - 5 * 60e3, t1 - minSpan));
  const span = (t1 - t0) / H;
  const max = usePct ? 100 : Math.max(1, ...points.map(val)) * 1.1;
  const x = (v) => m.l + ((v - t0) / (t1 - t0)) * (W - m.l - m.r);
  const y = (u) => m.t + (1 - u / max) * (Hh - m.t - m.b);
  const GAP = w.kind === 'session' ? 20 * 60e3 : 45 * 60e3;
  let svg = `<svg viewBox="0 0 ${W} ${Hh}" preserveAspectRatio="none" aria-hidden="true"><g class="grid axis">`;
  const ticksY = usePct ? [0, 50, 100] : [0, max / 2, max].map((v) => Math.round(v * 100) / 100);
  for (const u of ticksY) {
    svg += `<line class="${u === 0 ? 'baseline' : ''}" x1="${m.l}" x2="${W - m.r}" y1="${y(u)}" y2="${y(u)}"/>`;
    svg += `<text x="${m.l - 6}" y="${y(u) + 4}" text-anchor="end">${usePct ? `${u}%` : esc(new Intl.NumberFormat(locale(), { maximumFractionDigits: 1, notation: 'compact' }).format(u))}</text>`;
  }
  for (const tk of niceTicks(t0, t1, span)) {
    const d = new Date(tk);
    svg += `<text x="${x(tk)}" y="${Hh - 5}" text-anchor="middle">${esc(span <= 30 ? fmtTime(d) : fmtDate(d))}</text>`;
  }
  svg += '</g>';
  const pts = points.filter((p) => p[0] >= t0);
  if (pts.length) {
    let d = '', prev = null;
    for (const p of pts) {
      const newCycle = prev && prev[2] != null && p[2] != null && Math.abs(p[2] - prev[2]) > 5 * 60e3;
      d += `${!prev || p[0] - prev[0] > GAP || newCycle ? 'M' : 'L'}${x(p[0]).toFixed(1)},${y(val(p)).toFixed(1)}`;
      prev = p;
    }
    svg += `<g class="series"><path d="${d}" stroke="${color}"/></g>`;
  } else {
    svg += `<text class="nodata" x="${W / 2}" y="${Hh / 2}" text-anchor="middle">${esc(t('chart_empty'))}</text>`;
  }
  svg += `<g class="hover"></g><rect class="hit" x="${m.l}" y="0" width="${W - m.l - m.r}" height="${Hh}" fill="transparent"/></svg>`;
  box.innerHTML = svg;
  const svgEl = $('svg', box), hover = $('.hover', box), tip = $('#tooltip');
  $('.hit', box).addEventListener('pointermove', (ev) => {
    if (!pts.length) return;
    const r = svgEl.getBoundingClientRect();
    const tm = t0 + (((ev.clientX - r.left) / r.width) * W - m.l) / (W - m.l - m.r) * (t1 - t0);
    let best = pts[0];
    for (const p of pts) if (Math.abs(p[0] - tm) < Math.abs(best[0] - tm)) best = p;
    hover.innerHTML = `<line class="cross" x1="${x(best[0])}" x2="${x(best[0])}" y1="${m.t}" y2="${y(0)}"/><circle class="hover-dot" cx="${x(best[0])}" cy="${y(val(best))}" r="4.5" fill="${color}"/>`;
    tip.innerHTML = `<div class="t">${esc(fmtDate(new Date(best[0])))} ${esc(fmtTime(new Date(best[0])))}</div><b>${usePct ? `${Math.round(val(best))} %` : esc(fmtAmount(val(best), w.unit))}</b>`;
    tip.hidden = false;
    tip.style.left = `${Math.min(ev.clientX + 14, innerWidth - tip.offsetWidth - 8)}px`;
    tip.style.top = `${ev.clientY - tip.offsetHeight - 10}px`;
  });
  $('.hit', box).addEventListener('pointerleave', () => { hover.innerHTML = ''; tip.hidden = true; });
}

// =====================================================================
// Add account
// =====================================================================
const PCOLORS = ['#2a78d6', '#eb6834', '#1baf7a', '#6250d6', '#e87ba4', '#0e9aa7', '#c2410c', '#4d7c0f'];
function pColor(id) { let h = 0; for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0; return PCOLORS[h % PCOLORS.length]; }
const mono = (name) => name.replace(/[^A-Za-z0-9]/g, '').slice(0, 2).toUpperCase() || '?';
const CATS = [['subscription', 'cat_subscription'], ['api', 'cat_api'], ['custom', 'cat_custom']];
const pName = (p) => (p.id === 'custom' ? t('custom_name') : p.name);
const desc = (p) => (LANG === 'fr' ? p.description?.fr : p.description?.en) || '';
// Escaped description, with `inline code` rendered.
const descHtml = (p) => esc(desc(p)).replace(/`([^`]+)`/g, '<code>$1</code>');
const fieldLabel = (f) => (LANG === 'fr' ? f.label?.fr : f.label?.en) || f.id;

function renderGallery(host, onPick, filter = '') {
  const q = filter.trim().toLowerCase();
  const list = providers.filter((p) => !q || `${p.name} ${p.tool} ${desc(p)}`.toLowerCase().includes(q));
  host.innerHTML = CATS.map(([cat, label]) => {
    const items = list.filter((p) => p.category === cat);
    if (!items.length) return '';
    return `<div class="cat">${t(label)}</div><div class="gallery">${items.map((p) => `<button type="button" class="pcard" data-p="${esc(p.id)}">
      <span class="top"><span class="mono" style="--pc:${pColor(p.id)}">${esc(mono(pName(p)))}</span><span><span class="nm">${esc(pName(p))}</span>${p.tool && p.tool !== p.name && p.id !== 'custom' ? `<br><span class="tl">${esc(p.tool)}</span>` : ''}</span></span>
      <p>${descHtml(p)}</p>
      <span class="badges"><span class="badge">${t('auth_' + p.auth)}</span>${p.experimental ? `<span class="badge">${t('experimental')}</span>` : ''}</span>
    </button>`).join('')}</div>`;
  }).join('') || `<p class="muted">${t('no_match')}</p>`;
  $$('.pcard', host).forEach((b) => { b.onclick = () => onPick(providers.find((p) => p.id === b.dataset.p)); });
}

let add = null; // { step, provider, account, flow, … }
const dlg = () => $('#addDialog');

function openAdd(provider = null, account = null) {
  closeDrawer();
  add = { step: provider ? 'connect' : 'pick', provider, account, filter: '' };
  renderAdd();
  if (!dlg().open) dlg().showModal();
}

function addHeader(title, back) {
  return `<header>${back ? `<button class="btn ghost icon" data-a="back" aria-label="${t('back')}">${ic('back')}</button>` : ''}<h2>${esc(title)}</h2>
    <button class="btn ghost icon" data-a="close" aria-label="${t('close')}">${ic('close')}</button></header>`;
}

function renderAdd() {
  const body = $('#addBody');
  const p = add.provider;
  if (add.step === 'pick') {
    body.innerHTML = `${addHeader(t('add_title'))}<div class="content"><input class="search" id="psearch" placeholder="${esc(t('search_service'))}" value="${esc(add.filter)}" autocomplete="off"><div id="pgallery"></div></div>`;
    renderGallery($('#pgallery'), (prov) => { add.provider = prov; add.step = 'connect'; renderAdd(); }, add.filter);
    $('#psearch').oninput = (e) => { add.filter = e.target.value; renderGallery($('#pgallery'), (prov) => { add.provider = prov; add.step = 'connect'; renderAdd(); }, add.filter); };
    setTimeout(() => $('#psearch')?.focus(), 30);
    return;
  }
  if (add.step === 'done') { renderDone(body); return; }
  const title = add.account ? t('reconnect_title', { label: add.account.label }) : pName(p);
  if (p.auth === 'oauth') renderOauth(body, title);
  else if (p.auth === 'apiKey') renderKey(body, title);
  else renderCustom(body, title);
  setTimeout(() => $('#addBody input:not([disabled])')?.focus(), 30);
}

function nameField() {
  if (add.account) return '';
  const n = state.accounts.filter((a) => a.provider === add.provider.id).length;
  const base = pName(add.provider);
  const suggestion = n ? `${base} ${n + 1}` : base;
  return `<div class="field"><label class="f" for="aname">${t('display_name')}</label><input class="in" id="aname" maxlength="40" placeholder="${esc(suggestion)}" autocomplete="off"><div class="hint">${t('name_hint')}</div></div>`;
}
const chosenName = () => ($('#aname')?.value.trim() || $('#aname')?.placeholder || '');

// ---------- OAuth ----------
function renderOauth(body, title) {
  const p = add.provider;
  const pasteUrl = p.loginMode === 'pasteRedirectUrl';
  const st = add.flow ? (add.busy ? 'busy' : 'waiting') : 'start';
  body.innerHTML = `${addHeader(title, !add.account)}<div class="content">
    <p class="muted" style="margin-bottom:16px">${descHtml(p)}</p>
    ${nameField()}
    <div class="steps" style="margin-top:18px">
      <div class="step ${st === 'start' ? 'active' : 'done'}"><span class="n">1</span><div>
        <h4>${t('oauth_s1', { name: p.name })}</h4>
        <p>${t('oauth_s1_text')}</p>
        <button class="btn primary" data-a="open">${ic('ext')}${t('oauth_open', { name: p.name })}</button>
      </div></div>
      <div class="step ${st === 'start' ? 'inactive' : 'active'}"><span class="n">2</span><div>
        <h4>${t(pasteUrl ? 'oauth_s2_url' : 'oauth_s2_code')}</h4>
        <p>${t(pasteUrl ? 'oauth_s2_url_text' : 'oauth_s2_code_text')}</p>
        <input class="in mono" id="acode" autocomplete="off" spellcheck="false" ${st === 'start' ? 'disabled' : ''} placeholder="${pasteUrl ? 'http://localhost:1455/auth/callback?code=…' : 'xxxxxxxx#yyyyyyyy'}">
        <p class="msg" id="amsg" role="status">${st === 'busy' ? esc(t('connecting')) : ''}</p>
      </div></div>
    </div>
    <div class="callout" style="margin-top:6px">${ic('info')}<span>${t('oauth_note')}</span></div>
  </div>`;
  const code = $('#acode');
  const looksValid = (v) => (pasteUrl ? /[?&]code=/.test(v) : /^[\w-]{8,}#[\w-]{8,}$/.test(v.trim()));
  code.addEventListener('input', () => { if (looksValid(code.value) && !add.busy) finishOauth(code.value); });
  code.addEventListener('keydown', (e) => { if (e.key === 'Enter' && code.value.trim()) finishOauth(code.value); });
  if (st !== 'start') code.focus();
}

async function startOauth() {
  const win = window.open('about:blank', '_blank');
  try {
    add.label = chosenName();
    const r = await api('/api/login/start', { method: 'POST', body: { provider: add.provider.id, label: add.label, accountId: add.account?.id } });
    add.flow = r.flowId;
    if (win) { win.opener = null; win.location.href = r.url; } else window.open(r.url, '_blank', 'noopener');
    renderAdd();
  } catch (e) { win?.close(); toast(e.message); }
}

async function finishOauth(value) {
  add.busy = true;
  const msg = $('#amsg');
  if (msg) { msg.textContent = t('connecting'); msg.className = 'msg'; }
  try {
    const r = await api('/api/login/finish', { method: 'POST', body: { flowId: add.flow, code: value } });
    add.result = r;
    add.step = 'done';
    add.busy = false;
    await waitForAccount(r.id);
    renderAdd();
  } catch (e) {
    add.busy = false;
    if (msg) { msg.textContent = e.message; msg.className = 'msg err'; }
  }
}

// ---------- API key ----------
function renderKey(body, title) {
  const p = add.provider;
  body.innerHTML = `${addHeader(title, !add.account)}<div class="content">
    <p class="muted" style="margin-bottom:16px">${descHtml(p)}</p>
    ${nameField()}
    ${p.fields.map((f) => `<div class="field"><label class="f" for="f_${esc(f.id)}">${esc(fieldLabel(f))}${f.optional ? ` <span class="muted">(${t('optional')})</span>` : ''}</label>
      <input class="in ${f.secret ? 'mono' : ''}" id="f_${esc(f.id)}" data-f="${esc(f.id)}" ${f.secret ? 'type="password"' : ''} placeholder="${esc(f.placeholder || '')}" autocomplete="off" spellcheck="false"></div>`).join('')}
    ${p.keyUrl ? `<p class="hint" style="margin-top:8px"><a href="${esc(p.keyUrl)}" target="_blank" rel="noopener">${t('where_key')} ${ic('ext')}</a></p>` : ''}
    <div class="callout" style="margin-top:16px">${ic('info')}<span>${t('key_note')}</span></div>
    <p class="msg" id="amsg" role="status" style="margin-top:10px"></p>
  </div>
  <footer><button class="btn primary" data-a="key-submit">${t('test_and_add')}</button></footer>`;
  setTimeout(() => $('#aname, [data-f]')?.focus(), 30);
}

async function submitKey() {
  const fields = {};
  $$('[data-f]').forEach((i) => { fields[i.dataset.f] = i.value.trim(); });
  const msg = $('#amsg');
  msg.textContent = t('testing'); msg.className = 'msg';
  try {
    const r = await api('/api/accounts/key', { method: 'POST', body: { provider: add.provider.id, label: chosenName(), fields, accountId: add.account?.id } });
    add.result = r;
    add.step = 'done';
    await waitForAccount(r.id);
    renderAdd();
  } catch (e) { msg.textContent = e.message; msg.className = 'msg err'; }
}

// ---------- Custom connector ----------
const ROLES = ['', 'remaining', 'used', 'limit', 'percent', 'percent_remaining', 'resets_at'];
function leaves(v, path = '', out = []) {
  if (out.length > 300) return out;
  if (Array.isArray(v)) v.slice(0, 20).forEach((x, i) => leaves(x, `${path}/${i}`, out));
  else if (v && typeof v === 'object') Object.entries(v).forEach(([k, x]) => leaves(x, `${path}/${k.replace(/~/g, '~0').replace(/\//g, '~1')}`, out));
  else if (typeof v === 'number' || (typeof v === 'string' && (/^-?\d+(\.\d+)?$/.test(v.trim()) || /^\d{4}-\d\d-\d\dT/.test(v)))) out.push({ path: path || '/', value: v });
  return out;
}

function customSpec() {
  const c = add.custom;
  const headers = {};
  if (c.headerName.trim()) headers[c.headerName.trim()] = c.headerValue;
  const lim = { key: 'main', label: c.label || undefined, kind: c.kind, unit: c.unit || undefined };
  for (const [path, role] of Object.entries(c.roles)) if (role) lim[role === 'resets_at' ? 'resetsAt' : role === 'percent_remaining' ? 'percentRemaining' : role] = path;
  if (c.kind === 'month' && !lim.resetsAt) lim.reset = 'monthly';
  if (c.kind === 'day' && !lim.resetsAt) lim.reset = 'daily';
  const hasValue = ['remaining', 'used', 'limit', 'percent', 'percentRemaining'].some((k) => lim[k]);
  return { name: chosenName() || 'Custom', url: c.url.trim(), method: c.method, headers, limits: hasValue ? [lim] : [] };
}

function renderCustom(body, title) {
  add.custom ||= { url: '', method: 'GET', headerName: 'Authorization', headerValue: 'Bearer {{key}}', key: '', roles: {}, kind: 'balance', unit: '', label: '', raw: null, leaves: [], error: '' };
  const c = add.custom;
  const roleOpt = (r) => `<option value="${r}">${t('role_' + (r || 'none'))}</option>`;
  body.innerHTML = `${addHeader(title, !add.account)}<div class="content">
    <p class="muted" style="margin-bottom:16px">${descHtml(add.provider)}</p>
    ${nameField()}
    <div class="field"><label class="f" for="curl">${t('c_url')}</label>
      <div class="cols"><input class="in mono" id="curl" value="${esc(c.url)}" placeholder="https://api.example.com/v1/credits" spellcheck="false"><select class="in" id="cmethod"><option ${c.method === 'GET' ? 'selected' : ''}>GET</option><option ${c.method === 'POST' ? 'selected' : ''}>POST</option></select></div></div>
    <div class="field"><label class="f">${t('c_auth')}</label>
      <div class="cols" style="grid-template-columns:150px 1fr"><input class="in mono" id="chname" value="${esc(c.headerName)}" placeholder="Authorization"><input class="in mono" id="chvalue" value="${esc(c.headerValue)}" placeholder="Bearer {{key}}"></div>
      <div class="hint">${t('c_auth_hint')}</div></div>
    <div class="field"><label class="f" for="ckey">${t('c_key')}</label><input class="in mono" id="ckey" type="password" value="${esc(c.key)}" autocomplete="off"></div>
    <div style="margin-top:14px"><button class="btn" data-a="c-test">${t('c_test')}</button> <span class="msg ${c.error ? 'err' : ''}" id="amsg">${esc(c.error)}</span></div>
    ${c.raw ? `<div class="field" style="margin-top:18px"><label class="f">${t('c_fields')}</label><div class="hint" style="margin:-2px 0 8px">${t('c_fields_hint')}</div>
      <div class="paths">${c.leaves.map((l, i) => `<div class="p"><code>${esc(l.path)}</code><span class="v">${esc(String(l.value))}</span>
        <select class="in" data-role="${i}">${ROLES.map(roleOpt).join('')}</select></div>`).join('') || `<p class="muted small">${t('c_no_fields')}</p>`}</div></div>
      <div class="cols" style="grid-template-columns:1fr 1fr 110px;margin-top:12px">
        <div><label class="f" for="clabel">${t('c_label')}</label><input class="in" id="clabel" value="${esc(c.label)}" placeholder="${esc(t('c_label_ph'))}"></div>
        <div><label class="f" for="ckind">${t('c_kind')}</label><select class="in" id="ckind">${['balance', 'month', 'day', 'week', 'session'].map((k) => `<option value="${k}" ${c.kind === k ? 'selected' : ''}>${t('k_' + k)}</option>`).join('')}</select></div>
        <div><label class="f" for="cunit">${t('c_unit')}</label><input class="in" id="cunit" value="${esc(c.unit)}" placeholder="USD"></div>
      </div>
      <details class="adv"><summary>${t('c_advanced')}</summary><textarea class="in" id="cspec" rows="8"></textarea><div class="hint">${t('c_advanced_hint')}</div></details>` : ''}
  </div>
  ${c.raw ? `<footer><button class="btn primary" data-a="c-add">${t('test_and_add')}</button></footer>` : ''}`;
  $$('[data-role]').forEach((s) => {
    const l = c.leaves[Number(s.dataset.role)];
    s.value = c.roles[l.path] || '';
    s.onchange = () => { c.roles[l.path] = s.value; syncSpec(); };
  });
  const bind = (id, k) => { const i = $(id); if (i) i.oninput = () => { c[k] = i.value; syncSpec(); }; };
  bind('#curl', 'url'); bind('#chname', 'headerName'); bind('#chvalue', 'headerValue'); bind('#ckey', 'key'); bind('#clabel', 'label'); bind('#cunit', 'unit');
  const m = $('#cmethod'); if (m) m.onchange = () => { c.method = m.value; };
  const k = $('#ckind'); if (k) k.onchange = () => { c.kind = k.value; syncSpec(); };
  syncSpec();
}
function syncSpec() { const ta = $('#cspec'); if (ta && document.activeElement !== ta) ta.value = JSON.stringify(customSpec(), null, 2); }

async function testCustom() {
  const c = add.custom;
  const msg = $('#amsg');
  msg.textContent = t('testing'); msg.className = 'msg';
  try {
    const r = await api('/api/connectors/test', { method: 'POST', body: { provider: 'custom', fields: { key: c.key }, connector: { ...customSpec(), limits: [] } } });
    if (!r.ok) { c.error = r.error; c.raw = null; renderAdd(); return; }
    c.error = '';
    c.raw = r.raw;
    try { c.leaves = leaves(JSON.parse(r.raw)); } catch { c.leaves = []; }
    renderAdd();
  } catch (e) { msg.textContent = e.message; msg.className = 'msg err'; }
}

async function addCustom() {
  const c = add.custom;
  let spec = customSpec();
  const ta = $('#cspec');
  if (ta && ta.value.trim() && ta.value !== JSON.stringify(spec, null, 2)) {
    try { spec = JSON.parse(ta.value); } catch { toast(t('c_bad_json')); return; }
  }
  if (!spec.limits?.length) { toast(t('c_pick_one')); return; }
  const msg = $('#amsg');
  msg.textContent = t('testing'); msg.className = 'msg';
  try {
    const r = await api('/api/accounts/key', { method: 'POST', body: { provider: 'custom', label: chosenName(), fields: { key: c.key }, connector: spec, accountId: add.account?.id } });
    add.result = r;
    add.step = 'done';
    await waitForAccount(r.id);
    renderAdd();
  } catch (e) { msg.textContent = e.message; msg.className = 'msg err'; }
}

// ---------- Success ----------
async function waitForAccount(id) {
  for (let i = 0; i < 8; i++) {
    await loadState();
    const a = state.accounts.find((x) => x.id === id);
    if (a && a.windows.length) return a;
    await new Promise((r) => setTimeout(r, 900));
  }
  return null;
}

function renderDone(body) {
  const a = state.accounts.find((x) => x.id === add.result?.id);
  const chips = a ? sortWindows(a.windows).slice(0, 4).map((w) => `<span>${esc(shortLabel(w))} · ${esc(w.hasPercent ? pct(w.utilization) + ' %' : fmtAmount(w.remaining ?? w.used, w.unit))}</span>`).join('') : '';
  body.innerHTML = `${addHeader(t('add_title'))}<div class="content"><div class="success">
      <div class="big-ok">${ic('check')}</div>
      <h3>${esc(t(add.result?.reconnected ? 'done_reconnected' : 'done_added', { label: a?.label || '' }))}</h3>
      <p class="muted">${esc([a?.providerName, a?.plan, a?.email].filter(Boolean).join(' · '))}</p>
      ${chips ? `<div class="found">${chips}</div>` : `<p class="muted small" style="margin-top:12px">${t('done_first_measure')}</p>`}
    </div></div>
    <footer><button class="btn left" data-a="another">${t('add_another')}</button><button class="btn primary" data-a="close">${t('done')}</button></footer>`;
}

$('#addBody').addEventListener('click', (e) => {
  const b = e.target.closest('[data-a]');
  if (!b) return;
  switch (b.dataset.a) {
    case 'close': dlg().close(); break;
    case 'back': add = { step: 'pick', filter: '' }; renderAdd(); break;
    case 'open': startOauth(); break;
    case 'key-submit': submitKey(); break;
    case 'c-test': testCustom(); break;
    case 'c-add': addCustom(); break;
    case 'another': openAdd(); break;
    default:
  }
});
dlg().addEventListener('close', () => { add = null; });

// =====================================================================
// Wiring
// =====================================================================
$('#refreshAll').innerHTML = ic('refresh');
$('#settingsBtn').innerHTML = ic('gear');
$('.ic-plus').outerHTML = ic('plus');
$('#refreshAll').onclick = () => refresh();
$('#addAccount').onclick = () => openAdd();
$('#settingsBtn').onclick = (e) => { e.stopPropagation(); if ($('.pop')) closePops(); else openSettings(); };
$('#scrim').onclick = closeDrawer;
document.addEventListener('click', (e) => { if (!e.target.closest('.pop')) closePops(); });
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') { closePops(); if (drawerId && !dlg().open) closeDrawer(); }
});
$('#main').addEventListener('click', (e) => {
  const rc = e.target.closest('[data-act="reconnect"]');
  const tile = e.target.closest('.tile');
  if (!tile) return;
  const a = state.accounts.find((x) => x.id === tile.dataset.id);
  if (rc) { e.stopPropagation(); openAdd(providers.find((p) => p.id === a.provider), a); return; }
  openDrawer(tile.dataset.id);
});
$('#main').addEventListener('keydown', (e) => {
  const tile = e.target.closest('.tile');
  if (tile && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); openDrawer(tile.dataset.id); }
});
window.addEventListener('resize', () => { clearTimeout(window.__rz); window.__rz = setTimeout(() => { const a = state?.accounts.find((x) => x.id === drawerId); if (a) loadDrawerChart(a); }, 150); });

// Deep links: #account/<id> opens an account, #add or #add/<provider> the add dialog.
function route() {
  const [kind, arg] = location.hash.replace(/^#/, '').split('/');
  if (kind === 'account' && state?.accounts.some((a) => a.id === arg)) openDrawer(arg);
  else if (kind === 'add' && providers.length) openAdd(arg ? providers.find((p) => p.id === arg) || null : null);
}
window.addEventListener('hashchange', route);

applyI18n();
Promise.all([
  api('/api/providers').then((r) => { providers = r.providers || []; }).catch(() => {}),
  loadState(),
]).then(() => { if (state && !state.accounts.length) renderMain(); route(); });
setInterval(loadState, 20e3);
setInterval(() => { if (state) renderMeta(); }, 5e3);
document.addEventListener('visibilitychange', () => { if (!document.hidden) loadState(); });
