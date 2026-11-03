'use strict';
/* global t, applyI18n, setLang, LANG */

// ---------- Utilities ----------
const $ = (s, el = document) => el.querySelector(s);
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const H = 3600e3;
let state = null;
let providers = [];
let clockOffset = 0; // server - client
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
const locale = () => (LANG === 'fr' ? 'fr-FR' : 'en-GB');
const dayKey = (d) => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
const fmtTime = (d) => new Intl.DateTimeFormat(locale(), { hour: '2-digit', minute: '2-digit' }).format(d);
const fmtWeekday = (d) => new Intl.DateTimeFormat(locale(), { weekday: 'short' }).format(d);
const fmtDate = (d) => new Intl.DateTimeFormat(locale(), { day: '2-digit', month: '2-digit' }).format(d);
function fmtClock(ts) {
  const d = new Date(ts);
  if (dayKey(d) === dayKey(new Date(now()))) return fmtTime(d);
  if (dayKey(d) === dayKey(new Date(now() + 24 * H))) return `${t('tomorrow')} ${fmtTime(d)}`;
  if (ts - now() < 6 * 24 * H) return `${fmtWeekday(d)} ${fmtTime(d)}`;
  return `${fmtDate(d)} ${fmtTime(d)}`;
}
function fmtMoney(v, cur) {
  if (v == null) return '—';
  try { return new Intl.NumberFormat(locale(), { style: 'currency', currency: cur || 'USD' }).format(v); }
  catch { return `${v.toFixed(2)} ${cur || ''}`; }
}
const accColor = (c) => (/^acc-\d$/.test(c || '') ? `var(--${c})` : c || 'var(--acc-1)');

function limitLabel(w) {
  if (w.kind === 'session') {
    const h = w.windowMs ? Math.round(w.windowMs / H) : null;
    return h ? t('session_h', { h }) : t('session');
  }
  if (w.kind === 'week') return t('week');
  if (w.name) return w.windowMs && w.windowMs >= 6 * 24 * H ? t('week_of', { name: w.name }) : w.name;
  return w.kind === 'day' ? t('day') : w.key;
}
const shortLabel = (w) => (w.name || (w.kind === 'week' ? t('week') : w.kind === 'day' ? t('day') : t('session')));

const ICON = {
  star: '<svg viewBox="0 0 24 24"><path d="M12 2.8l2.8 5.9 6.4.8-4.7 4.4 1.2 6.4L12 17.2l-5.7 3.1 1.2-6.4-4.7-4.4 6.4-.8z"/></svg>',
  alert: '<svg viewBox="0 0 24 24"><path d="M12 3l9.5 17h-19zM12 10v4M12 17.5v.01"/></svg>',
  more: '<svg viewBox="0 0 24 24"><circle cx="5" cy="12" r="1.3"/><circle cx="12" cy="12" r="1.3"/><circle cx="19" cy="12" r="1.3"/></svg>',
};

const STATUS_CLASS = { ok: 'ok', degraded: 'warn', stale: 'warn', error: 'crit', reauth: 'crit', pending: 'pending' };
const LEVEL_KEY = { warn: 'lvl_warn', crit: 'lvl_crit', full: 'lvl_full' };

function projection(w) {
  if (!w.resetsAt) return { txt: t('proj_free'), cls: 'good' };
  if (w.level === 'full') return { txt: t('proj_blocked', { when: fmtClock(w.resetsAt) }), cls: 'bad' };
  if (w.hitsBeforeReset && w.etaFull) return { txt: t('proj_hit', { when: fmtClock(w.etaFull) }), cls: 'bad' };
  if (w.ratePerHour > 0) return { txt: t('proj_holds'), cls: 'good' };
  if (w.ratePerHour === 0) return { txt: w.kind === 'session' ? t('proj_idle_session') : t('proj_idle'), cls: '' };
  return { txt: '', cls: '' };
}

