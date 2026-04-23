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
    case 'cr-drill':
      drillIntoProject(el.dataset.id);
      break;
    case 'cr-back':
      crBackToOverview();
      break;
    case 'cr-close-detail':
      document.getElementById('cr-detail').style.display = 'none';
      break;
    case 'cr-att-click':
      if (el.dataset.project) drillIntoProject(el.dataset.project);
      break;
    case 'cr-new-project':
      crNewProject();
      break;
    case 'open-mission':
      openMissionDetail(el.dataset.id);
      break;
    case 'close-mission-detail':
      if (crCurrentProjectId) {
        document.getElementById('cr-detail').style.display = 'none';
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
      else document.getElementById('cr-detail').style.display = 'none';
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

