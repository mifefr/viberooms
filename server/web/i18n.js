'use strict';
// Minimal i18n: `t(key, vars)`, `[data-i18n]` / `[data-i18n-title]` / `[data-i18n-placeholder]` attributes.

const DICT = {
  en: {
    subtitle: 'AI subscriptions limits, self-hosted',
    refresh: 'Refresh', refresh_all: 'Refresh all accounts', add_account: 'Add account', toggle_theme: 'Toggle light / dark', toggle_lang: 'Français',
    last_measure: 'Last measure {ago}', unreachable: 'Service unreachable: {msg}', refresh_failed: 'Refresh failed: {msg}',
    just_now: 'just now', ago_s: '{n} s ago', ago_min: '{n} min ago', ago_dur: '{d} ago', never: 'never',
    min: 'min', h: 'h', d: 'd', tomorrow: 'tomorrow',
    st_ok: 'Up to date', st_degraded: 'Error, cached values', st_stale: 'Stale data', st_error: 'Error', st_reauth: 'Sign-in required', st_pending: 'Collecting…',
    lvl_warn: 'High', lvl_crit: 'Critical', lvl_full: 'Limit reached',
    session: 'Session', session_h: '{h}-hour session', week: 'Week', day: 'Day', week_of: 'Week · {name}',
    available_before_limit: 'available before the next limit', waiting_first: 'waiting for the first measure',
    session_reset_in: 'Session resets in', session_free: 'Session', free: 'free',
    reset_at: 'Resets {when} · in {dur}', no_session: 'No session in progress', not_started: 'Window not started yet',
    proj_free: 'Fully available', proj_blocked: 'Blocked until {when}', proj_hit: '⚠︎ Limit around {when} at this pace',
    proj_holds: 'Holds until the reset at this pace', proj_idle_session: 'Idle for the last 45 min', proj_idle: 'Stable over 24 h',
    pace_tip: 'Time elapsed in the window: {p} %. If the bar is past this mark, you are burning faster than a linear pace.',
    oauth_expired: 'The OAuth session of this account has expired.', reconnect: 'Reconnect',
    extra_credits: 'Extra usage credits (month)', unlimited: 'unlimited', split: 'Weekly usage by surface',
    updated: 'Updated {ago}', next_poll_in: 'Next collection in {dur}', next_poll_soon: 'Next collection imminent',
    recommended: '★ Best', reco_title: 'Use now', reco_line: '{tool}: {label} — {pct} % available', reco_why_session: 'session at {p} %', reco_why_free: 'session free', reco_why_week: 'week at {p} %',
    empty_title: 'No account connected', empty_text: 'Add your AI subscriptions to follow their limits in real time.',
    menu_actions: 'Account actions', m_refresh: 'Refresh now', m_rename: 'Rename…', m_reconnect: 'Reconnect account…', m_left: 'Move left', m_right: 'Move right', m_remove: 'Remove account',
    prompt_rename: 'New account name', confirm_remove: 'Stop tracking “{label}”? Its history will be deleted.',
    chart_session: 'Session · history', chart_session_desc: 'Usage of the short rolling window. Each drop is a reset.',
    chart_week: 'Week · history', chart_week_desc: 'Weekly limit (all models).', period: 'Period',
    chart_empty: 'History fills up with every collection (every 3 min).',
    zoom_note: 'Tracking since {when}: the axis will widen up to {range} as data comes in',
    table_view: 'Table view', th_account: 'Account', th_tool: 'Tool', th_plan: 'Plan', th_available: 'Available', th_reset: 'Reset', th_status: 'Status', th_measure: 'Measured',
    footer: 'VibeRooms {v} · {n} account(s) · collected every 3 min · {url}',
    login_title: 'Add an account', login_reconnect: 'Reconnect “{label}”', close: 'Close',
    login_provider: 'Service', login_label: 'Display name', login_label_ph: 'e.g. Personal, Work, Freelance…',
    login_open_text: 'Open the authorization page and sign in <b>with the right account</b>. For another account than the one in your browser, use a private window.',
    login_open: 'Open the authorization page',
    login_hint_code: 'Link opened. Click “Authorize”, then copy the code shown.',
    login_hint_url: 'Link opened. After authorizing, your browser lands on a localhost page that may fail to load: copy the full address from the address bar.',
    login_paste_code: 'Paste the code', login_paste_url: 'Paste the full address (http://localhost…)',
    login_note: 'VibeRooms gets its own OAuth session (profile and usage read only). Your CLI sessions are not touched.',
    connect: 'Connect', connecting: 'Connecting…', connected: '✓ Connected: {email}', reconnected: '✓ Reconnected: {email}',
    experimental: 'experimental',
  },
  fr: {
    subtitle: 'Quotas de tes abonnements IA, auto-hébergé',
    refresh: 'Rafraîchir', refresh_all: 'Rafraîchir tous les comptes', add_account: 'Ajouter un compte', toggle_theme: 'Basculer clair / sombre', toggle_lang: 'English',
    last_measure: 'Dernière mesure {ago}', unreachable: 'Service injoignable : {msg}', refresh_failed: 'Échec du rafraîchissement : {msg}',
    just_now: "à l'instant", ago_s: 'il y a {n} s', ago_min: 'il y a {n} min', ago_dur: 'il y a {d}', never: 'jamais',
    min: 'min', h: 'h', d: 'j', tomorrow: 'demain',
    st_ok: 'À jour', st_degraded: 'Erreur, valeurs en cache', st_stale: 'Données anciennes', st_error: 'Erreur', st_reauth: 'Reconnexion requise', st_pending: 'Collecte…',
    lvl_warn: 'Élevé', lvl_crit: 'Critique', lvl_full: 'Limite atteinte',
    session: 'Session', session_h: 'Session {h} h', week: 'Semaine', day: 'Jour', week_of: 'Semaine · {name}',
    available_before_limit: 'disponible avant la prochaine limite', waiting_first: 'en attente de la première mesure',
    session_reset_in: 'Session : reset dans', session_free: 'Session', free: 'libre',
    reset_at: 'Reset {when} · dans {dur}', no_session: 'Pas de session en cours', not_started: 'Fenêtre pas encore entamée',
    proj_free: 'Entièrement disponible', proj_blocked: "Bloqué jusqu'à {when}", proj_hit: '⚠︎ Limite vers {when} à ce rythme',
    proj_holds: "Tient jusqu'au reset à ce rythme", proj_idle_session: 'Inactif ces 45 dernières min', proj_idle: 'Stable sur 24 h',
    pace_tip: 'Temps écoulé dans la fenêtre : {p} %. Si la barre dépasse ce repère, tu consommes plus vite que le rythme linéaire.',
    oauth_expired: 'La session OAuth de ce compte a expiré.', reconnect: 'Reconnecter',
    extra_credits: 'Crédits supplémentaires (mois)', unlimited: 'illimité', split: 'Semaine par usage',
    updated: 'Mis à jour {ago}', next_poll_in: 'Prochaine collecte dans {dur}', next_poll_soon: 'Prochaine collecte imminente',
    recommended: '★ Recommandé', reco_title: 'À utiliser maintenant', reco_line: '{tool} : {label} — {pct} % disponible', reco_why_session: 'session à {p} %', reco_why_free: 'session libre', reco_why_week: 'semaine à {p} %',
    empty_title: 'Aucun compte connecté', empty_text: 'Ajoute tes abonnements IA pour suivre leurs quotas en temps réel.',
    menu_actions: 'Actions du compte', m_refresh: 'Rafraîchir maintenant', m_rename: 'Renommer…', m_reconnect: 'Reconnecter le compte…', m_left: 'Déplacer vers la gauche', m_right: 'Déplacer vers la droite', m_remove: 'Retirer le compte',
    prompt_rename: 'Nouveau nom du compte', confirm_remove: "Retirer « {label} » du suivi ? L'historique de ce compte sera supprimé.",
    chart_session: 'Session · historique', chart_session_desc: 'Utilisation de la fenêtre glissante courte. Chaque chute correspond à un reset.',
    chart_week: 'Semaine · historique', chart_week_desc: 'Quota hebdomadaire (tous modèles).', period: 'Période',
    chart_empty: "L'historique se remplit à chaque collecte (toutes les 3 min).",
    zoom_note: "Suivi depuis {when} : l'axe s'élargira jusqu'à {range} au fil des collectes",
    table_view: 'Vue tableau', th_account: 'Compte', th_tool: 'Outil', th_plan: 'Offre', th_available: 'Disponible', th_reset: 'Reset', th_status: 'Statut', th_measure: 'Mesure',
    footer: 'VibeRooms {v} · {n} compte(s) · collecte toutes les 3 min · {url}',
    login_title: 'Ajouter un compte', login_reconnect: 'Reconnecter « {label} »', close: 'Fermer',
    login_provider: 'Service', login_label: 'Nom affiché', login_label_ph: 'ex. Perso, Boulot, Freelance…',
    login_open_text: "Ouvre la page d'autorisation et connecte-toi <b>avec le bon compte</b>. Pour un autre compte que celui de ton navigateur, utilise une fenêtre privée.",
    login_open: "Ouvrir la page d'autorisation",
    login_hint_code: 'Lien ouvert. Clique « Authorize », puis copie le code affiché.',
    login_hint_url: "Lien ouvert. Après l'autorisation, ton navigateur arrive sur une page localhost qui peut ne pas se charger : copie l'adresse complète de la barre d'adresse.",
    login_paste_code: 'Colle le code obtenu', login_paste_url: "Colle l'adresse complète (http://localhost…)",
    login_note: "VibeRooms obtient sa propre session OAuth (lecture du profil et de l'usage uniquement). Tes sessions CLI ne sont pas touchées.",
    connect: 'Connecter', connecting: 'Connexion…', connected: '✓ Connecté : {email}', reconnected: '✓ Reconnecté : {email}',
    experimental: 'expérimental',
  },
};