function meterHtml(w, minor = false) {
  const p = projection(w);
  const label = limitLabel(w);
  const reset = w.resetsAt ? t('reset_at', { when: fmtClock(w.resetsAt), dur: fmtDur(w.resetsAt - now()) }) : (w.kind === 'session' ? t('no_session') : t('not_started'));
  const pace = w.elapsed != null ? `<div class="pace" style="left:${(w.elapsed * 100).toFixed(1)}%" title="${esc(t('pace_tip', { p: Math.round(w.elapsed * 100) }))}"></div>` : '';
  const lvl = LEVEL_KEY[w.level] ? `<small>${t(LEVEL_KEY[w.level])}</small>` : '';
  const track = `<div class="track" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(w.utilization)}" aria-label="${esc(label)}"><div class="fill" style="width:${w.utilization}%"></div>${pace}</div>`;
  if (minor) {
    return `<div class="meter minor lvl-${w.level}" title="${esc(label)} · ${esc(reset)}">
      <div class="meter-top"><span class="name">${esc(shortLabel(w))}</span><span class="val">${pct(w.utilization)} %${lvl}</span></div>${track}</div>`;
  }
  return `<div class="meter lvl-${w.level}">
    <div class="meter-top"><span class="name">${esc(label)}</span><span class="val">${pct(w.utilization)} %${lvl}</span></div>${track}
    <div class="meter-sub"><span>${esc(reset)}</span><span class="proj ${p.cls}">${esc(p.txt)}</span></div>
  </div>`;
}

function cardHtml(a) {
  const scls = STATUS_CLASS[a.status] || 'pending';
  const main = a.windows.filter((w) => ['session', 'week', 'day'].includes(w.kind));
  const minor = a.windows.filter((w) => !['session', 'week', 'day'].includes(w.kind));
  const hero = a.headroom != null
    ? `<div><div class="big">${pct(a.headroom)}<span>%</span></div><div class="caption">${t('available_before_limit')}</div></div>`
    : `<div><div class="big">—</div><div class="caption">${t('waiting_first')}</div></div>`;
  const nextReset = a.session?.resetsAt
    ? `<div class="side">${t('session_reset_in')}<b>${fmtDur(a.session.resetsAt - now())}</b></div>`
    : a.session ? `<div class="side">${t('session_free')}<b>${t('free')}</b></div>` : '';
  let alert = '';
  if (a.status === 'reauth') alert = `<div class="alert">${ICON.alert}<span>${t('oauth_expired')}</span><button class="btn" data-act="reconnect">${t('reconnect')}</button></div>`;
  else if (['error', 'degraded', 'stale'].includes(a.status) && a.lastError) alert = `<div class="alert warn">${ICON.alert}<span>${esc(a.lastError)}</span></div>`;
  const extra = a.extra?.enabled
    ? `<div class="extra"><span>${t('extra_credits')}</span><span><b>${fmtMoney(a.extra.used, a.extra.currency)}</b> / ${a.extra.limit != null ? fmtMoney(a.extra.limit, a.extra.currency) : t('unlimited')}${a.extra.utilization != null ? ` · ${pct(a.extra.utilization)} %` : ''}</span></div>`
    : '';
  const split = a.breakdown?.length
    ? `<div class="split"><span>${t('split')}</span><span>${a.breakdown.map((b) => `${esc(b.name)} <b>${pct(b.percent)} %</b>`).join(' · ')}</span></div>`
    : '';
  const left = a.nextPollAt ? a.nextPollAt - now() : null;
  const next = left == null ? '' : left > 60e3 ? t('next_poll_in', { dur: fmtDur(left) }) : t('next_poll_soon');
  const org = a.orgName && !String(a.orgName).includes(a.email || '\u0000') ? ` · ${esc(a.orgName)}` : '';
  return `<article class="card ${a.recommended ? 'recommended' : ''}" data-id="${esc(a.id)}" style="--acc:${accColor(a.color)}">
    <div class="card-head">
      <span class="chip" aria-hidden="true"></span>
      <div class="card-title">
        <h2>${esc(a.label)} <span class="badge">${esc([a.providerName, a.plan].filter(Boolean).join(' '))}</span>${a.recommended ? `<span class="badge reco-badge">${t('recommended')}</span>` : ''}</h2>
        <div class="sub" title="${esc(a.email || '')}">${esc(a.email || '—')}${org}</div>
      </div>
      <span class="status ${scls}"><i></i>${t('st_' + a.status)}</span>
      <div class="menu-wrap"><button class="btn ghost icon" data-act="menu" aria-label="${t('menu_actions')}" aria-haspopup="menu">${ICON.more}</button></div>
    </div>
    <div class="hero">${hero}${nextReset}</div>
    ${alert}
    <div class="meters">${main.map((w) => meterHtml(w)).join('')}</div>
    ${minor.length ? `<div class="minor-grid">${minor.map((w) => meterHtml(w, true)).join('')}</div>` : ''}
    ${extra}${split}
    <div class="card-foot"><span data-ago="${a.fetchedAt || ''}">${t('updated', { ago: fmtAgo(a.fetchedAt) })}</span><span>${esc(next)}</span></div>
  </article>`;
}

