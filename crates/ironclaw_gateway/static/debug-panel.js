/* Debug Inspector Panel — IronClaw Web Gateway */

(function () {
  'use strict';

  // ── Constants ──

  const MAX_ACTIVITY = 1000;
  const STATS_POLL_INTERVAL = 30000;
  const SESSION_TAB_KEY = 'ironclaw_debug_tab';
  const SESSION_OPEN_KEY = 'ironclaw_debug_open';

  // ── State ──

  let debugActive = false;
  let panelOpen = false;
  let activeTab = 'activity';
  let activityLog = [];    // all entries across turns
  let pendingTools = {};
  let currentTurn = 0;     // increments on each user message
  let viewingTurn = 0;     // which turn the Activity tab is showing
  let firstAvailableTurn = 1; // bumped when eviction drops a whole turn
  let overlay = null;
  let panelEl = null;
  let toolbarBtn = null;
  let statsTimer = null;
  let sseReconnects = 0;
  let totalEventsReceived = 0;
  let lastEventTime = null;

  let sessionStats = {
    turns: 0,
    inputTokens: 0,
    outputTokens: 0,
    cost: 0,
    toolCalls: 0,
    toolSuccess: 0,
    toolFailure: 0
  };

  // ── Initialization ──

  function init() {
    // Debug mode detection is done in <head> inline script, which sets window.isDebugMode
    debugActive = window.isDebugMode;
    if (!debugActive) return;

    activeTab = sessionStorage.getItem(SESSION_TAB_KEY) || 'activity';
    panelOpen = sessionStorage.getItem(SESSION_OPEN_KEY) !== 'false';

    currentTurn = 0;
    viewingTurn = 0;

    createToolbarButton();
    createPanel();
    hookSSE();
    hookSendMessage();

    if (panelOpen) openPanel();

    statsTimer = setInterval(function () {
      fetchGatewayStats();
      updateSseHealthDisplay();
    }, STATS_POLL_INTERVAL);

    // Auto-load data after a short delay to ensure DOM is ready
    setTimeout(function () {
      fetchPromptData();
      fetchGatewayStats();
      updateSseHealthDisplay();
    }, 300);
  }

  // ── Toolbar button ──

  function createToolbarButton() {
    var tabBar = document.querySelector('.tab-bar');
    if (!tabBar) return;

    var spacer = tabBar.querySelector('.spacer');
    if (!spacer) return;

    toolbarBtn = document.createElement('button');
    toolbarBtn.className = 'debug-toolbar-btn';
    toolbarBtn.type = 'button';
    toolbarBtn.innerHTML = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>';
    toolbarBtn.setAttribute('data-i18n', 'debug.togglePanel');
    toolbarBtn.setAttribute('data-i18n-attr', 'title');
    toolbarBtn.title = t('debug.togglePanel');
    toolbarBtn.addEventListener('click', togglePanel);

    tabBar.insertBefore(toolbarBtn, spacer.nextSibling);
  }

  // ── Panel DOM creation ──

  function createPanel() {
    // Overlay for mobile
    overlay = document.createElement('div');
    overlay.className = 'debug-panel-overlay';
    overlay.addEventListener('click', closePanel);
    document.body.appendChild(overlay);

    panelEl = document.createElement('div');
    panelEl.className = 'debug-panel';
    panelEl.id = 'debug-panel';

    // Header
    var header = document.createElement('div');
    header.className = 'debug-header';
    var title = document.createElement('span');
    title.className = 'debug-header-title';
    title.setAttribute('data-i18n', 'debug.title');
    title.textContent = t('debug.title');
    var closeBtn = document.createElement('button');
    closeBtn.className = 'debug-close-btn';
    closeBtn.textContent = '\u00D7';
    closeBtn.title = t('common.close');
    closeBtn.addEventListener('click', closePanel);
    header.appendChild(title);
    header.appendChild(closeBtn);

    // Tab bar
    var tabBar = document.createElement('div');
    tabBar.className = 'debug-tab-bar';
    var tabs = [
      { id: 'prompt', key: 'debug.tabPrompt', label: 'Prompt' },
      { id: 'activity', key: 'debug.tabActivity', label: 'Activity' },
      { id: 'stats', key: 'debug.tabStats', label: 'Stats' }
    ];
    tabs.forEach(function (tab) {
      var btn = document.createElement('button');
      btn.setAttribute('data-debug-tab', tab.id);
      btn.setAttribute('data-i18n', tab.key);
      btn.textContent = t(tab.key);
      if (tab.id === activeTab) btn.classList.add('active');
      btn.addEventListener('click', function () { switchDebugTab(tab.id); });
      tabBar.appendChild(btn);
    });

    // Tab content
    var content = document.createElement('div');
    content.className = 'debug-tab-content';

    // Prompt pane
    var promptPane = document.createElement('div');
    promptPane.className = 'debug-tab-pane' + (activeTab === 'prompt' ? ' active' : '');
    promptPane.id = 'debug-pane-prompt';
    promptPane.innerHTML = ''; // built dynamically
    buildPromptPane(promptPane);

    // Activity pane
    var activityPane = document.createElement('div');
    activityPane.className = 'debug-tab-pane' + (activeTab === 'activity' ? ' active' : '');
    activityPane.id = 'debug-pane-activity';
    buildActivityPane(activityPane);

    // Stats pane
    var statsPane = document.createElement('div');
    statsPane.className = 'debug-tab-pane' + (activeTab === 'stats' ? ' active' : '');
    statsPane.id = 'debug-pane-stats';
    buildStatsPane(statsPane);

    content.appendChild(promptPane);
    content.appendChild(activityPane);
    content.appendChild(statsPane);

    panelEl.appendChild(header);
    panelEl.appendChild(tabBar);
    panelEl.appendChild(content);

    // Insert into #tab-chat layout
    var tabChat = document.getElementById('tab-chat');
    if (tabChat) {
      tabChat.appendChild(panelEl);
    }
  }

  // ── Tab switching ──

  function switchDebugTab(tabId) {
    activeTab = tabId;
    sessionStorage.setItem(SESSION_TAB_KEY, tabId);
    if (!panelEl) return;
    panelEl.querySelectorAll('[data-debug-tab]').forEach(function (btn) {
      btn.classList.toggle('active', btn.getAttribute('data-debug-tab') === tabId);
    });
    panelEl.querySelectorAll('.debug-tab-pane').forEach(function (pane) {
      pane.classList.toggle('active', pane.id === 'debug-pane-' + tabId);
    });
    // Auto-refresh data when switching tabs
    if (tabId === 'prompt') fetchPromptData();
    if (tabId === 'activity') rebuildActivityDOM();
    if (tabId === 'stats') { fetchGatewayStats(); updateSseHealthDisplay(); }
  }

  // ── Open / close ──

  function openPanel() {
    panelOpen = true;
    sessionStorage.setItem(SESSION_OPEN_KEY, 'true');
    if (panelEl) panelEl.classList.add('open');
    if (toolbarBtn) toolbarBtn.classList.add('active');
    if (overlay) overlay.classList.add('open');
  }

  function closePanel() {
    panelOpen = false;
    sessionStorage.setItem(SESSION_OPEN_KEY, 'false');
    if (panelEl) panelEl.classList.remove('open');
    if (toolbarBtn) toolbarBtn.classList.remove('active');
    if (overlay) overlay.classList.remove('open');
  }

  function togglePanel() {
    if (panelOpen) closePanel(); else openPanel();
  }

  // ── SSE integration ──

  var currentEventSource = null;

  function hookSSE() {
    var isFirstConnect = true;
    // Register hook for app.js to call after creating eventSource
    window.onDebugSSEConnect = function (es) {
      currentEventSource = es;
      attachDebugListeners(es);
      if (!isFirstConnect) sseReconnects++;
      isFirstConnect = false;
    };

    // Trigger a reconnect so the hook fires with debug=true URL
    if (typeof window.connectSSE === 'function') {
      window.connectSSE();
    }
  }

  function attachDebugListeners(es) {
    es.addEventListener('status', function (e) {
      try {
        var data = JSON.parse(e.data);
        addActivity('think', t('debug.activityStatus'), timeNow(), null, data.message || null, { labelKey: 'debug.activityStatus' });
      } catch (_) { /* ignore */ }
      lastEventTime = Date.now(); totalEventsReceived++;
    });

    es.addEventListener('thinking', function (e) {
      try {
        var data = JSON.parse(e.data);
        addActivity('think', t('debug.activityThinking'), timeNow(), null, data.message || null, { labelKey: 'debug.activityThinking' });
      } catch (_) { /* ignore */ }
      lastEventTime = Date.now(); totalEventsReceived++;
    });

    es.addEventListener('tool_started', function (e) {
      try {
        var data = JSON.parse(e.data);
        var key = data.call_id || data.name;
        var id = addActivity('tool', data.name || 'tool', timeNow(), 'pending');
        pendingTools[key] = { id: id, start: Date.now(), name: data.name };
        sessionStats.toolCalls++;
        updateStatsDisplay();
      } catch (_) { /* ignore */ }
      lastEventTime = Date.now(); totalEventsReceived++;
    });

    es.addEventListener('tool_completed', function (e) {
      try {
        var data = JSON.parse(e.data);
        var key = data.call_id || data.name;
        var pending = pendingTools[key];
        var duration = pending ? (Date.now() - pending.start) : null;
        var status = data.success ? 'success' : 'failure';
        var meta = duration ? formatDuration(duration) : '';

        if (data.success) sessionStats.toolSuccess++;
        else sessionStats.toolFailure++;

        var extra = {};
        if (data.parameters) extra.params = data.parameters;
        if (data.error) extra.output = data.error;

        if (pending) {
          updateActivity(pending.id, status, meta, extra);
          // Keep entry briefly so tool_result / tool_result_full can still find it,
          // then remove it.
          setTimeout(function () {
            delete pendingTools[key];
          }, 5000);
        } else {
          addActivity('tool', data.name || 'tool', meta, status, null, extra);
        }
        updateStatsDisplay();
      } catch (_) { /* ignore */ }
      lastEventTime = Date.now(); totalEventsReceived++;
    });

    es.addEventListener('tool_result', function (e) {
      try {
        var data = JSON.parse(e.data);
        var key = data.call_id || data.name;
        var pending = pendingTools[key];
        if (pending) {
          appendActivityOutput(pending.id, data.preview || '');
        }
      } catch (_) { /* ignore */ }
      lastEventTime = Date.now(); totalEventsReceived++;
    });

    es.addEventListener('reasoning_update', function (e) {
      try {
        var data = JSON.parse(e.data);
        var body = data.narrative || '';
        if (data.decisions && data.decisions.length > 0) {
          body += '\n\n' + data.decisions.map(function (d) {
            return (d.chosen ? '\u2713 ' : '\u2717 ') + d.tool_name + ': ' + (d.reason || '');
          }).join('\n');
        }
        addActivity('think', t('debug.activityReasoning'), timeNow(), null, body, { labelKey: 'debug.activityReasoning' });
      } catch (_) { /* ignore */ }
      lastEventTime = Date.now(); totalEventsReceived++;
    });

    es.addEventListener('turn_cost', function (e) {
      try {
        var data = JSON.parse(e.data);
        sessionStats.turns++;
        sessionStats.inputTokens += data.input_tokens || 0;
        sessionStats.outputTokens += data.output_tokens || 0;
        var costVal = parseFloat(data.cost_usd);
        if (!isNaN(costVal)) sessionStats.cost += costVal;

        var costStr = (!isNaN(costVal) && costVal > 0) ? '$' + costVal.toFixed(4) : '';
        var info = t('debug.infoIn') + ' ' + formatNumber(data.input_tokens || 0) + 't  ' + t('debug.infoOut') + ' ' + formatNumber(data.output_tokens || 0) + 't';
        if (costStr) info += '  ' + t('debug.infoCost') + ' ' + costStr;

        addActivity('llm', t('debug.activityLlmCall') + ' #' + sessionStats.turns, '', null, null, { info: info, labelKey: 'debug.activityLlmCall' });
        updateStatsDisplay();
        // Refresh gateway stats to pick up latest model usage
        fetchGatewayStats();
      } catch (_) { /* ignore */ }
      lastEventTime = Date.now(); totalEventsReceived++;
    });

    es.addEventListener('response', function (e) {
      try {
        var data = JSON.parse(e.data);
        var preview = (data.content || '').substring(0, 100);
        if ((data.content || '').length > 100) preview += '...';
        addActivity('stream', t('debug.activityResponse'), timeNow(), 'success', preview, { labelKey: 'debug.activityResponse' });
      } catch (_) { /* ignore */ }
      lastEventTime = Date.now(); totalEventsReceived++;
    });

    es.addEventListener('error', function (e) {
      try {
        var data = JSON.parse(e.data);
        addActivity('error', t('debug.activityError'), timeNow(), 'failure', data.message || null, { labelKey: 'debug.activityError' });
      } catch (_) { /* ignore */ }
      lastEventTime = Date.now(); totalEventsReceived++;
    });

    es.addEventListener('turn_metrics', function (e) {
      try {
        var data = JSON.parse(e.data);
        var info = t('debug.infoModel') + ' ' + data.model;
        info += '\n' + t('debug.infoIn') + ' ' + formatNumber(data.input_tokens || 0) + 't  ' + t('debug.infoOut') + ' ' + formatNumber(data.output_tokens || 0) + 't';
        if (data.cache_read_tokens) info += '  ' + t('debug.infoCache') + ' ' + formatNumber(data.cache_read_tokens) + 't';
        var duration = data.duration_ms ? formatDuration(data.duration_ms) : '';
        addActivity('llm', t('debug.activityLlmCall') + ' #' + (data.iteration + 1), duration, null, null, { info: info, labelKey: 'debug.activityLlmCall' });
      } catch (_) { /* ignore */ }
      lastEventTime = Date.now(); totalEventsReceived++;
    });

    es.addEventListener('tool_result_full', function (e) {
      try {
        var data = JSON.parse(e.data);
        var key = data.call_id || data.name;
        var pending = pendingTools[key];
        if (!pending) {
          // Fallback: search by name for older events without call_id
          var keys = Object.keys(pendingTools);
          for (var i = 0; i < keys.length; i++) {
            if (pendingTools[keys[i]].name === data.name) { pending = pendingTools[keys[i]]; break; }
          }
        }
        if (pending) {
          appendActivityOutput(pending.id, data.output || '');
        }
      } catch (_) { /* ignore */ }
      lastEventTime = Date.now(); totalEventsReceived++;
    });

    es.addEventListener('stream_chunk', function () {
      lastEventTime = Date.now(); totalEventsReceived++;
    });
  }

  // ── Hook send message to start new turn ──

  function hookSendMessage() {
    var origSend = window.sendMessage;
    if (typeof origSend === 'function') {
      window.sendMessage = function () {
        startNewTurn();
        return origSend.apply(window, arguments);
      };
    }

    // Hook addMessage to stamp user messages with turn number + add click handler
    var origAddMessage = window.addMessage;
    if (typeof origAddMessage === 'function') {
      window.addMessage = function (role, content) {
        var el = origAddMessage.apply(window, arguments);
        if (el) {
          el.setAttribute('data-debug-turn', String(currentTurn));
          el.addEventListener('click', function () {
            var turn = parseInt(el.getAttribute('data-debug-turn'), 10);
            if (turn && turn > 0) {
              viewTurn(turn);
              if (!panelOpen) openPanel();
              switchDebugTab('activity');
            }
          });
        }
        return el;
      };
    }
  }

  // ── Activity management ──

  var activityIdCounter = 0;

  function addActivity(type, label, meta, status, body, extra) {
    // Merge consecutive entries of the same type within the same second
    var now = new Date();
    var timeStr = meta && /^\d{2}:\d{2}:\d{2}$/.test(meta) ? meta : '';
    if (timeStr && activityLog.length > 0) {
      var last = activityLog[activityLog.length - 1];
      if (last.turn === currentTurn && last.type === type && last.meta === timeStr && !status && !last.status) {
        // Append body text to the previous entry
        var newBody = body || '';
        if (last.body && newBody) {
          last.body = last.body + '\n' + newBody;
        } else if (newBody) {
          last.body = newBody;
        }
        // Update DOM
        var el = document.getElementById('debug-activity-' + last.id);
        if (el) {
          var pre = el.querySelector('.debug-activity-pre');
          if (pre) {
            pre.textContent = last.body;
          } else if (last.body) {
            var details = el.querySelector('.debug-activity-details');
            if (!details) {
              details = document.createElement('div');
              details.className = 'debug-activity-details';
              el.appendChild(details);
            }
            var p = document.createElement('pre');
            p.className = 'debug-activity-pre';
            p.textContent = last.body;
            details.appendChild(p);
          }
        }
        return last.id;
      }
    }

    var id = ++activityIdCounter;
    var entry = { id: id, turn: currentTurn, type: type, label: label, labelKey: extra && extra.labelKey || null, meta: meta || '', status: status, body: body || '', time: now };
    if (extra) {
      if (extra.params) entry.params = extra.params;
      if (extra.output) entry.output = extra.output;
      if (extra.info) entry.info = extra.info;
    }
    activityLog.push(entry);

    // Eviction — bump firstAvailableTurn when we drop the last entry
    // belonging to a turn so turn-nav can render an "evicted" placeholder
    // instead of silently showing an empty timeline for older turns.
    while (activityLog.length > MAX_ACTIVITY) {
      var dropped = activityLog.shift();
      if (dropped && dropped.turn >= firstAvailableTurn) {
        var stillHas = false;
        for (var i = 0; i < activityLog.length; i++) {
          if (activityLog[i].turn === dropped.turn) { stillHas = true; break; }
        }
        if (!stillHas) firstAvailableTurn = dropped.turn + 1;
      }
    }

    // Only render if viewing the current turn
    if (viewingTurn === currentTurn) {
      renderActivityEntry(entry);
    }
    return id;
  }

  function updateActivity(id, status, meta, extra) {
    var entry = activityLog.find(function (e) { return e.id === id; });
    if (!entry) return;
    if (status) entry.status = status;
    if (meta) entry.meta = meta;
    if (extra) {
      if (extra.params) entry.params = extra.params;
      if (extra.output) entry.output = extra.output;
    }

    var el = document.getElementById('debug-activity-' + id);
    if (!el) return;

    // Update status icon
    var statusEl = el.querySelector('.debug-activity-status-icon');
    if (statusEl) {
      statusEl.className = 'debug-activity-status-icon ' + (status || '');
      statusEl.textContent = status === 'success' ? '\u2713' : status === 'failure' ? '\u2717' : '\u2026';
    }
    // Update failure border
    if (status === 'failure') el.classList.add('failure');

    // Update badge
    var badgeEl = el.querySelector('.debug-activity-badge');
    if (badgeEl && meta) badgeEl.textContent = meta;

    // Rebuild details section
    if (extra && (extra.params || extra.output)) {
      var oldDetails = el.querySelector('.debug-activity-details');
      if (oldDetails) oldDetails.remove();

      var details = document.createElement('div');
      details.className = 'debug-activity-details';
      if (extra.params) {
        var pl = document.createElement('div');
        pl.className = 'debug-activity-section-label';
        pl.textContent = t('debug.activityParams');
        details.appendChild(pl);
        var pp = document.createElement('pre');
        pp.className = 'debug-activity-pre';
        pp.textContent = typeof extra.params === 'string' ? extra.params : JSON.stringify(extra.params, null, 2);
        details.appendChild(pp);
      }
      if (extra.output) {
        var ol = document.createElement('div');
        ol.className = 'debug-activity-section-label';
        ol.textContent = t('debug.activityOutput');
        details.appendChild(ol);
        var op = document.createElement('pre');
        op.className = 'debug-activity-pre debug-activity-output-pre';
        op.textContent = typeof extra.output === 'string' ? extra.output : JSON.stringify(extra.output, null, 2);
        details.appendChild(op);
      }
      el.appendChild(details);
    }

    updateTurnSummary();
  }

  function appendActivityOutput(id, text) {
    var entry = activityLog.find(function (e) { return e.id === id; });
    if (!entry) return;
    entry.output = (entry.output ? entry.output + '\n' : '') + text;
    var el = document.getElementById('debug-activity-' + id);
    if (!el) return;
    var outPre = el.querySelector('.debug-activity-details .debug-activity-output-pre');
    if (outPre) {
      outPre.textContent = entry.output;
    } else {
      // Create details section with output
      var details = el.querySelector('.debug-activity-details');
      if (!details) {
        details = document.createElement('div');
        details.className = 'debug-activity-details';
        el.appendChild(details);
      }
      var ol = document.createElement('div');
      ol.className = 'debug-activity-section-label';
      ol.textContent = t('debug.activityOutput');
      details.appendChild(ol);
      var op = document.createElement('pre');
      op.className = 'debug-activity-pre debug-activity-output-pre';
      op.textContent = text;
      details.appendChild(op);
    }
  }

  function startNewTurn() {
    pendingTools = {};
    currentTurn++;
    viewingTurn = currentTurn;
    // Evict oldest entries if over cap, mirroring the eviction in
    // addActivity(): bump firstAvailableTurn whenever the last entry of
    // a turn drops out of the buffer.
    while (activityLog.length > MAX_ACTIVITY) {
      var dropped = activityLog.shift();
      if (dropped && dropped.turn >= firstAvailableTurn) {
        var stillHas = false;
        for (var i = 0; i < activityLog.length; i++) {
          if (activityLog[i].turn === dropped.turn) { stillHas = true; break; }
        }
        if (!stillHas) firstAvailableTurn = dropped.turn + 1;
      }
    }
    rebuildActivityDOM();
    updateTurnNav();
  }

  function entriesForTurn(turn) {
    return activityLog.filter(function (e) { return e.turn === turn; });
  }

  function maxTurn() {
    return currentTurn;
  }

  function viewTurn(turn) {
    var lo = firstAvailableTurn;
    if (turn < lo) turn = lo;
    if (turn > maxTurn()) turn = maxTurn();
    viewingTurn = turn;
    rebuildActivityDOM();
    updateTurnNav();
  }

  function updateTurnNav() {
    var nav = document.getElementById('debug-turn-nav');
    if (!nav) return;
    var label = nav.querySelector('.debug-turn-label');
    var prevBtn = nav.querySelector('.debug-turn-prev');
    var nextBtn = nav.querySelector('.debug-turn-next');
    if (label) label.textContent = t('debug.activityTurn') + ' ' + viewingTurn + ' / ' + maxTurn();
    if (prevBtn) prevBtn.disabled = viewingTurn <= firstAvailableTurn;
    if (nextBtn) nextBtn.disabled = viewingTurn >= maxTurn();
    nav.style.display = maxTurn() > 0 ? 'flex' : 'none';
  }

  function rebuildActivityDOM() {
    var list = document.getElementById('debug-activity-list');
    if (!list) return;
    list.textContent = '';

    // If the user is browsing a turn whose entries have been evicted,
    // tell them so explicitly instead of silently rendering "no events"
    // — eviction at MAX_ACTIVITY=1000 is invisible without this.
    if (viewingTurn > 0 && viewingTurn < firstAvailableTurn) {
      var evicted = document.createElement('div');
      evicted.className = 'debug-activity-empty';
      evicted.textContent = t('debug.activityEvicted');
      list.appendChild(evicted);
      return;
    }

    var entries = entriesForTurn(viewingTurn);

    if (entries.length === 0) {
      var empty = document.createElement('div');
      empty.className = 'debug-activity-empty';
      empty.setAttribute('data-i18n', 'debug.activityEmpty');
      empty.textContent = t('debug.activityEmpty');
      list.appendChild(empty);
      return;
    }

    entries.forEach(function (entry) {
      renderActivityEntry(entry);
    });
  }

  function renderActivityEntry(entry) {
    var list = document.getElementById('debug-activity-list');
    if (!list) return;

    // Remove empty placeholder / clear notice
    var empty = list.querySelector('.debug-activity-empty');
    if (empty) empty.remove();
    var notice = list.querySelector('.debug-activity-clear-notice');
    if (notice) notice.remove();
    // Remove old summary before appending new entry
    var oldSummary = list.querySelector('.debug-activity-summary');
    if (oldSummary) oldSummary.remove();

    var el = document.createElement('div');
    el.className = 'debug-activity-entry';
    if (entry.status === 'failure') el.className += ' failure';
    el.id = 'debug-activity-' + entry.id;

    // Header row
    var head = document.createElement('div');
    head.className = 'debug-activity-head';
    head.addEventListener('click', function () {
      el.classList.toggle('expanded');
    });

    var icon = document.createElement('span');
    icon.className = 'debug-activity-icon ' + entry.type;
    var iconMap = { llm: '\u25CF', tool: '\u25C6', think: '\u25CB', stream: '\u25CF', error: '\u25CF' };
    icon.textContent = iconMap[entry.type] || '\u25CB';

    var label = document.createElement('span');
    label.className = 'debug-activity-label';
    // Resolve label from i18n key if available (supports language switching)
    if (entry.labelKey) {
      var resolved = t(entry.labelKey);
      // For LLM Call entries, append the turn number suffix
      if (entry.label && entry.label.indexOf('#') !== -1) {
        resolved += ' ' + entry.label.substring(entry.label.indexOf('#'));
      }
      label.textContent = resolved;
    } else {
      label.textContent = entry.label;
    }

    var badge = document.createElement('span');
    badge.className = 'debug-activity-badge';
    badge.textContent = entry.meta || '';

    head.appendChild(icon);
    head.appendChild(label);
    head.appendChild(badge);

    if (entry.status) {
      var statusIcon = document.createElement('span');
      statusIcon.className = 'debug-activity-status-icon ' + entry.status;
      statusIcon.textContent = entry.status === 'success' ? '\u2713' : entry.status === 'failure' ? '\u2717' : '\u2026';
      head.appendChild(statusIcon);
    }

    el.appendChild(head);

    // Info line (tokens for LLM calls)
    if (entry.info) {
      var info = document.createElement('div');
      info.className = 'debug-activity-info';
      info.textContent = entry.info;
      el.appendChild(info);
      el.classList.add('has-info');
    }

    // Collapsible details section
    var details = document.createElement('div');
    details.className = 'debug-activity-details';

    if (entry.params) {
      var paramLabel = document.createElement('div');
      paramLabel.className = 'debug-activity-section-label';
      paramLabel.textContent = t('debug.activityParams');
      details.appendChild(paramLabel);
      var paramPre = document.createElement('pre');
      paramPre.className = 'debug-activity-pre';
      paramPre.textContent = entry.params;
      details.appendChild(paramPre);
    }

    if (entry.output) {
      var outLabel = document.createElement('div');
      outLabel.className = 'debug-activity-section-label';
      outLabel.textContent = t('debug.activityOutput');
      details.appendChild(outLabel);
      var outPre = document.createElement('pre');
      outPre.className = 'debug-activity-pre debug-activity-output-pre';
      outPre.textContent = entry.output;
      details.appendChild(outPre);
    }

    if (entry.body) {
      var bodyPre = document.createElement('pre');
      bodyPre.className = 'debug-activity-pre';
      bodyPre.textContent = entry.body;
      details.appendChild(bodyPre);
    }

    if (details.children.length > 0) {
      el.appendChild(details);
    }

    list.appendChild(el);

    // Update turn summary
    updateTurnSummary();

    // Auto-scroll
    var content = panelEl ? panelEl.querySelector('.debug-tab-content') : null;
    if (content) {
      var isNearBottom = content.scrollHeight - content.scrollTop - content.clientHeight < 60;
      if (isNearBottom) {
        content.scrollTop = content.scrollHeight;
      }
    }
  }

  function updateTurnSummary() {
    var list = document.getElementById('debug-activity-list');
    if (!list) return;
    var old = list.querySelector('.debug-activity-summary');
    if (old) old.remove();

    var llmCalls = 0;
    var toolCalls = 0;
    entriesForTurn(viewingTurn).forEach(function (e) {
      if (e.type === 'llm') llmCalls++;
      if (e.type === 'tool') toolCalls++;
    });
    if (llmCalls === 0 && toolCalls === 0) return;

    var summary = document.createElement('div');
    summary.className = 'debug-activity-summary';
    var parts = [];
    if (llmCalls > 0) parts.push(t('debug.summaryLlmCalls').replace('{count}', llmCalls));
    if (toolCalls > 0) parts.push(t('debug.summaryToolCalls').replace('{count}', toolCalls));
    summary.textContent = parts.join(' | ');
    list.appendChild(summary);
  }

  // ── Build panes ──

  function buildPromptPane(pane) {
    var header = document.createElement('div');
    header.className = 'debug-prompt-header';

    var total = document.createElement('span');
    total.className = 'debug-prompt-total';
    total.id = 'debug-prompt-total';
    total.textContent = '';

    var refreshBtn = document.createElement('button');
    refreshBtn.className = 'debug-prompt-refresh';
    refreshBtn.setAttribute('data-i18n', 'debug.promptRefresh');
    refreshBtn.textContent = t('debug.promptRefresh');
    refreshBtn.addEventListener('click', fetchPromptData);

    header.appendChild(total);
    header.appendChild(refreshBtn);

    var progress = document.createElement('progress');
    progress.className = 'debug-prompt-progress';
    progress.id = 'debug-prompt-progress';
    progress.max = 100000;
    progress.value = 0;

    pane.appendChild(header);
    pane.appendChild(progress);

    var body = document.createElement('div');
    body.id = 'debug-prompt-body';

    var empty = document.createElement('div');
    empty.className = 'debug-prompt-empty';
    empty.setAttribute('data-i18n', 'debug.promptEmpty');
    empty.textContent = t('debug.promptEmpty');
    body.appendChild(empty);

    pane.appendChild(body);
  }

  function buildActivityPane(pane) {
    // Turn navigation bar
    var nav = document.createElement('div');
    nav.className = 'debug-turn-nav';
    nav.id = 'debug-turn-nav';
    nav.style.display = 'none';

    var prevBtn = document.createElement('button');
    prevBtn.className = 'debug-turn-prev';
    prevBtn.textContent = '\u25C0';
    prevBtn.title = t('debug.activityPrevTurn');
    prevBtn.addEventListener('click', function () { viewTurn(viewingTurn - 1); });

    var turnLabel = document.createElement('span');
    turnLabel.className = 'debug-turn-label';
    turnLabel.textContent = '';

    var nextBtn = document.createElement('button');
    nextBtn.className = 'debug-turn-next';
    nextBtn.textContent = '\u25B6';
    nextBtn.title = t('debug.activityNextTurn');
    nextBtn.addEventListener('click', function () { viewTurn(viewingTurn + 1); });

    var latestBtn = document.createElement('button');
    latestBtn.className = 'debug-turn-latest';
    latestBtn.textContent = t('debug.activityLatest');
    latestBtn.addEventListener('click', function () { viewTurn(maxTurn()); });

    nav.appendChild(prevBtn);
    nav.appendChild(turnLabel);
    nav.appendChild(nextBtn);
    nav.appendChild(latestBtn);

    pane.appendChild(nav);

    // Activity list
    var list = document.createElement('div');
    list.className = 'debug-activity-list';
    list.id = 'debug-activity-list';

    var empty = document.createElement('div');
    empty.className = 'debug-activity-empty';
    empty.setAttribute('data-i18n', 'debug.activityEmpty');
    empty.textContent = t('debug.activityEmpty');
    list.appendChild(empty);

    pane.appendChild(list);
  }

  function buildStatsPane(pane) {
    pane.id = 'debug-pane-stats';

    var grid = document.createElement('div');
    grid.className = 'debug-stats-grid';
    grid.id = 'debug-stats-grid';

    var cards = [
      { id: 'turns', key: 'debug.statsTurns', value: '0' },
      { id: 'tokens', key: 'debug.statsTotalTokens', value: '0' },
      { id: 'input', key: 'debug.statsInputTokens', value: '0' },
      { id: 'output', key: 'debug.statsOutputTokens', value: '0' },
      // Session cost is summed from per-turn `turn_cost` SSE events.
      { id: 'cost', key: 'debug.statsCost', value: '$0.00' },
      // Daily cost comes from the server (/api/gateway/status) and is
      // a separate metric — keep it in its own card so the two values
      // never overwrite each other on the 30 s poll cycle.
      { id: 'daily-cost', key: 'debug.statsDailyCost', value: '-' },
      { id: 'tools', key: 'debug.statsToolCalls', value: '0' }
    ];

    cards.forEach(function (card) {
      var el = document.createElement('div');
      el.className = 'debug-stat-card';

      var labelEl = document.createElement('div');
      labelEl.className = 'debug-stat-label';
      labelEl.setAttribute('data-i18n', card.key);
      labelEl.textContent = t(card.key);

      var valueEl = document.createElement('div');
      valueEl.className = 'debug-stat-value';
      valueEl.id = 'debug-stat-' + card.id;
      valueEl.textContent = card.value;

      el.appendChild(labelEl);
      el.appendChild(valueEl);
      grid.appendChild(el);
    });

    pane.appendChild(grid);

    // Model usage section
    var modelTitle = document.createElement('div');
    modelTitle.className = 'debug-stats-section-title';
    modelTitle.setAttribute('data-i18n', 'debug.statsModelUsage');
    modelTitle.textContent = t('debug.statsModelUsage');
    pane.appendChild(modelTitle);

    var modelList = document.createElement('div');
    modelList.className = 'debug-model-usage';
    modelList.id = 'debug-model-usage';
    pane.appendChild(modelList);

    // SSE Health
    var sseTitle = document.createElement('div');
    sseTitle.className = 'debug-stats-section-title';
    sseTitle.setAttribute('data-i18n', 'debug.statsSseHealth');
    sseTitle.textContent = t('debug.statsSseHealth');
    pane.appendChild(sseTitle);

    var sseHealth = document.createElement('div');
    sseHealth.className = 'debug-sse-health';
    sseHealth.id = 'debug-sse-health';
    pane.appendChild(sseHealth);

    updateStatsDisplay();
    updateSseHealthDisplay();
  }

  // ── Stats display ──

  function updateStatsDisplay() {
    setStatText('debug-stat-turns', sessionStats.turns);
    setStatText('debug-stat-tokens', formatNumber(sessionStats.inputTokens + sessionStats.outputTokens));
    setStatText('debug-stat-input', formatNumber(sessionStats.inputTokens));
    setStatText('debug-stat-output', formatNumber(sessionStats.outputTokens));
    setStatText('debug-stat-cost', isNaN(sessionStats.cost) ? '-' : '$' + sessionStats.cost.toFixed(4));
    var toolsEl = document.getElementById('debug-stat-tools');
    if (toolsEl) {
      toolsEl.textContent = '';
      toolsEl.appendChild(document.createTextNode(sessionStats.toolCalls + ' ('));
      var successSpan = document.createElement('span');
      successSpan.style.color = 'var(--success)';
      successSpan.textContent = sessionStats.toolSuccess;
      toolsEl.appendChild(successSpan);
      toolsEl.appendChild(document.createTextNode('/'));
      var failSpan = document.createElement('span');
      failSpan.style.color = sessionStats.toolFailure > 0 ? 'var(--danger)' : 'var(--text)';
      failSpan.textContent = sessionStats.toolFailure;
      toolsEl.appendChild(failSpan);
      toolsEl.appendChild(document.createTextNode(')'));
    }
  }

  function updateSseHealthDisplay() {
    var el = document.getElementById('debug-sse-health');
    if (!el) return;
    el.textContent = '';

    var connected = currentEventSource && currentEventSource.readyState === EventSource.OPEN;

    var dot = document.createElement('span');
    dot.className = 'debug-sse-dot ' + (connected ? 'connected' : 'disconnected');

    var info = document.createElement('div');
    info.className = 'debug-sse-info';
    info.textContent = connected ? t('debug.statsSseConnected') : t('debug.statsSseDisconnected');

    var detail = document.createElement('div');
    detail.className = 'debug-sse-detail';
    var parts = [t('debug.statsSseReconnects') + ': ' + sseReconnects];
    parts.push(t('debug.statsSseEvents') + ': ' + totalEventsReceived);
    if (lastEventTime) {
      var ago = Math.round((Date.now() - lastEventTime) / 1000);
      parts.push(t('debug.statsSseLastEvent') + ': ' + ago + 's');
    }
    detail.textContent = parts.join(' \u00B7 ');

    el.appendChild(dot);
    var textWrap = document.createElement('div');
    textWrap.style.flex = '1';
    textWrap.appendChild(info);
    textWrap.appendChild(detail);
    el.appendChild(textWrap);
  }

  // ── Prompt fetching ──

  function fetchPromptData() {
    var refreshBtn = document.querySelector('.debug-prompt-refresh');
    if (refreshBtn) refreshBtn.classList.add('loading');

    apiFetchCompat('/api/debug/prompt')
      .then(function (data) {
        renderPromptData(data);
      })
      .catch(function () {
        var body = document.getElementById('debug-prompt-body');
        if (body) {
          body.textContent = '';
          var err = document.createElement('div');
          err.className = 'debug-prompt-empty';
          err.textContent = t('debug.promptError');
          body.appendChild(err);
        }
      })
      .finally(function () {
        if (refreshBtn) refreshBtn.classList.remove('loading');
      });
  }

  function renderPromptData(data) {
    // Header: model + token progress
    var totalEl = document.getElementById('debug-prompt-total');
    if (totalEl) {
      totalEl.textContent = '';

      // Model name
      if (data.model) {
        var modelSpan = document.createElement('span');
        modelSpan.className = 'debug-prompt-model';
        modelSpan.textContent = data.model;
        totalEl.appendChild(modelSpan);
        totalEl.appendChild(document.createTextNode(' \u00B7 '));
      }

      var tokensUsed = data.total_estimated_tokens || 0;
      var contextLimit = data.context_limit || 100000;
      var strong = document.createElement('strong');
      strong.textContent = formatNumber(tokensUsed) + ' / ' + formatNumber(contextLimit) + ' tokens';
      totalEl.appendChild(strong);
    }

    // Context usage progress bar
    var progressEl = document.getElementById('debug-prompt-progress');
    if (progressEl) {
      var used = data.total_estimated_tokens || 0;
      var limit = data.context_limit || 100000;
      var pct = Math.min(100, Math.round((used / limit) * 100));
      progressEl.value = used;
      progressEl.max = limit;
      progressEl.title = pct + '% (' + formatNumber(used) + ' / ' + formatNumber(limit) + ')';
    }

    var body = document.getElementById('debug-prompt-body');
    if (!body) return;
    body.textContent = '';

    if (!data.components || data.components.length === 0) {
      var empty = document.createElement('div');
      empty.className = 'debug-prompt-empty';
      empty.textContent = t('debug.promptEmpty');
      body.appendChild(empty);
      return;
    }

    // Source filename → i18n key mapping
    var PROMPT_LABEL_KEYS = {
      'AGENTS.md': 'debug.promptLabelAgents',
      'SOUL.md': 'debug.promptLabelSoul',
      'USER.md': 'debug.promptLabelUser',
      'IDENTITY.md': 'debug.promptLabelIdentity',
      'TOOLS.md': 'debug.promptLabelTools',
      'MEMORY.md': 'debug.promptLabelMemory'
    };

    // Component breakdown
    data.components.forEach(function (comp) {
      var details = document.createElement('details');
      details.className = 'debug-prompt-section';

      var summary = document.createElement('summary');

      var labelEl = document.createElement('span');
      var labelKey = PROMPT_LABEL_KEYS[comp.source];
      labelEl.textContent = labelKey ? t(labelKey) : comp.label;
      if (labelKey) labelEl.setAttribute('data-i18n', labelKey);

      var badge = document.createElement('span');
      badge.className = 'debug-prompt-badge';
      badge.textContent = formatNumber(comp.estimated_tokens) + ' tok';

      var source = document.createElement('span');
      source.className = 'debug-prompt-source';
      source.textContent = comp.source;

      summary.appendChild(labelEl);
      summary.appendChild(source);
      summary.appendChild(badge);

      var content = document.createElement('div');
      content.className = 'debug-prompt-content';
      content.textContent = comp.content;

      details.appendChild(summary);
      details.appendChild(content);
      body.appendChild(details);
    });

    // Full assembled system prompt (collapsed by default)
    if (data.system_prompt) {
      var fullDetails = document.createElement('details');
      fullDetails.className = 'debug-prompt-section';

      var fullSummary = document.createElement('summary');
      var fullLabel = document.createElement('span');
      fullLabel.setAttribute('data-i18n', 'debug.promptFull');
      fullLabel.textContent = t('debug.promptFull');
      var fullBadge = document.createElement('span');
      fullBadge.className = 'debug-prompt-badge';
      fullBadge.textContent = formatNumber(data.total_estimated_tokens || 0) + ' tok';
      fullSummary.appendChild(fullLabel);
      fullSummary.appendChild(fullBadge);

      var fullContent = document.createElement('pre');
      fullContent.className = 'debug-prompt-full';
      fullContent.textContent = data.system_prompt;

      fullDetails.appendChild(fullSummary);
      fullDetails.appendChild(fullContent);
      body.appendChild(fullDetails);
    }

    // Note
    if (data.note) {
      var noteEl = document.createElement('div');
      noteEl.className = 'debug-prompt-note';
      noteEl.textContent = data.note;
      body.appendChild(noteEl);
    }
  }

  // ── Gateway stats fetch ──

  function fetchGatewayStats() {
    apiFetchCompat('/api/gateway/status')
      .then(function (data) {
        renderModelUsage(data.model_usage || []);
        // Render the server's daily cost in its own card and leave the
        // session cost card driven solely by the SSE turn_cost
        // accumulator. Mixing the two sources caused the displayed cost
        // to jump back and forth on the 30 s poll cadence.
        var dailyCost = null;
        if (data.daily_cost) {
          var dc = parseFloat(data.daily_cost);
          if (!isNaN(dc)) dailyCost = dc;
        }
        setStatText(
          'debug-stat-daily-cost',
          dailyCost !== null ? '$' + dailyCost.toFixed(4) : '-'
        );
        updateSseHealthDisplay();
      })
      .catch(function () { /* ignore */ });
  }

  function renderModelUsage(models) {
    var el = document.getElementById('debug-model-usage');
    if (!el) return;
    el.textContent = '';

    if (models.length === 0) {
      var empty = document.createElement('div');
      empty.className = 'debug-prompt-empty';
      empty.textContent = t('debug.statsNoModels');
      el.appendChild(empty);
      return;
    }

    models.forEach(function (m) {
      var row = document.createElement('div');
      row.className = 'debug-model-row';

      var name = document.createElement('span');
      name.className = 'debug-model-name';
      name.textContent = m.model;

      var tokens = document.createElement('span');
      tokens.className = 'debug-model-tokens';
      tokens.textContent = formatNumber(m.input_tokens) + ' ' + t('debug.statsIn') + ' / ' + formatNumber(m.output_tokens) + ' ' + t('debug.statsOut');

      if (m.cost) {
        var costEl = document.createElement('span');
        costEl.className = 'debug-model-tokens';
        costEl.textContent = '$' + m.cost;
        row.appendChild(name);
        row.appendChild(tokens);
        row.appendChild(costEl);
      } else {
        row.appendChild(name);
        row.appendChild(tokens);
      }

      el.appendChild(row);
    });
  }

  // ── Helpers ──

  // Delegate to app.js apiFetch which handles auth (token/OIDC)
  function apiFetchCompat(path, options) {
    if (typeof window.apiFetch === 'function') {
      return window.apiFetch(path, options);
    }
    // Fallback: plain fetch (will fail if auth required)
    return fetch(path, options || {}).then(function (r) {
      if (!r.ok) throw new Error(r.status + ' ' + r.statusText);
      return r.json();
    });
  }

  function t(key) {
    return (window.I18n && typeof window.I18n.t === 'function') ? window.I18n.t(key) : key.split('.').pop();
  }

  function setStatText(id, text) {
    var el = document.getElementById(id);
    if (el) el.textContent = text;
  }

  function formatNumber(n) {
    if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
    if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
    return String(n);
  }

  function formatDuration(ms) {
    if (ms < 1000) return ms + 'ms';
    return (ms / 1000).toFixed(1) + 's';
  }

  function timeNow() {
    var d = new Date();
    var h = String(d.getHours()).padStart(2, '0');
    var m = String(d.getMinutes()).padStart(2, '0');
    var s = String(d.getSeconds()).padStart(2, '0');
    return h + ':' + m + ':' + s;
  }

  // ── Public API ──

  /** Re-render dynamic sections that use t() but lack data-i18n attrs. */
  function refreshDynamicI18n() {
    if (!debugActive) return;
    updateStatsDisplay();
    updateSseHealthDisplay();
    fetchGatewayStats();
    rebuildActivityDOM();
    updateTurnNav();
    // Prompt labels with data-i18n are handled by I18n.updatePageContent()
  }

  window.DebugPanel = {
    toggle: togglePanel,
    isActive: function () { return debugActive; },
    getStats: function () { return Object.assign({}, sessionStats); },
    onLanguageChange: refreshDynamicI18n
  };

  // ── Bootstrap ──

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    // app.js runs synchronously before us, so defer one tick to let it finish
    setTimeout(init, 0);
  }
})();
