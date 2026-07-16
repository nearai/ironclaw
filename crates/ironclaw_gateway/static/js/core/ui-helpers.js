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

// --- Instance name (sidebar brand row) ---
//
// MOCK: the workspace instance name is client-side only for now — persisted
// in localStorage, no backend field. Click the name in the sidebar header to
// rename inline.

const INSTANCE_NAME_KEY = 'ironclaw-instance-name';

function getInstanceName() {
  try {
    const stored = (localStorage.getItem(INSTANCE_NAME_KEY) || '').trim();
    if (stored) return stored.slice(0, 40);
  } catch (e) {}
  return 'IronClaw';
}

function applyInstanceName() {
  const text = document.getElementById('instance-name-text');
  if (text) text.textContent = getInstanceName();
  if (typeof updateTopbarTitle === 'function') updateTopbarTitle();
}

function startInstanceRename() {
  const btn = document.getElementById('instance-name-btn');
  if (!btn || btn.querySelector('.instance-name-input')) return;
  const input = document.createElement('input');
  input.type = 'text';
  input.className = 'instance-name-input';
  input.maxLength = 40;
  input.value = getInstanceName();
  btn.style.display = 'none';
  btn.parentNode.insertBefore(input, btn);
  input.focus();
  input.select();

  let finished = false;
  const finish = (save) => {
    if (finished) return;
    finished = true;
    if (save) {
      const value = input.value.trim().slice(0, 40);
      try {
        if (value) localStorage.setItem(INSTANCE_NAME_KEY, value);
        else localStorage.removeItem(INSTANCE_NAME_KEY);
      } catch (e) {}
    }
    input.remove();
    btn.style.display = '';
    applyInstanceName();
  };
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); finish(true); }
    if (e.key === 'Escape') { e.preventDefault(); finish(false); }
  });
  input.addEventListener('blur', () => finish(true));
}

// The brand name in the sidebar header is intentionally not editable
// (startInstanceRename stays for the Settings surface to reuse).
applyInstanceName();