// ---------- Main rendering ----------
function byProvider() {
  const groups = new Map();
  for (const a of state.accounts) {
    if (!groups.has(a.provider)) groups.set(a.provider, { tool: a.tool, name: a.providerName, accounts: [] });
    groups.get(a.provider).accounts.push(a);
  }
  return groups;
}

function renderReco() {
  const el = $('#reco');
  const recs = state.accounts.filter((a) => a.recommended);
  if (!recs.length) { el.hidden = true; return; }
  const lines = recs.map((r) => {
    const why = [];
    if (r.session) why.push(r.session.resetsAt ? t('reco_why_session', { p: pct(r.session.utilization) }) : t('reco_why_free'));
    if (r.week) why.push(t('reco_why_week', { p: pct(r.week.utilization) }));
    return `<div><b>${esc(t('reco_line', { tool: r.tool, label: r.label, pct: pct(r.headroom) }))}</b> <span class="why small">${esc(why.join(' · '))}</span></div>`;
  });
  el.style.setProperty('--acc', accColor(recs[0].color));
  el.innerHTML = `<span class="star" aria-hidden="true">${ICON.star}</span><div><div class="muted small">${t('reco_title')}</div>${lines.join('')}</div>`;
  el.hidden = false;
}

function renderGroups() {
  const el = $('#groups');
  if (!state.accounts.length) {
    el.innerHTML = `<section class="cards"><div class="empty"><h2>${t('empty_title')}</h2><p>${t('empty_text')}</p><button class="btn primary" id="emptyAdd">${t('add_account')}</button></div></section>`;
    $('#emptyAdd').onclick = () => openLogin();
    return;
  }
  const groups = byProvider();
  const titled = groups.size > 1;
  el.innerHTML = [...groups.values()].map((g) => `<section class="group">
      ${titled ? `<h2 class="group-title">${esc(g.tool)}</h2>` : ''}
      <div class="cards">${g.accounts.map(cardHtml).join('')}</div>
    </section>`).join('');
}

function renderTable() {
  const panel = $('#tablePanel');
  panel.hidden = !state.accounts.length;
  const head = `<tr><th>${t('th_account')}</th><th>${t('th_tool')}</th><th>${t('th_plan')}</th><th>${t('th_available')}</th><th>${t('session')}</th><th>${t('th_reset')}</th><th>${t('week')}</th><th>${t('th_reset')}</th><th>${t('th_status')}</th><th>${t('th_measure')}</th></tr>`;
  const cell = (w) => (w ? `<td>${pct(w.utilization)} %</td><td>${w.resetsAt ? fmtClock(w.resetsAt) : '—'}</td>` : '<td>—</td><td>—</td>');
  const rows = state.accounts.map((a) => `<tr><td>${esc(a.label)}</td><td>${esc(a.tool)}</td><td>${esc(a.plan)}</td><td>${a.headroom != null ? pct(a.headroom) + ' %' : '—'}</td>${cell(a.session)}${cell(a.week)}<td>${t('st_' + a.status)}</td><td>${a.fetchedAt ? fmtClock(a.fetchedAt) : '—'}</td></tr>`).join('');
  $('#table').innerHTML = `<thead>${head}</thead><tbody>${rows}</tbody>`;
}

