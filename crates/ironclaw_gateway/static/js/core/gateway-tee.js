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

    // Hide ClawHub search section when registry is disabled
    var skillSearchSection = document.querySelector('.skill-search-section');
    if (skillSearchSection) {
      skillSearchSection.style.display = data.clawhub_enabled === false ? 'none' : '';
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

