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
    activeWorkStore.setEngineV2Enabled(!!data.engine_v2);
    applyEngineModeUi();
    refreshPersistentActivityBar();

    // Update restart button visibility
    restartEnabled = data.restart_enabled || false;
    updateRestartButtonVisibility();

    // Apply engine v2 / v1 tab visibility once.
    if (!engineModeApplied) {
      engineV2Enabled = !!data.engine_v2_enabled;
      applyEngineModeToTabs();
      engineModeApplied = true;
    }

    var popover = document.getElementById('gateway-popover');
    var html = '';

    // Version — show commit hash when not a tagged release
    if (data.version) {
      var versionText = 'IronClaw v' + escapeHtml(data.version);
      if (data.commit_hash) {
        versionText += ' (' + escapeHtml(data.commit_hash) + ')';
      }
      html += '<div class="gw-section-label">' + versionText + '</div>';
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
