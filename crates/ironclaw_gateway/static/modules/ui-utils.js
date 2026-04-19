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