// Keyboard shortcut: `n` starts a new chat (ignored while typing).
document.addEventListener('keydown', (e) => {
  if (e.key !== 'n' || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
  const target = e.target;
  if (target && (
    target.tagName === 'INPUT'
    || target.tagName === 'TEXTAREA'
    || target.tagName === 'SELECT'
    || target.isContentEditable
  )) return;
  e.preventDefault();
  if (currentTab !== 'chat') switchTab('chat');
  createNewThread();
});

// --- Shared lucide glyphs ---
//
// Small inline icon set (lucide path data) shared by Skills, Integrations,
// and Discover so every entry can carry a distinct, purposeful mark.
const LUCIDE_PATHS = {
  'sparkles': '<path d="m12 3-1.9 5.8a2 2 0 0 1-1.3 1.3L3 12l5.8 1.9a2 2 0 0 1 1.3 1.3L12 21l1.9-5.8a2 2 0 0 1 1.3-1.3L21 12l-5.8-1.9a2 2 0 0 1-1.3-1.3Z"/>',
  'globe': '<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>',
  'app-window': '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M10 4v4"/><path d="M2 8h20"/><path d="M6 4v4"/>',
  'search': '<circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>',
  'list': '<line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/>',
  'sunrise': '<path d="M12 2v8"/><path d="m4.93 10.93 1.41 1.41"/><path d="M2 18h2"/><path d="M20 18h2"/><path d="m19.07 10.93-1.41 1.41"/><path d="M22 22H2"/><path d="m8 6 4-4 4 4"/><path d="M16 18a4 4 0 0 0-8 0"/>',
  'inbox': '<polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>',
  'calendar-clock': '<path d="M21 7.5V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h3.5"/><path d="M16 2v4"/><path d="M8 2v4"/><path d="M3 10h5"/><circle cx="16" cy="16" r="6"/><path d="M16 14v2l1 1"/>',
  'file-diff': '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M9 10h6"/><path d="M12 13V7"/><path d="M9 17h6"/>',
  'bar-chart': '<line x1="12" y1="20" x2="12" y2="10"/><line x1="18" y1="20" x2="18" y2="4"/><line x1="6" y1="20" x2="6" y2="16"/>',
  'git-pull-request': '<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M13 6h3a2 2 0 0 1 2 2v7"/><line x1="6" x2="6" y1="9" y2="21"/>',
  'radar': '<path d="M19.07 4.93A10 10 0 0 0 6.99 3.34"/><path d="M4 6h.01"/><path d="M2.29 9.62a10 10 0 1 0 19.02-1.27"/><path d="M16.24 7.76a6 6 0 1 0-8.01 8.91"/><path d="M12 18h.01"/><path d="M17.99 11.66a6 6 0 0 1-2.22 4.75"/><circle cx="12" cy="12" r="2"/><path d="m13.41 10.59 5.66-5.66"/>',
};

function lucideGlyphSvg(name, size) {
  const path = LUCIDE_PATHS[name] || LUCIDE_PATHS['sparkles'];
  return '<svg width="' + (size || 15) + '" height="' + (size || 15) + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + path + '</svg>';
}

// --- Agent home / welcome card ---
//
// The empty chat state is the agent's home: it leads with what the agent can
// do (use-case quick-starts, capability-demonstrating prompts) and the state
// of its connections, instead of generic feature chips. Everything routes
// through chat — config surfaces stay out of the way.

function prefillChatPrompt(text) {
  const input = document.getElementById('chat-input');
  if (!input || input.disabled || !text) return;
  input.value = text;
  // Reuse the input pipeline so autosize + send-button state stay in sync.
  input.dispatchEvent(new Event('input'));
  input.focus();
}

// Derive 3-5 immediately actionable suggestions from the onboarding handoff
// (?usecase= + ?integrations= → sessionStorage, see landing.js). Each entry is
// { label, prompt }: the label renders on the chip, clicking fills the
// composer with the prompt (never auto-sends).
function getHandoffSuggestions() {
  const useCases = (typeof NUX_DATA !== 'undefined' && NUX_DATA.useCases) || [];
  const useCaseId = typeof getHandoffUseCaseId === 'function' ? getHandoffUseCaseId() : null;
  const connected = typeof getHandoffConnectedIntegrations === 'function'
    ? getHandoffConnectedIntegrations()
    : [];
  if (!useCaseId && connected.length === 0) return [];

  const suggestions = [];
  const seen = new Set();
  const push = (useCase) => {
    if (!useCase || seen.has(useCase.id) || suggestions.length >= 5) return;
    seen.add(useCase.id);
    suggestions.push({ label: useCase.title, prompt: useCase.prompt });
  };

  // 1. The use case picked during onboarding leads.
  const picked = useCases.find((u) => u.id === useCaseId) || null;
  push(picked);

  // 2. Use cases powered by the integrations the user connected.
  connected.forEach((id) => {
    useCases
      .filter((u) => (u.integrations || []).indexOf(id) !== -1)
      .forEach(push);
  });

  // 3. Pad with same-category neighbours so there are at least 3 chips.
  if (picked && suggestions.length < 3) {
    useCases.filter((u) => u.category === picked.category).forEach(push);
  }
  if (suggestions.length < 3) {
    useCases.forEach(push);
  }
  return suggestions.slice(0, 5);
}

// One coherent suggestion set for the empty state: use-case-aware handoff
// suggestions lead when onboarding params exist, padded from the curated
// use-case gallery. Always 4-5 chips, each pre-fills the composer.
function getWelcomeSuggestions() {
  const out = [];
  const seen = new Set();
  const add = (label, prompt) => {
    if (!label || !prompt || seen.has(label) || out.length >= 5) return;
    seen.add(label);
    out.push({ label: label, prompt: prompt });
  };
  getHandoffSuggestions().forEach((s) => add(s.label, s.prompt));
  ((typeof NUX_DATA !== 'undefined' && NUX_DATA.useCases) || [])
    .forEach((u) => add(u.title, u.prompt));
  return out;
}

// First-run setup checklist progress (MOCK: session-scoped, purely visual).
const NUX_CHECKLIST_DONE_KEY = 'ironclaw_nux_checklist_done';

function getChecklistDone() {
  try {
    const raw = sessionStorage.getItem(NUX_CHECKLIST_DONE_KEY);
    return new Set(raw ? JSON.parse(raw) : []);
  } catch (e) {
    return new Set();
  }
}

function markChecklistDone(id) {
  const done = getChecklistDone();
  done.add(id);
  try { sessionStorage.setItem(NUX_CHECKLIST_DONE_KEY, JSON.stringify([...done])); } catch (e) {}
}

const WELCOME_CLAW_SVG =
  '<svg viewBox="45.2 34.11 54.25 54.25" fill="currentColor" aria-hidden="true">'
  + '<path d="M93.67,34.12c-2.01,0-3.87,1.04-4.93,2.75l-11.34,16.83c-.37.55-.22,1.3.34,1.67.45.3,1.04.26,1.45-.09l11.16-9.68c.19-.17.47-.15.64.04.08.08.12.19.12.31v30.31c0,.25-.2.45-.45.45-.13,0-.26-.06-.35-.16l-33.74-40.39c-1.1-1.3-2.71-2.04-4.41-2.05h-1.18c-3.19,0-5.78,2.59-5.78,5.78v42.69c0,3.19,2.59,5.78,5.78,5.78,2.01,0,3.87-1.04,4.93-2.75l11.34-16.83c.37-.55.22-1.3-.34-1.67-.45-.3-1.04-.26-1.45.09l-11.16,9.68c-.19.17-.47.15-.64-.04-.08-.08-.12-.19-.11-.31v-30.32c0-.25.2-.45.45-.45.13,0,.26.06.35.16l33.73,40.39c1.1,1.3,2.71,2.04,4.41,2.05h1.18c3.19,0,5.78-2.58,5.78-5.78v-42.69c0-3.19-2.59-5.78-5.78-5.78h0Z"/></svg>';

function showWelcomeCard() {
  const container = document.getElementById('chat-messages');
  if (!container || container.querySelector('.welcome-card')) return;
  const card = document.createElement('div');
  card.className = 'welcome-card';

  // Hero: the claw glyph on a soft blue radial disc, above the headline.
  const hero = document.createElement('div');
  hero.className = 'welcome-hero';
  hero.setAttribute('aria-hidden', 'true');
  hero.innerHTML = WELCOME_CLAW_SVG;
  card.appendChild(hero);

  const heading = document.createElement('h2');
  heading.className = 'welcome-heading';
  heading.textContent = I18n.t('welcome.heading');
  card.appendChild(heading);

  const desc = document.createElement('p');
  desc.className = 'welcome-description';
  desc.textContent = I18n.t('welcome.description');
  card.appendChild(desc);

  // ONE coherent suggestion group (4-5 chips, use-case-aware when the
  // onboarding handoff provided params). Pre-fills the composer, never
  // auto-sends.
  const suggestions = getWelcomeSuggestions();
  if (suggestions.length > 0) {
    const chips = document.createElement('div');
    chips.className = 'welcome-chips';
    suggestions.forEach((s) => {
      const chip = document.createElement('button');
      chip.type = 'button';
      chip.className = 'welcome-chip';
      chip.textContent = s.label;
      chip.title = s.prompt;
      chip.addEventListener('click', () => prefillChatPrompt(s.prompt));
      chips.appendChild(chip);
    });
    card.appendChild(chips);
  }

  // Setup is an inline task from the agent (MOCK: client-authored message,
  // not a real engine turn) — replaces the old "Set up your agent" button.
  card.appendChild(buildWelcomeAgentSetup(suggestions));

  container.appendChild(card);
}

// Agent-authored first-run message with a small actionable checklist.
// MOCK: rendered client-side; completion state is session-scoped and purely
// cosmetic (see NUX_CHECKLIST_DONE_KEY).
function buildWelcomeAgentSetup(suggestions) {
  const wrap = document.createElement('div');
  wrap.className = 'welcome-agent-msg';

  const avatar = document.createElement('span');
  avatar.className = 'welcome-agent-avatar';
  avatar.setAttribute('aria-hidden', 'true');
  avatar.innerHTML = WELCOME_CLAW_SVG;
  wrap.appendChild(avatar);

  const bubble = document.createElement('div');
  bubble.className = 'welcome-agent-bubble';

  const intro = document.createElement('p');
  intro.className = 'welcome-agent-text';
  intro.textContent = I18n.t('welcome.agentSetupIntro');
  bubble.appendChild(intro);

  const items = [
    {
      id: 'channel',
      label: I18n.t('welcome.checkChannel'),
      run: () => switchTab('integrations'),
    },
    {
      id: 'task',
      label: I18n.t('welcome.checkFirstTask'),
      run: () => {
        const first = suggestions && suggestions[0];
        if (first) prefillChatPrompt(first.prompt);
      },
    },
    {
      id: 'integrations',
      label: I18n.t('welcome.checkIntegrations'),
      run: () => switchTab('integrations'),
    },
  ];

  const list = document.createElement('div');
  list.className = 'welcome-checklist';
  const done = getChecklistDone();
  items.forEach((item) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'welcome-check-item' + (done.has(item.id) ? ' done' : '');
    btn.innerHTML =
      '<span class="welcome-check-circle" aria-hidden="true">'
      + '<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>'
      + '</span>'
      + '<span class="welcome-check-label">' + escapeHtml(item.label) + '</span>';
    btn.addEventListener('click', () => {
      markChecklistDone(item.id);
      btn.classList.add('done');
      item.run();
    });
    list.appendChild(btn);
  });
  bubble.appendChild(list);

  wrap.appendChild(bubble);
  return wrap;
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

  // Close the mobile sidebar drawer
  closeMobileSidebar();
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
// Bug #3082 recovery affordances on the progress modal.
document.getElementById('restart-refresh-btn').addEventListener('click', () => window.location.reload());
document.getElementById('restart-dismiss-btn').addEventListener('click', () => dismissRestartLoader());
document.getElementById('thread-new-btn').addEventListener('click', () => {
  if (currentTab !== 'chat') switchTab('chat');
  createNewThread();
});
document.getElementById('sidebar-collapse-btn')?.addEventListener('click', () => toggleSidebarCollapsed());
document.getElementById('mobile-sidebar-btn')?.addEventListener('click', () => openMobileSidebar());
document.getElementById('sidebar-scrim')?.addEventListener('click', () => closeMobileSidebar());
document.getElementById('send-btn').addEventListener('click', () => sendMessage());
document.getElementById('memory-edit-btn').addEventListener('click', () => startMemoryEdit());
document.getElementById('memory-save-btn').addEventListener('click', () => saveMemoryEdit());
document.getElementById('memory-cancel-btn').addEventListener('click', () => cancelMemoryEdit());
document.getElementById('logs-server-level').addEventListener('change', (e) => setServerLogLevel(e.target.value));
document.getElementById('logs-pause-btn').addEventListener('click', () => toggleLogsPause());
document.getElementById('logs-download-btn').addEventListener('click', () => downloadLogsJsonl());
document.getElementById('logs-clear-btn').addEventListener('click', () => clearLogs());
document.getElementById('wasm-install-btn').addEventListener('click', () => installWasmExtension());
document.getElementById('mcp-add-btn').addEventListener('click', () => addMcpServer());
document.getElementById('skill-search-btn').addEventListener('click', () => searchClawHub());
document.getElementById('skill-install-btn').addEventListener('click', () => installSkillFromForm());
document.getElementById('settings-export-btn').addEventListener('click', () => exportSettings());
document.getElementById('settings-import-btn').addEventListener('click', () => importSettings());
document.getElementById('settings-back-btn')?.addEventListener('click', () => settingsBack());
// "Back to app" exits the settings takeover to the chat surface.
document.getElementById('settings-back-app')?.addEventListener('click', () => switchTab('chat'));

// --- "Just ask the agent" affordance ---
// Config surfaces exist 'just in case'; the canonical way to do anything is
// to ask the agent in chat. This hint teaches that behavior exactly where
// users would otherwise start clicking through menus.

function createAskAgentHint(labelKey, prompt) {
  const hint = document.createElement('button');
  hint.type = 'button';
  hint.className = 'ask-agent-hint';

  const glyph = document.createElement('span');
  glyph.className = 'ask-agent-hint-glyph';
  glyph.setAttribute('aria-hidden', 'true');
  glyph.textContent = '\u25C6';
  hint.appendChild(glyph);

  const text = document.createElement('span');
  text.textContent = I18n.t(labelKey);
  hint.appendChild(text);

  hint.addEventListener('click', () => {
    switchTab('chat');
    prefillChatPrompt(prompt);
  });
  return hint;
}

(function injectAskAgentHints() {
  const discoverHeader = document.querySelector('#tab-discover .discover-header');
  if (discoverHeader) {
    discoverHeader.appendChild(
      createAskAgentHint('askAgent.discover', I18n.t('askAgent.discoverPrompt'))
    );
  }
  const settingsToolbar = document.querySelector('#tab-settings .settings-toolbar');
  if (settingsToolbar) {
    settingsToolbar.parentNode.insertBefore(
      createAskAgentHint('askAgent.settings', I18n.t('askAgent.settingsPrompt')),
      settingsToolbar
    );
  }
})();

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
    case 'cr-drill':
      drillIntoProject(el.dataset.id);
      break;
    case 'cr-back':
      crBackToOverview();
      break;
    case 'cr-close-detail':
      closeCrDetail();
      break;
    case 'cr-att-click':
      if (el.dataset.project) drillIntoProject(el.dataset.project);
      break;
    case 'cr-new-project':
      crNewProject();
      break;
    case 'open-project-mission':
      openMissionFromProjects(el.dataset.id);
      break;
    case 'open-mission':
      openMissionDetail(el.dataset.id);
      break;
    case 'close-mission-detail':
      if (crCurrentProjectId) {
        closeCrDetail();
      } else {
        closeMissionDetail();
      }
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
      else closeCrDetail();
      break;
    case 'open-active-work':
      if (el.dataset.kind === 'job') {
        switchTab('jobs');
        openJobDetail(el.dataset.id);
      } else {
        switchTab('missions');
        openMissionDetail(el.dataset.missionId || el.dataset.id);
      }
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

  // 2b. Provider cards (Inference)
  var providerCards = activePanel.querySelectorAll('.provider-card');
  providerCards.forEach(function(card) {
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
    var visibleItems = section.querySelectorAll('.ext-card:not(.search-hidden), .tool-permission-row:not(.search-hidden), .provider-card:not(.search-hidden)');
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

