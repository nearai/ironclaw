var currentSecretEditName = null;

function updateSettingsToolbarForSubtab(subtab) {
  var exportBtn = document.getElementById('settings-export-btn');
  var importBtn = document.getElementById('settings-import-btn');
  var searchWrap = document.querySelector('.settings-search');
  var isSecrets = subtab === 'secrets';
  if (exportBtn) exportBtn.style.display = isSecrets ? 'none' : '';
  if (importBtn) importBtn.style.display = isSecrets ? 'none' : '';
  if (searchWrap) searchWrap.style.display = isSecrets ? 'none' : '';
}

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
  updateSettingsToolbarForSubtab(subtab);
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
  else if (subtab === 'secrets') loadSecretsSettings();
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

// --- Secrets Settings ---

function resetSecretForm() {
  currentSecretEditName = null;
  var nameInput = document.getElementById('secret-name-input');
  var valueInput = document.getElementById('secret-value-input');
  var providerInput = document.getElementById('secret-provider-input');
  var expiryInput = document.getElementById('secret-expiry-input');
  var note = document.getElementById('secret-editing-note');
  var saveBtn = document.getElementById('secret-save-btn');

  if (nameInput) {
    nameInput.disabled = false;
    nameInput.value = '';
  }
  if (valueInput) valueInput.value = '';
  if (providerInput) providerInput.value = '';
  if (expiryInput) expiryInput.value = '';
  if (note) note.style.display = 'none';
  if (saveBtn) saveBtn.textContent = I18n.t('secrets.save');
}

function beginSecretEdit(secret) {
  currentSecretEditName = secret.name;
  var nameInput = document.getElementById('secret-name-input');
  var valueInput = document.getElementById('secret-value-input');
  var providerInput = document.getElementById('secret-provider-input');
  var expiryInput = document.getElementById('secret-expiry-input');
  var note = document.getElementById('secret-editing-note');
  var saveBtn = document.getElementById('secret-save-btn');

  if (nameInput) {
    nameInput.value = secret.name || '';
    nameInput.disabled = true;
  }
  if (valueInput) {
    valueInput.value = '';
    valueInput.focus();
  }
  if (providerInput) providerInput.value = secret.provider || '';
  if (expiryInput) expiryInput.value = '';
  if (note) note.style.display = '';
  if (saveBtn) saveBtn.textContent = I18n.t('secrets.update');
}

function readSecretForm() {
  var name = currentSecretEditName || (document.getElementById('secret-name-input') || {}).value || '';
  var value = (document.getElementById('secret-value-input') || {}).value || '';
  var provider = (document.getElementById('secret-provider-input') || {}).value || '';
  var expiryRaw = (document.getElementById('secret-expiry-input') || {}).value || '';
  name = name.trim().toLowerCase();
  provider = provider.trim();
  expiryRaw = expiryRaw.trim();

  if (!name) throw new Error(I18n.t('secrets.nameRequired'));
  if (!value.trim()) throw new Error(I18n.t('secrets.valueRequired'));

  var payload = { value: value };
  if (provider) payload.provider = provider;
  if (expiryRaw) {
    var parsed = Number(expiryRaw);
    if (!Number.isFinite(parsed) || parsed <= 0) {
      throw new Error(I18n.t('secrets.expiryInvalid'));
    }
    payload.expires_in_days = Math.floor(parsed);
  }

  return { name: name, body: payload };
}

function formatSecretApprovalLocation(location) {
  if (!location || typeof location !== 'object') return '—';
  if (location.AuthorizationBearer !== undefined) return 'bearer';
  if (location.AuthorizationBasic) return 'basic_auth';
  if (location.Header) return 'header:' + (location.Header.name || '');
  if (location.QueryParam) return 'query_param:' + (location.QueryParam.name || '');
  if (location.UrlPath) return 'url_path:' + (location.UrlPath.placeholder || '');
  return '—';
}

