// IronClaw Web Gateway - Orchestrator
// All modules loaded via <script> tags before this file.

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
      if (crCurrentProjectId) { document.getElementById('cr-detail').style.display = 'none'; }
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