function renderMeta() {
  const last = Math.max(0, ...state.accounts.map((a) => a.fetchedAt || 0));
  $('#updated').textContent = last ? t('last_measure', { ago: fmtAgo(last) }) : '';
  $('#footInfo').textContent = t('footer', { v: state.version, n: state.accounts.length, url: state.publicUrl });
  document.querySelectorAll('[data-ago]').forEach((el) => { el.textContent = t('updated', { ago: fmtAgo(Number(el.dataset.ago) || null) }); });
}

let lastDataStamp = '';
function render() {
  applyI18n();
  document.querySelectorAll('.seg button').forEach((b) => {
    const h = Number(b.dataset.hours);
    b.textContent = h >= 48 ? `${h / 24} ${t('d')}` : `${h} ${t('h')}`;
  });
  $('#langLabel').textContent = LANG === 'fr' ? 'EN' : 'FR';
  renderReco();
  if (!document.querySelector('.menu')) renderGroups(); // do not close an open menu
  renderTable();
  renderMeta();
  $('#charts').hidden = !state.accounts.length;
  const stamp = state.accounts.map((a) => `${a.id}:${a.fetchedAt}:${a.color}:${a.label}`).join('|') + LANG;
  if (stamp !== lastDataStamp) {
    lastDataStamp = stamp;
    loadCharts();
  }
}

async function loadState() {
  try {
    const s = await api('/api/state');
    clockOffset = s.now - Date.now();
    state = s;
    render();
  } catch (e) {
    $('#updated').textContent = t('unreachable', { msg: e.message });
  }
}

// ---------- Account actions ----------
function closeMenus() { document.querySelectorAll('.menu').forEach((m) => m.remove()); }

function openMenu(card, btn) {
  closeMenus();
  const id = card.dataset.id;
  const idx = state.accounts.findIndex((a) => a.id === id);
  const m = document.createElement('div');
  m.className = 'menu';
  m.setAttribute('role', 'menu');
  m.innerHTML = `
    <button data-m="refresh" role="menuitem">${t('m_refresh')}</button>
    <button data-m="rename" role="menuitem">${t('m_rename')}</button>
    <button data-m="reconnect" role="menuitem">${t('m_reconnect')}</button>
    ${idx > 0 ? `<button data-m="up" role="menuitem">${t('m_left')}</button>` : ''}
    ${idx < state.accounts.length - 1 ? `<button data-m="down" role="menuitem">${t('m_right')}</button>` : ''}
    <hr><button data-m="delete" class="danger" role="menuitem">${t('m_remove')}</button>`;
  btn.parentElement.appendChild(m);
  m.addEventListener('click', async (ev) => {
    const act = ev.target.closest('button')?.dataset.m;
    if (!act) return;
    closeMenus();
    const acc = state.accounts[idx];
    try {
      if (act === 'refresh') await refresh(id);
      if (act === 'rename') {
        const label = prompt(t('prompt_rename'), acc.label);
        if (label && label.trim()) { state = await api(`/api/accounts/${id}`, { method: 'PATCH', body: { label } }); render(); }
      }
      if (act === 'reconnect') openLogin(acc);
      if (act === 'up' || act === 'down') {
        const other = state.accounts[act === 'up' ? idx - 1 : idx + 1];
        await api(`/api/accounts/${id}`, { method: 'PATCH', body: { order: other.order } });
        state = await api(`/api/accounts/${other.id}`, { method: 'PATCH', body: { order: acc.order } });
        render();
      }
      if (act === 'delete' && confirm(t('confirm_remove', { label: acc.label }))) {
        state = await api(`/api/accounts/${id}`, { method: 'DELETE' });
        render();
      }
    } catch (e) { alert(e.message); }
  });
}