function revokeSecretApproval(secretName, approval) {
  showConfirmModal(
    I18n.t('secrets.confirmRevokeTitle', { name: secretName }),
    I18n.t('secrets.confirmRevokeMessage', {
      artifact: approval.artifact_name || 'artifact',
      host: approval.host || 'host'
    }),
    function() {
      apiFetch('/api/secrets/' + encodeURIComponent(secretName) + '/approvals/revoke', {
        method: 'POST',
        body: { approval_id: approval.approval_id }
      }).then(function() {
        showToast(I18n.t('secrets.revokeSuccess'), 'success');
        loadSecretsSettings();
      }).catch(function(err) {
        showToast(I18n.t('secrets.revokeFailed', { message: err.message }), 'error');
      });
    },
    I18n.t('secrets.revoke')
  );
}

function appendSecretApprovalsRow(tbody, secret) {
  var approvals = Array.isArray(secret.approvals) ? secret.approvals : [];
  if (approvals.length === 0) return;

  var detailsTr = document.createElement('tr');
  var detailsTd = document.createElement('td');
  detailsTd.colSpan = 6;
  detailsTd.style.paddingTop = '0';

  var wrap = document.createElement('div');
  wrap.className = 'settings-description';

  var heading = document.createElement('div');
  heading.style.fontWeight = '600';
  heading.style.marginBottom = '8px';
  heading.textContent = I18n.t('secrets.approvalsHeading');
  wrap.appendChild(heading);

  approvals.forEach(function(approval) {
    var row = document.createElement('div');
    row.style.display = 'flex';
    row.style.alignItems = 'center';
    row.style.justifyContent = 'space-between';
    row.style.gap = '12px';
    row.style.padding = '8px 0';
    row.style.borderTop = '1px solid var(--border-color, rgba(255,255,255,0.08))';

    var info = document.createElement('div');
    info.textContent = I18n.t('secrets.approvalDetails', {
      kind: approval.artifact_kind || 'artifact',
      artifact: approval.artifact_name || 'artifact',
      host: approval.host || 'host',
      location: formatSecretApprovalLocation(approval.location),
      approvedAt: approval.approved_at ? formatDate(approval.approved_at) : '—',
      risk: approval.risk === 'high' ? I18n.t('secrets.riskHigh') : I18n.t('secrets.riskNormal')
    });
    info.style.flex = '1';
    info.style.minWidth = '0';
    wrap.appendChild(row);
    row.appendChild(info);

    var revokeBtn = document.createElement('button');
    revokeBtn.className = 'btn-secondary';
    revokeBtn.textContent = I18n.t('secrets.revoke');
    revokeBtn.addEventListener('click', function() {
      revokeSecretApproval(secret.name, approval);
    });
    row.appendChild(revokeBtn);
  });

  detailsTd.appendChild(wrap);
  detailsTr.appendChild(detailsTd);
  tbody.appendChild(detailsTr);
}

function renderSecretsTable(secrets) {
  var tbody = document.getElementById('secrets-tbody');
  var empty = document.getElementById('secrets-empty');
  if (!tbody || !empty) return;

  tbody.innerHTML = '';
  if (!secrets || secrets.length === 0) {
    empty.style.display = '';
    return;
  }

  empty.style.display = 'none';
  secrets.forEach(function(secret) {
    var tr = document.createElement('tr');

    var nameTd = document.createElement('td');
    nameTd.textContent = secret.name || '—';
    tr.appendChild(nameTd);

    var providerTd = document.createElement('td');
    providerTd.textContent = secret.provider || '—';
    tr.appendChild(providerTd);

    var updatedTd = document.createElement('td');
    updatedTd.textContent = secret.updated_at ? formatDate(secret.updated_at) : '—';
    tr.appendChild(updatedTd);

    var expiresTd = document.createElement('td');
    expiresTd.textContent = secret.expires_at ? formatDate(secret.expires_at) : '—';
    tr.appendChild(expiresTd);

    var approvalsTd = document.createElement('td');
    var approvalCount = Array.isArray(secret.approvals) ? secret.approvals.length : 0;
    approvalsTd.textContent = approvalCount === 0
      ? I18n.t('secrets.noApprovals')
      : I18n.t('secrets.approvalCount', { count: approvalCount });
    tr.appendChild(approvalsTd);

    var actionsTd = document.createElement('td');
    var editBtn = document.createElement('button');
    editBtn.className = 'btn-secondary';
    editBtn.textContent = I18n.t('secrets.edit');
    editBtn.addEventListener('click', function() { beginSecretEdit(secret); });
    actionsTd.appendChild(editBtn);

    var deleteBtn = document.createElement('button');
    deleteBtn.className = 'btn-danger';
    deleteBtn.style.marginLeft = '8px';
    deleteBtn.textContent = I18n.t('secrets.delete');
    deleteBtn.addEventListener('click', function() { deleteSecret(secret.name); });
    actionsTd.appendChild(deleteBtn);

    tr.appendChild(actionsTd);
    tbody.appendChild(tr);
    appendSecretApprovalsRow(tbody, secret);
  });
}

