'use strict';
// Minimal i18n: `t(key, vars)` + `[data-i18n]`, `[data-i18n-title]`, `[data-i18n-html]`, `[data-i18n-placeholder]`.

const DICT = {
  en: {
    subtitle: 'AI subscriptions & API limits, self-hosted', refresh_all: 'Refresh all accounts', add_account: 'Add account', settings: 'Settings',
    updated: 'Updated {ago}', unreachable: 'Server unreachable: {msg}', refresh_failed: 'Refresh failed: {msg}',
    just_now: 'just now', ago_s: '{n} s ago', ago_min: '{n} min ago', ago_dur: '{d} ago', never: 'never', soon: 'imminent',
    min: 'min', h: 'h', d: 'd', tomorrow: 'tomorrow',
    st_ok: 'Up to date', st_degraded: 'Error, showing cached values', st_stale: 'Stale data', st_error: 'Collection error', st_reauth: 'Sign-in required', st_pending: 'Collecting…',
    session: 'Session', session_h: '{h}-hour session', week: 'Week', day: 'Day', month: 'Month', balance: 'Balance', week_of: 'Week · {name}',
    k_session: 'Session', k_day: 'Day', k_week: 'Week', k_month: 'Month', k_balance: 'Balance', k_model: 'Model', k_other: 'Other',
    left: 'left', used: 'used', remaining: 'remaining', free: 'free', waiting_first: 'Waiting for the first measure',
    aria_headroom: '{p} % available before the next limit', more_limits: '+{n} more limits',
    n_accounts: '{n} account(s)', use_now: 'Use now: {label} · {p} %',
    oauth_expired: 'The session of this account expired.', key_refused: 'The service refuses this key.', reconnect: 'Reconnect',
    blocked_until: '{what} blocked until {when}', limit_around: '{what}: limit around {when} at this pace',
    welcome_title: 'Track your AI subscriptions', welcome_text: 'Pick a service to connect your first account. Everything stays on your server.',
    table_view: 'Table view', th_account: 'Account', th_tool: 'Tool', th_plan: 'Plan', th_available: 'Available', th_limit: 'Limit', th_value: 'Value', th_reset: 'Reset', th_status: 'Status',
    footer_poll: 'collected every 3 min',
    language: 'Language', theme: 'Theme', theme_auto: 'Auto', theme_light: 'Light', theme_dark: 'Dark', docs: 'Documentation', version: 'Version',
    proj_free: 'Fully available', proj_blocked: 'Blocked until {when}', proj_hit: '⚠︎ Limit around {when} at this pace', proj_holds: '✓ Holds until the reset at this pace',
    proj_idle_session: 'Idle for the last 45 min', proj_idle: 'Stable over 24 h',
    amounts: '{used} used of {limit} · {left} left', reset_at: 'Resets {when} · in {dur}', no_session: 'No session in progress', no_reset: 'Prepaid, no reset',
    pace_tip: 'Time elapsed in the window: {p} %. A bar past this mark burns faster than a linear pace.',
    summary_headroom: '<b>{p} %</b> available before the next limit.', summary_balance: 'Prepaid balance.', is_best: '★ Best account of this tool right now',
    limits: 'Limits', extra_credits: 'Extra usage credits (month)', unlimited: 'unlimited', split: 'Weekly usage by surface', history: 'History',
    chart_empty: 'History fills up with every collection.',
    account: 'Account', status: 'Status', measured: 'Measured', next_poll: 'Next collection', customize: 'Customize', display_name: 'Display name', save: 'Save', saved: 'Saved', color: 'Color',
    m_refresh: 'Refresh', m_reconnect: 'Reconnect…', m_replace_key: 'Replace the key…', m_remove: 'Remove', removed: 'Account removed', close: 'Close', back: 'Back',
    confirm_remove: 'Stop tracking “{label}”? Its history will be deleted.',
    add_title: 'Add an account', search_service: 'Search a service…', no_match: 'No service matches.',
    cat_subscription: 'Subscriptions', cat_api: 'API keys', cat_custom: 'Anything else',
    auth_oauth: 'Sign in', auth_apiKey: 'API key', auth_custom: 'Configurable', experimental: 'experimental',
    reconnect_title: 'Reconnect “{label}”', name_hint: 'Shown on the dashboard, Homepage and the menu bar.', optional: 'optional',
    oauth_s1: 'Authorize VibeRooms on {name}', oauth_s1_text: 'A new tab opens: sign in with the account to track. For another account than the one in your browser, open the link in a private window.',
    oauth_open: 'Open {name}', oauth_s2_code: 'Paste the code', oauth_s2_code_text: 'After clicking “Authorize”, the page shows a code: copy it here. It is checked automatically.',
    oauth_s2_url: 'Paste the address of the page', oauth_s2_url_text: 'After authorizing, your browser lands on a “localhost” page that does not load: copy its full address from the address bar.',
    oauth_note: 'VibeRooms gets its own read-only session (profile and usage). Your CLI sessions are never touched.', connecting: 'Connecting…',
    where_key: 'Where do I find my key?', key_note: 'The key is checked with a read-only call, then stored on your server only. It is never sent back to the browser.',
    test_and_add: 'Check and add', testing: 'Checking…',
    c_url: 'Endpoint that returns the quota or balance', c_auth: 'Authentication header', c_auth_hint: '{{key}} is replaced by the key below. Leave the name empty if no authentication is needed.',
    c_key: 'Key or token', c_test: 'Call the endpoint', c_fields: 'Values found in the answer', c_fields_hint: 'Give a role to the values that matter: remaining, used, limit, percentage or reset date.',
    c_no_fields: 'No numeric value found in the answer.', c_label: 'Name of the limit', c_label_ph: 'e.g. Credits', c_kind: 'Type', c_unit: 'Unit',
    c_advanced: 'Advanced: edit the JSON description', c_advanced_hint: 'Several limits, JSON paths with * wildcards, {{month_start}} variables… see the README.',
    c_bad_json: 'The JSON description is not valid.', c_pick_one: 'Give a role to at least one value.',
    role_none: '—', role_remaining: 'Remaining', role_used: 'Used', role_limit: 'Limit', role_percent: '% used', role_percent_remaining: '% remaining', role_resets_at: 'Reset date',
    custom_name: 'Custom connector', done_added: '{label} is connected', done_reconnected: '{label} is reconnected', done_first_measure: 'The first measure will show up in a few seconds.', add_another: 'Add another account', done: 'Done',
  },
  fr: {
    subtitle: 'Quotas de tes abonnements et API IA, auto-hébergé', refresh_all: 'Rafraîchir tous les comptes', add_account: 'Ajouter un compte', settings: 'Réglages',
    updated: 'Mis à jour {ago}', unreachable: 'Serveur injoignable : {msg}', refresh_failed: 'Échec du rafraîchissement : {msg}',
    just_now: "à l'instant", ago_s: 'il y a {n} s', ago_min: 'il y a {n} min', ago_dur: 'il y a {d}', never: 'jamais', soon: 'imminente',
    min: 'min', h: 'h', d: 'j', tomorrow: 'demain',
    st_ok: 'À jour', st_degraded: 'Erreur, valeurs en cache', st_stale: 'Données anciennes', st_error: 'Erreur de collecte', st_reauth: 'Reconnexion requise', st_pending: 'Collecte…',
    session: 'Session', session_h: 'Session {h} h', week: 'Semaine', day: 'Jour', month: 'Mois', balance: 'Solde', week_of: 'Semaine · {name}',
    k_session: 'Session', k_day: 'Jour', k_week: 'Semaine', k_month: 'Mois', k_balance: 'Solde', k_model: 'Modèle', k_other: 'Autre',
    left: 'dispo', used: 'utilisé', remaining: 'restant', free: 'libre', waiting_first: 'En attente de la première mesure',
    aria_headroom: '{p} % disponible avant la prochaine limite', more_limits: '+{n} autres limites',
    n_accounts: '{n} compte(s)', use_now: 'À utiliser : {label} · {p} %',
    oauth_expired: 'La session de ce compte a expiré.', key_refused: 'Le service refuse cette clé.', reconnect: 'Reconnecter',
    blocked_until: "{what} bloqué jusqu'à {when}", limit_around: '{what} : limite vers {when} à ce rythme',
    welcome_title: 'Suis tes abonnements IA', welcome_text: 'Choisis un service pour connecter ton premier compte. Tout reste sur ton serveur.',
    table_view: 'Vue tableau', th_account: 'Compte', th_tool: 'Outil', th_plan: 'Offre', th_available: 'Disponible', th_limit: 'Limite', th_value: 'Valeur', th_reset: 'Reset', th_status: 'Statut',
    footer_poll: 'collecte toutes les 3 min',
    language: 'Langue', theme: 'Thème', theme_auto: 'Auto', theme_light: 'Clair', theme_dark: 'Sombre', docs: 'Documentation', version: 'Version',
    proj_free: 'Entièrement disponible', proj_blocked: "Bloqué jusqu'à {when}", proj_hit: '⚠︎ Limite vers {when} à ce rythme', proj_holds: "✓ Tient jusqu'au reset à ce rythme",
    proj_idle_session: 'Inactif ces 45 dernières min', proj_idle: 'Stable sur 24 h',
    amounts: '{used} utilisés sur {limit} · reste {left}', reset_at: 'Reset {when} · dans {dur}', no_session: 'Pas de session en cours', no_reset: 'Prépayé, pas de reset',
    pace_tip: 'Temps écoulé dans la fenêtre : {p} %. Une barre au-delà de ce repère consomme plus vite que le rythme linéaire.',
    summary_headroom: '<b>{p} %</b> disponible avant la prochaine limite.', summary_balance: 'Solde prépayé.', is_best: '★ Meilleur compte de cet outil en ce moment',
    limits: 'Limites', extra_credits: 'Crédits supplémentaires (mois)', unlimited: 'illimité', split: 'Semaine par usage', history: 'Historique',
    chart_empty: "L'historique se remplit à chaque collecte.",
    account: 'Compte', status: 'Statut', measured: 'Mesuré', next_poll: 'Prochaine collecte', customize: 'Personnaliser', display_name: 'Nom affiché', save: 'Enregistrer', saved: 'Enregistré', color: 'Couleur',
    m_refresh: 'Rafraîchir', m_reconnect: 'Reconnecter…', m_replace_key: 'Remplacer la clé…', m_remove: 'Retirer', removed: 'Compte retiré', close: 'Fermer', back: 'Retour',
    confirm_remove: "Retirer « {label} » du suivi ? L'historique de ce compte sera supprimé.",
    add_title: 'Ajouter un compte', search_service: 'Rechercher un service…', no_match: 'Aucun service ne correspond.',
    cat_subscription: 'Abonnements', cat_api: 'Clés API', cat_custom: 'Autre service',
    auth_oauth: 'Connexion', auth_apiKey: 'Clé API', auth_custom: 'Configurable', experimental: 'expérimental',
    reconnect_title: 'Reconnecter « {label} »', name_hint: 'Affiché dans le tableau de bord, Homepage et la barre de menus.', optional: 'facultatif',
    oauth_s1: 'Autorise VibeRooms sur {name}', oauth_s1_text: "Un nouvel onglet s'ouvre : connecte-toi avec le compte à suivre. Pour un autre compte que celui de ton navigateur, ouvre le lien dans une fenêtre privée.",
    oauth_open: 'Ouvrir {name}', oauth_s2_code: 'Colle le code', oauth_s2_code_text: 'Après « Authorize », la page affiche un code : colle-le ici. Il est vérifié automatiquement.',
    oauth_s2_url: "Colle l'adresse de la page", oauth_s2_url_text: "Après l'autorisation, ton navigateur arrive sur une page « localhost » qui ne se charge pas : copie son adresse complète depuis la barre d'adresse.",
    oauth_note: "VibeRooms obtient sa propre session en lecture seule (profil et usage). Tes sessions CLI ne sont jamais touchées.", connecting: 'Connexion…',
    where_key: 'Où trouver ma clé ?', key_note: "La clé est vérifiée par un appel en lecture seule, puis stockée uniquement sur ton serveur. Elle n'est jamais renvoyée au navigateur.",
    test_and_add: 'Vérifier et ajouter', testing: 'Vérification…',
    c_url: 'Adresse qui renvoie le quota ou le solde', c_auth: "En-tête d'authentification", c_auth_hint: "{{key}} est remplacé par la clé ci-dessous. Laisse le nom vide si aucune authentification n'est nécessaire.",
    c_key: 'Clé ou jeton', c_test: "Appeler l'adresse", c_fields: 'Valeurs trouvées dans la réponse', c_fields_hint: 'Donne un rôle aux valeurs utiles : restant, utilisé, plafond, pourcentage ou date de reset.',
    c_no_fields: 'Aucune valeur numérique dans la réponse.', c_label: 'Nom de la limite', c_label_ph: 'ex. Crédits', c_kind: 'Type', c_unit: 'Unité',
    c_advanced: 'Avancé : modifier la description JSON', c_advanced_hint: 'Plusieurs limites, chemins JSON avec jokers *, variables {{month_start}}… voir le README.',
    c_bad_json: "La description JSON n'est pas valide.", c_pick_one: 'Donne un rôle à au moins une valeur.',
    role_none: '—', role_remaining: 'Restant', role_used: 'Utilisé', role_limit: 'Plafond', role_percent: '% utilisé', role_percent_remaining: '% restant', role_resets_at: 'Date de reset',
    custom_name: 'Connecteur personnalisé', done_added: '{label} est connecté', done_reconnected: '{label} est reconnecté', done_first_measure: 'La première mesure arrive dans quelques secondes.', add_another: 'Ajouter un autre compte', done: 'Terminé',
  },
};

let LANG = 'en';
try { LANG = localStorage.getItem('vr-lang') || ''; } catch {}
if (!DICT[LANG]) LANG = (navigator.language || 'en').toLowerCase().startsWith('fr') ? 'fr' : 'en';

function t(key, vars = {}) {
  const s = (DICT[LANG] && DICT[LANG][key]) ?? DICT.en[key] ?? key;
  return s.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? vars[k] : m));
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
