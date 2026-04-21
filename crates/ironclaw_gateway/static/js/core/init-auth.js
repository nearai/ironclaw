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
  refreshSlashSkillEntries();
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
    _sseDisconnectedAt = _sseDisconnectedAt || Date.now();
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