function loadSecretsSettings() {
  var tbody = document.getElementById('secrets-tbody');
  var empty = document.getElementById('secrets-empty');
  if (tbody) {
    tbody.innerHTML = '<tr><td colspan="6" class="empty-state">' + escapeHtml(I18n.t('common.loading')) + '</td></tr>';
  }
  if (empty) empty.style.display = 'none';

  apiFetch('/api/secrets').then(function(data) {
    renderSecretsTable(data.secrets || []);
  }).catch(function(err) {
    if (tbody) {
      tbody.innerHTML = '<tr><td colspan="6" class="empty-state">' + escapeHtml(I18n.t('common.loadFailed')) + ': ' + escapeHtml(err.message) + '</td></tr>';
    }
  });
}

function saveSecretFromForm() {
  var parsed;
  try {
    parsed = readSecretForm();
  } catch (err) {
    showToast(err.message, 'error');
    return;
  }

  apiFetch('/api/secrets/' + encodeURIComponent(parsed.name), {
    method: 'PUT',
    body: parsed.body,
  }).then(function(res) {
    showToast(I18n.t('secrets.saveSuccess', { name: res.name || parsed.name }), 'success');
    resetSecretForm();
    loadSecretsSettings();
  }).catch(function(err) {
    showToast(I18n.t('secrets.saveFailed', { message: err.message }), 'error');
  });
}

function deleteSecret(name) {
  showConfirmModal(
    I18n.t('secrets.confirmDeleteTitle', { name: name }),
    I18n.t('secrets.confirmDeleteMessage', { name: name }),
    function() {
      apiFetch('/api/secrets/' + encodeURIComponent(name), {
        method: 'DELETE',
      }).then(function() {
        if (currentSecretEditName === name) resetSecretForm();
        showToast(I18n.t('secrets.deleteSuccess', { name: name }), 'success');
        loadSecretsSettings();
      }).catch(function(err) {
        showToast(I18n.t('secrets.deleteFailed', { message: err.message }), 'error');
      });
    },
    I18n.t('secrets.delete')
  );
}

function importSecrets() {
  var input = document.createElement('input');
  input.type = 'file';
  input.accept = '.json,application/json';
  input.addEventListener('change', function() {
    if (!input.files || !input.files[0]) return;
    var reader = new FileReader();
    reader.onload = function() {
      try {
        var parsed = JSON.parse(reader.result);
        if (Array.isArray(parsed)) parsed = { secrets: parsed };
        if (!parsed || !Array.isArray(parsed.secrets)) {
          throw new Error(I18n.t('secrets.importFormatError'));
        }
        apiFetch('/api/secrets/import', {
          method: 'POST',
          body: parsed,
        }).then(function() {
          showToast(I18n.t('secrets.importSuccess'), 'success');
          loadSecretsSettings();
        }).catch(function(err) {
          showToast(I18n.t('secrets.importFailed', { message: err.message }), 'error');
        });
      } catch (err) {
        showToast(I18n.t('secrets.importFailed', { message: err.message }), 'error');
      }
    };
    reader.readAsText(input.files[0]);
  });
  input.click();
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

