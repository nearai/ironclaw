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