async function refresh(id) {
  const btn = $('#refreshAll');
  btn.classList.add('spin');
  btn.disabled = true;
  try {
    const r = await api('/api/refresh', { method: 'POST', body: id ? { id } : {} });
    state = r.state;
    clockOffset = state.now - Date.now();
    render();
  } catch (e) {
    $('#updated').textContent = t('refresh_failed', { msg: e.message });
  } finally {
    btn.classList.remove('spin');
    btn.disabled = false;
  }
}

$('#groups').addEventListener('click', (ev) => {
  const card = ev.target.closest('.card');
  const btn = ev.target.closest('[data-act]');
  if (!card || !btn) return;
  const acc = state.accounts.find((a) => a.id === card.dataset.id);
  if (btn.dataset.act === 'menu') { ev.stopPropagation(); openMenu(card, btn); }
  if (btn.dataset.act === 'reconnect') openLogin(acc);
});
document.addEventListener('click', (ev) => { if (!ev.target.closest('.menu')) closeMenus(); });
document.addEventListener('keydown', (ev) => { if (ev.key === 'Escape') closeMenus(); });
$('#refreshAll').onclick = () => refresh();
$('#addAccount').onclick = () => openLogin();
$('#langToggle').onclick = () => { setLang(LANG === 'fr' ? 'en' : 'fr'); lastDataStamp = ''; if (state) render(); };

// ---------- OAuth sign-in ----------
let flow = null;
function pickedProvider() { return $('#providerPick input:checked')?.value || providers[0]?.id || 'claude'; }
function modeOf(pid) { return providers.find((p) => p.id === pid)?.loginMode || 'pasteCode'; }

function updateLoginTexts() {
  const mode = flow?.mode || modeOf(pickedProvider());
  $('#codeLabel').textContent = mode === 'pasteRedirectUrl' ? t('login_paste_url') : t('login_paste_code');
  $('#loginCode').placeholder = mode === 'pasteRedirectUrl' ? 'http://localhost:…/?code=…' : 'xxxxxxxx#yyyyyyyy';
}

function openLogin(acc = null) {
  flow = { accountId: acc?.id || null, flowId: null, mode: acc ? modeOf(acc.provider) : null };
  $('#loginTitle').textContent = acc ? t('login_reconnect', { label: acc.label }) : t('login_title');
  $('#stepLabel').hidden = !!acc;
  $('#stepProvider').hidden = !!acc || providers.length < 2;
  $('#providerPick').innerHTML = providers.map((p, i) => `<label class="pp"><input type="radio" name="provider" value="${esc(p.id)}" ${i === 0 ? 'checked' : ''}><span><b>${esc(p.name)}</b> · ${esc(p.tool)}${p.experimental ? ` <em>${t('experimental')}</em>` : ''}</span></label>`).join('');
  $('#providerPick').onchange = updateLoginTexts;
  $('#loginLabel').value = '';
  $('#loginCode').value = '';
  $('#loginCode').disabled = true;
  $('#loginSubmit').disabled = true;
  $('#authHint').hidden = true;
  setMsg('');
  updateLoginTexts();
  applyI18n($('#loginDialog'));
  $('#loginDialog').showModal();
  (acc ? $('#openAuth') : $('#loginLabel')).focus();
}
function setMsg(txt, cls = '') { const m = $('#loginMsg'); m.textContent = txt; m.className = `form-msg ${cls}`; }

