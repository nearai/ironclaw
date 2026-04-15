// IronClaw Web Gateway - Client

// --- Theme Management (dark / light / system) ---
// Icon switching is handled by pure CSS via data-theme-mode on <html>.

function getSystemTheme() {
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

const VALID_THEME_MODES = { dark: true, light: true, system: true };

function getThemeMode() {
  const stored = localStorage.getItem('ironclaw-theme');
  return (stored && VALID_THEME_MODES[stored]) ? stored : 'system';
}

function resolveTheme(mode) {
  return mode === 'system' ? getSystemTheme() : mode;
}

function applyTheme(mode) {
  const resolved = resolveTheme(mode);
  document.documentElement.setAttribute('data-theme', resolved);
  document.documentElement.setAttribute('data-theme-mode', mode);
  const titleKeys = { dark: 'theme.tooltipDark', light: 'theme.tooltipLight', system: 'theme.tooltipSystem' };
  const btn = document.getElementById('theme-toggle');
  if (btn) btn.title = (typeof I18n !== 'undefined' && titleKeys[mode]) ? I18n.t(titleKeys[mode]) : ('Theme: ' + mode);
  const announce = document.getElementById('theme-announce');
  if (announce) announce.textContent = (typeof I18n !== 'undefined') ? I18n.t('theme.announce', { mode: mode }) : ('Theme: ' + mode);
}

function toggleTheme() {
  const cycle = { dark: 'light', light: 'system', system: 'dark' };
  const current = getThemeMode();
  const next = cycle[current] || 'dark';
  localStorage.setItem('ironclaw-theme', next);
  applyTheme(next);
}

// Apply theme immediately (FOUC prevention is done via inline script in <head>,
// but we call again here to ensure tooltip is set after DOM is ready).
applyTheme(getThemeMode());

// Delay enabling theme transition to avoid flash on initial load.
requestAnimationFrame(function() {
  requestAnimationFrame(function() {
    document.body.classList.add('theme-transition');
  });
});

// Listen for OS theme changes — only re-apply when in 'system' mode.
const mql = window.matchMedia('(prefers-color-scheme: light)');
const onSchemeChange = function() {
  if (getThemeMode() === 'system') {
    applyTheme('system');
  }
};
if (mql.addEventListener) {
  mql.addEventListener('change', onSchemeChange);
} else if (mql.addListener) {
  mql.addListener(onSchemeChange);
}

// Bind theme toggle buttons (CSP-compliant — no inline onclick).
document.getElementById('theme-toggle').addEventListener('click', toggleTheme);
document.getElementById('settings-theme-toggle')?.addEventListener('click', () => {
  toggleTheme();
  const btn = document.getElementById('settings-theme-toggle');
  if (btn) {
    const mode = localStorage.getItem('ironclaw-theme') || 'system';
    btn.textContent = I18n.t('theme.label', { mode: mode.charAt(0).toUpperCase() + mode.slice(1) });
  }
});

let token = '';
let oidcProxyAuth = false;
let eventSource = null;
let logEventSource = null;
let currentTab = 'chat';
let currentThreadId = null;
let currentThreadIsReadOnly = false;
let assistantThreadId = null;
let hasMore = false;
let oldestTimestamp = null;
let loadingOlder = false;
let sseHasConnectedBefore = false;
let jobEvents = new Map(); // job_id -> Array of events
let jobListRefreshTimer = null;
let pairingPollInterval = null;
let unreadThreads = new Map(); // thread_id -> unread count
let _loadThreadsTimer = null;
const JOB_EVENTS_CAP = 500;
const JOB_EVENTS_MAX_JOBS = 50;
const MAX_DOM_MESSAGES = 200;
const MEMORY_SEARCH_QUERY_MAX_LENGTH = 100;
let stagedImages = [];
let authFlowPending = false;
let _ghostSuggestion = '';
let currentSettingsSubtab = 'inference';
let generatedImagesByThread = new Map();
const GENERATED_IMAGE_THREAD_CACHE_CAP = 20;
const GENERATED_IMAGES_PER_THREAD_CAP = 8;

// --- Hash-based URL Navigation ---
//
// Encodes navigation state in window.location.hash so refreshing
// the page restores the current tab, thread, memory file, job detail, etc.
//
// Hash format: #/{tab}[/{detail}[/{subtab}]]
//   #/chat                     → chat tab, assistant thread
//   #/chat/{threadId}          → chat tab, specific thread
//   #/memory                   → memory tab, tree root
//   #/memory/{path/to/file}    → memory tab, specific file
//   #/jobs                     → jobs list
//   #/jobs/{jobId}             → job detail
//   #/routines                 → routines list
//   #/routines/{id}            → routine detail
//   #/settings/{subtab}        → settings tab with specific sub-tab
//   #/logs                     → logs tab

/** Suppress hash-change handling while we're programmatically updating. */
let _suppressHashChange = false;

/** Update the URL hash to reflect current navigation state. */
function updateHash() {
  if (_suppressHashChange) return;
  var parts = [currentTab];

  switch (currentTab) {
    case 'chat':
      if (currentThreadId && currentThreadId !== assistantThreadId) {
        parts.push(currentThreadId);
      }
      break;
    case 'memory':
      if (typeof currentMemoryPath === 'string' && currentMemoryPath) {
        parts.push(currentMemoryPath);
      }
      break;
    case 'jobs':
      if (typeof currentJobId !== 'undefined' && currentJobId) {
        parts.push(currentJobId);
      }
      break;
    case 'routines':
      if (typeof currentRoutineId !== 'undefined' && currentRoutineId) {
        parts.push(currentRoutineId);
      }
      break;
    case 'settings':
      if (currentSettingsSubtab && currentSettingsSubtab !== 'inference') {
        parts.push(currentSettingsSubtab);
      }
      break;
  }

  var hash = '#/' + parts.join('/');
  if (window.location.hash !== hash) {
    window.history.replaceState(null, '', hash);
  }
}

/** Parse the current URL hash into navigation state. */
function parseHash() {
  var hash = window.location.hash || '';
  if (!hash.startsWith('#/')) return null;
  var parts = hash.substring(2).split('/');
  return {
    tab: parts[0] || 'chat',
    detail: parts.slice(1).join('/') || null,
  };
}

/**
 * Restore navigation state from the URL hash.
 * Called once after authentication and on hashchange events.
 */
function restoreFromHash() {
  var state = parseHash();
  if (!state) return;

  // Suppress hash updates while restoring — switchTab/readMemoryFile/etc.
  // each call updateHash(), which would overwrite the full hash before
  // the detail part is restored.
  _suppressHashChange = true;

  // Switch tab
  if (state.tab && state.tab !== currentTab) {
    switchTab(state.tab);
  }

  // Restore detail state within the tab
  if (state.detail) {
    switch (state.tab) {
      case 'chat':
        // Defer thread switch until threads are loaded
        window._pendingThreadRestore = state.detail;
        break;
      case 'memory':
        readMemoryFile(state.detail);
        break;
      case 'jobs':
        openJobDetail(state.detail);
        break;
      case 'routines':
        openRoutineDetail(state.detail);
        break;
      case 'settings':
        switchSettingsSubtab(state.detail);
        break;
    }
  }

  _suppressHashChange = false;
}

window.addEventListener('hashchange', function() {
  if (_suppressHashChange) return;
  restoreFromHash();
});

// --- Streaming Debounce State ---
let _streamBuffer = '';
let _streamDebounceTimer = null;
const STREAM_DEBOUNCE_MS = 50;

// --- Connection Status Banner State ---
let _connectionLostTimer = null;
let _connectionLostAt = null;
let _reconnectAttempts = 0;
let _lastSseEventId = null;

// --- Turn Response Tracking State ---
// Safety net for lost SSE response events (see #2079): tracks whether we
// received a `response` event for the current turn so that a "Done" status
// arriving without one can trigger a history reload.
const DONE_WITHOUT_RESPONSE_TIMEOUT_MS = 1500;
// Single-thread tracking is intentional: background thread events are already
// filtered out by `isCurrentThread`, so only the active thread's turn state
// matters here. Per-thread state is unnecessary.
let _turnResponseReceived = false;
let _doneWithoutResponseTimer = null;

// Clean up connection-level timers and buffers.
// Called before creating a new connection, on tab hide, and on page unload
// to prevent leaked intervals/timeouts from accumulating across reconnects.
// Note: _doneWithoutResponseTimer is intentionally NOT cleared here — it is a
// turn-level concern managed by the onopen and response handlers (#2079).
function cleanupConnectionState() {
  if (_streamDebounceTimer) { clearInterval(_streamDebounceTimer); _streamDebounceTimer = null; }
  _streamBuffer = '';
  if (_connectionLostTimer) { clearTimeout(_connectionLostTimer); _connectionLostTimer = null; }
  if (jobListRefreshTimer) { clearTimeout(jobListRefreshTimer); jobListRefreshTimer = null; }
  if (_loadThreadsTimer) { clearTimeout(_loadThreadsTimer); _loadThreadsTimer = null; }
  if (gatewayStatusInterval) { clearInterval(gatewayStatusInterval); gatewayStatusInterval = null; }
}

// --- Send Cooldown State ---
let _sendCooldown = false;
let _recentLocalPairingApprovals = new Map();

// --- Slash Commands ---

const SLASH_COMMANDS = [
  { cmd: '/status',     desc: 'Show all jobs, or /status <id> for one job' },
  { cmd: '/list',       desc: 'List all jobs' },
  { cmd: '/cancel',     desc: '/cancel <job-id> — cancel a running job' },
  { cmd: '/undo',       desc: 'Revert the last turn' },
  { cmd: '/redo',       desc: 'Re-apply an undone turn' },
  { cmd: '/compact',    desc: 'Compress the context window' },
  { cmd: '/clear',      desc: 'Clear thread and start fresh' },
  { cmd: '/interrupt',  desc: 'Stop the current turn' },
  { cmd: '/heartbeat',  desc: 'Trigger manual heartbeat check' },
  { cmd: '/summarize',  desc: 'Summarize the current thread' },
  { cmd: '/suggest',    desc: 'Suggest next steps' },
  { cmd: '/help',       desc: 'Show help' },
  { cmd: '/version',    desc: 'Show version info' },
  { cmd: '/tools',      desc: 'List available tools' },
  { cmd: '/skills',     desc: 'List installed skills' },
  { cmd: '/model',      desc: 'Show or switch the LLM model' },
  { cmd: '/thread new', desc: 'Create a new conversation thread' },
];

let _slashSelected = -1;
let _slashMatches = [];

// --- Tool Activity State ---
let _activeGroup = null;
let _activeToolCards = {};
let _activityThinking = null;

// --- Auth ---

// Common post-auth initialization shared by token auth and OIDC auto-auth.
function initApp() {
  var authScreen = document.getElementById('auth-screen');
  var app = document.getElementById('app');
  // Cross-fade: fade out auth screen, then show app
  if (authScreen) authScreen.style.opacity = '0';
  // Show app container (invisible — opacity:0 in CSS) so layout computes
  app.style.display = 'flex';
  // Position tab indicator instantly (no transition) before fade-in
  var indicator = document.getElementById('tab-indicator');
  if (indicator) indicator.style.transition = 'none';
  updateTabIndicator();
  // Force layout so the instant position is applied, then restore transition
  if (indicator) {
    void indicator.offsetLeft;
    indicator.style.transition = '';
  }
  // Now fade in
  app.classList.add('visible');
  // Hide auth screen after fade-out transition completes
  setTimeout(function() { if (authScreen) authScreen.style.display = 'none'; }, 300);
  // Strip token and log_level from URL so they're not visible in the address bar
  var cleaned = new URL(window.location);
  var urlLogLevel = cleaned.searchParams.get('log_level');
  cleaned.searchParams.delete('token');
  cleaned.searchParams.delete('log_level');
  window.history.replaceState({}, '', cleaned.pathname + cleaned.search + cleaned.hash);
  connectSSE();
  connectLogSSE();
  startGatewayStatusPolling();
  // Fetch user profile and render avatar + account menu.
  apiFetch('/api/profile').then(function(profile) {
    if (!profile) return;
    window._currentUser = profile;
    // Hide admin tabs for non-admin users.
    if (profile.role !== 'admin') {
      var usersTab = document.querySelector('[data-settings-subtab="users"]');
      if (usersTab) usersTab.style.display = 'none';
    }
    // Render avatar.
    var avatarImg = document.getElementById('user-avatar-img');
    var avatarInitials = document.getElementById('user-avatar-initials');
    var displayName = profile.display_name || profile.email || profile.id || '?';
    if (avatarInitials) {
      avatarInitials.textContent = displayName.charAt(0).toUpperCase();
    }
    if (profile.avatar_url && avatarImg) {
      avatarImg.referrerPolicy = 'no-referrer';
      avatarImg.onload = function() {
        if (avatarInitials) avatarInitials.style.display = 'none';
      };
      avatarImg.src = profile.avatar_url;
      avatarImg.removeAttribute('hidden');
    }
    // Populate dropdown.
    var nameEl = document.getElementById('user-dropdown-name');
    var emailEl = document.getElementById('user-dropdown-email');
    var roleEl = document.getElementById('user-dropdown-role');
    if (nameEl) nameEl.textContent = profile.display_name || profile.id;
    if (emailEl) emailEl.textContent = profile.email || '';
    if (roleEl) roleEl.textContent = profile.role;
  }).catch(function() {});
  checkTeeStatus();
  loadThreads();
  loadMemoryTree();
  loadJobs();
  // Restore navigation state from URL hash (tab, thread, memory file, etc.)
  restoreFromHash();
  // Apply URL log_level param if present, otherwise just sync the dropdown
  if (urlLogLevel) {
    setServerLogLevel(urlLogLevel);
  } else {
    loadServerLogLevel();
  }
}

function authenticate() {
  token = document.getElementById('token-input').value.trim();
  if (!token) {
    document.getElementById('auth-error').textContent = I18n.t('auth.errorRequired');
    return;
  }

  // Loading state for Connect button
  const connectBtn = document.getElementById('auth-connect-btn');
  if (connectBtn) {
    connectBtn.disabled = true;
    connectBtn.textContent = I18n.t('auth.connecting');
  }

  // Test the token against the health-ish endpoint (chat/threads requires auth)
  apiFetch('/api/chat/threads')
    .then(() => {
      sessionStorage.setItem('ironclaw_token', token);
      initApp();
    })
    .catch(() => {
      sessionStorage.removeItem('ironclaw_token');
      document.getElementById('auth-screen').style.display = '';
      document.getElementById('auth-screen').style.opacity = '';
      document.getElementById('app').style.display = 'none';
      document.getElementById('auth-error').textContent = I18n.t('auth.errorInvalid');
      // Reset Connect button on error
      if (connectBtn) {
        connectBtn.disabled = false;
        connectBtn.textContent = I18n.t('auth.connect');
      }
    });
}

document.getElementById('token-input').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') authenticate();
});

// Close SSE connections on page unload to free the browser's connection pool.
// Without this, stale SSE connections from prior page loads linger and exhaust
// the HTTP/1.1 per-origin connection limit (6), blocking API fetch calls.
window.addEventListener('beforeunload', () => {
  cleanupConnectionState();
  if (eventSource) { eventSource.close(); eventSource = null; }
  if (logEventSource) { logEventSource.close(); logEventSource = null; }
});

// Pause SSE when the browser tab is hidden (another tab is focused) and resume
// when it becomes visible again. This frees connection slots for other tabs
// running the gateway — without this, each tab holds 1-2 SSE connections and
// the 3rd tab exhausts the browser's per-origin limit.
document.addEventListener('visibilitychange', () => {
  if (document.hidden) {
    cleanupConnectionState();
    if (eventSource) { eventSource.close(); eventSource = null; }
    if (logEventSource) { logEventSource.close(); logEventSource = null; }
  } else if (token) {
    connectSSE();
    startGatewayStatusPolling();
    if (currentTab === 'logs') connectLogSSE();
  }
});

// --- Social login (OAuth + NEAR wallet) ---

// Show the token form (used as fallback when no OAuth providers are available).
function showTokenForm() {
  var tokenForm = document.getElementById('auth-token-form');
  if (tokenForm) {
    tokenForm.style.display = '';
    var input = document.getElementById('token-input');
    if (input) input.focus();
  }
}

// Discover enabled providers and show corresponding buttons.
fetch('/auth/providers', { credentials: 'include' })
  .then(function(r) { return r.ok ? r.json() : { providers: [] }; })
  .then(function(data) {
    var providers = data.providers || [];
    if (providers.length === 0) { showTokenForm(); return; }
    // Store NEAR network for the wallet connector.
    if (data.near_network) window._nearNetwork = data.near_network;
    var social = document.getElementById('auth-social');
    if (social) social.style.display = '';
    providers.forEach(function(p) {
      var btn = document.getElementById('auth-' + p + '-btn');
      if (!btn) return;
      btn.style.display = '';
      if (p === 'near') {
        btn.addEventListener('click', authenticateWithNear);
      } else {
        btn.addEventListener('click', function() { window.location = '/auth/login/' + p; });
      }
    });
    // When social providers are available, collapse the token form
    // and show the "or use a token" divider instead.
    var tokenForm = document.getElementById('auth-token-form');
    var tokenDivider = document.getElementById('auth-token-divider');
    if (tokenForm && tokenDivider) {
      tokenForm.style.display = 'none';
      tokenDivider.style.display = '';
      tokenDivider.style.cursor = 'pointer';
      tokenDivider.addEventListener('click', function() {
        tokenForm.style.display = '';
        tokenDivider.style.display = 'none';
        var input = document.getElementById('token-input');
        if (input) input.focus();
      });
    }
  })
  .catch(function() { showTokenForm(); });

// NEAR wallet authentication via near-connect.
async function authenticateWithNear() {
  var nearBtn = document.getElementById('auth-near-btn');
  var errEl = document.getElementById('auth-error');
  if (nearBtn) { nearBtn.disabled = true; nearBtn.textContent = I18n.t('auth.connectingWallet'); }
  if (errEl) errEl.textContent = '';

  try {
    // 1. Get challenge nonce from the server.
    var challengeResp = await fetch('/auth/near/challenge', { credentials: 'include' });
    if (!challengeResp.ok) throw new Error('Failed to get challenge');
    var challenge = await challengeResp.json();

    // 2. Load near-connect dynamically if not already loaded.
    if (!window._nearConnector) {
      var mod = await import('https://esm.sh/@hot-labs/near-connect@0.11');
      var network = window._nearNetwork || 'mainnet';
      window._nearConnector = new mod.NearConnector({ network: network });
    }
    var connector = window._nearConnector;

    // 3. Connect wallet and request signature.
    if (nearBtn) nearBtn.textContent = I18n.t('auth.signWithWallet');
    var wallet = await connector.connect();
    var accounts = await wallet.getAccounts();
    if (!accounts || accounts.length === 0) throw new Error('No NEAR account found');

    var accountId = accounts[0].accountId;

    // Convert hex nonce to Uint8Array for signMessage.
    var nonceBytes = new Uint8Array(challenge.nonce.match(/.{2}/g).map(function(b) { return parseInt(b, 16); }));

    var signed = await wallet.signMessage({
      message: challenge.message,
      recipient: challenge.recipient || 'ironclaw',
      nonce: nonceBytes,
    });

    // 4. Send signature to server for verification.
    if (nearBtn) nearBtn.textContent = I18n.t('auth.verifying');
    var verifyResp = await fetch('/auth/near/verify', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      credentials: 'include',
      body: JSON.stringify({
        account_id: accountId,
        public_key: signed.publicKey,
        signature: signed.signature,
        nonce: challenge.nonce,
      }),
    });

    if (!verifyResp.ok) {
      var errText = await verifyResp.text();
      throw new Error(errText || 'Verification failed');
    }

    await verifyResp.json();

    // 5. Rely on the HttpOnly session cookie created by the backend.
    token = '';
    sessionStorage.removeItem('ironclaw_token');
    initApp();
  } catch (err) {
    if (errEl) errEl.textContent = err.message || 'NEAR wallet login failed';
    if (nearBtn) { nearBtn.disabled = false; nearBtn.textContent = I18n.t('auth.social.near'); }
  }
}

// Note: main event listener registration is at the bottom of this file (search
// "Event Listener Registration"). Do NOT add duplicate listeners here.

// Auto-authenticate from URL param, saved session, or OIDC proxy header.
//
// When behind a reverse proxy that injects auth (e.g., AWS ALB with OIDC),
// the proxy already authenticates every request. We probe /api/gateway/status
// without a token — if the proxy's header lets us through, skip the login
// screen entirely.
(function autoAuth() {
  const params = new URLSearchParams(window.location.search);
  const urlToken = params.get('token');
  if (urlToken) {
    document.getElementById('token-input').value = urlToken;
    authenticate();
    return;
  }
  // Restore OIDC proxy mode from session.
  if (sessionStorage.getItem('ironclaw_oidc') === '1') {
    oidcProxyAuth = true;
  }
  const saved = sessionStorage.getItem('ironclaw_token');
  if (saved) {
    document.getElementById('token-input').value = saved;
    document.getElementById('auth-screen').style.display = 'none';
    document.getElementById('app').style.display = 'flex';
    authenticate();
    return;
  }
  // Probe for proxy-injected OIDC auth (no token needed from the client).
  fetch('/api/gateway/status', { credentials: 'include' }).then(function(r) {
    if (r.ok) {
      oidcProxyAuth = true;
      sessionStorage.setItem('ironclaw_oidc', '1');
      document.getElementById('auth-screen').style.display = 'none';
      document.getElementById('app').style.display = 'flex';
      initApp();
    }
  }).catch(function() { /* proxy auth not available, show login */ });
})();

// --- API helper ---

function apiFetch(path, options) {
  const opts = options || {};
  opts.headers = opts.headers || {};
  // In OIDC mode the reverse proxy provides auth; skip the Authorization header.
  if (token && !oidcProxyAuth) {
    opts.headers['Authorization'] = 'Bearer ' + token;
  }
  if (opts.body && typeof opts.body === 'object') {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(opts.body);
  }
  return fetch(path, opts).then((res) => {
    if (!res.ok) {
      return res.text().then(function(body) {
        const err = new Error(body || (res.status + ' ' + res.statusText));
        err.status = res.status;
        throw err;
      });
    }
    if (res.status === 204) return null;
    return res.json();
  });
}

// --- Restart Feature ---

let isRestarting = false; // Track if we're currently restarting
let restartEnabled = false; // Track if restart is available in this deployment

function triggerRestart() {
  if (!currentThreadId) {
    alert(I18n.t('error.startConversation'));
    return;
  }

  // Show the confirmation modal
  const confirmModal = document.getElementById('restart-confirm-modal');
  confirmModal.style.display = 'flex';
}

function confirmRestart() {
  if (!currentThreadId) {
    alert(I18n.t('error.startConversation'));
    return;
  }

  // Hide confirmation modal
  const confirmModal = document.getElementById('restart-confirm-modal');
  confirmModal.style.display = 'none';

  const restartBtn = document.getElementById('restart-btn');
  const restartIcon = document.getElementById('restart-icon');

  // Mark as restarting
  isRestarting = true;
  restartBtn.disabled = true;
  if (restartIcon) restartIcon.classList.add('spinning');

  // Show progress modal
  const loaderEl = document.getElementById('restart-loader');
  loaderEl.style.display = 'flex';

  // Send restart command via chat
  console.log('[confirmRestart] Sending /restart command to server');
  apiFetch('/api/chat/send', {
    method: 'POST',
    body: {
      content: '/restart',
      thread_id: currentThreadId,
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    },
  })
    .then((response) => {
      console.log('[confirmRestart] API call succeeded, response:', response);
    })
    .catch((err) => {
      console.error('[confirmRestart] Restart request failed:', err);
      addMessage('system', I18n.t('error.restartFailed', { message: err.message }));
      isRestarting = false;
      restartBtn.disabled = false;
      if (restartIcon) restartIcon.classList.remove('spinning');
      loaderEl.style.display = 'none';
    });
}

function cancelRestart() {
  const confirmModal = document.getElementById('restart-confirm-modal');
  confirmModal.style.display = 'none';
}

function tryShowRestartModal() {
  // Defensive callback for when restart is detected in messages.
  if (!isRestarting) {
    isRestarting = true;
    const restartBtn = document.getElementById('restart-btn');
    const restartIcon = document.getElementById('restart-icon');
    restartBtn.disabled = true;
    if (restartIcon) restartIcon.classList.add('spinning');

    // Show progress modal
    const loaderEl = document.getElementById('restart-loader');
    loaderEl.style.display = 'flex';
  }
}

function updateRestartButtonVisibility() {
  const restartBtn = document.getElementById('restart-btn');
  if (restartBtn) {
    restartBtn.style.display = restartEnabled ? 'block' : 'none';
  }
}

// --- SSE ---

function rememberSseEventId(event) {
  if (!event || !event.lastEventId) return;
  _lastSseEventId = event.lastEventId;
  window.__e2e = window.__e2e || {};
  window.__e2e.lastSseEventId = event.lastEventId;
}

function connectSSE(lastEventIdOverride) {
  if (eventSource) eventSource.close();
  cleanupConnectionState();

  // In OIDC mode the reverse proxy provides auth; no query token needed.
  let chatSseUrl = (token && !oidcProxyAuth)
    ? '/api/chat/events?token=' + encodeURIComponent(token)
    : '/api/chat/events';
  const lastEventId = lastEventIdOverride || _lastSseEventId;
  if (lastEventId) {
    chatSseUrl += (chatSseUrl.includes('?') ? '&' : '?')
      + 'last_event_id=' + encodeURIComponent(lastEventId);
  }
  eventSource = new EventSource(chatSseUrl);

  const addTrackedEventListener = (eventType, handler) => {
    eventSource.addEventListener(eventType, (event) => {
      rememberSseEventId(event);
      handler(event);
    });
  };

  eventSource.onopen = () => {
    document.getElementById('sse-dot').classList.remove('disconnected');
    var statusEl = document.getElementById('sse-status');
    if (statusEl) statusEl.textContent = I18n.t('status.connected');
    _reconnectAttempts = 0;
    // Clear stale turn-tracking state from before the disconnect
    _turnResponseReceived = false;
    if (_doneWithoutResponseTimer) {
      clearTimeout(_doneWithoutResponseTimer);
      _doneWithoutResponseTimer = null;
    }

    // Dismiss connection-lost banner and show reconnected flash
    if (_connectionLostTimer) {
      clearTimeout(_connectionLostTimer);
      _connectionLostTimer = null;
    }
    const lostBanner = document.getElementById('connection-banner');
    if (lostBanner) {
      const wasDisconnectedLong = _connectionLostAt && (Date.now() - _connectionLostAt > 10000);
      lostBanner.textContent = I18n.t('connection.reconnected');
      lostBanner.className = 'connection-banner connection-banner-success';
      setTimeout(() => { lostBanner.remove(); }, 2000);
      _connectionLostAt = null;
      // If disconnected >10s, reload chat history to catch missed messages
      if (wasDisconnectedLong && currentThreadId) {
        loadHistory();
      }
    }

    // If we were restarting, close the modal and reset button now that server is back
    if (isRestarting) {
      const loaderEl = document.getElementById('restart-loader');
      if (loaderEl) loaderEl.style.display = 'none';
      const restartBtn = document.getElementById('restart-btn');
      const restartIcon = document.getElementById('restart-icon');
      if (restartBtn) restartBtn.disabled = false;
      if (restartIcon) restartIcon.classList.remove('spinning');
      isRestarting = false;
    }

    if (sseHasConnectedBefore && currentThreadId) {
      finalizeActivityGroup();
      loadHistory();
    }
    sseHasConnectedBefore = true;
  };

  eventSource.onerror = () => {
    _reconnectAttempts++;
    document.getElementById('sse-dot').classList.add('disconnected');
    var statusEl2 = document.getElementById('sse-status');
    if (statusEl2) statusEl2.textContent = I18n.t('status.reconnecting');

    // Update existing banner with attempt count
    const existingBanner = document.getElementById('connection-banner');
    if (existingBanner && existingBanner.classList.contains('connection-banner-warning')) {
      existingBanner.textContent = I18n.t('connection.reconnecting', { count: _reconnectAttempts });
    }

    // Start connection-lost banner timer (3s delay)
    if (!_connectionLostTimer && !existingBanner) {
      _connectionLostAt = _connectionLostAt || Date.now();
      _connectionLostTimer = setTimeout(() => {
        _connectionLostTimer = null;
        // Only show if still disconnected
        const dot = document.getElementById('sse-dot');
        if (dot?.classList.contains('disconnected')) {
          showConnectionBanner(I18n.t('connection.reconnecting', { count: _reconnectAttempts }), 'warning');
        }
      }, 3000);
    }
  };

  // Forward all SSE events to registered widget handlers.
  // Wraps addEventListener to intercept every named event and dispatch
  // to widget subscribers before the built-in handler runs.
  // Must run before any addTrackedEventListener calls so the wrapper is in place.
  //
  // NOTE: Only NAMED events (those dispatched via `addEventListener('foo', …)`
  // by the gateway, see `SseEvent` in `src/channels/web/types.rs`) are
  // forwarded. The generic `eventSource.onmessage` handler is intentionally
  // NOT wrapped because the IronClaw gateway never emits SSE frames without
  // an `event:` field — every frame carries a typed name (`response`,
  // `tool_started`, `gate_required`, etc.). Widget authors should subscribe
  // to those typed events via `IronClaw.api.on('<event_type>', handler)`
  // rather than relying on the generic message channel; if a widget needs
  // an untyped stream it must open its own `EventSource`.
  var _origAddEventListener = eventSource.addEventListener.bind(eventSource);
  eventSource.addEventListener = function(type, listener, opts) {
    _origAddEventListener(type, function(e) {
      // Dispatch to widget handlers
      if (IronClaw.api && e.data) {
        try {
          var parsed = JSON.parse(e.data);
          IronClaw.api._dispatch(type, parsed);
        } catch (parseErr) {
          console.warn('[IronClaw] SSE parse error for event', type, parseErr);
        }
      }
      // Call original handler
      listener(e);
    }, opts);
  };

  addTrackedEventListener('response', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) {
      if (data.thread_id) {
        unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
        debouncedLoadThreads();
      }
      return;
    }
    // Flush any remaining streaming buffer
    if (_streamDebounceTimer) {
      clearInterval(_streamDebounceTimer);
      _streamDebounceTimer = null;
    }
    if (_streamBuffer) {
      appendToLastAssistant(_streamBuffer);
      _streamBuffer = '';
    }
    // Remove streaming attribute from active assistant message
    const streamingMsg = document.querySelector('.message.assistant[data-streaming="true"]');
    if (streamingMsg) streamingMsg.removeAttribute('data-streaming');

    _turnResponseReceived = true;
    if (_doneWithoutResponseTimer) {
      clearTimeout(_doneWithoutResponseTimer);
      _doneWithoutResponseTimer = null;
    }
    finalizeActivityGroup();
    addMessage('assistant', data.content);
    pruneOldMessages();
    enableChatInput();
    // Refresh thread list so new titles appear after first message
    loadThreads();

    // Show restart modal if the response indicates restart was initiated
    if (data.content && data.content.toLowerCase().includes('restart initiated')) {
      setTimeout(() => tryShowRestartModal(), 500);
    }
  });

  addTrackedEventListener('thinking', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) {
      if (data.thread_id) debouncedLoadThreads();
      return;
    }
    clearSuggestionChips();
    showActivityThinking(data.message);
  });

  addTrackedEventListener('suggestions', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    if (data.suggestions && data.suggestions.length > 0) {
      showSuggestionChips(data.suggestions);
    }
  });

  addTrackedEventListener('tool_started', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    addToolCard(data.name);
  });

  addTrackedEventListener('tool_completed', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    completeToolCard(data.name, data.success, data.error, data.parameters);

    // Show restart modal only when the restart tool succeeds
    if (data.name.toLowerCase() === 'restart' && data.success) {
      setTimeout(() => tryShowRestartModal(), 500);
    }
  });

  addTrackedEventListener('tool_result', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    setToolCardOutput(data.name, data.preview);
  });

  addTrackedEventListener('stream_chunk', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    finalizeActivityGroup();

    // Mark the active assistant message as streaming
    const container = document.getElementById('chat-messages');
    let lastAssistant = container.querySelector('.message.assistant:last-of-type');
    if (!lastAssistant) {
      addMessage('assistant', '');
      lastAssistant = container.querySelector('.message.assistant:last-of-type');
    }
    if (lastAssistant) lastAssistant.setAttribute('data-streaming', 'true');

    // Mark turn as having received content so the Done safety net
    // does not trigger a spurious loadHistory() for streaming responses.
    _turnResponseReceived = true;

    // Accumulate chunks and debounce rendering at 50ms intervals
    _streamBuffer += data.content;
    // Force flush when buffer exceeds 10K chars to prevent memory buildup
    if (_streamBuffer.length > 10000) {
      appendToLastAssistant(_streamBuffer);
      _streamBuffer = '';
    }
    if (!_streamDebounceTimer) {
      _streamDebounceTimer = setInterval(() => {
        if (_streamBuffer) {
          appendToLastAssistant(_streamBuffer);
          _streamBuffer = '';
        }
      }, STREAM_DEBOUNCE_MS);
    }
  });

  addTrackedEventListener('status', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) {
      if (data.thread_id) debouncedLoadThreads();
      return;
    }
    // "Done" and "Awaiting approval" are terminal signals from the agent:
    // the agentic loop finished, so re-enable input as a safety net in case
    // the response SSE event is empty or lost.
    // Status text is not displayed — inline activity cards handle visual feedback.
    if (data.message === 'Done' || data.message === 'Awaiting approval') {
      finalizeActivityGroup();
      enableChatInput();
      // Safety net (#2079): if "Done" arrives but we never received a
      // `response` event for this turn, the message may have been lost
      // (broadcast lag, proxy buffering, brief SSE disconnect). Reload
      // history after a short delay so the user sees the answer.
      if (!_turnResponseReceived && data.message === 'Done') {
        if (!_doneWithoutResponseTimer) {
          _doneWithoutResponseTimer = setTimeout(() => {
            _doneWithoutResponseTimer = null;
            if (currentThreadId) loadHistory();
          }, DONE_WITHOUT_RESPONSE_TIMEOUT_MS);
        }
      }
      _turnResponseReceived = false;
    }
  });

  addTrackedEventListener('job_started', (e) => {
    const data = JSON.parse(e.data);
    showJobCard(data);
  });

  addTrackedEventListener('approval_needed', (e) => {
    const data = JSON.parse(e.data);
    const hasThread = !!data.thread_id;
    const forCurrentThread = !hasThread || isCurrentThread(data.thread_id);

    if (forCurrentThread) {
      showApproval(data);
    } else {
      // Keep thread list fresh when approval is requested in a background thread.
      unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
      debouncedLoadThreads();
    }

    // Extension setup flows can surface approvals from any settings subtab.
    if (currentTab === 'settings') refreshCurrentSettingsTab();
  });

  addTrackedEventListener('auth_required', (e) => {
    handleAuthRequired(JSON.parse(e.data));
  });

  addTrackedEventListener('auth_completed', (e) => {
    const data = JSON.parse(e.data);
    handleAuthCompleted(data);
  });

  addTrackedEventListener('pairing_required', (e) => {
    const data = JSON.parse(e.data);
    handlePairingRequired(data);
  });

  addTrackedEventListener('pairing_completed', (e) => {
    const data = JSON.parse(e.data);
    handlePairingCompleted(data);
  });

  addTrackedEventListener('gate_required', (e) => {
    const data = JSON.parse(e.data);
    handleGateRequired(data);
  });

  addTrackedEventListener('gate_resolved', (e) => {
    const data = JSON.parse(e.data);
    handleGateResolved(data);
  });

  addTrackedEventListener('extension_status', (e) => {
    if (currentTab === 'settings') refreshCurrentSettingsTab();
  });

  addTrackedEventListener('image_generated', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    rememberGeneratedImage(data.thread_id, data.event_id, data.data_url, data.path);
    addGeneratedImage(data.data_url, data.path, data.event_id);
  });

  addTrackedEventListener('error', (e) => {
    if (e.data) {
      const data = JSON.parse(e.data);
      if (!isCurrentThread(data.thread_id)) return;
      finalizeActivityGroup();
      addMessage('system', 'Error: ' + data.message);
      enableChatInput();
    }
  });

  // Job event listeners (activity stream for all sandbox jobs)
  const jobEventTypes = [
    'job_message', 'job_tool_use', 'job_tool_result',
    'job_status', 'job_result'
  ];
  for (const evtType of jobEventTypes) {
    addTrackedEventListener(evtType, (e) => {
      const data = JSON.parse(e.data);
      const jobId = data.job_id;
      if (!jobId) return;
      // Move jobId to end of Map insertion order (LRU: most-recent last).
      // delete+set keeps the Map ordered by last-access time so that
      // keys().next() always yields the least-recently-used entry in O(1).
      const existing = jobEvents.get(jobId);
      if (existing) jobEvents.delete(jobId);
      const events = existing || [];
      jobEvents.set(jobId, events);
      events.push({ type: evtType, data: data, ts: Date.now() });
      // Cap per-job events to prevent memory leak
      while (events.length > JOB_EVENTS_CAP) events.shift();
      // Cap total tracked jobs — evict the least-recently-used entry (O(1)).
      // Skip currentJobId so the user's actively-viewed job detail panel
      // doesn't go empty when many other jobs fire events.
      if (jobEvents.size > JOB_EVENTS_MAX_JOBS) {
        let evicted = false;
        for (const k of jobEvents.keys()) {
          if (k !== currentJobId) {
            jobEvents.delete(k);
            evicted = true;
            break;
          }
        }
        // Fallback: if every entry is currentJobId (impossible in practice),
        // evict the first key to maintain the cap.
        if (!evicted) {
          jobEvents.delete(jobEvents.keys().next().value);
        }
      }
      // If the Activity tab is currently visible for this job, refresh it
      refreshActivityTab(jobId);
      // Auto-refresh job list when on jobs tab (debounced)
      if ((evtType === 'job_result' || evtType === 'job_status') && currentTab === 'jobs' && !currentJobId) {
        clearTimeout(jobListRefreshTimer);
        jobListRefreshTimer = setTimeout(loadJobs, 200);
      }
      // Clean up finished job events after a viewing window
      if (evtType === 'job_result') {
        setTimeout(() => jobEvents.delete(jobId), 60000);
      }
    });
  }

  // Plan progress checklist
  addTrackedEventListener('plan_update', (e) => {
    const data = JSON.parse(e.data);
    if (data.thread_id && !isCurrentThread(data.thread_id)) return;
    renderPlanChecklist(data);
  });
}

// Check if an SSE event belongs to the currently viewed thread.
// Events without a thread_id are dropped (prevents notification leaking).
function isCurrentThread(threadId) {
  if (!threadId) return false;
  if (!currentThreadId) return true;
  return threadId === currentThreadId;
}

// --- Suggestion Chips ---

function showSuggestionChips(suggestions) {
  // Clear previous chips/ghost without restoring placeholder (we'll set it below)
  _ghostSuggestion = '';
  const container = document.getElementById('suggestion-chips');
  container.innerHTML = '';
  const ghost = document.getElementById('ghost-text');
  ghost.style.display = 'none';
  const wrapper = document.querySelector('.chat-input-wrapper');
  if (wrapper) wrapper.classList.remove('has-ghost');

  _ghostSuggestion = suggestions[0] || '';
  const input = document.getElementById('chat-input');
  suggestions.forEach(text => {
    const chip = document.createElement('button');
    chip.className = 'suggestion-chip';
    chip.textContent = text;
    chip.addEventListener('click', () => {
      input.value = text;
      clearSuggestionChips();
      autoResizeTextarea(input);
      input.focus();
      sendMessage();
    });
    container.appendChild(chip);
  });
  container.style.display = 'flex';
  // Show first suggestion as ghost text in the input so user knows Tab works
  if (_ghostSuggestion && input.value === '') {
    ghost.textContent = _ghostSuggestion;
    ghost.style.display = 'block';
    input.closest('.chat-input-wrapper').classList.add('has-ghost');
  }
}

function clearSuggestionChips() {
  _ghostSuggestion = '';
  const container = document.getElementById('suggestion-chips');
  if (container) {
    container.innerHTML = '';
    container.style.display = 'none';
  }
  const ghost = document.getElementById('ghost-text');
  if (ghost) ghost.style.display = 'none';
  const wrapper = document.querySelector('.chat-input-wrapper');
  if (wrapper) wrapper.classList.remove('has-ghost');
}

// --- Chat ---

function sendMessage() {
  clearSuggestionChips();
  removeWelcomeCard();
  _turnResponseReceived = false;
  if (_doneWithoutResponseTimer) {
    clearTimeout(_doneWithoutResponseTimer);
    _doneWithoutResponseTimer = null;
  }
  const input = document.getElementById('chat-input');
  if (authFlowPending) {
    showToast(I18n.t('chat.authRequiredBeforeSend'), 'info');
    const tokenField = document.querySelector('.auth-card .auth-token-input input');
    if (tokenField) tokenField.focus();
    return;
  }
  if (!currentThreadId) {
    console.warn('sendMessage: no thread selected, ignoring');
    return;
  }
  if (_sendCooldown) return;
  const content = input.value.trim();
  if (!content && stagedImages.length === 0) return;

  // Intercept approval keywords when an unresolved approval card is pending.
  // Find the most recent unresolved card for the current thread (resolved cards
  // linger 1.5s before removal; cards from other threads must not be matched).
  const approvalCards = Array.from(document.querySelectorAll('.approval-card'));
  const approvalCard = approvalCards.reverse().find(card => {
    if (card.querySelector('.approval-resolved')) return false;
    const cardThreadId = card.getAttribute('data-thread-id');
    return !cardThreadId || cardThreadId === currentThreadId;
  });
  if (approvalCard && content) {
    const lower = content.toLowerCase();
    let action = null;
    if (['yes', 'y', 'approve', 'ok', '/approve', '/yes', '/y'].includes(lower)) {
      action = 'approve';
    } else if (['always', 'a', 'yes always', 'approve always', '/always', '/a'].includes(lower)) {
      action = 'always';
    } else if (['no', 'n', 'deny', 'reject', 'cancel', '/deny', '/no', '/n'].includes(lower)) {
      action = 'deny';
    }
    if (action) {
      input.value = '';
      autoResizeTextarea(input);
      input.focus();
      const requestId = approvalCard.getAttribute('data-request-id');
      const threadId = approvalCard.getAttribute('data-thread-id');
      if (requestId) {
        sendApprovalAction(requestId, action, threadId);
      }
      return;
    }
  }

  const userMsg = addMessage('user', content || '(images attached)');
  pruneOldMessages();
  input.value = '';
  autoResizeTextarea(input);
  input.focus();

  const body = { content, thread_id: currentThreadId || undefined, timezone: Intl.DateTimeFormat().resolvedOptions().timeZone };
  if (stagedImages.length > 0) {
    body.images = stagedImages.map(img => ({ media_type: img.media_type, data: img.data }));
    stagedImages = [];
    renderImagePreviews();
  }

  apiFetch('/api/chat/send', {
    method: 'POST',
    body: body,
  }).catch((err) => {
    // Handle rate limiting (429)
    if (err.status === 429) {
      showToast(I18n.t('chat.rateLimited'), 'error');
      _sendCooldown = true;
      const sendBtn = document.getElementById('send-btn');
      if (sendBtn) sendBtn.disabled = true;
      setTimeout(() => {
        _sendCooldown = false;
        if (sendBtn) sendBtn.disabled = false;
      }, 2000);
    }
    // Keep the user message in DOM, add a retry link
    if (userMsg) {
      userMsg.classList.add('send-failed');
      userMsg.style.borderStyle = 'dashed';
      const retryLink = document.createElement('a');
      retryLink.className = 'retry-link';
      retryLink.href = '#';
      retryLink.textContent = I18n.t('common.retry');
      retryLink.addEventListener('click', (e) => {
        e.preventDefault();
        if (userMsg.parentNode) userMsg.parentNode.removeChild(userMsg);
        input.value = content;
        sendMessage();
      });
      userMsg.appendChild(retryLink);
    }
  });
}

function enableChatInput() {
  if (currentThreadIsReadOnly || authFlowPending) return;
  const input = document.getElementById('chat-input');
  const btn = document.getElementById('send-btn');
  if (input) {
    input.disabled = false;
  }
  if (btn) btn.disabled = false;
}

// --- Image Upload ---

function renderImagePreviews() {
  const strip = document.getElementById('image-preview-strip');
  strip.innerHTML = '';
  stagedImages.forEach((img, idx) => {
    const container = document.createElement('div');
    container.className = 'image-preview-container';

    const preview = document.createElement('img');
    preview.className = 'image-preview';
    preview.src = img.dataUrl;
    preview.alt = 'Attached image';

    const removeBtn = document.createElement('button');
    removeBtn.className = 'image-preview-remove';
    removeBtn.textContent = '\u00d7';
    removeBtn.addEventListener('click', () => {
      stagedImages.splice(idx, 1);
      renderImagePreviews();
    });

    container.appendChild(preview);
    container.appendChild(removeBtn);
    strip.appendChild(container);
  });
}

const MAX_IMAGE_SIZE_BYTES = 5 * 1024 * 1024; // 5 MB per image
const MAX_STAGED_IMAGES = 5;

function handleImageFiles(files) {
  Array.from(files).forEach(file => {
    if (!file.type.startsWith('image/')) return;
    if (file.size > MAX_IMAGE_SIZE_BYTES) {
      alert(I18n.t('chat.imageTooBig', { name: file.name, size: (file.size / 1024 / 1024).toFixed(1) }));
      return;
    }
    if (stagedImages.length >= MAX_STAGED_IMAGES) {
      alert(I18n.t('chat.maxImages', { n: MAX_STAGED_IMAGES }));
      return;
    }
    const reader = new FileReader();
    reader.onload = function(e) {
      const dataUrl = e.target.result;
      const commaIdx = dataUrl.indexOf(',');
      const meta = dataUrl.substring(0, commaIdx); // e.g. "data:image/png;base64"
      const base64 = dataUrl.substring(commaIdx + 1);
      const mediaType = meta.replace('data:', '').replace(';base64', '');
      stagedImages.push({ media_type: mediaType, data: base64, dataUrl: dataUrl });
      renderImagePreviews();
    };
    reader.readAsDataURL(file);
  });
}

document.getElementById('attach-btn').addEventListener('click', () => {
  document.getElementById('image-file-input').click();
});

document.getElementById('image-file-input').addEventListener('change', (e) => {
  handleImageFiles(e.target.files);
  e.target.value = '';
});

document.getElementById('chat-input').addEventListener('paste', (e) => {
  const items = (e.clipboardData || e.originalEvent.clipboardData).items;
  for (let i = 0; i < items.length; i++) {
    if (items[i].kind === 'file' && items[i].type.startsWith('image/')) {
      const file = items[i].getAsFile();
      if (file) handleImageFiles([file]);
    }
  }
});

const chatMessagesEl = document.getElementById('chat-messages');
chatMessagesEl.addEventListener('copy', (e) => {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed) return;
  const anchorNode = selection.anchorNode;
  const focusNode = selection.focusNode;
  if (!anchorNode || !focusNode) return;
  if (!chatMessagesEl.contains(anchorNode) || !chatMessagesEl.contains(focusNode)) return;
  const text = selection.toString();
  if (!text || !e.clipboardData) return;
  // Force plain-text clipboard output so dark-theme styling never leaks on paste.
  e.preventDefault();
  e.clipboardData.clearData();
  e.clipboardData.setData('text/plain', text);
});

function createGeneratedImageElement(dataUrl, path, eventId) {
  const card = document.createElement('div');
  card.className = 'generated-image-card';
  if (eventId) {
    card.dataset.imageEventId = eventId;
  }

  if (isSafeGeneratedImageDataUrl(dataUrl)) {
    const img = document.createElement('img');
    img.className = 'generated-image';
    img.src = dataUrl;
    img.alt = 'Generated image';
    card.appendChild(img);
  } else {
    const placeholder = document.createElement('div');
    placeholder.className = 'generated-image-placeholder';
    placeholder.textContent = 'Generated image unavailable in history payload';
    card.appendChild(placeholder);
  }

  if (path) {
    const pathLabel = document.createElement('div');
    pathLabel.className = 'generated-image-path';
    pathLabel.textContent = path;
    card.appendChild(pathLabel);
  }

  return card;
}

function isSafeGeneratedImageDataUrl(dataUrl) {
  return typeof dataUrl === 'string' && /^data:image\//i.test(dataUrl);
}

function hasRenderedGeneratedImage(container, eventId) {
  if (!eventId) return false;
  return Array.from(container.querySelectorAll('.generated-image-card')).some((card) => {
    return card.dataset.imageEventId === eventId;
  });
}

function addGeneratedImage(dataUrl, path, eventId, shouldScroll = true) {
  const container = document.getElementById('chat-messages');
  if (hasRenderedGeneratedImage(container, eventId)) {
    return;
  }
  const card = createGeneratedImageElement(dataUrl, path, eventId);
  container.appendChild(card);
  if (shouldScroll) {
    container.scrollTop = container.scrollHeight;
  }
}

function rememberGeneratedImage(threadId, eventId, dataUrl, path) {
  if (!threadId || !eventId || !isSafeGeneratedImageDataUrl(dataUrl)) return;
  const normalizedPath = path || null;
  let images = generatedImagesByThread.get(threadId);
  if (!images) {
    if (generatedImagesByThread.size >= GENERATED_IMAGE_THREAD_CACHE_CAP) {
      const oldestThreadId = generatedImagesByThread.keys().next().value;
      if (oldestThreadId) {
        generatedImagesByThread.delete(oldestThreadId);
      }
    }
    images = [];
    generatedImagesByThread.set(threadId, images);
  } else {
    // Refresh insertion order so recently viewed/updated threads stay cached.
    generatedImagesByThread.delete(threadId);
    generatedImagesByThread.set(threadId, images);
  }
  if (images.some(img => img.eventId === eventId)) {
    return;
  }
  images.push({ eventId, dataUrl, path: normalizedPath });
  while (images.length > GENERATED_IMAGES_PER_THREAD_CAP) {
    images.shift();
  }
}

function getRememberedGeneratedImage(threadId, eventId) {
  if (!threadId || !eventId) return null;
  const images = generatedImagesByThread.get(threadId);
  if (!images) return null;
  return images.find(img => img.eventId === eventId) || null;
}

function resolveGeneratedImageForRender(threadId, image) {
  const normalizedPath = image.path || null;
  if (image.data_url) {
    return { dataUrl: image.data_url, path: normalizedPath };
  }
  const remembered = getRememberedGeneratedImage(threadId, image.event_id);
  if (remembered) {
    return { dataUrl: remembered.dataUrl, path: remembered.path };
  }
  return { dataUrl: null, path: normalizedPath };
}

// --- Slash Autocomplete ---

function showSlashAutocomplete(matches) {
  const el = document.getElementById('slash-autocomplete');
  if (!el || matches.length === 0) { hideSlashAutocomplete(); return; }
  _slashMatches = matches;
  _slashSelected = -1;
  el.innerHTML = '';
  matches.forEach((item, i) => {
    const row = document.createElement('div');
    row.className = 'slash-ac-item';
    row.dataset.index = i;
    var cmdSpan = document.createElement('span');
    cmdSpan.className = 'slash-ac-cmd';
    cmdSpan.textContent = item.cmd;
    var descSpan = document.createElement('span');
    descSpan.className = 'slash-ac-desc';
    descSpan.textContent = item.desc;
    row.appendChild(cmdSpan);
    row.appendChild(descSpan);
    row.addEventListener('mousedown', (e) => {
      e.preventDefault(); // prevent blur
      selectSlashItem(item.cmd);
    });
    el.appendChild(row);
  });
  el.style.display = 'block';
}

function hideSlashAutocomplete() {
  const el = document.getElementById('slash-autocomplete');
  if (el) el.style.display = 'none';
  _slashSelected = -1;
  _slashMatches = [];
}

function selectSlashItem(cmd) {
  const input = document.getElementById('chat-input');
  input.value = cmd + ' ';
  input.focus();
  hideSlashAutocomplete();
  autoResizeTextarea(input);
}

function updateSlashHighlight() {
  const items = document.querySelectorAll('#slash-autocomplete .slash-ac-item');
  items.forEach((el, i) => el.classList.toggle('selected', i === _slashSelected));
  if (_slashSelected >= 0 && items[_slashSelected]) {
    items[_slashSelected].scrollIntoView({ block: 'nearest' });
  }
}

function filterSlashCommands(value) {
  if (!value.startsWith('/')) { hideSlashAutocomplete(); return; }
  // Only show autocomplete when the input is just a slash command prefix (no spaces except /thread new)
  const lower = value.toLowerCase();
  const matches = SLASH_COMMANDS.filter((c) => c.cmd.startsWith(lower));
  if (matches.length === 0 || (matches.length === 1 && matches[0].cmd === lower.trimEnd())) {
    hideSlashAutocomplete();
  } else {
    showSlashAutocomplete(matches);
  }
}

function sendApprovalAction(requestId, action, threadId) {
  const card = document.querySelector('.approval-card[data-request-id="' + requestId + '"]');
  const targetThreadId = threadId || (card ? card.getAttribute('data-thread-id') : null) || currentThreadId;
  apiFetch('/api/chat/gate/resolve', {
    method: 'POST',
    body: {
      request_id: requestId,
      thread_id: targetThreadId,
      resolution: action === 'deny' ? 'denied' : 'approved',
      always: action === 'always',
    },
  }).catch((err) => {
    addMessage('system', 'Failed to send approval: ' + err.message);
  });

  // Disable buttons and show confirmation on the card
  if (card) {
    const buttons = card.querySelectorAll('.approval-actions button');
    buttons.forEach((btn) => {
      btn.disabled = true;
    });
    const actions = card.querySelector('.approval-actions');
    const label = document.createElement('span');
    label.className = 'approval-resolved';
    const labelText = action === 'approve' ? I18n.t('approval.approved') : action === 'always' ? I18n.t('approval.alwaysApproved') : I18n.t('approval.denied');
    label.textContent = labelText;
    actions.appendChild(label);
    // Remove the card after showing the confirmation briefly
    setTimeout(() => { card.remove(); }, 1500);
  }
}

function renderMarkdown(text) {
  if (typeof marked !== 'undefined') {
    // Escape raw HTML error pages instead of rendering them as markup.
    // Only triggers when the text *starts with* a doctype or <html> tag
    // (after optional whitespace), so normal messages that mention HTML
    // tags in prose or code fences are not affected.  See #263.
    if (/^\s*<!doctype\s/i.test(text) || /^\s*<html[\s>]/i.test(text)) {
      return escapeHtml(text);
    }
    let html = marked.parse(text);
    // Sanitize HTML output to prevent XSS from tool output or LLM responses.
    html = sanitizeRenderedHtml(html);
    // Inject copy buttons into <pre> blocks
    html = html.replace(/<pre>/g, '<pre class="code-block-wrapper"><button class="copy-btn" data-action="copy-code">Copy</button>');
    return html;
  }
  return escapeHtml(text);
}

// Sanitize rendered HTML using DOMPurify to prevent XSS from tool output
// or prompt injection in LLM responses. DOMPurify is a DOM-based sanitizer
// that handles all known bypass vectors (SVG onload, newline-split event
// handlers, mutation XSS, etc.) unlike the regex approach it replaces.
function sanitizeRenderedHtml(html) {
  if (typeof DOMPurify !== 'undefined') {
    return DOMPurify.sanitize(html, {
      USE_PROFILES: { html: true },
      FORBID_TAGS: ['style', 'script'],
      FORBID_ATTR: ['style', 'onerror', 'onload']
    });
  }
  // DOMPurify not available (CDN unreachable) — return empty string rather than unsanitized HTML
  return '';
}

// ==================== Structured Data Rendering ====================
//
// Detects JSON objects and key-value data in assistant messages and
// renders them as styled cards instead of raw text. Also supports
// extensible chat renderers via IronClaw.registerChatRenderer().

/**
 * Post-process a .message-content element to upgrade structured data into cards.
 * Runs registered chat renderers first, then falls back to built-in JSON detection.
 */
function upgradeStructuredData(contentEl) {
  // 1. Run registered chat renderers.
  //
  // Each registered renderer receives the live `.message-content` element
  // and the textContent. The renderer is allowed to mutate the element —
  // attach event listeners, set data attributes, swap inner DOM — but any
  // HTML it injects must still pass DOMPurify before it reaches the user.
  // `renderMarkdown` already runs `sanitizeRenderedHtml` on the markdown
  // output BEFORE this function is called, but a renderer that does
  // `contentEl.innerHTML = '<form action="https://attacker">...'` would
  // bypass that sanitization step entirely. Re-run the sanitizer on
  // whatever the renderer leaves behind so the same HTML allowlist
  // applies regardless of how the content got there.
  //
  // CSP already blocks `<script>` execution either way; this guards the
  // form/iframe/object/clickjack-overlay vector that doesn't trip CSP.
  var renderers = (window.IronClaw && IronClaw._chatRenderers) || [];
  for (var i = 0; i < renderers.length; i++) {
    try {
      if (renderers[i].match(contentEl.textContent, contentEl)) {
        renderers[i].render(contentEl, contentEl.textContent);
        // Post-renderer sanitization — DOMPurify is idempotent on
        // already-safe HTML, so the cost on the happy path is bounded
        // by the sanitizer's own walk of the post-renderer subtree.
        contentEl.innerHTML = sanitizeRenderedHtml(contentEl.innerHTML);
        return; // First matching renderer wins
      }
    } catch (e) {
      console.error('[IronClaw] Chat renderer "' + renderers[i].id + '" failed:', e);
    }
  }

  // 2. Built-in: detect and upgrade inline JSON objects.
  //
  // Off by default — the bracket-counting heuristic false-positives on
  // any prose containing balanced `{...}` (e.g. an assistant explaining
  // "set the value to {x: 1, y: 2}"), and the rewrite then mangles the
  // explanation into a styled card. Operators that pipe structured data
  // through chat opt in via `chat.upgrade_inline_json` in
  // `.system/gateway/layout.json`.
  var layoutCfg = window.__IRONCLAW_LAYOUT__;
  if (layoutCfg && layoutCfg.chat && layoutCfg.chat.upgrade_inline_json === true) {
    upgradeInlineJson(contentEl);
  }
}

/**
 * Find JSON-like objects in text nodes and replace them with styled cards.
 *
 * Uses a linear bracket-counting scan instead of a regex with nested
 * quantifiers — the old `/(\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\})/g` exhibited
 * catastrophic backtracking on adversarial input. The current implementation
 * is bounded by two caps:
 *   - MAX_PARA_LEN: skip paragraphs larger than this entirely
 *   - MAX_SCAN:     each `{` scan is capped at this many chars
 *   - MAX_CANDIDATES per paragraph
 */
function upgradeInlineJson(contentEl) {
  var MAX_PARA_LEN = 20000;
  var paragraphs = contentEl.querySelectorAll('p');
  if (paragraphs.length === 0) {
    // No <p> tags — markdown might have produced bare text
    paragraphs = [contentEl];
  }

  paragraphs.forEach(function(p) {
    // Skip code blocks
    if (p.closest('pre') || p.closest('code')) return;

    var html = p.innerHTML;
    if (!html.includes('{')) return; // Fast path: no braces at all
    if (html.length > MAX_PARA_LEN) return; // Bail on very long content

    var candidates = _findJsonCandidates(html);
    if (candidates.length === 0) return;

    // Apply replacements in reverse order so earlier-index positions stay valid.
    var out = html;
    for (var i = candidates.length - 1; i >= 0; i--) {
      var c = candidates[i];
      var card = buildDataCard(c.obj);
      out = out.substring(0, c.start) + card + out.substring(c.end);
    }
    p.innerHTML = out;
  });
}

/**
 * Scan `html` once and return `{start, end, obj}` spans for every balanced
 * `{...}` that parses as a JSON object (not array, not primitive). Positions
 * inside `<code>…</code>` or `<pre>…</pre>` blocks are skipped.
 *
 * Linear in `html` length for typical input; bounded by MAX_SCAN and
 * MAX_CANDIDATES for adversarial input.
 * @private
 */
function _findJsonCandidates(html) {
  var MAX_SCAN = 5000;
  var MAX_CANDIDATES = 32;
  var results = [];
  var n = html.length;
  var i = 0;
  var lowerHtml = html.toLowerCase();

  while (i < n && results.length < MAX_CANDIDATES) {
    var ch = html.charCodeAt(i);

    // Fast-skip past <code>...</code> and <pre>...</pre> regions — avoids
    // counting braces that belong to rendered code samples.
    if (ch === 60 /* < */) {
      if (lowerHtml.substr(i, 5) === '<code') {
        var codeEnd = lowerHtml.indexOf('</code>', i + 5);
        i = codeEnd === -1 ? n : codeEnd + 7;
        continue;
      }
      if (lowerHtml.substr(i, 4) === '<pre') {
        var preEnd = lowerHtml.indexOf('</pre>', i + 4);
        i = preEnd === -1 ? n : preEnd + 6;
        continue;
      }
    }

    if (ch !== 123 /* { */) {
      i++;
      continue;
    }

    // Scan forward with brace counting; respect string literals so that
    // `"a}b"` inside an object doesn't prematurely end the scan.
    var end = _findBalancedEnd(html, i, MAX_SCAN);
    if (end === -1) {
      i++;
      continue;
    }

    var raw = html.substring(i, end);
    // Normalize Python-style single quotes to double quotes so input like
    // `{'k': 'v'}` parses as JSON. The naive `raw.replace(/'/g, '"')`
    // mangled apostrophes inside already-double-quoted string values
    // (e.g., `{"name": "it's"}` → `{"name": "it"s"}` → parse failure).
    // Walk the candidate with the same string-state tracking as
    // `_findBalancedEnd` and only rewrite single quotes that appear OUTSIDE
    // a double-quoted string. This preserves apostrophes inside `"it's"`
    // while still upgrading single-quoted JSON-like input.
    var normalized = _normalizeJsonQuotes(raw);
    try {
      var obj = JSON.parse(normalized);
      if (typeof obj === 'object' && obj !== null && !Array.isArray(obj)) {
        results.push({ start: i, end: end, obj: obj });
        i = end;
        continue;
      }
    } catch (_e) { /* not valid JSON — leave as text */ }

    i++;
  }

  return results;
}

/**
 * Rewrite single quotes that act as string delimiters to double quotes,
 * leaving single quotes that appear inside an already-double-quoted string
 * untouched. Mirrors the string-state tracking in `_findBalancedEnd` so the
 * upgrade is consistent with how the candidate was extracted.
 *
 * `{'k': 'v'}` → `{"k": "v"}`
 * `{"name": "it's"}` → `{"name": "it's"}` (apostrophe preserved)
 * `{'msg': "she said \"hi\""}` → `{"msg": "she said \"hi\""}`
 * @private
 */
function _normalizeJsonQuotes(raw) {
  var out = '';
  var inString = null; // '"' | "'" | null
  for (var k = 0; k < raw.length; k++) {
    var c = raw[k];
    if (inString) {
      // Inside a string literal — copy verbatim, including any single
      // quotes that happen to be apostrophes. Honor backslash escapes so
      // `"\""` doesn't terminate the literal early.
      if (c === '\\' && k + 1 < raw.length) {
        out += c + raw[k + 1];
        k++;
        continue;
      }
      if (c === inString) {
        // Closing quote: emit as `"` regardless of which quote opened the
        // string, so a single-quoted literal becomes a double-quoted one.
        out += '"';
        inString = null;
        continue;
      }
      out += c;
      continue;
    }
    if (c === '"' || c === "'") {
      // Opening quote: normalize to `"` and remember which character
      // closes this literal so apostrophes inside `"it's"` are preserved.
      inString = c;
      out += '"';
      continue;
    }
    out += c;
  }
  return out;
}

/**
 * Return the index one past the matching `}` for the `{` at `start`, or -1
 * if no balanced close is found within `maxLen` characters. Respects single
 * and double-quoted string literals (with backslash escapes) so `"a}b"`
 * doesn't terminate the scan.
 * @private
 */
function _findBalancedEnd(html, start, maxLen) {
  var depth = 0;
  var inString = null; // '"' | "'" | null
  var n = Math.min(html.length, start + maxLen);
  for (var j = start; j < n; j++) {
    var ch = html[j];
    if (inString) {
      if (ch === '\\') { j++; continue; } // skip escaped char
      if (ch === inString) inString = null;
      continue;
    }
    if (ch === '"' || ch === "'") { inString = ch; continue; }
    if (ch === '{') { depth++; continue; }
    if (ch === '}') {
      depth--;
      if (depth === 0) return j + 1;
    }
  }
  return -1;
}

/**
 * Build an HTML data card from a plain object.
 */
function buildDataCard(obj) {
  var keys = Object.keys(obj);
  if (keys.length === 0) return '';

  var rows = '';
  for (var i = 0; i < keys.length; i++) {
    var key = keys[i];
    var value = obj[key];
    var displayKey = key.replace(/_/g, ' ');
    var valueClass = 'data-card-value';
    var valueHtml;

    // Special rendering for known value types
    if (key === 'status' || key === 'state') {
      var badgeClass = 'status-badge';
      var sv = String(value).toLowerCase();
      if (sv === 'created' || sv === 'active' || sv === 'success' || sv === 'completed' || sv === 'ok' || sv === 'running') {
        badgeClass += ' status-success';
      } else if (sv === 'failed' || sv === 'error' || sv === 'cancelled' || sv === 'rejected') {
        badgeClass += ' status-error';
      } else if (sv === 'pending' || sv === 'waiting' || sv === 'queued') {
        badgeClass += ' status-pending';
      }
      valueHtml = '<span class="' + badgeClass + '">' + escapeHtml(String(value)) + '</span>';
    } else if (typeof value === 'object' && value !== null) {
      valueHtml = '<code>' + escapeHtml(JSON.stringify(value)) + '</code>';
    } else {
      // Check if value looks like a UUID or ID
      var strVal = String(value);
      if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(strVal)) {
        valueHtml = '<code class="data-card-id">' + escapeHtml(strVal) + '</code>';
      } else {
        valueHtml = '<span>' + escapeHtml(strVal) + '</span>';
      }
    }

    rows += '<div class="data-card-row">' +
      '<span class="data-card-label">' + escapeHtml(displayKey) + '</span>' +
      '<span class="' + valueClass + '">' + valueHtml + '</span>' +
      '</div>';
  }

  return '<div class="data-card">' + rows + '</div>';
}

function copyCodeBlock(btn) {
  const pre = btn.parentElement;
  const code = pre.querySelector('code');
  const text = code ? code.textContent : pre.textContent;
  navigator.clipboard.writeText(text).then(() => {
    btn.textContent = I18n.t('btn.copied');
    setTimeout(() => { btn.textContent = I18n.t('btn.copy'); }, 1500);
  });
}

function copyMessage(btn) {
  const message = btn.closest('.message');
  if (!message) return;
  const text = message.getAttribute('data-copy-text')
    || message.getAttribute('data-raw')
    || message.textContent
    || '';
  navigator.clipboard.writeText(text).then(() => {
    btn.textContent = I18n.t('message.copied');
    setTimeout(() => { btn.textContent = I18n.t('message.copy'); }, 1200);
  }).catch(() => {
    btn.textContent = I18n.t('common.copyFailed');
    setTimeout(() => { btn.textContent = I18n.t('message.copy'); }, 1200);
  });
}

let _lastMessageDate = null;

function maybeInsertTimeSeparator(container, timestamp) {
  const date = timestamp ? new Date(timestamp) : new Date();
  const dateStr = date.toDateString();
  if (_lastMessageDate === dateStr) return;
  _lastMessageDate = dateStr;

  const now = new Date();
  const today = now.toDateString();
  const yesterday = new Date(now.getTime() - 86400000).toDateString();

  let label;
  if (dateStr === today) label = 'Today';
  else if (dateStr === yesterday) label = 'Yesterday';
  else label = date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });

  const sep = document.createElement('div');
  sep.className = 'time-separator';
  sep.textContent = label;
  container.appendChild(sep);
}

// Remove oldest messages/activity groups from the DOM when the chat container
// exceeds MAX_DOM_MESSAGES elements. Users can scroll up to trigger
// loadHistory() for older content. This prevents unbounded DOM growth during
// long sessions. Elements with data-streaming="true" are preserved to avoid
// breaking mid-stream responses.
// Note: if every element has data-streaming="true", this function will
// under-prune and the DOM may temporarily exceed the cap. This is acceptable
// because streaming completes quickly and the next call will clean up.
function pruneOldMessages() {
  const container = document.getElementById('chat-messages');
  const items = container.querySelectorAll('.message, .activity-group, .time-separator');
  if (items.length <= MAX_DOM_MESSAGES) return;
  let removed = 0;
  const target = items.length - MAX_DOM_MESSAGES;
  for (let i = 0; i < items.length && removed < target; i++) {
    if (items[i].getAttribute('data-streaming') === 'true') continue;
    items[i].remove();
    removed++;
  }
  // Clean up orphaned leading time-separators left after pruning.
  // A separator is orphaned if no .message or .activity-group follows it
  // before the next separator (or end of container).
  const remaining = container.querySelectorAll('.message, .activity-group, .time-separator');
  for (let i = 0; i < remaining.length; i++) {
    if (!remaining[i].classList.contains('time-separator')) break;
    remaining[i].remove();
  }
}

function addMessage(role, content) {
  const container = document.getElementById('chat-messages');
  maybeInsertTimeSeparator(container);
  const div = createMessageElement(role, content);
  container.appendChild(div);
  container.scrollTop = container.scrollHeight;
  return div;
}

function appendToLastAssistant(chunk) {
  const container = document.getElementById('chat-messages');
  const messages = container.querySelectorAll('.message.assistant');
  if (messages.length > 0) {
    const last = messages[messages.length - 1];
    const raw = (last.getAttribute('data-raw') || '') + chunk;
    last.setAttribute('data-raw', raw);
    last.setAttribute('data-copy-text', raw);
    const content = last.querySelector('.message-content');
    if (content) {
      content.innerHTML = renderMarkdown(raw);
      // Syntax highlighting for code blocks
      if (typeof hljs !== 'undefined') {
        requestAnimationFrame(() => {
          content.querySelectorAll('pre code').forEach(block => {
            hljs.highlightElement(block);
          });
        });
      }
    }
    container.scrollTop = container.scrollHeight;
  } else {
    addMessage('assistant', chunk);
  }
}

// --- Inline Tool Activity Cards ---

function getOrCreateActivityGroup() {
  if (_activeGroup) return _activeGroup;
  const container = document.getElementById('chat-messages');
  const group = document.createElement('div');
  group.className = 'activity-group';
  container.appendChild(group);
  container.scrollTop = container.scrollHeight;
  _activeGroup = group;
  _activeToolCards = {};
  return group;
}

function showActivityThinking(message) {
  const group = getOrCreateActivityGroup();
  if (_activityThinking) {
    // Already exists — just update text and un-hide
    _activityThinking.style.display = '';
    _activityThinking.querySelector('.activity-thinking-text').textContent = message;
  } else {
    _activityThinking = document.createElement('div');
    _activityThinking.className = 'activity-thinking';
    _activityThinking.innerHTML =
      '<span class="activity-thinking-dots">'
      + '<span class="activity-thinking-dot"></span>'
      + '<span class="activity-thinking-dot"></span>'
      + '<span class="activity-thinking-dot"></span>'
      + '</span>'
      + '<span class="activity-thinking-text"></span>';
    group.appendChild(_activityThinking);
    _activityThinking.querySelector('.activity-thinking-text').textContent = message;
  }
  const container = document.getElementById('chat-messages');
  container.scrollTop = container.scrollHeight;
}

function removeActivityThinking() {
  if (_activityThinking) {
    _activityThinking.remove();
    _activityThinking = null;
  }
}

function addToolCard(name) {
  // Hide thinking instead of destroying — it may reappear between tool rounds
  if (_activityThinking) _activityThinking.style.display = 'none';
  const group = getOrCreateActivityGroup();

  const card = document.createElement('div');
  card.className = 'activity-tool-card';
  card.setAttribute('data-tool-name', name);
  card.setAttribute('data-status', 'running');

  const header = document.createElement('div');
  header.className = 'activity-tool-header';

  const icon = document.createElement('span');
  icon.className = 'activity-tool-icon';
  icon.innerHTML = '<div class="spinner"></div>';

  const toolName = document.createElement('span');
  toolName.className = 'activity-tool-name';
  toolName.textContent = name;

  const duration = document.createElement('span');
  duration.className = 'activity-tool-duration';
  duration.textContent = '';

  const chevron = document.createElement('span');
  chevron.className = 'activity-tool-chevron';
  chevron.innerHTML = '&#9656;';

  header.appendChild(icon);
  header.appendChild(toolName);
  header.appendChild(duration);
  header.appendChild(chevron);

  const body = document.createElement('div');
  body.className = 'activity-tool-body';

  const output = document.createElement('pre');
  output.className = 'activity-tool-output';
  body.appendChild(output);

  header.addEventListener('click', () => {
    body.classList.toggle('expanded');
    chevron.classList.toggle('expanded', body.classList.contains('expanded'));
  });

  card.appendChild(header);
  card.appendChild(body);
  group.appendChild(card);

  const startTime = Date.now();
  const timerInterval = setInterval(() => {
    const elapsed = (Date.now() - startTime) / 1000;
    if (elapsed > 300) { clearInterval(timerInterval); return; }
    duration.textContent = elapsed < 10 ? elapsed.toFixed(1) + 's' : Math.floor(elapsed) + 's';
  }, 100);

  if (!_activeToolCards[name]) _activeToolCards[name] = [];
  _activeToolCards[name].push({ card, startTime, timer: timerInterval, duration, icon, finalDuration: null });

  const container = document.getElementById('chat-messages');
  container.scrollTop = container.scrollHeight;
}

function completeToolCard(name, success, error, parameters) {
  const entries = _activeToolCards[name];
  if (!entries || entries.length === 0) return;
  // Find first running card
  let entry = null;
  for (let i = 0; i < entries.length; i++) {
    if (entries[i].card.getAttribute('data-status') === 'running') {
      entry = entries[i];
      break;
    }
  }
  if (!entry) entry = entries[entries.length - 1];

  clearInterval(entry.timer);
  const elapsed = (Date.now() - entry.startTime) / 1000;
  entry.finalDuration = elapsed;
  entry.duration.textContent = elapsed < 10 ? elapsed.toFixed(1) + 's' : Math.floor(elapsed) + 's';
  entry.icon.innerHTML = success
    ? '<span class="activity-icon-success">&#10003;</span>'
    : '<span class="activity-icon-fail">&#10007;</span>';
  entry.card.setAttribute('data-status', success ? 'success' : 'fail');

  // For failed tools, populate the body with error details and auto-expand
  if (!success && (error || parameters)) {
    const output = entry.card.querySelector('.activity-tool-output');
    if (output) {
      let detail = '';
      if (parameters) {
        detail += 'Input:\n' + parameters + '\n\n';
      }
      if (error) {
        detail += 'Error:\n' + error;
      }
      output.textContent = detail;

      // Auto-expand so the error is immediately visible
      const body = entry.card.querySelector('.activity-tool-body');
      const chevron = entry.card.querySelector('.activity-tool-chevron');
      if (body) body.classList.add('expanded');
      if (chevron) chevron.classList.add('expanded');
    }
  }
}

function setToolCardOutput(name, preview) {
  const entries = _activeToolCards[name];
  if (!entries || entries.length === 0) return;
  // Find first card with empty output
  let entry = null;
  for (let i = 0; i < entries.length; i++) {
    const out = entries[i].card.querySelector('.activity-tool-output');
    if (out && !out.textContent) {
      entry = entries[i];
      break;
    }
  }
  if (!entry) entry = entries[entries.length - 1];

  const output = entry.card.querySelector('.activity-tool-output');
  if (output) {
    const truncated = preview.length > 2000 ? preview.substring(0, 2000) + '\n... (truncated)' : preview;
    output.textContent = truncated;
  }
}

function finalizeActivityGroup() {
  removeActivityThinking();
  if (!_activeGroup) return;

  // Stop all timers
  for (const name in _activeToolCards) {
    const entries = _activeToolCards[name];
    for (let i = 0; i < entries.length; i++) {
      clearInterval(entries[i].timer);
    }
  }

  // Count tools and total duration
  let toolCount = 0;
  let totalDuration = 0;
  for (const tname in _activeToolCards) {
    const tentries = _activeToolCards[tname];
    for (let j = 0; j < tentries.length; j++) {
      const entry = tentries[j];
      toolCount++;
      if (entry.finalDuration !== null) {
        totalDuration += entry.finalDuration;
      } else {
        // Tool was still running when finalized
        totalDuration += (Date.now() - entry.startTime) / 1000;
      }
    }
  }

  if (toolCount === 0) {
    // No tools were used — remove the empty group
    _activeGroup.remove();
    _activeGroup = null;
    _activeToolCards = {};
    return;
  }

  // Wrap existing cards into a hidden container
  const cardsContainer = document.createElement('div');
  cardsContainer.className = 'activity-cards-container';
  cardsContainer.style.display = 'none';

  const cards = _activeGroup.querySelectorAll('.activity-tool-card');
  for (let k = 0; k < cards.length; k++) {
    cardsContainer.appendChild(cards[k]);
  }

  // Build summary line
  const durationStr = totalDuration < 10 ? totalDuration.toFixed(1) + 's' : Math.floor(totalDuration) + 's';
  const toolWord = toolCount === 1 ? 'tool' : 'tools';
  const summary = document.createElement('div');
  summary.className = 'activity-summary';
  summary.innerHTML = '<span class="activity-summary-chevron">&#9656;</span>'
    + '<span class="activity-summary-text">Used ' + toolCount + ' ' + toolWord + '</span>'
    + '<span class="activity-summary-duration">(' + durationStr + ')</span>';

  summary.addEventListener('click', () => {
    const isOpen = cardsContainer.style.display !== 'none';
    cardsContainer.style.display = isOpen ? 'none' : 'block';
    summary.querySelector('.activity-summary-chevron').classList.toggle('expanded', !isOpen);
  });

  // Clear group and add summary + hidden cards
  _activeGroup.innerHTML = '';
  _activeGroup.classList.add('collapsed');
  _activeGroup.appendChild(summary);
  _activeGroup.appendChild(cardsContainer);

  _activeGroup = null;
  _activeToolCards = {};
}

function humanizeToolName(rawName) {
  if (!rawName) return '';
  return String(rawName)
    .replace(/[_-]+/g, ' ')
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/^tool([a-zA-Z])/, 'tool $1')
    .replace(/\s+/g, ' ')
    .trim();
}

function shouldShowChannelConnectedMessage(extensionName, success) {
  return false;
}

function showApproval(data) {
  // Avoid duplicate cards on reconnect/history refresh.
  const existing = document.querySelector('.approval-card[data-request-id="' + CSS.escape(data.request_id) + '"]');
  if (existing) return;

  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'approval-card';
  card.setAttribute('data-request-id', data.request_id);
  const cardThreadId = data.thread_id || currentThreadId;
  if (cardThreadId) {
    card.setAttribute('data-thread-id', cardThreadId);
  }

  const header = document.createElement('div');
  header.className = 'approval-header';
  header.textContent = I18n.t('approval.title');
  card.appendChild(header);

  const toolName = document.createElement('div');
  toolName.className = 'approval-tool-name';
  toolName.textContent = humanizeToolName(data.tool_name);
  card.appendChild(toolName);

  if (data.description) {
    const desc = document.createElement('div');
    desc.className = 'approval-description';
    desc.textContent = data.description;
    card.appendChild(desc);
  }

  if (data.parameters) {
    const paramsToggle = document.createElement('button');
    paramsToggle.className = 'approval-params-toggle';
    paramsToggle.textContent = I18n.t('approval.showParams');
    const paramsBlock = document.createElement('pre');
    paramsBlock.className = 'approval-params';
    paramsBlock.textContent = data.parameters;
    paramsBlock.style.display = 'none';
    paramsToggle.addEventListener('click', () => {
      const visible = paramsBlock.style.display !== 'none';
      paramsBlock.style.display = visible ? 'none' : 'block';
      paramsToggle.textContent = visible ? I18n.t('approval.showParams') : I18n.t('approval.hideParams');
    });
    card.appendChild(paramsToggle);
    card.appendChild(paramsBlock);
  }

  const actions = document.createElement('div');
  actions.className = 'approval-actions';

  const approveBtn = document.createElement('button');
  approveBtn.className = 'approve';
  approveBtn.textContent = I18n.t('approval.approve');
  approveBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'approve', cardThreadId));

  const denyBtn = document.createElement('button');
  denyBtn.className = 'deny';
  denyBtn.textContent = I18n.t('approval.deny');
  denyBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'deny', cardThreadId));

  actions.appendChild(approveBtn);
  if (data.allow_always !== false) {
    const alwaysBtn = document.createElement('button');
    alwaysBtn.className = 'always';
    alwaysBtn.textContent = I18n.t('approval.always');
    alwaysBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'always', cardThreadId));
    actions.appendChild(alwaysBtn);
  }
  actions.appendChild(denyBtn);
  card.appendChild(actions);

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
}

// --- Plan Checklist ---

function renderPlanChecklist(data) {
  const chatContainer = document.getElementById('chat-messages');
  const planId = data.plan_id;

  // Find or create the plan container
  let container = chatContainer.querySelector('.plan-container[data-plan-id="' + CSS.escape(planId) + '"]');
  if (!container) {
    container = document.createElement('div');
    container.className = 'plan-container';
    container.setAttribute('data-plan-id', planId);
    chatContainer.appendChild(container);
  }

  // Clear and rebuild
  container.innerHTML = '';

  // Header
  const header = document.createElement('div');
  header.className = 'plan-header';

  const title = document.createElement('span');
  title.className = 'plan-title';
  title.textContent = data.title || planId;
  header.appendChild(title);

  const badge = document.createElement('span');
  badge.className = 'plan-status-badge plan-status-' + (data.status || 'draft');
  badge.textContent = data.status || 'draft';
  header.appendChild(badge);

  container.appendChild(header);

  // Steps
  if (data.steps && data.steps.length > 0) {
    const stepsList = document.createElement('div');
    stepsList.className = 'plan-steps';

    let completed = 0;
    for (const step of data.steps) {
      const stepEl = document.createElement('div');
      stepEl.className = 'plan-step';
      stepEl.setAttribute('data-status', step.status || 'pending');

      const icon = document.createElement('span');
      icon.className = 'plan-step-icon';
      if (step.status === 'completed') {
        icon.textContent = '\u2713'; // checkmark
        completed++;
      } else if (step.status === 'failed') {
        icon.textContent = '\u2717'; // X
      } else if (step.status === 'in_progress') {
        icon.innerHTML = '<span class="plan-spinner"></span>';
      } else {
        icon.textContent = '\u25CB'; // circle
      }
      stepEl.appendChild(icon);

      const text = document.createElement('span');
      text.className = 'plan-step-text';
      text.textContent = step.title;
      stepEl.appendChild(text);

      if (step.result) {
        const result = document.createElement('span');
        result.className = 'plan-step-result';
        result.textContent = step.result;
        stepEl.appendChild(result);
      }

      stepsList.appendChild(stepEl);
    }
    container.appendChild(stepsList);

    // Summary
    const summary = document.createElement('div');
    summary.className = 'plan-summary';
    summary.textContent = completed + ' of ' + data.steps.length + ' steps completed';
    if (data.mission_id) {
      summary.textContent += ' \u00b7 Mission: ' + data.mission_id.substring(0, 8);
    }
    container.appendChild(summary);
  }

  chatContainer.scrollTop = chatContainer.scrollHeight;
}

function showJobCard(data) {
  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'job-card';

  const icon = document.createElement('span');
  icon.className = 'job-card-icon';
  icon.textContent = '\u2692';
  card.appendChild(icon);

  const info = document.createElement('div');
  info.className = 'job-card-info';

  const title = document.createElement('div');
  title.className = 'job-card-title';
  title.textContent = data.title || I18n.t('sandbox.job');
  info.appendChild(title);

  const id = document.createElement('div');
  id.className = 'job-card-id';
  id.textContent = (data.job_id || '').substring(0, 8);
  info.appendChild(id);

  card.appendChild(info);

  const viewBtn = document.createElement('button');
  viewBtn.className = 'job-card-view';
  viewBtn.textContent = I18n.t('jobs.viewJob');
  viewBtn.addEventListener('click', () => {
    switchTab('jobs');
    openJobDetail(data.job_id);
  });
  card.appendChild(viewBtn);

  if (data.browse_url) {
    const browseBtn = document.createElement('a');
    browseBtn.className = 'job-card-browse';
    browseBtn.href = data.browse_url;
    browseBtn.target = '_blank';
    browseBtn.rel = 'noopener noreferrer';
    browseBtn.textContent = I18n.t('jobs.browse');
    card.appendChild(browseBtn);
  }

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
}

// --- Auth card ---

async function handleAuthRequired(data) {
  if (data.thread_id && !isCurrentThread(data.thread_id)) {
    unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
    debouncedLoadThreads();
    return;
  }
  if (data.extension_name && getConfigureOverlay(data.extension_name)) {
    setAuthFlowPending(true, data.instructions);
    return;
  }
  const existingCard = data.extension_name ? getAuthCard(data.extension_name) : getAuthCard();
  if (existingCard && !data.request_id) {
    const existingRequestId = existingCard.getAttribute('data-request-id');
    const existingThreadId = existingCard.getAttribute('data-thread-id');
    const incomingThreadId = data.thread_id || currentThreadId || null;
    if (existingRequestId && (!existingThreadId || !incomingThreadId || existingThreadId === incomingThreadId)) {
      return;
    }
  }
  if (!data.request_id) {
    const threadId = data.thread_id || currentThreadId || null;
    if (threadId) {
      try {
        const history = await apiFetch('/api/chat/history?thread_id=' + encodeURIComponent(threadId));
        const pendingGate = history && history.pending_gate;
        if (pendingGate && pendingGate.request_id) {
          const resumeKind = parseGateResumeKind(pendingGate.resume_kind);
          if (resumeKind && resumeKind.type === 'authentication') {
            handleGateRequired({
              ...pendingGate,
              thread_id: pendingGate.thread_id || threadId,
            });
            return;
          }
        }
      } catch (_) {
        // Fall through to the legacy card when pending-gate hydration fails.
      }
    }
  }
  setAuthFlowPending(true, data.instructions);
  if (data.auth_url) {
    // Token paste flow (with optional OAuth button): show the global auth
    // prompt card. This handles both OAuth credentials (auth_url present)
    // and skill-based credentials (instructions present, no auth_url).
    showAuthCard(data);
  } else {
    if (getConfigureOverlay(data.extension_name)) return;
    showSetupCardForExtension(data);
  }
}

function parseGateResumeKind(resumeKind) {
  if (!resumeKind || typeof resumeKind !== 'object') return null;
  if (resumeKind.Approval) return { type: 'approval', ...resumeKind.Approval };
  if (resumeKind.Authentication) return { type: 'authentication', ...resumeKind.Authentication };
  if (resumeKind.External) return { type: 'external', ...resumeKind.External };
  return null;
}

function handleGateRequired(data) {
  const hasThread = !!data.thread_id;
  const forCurrentThread = !hasThread || isCurrentThread(data.thread_id);
  const resume = parseGateResumeKind(data.resume_kind);
  if (!forCurrentThread) {
    unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
    debouncedLoadThreads();
    return;
  }
  if (resume && resume.type === 'authentication') {
    handleAuthRequired({
      extension_name: resume.credential_name,
      instructions: resume.instructions,
      auth_url: resume.auth_url || null,
      request_id: data.request_id,
      thread_id: data.thread_id || currentThreadId,
    });
    return;
  }
  showApproval({
    request_id: data.request_id,
    tool_name: data.tool_name,
    description: data.description,
    parameters: data.parameters,
    allow_always: !(resume && resume.type === 'approval' && resume.allow_always === false),
    thread_id: data.thread_id || currentThreadId,
  });
}

function handleGateResolved(data) {
  const hasThread = !!data.thread_id;
  if (hasThread && !isCurrentThread(data.thread_id)) {
    debouncedLoadThreads();
    return;
  }
  document.querySelectorAll('.approval-card[data-request-id="' + CSS.escape(data.request_id) + '"]').forEach((el) => el.remove());
  if (
    data.resolution === 'credential_provided'
    || data.resolution === 'cancelled'
    || data.resolution === 'external_callback'
  ) {
    removeAuthCard();
    enableChatInput();
  }
}

function handleAuthCompleted(data) {
  if (data.thread_id && !isCurrentThread(data.thread_id)) {
    debouncedLoadThreads();
    return;
  }
  showToast(data.message, data.success ? 'success' : 'error');
  // Dismiss only the matching extension's UI so stale prompts are cleared.
  removeAuthCard(data.extension_name);
  removeSetupCard(data.extension_name);
  closeConfigureModal(data.extension_name);
  if (!data.success) {
    setAuthFlowPending(false);
    if (currentTab === 'extensions') loadExtensions();
    enableChatInput();
    return;
  }
  setAuthFlowPending(false);
  if (shouldShowChannelConnectedMessage(data.extension_name, data.success)) {
    addMessage('system', 'Telegram is now connected. You can message me there and I can send you notifications.');
  }
  if (currentTab === 'settings') refreshCurrentSettingsTab();
  enableChatInput();
}

function handlePairingRequired(data) {
  if (data.thread_id && !isCurrentThread(data.thread_id)) {
    unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
    debouncedLoadThreads();
    return;
  }
  showPairingCard(data);
}

function handlePairingCompleted(data) {
  if (data.thread_id && !isCurrentThread(data.thread_id)) {
    debouncedLoadThreads();
    return;
  }
  removePairingCard(data.channel);
  const recentApprovalAt = _recentLocalPairingApprovals.get(data.channel);
  if (!recentApprovalAt || Date.now() - recentApprovalAt > 5000) {
    showToast(data.message, data.success ? 'success' : 'error');
  }
  _recentLocalPairingApprovals.delete(data.channel);
  if (currentTab === 'settings') refreshCurrentSettingsTab();
}

function queryByDataAttribute(selector, attributeName, attributeValue) {
  if (typeof attributeValue !== 'string') return document.querySelector(selector);

  if (window.CSS && typeof window.CSS.escape === 'function') {
    return document.querySelector(
      selector + '[' + attributeName + '="' + window.CSS.escape(attributeValue) + '"]'
    );
  }

  const candidates = document.querySelectorAll(selector);
  for (const candidate of candidates) {
    if (candidate.getAttribute(attributeName) === attributeValue) return candidate;
  }
  return null;
}

function getAuthOverlay(extensionName) {
  return queryByDataAttribute('.auth-overlay', 'data-extension-name', extensionName);
}

function getAuthCard(extensionName) {
  return queryByDataAttribute('.auth-card', 'data-extension-name', extensionName);
}

function getPairingCard(channel) {
  return queryByDataAttribute('.pairing-card', 'data-channel', channel);
}

function getConfigureOverlay(extensionName) {
  return queryByDataAttribute('.configure-overlay', 'data-extension-name', extensionName);
}

function removeSetupCard(extensionName) {
  removeAuthCard(extensionName);
}

function buildSetupFields(form, extensionName, secrets, submitFn) {
  const fields = [];
  (secrets || []).forEach((secret) => {
    const field = document.createElement('label');
    field.className = 'setup-field';

    const label = document.createElement('span');
    label.className = 'setup-label';
    label.textContent = secret.prompt;
    field.appendChild(label);

    const inputRow = document.createElement('div');
    inputRow.className = 'setup-input-row';

    const input = document.createElement('input');
    input.className = 'setup-input';
    input.type = 'password';
    input.name = secret.name;
    input.placeholder = secret.provided ? I18n.t('config.alreadySet') : secret.prompt;
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submitFn();
    });
    inputRow.appendChild(input);
    field.appendChild(inputRow);
    form.appendChild(field);
    fields.push({ name: secret.name, input });
  });
  return fields;
}

function showSetupCardForExtension(data) {
  // Dedup: don't open if a configure modal is already showing for this extension
  if (getConfigureOverlay(data.extension_name)) return;
  showConfigureModal(data.extension_name, { authData: data });
}

function showAuthCard(data) {
  if (data.thread_id && !isCurrentThread(data.thread_id)) return;
  // Keep a single global auth prompt so the experience is consistent across tabs.
  const existing = getAuthOverlay();
  if (existing) existing.remove();

  const overlay = document.createElement('div');
  overlay.className = 'auth-overlay';
  overlay.setAttribute('data-extension-name', data.extension_name);
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) cancelAuth(data.extension_name);
  });

  const card = document.createElement('div');
  card.className = 'auth-card auth-modal';
  card.setAttribute('data-extension-name', data.extension_name);
  if (data.thread_id) {
    card.setAttribute('data-thread-id', data.thread_id);
  }
  if (data.request_id) {
    card.setAttribute('data-request-id', data.request_id);
  }

  const header = document.createElement('div');
  header.className = 'auth-header';
  header.textContent = I18n.t('authRequired.title', {name: data.extension_name});
  card.appendChild(header);

  if (data.instructions) {
    const instr = document.createElement('div');
    instr.className = 'auth-instructions';
    instr.textContent = data.instructions;
    card.appendChild(instr);
  }

  const links = document.createElement('div');
  links.className = 'auth-links';

  if (data.auth_url) {
    const parsedAuthUrl = parseHttpsOAuthUrl(data.auth_url);
    if (parsedAuthUrl) {
      const oauthLink = document.createElement('a');
      oauthLink.className = 'auth-oauth';
      oauthLink.href = parsedAuthUrl.href;
      oauthLink.target = '_blank';
      // Match the other external links: include `noreferrer` so the
      // OAuth provider does not see the in-app Referer header.
      oauthLink.rel = 'noopener noreferrer';
      oauthLink.textContent = I18n.t('authRequired.authenticateWith', {name: data.extension_name});
      links.appendChild(oauthLink);
    }
  }

  if (data.setup_url) {
    const parsedSetupUrl = parseHttpsExternalUrl(data.setup_url, 'setup');
    if (parsedSetupUrl) {
      const setupLink = document.createElement('a');
      setupLink.href = parsedSetupUrl.href;
      setupLink.target = '_blank';
      setupLink.rel = 'noopener noreferrer';
      setupLink.textContent = I18n.t('authRequired.getToken');
      links.appendChild(setupLink);
    }
  }

  if (links.children.length > 0) {
    card.appendChild(links);
  }

  // Token input
  const tokenRow = document.createElement('div');
  tokenRow.className = 'auth-token-input';

  const tokenInput = document.createElement('input');
  tokenInput.type = 'password';
  tokenInput.placeholder = data.instructions
    || I18n.t('auth.tokenPlaceholder');
  tokenInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') submitAuthToken(data.extension_name, tokenInput.value);
  });
  tokenRow.appendChild(tokenInput);
  card.appendChild(tokenRow);

  // Error display (hidden initially)
  const errorEl = document.createElement('div');
  errorEl.className = 'auth-error';
  errorEl.style.display = 'none';
  card.appendChild(errorEl);

  // Action buttons
  const actions = document.createElement('div');
  actions.className = 'auth-actions';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'auth-submit';
  submitBtn.textContent = I18n.t('btn.submit');
  submitBtn.addEventListener('click', () => submitAuthToken(data.extension_name, tokenInput.value));

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'auth-cancel';
  cancelBtn.textContent = I18n.t('btn.cancel');
  cancelBtn.addEventListener('click', () => cancelAuth(data.extension_name));

  actions.appendChild(submitBtn);
  actions.appendChild(cancelBtn);
  card.appendChild(actions);

  overlay.appendChild(card);
  document.body.appendChild(overlay);
  tokenInput.focus();
}

function removeAuthCard(extensionName) {
  const overlay = getAuthOverlay(extensionName);
  if (overlay) {
    overlay.remove();
    return;
  }
  const card = getAuthCard(extensionName);
  if (card) {
    const parentOverlay = card.closest('.auth-overlay');
    if (parentOverlay) parentOverlay.remove();
    else card.remove();
  }
}

function showPairingCard(data) {
  if (data.thread_id && !isCurrentThread(data.thread_id)) return;
  removePairingCard(data.channel);

  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'auth-card pairing-card';
  card.setAttribute('data-channel', data.channel);
  if (data.thread_id) {
    card.setAttribute('data-thread-id', data.thread_id);
  }

  const header = document.createElement('div');
  header.className = 'auth-header';
  header.textContent = (data.onboarding && data.onboarding.pairing_title) || ('Claim ownership for ' + data.channel);
  card.appendChild(header);

  const instr = document.createElement('div');
  instr.className = 'auth-instructions';
  instr.textContent = (data.onboarding && data.onboarding.pairing_instructions)
    || data.instructions
    || ('Paste the pairing code from ' + data.channel + '.');
  card.appendChild(instr);

  if (data.onboarding && data.onboarding.restart_instructions) {
    const restart = document.createElement('div');
    restart.className = 'setup-next-step pairing-restart';
    restart.textContent = data.onboarding.restart_instructions;
    card.appendChild(restart);
  }

  const inputRow = document.createElement('div');
  inputRow.className = 'auth-token-input';

  const codeInput = document.createElement('input');
  codeInput.type = 'text';
  codeInput.placeholder = I18n.t('extensions.pairingCodePlaceholder');
  codeInput.autocomplete = 'off';
  codeInput.spellcheck = false;
  codeInput.autocapitalize = 'characters';
  codeInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') submitPairingCode(data.channel, codeInput.value, card);
  });
  inputRow.appendChild(codeInput);
  card.appendChild(inputRow);

  const errorEl = document.createElement('div');
  errorEl.className = 'auth-error';
  errorEl.style.display = 'none';
  card.appendChild(errorEl);

  const actions = document.createElement('div');
  actions.className = 'auth-actions';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'auth-submit pairing-submit';
  submitBtn.textContent = I18n.t('approval.approve');
  submitBtn.addEventListener('click', () => submitPairingCode(data.channel, codeInput.value, card));

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'auth-cancel pairing-cancel';
  cancelBtn.textContent = I18n.t('btn.cancel');
  cancelBtn.addEventListener('click', () => cancelPairingCard(data.channel, data.onboarding));

  actions.appendChild(submitBtn);
  actions.appendChild(cancelBtn);
  card.appendChild(actions);

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
  codeInput.focus();
}

function cancelPairingCard(channel, onboarding) {
  removePairingCard(channel);
  showToast(
    (onboarding && onboarding.restart_instructions) || I18n.t('extensions.pairingRestartHint'),
    'info'
  );
}

function removePairingCard(channel) {
  const card = getPairingCard(channel);
  if (card) card.remove();
}

function showPairingCardError(channel, message) {
  const card = getPairingCard(channel);
  if (!card) return;
  card.querySelectorAll('button').forEach((btn) => {
    btn.disabled = false;
  });
  const errorEl = card.querySelector('.auth-error');
  if (errorEl) {
    errorEl.textContent = message;
    errorEl.style.display = 'block';
  }
}

function submitPairingCode(channel, codeValue, cardEl) {
  approvePairing(channel, codeValue, {
    skipSuccessToast: true,
    skipRefresh: true,
    onSuccess: function() {
      removePairingCard(channel);
    },
    onError: function(message) {
      showPairingCardError(channel, message);
      const card = cardEl || getPairingCard(channel);
      if (card) {
        const input = card.querySelector('.auth-token-input input');
        if (input) input.focus();
      }
    }
  });
}

function submitAuthToken(extensionName, tokenValue) {
  if (!tokenValue || !tokenValue.trim()) return;

  // Disable submit button while in flight
  const card = getAuthCard(extensionName);
  const threadId = card ? card.getAttribute('data-thread-id') : null;
  if (card) {
    const btns = card.querySelectorAll('button');
    btns.forEach((b) => { b.disabled = true; });
  }

  const isGateResolution = !!(card && card.getAttribute('data-request-id'));
  const requestId = card ? card.getAttribute('data-request-id') : null;
  const request = isGateResolution ? apiFetch('/api/chat/gate/resolve', {
    method: 'POST',
    body: {
      request_id: requestId,
      thread_id: threadId || currentThreadId || undefined,
      resolution: 'credential_provided',
      token: tokenValue.trim(),
    },
  }) : apiFetch('/api/chat/auth-token', {
    method: 'POST',
    body: {
      extension_name: extensionName,
      token: tokenValue.trim(),
      request_id: requestId,
      thread_id: threadId || currentThreadId || undefined,
    },
  });

  request.then((result) => {
    if (result.success) {
      // Close immediately for responsiveness; the authoritative success UX
      // (toast + extensions refresh) still comes from auth_completed SSE.
      removeAuthCard(extensionName);
      enableChatInput();
    } else {
      showAuthCardError(extensionName, result.message);
    }
  }).catch((err) => {
    showAuthCardError(extensionName, 'Failed: ' + err.message);
  });
}

function cancelAuth(extensionName) {
  const card = getAuthCard(extensionName);
  const threadId = card ? card.getAttribute('data-thread-id') : null;
  const requestId = card ? card.getAttribute('data-request-id') : null;
  const request = requestId ? apiFetch('/api/chat/gate/resolve', {
    method: 'POST',
    body: {
      request_id: requestId,
      thread_id: threadId || currentThreadId || undefined,
      resolution: 'cancelled',
    },
  }) : apiFetch('/api/chat/auth-cancel', {
    method: 'POST',
    body: {
      extension_name: extensionName,
      request_id: requestId,
      thread_id: threadId || currentThreadId || undefined,
    },
  });
  request.catch(() => {});
  removeAuthCard(extensionName);
  setAuthFlowPending(false);
  enableChatInput();
}

function showAuthCardError(extensionName, message) {
  const card = getAuthCard(extensionName);
  if (!card) return;
  // Re-enable buttons
  const btns = card.querySelectorAll('button');
  btns.forEach((b) => { b.disabled = false; });
  // Show error
  const errorEl = card.querySelector('.auth-error');
  if (errorEl) {
    errorEl.textContent = message;
    errorEl.style.display = 'block';
  }
}

function setAuthFlowPending(pending, instructions) {
  authFlowPending = !!pending;
  const input = document.getElementById('chat-input');
  const btn = document.getElementById('send-btn');
  if (!input || !btn) return;
  if (authFlowPending) {
    input.disabled = true;
    btn.disabled = true;
    return;
  }
  if (!currentThreadIsReadOnly) {
    input.disabled = false;
    btn.disabled = false;
  }
}

function loadHistory(before) {
  clearSuggestionChips();
  let historyUrl = '/api/chat/history?limit=50';
  if (currentThreadId) {
    historyUrl += '&thread_id=' + encodeURIComponent(currentThreadId);
  }
  if (before) {
    historyUrl += '&before=' + encodeURIComponent(before);
  }

  const isPaginating = !!before;
  if (isPaginating) loadingOlder = true;

  // Show skeleton while loading (only for fresh loads)
  if (!isPaginating) {
    const chatContainer = document.getElementById('chat-messages');
    chatContainer.innerHTML = '';
    chatContainer.appendChild(renderSkeleton('message', 3));
  }

  apiFetch(historyUrl).then((data) => {
    const container = document.getElementById('chat-messages');

    if (!isPaginating) {
      // Fresh load: clear and render
      container.innerHTML = '';
      for (const turn of data.turns) {
        if (turn.user_input) {
          addMessage('user', turn.user_input);
        }
        if (turn.tool_calls && turn.tool_calls.length > 0) {
          addToolCallsSummary(turn.tool_calls);
        }
        if (turn.generated_images && turn.generated_images.length > 0) {
          for (const image of turn.generated_images) {
            const resolvedImage = resolveGeneratedImageForRender(currentThreadId, image);
            rememberGeneratedImage(
              currentThreadId,
              image.event_id,
              resolvedImage.dataUrl,
              resolvedImage.path
            );
            addGeneratedImage(
              resolvedImage.dataUrl,
              resolvedImage.path,
              image.event_id,
              false
            );
          }
        }
        if (turn.response) {
          addMessage('assistant', turn.response);
        }
      }
      container.scrollTop = container.scrollHeight;
      // Show welcome card when history is empty
      if (data.turns.length === 0) {
        showWelcomeCard();
      }
      // Show processing indicator if the last turn is still in-progress
      var lastTurn = data.turns.length > 0 ? data.turns[data.turns.length - 1] : null;
      if (lastTurn && !lastTurn.response && lastTurn.state === 'Processing') {
        showActivityThinking('Processing...');
      }
      if (data.pending_gate) {
        handleGateRequired({
          ...data.pending_gate,
          thread_id: data.pending_gate.thread_id || currentThreadId,
        });
      } else {
        // No pending gate for this history view. Keep a global auth overlay if
        // it belongs to a different thread; another tab/thread may still be
        // waiting on it.
        const overlay = getAuthOverlay();
        if (overlay) {
          const overlayThreadId = overlay.getAttribute('data-thread-id');
          if (overlayThreadId && overlayThreadId !== currentThreadId) {
            return;
          }
        }
        removeAuthCard();
        setAuthFlowPending(false);
      }
    } else {
      // Pagination: prepend older messages
      const savedHeight = container.scrollHeight;
      const fragment = document.createDocumentFragment();
      for (const turn of data.turns) {
        if (turn.user_input) {
          const userDiv = createMessageElement('user', turn.user_input);
          fragment.appendChild(userDiv);
        }
        if (turn.tool_calls && turn.tool_calls.length > 0) {
          fragment.appendChild(createToolCallsSummaryElement(turn.tool_calls));
        }
        if (turn.generated_images && turn.generated_images.length > 0) {
          for (const image of turn.generated_images) {
            const resolvedImage = resolveGeneratedImageForRender(currentThreadId, image);
            rememberGeneratedImage(
              currentThreadId,
              image.event_id,
              resolvedImage.dataUrl,
              resolvedImage.path
            );
            fragment.appendChild(
              createGeneratedImageElement(
                resolvedImage.dataUrl,
                resolvedImage.path,
                image.event_id
              )
            );
          }
        }
        if (turn.response) {
          const assistantDiv = createMessageElement('assistant', turn.response);
          fragment.appendChild(assistantDiv);
        }
      }
      container.insertBefore(fragment, container.firstChild);
      // Restore scroll position so the user doesn't jump
      container.scrollTop = container.scrollHeight - savedHeight;
    }

    hasMore = data.has_more || false;
    oldestTimestamp = data.oldest_timestamp || null;
  }).catch(() => {
    // No history or no active thread
  }).finally(() => {
    loadingOlder = false;
    removeScrollSpinner();
  });
}

// Create a message DOM element without appending it (for prepend operations)
function createMessageElement(role, content) {
  const div = document.createElement('div');
  div.className = 'message ' + role;

  const ts = document.createElement('span');
  ts.className = 'message-timestamp';
  ts.textContent = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  div.appendChild(ts);

  // Message content
  const contentEl = document.createElement('div');
  contentEl.className = 'message-content';
  if (role === 'user' || role === 'system') {
    contentEl.textContent = content;
  } else {
    div.setAttribute('data-raw', content);
    contentEl.innerHTML = renderMarkdown(content);
    // Upgrade structured data (JSON objects, etc.) into styled cards
    upgradeStructuredData(contentEl);
    // Syntax highlighting for code blocks
    if (typeof hljs !== 'undefined') {
      requestAnimationFrame(() => {
        contentEl.querySelectorAll('pre code').forEach(block => {
          hljs.highlightElement(block);
        });
      });
    }
  }
  div.appendChild(contentEl);

  if (role === 'assistant' || role === 'user') {
    div.classList.add('has-copy');
    div.setAttribute('data-copy-text', content);
    const copyBtn = document.createElement('button');
    copyBtn.className = 'message-copy-btn';
    copyBtn.type = 'button';
    copyBtn.setAttribute('aria-label', I18n.t('message.copy'));
    copyBtn.textContent = I18n.t('message.copy');
    copyBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      copyMessage(copyBtn);
    });
    div.appendChild(copyBtn);
  }

  return div;
}

function addToolCallsSummary(toolCalls) {
  const container = document.getElementById('chat-messages');
  container.appendChild(createToolCallsSummaryElement(toolCalls));
  container.scrollTop = container.scrollHeight;
}

function createToolCallsSummaryElement(toolCalls) {
  const div = document.createElement('div');
  div.className = 'tool-calls-summary';

  const header = document.createElement('div');
  header.className = 'tool-calls-header';
  header.textContent = toolCalls.length + ' tool' + (toolCalls.length !== 1 ? 's' : '') + ' used';
  div.appendChild(header);

  const list = document.createElement('div');
  list.className = 'tool-calls-list';

  for (const tc of toolCalls) {
    const item = document.createElement('div');
    item.className = 'tool-call-item' + (tc.has_error ? ' tool-error' : '');

    const icon = tc.has_error ? '\u2717' : '\u2713';
    const nameSpan = document.createElement('span');
    nameSpan.className = 'tool-call-name';
    nameSpan.textContent = icon + ' ' + tc.name;
    item.appendChild(nameSpan);

    if (tc.result_preview) {
      const preview = document.createElement('div');
      preview.className = 'tool-call-preview';
      preview.textContent = tc.result_preview;
      item.appendChild(preview);
    }
    if (tc.error) {
      const errDiv = document.createElement('div');
      errDiv.className = 'tool-call-error-text';
      errDiv.textContent = tc.error;
      item.appendChild(errDiv);
    }

    list.appendChild(item);
  }

  div.appendChild(list);

  header.style.cursor = 'pointer';
  header.addEventListener('click', () => {
    list.classList.toggle('expanded');
    header.classList.toggle('expanded');
  });

  return div;
}

function removeScrollSpinner() {
  const spinner = document.getElementById('scroll-load-spinner');
  if (spinner) spinner.remove();
}

// --- Threads ---

function threadTitle(thread) {
  if (thread.title) return thread.title;
  const ch = thread.channel || 'gateway';
  if (thread.thread_type === 'heartbeat') return I18n.t('thread.heartbeatAlerts');
  if (thread.thread_type === 'routine') return I18n.t('thread.routine');
  if (ch !== 'gateway') return ch.charAt(0).toUpperCase() + ch.slice(1);
  if (thread.turn_count === 0) return 'New chat';
  return thread.id.substring(0, 8);
}

function relativeTime(isoStr) {
  if (!isoStr) return '';
  const diff = Date.now() - new Date(isoStr).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'now';
  if (mins < 60) return mins + 'm ago';
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return hrs + 'h ago';
  const days = Math.floor(hrs / 24);
  return days + 'd ago';
}

function isReadOnlyChannel(channel) {
  return channel && channel !== 'gateway' && channel !== 'routine' && channel !== 'heartbeat';
}

function debouncedLoadThreads() {
  if (_loadThreadsTimer) clearTimeout(_loadThreadsTimer);
  _loadThreadsTimer = setTimeout(() => { _loadThreadsTimer = null; loadThreads(); }, 500);
}

function loadThreads() {
  // Show skeleton while loading
  const threadListEl = document.getElementById('thread-list');
  if (threadListEl && threadListEl.children.length === 0) {
    threadListEl.innerHTML = '';
    threadListEl.appendChild(renderSkeleton('row', 4));
  }

  apiFetch('/api/chat/threads').then((data) => {
    // Pinned assistant thread
    if (data.assistant_thread) {
      assistantThreadId = data.assistant_thread.id;
      const el = document.getElementById('assistant-thread');
      const isActive = currentThreadId === assistantThreadId;
      el.className = 'assistant-item' + (isActive ? ' active' : '');
      const labelEl = document.getElementById('assistant-label');
      if (labelEl) {
        const at = data.assistant_thread;
        labelEl.textContent = I18n.t('thread.assistant');
      }
      const meta = document.getElementById('assistant-meta');
      meta.textContent = relativeTime(data.assistant_thread.updated_at);
    }

    // Regular threads
    const list = document.getElementById('thread-list');
    list.innerHTML = '';
    const threads = data.threads || [];
    for (const thread of threads) {
      const item = document.createElement('div');
      const isActive = thread.id === currentThreadId;
      item.className = 'thread-item' + (isActive ? ' active' : '');

      // Channel badge for non-gateway threads
      const ch = thread.channel || 'gateway';
      if (ch !== 'gateway') {
        const badge = document.createElement('span');
        badge.className = 'thread-badge thread-badge-' + ch;
        badge.textContent = ch;
        item.appendChild(badge);
      }

      const label = document.createElement('span');
      label.className = 'thread-label';
      label.textContent = threadTitle(thread);
      label.title = (thread.title || '') + ' (' + thread.id + ')';
      item.appendChild(label);

      const meta = document.createElement('span');
      meta.className = 'thread-meta';
      meta.textContent = relativeTime(thread.updated_at);
      item.appendChild(meta);

      // Unread dot
      const unread = unreadThreads.get(thread.id) || 0;
      if (unread > 0 && !isActive) {
        const dot = document.createElement('span');
        dot.className = 'thread-unread';
        dot.textContent = unread > 9 ? '9+' : String(unread);
        item.appendChild(dot);
      }

      item.addEventListener('click', () => switchThread(thread.id));
      list.appendChild(item);
    }

    // Restore thread from URL hash if pending (deferred from restoreFromHash)
    if (window._pendingThreadRestore) {
      var pendingId = window._pendingThreadRestore;
      window._pendingThreadRestore = null;
      // Verify the thread exists in the loaded list
      var found = (pendingId === assistantThreadId) ||
        threads.some(function(t) { return t.id === pendingId; });
      if (found) {
        switchThread(pendingId);
        return;
      }
    }

    // Reopen the server's active thread on first load. This keeps the visible
    // chat attached to an in-flight agent turn after a browser refresh, even
    // when the URL does not carry an explicit thread hash.
    if (!currentThreadId) {
      const activeThreadId = data.active_thread || null;
      if (activeThreadId && activeThreadId === assistantThreadId) {
        switchToAssistant();
        return;
      }
      if (activeThreadId && threads.some(t => t.id === activeThreadId)) {
        switchThread(activeThreadId);
        return;
      }
      if (assistantThreadId) {
        switchToAssistant();
        return;
      }
    }

    // Enable/disable chat input based on channel type
    if (currentThreadId) {
      const currentThread = currentThreadId === assistantThreadId
        ? data.assistant_thread
        : threads.find(t => t.id === currentThreadId);
      const ch = currentThread ? currentThread.channel : 'gateway';
      currentThreadIsReadOnly = isReadOnlyChannel(ch);
      if (currentThreadIsReadOnly) {
        disableChatInputReadOnly();
      } else {
        enableChatInput();
      }
    }
  }).catch(() => {});
}

function disableChatInputReadOnly() {
  const input = document.getElementById('chat-input');
  const btn = document.getElementById('send-btn');
  if (input) {
    input.disabled = true;
    input.placeholder = I18n.t('chat.readOnlyThread');
  }
  if (btn) btn.disabled = true;
}

function switchToAssistant() {
  if (!assistantThreadId) return;
  finalizeActivityGroup();
  currentThreadId = assistantThreadId;
  currentThreadIsReadOnly = false;
  unreadThreads.delete(assistantThreadId);
  hasMore = false;
  oldestTimestamp = null;
  loadHistory();
  loadThreads();
  updateHash();
  if (window.innerWidth <= 768) {
    const sidebar = document.getElementById('thread-sidebar');
    sidebar.classList.remove('expanded-mobile');
    document.getElementById('thread-toggle-btn').innerHTML = '&raquo;';
  }
}

function switchThread(threadId) {
  clearSuggestionChips();
  finalizeActivityGroup();
  _turnResponseReceived = false;
  if (_doneWithoutResponseTimer) {
    clearTimeout(_doneWithoutResponseTimer);
    _doneWithoutResponseTimer = null;
  }
  currentThreadId = threadId;
  unreadThreads.delete(threadId);
  hasMore = false;
  oldestTimestamp = null;
  loadHistory();
  loadThreads();
  updateHash();
  if (window.innerWidth <= 768) {
    const sidebar = document.getElementById('thread-sidebar');
    sidebar.classList.remove('expanded-mobile');
    document.getElementById('thread-toggle-btn').innerHTML = '&raquo;';
  }
}

function createNewThread() {
  apiFetch('/api/chat/thread/new', { method: 'POST' }).then((data) => {
    currentThreadId = data.id || null;
    currentThreadIsReadOnly = false;
    document.getElementById('chat-messages').innerHTML = '';
    showWelcomeCard();
    enableChatInput();
    loadThreads();
    updateHash();
  }).catch((err) => {
    showToast(I18n.t('chat.threadCreateFailed', { message: err.message }), 'error');
  });
}

function toggleThreadSidebar() {
  const sidebar = document.getElementById('thread-sidebar');
  const isMobile = window.innerWidth <= 768;
  if (isMobile) {
    sidebar.classList.toggle('expanded-mobile');
  } else {
    sidebar.classList.toggle('collapsed');
  }
  const btn = document.getElementById('thread-toggle-btn');
  const isOpen = isMobile
    ? sidebar.classList.contains('expanded-mobile')
    : !sidebar.classList.contains('collapsed');
  btn.innerHTML = isOpen ? '&laquo;' : '&raquo;';
}

// Chat input auto-resize and keyboard handling
const chatInput = document.getElementById('chat-input');
chatInput.addEventListener('keydown', (e) => {
  const acEl = document.getElementById('slash-autocomplete');
  const acVisible = acEl && acEl.style.display !== 'none';

  // Accept first suggestion with Tab (plain Tab only, not Shift+Tab)
  if (e.key === 'Tab' && !e.shiftKey && !acVisible && _ghostSuggestion && chatInput.value === '') {
    e.preventDefault();
    chatInput.value = _ghostSuggestion;
    clearSuggestionChips();
    autoResizeTextarea(chatInput);
    return;
  }

  if (acVisible) {
    const items = acEl.querySelectorAll('.slash-ac-item');
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      _slashSelected = Math.min(_slashSelected + 1, items.length - 1);
      updateSlashHighlight();
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      _slashSelected = Math.max(_slashSelected - 1, -1);
      updateSlashHighlight();
      return;
    }
    if (e.key === 'Tab' || e.key === 'Enter') {
      e.preventDefault();
      const pick = _slashSelected >= 0 ? _slashMatches[_slashSelected] : _slashMatches[0];
      if (pick) selectSlashItem(pick.cmd);
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      hideSlashAutocomplete();
      return;
    }
  }

  // Safari fires compositionend before keydown, so e.isComposing is already false
  // when Enter confirms IME input. keyCode 229 (VK_PROCESS) catches this case.
  // See https://bugs.webkit.org/show_bug.cgi?id=165004
  if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.keyCode !== 229) {
    e.preventDefault();
    hideSlashAutocomplete();
    sendMessage();
  }
});
chatInput.addEventListener('input', () => {
  autoResizeTextarea(chatInput);
  filterSlashCommands(chatInput.value);
  const ghost = document.getElementById('ghost-text');
  const wrapper = chatInput.closest('.chat-input-wrapper');
  if (chatInput.value !== '') {
    ghost.style.display = 'none';
    wrapper.classList.remove('has-ghost');
  } else if (_ghostSuggestion) {
    ghost.textContent = _ghostSuggestion;
    ghost.style.display = 'block';
    wrapper.classList.add('has-ghost');
  }
  const sendBtn = document.getElementById('send-btn');
  if (sendBtn) {
    sendBtn.classList.toggle('active', chatInput.value.trim().length > 0);
  }
});
chatInput.addEventListener('blur', () => {
  // Small delay so mousedown on autocomplete item fires first
  setTimeout(hideSlashAutocomplete, 150);
});

// Infinite scroll: load older messages when scrolled near the top.
// Also toggles the scroll-to-bottom button when the user has scrolled up.
// The handler is rAF-throttled so rapid scroll events coalesce into at most
// one layout read per frame.
let _scrollRafPending = false;
document.getElementById('chat-messages').addEventListener('scroll', function () {
  const container = this;
  if (container.scrollTop < 100 && hasMore && !loadingOlder) {
    loadingOlder = true;
    // Show spinner at top
    const spinner = document.createElement('div');
    spinner.id = 'scroll-load-spinner';
    spinner.className = 'scroll-load-spinner';
    spinner.innerHTML = '<div class="spinner"></div> Loading older messages...';
    container.insertBefore(spinner, container.firstChild);
    loadHistory(oldestTimestamp);
  }
  if (_scrollRafPending) return;
  _scrollRafPending = true;
  requestAnimationFrame(() => {
    _scrollRafPending = false;
    const btn = document.getElementById('scroll-to-bottom-btn');
    if (!btn) return;
    const distanceFromBottom = container.scrollHeight - container.scrollTop - container.clientHeight;
    btn.style.display = distanceFromBottom > 200 ? 'flex' : 'none';
  });
});

document.getElementById('scroll-to-bottom-btn').addEventListener('click', () => {
  const container = document.getElementById('chat-messages');
  container.scrollTo({ top: container.scrollHeight, behavior: 'smooth' });
});

// Keep the scroll-to-bottom button anchored just above the chat input,
// even when the textarea grows to multiple lines.
(() => {
  const input = document.querySelector('.chat-container .chat-input');
  const container = document.querySelector('.chat-container');
  if (!input || !container || typeof ResizeObserver === 'undefined') return;
  const ro = new ResizeObserver((entries) => {
    for (const entry of entries) {
      const h = entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height;
      container.style.setProperty('--chat-input-height', `${Math.ceil(h)}px`);
    }
  });
  ro.observe(input);
})();

function autoResizeTextarea(el) {
  const prev = el.offsetHeight;
  el.style.height = 'auto';
  const target = Math.min(el.scrollHeight, 120);
  el.style.height = prev + 'px';
  requestAnimationFrame(() => {
    el.style.height = target + 'px';
  });
}

// --- Tabs ---

document.querySelectorAll('.tab-bar button[data-tab]').forEach((btn) => {
  btn.addEventListener('click', () => {
    const tab = btn.getAttribute('data-tab');
    switchTab(tab);
  });
});

function switchTab(tab) {
  currentTab = tab;
  // NOTE: this function takes a `tab` argument that may originate from
  // workspace-supplied `layout.tabs.default_tab`, so it must NOT be
  // refactored into a `querySelector('[data-tab="' + tab + '"]')`
  // shape. The current form does string equality on the
  // `getAttribute('data-tab')` value of every button (the loop below)
  // and on `p.id === 'tab-' + tab` for the panel — neither path
  // interpolates `tab` into a CSS selector, so a hostile id can't
  // alter the selector match. If a future change needs to look up a
  // single button by id directly, wrap `tab` in `CSS.escape()` first.
  document.querySelectorAll('.tab-bar button[data-tab]').forEach((b) => {
    b.classList.toggle('active', b.getAttribute('data-tab') === tab);
  });
  document.querySelectorAll('.tab-panel').forEach((p) => {
    p.classList.toggle('active', p.id === 'tab-' + tab);
  });
  applyAriaAttributes();

  if (tab === 'memory') {
    loadMemoryTree();
    // Auto-open README.md on first visit (no file selected yet)
    if (!currentMemoryPath) readMemoryFile('README.md');
  }
  if (tab === 'jobs') loadJobs();
  if (tab === 'missions') loadMissions();
  if (tab === 'routines') loadRoutines();
  if (tab === 'logs') { connectLogSSE(); applyLogFilters(); }
  else if (logEventSource) { logEventSource.close(); logEventSource = null; }
  if (tab === 'settings') {
    loadSettingsSubtab(currentSettingsSubtab);
  } else {
    stopPairingPoll();
  }
  updateTabIndicator();
  updateHash();
}

function updateTabIndicator() {
  const indicator = document.getElementById('tab-indicator');
  if (!indicator) return;
  const activeBtn = document.querySelector('.tab-bar button[data-tab].active');
  if (!activeBtn) {
    indicator.style.width = '0';
    return;
  }
  const bar = activeBtn.closest('.tab-bar');
  const barRect = bar.getBoundingClientRect();
  const btnRect = activeBtn.getBoundingClientRect();
  indicator.style.left = (btnRect.left - barRect.left) + 'px';
  indicator.style.width = btnRect.width + 'px';
}

window.addEventListener('resize', updateTabIndicator);

// --- Memory (filesystem tree) ---

let memorySearchTimeout = null;
let currentMemoryPath = null;
let currentMemoryContent = null;
// Tree state: nested nodes persisted across renders
// { name, path, is_dir, children: [] | null, expanded: bool, loaded: bool }
let memoryTreeState = null;

document.getElementById('memory-search').addEventListener('input', (e) => {
  clearTimeout(memorySearchTimeout);
  const query = e.target.value.trim();
  if (!query) {
    loadMemoryTree();
    return;
  }
  memorySearchTimeout = setTimeout(() => searchMemory(query), 300);
});

function loadMemoryTree() {
  // Only load top-level on first load (or refresh)
  apiFetch('/api/memory/list?path=').then((data) => {
    memoryTreeState = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    renderTree();
  }).catch(() => {});
}

function renderTree() {
  const container = document.getElementById('memory-tree');
  container.innerHTML = '';
  if (!memoryTreeState || memoryTreeState.length === 0) {
    container.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No files in workspace</div>';
    return;
  }
  renderNodes(memoryTreeState, container, 0);
}

function renderNodes(nodes, container, depth) {
  for (const node of nodes) {
    const row = document.createElement('div');
    row.className = 'tree-row';
    row.style.paddingLeft = (depth * 16 + 8) + 'px';
    row.tabIndex = 0;
    row.setAttribute('role', 'treeitem');

    if (node.is_dir) {
      row.setAttribute('aria-expanded', node.expanded ? 'true' : 'false');
      const arrow = document.createElement('span');
      arrow.className = 'expand-arrow' + (node.expanded ? ' expanded' : '');
      arrow.textContent = '\u25B6';
      row.appendChild(arrow);

      const label = document.createElement('span');
      label.className = 'tree-label dir';
      label.textContent = node.name;
      row.appendChild(label);

      row.addEventListener('click', () => toggleExpand(node));
      row.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleExpand(node); }
      });
    } else {
      const spacer = document.createElement('span');
      spacer.className = 'expand-arrow-spacer';
      row.appendChild(spacer);

      const label = document.createElement('span');
      label.className = 'tree-label file';
      label.textContent = node.name;
      row.appendChild(label);

      row.addEventListener('click', () => readMemoryFile(node.path));
      row.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); readMemoryFile(node.path); }
      });
    }

    container.appendChild(row);

    if (node.is_dir && node.expanded && node.children) {
      const childContainer = document.createElement('div');
      childContainer.className = 'tree-children';
      renderNodes(node.children, childContainer, depth + 1);
      container.appendChild(childContainer);
    }
  }
}

function toggleExpand(node) {
  if (node.expanded) {
    node.expanded = false;
    renderTree();
    return;
  }

  if (node.loaded) {
    node.expanded = true;
    renderTree();
    return;
  }

  // Lazy-load children
  apiFetch('/api/memory/list?path=' + encodeURIComponent(node.path)).then((data) => {
    node.children = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    node.loaded = true;
    node.expanded = true;
    renderTree();
  }).catch(() => {});
}

function readMemoryFile(path) {
  currentMemoryPath = path;
  updateHash();
  // Update breadcrumb
  document.getElementById('memory-breadcrumb-path').innerHTML = buildBreadcrumb(path);
  document.getElementById('memory-edit-btn').style.display = 'inline-block';

  // Exit edit mode if active
  cancelMemoryEdit();

  apiFetch('/api/memory/read?path=' + encodeURIComponent(path)).then((data) => {
    currentMemoryContent = data.content;
    const viewer = document.getElementById('memory-viewer');
    // Render markdown if it's a .md file
    if (path.endsWith('.md')) {
      viewer.innerHTML = '<div class="memory-rendered">' + renderMarkdown(data.content) + '</div>';
      viewer.classList.add('rendered');
    } else {
      viewer.textContent = data.content;
      viewer.classList.remove('rendered');
    }
  }).catch((err) => {
    currentMemoryContent = null;
    document.getElementById('memory-viewer').innerHTML = '<div class="empty">Error: ' + escapeHtml(err.message) + '</div>';
  });
}

function startMemoryEdit() {
  if (!currentMemoryPath || currentMemoryContent === null) return;
  document.getElementById('memory-viewer').style.display = 'none';
  const editor = document.getElementById('memory-editor');
  editor.style.display = 'flex';
  const textarea = document.getElementById('memory-edit-textarea');
  textarea.value = currentMemoryContent;
  textarea.focus();
}

function cancelMemoryEdit() {
  document.getElementById('memory-viewer').style.display = '';
  document.getElementById('memory-editor').style.display = 'none';
}

function saveMemoryEdit() {
  if (!currentMemoryPath) return;
  const content = document.getElementById('memory-edit-textarea').value;
  apiFetch('/api/memory/write', {
    method: 'POST',
    body: { path: currentMemoryPath, content: content },
  }).then(() => {
    showToast(I18n.t('memory.savedPath', { path: currentMemoryPath }), 'success');
    cancelMemoryEdit();
    readMemoryFile(currentMemoryPath);
  }).catch((err) => {
    showToast(I18n.t('memory.saveFailed', { message: err.message }), 'error');
  });
}

function buildBreadcrumb(path) {
  const parts = path.split('/');
  let html = '<a data-action="breadcrumb-root" href="#">workspace</a>';
  let current = '';
  for (const part of parts) {
    current += (current ? '/' : '') + part;
    html += ' / <a data-action="breadcrumb-file" data-path="' + escapeHtml(current) + '" href="#">' + escapeHtml(part) + '</a>';
  }
  return html;
}

function searchMemory(query) {
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) return;

  apiFetch('/api/memory/search', {
    method: 'POST',
    body: { query: normalizedQuery, limit: 20 },
  }).then((data) => {
    const tree = document.getElementById('memory-tree');
    tree.innerHTML = '';
    if (data.results.length === 0) {
      tree.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No results</div>';
      return;
    }
    for (const result of data.results) {
      const item = document.createElement('div');
      item.className = 'search-result';
      const snippet = snippetAround(result.content, normalizedQuery, 120);
      item.innerHTML = '<div class="path">' + escapeHtml(result.path) + '</div>'
        + '<div class="snippet">' + highlightQuery(snippet, normalizedQuery) + '</div>';
      item.addEventListener('click', () => readMemoryFile(result.path));
      tree.appendChild(item);
    }
  }).catch(() => {});
}

function normalizeSearchQuery(query) {
  return (typeof query === 'string' ? query : '').slice(0, MEMORY_SEARCH_QUERY_MAX_LENGTH);
}

function snippetAround(text, query, len) {
  const normalizedQuery = normalizeSearchQuery(query);
  const lower = text.toLowerCase();
  const idx = lower.indexOf(normalizedQuery.toLowerCase());
  if (idx < 0) return text.substring(0, len);
  const start = Math.max(0, idx - Math.floor(len / 2));
  const end = Math.min(text.length, start + len);
  let s = text.substring(start, end);
  if (start > 0) s = '...' + s;
  if (end < text.length) s = s + '...';
  return s;
}

function highlightQuery(text, query) {
  if (!query) return escapeHtml(text);
  const escaped = escapeHtml(text);
  const normalizedQuery = normalizeSearchQuery(query);
  const queryEscaped = normalizedQuery.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const re = new RegExp('(' + queryEscaped + ')', 'gi');
  return escaped.replace(re, '<mark>$1</mark>');
}
// --- Logs ---

const LOG_MAX_ENTRIES = 2000;
let logsPaused = false;
let logBuffer = []; // buffer while paused

function connectLogSSE() {
  if (logEventSource) logEventSource.close();

  const logSseUrl = (token && !oidcProxyAuth)
    ? '/api/logs/events?token=' + encodeURIComponent(token)
    : '/api/logs/events';
  logEventSource = new EventSource(logSseUrl);

  logEventSource.addEventListener('log', (e) => {
    const entry = JSON.parse(e.data);
    if (logsPaused) {
      logBuffer.push(entry);
      return;
    }
    prependLogEntry(entry);
  });

  logEventSource.onerror = () => {
    // Silent reconnect
  };
}

function prependLogEntry(entry) {
  const output = document.getElementById('logs-output');

  // Level filter
  const levelFilter = document.getElementById('logs-level-filter').value;
  const targetFilter = document.getElementById('logs-target-filter').value.trim().toLowerCase();

  const div = document.createElement('div');
  div.className = 'log-entry level-' + entry.level;
  div.setAttribute('data-level', entry.level);
  div.setAttribute('data-target', entry.target);

  const ts = document.createElement('span');
  ts.className = 'log-ts';
  ts.textContent = entry.timestamp.substring(11, 23);
  div.appendChild(ts);

  const lvl = document.createElement('span');
  lvl.className = 'log-level';
  lvl.textContent = entry.level.padEnd(5);
  div.appendChild(lvl);

  const tgt = document.createElement('span');
  tgt.className = 'log-target';
  tgt.textContent = entry.target;
  div.appendChild(tgt);

  const msg = document.createElement('span');
  msg.className = 'log-msg';
  msg.textContent = entry.message;
  div.appendChild(msg);

  div.addEventListener('click', () => div.classList.toggle('expanded'));

  // Apply current filters as visibility
  const matchesLevel = levelFilter === 'all' || entry.level === levelFilter;
  const matchesTarget = !targetFilter || entry.target.toLowerCase().includes(targetFilter);
  if (!matchesLevel || !matchesTarget) {
    div.style.display = 'none';
  }

  output.prepend(div);

  // Cap entries (remove oldest at the bottom)
  while (output.children.length > LOG_MAX_ENTRIES) {
    output.removeChild(output.lastChild);
  }

  // Auto-scroll to top (newest entries are at the top)
  if (document.getElementById('logs-autoscroll').checked) {
    output.scrollTop = 0;
  }
}

function toggleLogsPause() {
  logsPaused = !logsPaused;
  const btn = document.getElementById('logs-pause-btn');
  btn.textContent = logsPaused ? I18n.t('logs.resume') : I18n.t('logs.pause');

  if (!logsPaused) {
    // Flush buffer: oldest-first + prepend naturally puts newest at top
    for (const entry of logBuffer) {
      prependLogEntry(entry);
    }
    logBuffer = [];
  }
}

function clearLogs() {
  if (!confirm(I18n.t('logs.confirmClear'))) return;
  document.getElementById('logs-output').innerHTML = '';
  logBuffer = [];
}

// Re-apply filters when level or target changes
document.getElementById('logs-level-filter').addEventListener('change', applyLogFilters);
document.getElementById('logs-target-filter').addEventListener('input', applyLogFilters);

function applyLogFilters() {
  const levelFilter = document.getElementById('logs-level-filter').value;
  const targetFilter = document.getElementById('logs-target-filter').value.trim().toLowerCase();
  const entries = document.querySelectorAll('#logs-output .log-entry');
  for (const el of entries) {
    const matchesLevel = levelFilter === 'all' || el.getAttribute('data-level') === levelFilter;
    const matchesTarget = !targetFilter || el.getAttribute('data-target').toLowerCase().includes(targetFilter);
    el.style.display = (matchesLevel && matchesTarget) ? '' : 'none';
  }
}

// --- Server-side log level control ---

function setServerLogLevel(level) {
  apiFetch('/api/logs/level', {
    method: 'PUT',
    body: { level },
  })
    .then(data => {
      document.getElementById('logs-server-level').value = data.level;
    })
    .catch(err => console.error('Failed to set server log level:', err));
}

function loadServerLogLevel() {
  apiFetch('/api/logs/level')
    .then(data => {
      document.getElementById('logs-server-level').value = data.level;
    })
    .catch(() => {}); // ignore if not available
}

// --- Extensions ---

var kindLabels = { 'wasm_channel': 'Channel', 'wasm_tool': 'Tool', 'mcp_server': 'MCP' };

function loadExtensions() {
  const extList = document.getElementById('extensions-list');
  const wasmList = document.getElementById('available-wasm-list');
  extList.innerHTML = renderCardsSkeleton(3);

  // Fetch extensions and registry in parallel
  Promise.all([
    apiFetch('/api/extensions').catch(() => ({ extensions: [] })),
    apiFetch('/api/extensions/registry').catch(function(err) { console.warn('registry fetch failed:', err); return { entries: [] }; }),
  ]).then(([extData, registryData]) => {
    // Render installed extensions (exclude wasm_channel and mcp_server — shown in their own tabs)
    var nonChannelExts = extData.extensions.filter(function(e) {
      return e.kind !== 'wasm_channel' && e.kind !== 'mcp_server';
    });
    if (nonChannelExts.length === 0) {
      extList.innerHTML = '<div class="empty-state">' + I18n.t('extensions.noInstalled') + '</div>';
    } else {
      extList.innerHTML = '';
      for (const ext of nonChannelExts) {
        extList.appendChild(renderExtensionCard(ext));
      }
    }

    // Available extensions (exclude MCP servers and channels — they have their own tabs)
    var wasmEntries = registryData.entries.filter(function(e) {
      return e.kind !== 'mcp_server' && e.kind !== 'wasm_channel' && e.kind !== 'channel' && !e.installed;
    });

    var wasmSection = document.getElementById('available-wasm-section');
    if (wasmEntries.length === 0) {
      if (wasmSection) wasmSection.style.display = 'none';
    } else {
      if (wasmSection) wasmSection.style.display = '';
      wasmList.innerHTML = '';
      for (const entry of wasmEntries) {
        wasmList.appendChild(renderAvailableExtensionCard(entry));
      }
    }

  });
}

function renderAvailableExtensionCard(entry) {
  const card = document.createElement('div');
  card.className = 'ext-card ext-available';

  const header = document.createElement('div');
  header.className = 'ext-header';

  const name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = entry.display_name;
  header.appendChild(name);

  const kind = document.createElement('span');
  kind.className = 'ext-kind kind-' + entry.kind;
  kind.textContent = kindLabels[entry.kind] || entry.kind;
  header.appendChild(kind);

  if (entry.version) {
    const ver = document.createElement('span');
    ver.className = 'ext-version';
    ver.textContent = 'v' + entry.version;
    header.appendChild(ver);
  }

  card.appendChild(header);

  const desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = entry.description;
  card.appendChild(desc);

  if (entry.keywords && entry.keywords.length > 0) {
    const kw = document.createElement('div');
    kw.className = 'ext-keywords';
    kw.textContent = entry.keywords.join(', ');
    card.appendChild(kw);
  }

  const actions = document.createElement('div');
  actions.className = 'ext-actions';

  const installBtn = document.createElement('button');
  installBtn.className = 'btn-ext install';
  installBtn.textContent = I18n.t('extensions.install');
  installBtn.addEventListener('click', function() {
    installBtn.disabled = true;
    installBtn.textContent = I18n.t('extensions.installing');
    apiFetch('/api/extensions/install', {
      method: 'POST',
      body: { name: entry.name, kind: entry.kind },
    }).then(function(res) {
      if (res.success) {
        showToast(I18n.t('extensions.installedSuccess', {name: entry.display_name}), 'success');
        // OAuth popup if auth started during install (builtin creds)
        if (res.auth_url) {
          showAuthCard({
            extension_name: entry.name,
            auth_url: res.auth_url,
          });
          showToast(I18n.t('extensions.openingAuth', { name: entry.display_name }), 'info');
          openOAuthUrl(res.auth_url);
        }
        refreshCurrentSettingsTab();
        // Auto-open configure for WASM channels
        if (entry.kind === 'wasm_channel') {
          showConfigureModal(entry.name);
        }
      } else {
        showToast(I18n.t('extensions.installFailed', { message: res.message || 'unknown error' }), 'error');
        refreshCurrentSettingsTab();
      }
    }).catch(function(err) {
      showToast(I18n.t('extensions.installFailed', { message: err.message }), 'error');
      refreshCurrentSettingsTab();
    });
  });
  actions.appendChild(installBtn);

  card.appendChild(actions);
  return card;
}

function renderMcpServerCard(entry, installedExt) {
  var card = document.createElement('div');
  card.className = 'ext-card' + (installedExt ? '' : ' ext-available');

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = entry.display_name;
  header.appendChild(name);

  var kind = document.createElement('span');
  kind.className = 'ext-kind kind-mcp_server';
  kind.textContent = kindLabels['mcp_server'] || 'mcp_server';
  header.appendChild(kind);

  if (installedExt) {
    var authDot = document.createElement('span');
    authDot.className = 'ext-auth-dot ' + (installedExt.authenticated ? 'authed' : 'unauthed');
    authDot.title = installedExt.authenticated ? I18n.t('auth.authenticated') : I18n.t('auth.notAuthenticated');
    header.appendChild(authDot);
  }

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = entry.description;
  card.appendChild(desc);

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  if (installedExt) {
    if (!installedExt.active) {
      var activateBtn = document.createElement('button');
      activateBtn.className = 'btn-ext activate';
      activateBtn.textContent = I18n.t('common.activate');
      activateBtn.addEventListener('click', function() { activateExtension(installedExt.name); });
      actions.appendChild(activateBtn);
    } else {
      var activeLabel = document.createElement('span');
      activeLabel.className = 'ext-active-label';
      activeLabel.textContent = I18n.t('ext.active');
      actions.appendChild(activeLabel);
    }
    if (installedExt.needs_setup || (installedExt.has_auth && installedExt.authenticated)) {
      var configBtn = document.createElement('button');
      configBtn.className = 'btn-ext configure';
      configBtn.textContent = installedExt.authenticated ? I18n.t('ext.reconfigure') : I18n.t('ext.configure');
      configBtn.addEventListener('click', function() { showConfigureModal(installedExt.name); });
      actions.appendChild(configBtn);
    }
    var removeBtn = document.createElement('button');
    removeBtn.className = 'btn-ext remove';
    removeBtn.textContent = I18n.t('ext.remove');
    removeBtn.addEventListener('click', function() { removeExtension(installedExt.name); });
    actions.appendChild(removeBtn);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-ext install';
    installBtn.textContent = I18n.t('ext.install');
    installBtn.addEventListener('click', function() {
      installBtn.disabled = true;
      installBtn.textContent = I18n.t('ext.installing');
      apiFetch('/api/extensions/install', {
        method: 'POST',
        body: { name: entry.name, kind: entry.kind },
      }).then(function(res) {
        if (res.success) {
          showToast(I18n.t('extensions.installedSuccess', { name: entry.display_name }), 'success');
        } else {
          showToast(I18n.t('ext.install') + ': ' + (res.message || 'unknown error'), 'error');
        }
        loadMcpServers();
      }).catch(function(err) {
        showToast(I18n.t('ext.installFailed', { message: err.message }), 'error');
        loadMcpServers();
      });
    });
    actions.appendChild(installBtn);
  }

  card.appendChild(actions);
  return card;
}

function createReconfigureButton(extName) {
  var btn = document.createElement('button');
  btn.className = 'btn-ext configure';
  btn.textContent = I18n.t('ext.reconfigure');
  btn.addEventListener('click', function() { showConfigureModal(extName); });
  return btn;
}

function renderExtensionCard(ext) {
  const card = document.createElement('div');
  var stateClass = 'state-inactive';
  if (ext.kind === 'wasm_channel') {
    var s = ext.onboarding_state || ext.activation_status || 'installed';
    if (s === 'active') stateClass = 'state-active';
    else if (s === 'ready') stateClass = 'state-active';
    else if (s === 'failed') stateClass = 'state-error';
    else if (s === 'pairing') stateClass = 'state-pairing';
    else if (s === 'pairing_required') stateClass = 'state-pairing';
  } else if (ext.active) {
    stateClass = 'state-active';
  }
  card.className = 'ext-card ' + stateClass;

  const header = document.createElement('div');
  header.className = 'ext-header';

  const name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = ext.display_name || ext.name;
  header.appendChild(name);

  const kind = document.createElement('span');
  kind.className = 'ext-kind kind-' + ext.kind;
  kind.textContent = kindLabels[ext.kind] || ext.kind;
  header.appendChild(kind);

  if (ext.version) {
    const ver = document.createElement('span');
    ver.className = 'ext-version';
    ver.textContent = 'v' + ext.version;
    header.appendChild(ver);
  }

  // Auth dot only for non-WASM-channel extensions (channels use the stepper instead)
  if (ext.kind !== 'wasm_channel') {
    const authDot = document.createElement('span');
    authDot.className = 'ext-auth-dot ' + (ext.authenticated ? 'authed' : 'unauthed');
    authDot.title = ext.authenticated ? I18n.t('auth.authenticated') : I18n.t('auth.notAuthenticated');
    header.appendChild(authDot);
  }

  card.appendChild(header);

  // WASM channels get a progress stepper
  if (ext.kind === 'wasm_channel') {
    card.appendChild(renderWasmChannelStepper(ext));
  }

  if (ext.description) {
    const desc = document.createElement('div');
    desc.className = 'ext-desc';
    desc.textContent = ext.description;
    card.appendChild(desc);
  }

  if (ext.url) {
    const url = document.createElement('div');
    url.className = 'ext-url';
    url.textContent = ext.url;
    url.title = ext.url;
    card.appendChild(url);
  }

  if (ext.tools && ext.tools.length > 0) {
    const tools = document.createElement('div');
    tools.className = 'ext-tools';
    tools.textContent = I18n.t('extensions.toolsLabel', { list: ext.tools.join(', ') });
    card.appendChild(tools);
  }

  // Show activation error for WASM channels
  if (ext.kind === 'wasm_channel' && ext.activation_error) {
    const errorDiv = document.createElement('div');
    errorDiv.className = 'ext-error';
    errorDiv.textContent = ext.activation_error;
    card.appendChild(errorDiv);
  }


  const actions = document.createElement('div');
  actions.className = 'ext-actions';

  if (ext.kind === 'wasm_channel') {
    // WASM channels: state-based buttons (no generic Activate)
    var status = ext.onboarding_state || ext.activation_status || 'installed';
    if (status === 'active' || status === 'ready') {
      var activeLabel = document.createElement('span');
      activeLabel.className = 'ext-active-label';
      activeLabel.textContent = I18n.t('ext.active');
      actions.appendChild(activeLabel);
      actions.appendChild(createReconfigureButton(ext.name));
    } else if (status === 'pairing' || status === 'pairing_required') {
      var pairingLabel = document.createElement('span');
      pairingLabel.className = 'ext-pairing-label';
      pairingLabel.textContent = I18n.t('status.awaitingPairing');
      actions.appendChild(pairingLabel);
      actions.appendChild(createReconfigureButton(ext.name));
    } else if (status === 'failed') {
      actions.appendChild(createReconfigureButton(ext.name));
    } else {
      var reconfigureBtn = document.createElement('button');
      reconfigureBtn.className = 'btn-ext configure';
      reconfigureBtn.textContent = I18n.t('extensions.reconfigure');
      reconfigureBtn.addEventListener('click', function() { showConfigureModal(ext.name); });
      actions.appendChild(reconfigureBtn);
    }
  } else {
    // WASM tools / MCP servers
    const activeLabel = document.createElement('span');
    activeLabel.className = 'ext-active-label';
    activeLabel.textContent = ext.active ? I18n.t('ext.active') : I18n.t('status.installed');
    actions.appendChild(activeLabel);

    // MCP servers and channel-relay extensions may be installed but inactive — show Activate button
    if ((ext.kind === 'mcp_server' || ext.kind === 'channel_relay') && !ext.active) {
      const activateBtn = document.createElement('button');
      activateBtn.className = 'btn-ext activate';
      activateBtn.textContent = I18n.t('common.activate');
      activateBtn.addEventListener('click', () => activateExtension(ext.name));
      actions.appendChild(activateBtn);
    }

    // Show Configure/Reconfigure button when there are secrets to enter.
    // Skip when has_auth is true but needs_setup is false and not yet authenticated —
    // this means OAuth credentials resolve automatically (builtin/env) and the user
    // just needs to complete the OAuth flow, not fill in a config form.
    if (ext.needs_setup || (ext.has_auth && ext.authenticated)) {
      const configBtn = document.createElement('button');
      configBtn.className = 'btn-ext configure';
      configBtn.textContent = ext.authenticated ? I18n.t('ext.reconfigure') : I18n.t('ext.configure');
      configBtn.addEventListener('click', () => showConfigureModal(ext.name));
      actions.appendChild(configBtn);
    }
  }

  const removeBtn = document.createElement('button');
  removeBtn.className = 'btn-ext remove';
  removeBtn.textContent = I18n.t('ext.remove');
  removeBtn.addEventListener('click', () => removeExtension(ext.name));
  actions.appendChild(removeBtn);

  card.appendChild(actions);

  // For WASM channels, check for pending pairing requests.
  if (ext.kind === 'wasm_channel') {
    if ((ext.onboarding_state || ext.activation_status || 'installed') === 'setup_required') {
      const setupSection = document.createElement('div');
      setupSection.className = 'ext-onboarding';
      card.appendChild(setupSection);
      loadInlineChannelSetup(ext, setupSection);
    }
    if ((ext.onboarding_state || ext.activation_status || 'installed') === 'pairing_required'
      || (ext.onboarding_state || ext.activation_status || 'installed') === 'pairing') {
      const pairingSection = document.createElement('div');
      pairingSection.className = 'ext-pairing';
      pairingSection.setAttribute('data-channel', ext.name);
      pairingSection.__onboarding = ext.onboarding || null;
      card.appendChild(pairingSection);
      if (currentUserIsAdmin()) {
        loadPairingRequests(ext.name, pairingSection, ext.onboarding || null);
      } else {
        renderMemberPairingClaim(ext, pairingSection, ext.onboarding || null);
      }
    }
  }

  return card;
}

function loadInlineChannelSetup(ext, container) {
  apiFetch('/api/extensions/' + encodeURIComponent(ext.name) + '/setup')
    .then((setup) => {
      const onboarding = setup.onboarding || ext.onboarding || {};
      const secrets = Array.isArray(setup.secrets) ? setup.secrets : [];
      if (secrets.length === 0) {
        container.innerHTML = '';
        return;
      }

      container.innerHTML = '';

      const title = document.createElement('div');
      title.className = 'ext-onboarding-title';
      title.textContent = onboarding.credential_title || ('Configure credentials for ' + (ext.display_name || ext.name));
      container.appendChild(title);

      if (onboarding.credential_instructions) {
        const text = document.createElement('div');
        text.className = 'ext-onboarding-text';
        text.textContent = onboarding.credential_instructions;
        container.appendChild(text);
      }

      if (onboarding.setup_url) {
        // Strict HTTPS validation via shared helper.
        const parsedSetupUrl2 = parseHttpsExternalUrl(onboarding.setup_url, 'setup');
        if (parsedSetupUrl2) {
          const links = document.createElement('div');
          links.className = 'auth-links';
          const link = document.createElement('a');
          link.href = parsedSetupUrl2.href;
          link.target = '_blank';
          link.rel = 'noopener noreferrer';
          link.textContent = I18n.t('authRequired.getToken');
          links.appendChild(link);
          container.appendChild(links);
        }
      }

      const form = document.createElement('div');
      form.className = 'setup-form inline';
      container.appendChild(form);

      let fields = [];
      const submit = () => submitInlineChannelSetup(ext.name, fields, container);
      fields = buildSetupFields(form, ext.name, secrets, submit);

      if (onboarding.credential_next_step) {
        const nextStep = document.createElement('div');
        nextStep.className = 'setup-next-step';
        nextStep.textContent = onboarding.credential_next_step;
        container.appendChild(nextStep);
      }

      const actions = document.createElement('div');
      actions.className = 'ext-actions';
      const submitBtn = document.createElement('button');
      submitBtn.className = 'btn-ext activate';
      submitBtn.textContent = I18n.t('config.save');
      submitBtn.addEventListener('click', submit);
      actions.appendChild(submitBtn);
      container.appendChild(actions);
    })
    .catch(() => {
      container.innerHTML = '';
    });
}

function submitInlineChannelSetup(name, fields, container) {
  const secrets = {};
  (fields || []).forEach((field) => {
    const value = (field.input.value || '').trim();
    if (value) secrets[field.name] = value;
  });

  const buttons = container.querySelectorAll('button');
  buttons.forEach((btn) => { btn.disabled = true; });

  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup', {
    method: 'POST',
    body: { secrets, fields: {} },
  }).then((res) => {
    if (!res.success) {
      showToast(res.message || 'Configuration failed', 'error');
      buttons.forEach((btn) => { btn.disabled = false; });
      return;
    }
    if (res.onboarding_state === 'pairing_required') {
      showPairingCard({
        channel: name,
        instructions: res.onboarding && res.onboarding.pairing_instructions,
        onboarding: res.onboarding || null,
      });
    }
    refreshCurrentSettingsTab();
  }).catch((err) => {
    buttons.forEach((btn) => { btn.disabled = false; });
    showToast(I18n.t('extensions.configFailed', { message: err.message }), 'error');
  });
}

function refreshCurrentSettingsTab() {
  if (currentSettingsSubtab === 'extensions') loadExtensions();
  if (currentSettingsSubtab === 'channels') loadChannelsStatus();
  if (currentSettingsSubtab === 'mcp') loadMcpServers();
}

function activateExtension(name) {
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/activate', { method: 'POST' })
    .then((res) => {
      if (res.success) {
        // Even on success, the tool may need OAuth (e.g., WASM loaded but no token yet)
        if (res.auth_url) {
          showAuthCard({
            extension_name: name,
            auth_url: res.auth_url,
          });
          showToast(I18n.t('extensions.openingAuth', { name: name }), 'info');
          openOAuthUrl(res.auth_url);
        }
        refreshCurrentSettingsTab();
        return;
      }

      if (res.auth_url) {
        showAuthCard({
          extension_name: name,
          auth_url: res.auth_url,
        });
        showToast(I18n.t('extensions.openingAuth', { name: name }), 'info');
        openOAuthUrl(res.auth_url);
      } else if (res.awaiting_token) {
        showConfigureModal(name);
      } else {
        showToast(I18n.t('extensions.activateFailed', { message: res.message }), 'error');
      }
      refreshCurrentSettingsTab();
    })
    .catch((err) => showToast(I18n.t('extensions.activateFailed', { message: err.message }), 'error'));
}

function removeExtension(name) {
  showConfirmModal(I18n.t('ext.confirmRemove', { name: name }), '', function() {
    apiFetch('/api/extensions/' + encodeURIComponent(name) + '/remove', { method: 'POST' })
      .then((res) => {
        if (!res.success) {
          showToast(I18n.t('ext.removeFailed', { message: res.message }), 'error');
        } else {
          showToast(I18n.t('ext.removed', { name: name }), 'success');
        }
        refreshCurrentSettingsTab();
      })
      .catch((err) => showToast(I18n.t('ext.removeFailed', { message: err.message }), 'error'));
  }, I18n.t('common.remove'), 'btn-danger');
}

function showConfigureModal(name, options) {
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup')
    .then((setup) => {
      const secrets = Array.isArray(setup.secrets) ? setup.secrets : [];
      const setupFields = Array.isArray(setup.fields) ? setup.fields : [];
      if (secrets.length === 0 && setupFields.length === 0) {
        if (options && options.authData) {
          showAuthCard(options.authData);
        } else {
          showToast(I18n.t('extensions.noConfigNeeded', { name: name }), 'info');
        }
        return;
      }
      renderConfigureModal(name, secrets, setupFields, setup.onboarding || null, options);
    })
    .catch((err) => {
      showToast(I18n.t('extensions.setupLoadFailed', { message: err.message }), 'error');
      if (options && options.authData) {
        showAuthCard(options.authData);
      }
    });
}

function renderConfigureModal(name, secrets, setupFields, onboarding, options) {
  // Cancel any existing auth-flow overlay before replacing it.
  // Remove directly (don't clear authFlowPending) since a new overlay is about to be appended.
  var existingOverlay = document.querySelector('.configure-overlay');
  if (existingOverlay && existingOverlay.getAttribute('data-auth-flow')) {
    var extName = existingOverlay.getAttribute('data-auth-extension') || existingOverlay.getAttribute('data-extension-name');
    apiFetch('/api/chat/auth-cancel', { method: 'POST', body: { extension_name: extName } }).catch(function() {});
    existingOverlay.remove();
  } else {
    closeConfigureModal();
  }
  const overlay = document.createElement('div');
  overlay.className = 'configure-overlay';
  overlay.setAttribute('data-extension-name', name);
  if (options && options.authData) {
    overlay.setAttribute('data-auth-flow', 'true');
    overlay.setAttribute('data-auth-extension', options.authData.extension_name || name);
    if (options.authData.request_id) overlay.setAttribute('data-request-id', options.authData.request_id);
    if (options.authData.thread_id) overlay.setAttribute('data-thread-id', options.authData.thread_id);
  }
  overlay.addEventListener('click', (e) => {
    if (e.target !== overlay) return;
    if (overlay.getAttribute('data-auth-flow')) {
      cancelAuthFromConfigureModal(overlay);
    } else {
      closeConfigureModal();
    }
  });

  const modal = document.createElement('div');
  modal.className = 'configure-modal';

  const header = document.createElement('h3');
  header.textContent = I18n.t('config.title', { name: name });
  modal.appendChild(header);

  if (onboarding && onboarding.credential_instructions) {
    const hint = document.createElement('div');
    hint.className = 'configure-hint';
    hint.textContent = onboarding.credential_instructions;
    modal.appendChild(hint);
  }

  const form = document.createElement('div');
  form.className = 'configure-form';

  const fields = [];
  for (const secret of secrets) {
    const field = document.createElement('div');
    field.className = 'configure-field';
    field.dataset.secretName = secret.name;

    const label = document.createElement('label');
    label.textContent = secret.prompt;
    if (secret.optional) {
      const opt = document.createElement('span');
      opt.className = 'field-optional';
      opt.textContent = I18n.t('config.optional');
      label.appendChild(opt);
    }
    field.appendChild(label);

    const inputRow = document.createElement('div');
    inputRow.className = 'configure-input-row';

    const input = document.createElement('input');
    input.type = 'password';
    input.name = secret.name;
    input.placeholder = secret.provided ? I18n.t('config.alreadySet') : '';
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submitConfigureModal(name, fields);
    });
    inputRow.appendChild(input);

    if (secret.provided) {
      const badge = document.createElement('span');
      badge.className = 'field-provided';
      badge.textContent = '\u2713';
      badge.title = I18n.t('config.alreadyConfigured');
      inputRow.appendChild(badge);
    }
    if (secret.auto_generate && !secret.provided) {
      const hint = document.createElement('span');
      hint.className = 'field-autogen';
      hint.textContent = I18n.t('config.autoGenerate');
      inputRow.appendChild(hint);
    }

    field.appendChild(inputRow);
    form.appendChild(field);
    fields.push({ kind: 'secret', name: secret.name, input: input });
  }

  for (const setupField of setupFields) {
    const field = document.createElement('div');
    field.className = 'configure-field';

    const label = document.createElement('label');
    label.textContent = setupField.prompt;
    if (setupField.optional) {
      const opt = document.createElement('span');
      opt.className = 'field-optional';
      opt.textContent = I18n.t('config.optional');
      label.appendChild(opt);
    }
    field.appendChild(label);

    const inputRow = document.createElement('div');
    inputRow.className = 'configure-input-row';

    const input = document.createElement('input');
    input.type = setupField.input_type === 'password' ? 'password' : 'text';
    input.name = setupField.name;
    input.placeholder = setupField.provided ? I18n.t('config.alreadySet') : '';
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submitConfigureModal(name, fields);
    });
    inputRow.appendChild(input);

    if (setupField.provided) {
      const badge = document.createElement('span');
      badge.className = 'field-provided';
      badge.textContent = '\u2713';
      badge.title = I18n.t('config.alreadyConfigured');
      inputRow.appendChild(badge);
    }

    field.appendChild(inputRow);
    form.appendChild(field);
    fields.push({ kind: 'field', name: setupField.name, input: input });
  }

  modal.appendChild(form);

  const error = document.createElement('div');
  error.className = 'configure-inline-error';
  error.style.display = 'none';
  modal.appendChild(error);

  const actions = document.createElement('div');
  actions.className = 'configure-actions';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'btn-ext activate';
  submitBtn.textContent = I18n.t('config.save');
  submitBtn.addEventListener('click', () => submitConfigureModal(name, fields));
  actions.appendChild(submitBtn);

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'btn-ext remove';
  cancelBtn.textContent = I18n.t('config.cancel');
  cancelBtn.addEventListener('click', function() {
    if (overlay.getAttribute('data-auth-flow')) {
      cancelAuthFromConfigureModal(overlay);
    } else {
      closeConfigureModal();
    }
  });
  actions.appendChild(cancelBtn);

  modal.appendChild(actions);
  overlay.appendChild(modal);
  document.body.appendChild(overlay);

  if (fields.length > 0) fields[0].input.focus();
}

function setConfigureInlineError(overlay, message) {
  const error = overlay && overlay.querySelector('.configure-inline-error');
  if (!error) return;
  error.textContent = message || '';
  error.style.display = message ? 'block' : 'none';
}

function clearConfigureInlineError(overlay) {
  setConfigureInlineError(overlay, '');
}

function submitConfigureModal(name, fields, options) {
  options = options || {};
  const secrets = {};
  const setupFields = {};
  for (const f of fields) {
    const value = f.input.value.trim();
    if (!value) {
      continue;
    }
    if (f.kind === 'secret') {
      secrets[f.name] = value;
    } else {
      setupFields[f.name] = value;
    }
  }

  const overlay = getConfigureOverlay(name) || document.querySelector('.configure-overlay');
  clearConfigureInlineError(overlay);

  // Disable buttons to prevent double-submit
  var btns = overlay ? overlay.querySelectorAll('.configure-actions button') : [];
  btns.forEach(function(b) { b.disabled = true; });

  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup', {
    method: 'POST',
    body: { secrets, fields: setupFields },
  })
    .then((res) => {
      if (res.success) {
        // Strip auth-flow flag before closing so closeConfigureModal
        // does not trigger a spurious auth-cancel API call.
        if (overlay) overlay.removeAttribute('data-auth-flow');
        closeConfigureModal();
        if (res.auth_url) {
          showAuthCard({
            extension_name: name,
            auth_url: res.auth_url,
          });
          showToast(I18n.t('extensions.openingOAuth', { name: name }), 'info');
          openOAuthUrl(res.auth_url);
          refreshCurrentSettingsTab();
        }
        // Transition to pairing if the channel requires it.
        if (res.onboarding_state === 'pairing_required') {
          showPairingCard({
            channel: name,
            instructions: res.onboarding && res.onboarding.pairing_instructions,
            onboarding: res.onboarding || null,
          });
          refreshCurrentSettingsTab();
        }
        // For non-OAuth success: the server always broadcasts auth_completed SSE,
        // which will show the toast and refresh extensions — no need to do it here too.
      } else {
        // Keep modal open so the user can correct their input and retry.
        btns.forEach(function(b) { b.disabled = false; });
        setConfigureInlineError(overlay, res.message || 'Configuration failed');
        showToast(res.message || 'Configuration failed', 'error');
      }
    })
    .catch((err) => {
      btns.forEach(function(b) { b.disabled = false; });
      setConfigureInlineError(overlay, 'Configuration failed: ' + err.message);
      showToast(I18n.t('extensions.configFailed', { message: err.message }), 'error');
    });
}

function closeConfigureModal(extensionName) {
  if (typeof extensionName !== 'string') extensionName = null;
  const existing = getConfigureOverlay(extensionName);
  if (existing) existing.remove();
  if (!document.querySelector('.configure-overlay') && !document.querySelector('.auth-card')) {
    setAuthFlowPending(false);
    enableChatInput();
  }
}

function cancelAuthFromConfigureModal(overlay) {
  var extName = overlay.getAttribute('data-auth-extension') || overlay.getAttribute('data-extension-name');
  var requestId = overlay.getAttribute('data-request-id');
  var threadId = overlay.getAttribute('data-thread-id');
  var request = requestId
    ? apiFetch('/api/chat/gate/resolve', { method: 'POST', body: { request_id: requestId, thread_id: threadId || currentThreadId || undefined, resolution: 'cancelled' } })
    : apiFetch('/api/chat/auth-cancel', { method: 'POST', body: { extension_name: extName, thread_id: threadId || currentThreadId || undefined } });
  request.catch(function() {});
  overlay.remove();
  if (!document.querySelector('.configure-overlay') && !document.querySelector('.auth-card')) {
    setAuthFlowPending(false);
    enableChatInput();
  }
}

function currentUserIsAdmin() {
  return !!(window._currentUser && window._currentUser.role === 'admin');
}

// Validate that a server-supplied OAuth URL is HTTPS before opening a popup.
// Rejects javascript:, data:, and other non-HTTPS schemes to prevent URL-injection.
// Uses the URL constructor to safely parse and validate the scheme, which also
// handles non-string values (objects, null, etc.) that would throw on .startsWith().
function parseHttpsExternalUrl(url, label) {
  let parsed;
  try {
    parsed = new URL(url);
    if (parsed.protocol !== 'https:') {
      throw new Error('non-HTTPS protocol: ' + parsed.protocol);
    }
  } catch (e) {
    console.warn(`Blocked invalid/non-HTTPS ${label} URL:`, url, e.message);
    showToast(I18n.t('extensions.invalidOAuthUrl'), 'error');
    return null;
  }
  return parsed;
}

function parseHttpsOAuthUrl(url) {
  return parseHttpsExternalUrl(url, 'OAuth');
}

function openOAuthUrl(url) {
  const parsed = parseHttpsOAuthUrl(url);
  if (!parsed) return;
  // `noopener,noreferrer` defends against tabnabbing — without these the
  // OAuth provider page can read `window.opener` and reach back into the
  // app tab. `noreferrer` also strips the Referer header.
  const opened = window.open(
    parsed.href,
    '_blank',
    'width=600,height=700,noopener,noreferrer',
  );
  // Some browsers ignore the noopener feature flag in window.open's third
  // argument when the window is non-null; explicitly null the opener as a
  // belt-and-suspenders defense.
  if (opened) {
    try {
      opened.opener = null;
    } catch (_) {
      /* opener may already be null in cross-origin contexts */
    }
  }
}

// --- Pairing ---

function loadPairingRequests(channel, container, onboarding) {
  if (!currentUserIsAdmin()) return;

  apiFetch('/api/pairing/' + encodeURIComponent(channel))
    .then(data => {
      container.innerHTML = '';

      const info = onboarding || {};

      const heading = document.createElement('div');
      heading.className = 'pairing-heading';
      heading.textContent = info.pairing_title || I18n.t('extensions.claimPairing');
      container.appendChild(heading);

      const help = document.createElement('div');
      help.className = 'pairing-help';
      help.textContent = info.pairing_instructions || I18n.t('extensions.claimPairingHelp');
      container.appendChild(help);

      const manual = document.createElement('div');
      manual.className = 'pairing-row pairing-manual';

      const input = document.createElement('input');
      input.className = 'pairing-manual-input';
      input.type = 'text';
      input.placeholder = I18n.t('extensions.pairingCodePlaceholder');
      input.autocomplete = 'off';
      input.spellcheck = false;
      input.autocapitalize = 'characters';
      input.maxLength = 64;
      input.addEventListener('keydown', function(event) {
        if (event.key === 'Enter') {
          event.preventDefault();
          approvePairing(channel, input.value, {
            onSuccess: function() {
              input.value = '';
              loadPairingRequests(channel, container, onboarding);
            }
          });
        }
      });
      manual.appendChild(input);

      const manualBtn = document.createElement('button');
      manualBtn.className = 'btn-ext activate pairing-manual-submit';
      manualBtn.textContent = I18n.t('approval.approve');
      manualBtn.addEventListener('click', function() {
        approvePairing(channel, input.value, {
          onSuccess: function() {
            input.value = '';
            loadPairingRequests(channel, container, onboarding);
          }
        });
      });
      manual.appendChild(manualBtn);
      container.appendChild(manual);

      if (info.restart_instructions) {
        const restart = document.createElement('div');
        restart.className = 'pairing-help pairing-restart';
        restart.textContent = info.restart_instructions;
        container.appendChild(restart);
      }

      if (!data.requests || data.requests.length === 0) return;

      const pendingHeading = document.createElement('div');
      pendingHeading.className = 'pairing-heading';
      pendingHeading.textContent = I18n.t('extensions.pendingPairing');
      container.appendChild(pendingHeading);

      data.requests.forEach(req => {
        const row = document.createElement('div');
        row.className = 'pairing-row';

        const code = document.createElement('span');
        code.className = 'pairing-code';
        code.textContent = req.code;
        row.appendChild(code);

        const sender = document.createElement('span');
        sender.className = 'pairing-sender';
        sender.textContent = I18n.t('extensions.from') + ' ' + req.sender_id;
        row.appendChild(sender);

        const btn = document.createElement('button');
        btn.className = 'btn-ext activate';
        btn.textContent = I18n.t('common.approve');
        btn.addEventListener('click', function() {
          approvePairing(channel, req.code, {
            onSuccess: function() {
              loadPairingRequests(channel, container, onboarding);
            }
          });
        });
        row.appendChild(btn);

        container.appendChild(row);
      });
    })
    .catch(() => {});
}

function renderMemberPairingClaim(ext, container, onboarding) {
  const info = onboarding || {};
  const heading = document.createElement('div');
  heading.className = 'pairing-heading';
  heading.textContent = info.pairing_title || I18n.t('extensions.claimPairing');
  container.appendChild(heading);

  const help = document.createElement('div');
  help.className = 'pairing-help';
  help.textContent = info.pairing_instructions || I18n.t('extensions.claimPairingHelp');
  container.appendChild(help);

  const row = document.createElement('div');
  row.className = 'pairing-row';

  const input = document.createElement('input');
  input.className = 'pairing-input';
  input.type = 'text';
  input.placeholder = I18n.t('extensions.pairingCodePlaceholder');
  input.autocomplete = 'off';
  input.spellcheck = false;
  input.maxLength = 64;
  row.appendChild(input);

  const btn = document.createElement('button');
  btn.className = 'btn-ext activate';
  btn.textContent = I18n.t('extensions.claimPairingAction');
  btn.addEventListener('click', function() {
    approvePairing(ext.name, input.value, {
      onSuccess: function() {
        input.value = '';
      }
    });
  });
  row.appendChild(btn);

  input.addEventListener('keydown', function(event) {
    if (event.key === 'Enter') {
      event.preventDefault();
      btn.click();
    }
  });

  container.appendChild(row);

  if (info.restart_instructions) {
    const restart = document.createElement('div');
    restart.className = 'pairing-help pairing-restart';
    restart.textContent = info.restart_instructions;
    container.appendChild(restart);
  }
}

function approvePairing(channel, code, options) {
  options = options || {};
  const normalizedCode = (code || '').trim().toUpperCase();
  if (!normalizedCode) {
    const message = I18n.t('extensions.pairingCodeRequired');
    if (typeof options.onError === 'function') {
      options.onError(message);
    } else {
      showToast(message, 'error');
    }
    return Promise.resolve();
  }

  return apiFetch('/api/pairing/' + encodeURIComponent(channel) + '/approve', {
    method: 'POST',
    body: { code: normalizedCode },
  }).then(res => {
    if (res.success) {
      _recentLocalPairingApprovals.set(channel, Date.now());
      if (!options.skipSuccessToast) {
        showToast(I18n.t('extensions.pairingApproved'), 'success');
      }
      if (typeof options.onSuccess === 'function') options.onSuccess(res);
      if (!options.skipRefresh && currentTab === 'settings') refreshCurrentSettingsTab();
    } else {
      const message = res.message || I18n.t('extensions.approveFailed');
      if (typeof options.onError === 'function') {
        options.onError(message);
      } else {
        showToast(message, 'error');
      }
    }
  }).catch(err => {
    const message = I18n.t('extensions.pairingError', { message: err.message });
    if (typeof options.onError === 'function') {
      options.onError(message);
    } else {
      showToast(message, 'error');
    }
  });
}

function startPairingPoll() {
  stopPairingPoll();
  pairingPollInterval = setInterval(function() {
    document.querySelectorAll('.ext-pairing[data-channel]').forEach(function(el) {
      loadPairingRequests(el.getAttribute('data-channel'), el, el.__onboarding || null);
    });
  }, 10000);
}

function stopPairingPoll() {
  if (pairingPollInterval) {
    clearInterval(pairingPollInterval);
    pairingPollInterval = null;
  }
}

// --- WASM channel stepper ---

function renderWasmChannelStepper(ext) {
  var stepper = document.createElement('div');
  stepper.className = 'ext-stepper';

  var status = ext.onboarding_state || ext.activation_status || 'installed';
  var requiresPairing = !!(ext.onboarding && ext.onboarding.requires_pairing);

  var steps = [
    { label: I18n.t('missions.stepConfigured'), key: 'setup_required' },
    { label: requiresPairing ? I18n.t('missions.stepAwaitingPairing') : I18n.t('extensions.activate'), key: 'pairing_required' },
    { label: I18n.t('missions.stepActive'), key: 'ready' },
  ];

  var reachedIdx;
  if (status === 'active' || status === 'ready') reachedIdx = 2;
  else if (status === 'pairing' || status === 'pairing_required') reachedIdx = 1;
  else if (status === 'failed') reachedIdx = 2;
  else if (status === 'configured' || status === 'activation_in_progress') reachedIdx = 1;
  else reachedIdx = 0;

  for (var i = 0; i < steps.length; i++) {
    if (i > 0) {
      var connector = document.createElement('div');
      connector.className = 'stepper-connector' + (i <= reachedIdx ? ' completed' : '');
      stepper.appendChild(connector);
    }

    var step = document.createElement('div');
    var stepState;
    if (i < reachedIdx) {
      stepState = 'completed';
    } else if (i === reachedIdx) {
      if (status === 'failed') {
        stepState = 'failed';
      } else if (status === 'pairing' || status === 'pairing_required' || status === 'activation_in_progress') {
        stepState = 'in-progress';
      } else if (status === 'setup_required') {
        stepState = 'in-progress';
      } else if (status === 'active' || status === 'ready' || status === 'configured' || status === 'installed') {
        stepState = 'completed';
      } else {
        stepState = 'pending';
      }
    } else {
      stepState = 'pending';
    }
    step.className = 'stepper-step ' + stepState;

    var circle = document.createElement('span');
    circle.className = 'stepper-circle';
    if (stepState === 'completed') circle.textContent = '\u2713';
    else if (stepState === 'failed') circle.textContent = '\u2717';
    step.appendChild(circle);

    var label = document.createElement('span');
    label.className = 'stepper-label';
    label.textContent = steps[i].label;
    step.appendChild(label);

    stepper.appendChild(step);
  }

  return stepper;
}

// --- Jobs ---

let currentJobId = null;
let currentJobSubTab = 'overview';
let jobFilesTreeState = null;

function loadJobs() {
  currentJobId = null;
  jobFilesTreeState = null;

  // Rebuild DOM if renderJobDetail() destroyed it (it wipes .jobs-container innerHTML).
  const container = document.querySelector('.jobs-container');
  if (!document.getElementById('jobs-summary')) {
    container.innerHTML =
      '<div class="jobs-summary" id="jobs-summary"></div>'
      + '<table class="jobs-table" id="jobs-table"><thead><tr>'
      + '<th>ID</th><th>Title</th><th>Status</th><th>Created</th><th>Actions</th>'
      + '</tr></thead><tbody id="jobs-tbody"></tbody></table>'
      + '<div class="empty-state" id="jobs-empty" style="display:none">No jobs found</div>';
  }

  Promise.all([
    apiFetch('/api/jobs/summary'),
    apiFetch('/api/jobs'),
  ]).then(([summary, jobList]) => {
    renderJobsSummary(summary);
    renderJobsList(jobList.jobs);
  }).catch(() => {});
}

function renderJobsSummary(s) {
  document.getElementById('jobs-summary').innerHTML = ''
    + summaryCard(I18n.t('jobs.summary.total'), s.total, '')
    + summaryCard(I18n.t('jobs.summary.inProgress'), s.in_progress, 'active')
    + summaryCard(I18n.t('jobs.summary.completed'), s.completed, 'completed')
    + summaryCard(I18n.t('jobs.summary.failed'), s.failed, 'failed')
    + summaryCard(I18n.t('jobs.summary.stuck'), s.stuck, 'stuck');
}

function summaryCard(label, count, cls) {
  return '<div class="summary-card ' + cls + '">'
    + '<div class="count">' + count + '</div>'
    + '<div class="label">' + label + '</div>'
    + '</div>';
}

function renderJobsList(jobs) {
  const tbody = document.getElementById('jobs-tbody');
  const empty = document.getElementById('jobs-empty');

  if (jobs.length === 0) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = jobs.map((job) => {
    const shortId = job.id.substring(0, 8);
    const stateClass = job.state.replace(' ', '_');

    let actionBtns = '';
    if (job.state === 'pending' || job.state === 'in_progress') {
      actionBtns = '<button class="btn-cancel" data-action="cancel-job" data-id="' + escapeHtml(job.id) + '">Cancel</button>';
    }
    // Retry is only shown in the detail view where can_restart is available.

    return '<tr class="job-row" data-action="open-job" data-id="' + escapeHtml(job.id) + '">'
      + '<td title="' + escapeHtml(job.id) + '">' + shortId + '</td>'
      + '<td>' + escapeHtml(job.title) + '</td>'
      + '<td><span class="badge ' + stateClass + '">' + escapeHtml(job.state) + '</span></td>'
      + '<td>' + formatDate(job.created_at) + '</td>'
      + '<td>' + actionBtns + '</td>'
      + '</tr>';
  }).join('');
}

function cancelJob(jobId) {
  if (!confirm(I18n.t('jobs.confirmCancel'))) return;
  apiFetch('/api/jobs/' + jobId + '/cancel', { method: 'POST' })
    .then(() => {
      showToast(I18n.t('jobs.cancelled'), 'success');
      if (currentJobId) openJobDetail(currentJobId);
      else loadJobs();
    })
    .catch((err) => {
      showToast(I18n.t('jobs.cancelFailed', { message: err.message }), 'error');
    });
}

function restartJob(jobId) {
  apiFetch('/api/jobs/' + jobId + '/restart', { method: 'POST' })
    .then((res) => {
      showToast(I18n.t('jobs.restarted', { id: (res.new_job_id || '').substring(0, 8) }), 'success');
    })
    .catch((err) => {
      showToast(I18n.t('jobs.restartFailed', { message: err.message }), 'error');
    })
    .finally(() => {
      loadJobs();
    });
}

function openJobDetail(jobId) {
  currentJobId = jobId;
  currentJobSubTab = 'activity';
  updateHash();
  apiFetch('/api/jobs/' + jobId).then((job) => {
    renderJobDetail(job);
  }).catch((err) => {
    addMessage('system', 'Failed to load job: ' + err.message);
    closeJobDetail();
  });
}

function closeJobDetail() {
  currentJobId = null;
  jobFilesTreeState = null;
  loadJobs();
  updateHash();
}

function renderJobDetail(job) {
  const container = document.querySelector('.jobs-container');
  const stateClass = job.state.replace(' ', '_');

  container.innerHTML = '';

  // Header
  const header = document.createElement('div');
  header.className = 'job-detail-header';

  let headerHtml = '<button class="btn-back" data-action="close-job-detail">' + escapeHtml(I18n.t('common.back')) + '</button>'
    + '<h2>' + escapeHtml(job.title) + '</h2>'
    + '<span class="badge ' + stateClass + '">' + escapeHtml(job.state) + '</span>';

  if ((job.state === 'failed' || job.state === 'interrupted') && job.can_restart === true) {
    headerHtml += '<button class="btn-restart" data-action="restart-job" data-id="' + escapeHtml(job.id) + '">Retry</button>';
  }
  if (job.browse_url) {
    headerHtml += '<a class="btn-browse" href="' + escapeHtml(job.browse_url) + '" target="_blank" rel="noopener noreferrer">Browse Files</a>';
  }

  header.innerHTML = headerHtml;
  container.appendChild(header);

  // Sub-tab bar
  const tabs = document.createElement('div');
  tabs.className = 'job-detail-tabs';
  const subtabs = ['overview', 'activity', 'files'];
  for (const st of subtabs) {
    const btn = document.createElement('button');
    btn.textContent = st.charAt(0).toUpperCase() + st.slice(1);
    btn.className = st === currentJobSubTab ? 'active' : '';
    btn.addEventListener('click', () => {
      currentJobSubTab = st;
      renderJobDetail(job);
    });
    tabs.appendChild(btn);
  }
  container.appendChild(tabs);

  // Content
  const content = document.createElement('div');
  content.className = 'job-detail-content';
  container.appendChild(content);

  switch (currentJobSubTab) {
    case 'overview': renderJobOverview(content, job); break;
    case 'files': renderJobFiles(content, job); break;
    case 'activity': renderJobActivity(content, job); break;
  }
}

function metaItem(label, value) {
  return '<div class="meta-item"><div class="meta-label">' + escapeHtml(label)
    + '</div><div class="meta-value">' + escapeHtml(String(value != null ? value : '-'))
    + '</div></div>';
}

function formatDuration(secs) {
  if (secs == null) return '-';
  if (secs < 60) return secs + 's';
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  if (m < 60) return m + 'm ' + s + 's';
  const h = Math.floor(m / 60);
  return h + 'h ' + (m % 60) + 'm';
}

function renderJobOverview(container, job) {
  // Metadata grid
  const grid = document.createElement('div');
  grid.className = 'job-meta-grid';
  grid.innerHTML = metaItem(I18n.t('jobs.id'), job.id)
    + metaItem(I18n.t('jobs.state'), job.state)
    + metaItem(I18n.t('jobs.created'), formatDate(job.created_at))
    + metaItem(I18n.t('jobs.startedLabel'), formatDate(job.started_at))
    + metaItem(I18n.t('jobs.completedLabel'), formatDate(job.completed_at))
    + metaItem(I18n.t('jobs.duration'), formatDuration(job.elapsed_secs))
    + (job.job_mode ? metaItem(I18n.t('jobs.mode'), job.job_mode) : '');
  container.appendChild(grid);

  // Description
  if (job.description) {
    const descSection = document.createElement('div');
    descSection.className = 'job-description';
    const descHeader = document.createElement('h3');
    descHeader.textContent = I18n.t('jobs.description');
    descSection.appendChild(descHeader);
    const descBody = document.createElement('div');
    descBody.className = 'job-description-body';
    descBody.innerHTML = renderMarkdown(job.description);
    descSection.appendChild(descBody);
    container.appendChild(descSection);
  }

  // State transitions timeline
  if (job.transitions.length > 0) {
    const timelineSection = document.createElement('div');
    timelineSection.className = 'job-timeline-section';
    const tlHeader = document.createElement('h3');
    tlHeader.textContent = I18n.t('jobs.stateTransitions');
    timelineSection.appendChild(tlHeader);

    const timeline = document.createElement('div');
    timeline.className = 'timeline';
    for (const t of job.transitions) {
      const entry = document.createElement('div');
      entry.className = 'timeline-entry';
      const dot = document.createElement('div');
      dot.className = 'timeline-dot';
      entry.appendChild(dot);
      const info = document.createElement('div');
      info.className = 'timeline-info';
      info.innerHTML = '<span class="badge ' + t.from.replace(' ', '_') + '">' + escapeHtml(t.from) + '</span>'
        + ' &rarr; '
        + '<span class="badge ' + t.to.replace(' ', '_') + '">' + escapeHtml(t.to) + '</span>'
        + '<span class="timeline-time">' + formatDate(t.timestamp) + '</span>'
        + (t.reason ? '<div class="timeline-reason">' + escapeHtml(t.reason) + '</div>' : '');
      entry.appendChild(info);
      timeline.appendChild(entry);
    }
    timelineSection.appendChild(timeline);
    container.appendChild(timelineSection);
  }
}

function renderJobFiles(container, job) {
  container.innerHTML = '<div class="job-files">'
    + '<div class="job-files-sidebar"><div class="job-files-tree"></div></div>'
    + '<div class="job-files-viewer"><div class="empty-state">Select a file to view</div></div>'
    + '</div>';

  container._jobId = job ? job.id : null;

  apiFetch('/api/jobs/' + job.id + '/files/list?path=').then((data) => {
    jobFilesTreeState = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    renderJobFilesTree();
  }).catch(() => {
    const treeContainer = document.querySelector('.job-files-tree');
    if (treeContainer) {
      treeContainer.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No project files</div>';
    }
  });
}

function renderJobFilesTree() {
  const treeContainer = document.querySelector('.job-files-tree');
  if (!treeContainer) return;
  treeContainer.innerHTML = '';
  if (!jobFilesTreeState || jobFilesTreeState.length === 0) {
    treeContainer.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No files in workspace</div>';
    return;
  }
  renderJobFileNodes(jobFilesTreeState, treeContainer, 0);
}

function renderJobFileNodes(nodes, container, depth) {
  for (const node of nodes) {
    const row = document.createElement('div');
    row.className = 'tree-row';
    row.style.paddingLeft = (depth * 16 + 8) + 'px';

    if (node.is_dir) {
      const arrow = document.createElement('span');
      arrow.className = 'expand-arrow' + (node.expanded ? ' expanded' : '');
      arrow.textContent = '\u25B6';
      arrow.addEventListener('click', (e) => {
        e.stopPropagation();
        toggleJobFileExpand(node);
      });
      row.appendChild(arrow);

      const label = document.createElement('span');
      label.className = 'tree-label dir';
      label.textContent = node.name;
      label.addEventListener('click', () => toggleJobFileExpand(node));
      row.appendChild(label);
    } else {
      const spacer = document.createElement('span');
      spacer.className = 'expand-arrow-spacer';
      row.appendChild(spacer);

      const label = document.createElement('span');
      label.className = 'tree-label file';
      label.textContent = node.name;
      label.addEventListener('click', () => readJobFile(node.path));
      row.appendChild(label);
    }

    container.appendChild(row);

    if (node.is_dir && node.expanded && node.children) {
      const childContainer = document.createElement('div');
      childContainer.className = 'tree-children';
      renderJobFileNodes(node.children, childContainer, depth + 1);
      container.appendChild(childContainer);
    }
  }
}

function getJobId() {
  const container = document.querySelector('.job-detail-content');
  return (container && container._jobId) || null;
}

function toggleJobFileExpand(node) {
  if (node.expanded) {
    node.expanded = false;
    renderJobFilesTree();
    return;
  }
  if (node.loaded) {
    node.expanded = true;
    renderJobFilesTree();
    return;
  }
  const jobId = getJobId();
  apiFetch('/api/jobs/' + jobId + '/files/list?path=' + encodeURIComponent(node.path)).then((data) => {
    node.children = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    node.loaded = true;
    node.expanded = true;
    renderJobFilesTree();
  }).catch(() => {});
}

function readJobFile(path) {
  const viewer = document.querySelector('.job-files-viewer');
  if (!viewer) return;
  const jobId = getJobId();
  apiFetch('/api/jobs/' + jobId + '/files/read?path=' + encodeURIComponent(path)).then((data) => {
    viewer.innerHTML = '<div class="job-files-path">' + escapeHtml(path) + '</div>'
      + '<pre class="job-files-content">' + escapeHtml(data.content) + '</pre>';
  }).catch((err) => {
    viewer.innerHTML = '<div class="empty-state">Error: ' + escapeHtml(err.message) + '</div>';
  });
}

// --- Activity tab (unified for all sandbox jobs) ---

let activityCurrentJobId = null;
// Track how many live SSE events we've already rendered so refreshActivityTab
// only appends new ones (avoids duplicates on each SSE tick).
let activityRenderedLiveIndex = 0;

function renderJobActivity(container, job) {
  activityCurrentJobId = job ? job.id : null;
  activityRenderedLiveIndex = 0;

  let html = '<div class="activity-toolbar">'
    + '<select id="activity-type-filter">'
    + '<option value="all">All Events</option>'
    + '<option value="message">Messages</option>'
    + '<option value="tool_use">Tool Calls</option>'
    + '<option value="tool_result">Results</option>'
    + '</select>'
    + '<label class="logs-checkbox"><input type="checkbox" id="activity-autoscroll" checked> ' + escapeHtml(I18n.t('jobs.autoScroll')) + '</label>'
    + '</div>'
    + '<div class="activity-terminal" id="activity-terminal"></div>';

  if (job && job.can_prompt === true) {
    html += '<div class="activity-input-bar" id="activity-input-bar">'
      + '<input type="text" id="activity-prompt-input" placeholder="' + escapeHtml(I18n.t('jobs.followUpPlaceholder')) + '" />'
      + '<button id="activity-send-btn">' + escapeHtml(I18n.t('chat.send')) + '</button>'
      + '<button id="activity-done-btn" title="' + escapeHtml(I18n.t('jobs.signalDone')) + '">' + escapeHtml(I18n.t('jobs.done')) + '</button>'
      + '</div>';
  }

  container.innerHTML = html;

  document.getElementById('activity-type-filter').addEventListener('change', applyActivityFilter);

  const terminal = document.getElementById('activity-terminal');
  const input = document.getElementById('activity-prompt-input');
  const sendBtn = document.getElementById('activity-send-btn');
  const doneBtn = document.getElementById('activity-done-btn');

  if (sendBtn) sendBtn.addEventListener('click', () => sendJobPrompt(job.id, false));
  if (doneBtn) doneBtn.addEventListener('click', () => sendJobPrompt(job.id, true));
  if (input) input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') sendJobPrompt(job.id, false);
  });

  // Load persisted events from DB, then catch up with any live SSE events
  apiFetch('/api/jobs/' + job.id + '/events').then((data) => {
    if (data.events && data.events.length > 0) {
      for (const evt of data.events) {
        appendActivityEvent(terminal, evt.event_type, evt.data);
      }
    }
    appendNewLiveEvents(terminal, job.id);
  }).catch(() => {
    appendNewLiveEvents(terminal, job.id);
  });
}

function appendNewLiveEvents(terminal, jobId) {
  const live = jobEvents.get(jobId) || [];
  for (let i = activityRenderedLiveIndex; i < live.length; i++) {
    const evt = live[i];
    appendActivityEvent(terminal, evt.type.replace('job_', ''), evt.data);
  }
  activityRenderedLiveIndex = live.length;
  const autoScroll = document.getElementById('activity-autoscroll');
  if (!autoScroll || autoScroll.checked) {
    terminal.scrollTop = terminal.scrollHeight;
  }
}

function applyActivityFilter() {
  const filter = document.getElementById('activity-type-filter').value;
  const events = document.querySelectorAll('#activity-terminal .activity-event');
  for (const el of events) {
    if (filter === 'all') {
      el.style.display = '';
    } else {
      el.style.display = el.getAttribute('data-event-type') === filter ? '' : 'none';
    }
  }
}

function appendActivityEvent(terminal, eventType, data) {
  if (!terminal) return;
  const el = document.createElement('div');
  el.className = 'activity-event activity-event-' + eventType;
  el.setAttribute('data-event-type', eventType);

  // Respect current filter
  const filterEl = document.getElementById('activity-type-filter');
  if (filterEl && filterEl.value !== 'all' && filterEl.value !== eventType) {
    el.style.display = 'none';
  }

  switch (eventType) {
    case 'message':
      el.innerHTML = '<span class="activity-role">' + escapeHtml(data.role || 'assistant') + '</span> '
        + '<span class="activity-content">' + escapeHtml(data.content || '') + '</span>';
      break;
    case 'tool_use':
      el.innerHTML = '<details class="activity-tool-block"><summary>'
        + '<span class="activity-tool-icon">&#9881;</span> '
        + escapeHtml(data.tool_name || 'tool')
        + '</summary><pre class="activity-tool-input">'
        + escapeHtml(typeof data.input === 'string' ? data.input : JSON.stringify(data.input, null, 2))
        + '</pre></details>';
      break;
    case 'tool_result': {
      const trSuccess = data.success !== false;
      const trIcon = trSuccess ? '&#10003;' : '&#10007;';
      const trOutput = data.output || data.error || '';
      const trClass = 'activity-tool-block activity-tool-result'
        + (trSuccess ? '' : ' activity-tool-error');
      el.innerHTML = '<details class="' + trClass + '"><summary>'
        + '<span class="activity-tool-icon">' + trIcon + '</span> '
        + escapeHtml(data.tool_name || 'result')
        + '</summary><pre class="activity-tool-output">'
        + escapeHtml(trOutput)
        + '</pre></details>';
      break;
    }
    case 'status':
      el.innerHTML = '<span class="activity-status">' + escapeHtml(data.message || '') + '</span>';
      break;
    case 'result':
      el.className += ' activity-final';
      const success = data.success !== false;
      el.innerHTML = '<span class="activity-result-status" data-success="' + success + '">'
        + escapeHtml(data.message || data.error || data.status || 'done') + '</span>';
      if (data.session_id) {
        el.innerHTML += ' <span class="activity-session-id">session: ' + escapeHtml(data.session_id) + '</span>';
      }
      break;
    default:
      el.innerHTML = '<span class="activity-status">' + escapeHtml(JSON.stringify(data)) + '</span>';
  }

  terminal.appendChild(el);
}

function refreshActivityTab(jobId) {
  if (activityCurrentJobId !== jobId) return;
  if (currentJobSubTab !== 'activity') return;
  const terminal = document.getElementById('activity-terminal');
  if (!terminal) return;
  appendNewLiveEvents(terminal, jobId);
}

function sendJobPrompt(jobId, done) {
  const input = document.getElementById('activity-prompt-input');
  const content = input ? input.value.trim() : '';
  if (!content && !done) return;

  apiFetch('/api/jobs/' + jobId + '/prompt', {
    method: 'POST',
    body: { content: content || '(done)', done: done },
  }).then(() => {
    if (input) input.value = '';
    if (done) {
      const bar = document.getElementById('activity-input-bar');
      if (bar) bar.innerHTML = '<span class="activity-status">Done signal sent</span>';
    }
  }).catch((err) => {
    const terminal = document.getElementById('activity-terminal');
    if (terminal) {
      appendActivityEvent(terminal, 'status', { message: 'Failed to send: ' + err.message });
    }
  });
}

// --- Routines ---

let currentRoutineId = null;

function loadRoutines() {
  currentRoutineId = null;

  // Restore list view if detail was open
  const detail = document.getElementById('routine-detail');
  if (detail) detail.style.display = 'none';
  const table = document.getElementById('routines-table');
  if (table) table.style.display = '';

  Promise.all([
    apiFetch('/api/routines/summary'),
    apiFetch('/api/routines'),
  ]).then(([summary, listData]) => {
    renderRoutinesSummary(summary);
    renderRoutinesList(listData.routines);
  }).catch(() => {});
}

function renderRoutinesSummary(s) {
  document.getElementById('routines-summary').innerHTML = ''
    + summaryCard(I18n.t('routines.summary.total'), s.total, '')
    + summaryCard(I18n.t('routines.summary.enabled'), s.enabled, 'active')
    + summaryCard(I18n.t('routines.summary.disabled'), s.disabled, '')
    + summaryCard(I18n.t('routines.summary.unverified'), s.unverified, 'pending')
    + summaryCard(I18n.t('routines.summary.failing'), s.failing, 'failed')
    + summaryCard(I18n.t('routines.summary.runsToday'), s.runs_today, 'completed');
}

function renderRoutinesList(routines) {
  const tbody = document.getElementById('routines-tbody');
  const empty = document.getElementById('routines-empty');

  if (!routines || routines.length === 0) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = routines.map((r) => {
    const statusClass = r.status === 'active' ? 'completed'
      : r.status === 'failing' ? 'failed'
      : r.status === 'attention' ? 'stuck'
      : r.status === 'running' ? 'in_progress'
      : 'pending';

    const toggleLabel = r.enabled ? 'Disable' : 'Enable';
    const toggleClass = r.enabled ? 'btn-cancel' : 'btn-restart';
    const triggerTitle = (r.trigger_type === 'cron' && r.trigger_raw)
      ? ' title="' + escapeHtml(r.trigger_raw) + '"'
      : '';
    const runLabel = (r.verification_status === 'unverified' || r.status === 'unverified')
      ? 'Verify now'
      : 'Run';

    return '<tr class="routine-row" data-action="open-routine" data-id="' + escapeHtml(r.id) + '">'
      + '<td>' + escapeHtml(r.name) + '</td>'
      + '<td' + triggerTitle + '>' + escapeHtml(r.trigger_summary) + '</td>'
      + '<td>' + escapeHtml(r.action_type) + '</td>'
      + '<td>' + formatRelativeTime(r.last_run_at) + '</td>'
      + '<td>' + formatRelativeTime(r.next_fire_at) + '</td>'
      + '<td>' + r.run_count + '</td>'
      + '<td><span class="badge ' + statusClass + '">' + escapeHtml(r.status) + '</span></td>'
      + '<td>'
      + '<button class="' + toggleClass + '" data-action="toggle-routine" data-id="' + escapeHtml(r.id) + '">' + toggleLabel + '</button> '
      + '<button class="btn-restart" data-action="trigger-routine" data-id="' + escapeHtml(r.id) + '">' + runLabel + '</button> '
      + '<button class="btn-cancel" data-action="delete-routine" data-id="' + escapeHtml(r.id) + '" data-name="' + escapeHtml(r.name) + '">Delete</button>'
      + '</td>'
      + '</tr>';
  }).join('');
}

function openRoutineDetail(id) {
  currentRoutineId = id;
  updateHash();
  apiFetch('/api/routines/' + id).then((routine) => {
    renderRoutineDetail(routine);
  }).catch((err) => {
    showToast(I18n.t('routines.loadFailed', { message: err.message }), 'error');
  });
}

function closeRoutineDetail() {
  currentRoutineId = null;
  loadRoutines();
  updateHash();
}

function renderRoutineDetail(routine) {
  const table = document.getElementById('routines-table');
  if (table) table.style.display = 'none';
  document.getElementById('routines-empty').style.display = 'none';

  const detail = document.getElementById('routine-detail');
  detail.style.display = 'block';

  const statusClass = routine.status === 'active' ? 'completed'
    : routine.status === 'failing' ? 'failed'
    : routine.status === 'attention' ? 'stuck'
    : routine.status === 'running' ? 'in_progress'
    : 'pending';
  const statusLabel = routine.status || 'active';

  let html = '<div class="job-detail-header">'
    + '<button class="btn-back" data-action="close-routine-detail">&larr; Back</button>'
    + '<h2>' + escapeHtml(routine.name) + '</h2>'
    + '<span class="badge ' + statusClass + '">' + escapeHtml(statusLabel) + '</span>'
    + '</div>';

  // Metadata grid
  html += '<div class="job-meta-grid">'
    + metaItem(I18n.t('routines.id'), routine.id)
    + metaItem(I18n.t('routines.enabled'), routine.enabled ? I18n.t('settings.on') : I18n.t('settings.off'))
    + metaItem(I18n.t('routines.runCount'), routine.run_count)
    + metaItem(I18n.t('routines.failures'), routine.consecutive_failures)
    + metaItem(I18n.t('routines.lastRun'), formatDate(routine.last_run_at))
    + metaItem(I18n.t('routines.nextFire'), formatDate(routine.next_fire_at))
    + metaItem(I18n.t('routines.created'), formatDate(routine.created_at))
    + '</div>';

  // Description
  if (routine.description) {
    html += '<div class="job-description"><h3>Description</h3>'
      + '<div class="job-description-body">' + escapeHtml(routine.description) + '</div></div>';
  }

  if (routine.verification_status === 'unverified') {
    let verificationCopy = 'Created or updated, but not yet verified with a successful run.';
    if (routine.recent_runs && routine.recent_runs.length > 0) {
      const latestRun = routine.recent_runs[0];
      if (latestRun.status === 'failed') {
        verificationCopy = 'The latest verification attempt failed. Review the run details and verify again after fixing it.';
      } else if (latestRun.status === 'attention') {
        verificationCopy = 'The latest verification attempt needs attention. Review the run details and verify again when ready.';
      }
    }
    html += '<div class="job-description"><h3>Verification</h3>'
      + '<div class="job-description-body">' + escapeHtml(verificationCopy) + '</div></div>';
  }

  // Trigger config
  if (routine.trigger_type === 'cron') {
    const summary = routine.trigger_summary || 'cron';
    const raw = routine.trigger_raw || '';
    const timezone = routine.trigger && routine.trigger.timezone ? String(routine.trigger.timezone) : '';
    html += '<div class="job-description"><h3>Trigger</h3>'
      + '<div class="job-description-body"><strong>' + escapeHtml(summary) + '</strong></div>';
    if (raw) {
      html += '<div class="job-meta-item">'
        + '<span class="job-meta-label">Raw</span>'
        + '<span class="job-meta-value">' + escapeHtml(raw + (timezone ? ' (' + timezone + ')' : '')) + '</span>'
        + '</div>';
    }
    html += '</div>';
  } else {
    html += '<div class="job-description"><h3>Trigger</h3>'
      + '<pre class="action-json">' + escapeHtml(JSON.stringify(routine.trigger, null, 2)) + '</pre></div>';
  }

  html += '<div class="job-description"><h3>Action</h3>'
    + '<pre class="action-json">' + escapeHtml(JSON.stringify(routine.action, null, 2)) + '</pre></div>';

  // Conversation thread link
  if (routine.conversation_id) {
    html += '<div class="job-description">'
      + '<a href="#" data-action="view-routine-thread" data-id="' + escapeHtml(routine.conversation_id) + '" class="btn-primary" style="display:inline-block;margin:0.5rem 0">'
      + 'View Execution Thread</a></div>';
  }

  // Recent runs
  if (routine.recent_runs && routine.recent_runs.length > 0) {
    html += '<div class="job-timeline-section"><h3>Recent Runs</h3>'
      + '<table class="routines-table"><thead><tr>'
      + '<th>Trigger</th><th>Started</th><th>Completed</th><th>Status</th><th>Summary</th><th>Tokens</th>'
      + '</tr></thead><tbody>';
    for (const run of routine.recent_runs) {
      const runStatusClass = run.status === 'ok' ? 'completed'
        : run.status === 'failed' ? 'failed'
        : run.status === 'attention' ? 'stuck'
        : 'in_progress';
      html += '<tr>'
        + '<td>' + escapeHtml(run.trigger_type) + '</td>'
        + '<td>' + formatDate(run.started_at) + '</td>'
        + '<td>' + formatDate(run.completed_at) + '</td>'
        + '<td><span class="badge ' + runStatusClass + '">' + escapeHtml(run.status) + '</span></td>'
        + '<td>' + escapeHtml(run.result_summary || '-')
          + (run.job_id ? ' <a href="#" data-action="view-run-job" data-id="' + escapeHtml(run.job_id) + '">[view job]</a>' : '')
          + '</td>'
        + '<td>' + (run.tokens_used != null ? run.tokens_used : '-') + '</td>'
        + '</tr>';
    }
    html += '</tbody></table></div>';
  }

  detail.innerHTML = html;
}

function triggerRoutine(id) {
  apiFetch('/api/routines/' + id + '/trigger', { method: 'POST' })
    .then(() => {
      showToast(I18n.t('routines.triggered'), 'success');
      if (currentRoutineId === id) openRoutineDetail(id);
      else loadRoutines();
    })
    .catch((err) => showToast(I18n.t('routines.triggerFailed', { message: err.message }), 'error'));
}

function toggleRoutine(id) {
  apiFetch('/api/routines/' + id + '/toggle', { method: 'POST' })
    .then((res) => {
      showToast(I18n.t('routines.toggled', { status: res.status || 'toggled' }), 'success');
      if (currentRoutineId) openRoutineDetail(currentRoutineId);
      else loadRoutines();
    })
    .catch((err) => showToast(I18n.t('routines.toggleFailed', { message: err.message }), 'error'));
}

function deleteRoutine(id, name) {
  if (!confirm(I18n.t('routines.confirmDelete', { name: name }))) return;
  apiFetch('/api/routines/' + id, { method: 'DELETE' })
    .then(() => {
      showToast(I18n.t('routines.deleted'), 'success');
      if (currentRoutineId === id) closeRoutineDetail();
      else loadRoutines();
    })
    .catch((err) => showToast(I18n.t('routines.deleteFailed', { message: err.message }), 'error'));
}

// ── Missions ──────────────────────────────────────────────

let currentMissionId = null;

function loadMissions() {
  currentMissionId = null;
  const detail = document.getElementById('mission-detail');
  if (detail) detail.style.display = 'none';
  const table = document.getElementById('missions-table');
  if (table) table.style.display = '';

  Promise.all([
    apiFetch('/api/engine/missions/summary'),
    apiFetch('/api/engine/missions'),
  ]).then(([summary, listData]) => {
    renderMissionsSummary(summary);
    renderMissionsList(listData.missions);
  }).catch(() => {});
}

function renderMissionsSummary(s) {
  document.getElementById('missions-summary').innerHTML = ''
    + summaryCard(I18n.t('missions.summary.total'), s.total, '')
    + summaryCard(I18n.t('missions.summary.active'), s.active, 'active')
    + summaryCard(I18n.t('missions.summary.paused'), s.paused, '')
    + summaryCard(I18n.t('missions.summary.completed'), s.completed, 'completed')
    + summaryCard(I18n.t('missions.summary.failed'), s.failed, 'failed');
}

function renderMissionsList(missions) {
  const tbody = document.getElementById('missions-tbody');
  const empty = document.getElementById('missions-empty');

  if (!missions || missions.length === 0) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = missions.map((m) => {
    const statusClass = m.status === 'Active' ? 'in_progress'
      : m.status === 'Completed' ? 'completed'
      : m.status === 'Paused' ? 'pending'
      : 'failed';

    return '<tr class="mission-row" data-action="open-mission" data-id="' + escapeHtml(m.id) + '">'
      + '<td>' + escapeHtml(m.name) + '</td>'
      + '<td class="truncate">' + escapeHtml(m.goal) + '</td>'
      + '<td>' + escapeHtml(m.cadence_description || m.cadence_type) + '</td>'
      + '<td>' + m.thread_count + '</td>'
      + '<td><span class="badge ' + statusClass + '">' + escapeHtml(m.status) + '</span></td>'
      + '<td>'
      + (m.status === 'Active' ? '<button class="btn-cancel" data-action="pause-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.pause')) + '</button> ' : '')
      + (m.status === 'Paused' ? '<button class="btn-restart" data-action="resume-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.resume')) + '</button> ' : '')
      + '<button class="btn-restart" data-action="fire-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.fire')) + '</button>'
      + '</td>'
      + '</tr>';
  }).join('');
}

function openMissionDetail(id) {
  currentMissionId = id;
  apiFetch('/api/engine/missions/' + id).then((data) => {
    renderMissionDetail(data.mission);
  }).catch((err) => {
    showToast(I18n.t('missions.loadFailed', { message: err.message }), 'error');
  });
}

function closeMissionDetail() {
  currentMissionId = null;
  loadMissions();
}

function renderMissionDetail(m) {
  const table = document.getElementById('missions-table');
  if (table) table.style.display = 'none';
  document.getElementById('missions-empty').style.display = 'none';

  const detail = document.getElementById('mission-detail');
  detail.style.display = 'block';

  const statusClass = m.status === 'Active' ? 'in_progress'
    : m.status === 'Completed' ? 'completed'
    : m.status === 'Paused' ? 'pending'
    : 'failed';

  let html = '<div class="job-detail-header">'
    + '<button class="btn-back" data-action="close-mission-detail">' + escapeHtml(I18n.t('common.back')) + '</button>'
    + '<h2>' + escapeHtml(m.name) + '</h2>'
    + '<span class="badge ' + statusClass + '">' + escapeHtml(m.status) + '</span>'
    + '</div>';

  // Goal — full-width markdown block
  html += '<div class="job-description"><h3>Goal</h3>'
    + '<div class="job-description-body">' + renderMarkdown(m.goal) + '</div></div>';

  html += '<div class="job-meta-grid">'
    + metaItem(I18n.t('missions.cadence'), m.cadence_description || m.cadence_type)
    + metaItem(I18n.t('missions.status'), m.status)
    + metaItem(I18n.t('missions.threadsToday'), m.threads_today + ' / ' + (m.max_threads_per_day || '∞'))
    + metaItem(I18n.t('missions.totalThreads'), m.thread_count)
    + metaItem(I18n.t('missions.created'), formatDate(m.created_at))
    + metaItem(I18n.t('missions.nextFire'), m.next_fire_at ? formatDate(m.next_fire_at) : I18n.t('common.noData'))
    + '</div>';

  if (m.current_focus) {
    html += '<div class="job-description"><h3>Current Focus</h3>'
      + '<div class="job-description-body">' + renderMarkdown(m.current_focus) + '</div></div>';
  }

  if (m.success_criteria) {
    html += '<div class="job-description"><h3>Success Criteria</h3>'
      + '<div class="job-description-body">' + renderMarkdown(m.success_criteria) + '</div></div>';
  }

  if (m.notify_channels && m.notify_channels.length > 0) {
    html += '<div class="job-description"><h3>Notify Channels</h3>'
      + '<div class="job-description-body">' + m.notify_channels.map(escapeHtml).join(', ') + '</div></div>';
  }

  if (m.approach_history && m.approach_history.length > 0) {
    html += '<div class="job-description"><h3>Approach History</h3>';
    m.approach_history.forEach((a, i) => {
      html += '<div class="job-description-body" style="margin-bottom:8px">'
        + '<strong>Run ' + (i + 1) + '</strong><br>'
        + renderMarkdown(a) + '</div>';
    });
    html += '</div>';
  }

  if (m.threads && m.threads.length > 0) {
    html += '<div class="job-description"><h3>Spawned Threads</h3>'
      + '<table class="missions-table"><thead><tr>'
      + '<th>Goal</th><th>Type</th><th>State</th><th>Steps</th><th>Tokens</th><th>Created</th>'
      + '</tr></thead><tbody>';
    m.threads.forEach((t) => {
      var tState = t.state === 'Done' || t.state === 'Completed' ? 'completed'
        : t.state === 'Failed' ? 'failed'
        : t.state === 'Running' ? 'in_progress'
        : 'pending';
      html += '<tr class="mission-row" data-action="open-engine-thread" data-id="' + escapeHtml(t.id) + '">'
        + '<td class="truncate">' + escapeHtml(t.goal) + '</td>'
        + '<td>' + escapeHtml(t.thread_type) + '</td>'
        + '<td><span class="badge ' + tState + '">' + escapeHtml(t.state) + '</span></td>'
        + '<td>' + t.step_count + '</td>'
        + '<td>' + t.total_tokens.toLocaleString() + '</td>'
        + '<td>' + formatDate(t.created_at) + '</td>'
        + '</tr>';
    });
    html += '</tbody></table></div>';
  }

  // Action buttons
  html += '<div style="margin-top:16px;">';
  if (m.status === 'Active') {
    html += '<button class="btn-cancel" data-action="pause-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.pause')) + '</button> ';
  }
  if (m.status === 'Paused') {
    html += '<button class="btn-restart" data-action="resume-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.resume')) + '</button> ';
  }
  html += '<button class="btn-restart" data-action="fire-mission" data-id="' + escapeHtml(m.id) + '">' + escapeHtml(I18n.t('missions.fireNow')) + '</button>';
  html += '</div>';

  detail.innerHTML = html;
}

function openEngineThread(threadId) {
  apiFetch('/api/engine/threads/' + threadId).then((data) => {
    var t = data.thread;
    var detail = document.getElementById('mission-detail');

    var stateClass = t.state === 'Done' || t.state === 'Completed' ? 'completed'
      : t.state === 'Failed' ? 'failed'
      : t.state === 'Running' ? 'in_progress'
      : 'pending';

    var html = '<div class="job-detail-header">'
      + '<button class="btn-back" data-action="back-to-mission">' + escapeHtml(I18n.t('missions.backToMission')) + '</button>'
      + '<h2>Thread: ' + escapeHtml(t.goal) + '</h2>'
      + '<span class="badge ' + stateClass + '">' + escapeHtml(t.state) + '</span>'
      + '</div>';

    html += '<div class="job-meta-grid">'
      + metaItem(I18n.t('missions.threadId'), t.id)
      + metaItem(I18n.t('missions.type'), t.thread_type)
      + metaItem(I18n.t('missions.steps'), t.step_count)
      + metaItem(I18n.t('missions.tokens'), t.total_tokens.toLocaleString())
      + metaItem(I18n.t('missions.cost'), t.total_cost_usd > 0 ? '$' + t.total_cost_usd.toFixed(4) : '-')
      + metaItem(I18n.t('missions.maxIterations'), t.max_iterations)
      + metaItem(I18n.t('missions.created'), formatDate(t.created_at))
      + metaItem(I18n.t('jobs.completedLabel'), t.completed_at ? formatDate(t.completed_at) : '-')
      + '</div>';

    if (t.messages && t.messages.length > 0) {
      html += '<div class="job-description"><h3>Messages (' + t.messages.length + ')</h3>';
      t.messages.forEach(function(msg, i) {
        var roleClass = msg.role === 'Assistant' ? 'assistant' : msg.role === 'User' ? 'user' : 'system';
        html += '<div class="thread-message thread-msg-' + roleClass + '">'
          + '<div class="thread-msg-role">' + escapeHtml(msg.role) + '</div>'
          + '<div class="thread-msg-content">' + renderMarkdown(msg.content) + '</div>'
          + '</div>';
      });
      html += '</div>';
    }

    detail.innerHTML = html;
  }).catch(function(err) {
    showToast(I18n.t('missions.threadLoadFailed', { message: err.message }), 'error');
  });
}

function fireMission(id) {
  apiFetch('/api/engine/missions/' + id + '/fire', { method: 'POST' })
    .then((data) => {
      if (data.fired) {
        showToast(I18n.t('missions.fired', { id: data.thread_id }), 'success');
      } else {
        showToast(I18n.t('missions.notFired'), 'warning');
      }
      if (currentMissionId === id) openMissionDetail(id);
      else loadMissions();
    })
    .catch((err) => showToast(I18n.t('missions.fireFailed', { message: err.message }), 'error'));
}

function pauseMission(id) {
  apiFetch('/api/engine/missions/' + id + '/pause', { method: 'POST' })
    .then(() => {
      showToast(I18n.t('missions.paused'), 'success');
      if (currentMissionId === id) openMissionDetail(id);
      else loadMissions();
    })
    .catch((err) => showToast(I18n.t('missions.pauseFailed', { message: err.message }), 'error'));
}

function resumeMission(id) {
  apiFetch('/api/engine/missions/' + id + '/resume', { method: 'POST' })
    .then(() => {
      showToast(I18n.t('missions.resumed'), 'success');
      if (currentMissionId === id) openMissionDetail(id);
      else loadMissions();
    })
    .catch((err) => showToast(I18n.t('missions.resumeFailed', { message: err.message }), 'error'));
}

function formatRelativeTime(isoString) {
  if (!isoString) return '-';
  const d = new Date(isoString);
  const now = Date.now();
  const diffMs = now - d.getTime();
  const absDiff = Math.abs(diffMs);
  const future = diffMs < 0;

  if (absDiff < 60000)
    return future ? I18n.t('time.lessThan1MinuteFromNow') : I18n.t('time.lessThan1MinuteAgo');
  if (absDiff < 3600000) {
    const m = Math.floor(absDiff / 60000);
    return future ? I18n.t('time.minutesFromNow', { n: m }) : I18n.t('time.minutesAgo', { n: m });
  }
  if (absDiff < 86400000) {
    const h = Math.floor(absDiff / 3600000);
    return future ? I18n.t('time.hoursFromNow', { n: h }) : I18n.t('time.hoursAgo', { n: h });
  }
  const days = Math.floor(absDiff / 86400000);
  return future ? I18n.t('time.daysFromNow', { n: days }) : I18n.t('time.daysAgo', { n: days });
}

// --- Users (admin) ---

function loadUsers() {
  apiFetch('/api/admin/users').then(function(data) {
    renderUsersList(data.users || []);
  }).catch(function(err) {
    var tbody = document.getElementById('users-tbody');
    var empty = document.getElementById('users-empty');
    if (tbody) tbody.innerHTML = '';
    if (empty) {
      empty.style.display = 'block';
      if (err.status === 403 || err.status === 401) {
        empty.textContent = I18n.t('users.adminRequired');
      } else {
        empty.textContent = I18n.t('users.failedToLoad') + ': ' + err.message;
      }
    }
  });
}

function renderUsersList(users) {
  var tbody = document.getElementById('users-tbody');
  var empty = document.getElementById('users-empty');
  if (!users || users.length === 0) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    empty.textContent = I18n.t('users.emptyState');
    return;
  }
  empty.style.display = 'none';
  tbody.innerHTML = users.map(function(u) {
    var statusClass = u.status === 'active' ? 'active' : 'failed';
    var roleLabel = u.role === 'admin' ? '<span class="badge badge-admin">' + I18n.t('users.roleAdmin') + '</span>' : '<span class="badge">' + I18n.t('users.roleMember') + '</span>';
    var actions = '';
    if (u.status === 'active') {
      actions += '<button class="btn-small btn-danger" data-action="suspend-user" data-user-id="' + escapeHtml(u.id) + '">' + I18n.t('users.suspend') + '</button> ';
    } else {
      actions += '<button class="btn-small btn-primary" data-action="activate-user" data-user-id="' + escapeHtml(u.id) + '">' + I18n.t('users.activate') + '</button> ';
    }
    if (u.role === 'member') {
      actions += '<button class="btn-small" data-action="change-role" data-user-id="' + escapeHtml(u.id) + '" data-role="admin">' + I18n.t('users.makeAdmin') + '</button> ';
    } else {
      actions += '<button class="btn-small" data-action="change-role" data-user-id="' + escapeHtml(u.id) + '" data-role="member">' + I18n.t('users.makeMember') + '</button> ';
    }
    actions += '<button class="btn-small" data-action="create-token" data-user-id="' + escapeHtml(u.id) + '" data-user-name="' + escapeHtml(u.display_name) + '">' + I18n.t('users.addToken') + '</button>';
    return '<tr>'
      + '<td class="user-id" title="' + escapeHtml(u.id) + '">' + escapeHtml(u.id.substring(0, 8)) + '…</td>'
      + '<td>' + escapeHtml(u.display_name) + '</td>'
      + '<td>' + escapeHtml(u.email || '—') + '</td>'
      + '<td>' + roleLabel + '</td>'
      + '<td><span class="status-badge ' + statusClass + '">' + escapeHtml(u.status) + '</span></td>'
      + '<td>' + (u.job_count || 0) + '</td>'
      + '<td>' + formatCost(u.total_cost) + '</td>'
      + '<td>' + (u.last_active_at ? formatRelativeTime(u.last_active_at) : '—') + '</td>'
      + '<td>' + formatRelativeTime(u.created_at) + '</td>'
      + '<td>' + actions + '</td>'
      + '</tr>';
  }).join('');
}

function suspendUser(userId) {
  apiFetch('/api/admin/users/' + userId + '/suspend', { method: 'POST' })
    .then(function() { loadUsers(); })
    .catch(function(e) { alert(I18n.t('users.failedSuspend') + ': ' + e.message); });
}

function activateUser(userId) {
  apiFetch('/api/admin/users/' + userId + '/activate', { method: 'POST' })
    .then(function() { loadUsers(); })
    .catch(function(e) { alert(I18n.t('users.failedActivate') + ': ' + e.message); });
}

function changeUserRole(userId, newRole) {
  apiFetch('/api/admin/users/' + userId, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ role: newRole })
  })
    .then(function() { loadUsers(); })
    .catch(function(e) { alert(I18n.t('users.failedRoleChange') + ': ' + e.message); });
}

function createTokenForUser(userId, displayName) {
  var tokenName = prompt('Token name for ' + displayName + ':', 'api-token');
  if (!tokenName) return;
  apiFetch('/api/tokens', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: tokenName, user_id: userId }),
  }).then(function(data) {
    showTokenBanner(data.token, I18n.t('users.tokenCreated'));
  }).catch(function(e) { alert(I18n.t('users.failedCreate') + ': ' + e.message); });
}

function showTokenBanner(tokenValue, title) {
  var banner = document.getElementById('users-token-result');
  if (!banner) return;
  var heading = title || I18n.t('users.tokenCreated');
  var loginUrl = window.location.origin + '/?token=' + encodeURIComponent(tokenValue);
  banner.style.display = 'block';
  banner.innerHTML = '<strong>' + escapeHtml(heading) + '</strong> ' + I18n.t('users.tokenShareMessage') + '<br>'
    + '<code class="token-display" id="token-copy-value">' + escapeHtml(loginUrl) + '</code>'
    + '<button class="btn-small" id="token-copy-link">Copy Link</button>'
    + '<br><span style="font-size:0.8em;color:var(--text-muted)">' + I18n.t('users.rawToken') + ' ' + escapeHtml(tokenValue) + '</span>';
  document.getElementById('token-copy-link').addEventListener('click', function() {
    navigator.clipboard.writeText(loginUrl);
    this.textContent = I18n.t('users.copied');
  });
}

// Delegated click handler for user action buttons (CSP-safe, no inline onclick)
document.getElementById('users-table')?.addEventListener('click', function(e) {
  var btn = e.target.closest('[data-action]');
  if (!btn) return;
  var action = btn.getAttribute('data-action');
  var userId = btn.getAttribute('data-user-id');
  var userName = btn.getAttribute('data-user-name');
  if (action === 'suspend-user') suspendUser(userId);
  else if (action === 'activate-user') activateUser(userId);
  else if (action === 'change-role') changeUserRole(userId, btn.getAttribute('data-role'));
  else if (action === 'create-token') createTokenForUser(userId, userName || '');
});

// Wire up Users tab create form
document.getElementById('users-create-btn')?.addEventListener('click', function() {
  document.getElementById('users-create-form').style.display = 'flex';
  document.getElementById('users-token-result').style.display = 'none';
  document.getElementById('user-display-name').focus();
});

document.getElementById('users-create-cancel')?.addEventListener('click', function() {
  document.getElementById('users-create-form').style.display = 'none';
});

document.getElementById('users-create-submit')?.addEventListener('click', function() {
  var displayName = document.getElementById('user-display-name').value.trim();
  var email = document.getElementById('user-email').value.trim();
  var role = document.getElementById('user-role').value;
  if (!displayName) { alert(I18n.t('users.displayNameRequired')); return; }

  apiFetch('/api/admin/users', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      display_name: displayName,
      email: email || undefined,
      role: role,
    }),
  }).then(function(data) {
    document.getElementById('users-create-form').style.display = 'none';
    document.getElementById('user-display-name').value = '';
    document.getElementById('user-email').value = '';
    if (data.token) {
      showTokenBanner(data.token, I18n.t('users.userCreated'));
    }
    loadUsers();
  }).catch(function(e) { alert(I18n.t('users.failedCreate') + ': ' + e.message); });
});

// --- Gateway status widget ---

let gatewayStatusInterval = null;

function startGatewayStatusPolling() {
  if (gatewayStatusInterval) return; // already polling
  fetchGatewayStatus();
  gatewayStatusInterval = setInterval(fetchGatewayStatus, 30000);
}

function formatTokenCount(n) {
  if (n == null || n === 0) return '0';
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'k';
  return '' + n;
}

function formatCost(costStr) {
  if (!costStr) return '$0.00';
  var n = parseFloat(costStr);
  if (n < 0.01) return '$' + n.toFixed(4);
  return '$' + n.toFixed(2);
}

function shortModelName(model) {
  // Strip provider prefix and shorten common model names
  var m = model.indexOf('/') >= 0 ? model.split('/').pop() : model;
  // Shorten dated suffixes
  m = m.replace(/-20\d{6}$/, '');
  return m;
}

function fetchGatewayStatus() {
  apiFetch('/api/gateway/status').then(function(data) {
    // Update restart button visibility
    restartEnabled = data.restart_enabled || false;
    updateRestartButtonVisibility();

    var popover = document.getElementById('gateway-popover');
    var html = '';

    // Version
    if (data.version) {
      html += '<div class="gw-section-label">IronClaw v' + escapeHtml(data.version) + '</div>';
      html += '<div class="gw-divider"></div>';
    }

    // Connection info
    html += '<div class="gw-section-label">' + I18n.t('dashboard.connections') + '</div>';
    html += '<div class="gw-stat"><span>' + I18n.t('dashboard.sse') + '</span><span>' + (data.sse_connections || 0) + '</span></div>';
    html += '<div class="gw-stat"><span>' + I18n.t('dashboard.websocket') + '</span><span>' + (data.ws_connections || 0) + '</span></div>';
    html += '<div class="gw-stat"><span>' + I18n.t('dashboard.uptime') + '</span><span>' + formatDuration(data.uptime_secs) + '</span></div>';

    // Cost tracker
    if (data.daily_cost != null) {
      html += '<div class="gw-divider"></div>';
      html += '<div class="gw-section-label">' + I18n.t('dashboard.costToday') + '</div>';
      html += '<div class="gw-stat"><span>' + I18n.t('dashboard.spent') + '</span><span>' + formatCost(data.daily_cost) + '</span></div>';
      if (data.actions_this_hour != null) {
        html += '<div class="gw-stat"><span>' + I18n.t('dashboard.actionsPerHour') + '</span><span>' + data.actions_this_hour + '</span></div>';
      }
    }

    // Per-model token usage
    if (data.model_usage && data.model_usage.length > 0) {
      html += '<div class="gw-divider"></div>';
      html += '<div class="gw-section-label">Token Usage</div>';
      data.model_usage.sort(function(a, b) {
        return (b.input_tokens + b.output_tokens) - (a.input_tokens + a.output_tokens);
      });
      for (var i = 0; i < data.model_usage.length; i++) {
        var m = data.model_usage[i];
        var name = escapeHtml(shortModelName(m.model));
        html += '<div class="gw-model-row">'
          + '<span class="gw-model-name">' + name + '</span>'
          + '<span class="gw-model-cost">' + escapeHtml(formatCost(m.cost)) + '</span>'
          + '</div>';
        html += '<div class="gw-token-detail">'
          + '<span>in: ' + formatTokenCount(m.input_tokens) + '</span>'
          + '<span>out: ' + formatTokenCount(m.output_tokens) + '</span>'
          + '</div>';
      }
    }

    popover.innerHTML = html;
  }).catch(function() {});
}

// Gateway popover is now inline in the user dropdown — no hover toggle needed.
// The popover content is updated by startGatewayStatusPolling() into #gateway-popover.

// --- TEE attestation ---

let teeInfo = null;
let teeReportCache = null;
let teeReportLoading = false;

function teeApiBase() {
    var hostname = window.location.hostname;
    // Skip IP addresses (IPv4 and IPv6) and localhost
    if (hostname === "localhost" || /^(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)$/.test(hostname) || hostname.indexOf(":") !== -1) {
        return null;
    }
    var parts = hostname.split(".");
    if (parts.length < 2) return null;
    var domain = parts.slice(1).join(".");
    return window.location.protocol + "//api." + domain;
}

function teeInstanceName() {
  return window.location.hostname.split('.')[0];
}

function checkTeeStatus() {
  var base = teeApiBase();
  if (!base) return;
  var name = teeInstanceName();
  try {
    fetch(base + '/instances/' + encodeURIComponent(name) + '/attestation').then(function(res) {
      if (!res.ok) throw new Error(res.status);
      return res.json();
    }).then(function(data) {
      teeInfo = data;
      document.getElementById('tee-shield').style.display = 'flex';
    }).catch(function(err) {
      console.warn('Failed to fetch TEE attestation:', err);
    });
  } catch (e) {
    console.warn("Failed to check TEE status:", e);
  }
}

function fetchTeeReport() {
  if (teeReportCache) {
    renderTeePopover(teeReportCache);
    return;
  }
  if (teeReportLoading) return;
  teeReportLoading = true;
  var base = teeApiBase();
  if (!base) return;
  var popover = document.getElementById('tee-popover');
  popover.innerHTML = '<div class="tee-popover-loading">Loading attestation report...</div>';
  fetch(base + '/attestation/report').then(function(res) {
    if (!res.ok) throw new Error(res.status);
    return res.json();
  }).then(function(data) {
    teeReportCache = data;
    renderTeePopover(data);
  }).catch(function() {
    popover.innerHTML = '<div class="tee-popover-loading">Could not load attestation report</div>';
  }).finally(function() {
    teeReportLoading = false;
  });
}

function renderTeePopover(report) {
  var popover = document.getElementById('tee-popover');
  var na = I18n.t('common.noData');
  var digest = (teeInfo && teeInfo.image_digest) || na;
  var fingerprint = report.tls_certificate_fingerprint || na;
  var reportData = report.report_data || '';
  var vmConfig = report.vm_config || na;
  var truncated = reportData.length > 32 ? reportData.slice(0, 32) + '...' : reportData;
  popover.innerHTML = '<div class="tee-popover-title">'
    + '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>'
    + 'TEE Attestation</div>'
    + '<div class="tee-field"><div class="tee-field-label">Image Digest</div>'
    + '<div class="tee-field-value">' + escapeHtml(digest) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">TLS Certificate Fingerprint</div>'
    + '<div class="tee-field-value">' + escapeHtml(fingerprint) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">Report Data</div>'
    + '<div class="tee-field-value">' + escapeHtml(truncated) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">VM Config</div>'
    + '<div class="tee-field-value">' + escapeHtml(vmConfig) + '</div></div>'
    + '<div class="tee-popover-actions">'
    + '<button class="tee-btn-copy" data-action="copy-tee-report">Copy Full Report</button></div>';
}

function copyTeeReport() {
  if (!teeReportCache) return;
  var combined = Object.assign({}, teeReportCache, teeInfo || {});
  navigator.clipboard.writeText(JSON.stringify(combined, null, 2)).then(function() {
    showToast(I18n.t('tee.reportCopied'), 'success');
  }).catch(function() {
    showToast(I18n.t('tee.copyFailed'), 'error');
  });
}

document.getElementById('tee-shield').addEventListener('mouseenter', function() {
  fetchTeeReport();
  document.getElementById('tee-popover').classList.add('visible');
});
document.getElementById('tee-shield').addEventListener('mouseleave', function() {
  document.getElementById('tee-popover').classList.remove('visible');
});

// --- Extension install ---

function installWasmExtension() {
  var name = document.getElementById('wasm-install-name').value.trim();
  if (!name) {
    showToast(I18n.t('extensions.nameRequired'), 'error');
    return;
  }
  var url = document.getElementById('wasm-install-url').value.trim();
  if (!url) {
    showToast(I18n.t('extensions.urlRequired'), 'error');
    return;
  }

  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: name, url: url, kind: 'wasm_tool' },
  }).then(function(res) {
    if (res.success) {
      showToast(I18n.t('extensions.installedName', { name: name }), 'success');
      document.getElementById('wasm-install-name').value = '';
      document.getElementById('wasm-install-url').value = '';
      loadExtensions();
    } else {
      showToast(I18n.t('extensions.installFailed', { message: res.message || 'unknown error' }), 'error');
    }
  }).catch(function(err) {
    showToast(I18n.t('extensions.installFailed', { message: err.message }), 'error');
  });
}

function addMcpServer() {
  var name = document.getElementById('mcp-install-name').value.trim();
  if (!name) {
    showToast(I18n.t('mcp.serverNameRequired'), 'error');
    return;
  }
  var url = document.getElementById('mcp-install-url').value.trim();
  if (!url) {
    showToast(I18n.t('mcp.urlRequired'), 'error');
    return;
  }

  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: name, url: url, kind: 'mcp_server' },
  }).then(function(res) {
    if (res.success) {
      showToast(I18n.t('mcp.added', { name: name }), 'success');
      document.getElementById('mcp-install-name').value = '';
      document.getElementById('mcp-install-url').value = '';
      loadMcpServers();
    } else {
      showToast(I18n.t('mcp.addFailed', { message: res.message || 'unknown error' }), 'error');
    }
  }).catch(function(err) {
    showToast(I18n.t('mcp.addFailed', { message: err.message }), 'error');
  });
}

// --- Skills ---

function loadSkills() {
  var skillsList = document.getElementById('skills-list');
  skillsList.innerHTML = renderCardsSkeleton(3);
  apiFetch('/api/skills').then(function(data) {
    if (!data.skills || data.skills.length === 0) {
      skillsList.innerHTML = '<div class="empty-state">' + I18n.t('skills.noInstalled') + '</div>';
      return;
    }
    skillsList.innerHTML = '';
    for (var i = 0; i < data.skills.length; i++) {
      skillsList.appendChild(renderSkillCard(data.skills[i]));
    }
  }).catch(function(err) {
    skillsList.innerHTML = '<div class="empty-state">' + I18n.t('skills.loadFailed', {message: escapeHtml(err.message)}) + '</div>';
  });
}

function renderSkillCard(skill) {
  var card = document.createElement('div');
  card.className = 'ext-card state-active';

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = skill.name;
  header.appendChild(name);

  var trust = document.createElement('span');
  var trustClass = skill.trust.toLowerCase() === 'trusted' ? 'trust-trusted' : 'trust-installed';
  trust.className = 'skill-trust ' + trustClass;
  trust.textContent = skill.trust;
  header.appendChild(trust);

  var version = document.createElement('span');
  version.className = 'skill-version';
  version.textContent = 'v' + skill.version;
  header.appendChild(version);

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = skill.description;
  card.appendChild(desc);

  if (skill.keywords && skill.keywords.length > 0) {
    var kw = document.createElement('div');
    kw.className = 'ext-keywords';
    kw.textContent = I18n.t('skills.activatesOn') + ': ' + skill.keywords.join(', ');
    card.appendChild(kw);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  // Only show Remove for registry-installed skills, not user-placed trusted skills
  if (skill.trust.toLowerCase() !== 'trusted') {
    var removeBtn = document.createElement('button');
    removeBtn.className = 'btn-ext remove';
    removeBtn.textContent = I18n.t('skills.remove');
    removeBtn.addEventListener('click', function() { removeSkill(skill.name); });
    actions.appendChild(removeBtn);
  }

  card.appendChild(actions);
  return card;
}

function searchClawHub() {
  var input = document.getElementById('skill-search-input');
  var query = input.value.trim();
  if (!query) return;

  var resultsDiv = document.getElementById('skill-search-results');
  resultsDiv.innerHTML = '<div class="empty-state">' + I18n.t('skills.searching') + '</div>';

  apiFetch('/api/skills/search', {
    method: 'POST',
    body: { query: query },
  }).then(function(data) {
    resultsDiv.innerHTML = '';

    // Show registry error as a warning banner if present
    if (data.catalog_error) {
      var warning = document.createElement('div');
      warning.className = 'empty-state';
      warning.style.color = '#f0ad4e';
      warning.style.borderLeft = '3px solid #f0ad4e';
      warning.style.paddingLeft = '12px';
      warning.style.marginBottom = '16px';
      warning.textContent = I18n.t('skills.registryError', {message: data.catalog_error});
      resultsDiv.appendChild(warning);
    }

    // Show catalog results
    if (data.catalog && data.catalog.length > 0) {
      // Build a set of installed skill names for quick lookup
      var installedNames = {};
      if (data.installed) {
        for (var j = 0; j < data.installed.length; j++) {
          installedNames[data.installed[j].name] = true;
        }
      }

      for (var i = 0; i < data.catalog.length; i++) {
        var card = renderCatalogSkillCard(data.catalog[i], installedNames);
        card.style.animationDelay = (i * 0.06) + 's';
        resultsDiv.appendChild(card);
      }
    }

    // Show matching installed skills too
    if (data.installed && data.installed.length > 0) {
      for (var k = 0; k < data.installed.length; k++) {
        var installedCard = renderSkillCard(data.installed[k]);
        installedCard.style.animationDelay = ((data.catalog ? data.catalog.length : 0) + k) * 0.06 + 's';
        installedCard.classList.add('skill-search-result');
        resultsDiv.appendChild(installedCard);
      }
    }

    if (resultsDiv.children.length === 0) {
      resultsDiv.innerHTML = '<div class="empty-state">' + I18n.t('skills.noResults', {query: escapeHtml(query)}) + '</div>';
    }
  }).catch(function(err) {
    resultsDiv.innerHTML = '<div class="empty-state">' + I18n.t('skills.searchFailed', {message: escapeHtml(err.message)}) + '</div>';
  });
}

function renderCatalogSkillCard(entry, installedNames) {
  var card = document.createElement('div');
  card.className = 'ext-card ext-available skill-search-result';

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('a');
  name.className = 'ext-name';
  name.textContent = entry.name || entry.slug;
  name.href = 'https://clawhub.ai/skills/' + encodeURIComponent(entry.slug);
  name.target = '_blank';
  name.rel = 'noopener noreferrer';
  name.style.textDecoration = 'none';
  name.style.color = 'inherit';
  name.title = I18n.t('skills.viewOnClawHub');
  header.appendChild(name);

  if (entry.version) {
    var version = document.createElement('span');
    version.className = 'skill-version';
    version.textContent = 'v' + entry.version;
    header.appendChild(version);
  }

  card.appendChild(header);

  if (entry.description) {
    var desc = document.createElement('div');
    desc.className = 'ext-desc';
    desc.textContent = entry.description;
    card.appendChild(desc);
  }

  // Metadata row: owner, stars, downloads, recency
  var meta = document.createElement('div');
  meta.className = 'ext-meta';
  meta.style.fontSize = '11px';
  meta.style.color = '#888';
  meta.style.marginTop = '6px';

  function addMetaSep() {
    if (meta.children.length > 0) {
      meta.appendChild(document.createTextNode(' \u00b7 '));
    }
  }

  if (entry.owner) {
    var ownerSpan = document.createElement('span');
    ownerSpan.textContent = 'by ' + entry.owner;
    meta.appendChild(ownerSpan);
  }

  if (entry.stars != null) {
    addMetaSep();
    var starsSpan = document.createElement('span');
    starsSpan.textContent = entry.stars + ' stars';
    meta.appendChild(starsSpan);
  }

  if (entry.downloads != null) {
    addMetaSep();
    var dlSpan = document.createElement('span');
    dlSpan.textContent = formatCompactNumber(entry.downloads) + ' downloads';
    meta.appendChild(dlSpan);
  }

  if (entry.updatedAt) {
    var ago = formatTimeAgo(entry.updatedAt);
    if (ago) {
      addMetaSep();
      var updatedSpan = document.createElement('span');
      updatedSpan.textContent = 'updated ' + ago;
      meta.appendChild(updatedSpan);
    }
  }

  if (meta.children.length > 0) {
    card.appendChild(meta);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  var slug = entry.slug || entry.name;
  var slugSuffix = slug.indexOf('/') >= 0 ? slug.split('/').pop() : slug;
  var isInstalled = entry.installed || installedNames[entry.name] || installedNames[slug] || installedNames[slugSuffix];

  if (isInstalled) {
    var label = document.createElement('span');
    label.className = 'ext-active-label';
    label.textContent = I18n.t('status.installed');
    actions.appendChild(label);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-ext install';
    installBtn.textContent = I18n.t('extensions.install');
    installBtn.addEventListener('click', (function(displayName, slugValue, btn) {
      return function() {
        if (!confirm(I18n.t('skills.confirmInstallHub', { name: displayName }))) return;
        btn.disabled = true;
        btn.textContent = I18n.t('extensions.installing');
        installSkill(displayName, null, btn, slugValue);
      };
    })(entry.name || slug, slug, installBtn));
    actions.appendChild(installBtn);
  }

  card.appendChild(actions);
  return card;
}

function formatCompactNumber(n) {
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
  return '' + n;
}

function formatTimeAgo(epochMs) {
  var now = Date.now();
  var diff = now - epochMs;
  if (diff < 0) return null;
  var minutes = Math.floor(diff / 60000);
  if (minutes < 60) return minutes <= 1 ? 'just now' : minutes + 'm ago';
  var hours = Math.floor(minutes / 60);
  if (hours < 24) return hours + 'h ago';
  var days = Math.floor(hours / 24);
  if (days < 30) return days + 'd ago';
  var months = Math.floor(days / 30);
  if (months < 12) return months + 'mo ago';
  return Math.floor(months / 12) + 'y ago';
}

function installSkill(name, url, btn, slug) {
  var body = { name: name };
  if (slug) body.slug = slug;
  if (url) body.url = url;

  apiFetch('/api/skills/install', {
    method: 'POST',
    headers: { 'X-Confirm-Action': 'true' },
    body: body,
  }).then(function(res) {
    if (res.success) {
      showToast(I18n.t('skills.installedSuccess', {name: name}), 'success');
      if (btn && btn.parentNode) {
        var label = document.createElement('span');
        label.className = 'ext-active-label';
        label.textContent = I18n.t('status.installed');
        btn.parentNode.innerHTML = '';
        btn.parentNode.appendChild(label);
      }
    } else {
      showToast(I18n.t('extensions.installFailed', { message: res.message || 'unknown error' }), 'error');
    }
    loadSkills();
    if (btn && !res.success) { btn.disabled = false; btn.textContent = I18n.t('extensions.install'); }
  }).catch(function(err) {
    showToast(I18n.t('extensions.installFailed', { message: err.message }), 'error');
    if (btn) { btn.disabled = false; btn.textContent = I18n.t('extensions.install'); }
  });
}

function removeSkill(name) {
  showConfirmModal(I18n.t('skills.confirmRemove', { name: name }), '', function() {
    apiFetch('/api/skills/' + encodeURIComponent(name), {
      method: 'DELETE',
      headers: { 'X-Confirm-Action': 'true' },
    }).then(function(res) {
      if (res.success) {
        showToast(I18n.t('skills.removed', { name: name }), 'success');
      } else {
        showToast(I18n.t('skills.removeFailed', { message: res.message || 'unknown error' }), 'error');
      }
      loadSkills();
    }).catch(function(err) {
      showToast(I18n.t('skills.removeFailed', { message: err.message }), 'error');
    });
  }, I18n.t('common.remove'), 'btn-danger');
}

function installSkillFromForm() {
  var name = document.getElementById('skill-install-name').value.trim();
  if (!name) { showToast(I18n.t('skills.nameRequired'), 'error'); return; }
  var url = document.getElementById('skill-install-url').value.trim() || null;
  if (url && !url.startsWith('https://')) {
    showToast(I18n.t('skills.httpsRequired'), 'error');
    return;
  }
  if (!confirm(I18n.t('skills.confirmInstall', { name: name }))) return;
  installSkill(name, url, null);
  document.getElementById('skill-install-name').value = '';
  document.getElementById('skill-install-url').value = '';
}

// Wire up Enter key on search input
document.getElementById('skill-search-input').addEventListener('keydown', function(e) {
  if (e.key === 'Enter') searchClawHub();
});

// --- Tool Permissions ---

function loadToolsPermissions() {
  var container = document.getElementById('tools-permissions-list');
  if (!container) return;
  container.innerHTML = '<div class="empty-state">' + I18n.t('common.loading') + '</div>';
  apiFetch('/api/settings/tools').then(function(data) {
    if (!data.tools || data.tools.length === 0) {
      container.innerHTML = '<div class="empty-state">' + I18n.t('tools.noTools') + '</div>';
      return;
    }
    container.innerHTML = '';
    for (var i = 0; i < data.tools.length; i++) {
      container.appendChild(renderToolPermissionRow(data.tools[i]));
    }
  }).catch(function(err) {
    container.innerHTML = '<div class="empty-state">' + I18n.t('common.loadFailed') + ': ' + escapeHtml(err.message) + '</div>';
  });
}

function renderToolPermissionRow(tool) {
  var row = document.createElement('div');
  row.className = 'tool-permission-row';
  row.dataset.toolName = tool.name;

  // Left: name + description
  var info = document.createElement('div');
  info.className = 'tool-permission-info';

  var name = document.createElement('span');
  name.className = 'tool-permission-name';
  name.textContent = tool.name;

  var desc = document.createElement('span');
  desc.className = 'tool-permission-desc';
  desc.textContent = tool.description;

  info.appendChild(name);
  info.appendChild(desc);

  // Right: lock icon or toggle + default badge
  var controls = document.createElement('div');
  controls.className = 'tool-permission-controls';

  if (tool.locked) {
    var lock = document.createElement('span');
    lock.className = 'tool-lock-icon';
    lock.title = I18n.t('tools.lockedTooltip');
    lock.textContent = '\uD83D\uDD12';
    controls.appendChild(lock);
  } else {
    var toggle = document.createElement('div');
    toggle.className = 'tool-permission-toggle';

    var states = [
      { value: 'always_allow', label: I18n.t('tools.alwaysAllow') },
      { value: 'ask_each_time', label: I18n.t('tools.askEachTime') },
      { value: 'disabled', label: I18n.t('tools.disabled') },
    ];

    for (var j = 0; j < states.length; j++) {
      (function(state) {
        var btn = document.createElement('button');
        btn.textContent = state.label;
        btn.dataset.state = state.value;
        btn.setAttribute('aria-pressed', tool.current_state === state.value);
        if (tool.current_state === state.value) btn.classList.add('active');
        btn.addEventListener('click', function() {
          setToolPermission(tool.name, state.value, row);
        });
        toggle.appendChild(btn);
      })(states[j]);
    }

    controls.appendChild(toggle);
  }

  if (tool.current_state === tool.default_state) {
    var badge = document.createElement('span');
    badge.className = 'tool-default-badge';
    badge.textContent = I18n.t('tools.defaultBadge');
    controls.appendChild(badge);
  }

  row.appendChild(info);
  row.appendChild(controls);
  return row;
}

function setToolPermission(toolName, newState, rowEl) {
  apiFetch('/api/settings/tools/' + encodeURIComponent(toolName), {
    method: 'PUT',
    body: { state: newState },
  }).then(function(updated) {
    // Re-render just this row in-place.
    var newRow = renderToolPermissionRow(updated);
    if (rowEl && rowEl.parentNode) {
      rowEl.parentNode.replaceChild(newRow, rowEl);
    }
  }).catch(function(err) {
    showToast(I18n.t('tools.saveFailed', { message: err.message }), 'error');
  });
}

// --- Keyboard shortcuts ---

document.addEventListener('keydown', (e) => {
  const mod = e.metaKey || e.ctrlKey;
  const tag = (e.target.tagName || '').toLowerCase();
  const inInput = tag === 'input' || tag === 'textarea';

  // Mod+1-5: switch tabs
  if (mod && e.key >= '1' && e.key <= '5') {
    e.preventDefault();
    const tabs = ['chat', 'memory', 'jobs', 'routines', 'settings'];
    const idx = parseInt(e.key) - 1;
    if (tabs[idx]) switchTab(tabs[idx]);
    return;
  }

  // Mod+K: focus chat input or memory search
  if (mod && e.key === 'k') {
    e.preventDefault();
    if (currentTab === 'memory') {
      document.getElementById('memory-search').focus();
    } else {
      document.getElementById('chat-input').focus();
    }
    return;
  }

  // Mod+N: new thread
  if (mod && e.key === 'n' && currentTab === 'chat') {
    e.preventDefault();
    createNewThread();
    return;
  }

  // Mod+/: toggle shortcuts overlay
  if (mod && e.key === '/') {
    e.preventDefault();
    toggleShortcutsOverlay();
    return;
  }

  // Escape: close modals, autocomplete, job detail, or blur input
  if (e.key === 'Escape') {
    const acEl = document.getElementById('slash-autocomplete');
    if (acEl && acEl.style.display !== 'none') {
      hideSlashAutocomplete();
      return;
    }
    // Close shortcuts overlay if open
    const shortcutsOverlay = document.getElementById('shortcuts-overlay');
    if (shortcutsOverlay?.style.display === 'flex') {
      shortcutsOverlay.style.display = 'none';
      return;
    }
    closeModals();
    if (currentJobId) {
      closeJobDetail();
    } else if (inInput) {
      e.target.blur();
    }
    return;
  }
});

// --- Settings Tab ---

document.querySelectorAll('.settings-subtab').forEach(function(btn) {
  btn.addEventListener('click', function() {
    switchSettingsSubtab(btn.getAttribute('data-settings-subtab'));
  });
});

function switchSettingsSubtab(subtab) {
  currentSettingsSubtab = subtab;
  document.querySelectorAll('.settings-subtab').forEach(function(b) {
    b.classList.toggle('active', b.getAttribute('data-settings-subtab') === subtab);
  });
  document.querySelectorAll('.settings-subpanel').forEach(function(p) {
    p.classList.toggle('active', p.id === 'settings-' + subtab);
  });
  // Clear search when switching subtabs so stale filters don't apply
  var searchInput = document.getElementById('settings-search-input');
  if (searchInput && searchInput.value) {
    searchInput.value = '';
    searchInput.dispatchEvent(new Event('input'));
  }
  // On mobile, drill into detail view
  if (window.innerWidth <= 768) {
    document.querySelector('.settings-layout').classList.add('settings-detail-active');
  }
  loadSettingsSubtab(subtab);
  updateHash();
}

function settingsBack() {
  document.querySelector('.settings-layout').classList.remove('settings-detail-active');
}

function loadSettingsSubtab(subtab) {
  if (subtab === 'inference') loadInferenceSettings();
  else if (subtab === 'agent') loadAgentSettings();
  else if (subtab === 'channels') { loadChannelsStatus(); startPairingPoll(); }
  else if (subtab === 'networking') loadNetworkingSettings();
  else if (subtab === 'extensions') { loadExtensions(); startPairingPoll(); }
  else if (subtab === 'mcp') loadMcpServers();
  else if (subtab === 'skills') loadSkills();
  else if (subtab === 'users') loadUsers();
  else if (subtab === 'tools') loadToolsPermissions();
  if (subtab !== 'extensions' && subtab !== 'channels') stopPairingPoll();
}

// --- Structured Settings Definitions ---

var INFERENCE_SETTINGS = [
  {
    group: 'cfg.group.inference',
    settings: [
      { key: 'temperature', label: 'cfg.temperature.label', description: 'cfg.temperature.desc', type: 'float', min: 0, max: 2, step: 0.1 },
    ]
  },
  {
    group: 'cfg.group.embeddings',
    settings: [
      { key: 'embeddings.enabled', label: 'cfg.embeddings_enabled.label', description: 'cfg.embeddings_enabled.desc', type: 'boolean' },
      { key: 'embeddings.provider', label: 'cfg.embeddings_provider.label', description: 'cfg.embeddings_provider.desc',
        type: 'select', options: ['openai', 'nearai'] },
      { key: 'embeddings.model', label: 'cfg.embeddings_model.label', description: 'cfg.embeddings_model.desc', type: 'text' },
    ]
  },
];

var AGENT_SETTINGS = [
  {
    group: 'cfg.group.agent',
    settings: [
      { key: 'agent.name', label: 'cfg.agent_name.label', description: 'cfg.agent_name.desc', type: 'text' },
      { key: 'agent.max_parallel_jobs', label: 'cfg.agent_max_parallel_jobs.label', description: 'cfg.agent_max_parallel_jobs.desc', type: 'number' },
      { key: 'agent.job_timeout_secs', label: 'cfg.agent_job_timeout.label', description: 'cfg.agent_job_timeout.desc', type: 'number' },
      { key: 'agent.max_tool_iterations', label: 'cfg.agent_max_tool_iterations.label', description: 'cfg.agent_max_tool_iterations.desc', type: 'number' },
      { key: 'agent.use_planning', label: 'cfg.agent_use_planning.label', description: 'cfg.agent_use_planning.desc', type: 'boolean' },
      { key: 'agent.auto_approve_tools', label: 'cfg.agent_auto_approve.label', description: 'cfg.agent_auto_approve.desc', type: 'boolean' },
      { key: 'agent.default_timezone', label: 'cfg.agent_timezone.label', description: 'cfg.agent_timezone.desc', type: 'text' },
      { key: 'agent.session_idle_timeout_secs', label: 'cfg.agent_session_idle.label', description: 'cfg.agent_session_idle.desc', type: 'number' },
      { key: 'agent.stuck_threshold_secs', label: 'cfg.agent_stuck_threshold.label', description: 'cfg.agent_stuck_threshold.desc', type: 'number' },
      { key: 'agent.max_repair_attempts', label: 'cfg.agent_max_repair.label', description: 'cfg.agent_max_repair.desc', type: 'number' },
      { key: 'agent.max_cost_per_day_cents', label: 'cfg.agent_max_cost.label', description: 'cfg.agent_max_cost.desc', type: 'number', min: 0 },
      { key: 'agent.max_actions_per_hour', label: 'cfg.agent_max_actions.label', description: 'cfg.agent_max_actions.desc', type: 'number', min: 0 },
      { key: 'agent.allow_local_tools', label: 'cfg.agent_allow_local.label', description: 'cfg.agent_allow_local.desc', type: 'boolean' },
    ]
  },
  {
    group: 'cfg.group.heartbeat',
    settings: [
      { key: 'heartbeat.enabled', label: 'cfg.heartbeat_enabled.label', description: 'cfg.heartbeat_enabled.desc', type: 'boolean' },
      { key: 'heartbeat.interval_secs', label: 'cfg.heartbeat_interval.label', description: 'cfg.heartbeat_interval.desc', type: 'number' },
      { key: 'heartbeat.notify_channel', label: 'cfg.heartbeat_notify_channel.label', description: 'cfg.heartbeat_notify_channel.desc', type: 'text' },
      { key: 'heartbeat.notify_user', label: 'cfg.heartbeat_notify_user.label', description: 'cfg.heartbeat_notify_user.desc', type: 'text' },
      { key: 'heartbeat.quiet_hours_start', label: 'cfg.heartbeat_quiet_start.label', description: 'cfg.heartbeat_quiet_start.desc', type: 'number', min: 0, max: 23 },
      { key: 'heartbeat.quiet_hours_end', label: 'cfg.heartbeat_quiet_end.label', description: 'cfg.heartbeat_quiet_end.desc', type: 'number', min: 0, max: 23 },
      { key: 'heartbeat.timezone', label: 'cfg.heartbeat_timezone.label', description: 'cfg.heartbeat_timezone.desc', type: 'text' },
    ]
  },
  {
    group: 'cfg.group.sandbox',
    settings: [
      { key: 'sandbox.enabled', label: 'cfg.sandbox_enabled.label', description: 'cfg.sandbox_enabled.desc', type: 'boolean' },
      { key: 'sandbox.policy', label: 'cfg.sandbox_policy.label', description: 'cfg.sandbox_policy.desc',
        type: 'select', options: ['readonly', 'workspace_write', 'full_access'] },
      { key: 'sandbox.timeout_secs', label: 'cfg.sandbox_timeout.label', description: 'cfg.sandbox_timeout.desc', type: 'number', min: 0 },
      { key: 'sandbox.memory_limit_mb', label: 'cfg.sandbox_memory.label', description: 'cfg.sandbox_memory.desc', type: 'number', min: 0 },
      { key: 'sandbox.image', label: 'cfg.sandbox_image.label', description: 'cfg.sandbox_image.desc', type: 'text' },
    ]
  },
  {
    group: 'cfg.group.routines',
    settings: [
      { key: 'routines.max_concurrent', label: 'cfg.routines_max_concurrent.label', description: 'cfg.routines_max_concurrent.desc', type: 'number', min: 0 },
      { key: 'routines.default_cooldown_secs', label: 'cfg.routines_cooldown.label', description: 'cfg.routines_cooldown.desc', type: 'number', min: 0 },
    ]
  },
  {
    group: 'cfg.group.safety',
    settings: [
      { key: 'safety.max_output_length', label: 'cfg.safety_max_output.label', description: 'cfg.safety_max_output.desc', type: 'number', min: 0 },
      { key: 'safety.injection_check_enabled', label: 'cfg.safety_injection_check.label', description: 'cfg.safety_injection_check.desc', type: 'boolean' },
    ]
  },
  {
    group: 'cfg.group.skills',
    settings: [
      { key: 'skills.max_active', label: 'cfg.skills_max_active.label', description: 'cfg.skills_max_active.desc', type: 'number', min: 0 },
      { key: 'skills.max_context_tokens', label: 'cfg.skills_max_tokens.label', description: 'cfg.skills_max_tokens.desc', type: 'number', min: 0 },
    ]
  },
  {
    group: 'cfg.group.search',
    settings: [
      { key: 'search.fusion_strategy', label: 'cfg.search_fusion.label', description: 'cfg.search_fusion.desc',
        type: 'select', options: ['rrf', 'weighted'] },
    ]
  },
];

function renderSettingsSkeleton(rows) {
  var html = '<div class="settings-group" style="border:none;background:none">';
  for (var i = 0; i < (rows || 5); i++) {
    var w1 = 100 + Math.floor(Math.random() * 60);
    var w2 = 140 + Math.floor(Math.random() * 60);
    html += '<div class="skeleton-row"><div class="skeleton-bar" style="width:' + w1 + 'px"></div><div class="skeleton-bar" style="width:' + w2 + 'px"></div></div>';
  }
  html += '</div>';
  return html;
}

function renderCardsSkeleton(count) {
  var html = '';
  for (var i = 0; i < (count || 3); i++) {
    html += '<div class="skeleton-card"><div class="skeleton-bar" style="width:60%;height:14px"></div><div class="skeleton-bar" style="width:90%;height:10px"></div><div class="skeleton-bar" style="width:40%;height:10px"></div></div>';
  }
  return html;
}

function renderSkeleton(type, count) {
  count = count || 3;
  var container = document.createElement('div');
  container.className = 'skeleton-container';
  for (var i = 0; i < count; i++) {
    var el = document.createElement('div');
    el.className = 'skeleton-' + type;
    el.innerHTML = '<div class="skeleton-bar shimmer"></div>';
    container.appendChild(el);
  }
  return container;
}

function loadInferenceSettings() {
  var container = document.getElementById('settings-inference-content');
  container.innerHTML = renderSettingsSkeleton(6);

  Promise.all([
    apiFetch('/api/settings/export'),
    apiFetch('/api/gateway/status').catch(function() { return {}; }),
  ]).then(function(results) {
    var settings = results[0].settings || {};
    var status = results[1];
    container.innerHTML = '';

    // LLM Provider display — derived from active Model Provider
    var activeBackend = settings['llm_backend'] || status.llm_backend || 'nearai';
    var activeModel = settings['selected_model'] || status.llm_model || '';
    var allP = _builtinProviders;
    var customP = [];
    try {
      var cpVal = settings['llm_custom_providers'];
      customP = Array.isArray(cpVal) ? cpVal : (cpVal ? JSON.parse(cpVal) : []);
    } catch (e) { customP = []; }
    var provider = allP.concat(customP).find(function(p) { return p.id === activeBackend; });
    var providerName = provider ? (provider.name || provider.id) : activeBackend;
    if (!activeModel && provider) activeModel = provider.default_model || '';

    var group = document.createElement('div');
    group.className = 'settings-group';
    var title = document.createElement('div');
    title.className = 'settings-group-title';
    title.textContent = I18n.t('cfg.group.llm');
    group.appendChild(title);

    var notice = document.createElement('div');
    notice.className = 'config-notice';
    notice.id = 'llm-restart-notice';
    var restartNoticeEl = document.getElementById('config-restart-notice');
    notice.style.display = (restartNoticeEl && restartNoticeEl.style.display !== 'none') ? 'flex' : 'none';
    notice.innerHTML = '<span>\u26A0</span><span>' + escapeHtml(I18n.t('config.restartNotice')) + '</span>';
    group.appendChild(notice);

    var backendRow = document.createElement('div');
    backendRow.className = 'settings-row';
    backendRow.innerHTML =
      '<div class="settings-label-wrap"><label class="settings-label">' + escapeHtml(I18n.t('cfg.llm_backend.label')) + '</label>' +
      '<div class="settings-description">' + escapeHtml(I18n.t('cfg.llm_backend.desc')) + '</div></div>' +
      '<div class="settings-display-value">' + escapeHtml(providerName) + '</div>';
    group.appendChild(backendRow);

    var modelRow = document.createElement('div');
    modelRow.className = 'settings-row';
    modelRow.innerHTML =
      '<div class="settings-label-wrap"><label class="settings-label">' + escapeHtml(I18n.t('cfg.selected_model.label')) + '</label>' +
      '<div class="settings-description">' + escapeHtml(I18n.t('cfg.selected_model.desc')) + '</div></div>' +
      '<div class="settings-display-value">' + escapeHtml(activeModel || '\u2014') + '</div>';
    group.appendChild(modelRow);

    container.appendChild(group);

    // Remaining editable settings (embeddings, etc.)
    renderStructuredSettingsInto(container, INFERENCE_SETTINGS, settings, {});
    loadConfig();
  }).catch(function(err) {
    container.innerHTML = '<div class="empty-state">' + I18n.t('common.loadFailed') + ': '
      + escapeHtml(err.message) + '</div>';
    loadConfig();
  });
}

function loadAgentSettings() {
  loadStructuredSettings('settings-agent-content', AGENT_SETTINGS);
}

function loadStructuredSettings(containerId, settingsDefs) {
  var container = document.getElementById(containerId);
  container.innerHTML = renderSettingsSkeleton(8);

  apiFetch('/api/settings/export').then(function(data) {
    var settings = data.settings || {};
    container.innerHTML = '';
    renderStructuredSettingsInto(container, settingsDefs, settings, {});
  }).catch(function(err) {
    container.innerHTML = '<div class="empty-state">' + I18n.t('common.loadFailed') + ': '
      + escapeHtml(err.message) + '</div>';
  });
}

function renderStructuredSettingsInto(container, settingsDefs, settings, activeValues) {
    for (var gi = 0; gi < settingsDefs.length; gi++) {
      var groupDef = settingsDefs[gi];
      var group = document.createElement('div');
      group.className = 'settings-group';

      var title = document.createElement('div');
      title.className = 'settings-group-title';
      title.textContent = I18n.t(groupDef.group);
      group.appendChild(title);

      var rows = [];
      for (var si = 0; si < groupDef.settings.length; si++) {
        var def = groupDef.settings[si];
        var activeVal = activeValues ? activeValues[def.key] : undefined;
        var row = renderStructuredSettingsRow(def, settings[def.key], activeVal);
        if (def.showWhen) {
          row.setAttribute('data-show-when-key', def.showWhen.key);
          row.setAttribute('data-show-when-value', def.showWhen.value);
          var currentVal = settings[def.showWhen.key];
          if (currentVal === def.showWhen.value) {
            row.classList.remove('hidden');
          } else {
            row.classList.add('hidden');
          }
        }
        rows.push(row);
        group.appendChild(row);
      }

      container.appendChild(group);

      // Wire up showWhen reactivity for select fields in this group
      (function(groupRows, allSettings) {
        for (var ri = 0; ri < groupRows.length; ri++) {
          var sel = groupRows[ri].querySelector('.settings-select');
          if (sel) {
            sel.addEventListener('change', function() {
              var changedKey = this.getAttribute('data-setting-key');
              var changedVal = this.value;
              for (var rj = 0; rj < groupRows.length; rj++) {
                var whenKey = groupRows[rj].getAttribute('data-show-when-key');
                var whenVal = groupRows[rj].getAttribute('data-show-when-value');
                if (whenKey === changedKey) {
                  if (changedVal === whenVal) {
                    groupRows[rj].classList.remove('hidden');
                  } else {
                    groupRows[rj].classList.add('hidden');
                  }
                }
              }
            });
          }
        }
      })(rows, settings);
    }

    if (container.children.length === 0) {
      container.innerHTML = '<div class="empty-state">' + I18n.t('settings.noSettings') + '</div>';
    }
}

function renderStructuredSettingsRow(def, value, activeValue) {
  var row = document.createElement('div');
  row.className = 'settings-row';

  var labelWrap = document.createElement('div');
  labelWrap.className = 'settings-label-wrap';

  var label = document.createElement('div');
  label.className = 'settings-label';
  label.textContent = I18n.t(def.label);
  labelWrap.appendChild(label);

  if (def.description) {
    var desc = document.createElement('div');
    desc.className = 'settings-description';
    desc.textContent = I18n.t(def.description);
    labelWrap.appendChild(desc);
  }

  row.appendChild(labelWrap);

  var inputWrap = document.createElement('div');
  inputWrap.style.display = 'flex';
  inputWrap.style.alignItems = 'center';
  inputWrap.style.gap = '8px';

  var ariaLabel = I18n.t(def.label) + (def.description ? '. ' + I18n.t(def.description) : '');
  function formatSettingValue(raw) {
    if (Array.isArray(raw)) return raw.join(', ');
    if (raw === null || raw === undefined) return '';
    return String(raw);
  }

  var activeValueText = formatSettingValue(activeValue);
  var placeholderText = activeValueText ? I18n.t('settings.envValue', { value: activeValueText }) : (def.placeholder || I18n.t('settings.envDefault'));

  if (def.type === 'boolean') {
    var toggle = document.createElement('div');
    toggle.className = 'toggle-switch' + (value === 'true' || value === true ? ' on' : '');
    toggle.setAttribute('role', 'switch');
    toggle.setAttribute('aria-checked', value === 'true' || value === true ? 'true' : 'false');
    toggle.setAttribute('aria-label', ariaLabel);
    toggle.setAttribute('tabindex', '0');

    var savedIndicator = document.createElement('span');
    savedIndicator.className = 'settings-saved-indicator';
    savedIndicator.textContent = I18n.t('settings.saved');

    toggle.addEventListener('click', function() {
      var isOn = this.classList.toggle('on');
      this.setAttribute('aria-checked', isOn ? 'true' : 'false');
      saveSetting(def.key, isOn ? 'true' : 'false', savedIndicator);
    });
    toggle.addEventListener('keydown', function(e) {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        this.click();
      }
    });
    inputWrap.appendChild(toggle);
    inputWrap.appendChild(savedIndicator);
  } else if (def.type === 'select' && def.options) {
    var sel = document.createElement('select');
    sel.className = 'settings-select';
    sel.setAttribute('data-setting-key', def.key);
    sel.setAttribute('aria-label', ariaLabel);
    var emptyOpt = document.createElement('option');
    emptyOpt.value = '';
    emptyOpt.textContent = activeValue ? '\u2014 ' + I18n.t('settings.envValue', { value: activeValue }) + ' \u2014' : '\u2014 ' + I18n.t('settings.useEnvDefault') + ' \u2014';
    if (!value && value !== false && value !== 0) emptyOpt.selected = true;
    sel.appendChild(emptyOpt);
    for (var oi = 0; oi < def.options.length; oi++) {
      var opt = document.createElement('option');
      opt.value = def.options[oi];
      opt.textContent = def.options[oi];
      if (String(value) === def.options[oi]) opt.selected = true;
      sel.appendChild(opt);
    }
    sel.addEventListener('change', (function(k, el) {
      return function() { saveSetting(k, el.value === '' ? null : el.value); };
    })(def.key, sel));
    inputWrap.appendChild(sel);
  } else if (def.type === 'number' || def.type === 'float') {
    var numInp = document.createElement('input');
    numInp.type = 'number';
    numInp.step = def.step !== undefined ? String(def.step) : (def.type === 'float' ? 'any' : '1');
    numInp.className = 'settings-input';
    numInp.setAttribute('aria-label', ariaLabel);
    numInp.value = (value === null || value === undefined) ? '' : value;
    if (!value && value !== 0) numInp.placeholder = placeholderText;
    if (def.min !== undefined) numInp.min = def.min;
    if (def.max !== undefined) numInp.max = def.max;
    numInp.addEventListener('change', (function(k, el, isFloat) {
      return function() {
        if (el.value === '') return saveSetting(k, null);
        var parsed = isFloat ? parseFloat(el.value) : parseInt(el.value, 10);
        if (isNaN(parsed)) return;
        el.value = parsed;
        saveSetting(k, parsed);
      };
    })(def.key, numInp, def.type === 'float'));
    inputWrap.appendChild(numInp);
  } else if (def.type === 'list') {
    var listInp = document.createElement('input');
    listInp.type = 'text';
    listInp.className = 'settings-input';
    listInp.setAttribute('aria-label', ariaLabel);
    var listValue = '';
    if (Array.isArray(value)) listValue = value.join(', ');
    else if (typeof value === 'string') listValue = value;
    listInp.value = listValue;
    if (!listValue) listInp.placeholder = placeholderText;
    listInp.addEventListener('change', (function(k, el) {
      return function() {
        if (el.value.trim() === '') return saveSetting(k, null);
        var items = el.value.split(/[\n,]/).map(function(item) {
          return item.trim();
        }).filter(Boolean);
        saveSetting(k, items);
      };
    })(def.key, listInp));
    inputWrap.appendChild(listInp);
  } else {
    var textInp = document.createElement('input');
    textInp.type = 'text';
    textInp.className = 'settings-input';
    textInp.setAttribute('aria-label', ariaLabel);
    textInp.value = (value === null || value === undefined) ? '' : String(value);
    if (!value) textInp.placeholder = placeholderText;
    // Attach datalist for autocomplete suggestions (e.g., model list)
    if (def.suggestions && def.suggestions.length > 0) {
      var dlId = 'dl-' + def.key.replace(/\./g, '-');
      var dl = document.createElement('datalist');
      dl.id = dlId;
      for (var di = 0; di < def.suggestions.length; di++) {
        var dlOpt = document.createElement('option');
        dlOpt.value = def.suggestions[di];
        dl.appendChild(dlOpt);
      }
      textInp.setAttribute('list', dlId);
      inputWrap.appendChild(dl);
    }
    textInp.addEventListener('change', (function(k, el) {
      return function() { saveSetting(k, el.value === '' ? null : el.value); };
    })(def.key, textInp));
    inputWrap.appendChild(textInp);
  }

  var saved = document.createElement('span');
  saved.className = 'settings-saved-indicator';
  saved.textContent = '\u2713 ' + I18n.t('settings.saved');
  saved.setAttribute('data-key', def.key);
  saved.setAttribute('role', 'status');
  saved.setAttribute('aria-live', 'polite');
  inputWrap.appendChild(saved);

  row.appendChild(inputWrap);
  return row;
}

var RESTART_REQUIRED_KEYS = ['embeddings.enabled', 'embeddings.provider', 'embeddings.model',
  'agent.auto_approve_tools', 'tunnel.provider', 'tunnel.public_url', 'gateway.rate_limit', 'gateway.max_connections'];

var _settingsSavedTimers = {};

function saveSetting(key, value) {
  var method = (value === null || value === undefined) ? 'DELETE' : 'PUT';
  var opts = { method: method };
  if (method === 'PUT') opts.body = { value: value };
  apiFetch('/api/settings/' + encodeURIComponent(key), opts).then(function() {
    var indicator = document.querySelector('.settings-saved-indicator[data-key="' + key + '"]');
    if (indicator) {
      if (_settingsSavedTimers[key]) clearTimeout(_settingsSavedTimers[key]);
      indicator.classList.add('visible');
      _settingsSavedTimers[key] = setTimeout(function() { indicator.classList.remove('visible'); }, 2000);
    }
    // Show restart banner for inference settings
    if (RESTART_REQUIRED_KEYS.indexOf(key) !== -1) {
      showRestartBanner();
    }
  }).catch(function(err) {
    showToast(I18n.t('settings.saveFailed', { key: key, message: err.message }), 'error');
  });
}

function showRestartBanner() {
  var container = document.querySelector('.settings-content');
  if (!container || container.querySelector('.restart-banner')) return;
  var banner = document.createElement('div');
  banner.className = 'restart-banner';
  banner.setAttribute('role', 'alert');
  var textSpan = document.createElement('span');
  textSpan.className = 'restart-banner-text';
  textSpan.textContent = '\u26A0\uFE0F ' + I18n.t('settings.restartRequired');
  banner.appendChild(textSpan);
  var restartBtn = document.createElement('button');
  restartBtn.className = 'restart-banner-btn';
  restartBtn.textContent = I18n.t('settings.restartNow');
  restartBtn.addEventListener('click', function() { triggerRestart(); });
  banner.appendChild(restartBtn);
  container.insertBefore(banner, container.firstChild);
}

function loadMcpServers() {
  var mcpList = document.getElementById('mcp-servers-list');
  mcpList.innerHTML = renderCardsSkeleton(2);

  Promise.all([
    apiFetch('/api/extensions').catch(function() { return { extensions: [] }; }),
    apiFetch('/api/extensions/registry').catch(function() { return { entries: [] }; }),
  ]).then(function(results) {
    var extData = results[0];
    var registryData = results[1];
    var mcpEntries = (registryData.entries || []).filter(function(e) { return e.kind === 'mcp_server'; });
    var installedMcp = (extData.extensions || []).filter(function(e) { return e.kind === 'mcp_server'; });

    mcpList.innerHTML = '';
    var renderedNames = {};

    // Registry entries (cross-referenced with installed)
    for (var i = 0; i < mcpEntries.length; i++) {
      renderedNames[mcpEntries[i].name] = true;
      var installedExt = installedMcp.find(function(e) { return e.name === mcpEntries[i].name; });
      mcpList.appendChild(renderMcpServerCard(mcpEntries[i], installedExt));
    }

    // Custom installed MCP servers not in registry
    for (var j = 0; j < installedMcp.length; j++) {
      if (!renderedNames[installedMcp[j].name]) {
        mcpList.appendChild(renderExtensionCard(installedMcp[j]));
      }
    }

    if (mcpList.children.length === 0) {
      mcpList.innerHTML = '<div class="empty-state">' + I18n.t('mcp.noServers') + '</div>';
    }
  }).catch(function(err) {
    mcpList.innerHTML = '<div class="empty-state">' + I18n.t('common.loadFailed') + ': '
      + escapeHtml(err.message) + '</div>';
  });
}

function loadChannelsStatus() {
  var container = document.getElementById('settings-channels-content');
  container.innerHTML = renderCardsSkeleton(4);

  Promise.all([
    apiFetch('/api/gateway/status').catch(function() { return {}; }),
    apiFetch('/api/extensions').catch(function() { return { extensions: [] }; }),
    apiFetch('/api/extensions/registry').catch(function() { return { entries: [] }; }),
  ]).then(function(results) {
    var status = results[0];
    var extensions = results[1].extensions || [];
    var registry = results[2].entries || [];

    container.innerHTML = '';

    // Built-in Channels section
    var builtinSection = document.createElement('div');
    builtinSection.className = 'extensions-section';
    var builtinTitle = document.createElement('h3');
    builtinTitle.textContent = I18n.t('channels.builtin');
    builtinSection.appendChild(builtinTitle);
    var builtinList = document.createElement('div');
    builtinList.className = 'extensions-list';

    builtinList.appendChild(renderBuiltinChannelCard(
      I18n.t('channels.webGateway'),
      I18n.t('channels.webGatewayDesc'),
      true,
      'SSE: ' + (status.sse_connections || 0) + ' \u00B7 WS: ' + (status.ws_connections || 0)
    ));

    var enabledChannels = status.enabled_channels || [];

    builtinList.appendChild(renderBuiltinChannelCard(
      I18n.t('channels.httpWebhook'),
      I18n.t('channels.httpWebhookDesc'),
      enabledChannels.indexOf('http') !== -1,
      I18n.t('channels.configureVia', { env: 'ENABLE_HTTP=true' })
    ));

    builtinList.appendChild(renderBuiltinChannelCard(
      I18n.t('channels.cli'),
      I18n.t('channels.cliDesc'),
      enabledChannels.indexOf('cli') !== -1,
      I18n.t('channels.runWith', { cmd: 'ironclaw run --cli' })
    ));

    builtinList.appendChild(renderBuiltinChannelCard(
      I18n.t('channels.repl'),
      I18n.t('channels.replDesc'),
      enabledChannels.indexOf('repl') !== -1,
      I18n.t('channels.runWith', { cmd: 'ironclaw run --repl' })
    ));

    builtinSection.appendChild(builtinList);
    container.appendChild(builtinSection);

    // Messaging Channels section — use extension cards with full stepper/pairing UI
    var channelEntries = registry.filter(function(e) {
      return e.kind === 'wasm_channel' || e.kind === 'channel';
    });
    var installedChannels = extensions.filter(function(e) {
      return e.kind === 'wasm_channel';
    });

    if (channelEntries.length > 0 || installedChannels.length > 0) {
      var messagingSection = document.createElement('div');
      messagingSection.className = 'extensions-section';
      var messagingTitle = document.createElement('h3');
      messagingTitle.textContent = I18n.t('channels.messaging');
      messagingSection.appendChild(messagingTitle);
      var messagingList = document.createElement('div');
      messagingList.className = 'extensions-list';

      var renderedNames = {};

      // Registry entries: show full ext card if installed, available card if not
      for (var i = 0; i < channelEntries.length; i++) {
        var entry = channelEntries[i];
        renderedNames[entry.name] = true;
        var installed = null;
        for (var k = 0; k < installedChannels.length; k++) {
          if (installedChannels[k].name === entry.name) { installed = installedChannels[k]; break; }
        }
        if (installed) {
          messagingList.appendChild(renderExtensionCard(installed));
        } else {
          messagingList.appendChild(renderAvailableExtensionCard(entry));
        }
      }

      // Installed channels not in registry (custom installs)
      for (var j = 0; j < installedChannels.length; j++) {
        if (!renderedNames[installedChannels[j].name]) {
          messagingList.appendChild(renderExtensionCard(installedChannels[j]));
        }
      }

      messagingSection.appendChild(messagingList);
      container.appendChild(messagingSection);
    }
  });
}

function renderBuiltinChannelCard(name, description, active, detail) {
  var card = document.createElement('div');
  card.className = 'ext-card ' + (active ? 'state-active' : 'state-inactive');

  var header = document.createElement('div');
  header.className = 'ext-header';

  var nameEl = document.createElement('span');
  nameEl.className = 'ext-name';
  nameEl.textContent = name;
  header.appendChild(nameEl);

  var kindEl = document.createElement('span');
  kindEl.className = 'ext-kind kind-builtin';
  kindEl.textContent = I18n.t('ext.builtin');
  header.appendChild(kindEl);

  var statusDot = document.createElement('span');
  statusDot.className = 'ext-auth-dot ' + (active ? 'authed' : 'unauthed');
  statusDot.title = active ? I18n.t('ext.active') : I18n.t('ext.inactive');
  header.appendChild(statusDot);

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = description;
  card.appendChild(desc);

  if (detail) {
    var detailEl = document.createElement('div');
    detailEl.className = 'ext-url';
    detailEl.textContent = detail;
    card.appendChild(detailEl);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions';
  var label = document.createElement('span');
  label.className = 'ext-active-label';
  label.textContent = active ? I18n.t('ext.active') : I18n.t('ext.inactive');
  actions.appendChild(label);
  card.appendChild(actions);

  return card;
}

// --- Networking Settings ---

var NETWORKING_SETTINGS = [
  {
    group: 'cfg.group.tunnel',
    settings: [
      { key: 'tunnel.provider', label: 'cfg.tunnel_provider.label', description: 'cfg.tunnel_provider.desc',
        type: 'select', options: ['none', 'cloudflare', 'ngrok', 'tailscale', 'custom'] },
      { key: 'tunnel.public_url', label: 'cfg.tunnel_public_url.label', description: 'cfg.tunnel_public_url.desc', type: 'text' },
    ]
  },
  {
    group: 'cfg.group.gateway',
    settings: [
      { key: 'gateway.rate_limit', label: 'cfg.gateway_rate_limit.label', description: 'cfg.gateway_rate_limit.desc', type: 'number', min: 0 },
      { key: 'gateway.max_connections', label: 'cfg.gateway_max_connections.label', description: 'cfg.gateway_max_connections.desc', type: 'number', min: 0 },
    ]
  },
];

function loadNetworkingSettings() {
  var container = document.getElementById('settings-networking-content');
  container.innerHTML = renderSettingsSkeleton(4);

  apiFetch('/api/settings/export').then(function(data) {
    var settings = data.settings || {};
    container.innerHTML = '';
    renderStructuredSettingsInto(container, NETWORKING_SETTINGS, settings, {});
  }).catch(function(err) {
    container.innerHTML = '<div class="empty-state">' + I18n.t('common.loadFailed') + ': '
      + escapeHtml(err.message) + '</div>';
  });
}

// --- Toasts ---

function showToast(message, type) {
  const container = document.getElementById('toasts');
  const toast = document.createElement('div');
  toast.className = 'toast toast-' + (type || 'info');

  // Icon prefix
  const icon = document.createElement('span');
  icon.className = 'toast-icon';
  if (type === 'success') icon.textContent = '\u2713';
  else if (type === 'error') icon.textContent = '\u2717';
  else icon.textContent = '\u2139';
  toast.appendChild(icon);

  // Message text
  const text = document.createElement('span');
  text.textContent = message;
  toast.appendChild(text);

  // Countdown bar
  const countdown = document.createElement('div');
  countdown.className = 'toast-countdown';
  toast.appendChild(countdown);

  container.appendChild(toast);
  // Trigger slide-in
  requestAnimationFrame(() => toast.classList.add('visible'));
  setTimeout(() => {
    toast.classList.add('dismissing');
    toast.addEventListener('transitionend', () => toast.remove(), { once: true });
    // Fallback removal if transitionend doesn't fire
    setTimeout(() => { if (toast.parentNode) toast.remove(); }, 500);
  }, 4000);
}

// --- Welcome Card (Phase 4.2) ---

function showWelcomeCard() {
  const container = document.getElementById('chat-messages');
  if (!container || container.querySelector('.welcome-card')) return;
  const card = document.createElement('div');
  card.className = 'welcome-card';

  const heading = document.createElement('h2');
  heading.className = 'welcome-heading';
  heading.textContent = I18n.t('welcome.heading');
  card.appendChild(heading);

  const desc = document.createElement('p');
  desc.className = 'welcome-description';
  desc.textContent = I18n.t('welcome.description');
  card.appendChild(desc);

  const chips = document.createElement('div');
  chips.className = 'welcome-chips';

  const suggestions = [
    { key: 'welcome.runTool', fallback: 'Run a tool' },
    { key: 'welcome.checkJobs', fallback: 'Check job status' },
    { key: 'welcome.searchMemory', fallback: 'Search memory' },
    { key: 'welcome.manageRoutines', fallback: 'Manage routines' },
    { key: 'welcome.systemStatus', fallback: 'System status' },
    { key: 'welcome.writeCode', fallback: 'Write code' },
  ];
  suggestions.forEach(({ key, fallback }) => {
    const chip = document.createElement('button');
    chip.className = 'welcome-chip';
    chip.textContent = I18n.t(key) || fallback;
    chip.addEventListener('click', () => sendSuggestion(chip));
    chips.appendChild(chip);
  });

  card.appendChild(chips);
  container.appendChild(card);
}

function renderEmptyState({ icon, title, hint, action }) {
  const wrapper = document.createElement('div');
  wrapper.className = 'empty-state-card';

  if (icon) {
    const iconEl = document.createElement('div');
    iconEl.className = 'empty-state-icon';
    iconEl.textContent = icon;
    wrapper.appendChild(iconEl);
  }

  if (title) {
    const titleEl = document.createElement('div');
    titleEl.className = 'empty-state-title';
    titleEl.textContent = title;
    wrapper.appendChild(titleEl);
  }

  if (hint) {
    const hintEl = document.createElement('div');
    hintEl.className = 'empty-state-hint';
    hintEl.textContent = hint;
    wrapper.appendChild(hintEl);
  }

  if (action) {
    const btn = document.createElement('button');
    btn.className = 'empty-state-action';
    btn.textContent = action.label || 'Go';
    if (action.onClick) btn.addEventListener('click', action.onClick);
    wrapper.appendChild(btn);
  }

  return wrapper;
}

function sendSuggestion(btn) {
  const textarea = document.getElementById('chat-input');
  if (textarea) {
    textarea.value = btn.textContent;
    sendMessage();
  }
}

function removeWelcomeCard() {
  const card = document.querySelector('.welcome-card');
  if (card) card.remove();
}

// --- Connection Status Banner (Phase 4.1) ---

function showConnectionBanner(message, type) {
  const existing = document.getElementById('connection-banner');
  if (existing) existing.remove();

  const banner = document.createElement('div');
  banner.id = 'connection-banner';
  banner.className = 'connection-banner connection-banner-' + type;
  banner.textContent = message;
  document.body.appendChild(banner);
}

// --- Keyboard Shortcut Helpers (Phase 7.4) ---

function focusMemorySearch() {
  const memSearch = document.getElementById('memory-search');
  if (memSearch) {
    if (currentTab !== 'memory') switchTab('memory');
    memSearch.focus();
  }
}

function toggleShortcutsOverlay() {
  let overlay = document.getElementById('shortcuts-overlay');
  if (!overlay) {
    overlay = document.createElement('div');
    overlay.id = 'shortcuts-overlay';
    overlay.className = 'shortcuts-overlay';
    overlay.style.display = 'none';
    overlay.innerHTML =
      '<div class="shortcuts-content">'
      + '<h3>Keyboard Shortcuts</h3>'
      + '<div class="shortcut-row"><kbd>Ctrl/Cmd + 1-5</kbd> Switch tabs</div>'
      + '<div class="shortcut-row"><kbd>Ctrl/Cmd + N</kbd> New thread</div>'
      + '<div class="shortcut-row"><kbd>Ctrl/Cmd + K</kbd> Focus search/input</div>'
      + '<div class="shortcut-row"><kbd>Ctrl/Cmd + /</kbd> Toggle this overlay</div>'
      + '<div class="shortcut-row"><kbd>Escape</kbd> Close modals</div>'
      + '<button class="shortcuts-close">Close</button>'
      + '</div>';
    document.body.appendChild(overlay);
    overlay.querySelector('.shortcuts-close').addEventListener('click', () => {
      overlay.style.display = 'none';
    });
    overlay.addEventListener('click', (e) => {
      if (e.target === overlay) overlay.style.display = 'none';
    });
  }
  overlay.style.display = overlay.style.display === 'flex' ? 'none' : 'flex';
}

function closeModals() {
  // Close shortcuts overlay
  const shortcutsOverlay = document.getElementById('shortcuts-overlay');
  if (shortcutsOverlay) shortcutsOverlay.style.display = 'none';

  // Close restart confirmation modal
  const restartModal = document.getElementById('restart-confirm-modal');
  if (restartModal) restartModal.style.display = 'none';
}

// --- ARIA Accessibility (Phase 5.2) ---

function applyAriaAttributes() {
  const tabBar = document.querySelector('.tab-bar');
  if (tabBar) tabBar.setAttribute('role', 'tablist');

  document.querySelectorAll('.tab-bar button[data-tab]').forEach(btn => {
    btn.setAttribute('role', 'tab');
    btn.setAttribute('aria-selected', btn.classList.contains('active') ? 'true' : 'false');
  });

  document.querySelectorAll('.tab-panel').forEach(panel => {
    panel.setAttribute('role', 'tabpanel');
    panel.setAttribute('aria-hidden', panel.classList.contains('active') ? 'false' : 'true');
  });
}

// Apply ARIA attributes on initial load
applyAriaAttributes();

// --- Utilities ---

function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

function formatDate(isoString) {
  if (!isoString) return '-';
  const d = new Date(isoString);
  return d.toLocaleString();
}

// --- Event Listener Registration (CSP-safe, no inline handlers) ---

document.getElementById('auth-connect-btn').addEventListener('click', () => authenticate());

// User avatar dropdown toggle.
document.getElementById('user-avatar-btn').addEventListener('click', function(e) {
  e.stopPropagation();
  var dd = document.getElementById('user-dropdown');
  if (dd) dd.style.display = dd.style.display === 'none' ? '' : 'none';
});
// Close dropdown on click outside.
document.addEventListener('click', function(e) {
  var dd = document.getElementById('user-dropdown');
  var account = document.getElementById('user-account');
  if (dd && account && !account.contains(e.target)) {
    dd.style.display = 'none';
  }
});
// Logout handler.
document.getElementById('user-logout-btn').addEventListener('click', function() {
  fetch('/auth/logout', { method: 'POST', credentials: 'include' })
    .finally(function() {
      sessionStorage.removeItem('ironclaw_token');
      sessionStorage.removeItem('ironclaw_oidc');
      window.location.reload();
    });
});
document.getElementById('restart-overlay').addEventListener('click', () => cancelRestart());
document.getElementById('restart-close-btn').addEventListener('click', () => cancelRestart());
document.getElementById('restart-cancel-btn').addEventListener('click', () => cancelRestart());
document.getElementById('restart-confirm-btn').addEventListener('click', () => confirmRestart());
document.getElementById('restart-btn').addEventListener('click', () => triggerRestart());
document.getElementById('thread-new-btn').addEventListener('click', () => createNewThread());
document.getElementById('thread-toggle-btn').addEventListener('click', () => toggleThreadSidebar());
document.getElementById('assistant-thread').addEventListener('click', () => switchToAssistant());
document.getElementById('send-btn').addEventListener('click', () => sendMessage());
document.getElementById('memory-edit-btn').addEventListener('click', () => startMemoryEdit());
document.getElementById('memory-save-btn').addEventListener('click', () => saveMemoryEdit());
document.getElementById('memory-cancel-btn').addEventListener('click', () => cancelMemoryEdit());
document.getElementById('logs-server-level').addEventListener('change', (e) => setServerLogLevel(e.target.value));
document.getElementById('logs-pause-btn').addEventListener('click', () => toggleLogsPause());
document.getElementById('logs-clear-btn').addEventListener('click', () => clearLogs());
document.getElementById('wasm-install-btn').addEventListener('click', () => installWasmExtension());
document.getElementById('mcp-add-btn').addEventListener('click', () => addMcpServer());
document.getElementById('skill-search-btn').addEventListener('click', () => searchClawHub());
document.getElementById('skill-install-btn').addEventListener('click', () => installSkillFromForm());
document.getElementById('settings-export-btn').addEventListener('click', () => exportSettings());
document.getElementById('settings-import-btn').addEventListener('click', () => importSettings());
document.getElementById('settings-back-btn')?.addEventListener('click', () => settingsBack());

// --- Mobile: close thread sidebar on outside click ---
document.addEventListener('click', function(e) {
  const sidebar = document.getElementById('thread-sidebar');
  if (sidebar && sidebar.classList.contains('expanded-mobile') &&
      !sidebar.contains(e.target)) {
    sidebar.classList.remove('expanded-mobile');
    document.getElementById('thread-toggle-btn').innerHTML = '&raquo;';
  }
});

// --- Delegated Event Handlers (for dynamically generated HTML) ---

document.addEventListener('click', function(e) {
  const el = e.target.closest('[data-action]');
  if (!el) return;
  const action = el.dataset.action;

  switch (action) {
    case 'copy-code':
      copyCodeBlock(el);
      break;
    case 'breadcrumb-root':
      e.preventDefault();
      loadMemoryTree();
      break;
    case 'breadcrumb-file':
      e.preventDefault();
      readMemoryFile(el.dataset.path);
      break;
    case 'cancel-job':
      e.stopPropagation();
      cancelJob(el.dataset.id);
      break;
    case 'open-job':
      openJobDetail(el.dataset.id);
      break;
    case 'close-job-detail':
      closeJobDetail();
      break;
    case 'restart-job':
      restartJob(el.dataset.id);
      break;
    case 'open-routine':
      openRoutineDetail(el.dataset.id);
      break;
    case 'toggle-routine':
      e.stopPropagation();
      toggleRoutine(el.dataset.id);
      break;
    case 'trigger-routine':
      e.stopPropagation();
      triggerRoutine(el.dataset.id);
      break;
    case 'delete-routine':
      e.stopPropagation();
      deleteRoutine(el.dataset.id, el.dataset.name);
      break;
    case 'close-routine-detail':
      closeRoutineDetail();
      break;
    case 'open-mission':
      openMissionDetail(el.dataset.id);
      break;
    case 'close-mission-detail':
      closeMissionDetail();
      break;
    case 'fire-mission':
      e.stopPropagation();
      fireMission(el.dataset.id);
      break;
    case 'pause-mission':
      e.stopPropagation();
      pauseMission(el.dataset.id);
      break;
    case 'resume-mission':
      e.stopPropagation();
      resumeMission(el.dataset.id);
      break;
    case 'open-engine-thread':
      openEngineThread(el.dataset.id);
      break;
    case 'back-to-mission':
      if (currentMissionId) openMissionDetail(currentMissionId);
      else closeMissionDetail();
      break;
    case 'view-run-job':
      e.preventDefault();
      switchTab('jobs');
      openJobDetail(el.dataset.id);
      break;
    case 'view-routine-thread':
      e.preventDefault();
      switchTab('chat');
      switchThread(el.dataset.id);
      break;
    case 'copy-tee-report':
      copyTeeReport();
      break;
    case 'switch-language':
      if (typeof switchLanguage === 'function') switchLanguage(el.dataset.lang);
      break;
    case 'set-active-provider':
      setActiveProvider(el.dataset.id);
      break;
    case 'delete-custom-provider':
      deleteCustomProvider(el.dataset.id);
      break;
    case 'edit-custom-provider':
      editCustomProvider(el.dataset.id);
      break;
    case 'configure-builtin-provider':
      configureBuiltinProvider(el.dataset.id);
      break;
  }
});

document.getElementById('language-btn').addEventListener('click', function() {
  if (typeof toggleLanguageMenu === 'function') toggleLanguageMenu();
});

// --- Confirmation Modal ---

var _confirmModalCallback = null;

function showConfirmModal(title, message, onConfirm, confirmLabel, confirmClass) {
  var modal = document.getElementById('confirm-modal');
  document.getElementById('confirm-modal-title').textContent = title;
  document.getElementById('confirm-modal-message').textContent = message || '';
  document.getElementById('confirm-modal-message').style.display = message ? '' : 'none';
  var btn = document.getElementById('confirm-modal-btn');
  btn.textContent = confirmLabel || I18n.t('btn.confirm');
  btn.className = confirmClass || 'btn-danger';
  _confirmModalCallback = onConfirm;
  modal.style.display = 'flex';
  btn.focus();
}

function closeConfirmModal() {
  document.getElementById('confirm-modal').style.display = 'none';
  _confirmModalCallback = null;
}

document.getElementById('confirm-modal-btn').addEventListener('click', function() {
  if (_confirmModalCallback) _confirmModalCallback();
  closeConfirmModal();
});
document.getElementById('confirm-modal-cancel-btn').addEventListener('click', closeConfirmModal);
document.getElementById('confirm-modal').addEventListener('click', function(e) {
  if (e.target === this) closeConfirmModal();
});
document.addEventListener('keydown', function(e) {
  if (e.key === 'Escape' && document.getElementById('confirm-modal').style.display === 'flex') {
    closeConfirmModal();
  }
  if (e.key === 'Escape' && document.getElementById('provider-dialog').style.display === 'flex') {
    resetProviderForm();
  }
});

// --- Settings Import/Export ---

function exportSettings() {
  apiFetch('/api/settings/export').then(function(data) {
    var blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
    var url = URL.createObjectURL(blob);
    var a = document.createElement('a');
    a.href = url;
    a.download = 'ironclaw-settings.json';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
    showToast(I18n.t('settings.exportSuccess'), 'success');
  }).catch(function(err) {
    showToast(I18n.t('settings.exportFailed', { message: err.message }), 'error');
  });
}

function importSettings() {
  var input = document.createElement('input');
  input.type = 'file';
  input.accept = '.json,application/json';
  input.addEventListener('change', function() {
    if (!input.files || !input.files[0]) return;
    var reader = new FileReader();
    reader.onload = function() {
      try {
        var data = JSON.parse(reader.result);
        apiFetch('/api/settings/import', {
          method: 'POST',
          body: data,
        }).then(function() {
          showToast(I18n.t('settings.importSuccess'), 'success');
          loadSettingsSubtab(currentSettingsSubtab);
        }).catch(function(err) {
          showToast(I18n.t('settings.importFailed', { message: err.message }), 'error');
        });
      } catch (e) {
        showToast(I18n.t('settings.importFailed', { message: e.message }), 'error');
      }
    };
    reader.readAsText(input.files[0]);
  });
  input.click();
}

// --- Settings Search ---

document.getElementById('settings-search-input').addEventListener('input', function() {
  var query = this.value.toLowerCase();
  var activePanel = document.querySelector('.settings-subpanel.active');
  if (!activePanel) return;
  var visibleCount = 0;

  // --- Filter individual items ---

  // 1. Structured settings rows (Agent, Inference, Networking)
  var rows = activePanel.querySelectorAll('.settings-row');
  rows.forEach(function(row) {
    var text = row.textContent.toLowerCase();
    if (query === '' || text.indexOf(query) !== -1) {
      row.classList.remove('search-hidden');
      if (!row.classList.contains('hidden')) visibleCount++;
    } else {
      row.classList.add('search-hidden');
    }
  });

  // 2. Extension/channel/MCP/skill cards (Channels, Extensions, MCP, Skills)
  var cards = activePanel.querySelectorAll('.ext-card');
  cards.forEach(function(card) {
    var text = card.textContent.toLowerCase();
    if (query === '' || text.indexOf(query) !== -1) {
      card.classList.remove('search-hidden');
      visibleCount++;
    } else {
      card.classList.add('search-hidden');
    }
  });

  // 3. Tool permission rows (Tools)
  var toolRows = activePanel.querySelectorAll('.tool-permission-row');
  toolRows.forEach(function(row) {
    var text = row.textContent.toLowerCase();
    if (query === '' || text.indexOf(query) !== -1) {
      row.classList.remove('search-hidden');
      visibleCount++;
    } else {
      row.classList.add('search-hidden');
    }
  });

  // 4. User table rows (User Management)
  var userRows = activePanel.querySelectorAll('#users-tbody tr');
  userRows.forEach(function(row) {
    var text = row.textContent.toLowerCase();
    if (query === '' || text.indexOf(query) !== -1) {
      row.classList.remove('search-hidden');
      visibleCount++;
    } else {
      row.classList.add('search-hidden');
    }
  });

  // --- Update container visibility after all items are filtered ---

  var groups = activePanel.querySelectorAll('.settings-group');
  groups.forEach(function(group) {
    var visibleRows = group.querySelectorAll('.settings-row:not(.search-hidden):not(.hidden)');
    if (visibleRows.length === 0 && query !== '') {
      group.style.display = 'none';
    } else {
      group.style.display = '';
    }
  });

  var sections = activePanel.querySelectorAll('.extensions-section');
  sections.forEach(function(section) {
    var visibleItems = section.querySelectorAll('.ext-card:not(.search-hidden), .tool-permission-row:not(.search-hidden)');
    if (visibleItems.length === 0 && query !== '') {
      section.style.display = 'none';
    } else {
      section.style.display = '';
    }
  });

  // Show/hide empty state
  var existingEmpty = activePanel.querySelector('.settings-search-empty');
  if (existingEmpty) existingEmpty.remove();
  if (query !== '' && visibleCount === 0) {
    var empty = document.createElement('div');
    empty.className = 'settings-search-empty';
    empty.textContent = I18n.t('settings.noMatchingSettings', { query: this.value });
    activePanel.appendChild(empty);
  }
});

// --- Config Tab ---

// Like apiFetch but for endpoints that return 204 No Content
// Like apiFetch but discards the response body (for 204 No Content endpoints).
function apiFetchVoid(path, options) {
  return apiFetch(path, options).then(function() {});
}

/** Sentinel value meaning "key is unchanged, don't touch it". Must match backend. */
const API_KEY_UNCHANGED = '\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022';

const ADAPTER_LABELS = {
  open_ai_completions: 'OpenAI Compatible',
  anthropic: 'Anthropic',
  ollama: 'Ollama',
  bedrock: 'AWS Bedrock',
  nearai: 'NEAR AI',
};

let _builtinProviders = [];
let _customProviders = [];
let _activeLlmBackend = '';
let _selectedModel = '';
let _builtinOverrides = {};
let _editingProviderId = null;
let _configuringBuiltinId = null;
let _configLoaded = false;

function loadConfig() {
  const list = document.getElementById('providers-list');
  list.innerHTML = '<div class="empty-state">' + I18n.t('common.loading') + '</div>';

  Promise.all([
    apiFetch('/api/settings/export'),
    apiFetch('/api/llm/providers').catch(function() { return []; }),
  ]).then(function(results) {
    const s = (results[0] && results[0].settings) ? results[0].settings : {};
    _builtinProviders = Array.isArray(results[1]) ? results[1] : [];
    _activeLlmBackend = s['llm_backend'] ? String(s['llm_backend']) : 'nearai';
    _selectedModel = s['selected_model'] ? String(s['selected_model']) : '';
    try {
      const val = s['llm_custom_providers'];
      _customProviders = Array.isArray(val) ? val : (val ? JSON.parse(val) : []);
    } catch (e) {
      _customProviders = [];
    }
    try {
      const val = s['llm_builtin_overrides'];
      _builtinOverrides = (val && typeof val === 'object' && !Array.isArray(val)) ? val : {};
    } catch (e) {
      _builtinOverrides = {};
    }
    _configLoaded = true;
    renderProviders();
  }).catch(function() {
    _activeLlmBackend = 'nearai';
    _selectedModel = '';
    _builtinProviders = [];
    _customProviders = [];
    _builtinOverrides = {};
    _configLoaded = true;
    renderProviders();
  });
}

function scrollToProviders() {
  const section = document.getElementById('providers-section');
  if (section) section.scrollIntoView({ behavior: 'smooth', block: 'start' });
}

function renderProviders() {
  const list = document.getElementById('providers-list');
  const allProviders = [..._builtinProviders, ..._customProviders].sort((a, b) => {
    if (a.id === _activeLlmBackend) return -1;
    if (b.id === _activeLlmBackend) return 1;
    return 0;
  });

  if (allProviders.length === 0) {
    list.innerHTML = '<div class="empty-state">No providers</div>';
    return;
  }

  list.innerHTML = allProviders.map((p) => {
    const isActive = p.id === _activeLlmBackend;
    const adapterLabel = ADAPTER_LABELS[p.adapter] || p.adapter;
    const activeBadge = isActive
      ? '<span class="provider-badge provider-badge-active">' + I18n.t('status.active') + '</span>'
      : '';
    const builtinBadge = p.builtin
      ? '<span class="provider-badge provider-badge-builtin">' + I18n.t('config.builtin') + '</span>'
      : '';
    const deleteBtn = !p.builtin && !isActive
      ? '<button class="provider-action-btn provider-delete-btn" data-action="delete-custom-provider" data-id="' + escapeHtml(p.id) + '">' + I18n.t('common.delete') + '</button>'
      : '';
    const editBtn = !p.builtin
      ? '<button class="provider-action-btn" data-action="edit-custom-provider" data-id="' + escapeHtml(p.id) + '">' + I18n.t('common.edit') + '</button>'
      : '';
    // Show Configure for built-in providers that support it (not bedrock — uses AWS credential chain)
    const configureBtn = p.builtin && p.id !== 'bedrock'
      ? '<button class="provider-action-btn" data-action="configure-builtin-provider" data-id="' + escapeHtml(p.id) + '">' + I18n.t('config.configureProvider') + '</button>'
      : '';
    const useBtn = !isActive
      ? '<button class="provider-action-btn" data-action="set-active-provider" data-id="' + escapeHtml(p.id) + '">' + I18n.t('config.useProvider') + '</button>'
      : '';
    const overrideBaseUrl = p.builtin && _builtinOverrides[p.id] ? (_builtinOverrides[p.id].base_url || '') : '';
    const effectiveBaseUrl = overrideBaseUrl || p.env_base_url || p.base_url;
    const baseUrlText = effectiveBaseUrl
      ? '<span class="provider-url">' + escapeHtml(effectiveBaseUrl) + '</span>'
      : '';
    // Show configured model: for active provider use _selectedModel, for others check _builtinOverrides then env defaults
    const overrideModel = p.builtin && _builtinOverrides[p.id] ? (_builtinOverrides[p.id].model || '') : '';
    const displayModel = isActive
      ? (_selectedModel || p.env_model || '')
      : (overrideModel || p.env_model || '');
    const modelText = displayModel
      ? '<span class="provider-current-model">' + escapeHtml(I18n.t('config.currentModel', { model: displayModel })) + '</span>'
      : '';

    return '<div class="provider-card' + (isActive ? ' provider-card-active' : '') + '">'
      + '<div class="provider-card-header">'
      +   '<span class="provider-name">' + escapeHtml(p.name || p.id) + '</span>'
      +   '<span class="provider-id-label">' + escapeHtml(p.id) + '</span>'
      +   activeBadge + builtinBadge
      + '</div>'
      + '<div class="provider-card-meta">'
      +   '<span class="provider-adapter">' + escapeHtml(adapterLabel) + '</span>'
      +   baseUrlText
      +   modelText
      + '</div>'
      + '<div class="provider-card-actions">'
      +   useBtn + configureBtn + editBtn + deleteBtn
      + '</div>'
      + '</div>';
  }).join('');
}

function setActiveProvider(id) {
  const provider = [..._builtinProviders, ..._customProviders].find((p) => p.id === id);
  // Restore the last-configured model for this provider, falling back to the provider's default
  const restoredModel =
    (_builtinOverrides[id] && _builtinOverrides[id].model) ||
    (provider && provider.default_model) ||
    null;
  const defaultModel = restoredModel;
  const modelUpdate = () => defaultModel
    ? apiFetchVoid('/api/settings/selected_model', { method: 'PUT', body: { value: defaultModel } })
    : apiFetchVoid('/api/settings/selected_model', { method: 'DELETE' });
  apiFetchVoid('/api/settings/llm_backend', { method: 'PUT', body: { value: id } })
    .then(() => modelUpdate())
    .then(() => {
      _activeLlmBackend = id;
      _selectedModel = defaultModel || '';
      renderProviders();
      loadInferenceSettings();
      scrollToProviders();
      document.getElementById('config-restart-notice').style.display = 'flex';
      var llmNotice = document.getElementById('llm-restart-notice');
      if (llmNotice) llmNotice.style.display = 'flex';
      showToast(I18n.t('config.providerActivated', { name: id }));
    })
    .catch((e) => showToast(I18n.t('error.unknown') + ': ' + e.message, 'error'));
}

function deleteCustomProvider(id) {
  if (id === _activeLlmBackend) {
    showToast(I18n.t('config.cannotDeleteActiveProvider'), 'error');
    return;
  }
  if (!confirm(I18n.t('config.confirmDeleteProvider', { id }))) return;
  const originalProviders = _customProviders;
  _customProviders = _customProviders.filter((p) => p.id !== id);
  saveCustomProviders().then(() => {
    renderProviders();
    showToast(I18n.t('config.providerDeleted'));
  }).catch((e) => {
    _customProviders = originalProviders;
    showToast(I18n.t('error.unknown') + ': ' + e.message, 'error');
  });
}

function saveCustomProviders() {
  return apiFetchVoid('/api/settings/llm_custom_providers', { method: 'PUT', body: { value: _customProviders } });
}

function editCustomProvider(id) {
  const p = _customProviders.find((p) => p.id === id);
  if (!p) return;
  _editingProviderId = id;
  const titleEl = document.getElementById('provider-form-title');
  titleEl.textContent = I18n.t('config.editProvider');
  titleEl.removeAttribute('data-i18n');
  document.getElementById('provider-name').value = p.name || '';
  const idField = document.getElementById('provider-id');
  idField.value = p.id;
  idField.readOnly = true;
  idField.style.opacity = '0.6';
  document.getElementById('provider-adapter').value = p.adapter || 'open_ai_completions';
  document.getElementById('provider-base-url').value = p.base_url || '';
  const editApiKeyInput = document.getElementById('provider-api-key');
  if (p.api_key === API_KEY_UNCHANGED) {
    editApiKeyInput.value = '';
    editApiKeyInput.placeholder = I18n.t('config.apiKeyConfigured');
  } else {
    editApiKeyInput.value = '';
    editApiKeyInput.placeholder = I18n.t('config.apiKeyEnter');
  }
  document.getElementById('provider-model').value = p.default_model || '';
  openProviderDialog(true);
  document.getElementById('provider-name').focus();
}

function configureBuiltinProvider(id) {
  const p = _builtinProviders.find((p) => p.id === id);
  if (!p) return;
  _configuringBuiltinId = id;
  const titleEl = document.getElementById('provider-form-title');
  titleEl.textContent = I18n.t('config.configureProvider') + ': ' + (p.name || id);
  titleEl.removeAttribute('data-i18n');
  // Hide name/id/adapter rows; show base-url as editable
  document.getElementById('provider-name-row').style.display = 'none';
  document.getElementById('provider-id-row').style.display = 'none';
  document.getElementById('provider-adapter-row').style.display = 'none';
  const baseUrlInput = document.getElementById('provider-base-url');
  const override = _builtinOverrides[id] || {};
  // Priority: db override > env > hardcoded default
  const effectiveBaseUrl = override.base_url || p.env_base_url || p.base_url;
  document.getElementById('provider-base-url-row').style.display = '';
  baseUrlInput.value = effectiveBaseUrl || '';
  baseUrlInput.readOnly = false;
  baseUrlInput.style.opacity = '';
  baseUrlInput.placeholder = p.base_url || '';
  document.getElementById('provider-api-key-row').style.display = p.api_key_required !== false ? '' : 'none';
  document.getElementById('fetch-models-btn').style.display = p.can_list_models ? '' : 'none';
  const apiKeyInput = document.getElementById('provider-api-key');
  const hasDbKey = override.api_key === API_KEY_UNCHANGED;
  const hasEnvKey = p.has_api_key === true;
  apiKeyInput.value = '';
  if (hasDbKey) {
    apiKeyInput.placeholder = I18n.t('config.apiKeyConfigured');
  } else if (hasEnvKey) {
    apiKeyInput.placeholder = I18n.t('config.apiKeyFromEnv');
  } else {
    apiKeyInput.placeholder = I18n.t('config.apiKeyEnter');
  }
  document.getElementById('provider-model').value = override.model || p.env_model || p.default_model || '';
  openProviderDialog(true);
  document.getElementById('provider-model').focus();
}

// Add provider form

document.getElementById('add-provider-btn').addEventListener('click', () => {
  openProviderDialog(false);
});

document.getElementById('cancel-provider-btn').addEventListener('click', () => {
  resetProviderForm();
});

document.getElementById('cancel-provider-footer-btn').addEventListener('click', () => {
  resetProviderForm();
});

document.getElementById('provider-dialog-overlay').addEventListener('click', () => {
  resetProviderForm();
});

function openProviderDialog(isEdit) {
  if (!isEdit) {
    // Add mode: ensure all rows visible
    ['provider-name-row', 'provider-id-row', 'provider-adapter-row',
     'provider-base-url-row', 'provider-api-key-row'].forEach((id) => {
      document.getElementById(id).style.display = '';
    });
    document.getElementById('fetch-models-btn').style.display = '';
  }
  document.getElementById('provider-dialog').style.display = 'flex';
  if (!isEdit) {
    document.getElementById('provider-name').focus();
  }
}

document.getElementById('test-provider-btn').addEventListener('click', () => {
  let adapter = document.getElementById('provider-adapter').value;
  let baseUrl = document.getElementById('provider-base-url').value.trim();
  const apiKey = document.getElementById('provider-api-key').value.trim();
  const model = document.getElementById('provider-model').value.trim();

  // For built-in providers, use the adapter from the registry.
  // base_url comes from the form which already reflects: env > hardcoded default.
  if (_configuringBuiltinId) {
    const p = _builtinProviders.find((x) => x.id === _configuringBuiltinId);
    if (p) {
      adapter = p.adapter;
      if (!baseUrl) baseUrl = p.base_url;
    }
  }

  const btn = document.getElementById('test-provider-btn');
  const result = document.getElementById('test-connection-result');

  btn.disabled = true;
  btn.textContent = I18n.t('config.testing');
  result.style.display = 'none';
  result.className = 'test-connection-result';

  // Resolve provider_id so the backend can look up vaulted API keys.
  const providerId = _configuringBuiltinId || document.getElementById('provider-id').value.trim();

  if (!model) {
    result.textContent = I18n.t('config.modelRequired') || 'Model is required for connection test';
    result.className = 'test-connection-result test-fail';
    result.style.display = '';
    btn.disabled = false;
    btn.textContent = I18n.t('config.testConnection');
    return;
  }

  apiFetch('/api/llm/test_connection', {
    method: 'POST',
    body: {
      adapter, base_url: baseUrl,
      api_key: apiKey || undefined,
      model,
      provider_id: providerId || undefined,
      provider_type: _configuringBuiltinId ? 'builtin' : 'custom',
    },
  })
    .then((data) => {
      result.textContent = data.message;
      result.className = 'test-connection-result ' + (data.ok ? 'test-ok' : 'test-fail');
      result.style.display = '';
    })
    .catch((e) => {
      result.textContent = e.message;
      result.className = 'test-connection-result test-fail';
      result.style.display = '';
    })
    .finally(() => {
      btn.disabled = false;
      btn.textContent = I18n.t('config.testConnection');
    });
});

document.getElementById('save-provider-btn').addEventListener('click', () => {
  // Built-in configure mode: save api_key + model to llm_builtin_overrides
  if (_configuringBuiltinId) {
    const apiKey = document.getElementById('provider-api-key').value.trim();
    const model = document.getElementById('provider-model').value.trim();
    const baseUrl = document.getElementById('provider-base-url').value.trim();
    const id = _configuringBuiltinId;
    const prevOverride = _builtinOverrides[id] || {};
    const hadKey = prevOverride.api_key === API_KEY_UNCHANGED;
    const override = {};
    if (apiKey) {
      override.api_key = apiKey;  // New key entered — backend will encrypt it
    } else if (hadKey) {
      override.api_key = API_KEY_UNCHANGED;  // Sentinel: keep existing encrypted key
    }
    // If neither — key is cleared (no key configured)
    if (model) override.model = model;
    if (baseUrl) override.base_url = baseUrl;
    const prev = _builtinOverrides[id];
    _builtinOverrides[id] = override;
    const isActive = id === _activeLlmBackend;
    const modelUpdate = () => {
      if (!isActive) return Promise.resolve();
      if (model) {
        return apiFetchVoid('/api/settings/selected_model', { method: 'PUT', body: { value: model } });
      }
      return apiFetchVoid('/api/settings/selected_model', { method: 'DELETE' });
    };
    apiFetchVoid('/api/settings/llm_builtin_overrides', { method: 'PUT', body: { value: _builtinOverrides } })
      .then(() => modelUpdate())
      .then(() => {
        if (isActive) _selectedModel = model;
        renderProviders();
        if (isActive) loadInferenceSettings();
        resetProviderForm();
        scrollToProviders();
        if (isActive) {
          document.getElementById('config-restart-notice').style.display = 'flex';
          var llmNotice = document.getElementById('llm-restart-notice');
          if (llmNotice) llmNotice.style.display = 'flex';
        }
        showToast(I18n.t('config.providerConfigured', { name: id }));
      })
      .catch((e) => {
        if (prev !== undefined) { _builtinOverrides[id] = prev; } else { delete _builtinOverrides[id]; }
        showToast(I18n.t('error.unknown') + ': ' + e.message, 'error');
      });
    return;
  }

  const name = document.getElementById('provider-name').value.trim();
  const id = document.getElementById('provider-id').value.trim();
  const adapter = document.getElementById('provider-adapter').value;
  const baseUrl = document.getElementById('provider-base-url').value.trim();
  const apiKey = document.getElementById('provider-api-key').value.trim();
  const model = document.getElementById('provider-model').value.trim();

  if (!id || !name) {
    showToast(I18n.t('config.providerFieldsRequired'), 'error');
    return;
  }

  if (_editingProviderId) {
    // Update existing provider
    const idx = _customProviders.findIndex((p) => p.id === _editingProviderId);
    if (idx === -1) return;
    const original = _customProviders[idx];
    const hadCustomKey = original.api_key === API_KEY_UNCHANGED;
    let effectiveApiKey;
    if (apiKey) {
      effectiveApiKey = apiKey;  // New key — backend will encrypt it
    } else if (hadCustomKey) {
      effectiveApiKey = API_KEY_UNCHANGED;  // Sentinel: keep existing encrypted key
    } else {
      effectiveApiKey = undefined;  // No key
    }
    _customProviders[idx] = { ...original, name, adapter, base_url: baseUrl, default_model: model || undefined, api_key: effectiveApiKey };
    const isActive = _editingProviderId === _activeLlmBackend;
    const modelUpdate = () => {
      if (!isActive) return Promise.resolve();
      if (model) {
        return apiFetchVoid('/api/settings/selected_model', { method: 'PUT', body: { value: model } });
      }
      return apiFetchVoid('/api/settings/selected_model', { method: 'DELETE' });
    };
    saveCustomProviders().then(() => modelUpdate()).then(() => {
      if (isActive) _selectedModel = model;
      renderProviders();
      if (isActive) loadInferenceSettings();
      resetProviderForm();
      scrollToProviders();
      if (isActive) {
        document.getElementById('config-restart-notice').style.display = 'flex';
        var llmNotice = document.getElementById('llm-restart-notice');
        if (llmNotice) llmNotice.style.display = 'flex';
      }
      showToast(I18n.t('config.providerUpdated', { name }));
    }).catch((e) => {
      _customProviders[idx] = original;
      showToast(I18n.t('error.unknown') + ': ' + e.message, 'error');
    });
    return;
  }

  if (!/^[a-z0-9_-]+$/.test(id)) {
    showToast(I18n.t('config.providerIdInvalid'), 'error');
    return;
  }
  const allIds = [..._builtinProviders.map((p) => p.id), ..._customProviders.map((p) => p.id)];
  if (allIds.includes(id)) {
    showToast(I18n.t('config.providerIdTaken', { id }), 'error');
    return;
  }

  const newProvider = { id, name, adapter, base_url: baseUrl, default_model: model, api_key: apiKey || undefined, builtin: false };
  _customProviders.push(newProvider);

  saveCustomProviders().then(() => {
    renderProviders();
    resetProviderForm();
    scrollToProviders();
    showToast(I18n.t('config.providerAdded', { name }));
  }).catch((e) => {
    _customProviders.pop();
    showToast(I18n.t('error.unknown') + ': ' + e.message, 'error');
  });
});

function resetProviderForm() {
  _editingProviderId = null;
  _configuringBuiltinId = null;
  document.getElementById('provider-dialog').style.display = 'none';
  // Restore all hidden rows and buttons
  ['provider-name-row', 'provider-id-row', 'provider-adapter-row',
   'provider-base-url-row', 'provider-api-key-row'].forEach((id) => {
    document.getElementById(id).style.display = '';
  });
  document.getElementById('fetch-models-btn').style.display = '';
  const titleEl = document.getElementById('provider-form-title');
  titleEl.setAttribute('data-i18n', 'config.newProvider');
  titleEl.textContent = I18n.t('config.newProvider');
  const idField = document.getElementById('provider-id');
  idField.readOnly = false;
  idField.style.opacity = '';
  delete idField.dataset.edited;
  const baseUrlField = document.getElementById('provider-base-url');
  baseUrlField.readOnly = false;
  baseUrlField.style.opacity = '';
  ['provider-name', 'provider-id', 'provider-base-url', 'provider-api-key', 'provider-model'].forEach((id) => {
    document.getElementById(id).value = '';
  });
  document.getElementById('provider-adapter').selectedIndex = 0;
  const sel = document.getElementById('provider-model-select');
  sel.innerHTML = '';
  sel.style.display = 'none';
  document.getElementById('test-connection-result').style.display = 'none';
}

document.getElementById('provider-model-select').addEventListener('change', (e) => {
  document.getElementById('provider-model').value = e.target.value;
});

document.getElementById('fetch-models-btn').addEventListener('click', () => {
  let adapter = document.getElementById('provider-adapter').value;
  let baseUrl = document.getElementById('provider-base-url').value.trim();
  const apiKey = document.getElementById('provider-api-key').value.trim();

  // For built-in providers, use the adapter from the registry.
  // base_url comes from the form which already reflects: env > hardcoded default.
  if (_configuringBuiltinId) {
    const p = _builtinProviders.find((x) => x.id === _configuringBuiltinId);
    if (p) {
      adapter = p.adapter;
      if (!baseUrl) baseUrl = p.base_url;
    }
  }

  if (!baseUrl) {
    showToast(I18n.t('config.providerBaseUrlRequired'), 'error');
    return;
  }

  const btn = document.getElementById('fetch-models-btn');
  btn.disabled = true;
  btn.textContent = I18n.t('config.fetchingModels');

  // Resolve provider_id so the backend can look up vaulted API keys.
  const providerId = _configuringBuiltinId || document.getElementById('provider-id').value.trim();

  apiFetch('/api/llm/list_models', {
    method: 'POST',
    body: {
      adapter, base_url: baseUrl,
      api_key: apiKey || undefined,
      provider_id: providerId || undefined,
      provider_type: _configuringBuiltinId ? 'builtin' : 'custom',
    },
  })
    .then((data) => {
      const select = document.getElementById('provider-model-select');
      if (data.ok && data.models && data.models.length > 0) {
        const currentModel = document.getElementById('provider-model').value;
        select.innerHTML = data.models
          .map((m) => `<option value="${escapeHtml(m)}"${m === currentModel ? ' selected' : ''}>${escapeHtml(m)}</option>`)
          .join('');
        select.style.display = '';
        btn.style.display = 'none';
        showToast(I18n.t('config.modelsFetched', { count: data.models.length }));
      } else {
        showToast(data.message || I18n.t('config.modelsFetchFailed'), 'error');
      }
    })
    .catch((e) => showToast(e.message, 'error'))
    .finally(() => {
      btn.disabled = false;
      btn.textContent = I18n.t('config.fetchModels');
    });
});

// Auto-fill provider ID from name
document.getElementById('provider-name').addEventListener('input', (e) => {
  const idField = document.getElementById('provider-id');
  if (!idField.dataset.edited) {
    idField.value = e.target.value.toLowerCase().replace(/[^a-z0-9_]+/g, '-').replace(/^-|-$/g, '');
  }
});

document.getElementById('provider-id').addEventListener('input', (e) => {
  e.target.dataset.edited = e.target.value ? '1' : '';
});

// ==================== Widget Extension System ====================
//
// Provides a registration API for frontend widgets. Widgets are self-contained
// components that plug into named slots in the UI (tabs, sidebar, status bar, etc.).
//
// Widget authors call IronClaw.registerWidget({ id, name, slot, init, ... })
// from their module script. The init() function receives a container DOM element
// and the IronClaw.api object for authenticated fetch, event subscription, etc.

// Define `window.IronClaw` as a non-writable, non-configurable property
// rather than `window.IronClaw = window.IronClaw || {}`. The `|| {}` form
// would honor any pre-existing value on `window.IronClaw`, which in
// principle could be set by an inline script that ran before app.js — a
// hostile pre-init could install a fake `registerWidget` trap and
// intercept every widget registration. In practice the gateway HTML
// loads app.js before any deferred `type="module"` widget script and
// has no inline scripts that touch `window.IronClaw`, so this is
// defense-in-depth against future template changes (or a stray browser
// extension), not a fix for an exploitable bug. Using
// `Object.defineProperty` with `writable: false` / `configurable: false`
// also locks the binding so a hostile widget can't replace the entire
// `IronClaw` object after the fact — its only path is to mutate properties
// on the fixed object, which is the same authority every other widget has.
Object.defineProperty(window, 'IronClaw', {
  value: {},
  writable: false,
  configurable: false,
  enumerable: true,
});
IronClaw.widgets = new Map();
IronClaw._widgetInitQueue = [];
IronClaw._chatRenderers = [];

/**
 * Register a widget component.
 * @param {Object} def - Widget definition
 * @param {string} def.id - Unique widget identifier
 * @param {string} def.name - Display name
 * @param {string} def.slot - Target slot ('tab', 'chat_header', etc.)
 * @param {string} [def.icon] - Icon identifier
 * @param {Function} def.init - Called with (container, api) when widget activates
 * @param {Function} [def.activate] - Called when widget becomes visible
 * @param {Function} [def.deactivate] - Called when widget is hidden
 * @param {Function} [def.destroy] - Called when widget is removed
 */
IronClaw.registerWidget = function(def) {
  if (!def.id || !def.init) {
    console.error('[IronClaw] Widget registration requires id and init:', def);
    return;
  }
  IronClaw.widgets.set(def.id, def);

  if (def.slot === 'tab') {
    _addWidgetTab(def);
  }
};

/**
 * Register a chat renderer for custom inline rendering of structured data.
 *
 * Chat renderers run against each assistant message. The first renderer
 * whose `match()` returns true gets to transform the content.
 *
 * @param {Object} def - Renderer definition
 * @param {string} def.id - Unique identifier
 * @param {Function} def.match - (textContent, element) => boolean
 * @param {Function} def.render - (element, textContent) => void (mutate element in place)
 * @param {number} [def.priority=0] - Higher priority runs first
 */
IronClaw.registerChatRenderer = function(def) {
  if (!def.id || !def.match || !def.render) {
    console.error('[IronClaw] Chat renderer requires id, match, and render:', def);
    return;
  }
  IronClaw._chatRenderers.push(def);
  // Sort by priority (higher first)
  IronClaw._chatRenderers.sort(function(a, b) {
    return (b.priority || 0) - (a.priority || 0);
  });
};

/**
 * API object exposed to widgets for safe interaction with the app.
 */
IronClaw.api = {
  /**
   * Authenticated fetch wrapper — injects the session token.
   *
   * **Same-origin enforcement.** The session token is injected into the
   * `Authorization` header on every call, so a cross-origin URL would
   * leak the token to an attacker-controlled host. Resolve the requested
   * path against the page's own origin and reject anything that lands on
   * a different origin. Site-relative paths (`/api/foo`) and same-origin
   * absolute URLs are still allowed; everything else (`https://evil.example/...`,
   * protocol-relative `//evil.example/...`, `javascript:`, `data:`) is
   * rejected with a clear `TypeError` so the widget author sees the
   * misuse at the offending call site instead of having the request fly
   * silently to a hostile host.
   */
  fetch: function(path, opts) {
    var resolved;
    try {
      resolved = new URL(path, window.location.origin);
    } catch (e) {
      return Promise.reject(
        new TypeError('IronClaw.api.fetch: invalid URL ' + JSON.stringify(path))
      );
    }
    if (resolved.origin !== window.location.origin) {
      return Promise.reject(
        new TypeError(
          'IronClaw.api.fetch: cross-origin requests are not allowed (got ' +
          resolved.origin + ', expected ' + window.location.origin +
          '). Use a relative path or a same-origin absolute URL.'
        )
      );
    }
    opts = opts || {};
    opts.headers = Object.assign({}, opts.headers || {}, {
      'Authorization': 'Bearer ' + token
    });
    return fetch(resolved.toString(), opts);
  },

  /** Subscribe to an SSE/WebSocket event type. Returns an unsubscribe function. */
  subscribe: function(eventType, handler) {
    if (!window._widgetEventHandlers) window._widgetEventHandlers = {};
    if (!window._widgetEventHandlers[eventType]) window._widgetEventHandlers[eventType] = [];
    window._widgetEventHandlers[eventType].push(handler);
    return function() {
      var handlers = window._widgetEventHandlers[eventType];
      if (handlers) {
        var idx = handlers.indexOf(handler);
        if (idx !== -1) handlers.splice(idx, 1);
      }
    };
  },

  /**
   * Dispatch an SSE event to registered widget handlers.
   * Called internally by SSE event listeners — not for widget use.
   * @private
   */
  _dispatch: function(eventType, data) {
    var handlers = window._widgetEventHandlers && window._widgetEventHandlers[eventType];
    if (!handlers || handlers.length === 0) return;
    for (var i = 0; i < handlers.length; i++) {
      try { handlers[i](data); } catch (e) {
        console.error('[IronClaw] Widget event handler error (' + eventType + '):', e);
      }
    }
  },

  /** Current theme information. */
  theme: {
    get current() { return document.documentElement.dataset.theme || 'dark'; }
  },

  /** Internationalization helper. */
  i18n: {
    t: function(key) { return (window.I18n && window.I18n.t) ? window.I18n.t(key) : key; }
  },

  /** Navigate to a tab by ID. */
  navigate: function(tabId) {
    if (typeof switchTab === 'function') switchTab(tabId);
  }
};

/**
 * Add a widget as a new tab in the tab bar.
 * @private
 */
function _addWidgetTab(def) {
  var tabBar = document.querySelector('.tab-bar');
  // Tab panels live as siblings of `.tab-bar` inside `#app`. Earlier
  // versions of this code looked for a dedicated `.tab-content` /
  // `#tab-content` element that the gateway HTML never actually shipped,
  // so widget tabs were silently queued forever. Use the parent of the
  // first existing `.tab-panel` (falling back to `#app`) so widgets mount
  // into the same container as the built-in tabs.
  var existingPanel = document.querySelector('.tab-panel');
  var tabContent = (existingPanel && existingPanel.parentNode)
    || document.querySelector('.tab-content')
    || document.getElementById('tab-content')
    || document.getElementById('app');
  if (!tabBar || !tabContent) {
    // DOM not ready yet — queue for later
    IronClaw._widgetInitQueue.push(def);
    return;
  }

  // Create tab button
  var btn = document.createElement('button');
  btn.className = 'tab-btn';
  btn.dataset.tab = def.id;
  btn.textContent = def.name;
  if (def.icon) {
    btn.dataset.icon = def.icon;
  }
  btn.addEventListener('click', function() {
    if (typeof switchTab === 'function') switchTab(def.id);
  });
  // Insert before the settings tab (last built-in tab) or at the end
  var settingsBtn = tabBar.querySelector('[data-tab="settings"]');
  if (settingsBtn) {
    tabBar.insertBefore(btn, settingsBtn);
  } else {
    tabBar.appendChild(btn);
  }

  // Create container panel (id must match switchTab's `p.id === 'tab-' + tab`)
  var panel = document.createElement('div');
  panel.id = 'tab-' + def.id;
  panel.className = 'tab-panel';
  panel.dataset.tab = def.id;
  panel.dataset.widget = def.id;
  tabContent.appendChild(panel);

  // Initialize the widget
  try {
    def.init(panel, IronClaw.api);
  } catch (e) {
    console.error('[IronClaw] Widget "' + def.id + '" init failed:', e);
    // Escape both the widget id and the thrown message before injecting
    // them into the error banner. CSP blocks the script vector here, but
    // every other branch in this file routes user-controlled strings
    // through escapeHtml(), and an unescaped innerHTML write is a
    // discipline regression that future readers shouldn't have to
    // re-litigate. textContent would also work, but innerHTML lets the
    // styled <div> survive without an extra wrapper element.
    panel.innerHTML = '<div style="padding:2rem;color:var(--color-error,red);">Widget "' +
      escapeHtml(def.id) + '" failed to load: ' +
      escapeHtml(String(e && e.message ? e.message : e)) + '</div>';
  }
}

// Apply layout config if injected by the server
if (window.__IRONCLAW_LAYOUT__) {
  (function() {
    var layout = window.__IRONCLAW_LAYOUT__;

    // Apply branding title
    if (layout.branding && layout.branding.title) {
      var titleEl = document.querySelector('.app-title');
      if (titleEl) titleEl.textContent = layout.branding.title;
    }

    // Apply tab visibility — hide specified tabs.
    //
    // The selector must match BOTH built-in tab buttons (rendered in
    // `index.html` as plain `<button data-tab="…">`, no class) and
    // widget-injected tab buttons (created by `_addWidgetTab` with
    // `class="tab-btn"`). The earlier `.tab-btn[data-tab=…]` form only
    // matched widget tabs, so `tabs.hidden: ["routines"]` (a built-in)
    // silently no-opped. Scope the selector to `.tab-bar` so a stray
    // `<button data-tab>` elsewhere on the page can't be hidden by
    // accident, then accept any descendant button.
    if (layout.tabs && layout.tabs.hidden) {
      layout.tabs.hidden.forEach(function(tabId) {
        // CSS.escape() the workspace-supplied tab id before
        // interpolation. The endpoint that writes layout.json is now
        // admin-only (PR #1725 P-H9 fix), so the realistic exploit is
        // admin-on-self — but a one-line `CSS.escape` removes the
        // attribute-selector breakout vector entirely. An admin who
        // pastes a workspace doc fragment into `layout.json` shouldn't
        // be able to footgun themselves into a side-channel CSS probe.
        // CSS.escape is a stable browser API since 2015 and ships in
        // every gateway-supported browser; no fallback needed.
        var safe = (typeof CSS !== 'undefined' && CSS.escape)
          ? CSS.escape(tabId)
          : tabId;
        var btn = document.querySelector(
          '.tab-bar button[data-tab="' + safe + '"]'
        );
        if (btn) btn.style.display = 'none';
      });
    }

    // Apply tab ordering — reorder tab buttons in the tab bar
    if (layout.tabs && layout.tabs.order && layout.tabs.order.length > 0) {
      var tabBar = document.querySelector('.tab-bar');
      if (tabBar) {
        var order = layout.tabs.order;
        // Sort existing buttons by the specified order
        var buttons = Array.from(tabBar.querySelectorAll('button[data-tab]'));
        var orderIndex = {};
        order.forEach(function(id, i) { orderIndex[id] = i; });
        buttons.sort(function(a, b) {
          var ai = orderIndex[a.getAttribute('data-tab')];
          var bi = orderIndex[b.getAttribute('data-tab')];
          if (ai === undefined) ai = 999;
          if (bi === undefined) bi = 999;
          return ai - bi;
        });
        buttons.forEach(function(btn) { tabBar.appendChild(btn); });
        updateTabIndicator();
      }
    }

    // NOTE: `default_tab` is intentionally applied *after* the widget
    // queue drains below — see the post-drain block. Applying it here
    // would silently no-op for any widget-provided tab id, because
    // `switchTab()` looks up `#tab-{id}` and the widget panel hasn't
    // been mounted yet.

    // Apply chat config
    if (layout.chat) {
      if (layout.chat.suggestions === false) {
        var chips = document.getElementById('suggestion-chips');
        if (chips) chips.style.display = 'none';
      }
      if (layout.chat.image_upload === false) {
        // The visible affordance is `#attach-btn` (the paperclip in the
        // composer); the file input it triggers is `#image-file-input`.
        // Hide the button AND disable the input — hiding the button alone
        // wouldn't stop a programmatic `document.getElementById('image-file-input').click()`,
        // and operators that flip this flag almost always want the
        // capability gone, not just the chrome.
        var attachBtn = document.getElementById('attach-btn');
        if (attachBtn) attachBtn.style.display = 'none';
        var imgInput = document.getElementById('image-file-input');
        if (imgInput) imgInput.disabled = true;
      }
    }
  })();
}

// Drain any widgets that were registered before the DOM was ready.
// _addWidgetTab queues them in _widgetInitQueue when tab-bar doesn't exist yet.
if (IronClaw._widgetInitQueue && IronClaw._widgetInitQueue.length > 0) {
  IronClaw._widgetInitQueue.forEach(function(def) {
    _addWidgetTab(def);
  });
  IronClaw._widgetInitQueue = [];
}

// Apply `default_tab` after the widget queue has drained.
//
// If a layout sets `tabs.default_tab` to a widget-provided id (say
// "dashboard"), the corresponding `#tab-dashboard` panel does not exist
// until `_addWidgetTab` runs. Calling `switchTab("dashboard")` from
// inside the layout IIFE above (which runs first) used to silently
// no-op — the user landed on the default built-in tab instead and the
// `default_tab` setting appeared broken.
//
// Hash navigation still wins (so `#chat` deep-links survive a
// customized default_tab) and we only switch if a layout was injected.
if (window.__IRONCLAW_LAYOUT__
    && window.__IRONCLAW_LAYOUT__.tabs
    && window.__IRONCLAW_LAYOUT__.tabs.default_tab
    && !window.location.hash) {
  switchTab(window.__IRONCLAW_LAYOUT__.tabs.default_tab);
}