let LANG = 'en';
try { LANG = localStorage.getItem('vr-lang') || ''; } catch {}
if (!DICT[LANG]) LANG = (navigator.language || 'en').toLowerCase().startsWith('fr') ? 'fr' : 'en';

function t(key, vars = {}) {
  const s = (DICT[LANG] && DICT[LANG][key]) ?? DICT.en[key] ?? key;
  return s.replace(/\{(\w+)\}/g, (_, k) => (vars[k] ?? ''));
}

function applyI18n(root = document) {
  document.documentElement.lang = LANG;
  root.querySelectorAll('[data-i18n]').forEach((el) => { el.textContent = t(el.dataset.i18n); });
  root.querySelectorAll('[data-i18n-html]').forEach((el) => { el.innerHTML = t(el.dataset.i18nHtml); });
  root.querySelectorAll('[data-i18n-title]').forEach((el) => { el.title = t(el.dataset.i18nTitle); el.setAttribute('aria-label', t(el.dataset.i18nTitle)); });
  root.querySelectorAll('[data-i18n-placeholder]').forEach((el) => { el.placeholder = t(el.dataset.i18nPlaceholder); });
}

function setLang(l) {
  LANG = l;
  try { localStorage.setItem('vr-lang', l); } catch {}
  applyI18n();
}