$('#openAuth').onclick = async () => {
  // Open the window synchronously (user gesture), then redirect it: avoids the pop-up blocker.
  const win = window.open('about:blank', '_blank');
  try {
    const r = await api('/api/login/start', { method: 'POST', body: { provider: pickedProvider(), label: $('#loginLabel').value, accountId: flow.accountId } });
    flow.flowId = r.flowId;
    flow.mode = r.loginMode;
    if (win) { win.opener = null; win.location.href = r.url; } else window.open(r.url, '_blank', 'noopener');
    $('#authHint').textContent = r.loginMode === 'pasteRedirectUrl' ? t('login_hint_url') : t('login_hint_code');
    $('#authHint').hidden = false;
    $('#loginCode').disabled = false;
    updateLoginTexts();
    $('#loginCode').focus();
    setMsg('');
  } catch (e) {
    win?.close();
    setMsg(e.message, 'err');
  }
};
$('#loginCode').addEventListener('input', () => { $('#loginSubmit').disabled = !$('#loginCode').value.trim(); });
$('#loginCode').addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); $('#loginSubmit').click(); } });
$('#loginSubmit').onclick = async () => {
  const btn = $('#loginSubmit');
  btn.disabled = true;
  setMsg(t('connecting'));
  try {
    const r = await api('/api/login/finish', { method: 'POST', body: { flowId: flow.flowId, code: $('#loginCode').value } });
    setMsg(t(r.reconnected ? 'reconnected' : 'connected', { email: r.email || '' }), 'ok');
    setTimeout(() => $('#loginDialog').close(), 900);
    setTimeout(loadState, 1500);
    setTimeout(loadState, 4000);
  } catch (e) {
    setMsg(e.message, 'err');
    btn.disabled = false;
  }
};

// ---------- Theme ----------
(function theme() {
  let th = null;
  try { th = localStorage.getItem('vr-theme'); } catch {}
  if (th) document.documentElement.dataset.theme = th;
  $('#themeToggle').onclick = () => {
    const cur = document.documentElement.dataset.theme || (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
    const next = cur === 'dark' ? 'light' : 'dark';
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem('vr-theme', next); } catch {}
    lastDataStamp = '';
    render();
  };
})();

// ---------- Charts ----------
const ranges = { session: 24, week: 168 };
const historyCache = new Map();

document.querySelectorAll('.chart-panel').forEach((panel) => {
  panel.querySelector('.seg').addEventListener('click', (ev) => {
    const b = ev.target.closest('button');
    if (!b) return;
    panel.querySelectorAll('.seg button').forEach((x) => x.classList.toggle('on', x === b));
    ranges[panel.dataset.kind] = Number(b.dataset.hours);
    loadChart(panel);
  });
});

function loadCharts() { document.querySelectorAll('.chart-panel').forEach(loadChart); }

async function loadChart(panel) {
  if (!state?.accounts.length) return;
  const kind = panel.dataset.kind;
  const hours = ranges[kind];
  // Each provider names its windows differently: chart the account's own window of that kind.
  const targets = state.accounts.map((a) => ({ a, w: a.windows.find((w) => w.kind === kind) })).filter((x) => x.w);
  const series = await Promise.all(targets.map(async ({ a, w }) => {
    const base = { id: a.id, label: a.label, color: resolveColor(accColor(a.color)), sessionMs: w.windowMs };
    try {
      const r = await api(`/api/history?account=${encodeURIComponent(a.id)}&key=${encodeURIComponent(w.key)}&hours=${hours}`);
      return { ...base, points: r.points };
    } catch { return { ...base, points: [] }; }
  }));
  historyCache.set(kind, { hours, series });
  drawChart(panel, kind);
}

function resolveColor(c) {
  if (!c.startsWith('var(')) return c;
  return getComputedStyle(document.documentElement).getPropertyValue(c.slice(4, -1)).trim() || '#888';
}

window.addEventListener('resize', () => { clearTimeout(window.__rz); window.__rz = setTimeout(() => document.querySelectorAll('.chart-panel').forEach((p) => drawChart(p, p.dataset.kind)), 150); });

function niceTicks(t0, t1, hours) {
  const step = hours <= 1.5 ? 15 * 60e3 : hours <= 3 ? 30 * 60e3 : hours <= 8 ? H : hours <= 16 ? 2 * H
    : hours <= 30 ? 4 * H : hours <= 80 ? 12 * H : hours <= 200 ? 24 * H : 5 * 24 * H;
  const d = new Date(t0);
  d.setSeconds(0, 0);
  if (step >= 24 * H) d.setHours(0, 0);
  else if (step >= H) d.setHours(Math.floor(d.getHours() / (step / H)) * (step / H), 0);
  else d.setMinutes(Math.floor(d.getMinutes() / (step / 60e3)) * (step / 60e3));
  const out = [];
  for (let x = d.getTime(); x <= t1; x += step) if (x >= t0) out.push(x);
  return out;
}
function tickLabel(x, hours) {
  const d = new Date(x);
  if (hours <= 30) return fmtTime(d);
  if (hours <= 80) return `${fmtWeekday(d)} ${d.getHours()}h`;
  return fmtDate(d);
}

function drawChart(panel, kind) {
  const data = historyCache.get(kind);
  const box = panel.querySelector('.chart');
  if (!data) return;
  const { hours, series } = data;
  const W = Math.max(280, box.clientWidth);
  const Hh = box.clientHeight || 220;
  const longest = Math.max(0, ...series.map((s) => (series.length > 1 ? s.label.length + 1 : 0) + 5));
  const m = { l: 34, r: Math.max(56, 16 + longest * 6.8), t: 10, b: 24 };
  const t1 = now();
  const firsts = series.filter((s) => s.points.length).map((s) => s.points[0][0]);
  const first = firsts.length ? Math.min(...firsts) : null;
  const minSpan = kind === 'session' ? 2 * H : 6 * H;
  const t0 = first == null ? t1 - hours * H : Math.max(t1 - hours * H, Math.min(first - 5 * 60e3, t1 - minSpan));
  const span = (t1 - t0) / H;
  const zoomed = t0 > t1 - hours * H + 60e3;
  const x = (v) => m.l + ((v - t0) / (t1 - t0)) * (W - m.l - m.r);
  const y = (u) => m.t + (1 - u / 100) * (Hh - m.t - m.b);
  const GAP = kind === 'session' ? 20 * 60e3 : 45 * 60e3;

  const range = hours >= 48 ? `${Math.round(hours / 24)} ${t('d')}` : `${hours} ${t('h')}`;
  panel.querySelector('.legend').innerHTML = (series.length > 1 ? series.map((s) => `<span><i style="background:${s.color}"></i>${esc(s.label)}</span>`).join('') : '')
    + (zoomed && first != null ? `<span class="zoom-note">${esc(t('zoom_note', { when: fmtClock(first), range }))}</span>` : '');

  let svg = `<svg viewBox="0 0 ${W} ${Hh}" preserveAspectRatio="none" aria-hidden="true"><g class="grid axis">`;
  for (const u of [0, 25, 50, 75, 100]) {
    svg += u === 0 ? `<line class="baseline" x1="${m.l}" x2="${W - m.r}" y1="${y(0)}" y2="${y(0)}"/>` : `<line x1="${m.l}" x2="${W - m.r}" y1="${y(u)}" y2="${y(u)}"/>`;
    svg += `<text x="${m.l - 8}" y="${y(u) + 4}" text-anchor="end">${u}%</text>`;
  }
  for (const tk of niceTicks(t0, t1, span)) svg += `<text x="${x(tk)}" y="${Hh - 6}" text-anchor="middle">${esc(tickLabel(tk, span))}</text>`;
  svg += '</g>';

  const ends = [];
  let any = false;
  for (const s of series) {
    const pts = s.points.filter((p) => p[0] >= t0);
    if (!pts.length) continue;
    any = true;
    let d = '';
    let prev = null;
    for (const p of pts) {
      // New cycle only when the reset really moves (> 5 min), not for a few seconds of jitter.
      const newCycle = prev && prev[2] != null && p[2] != null && Math.abs(p[2] - prev[2]) > 5 * 60e3;
      const brk = !prev || p[0] - prev[0] > GAP || newCycle;
      d += `${brk ? 'M' : 'L'}${x(p[0]).toFixed(1)},${y(p[1]).toFixed(1)}`;
      prev = p;
    }
    svg += `<g class="series"><path d="${d}" stroke="${s.color}"/></g>`;
    const last = pts[pts.length - 1];
    ends.push({ s, x: x(last[0]), y: y(last[1]), u: last[1] });
  }
  if (!any) svg += `<text class="nodata" x="${(W + m.l - m.r) / 2}" y="${Hh / 2}" text-anchor="middle">${esc(t('chart_empty'))}</text>`;
  ends.sort((a, b) => a.y - b.y);
  for (let i = 1; i < ends.length; i++) if (ends[i].y - ends[i - 1].y < 14) ends[i].y = ends[i - 1].y + 14;
  for (const e of ends) svg += `<text class="dlabel" x="${e.x + 8}" y="${e.y + 4}">${series.length > 1 ? esc(e.s.label) + ' ' : ''}${Math.round(e.u)} %</text>`;
  svg += `<g class="hover"></g><rect class="hit" x="${m.l}" y="0" width="${W - m.l - m.r}" height="${Hh}" fill="transparent" stroke="none"/></svg>`;
  box.innerHTML = svg;
  box.setAttribute('aria-label', `${panel.querySelector('h2').textContent}: ${series.map((s) => `${s.label} ${s.points.length ? Math.round(s.points[s.points.length - 1][1]) + ' %' : '—'}`).join(', ')}`);

  const svgEl = box.querySelector('svg');
  const hover = box.querySelector('.hover');
  const tip = $('#tooltip');
  const hit = box.querySelector('.hit');
  hit.addEventListener('pointermove', (ev) => {
    const r = svgEl.getBoundingClientRect();
    const px = ((ev.clientX - r.left) / r.width) * W;
    const tm = t0 + ((px - m.l) / (W - m.l - m.r)) * (t1 - t0);
    const tol = Math.max(GAP, (t1 - t0) / 60);
    const rows = [];
    let tx = null;
    for (const s of series) {
      let best = null;
      for (const p of s.points) if (!best || Math.abs(p[0] - tm) < Math.abs(best[0] - tm)) best = p;
      if (best && Math.abs(best[0] - tm) <= tol) { rows.push({ s, p: best }); if (tx == null || Math.abs(best[0] - tm) < Math.abs(tx - tm)) tx = best[0]; }
    }
    if (!rows.length) { hover.innerHTML = ''; tip.hidden = true; return; }
    let h = `<line class="cross" x1="${x(tx)}" x2="${x(tx)}" y1="${m.t}" y2="${y(0)}"/>`;
    for (const { s, p } of rows) h += `<circle class="hover-dot" cx="${x(p[0])}" cy="${y(p[1])}" r="4.5" fill="${s.color}"/>`;
    hover.innerHTML = h;
    tip.innerHTML = `<div class="t">${esc(fmtDate(new Date(tx)))} ${esc(fmtTime(new Date(tx)))}</div>`
      + rows.map(({ s, p }) => `<div class="r"><span><i style="background:${s.color}"></i>${esc(s.label)}</span><b>${Math.round(p[1])} %</b></div>`).join('');
    tip.hidden = false;
    const tw = tip.offsetWidth, th = tip.offsetHeight;
    let left = ev.clientX + 14, top = ev.clientY - th - 10;
    if (left + tw > innerWidth - 8) left = ev.clientX - tw - 14;
    if (top < 8) top = ev.clientY + 16;
    tip.style.left = `${left}px`;
    tip.style.top = `${top}px`;
  });
  hit.addEventListener('pointerleave', () => { hover.innerHTML = ''; tip.hidden = true; });
}

// ---------- Loop ----------
applyI18n();
api('/api/providers').then((r) => { providers = r.providers || []; }).catch(() => {});
loadState();
setInterval(loadState, 20e3);
setInterval(() => { if (state) renderMeta(); }, 5e3);
document.addEventListener('visibilitychange', () => { if (!document.hidden) loadState(); });
