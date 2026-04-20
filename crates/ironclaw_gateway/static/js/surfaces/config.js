/** Sentinel value meaning "key is unchanged, don't touch it". Must match backend. */
const API_KEY_UNCHANGED = '\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022';

const ADAPTER_LABELS = {
  open_ai_completions: 'OpenAI Compatible',
  anthropic: 'Anthropic',
  ollama: 'Ollama',
  bedrock: 'AWS Bedrock',
  nearai: 'NEAR AI',
};

let _builtinProviders = [];
let _customProviders = [];
let _activeLlmBackend = '';
let _selectedModel = '';
let _builtinOverrides = {};
let _editingProviderId = null;
let _configuringBuiltinId = null;
let _configLoaded = false;

function loadConfig() {
  const list = document.getElementById('providers-list');
  list.innerHTML = '<div class="empty-state">' + I18n.t('common.loading') + '</div>';

  Promise.all([
    apiFetch('/api/settings/export'),
    apiFetch('/api/llm/providers').catch(function() { return []; }),
  ]).then(function(results) {
    const s = (results[0] && results[0].settings) ? results[0].settings : {};
    _builtinProviders = Array.isArray(results[1]) ? results[1] : [];
    _activeLlmBackend = s['llm_backend'] ? String(s['llm_backend']) : 'nearai';
    _selectedModel = s['selected_model'] ? String(s['selected_model']) : '';
    try {
      const val = s['llm_custom_providers'];
      _customProviders = Array.isArray(val) ? val : (val ? JSON.parse(val) : []);
    } catch (e) {
      _customProviders = [];
    }
    try {
      const val = s['llm_builtin_overrides'];
      _builtinOverrides = (val && typeof val === 'object' && !Array.isArray(val)) ? val : {};
    } catch (e) {
      _builtinOverrides = {};
    }
    _configLoaded = true;
    renderProviders();
  }).catch(function() {
    _activeLlmBackend = 'nearai';
    _selectedModel = '';
    _builtinProviders = [];
    _customProviders = [];
    _builtinOverrides = {};
    _configLoaded = true;
    renderProviders();
  });
}

function scrollToProviders() {
  const section = document.getElementById('providers-section');
  if (section) section.scrollIntoView({ behavior: 'smooth', block: 'start' });
}

function renderProviders() {
  const list = document.getElementById('providers-list');
  const allProviders = [..._builtinProviders, ..._customProviders].sort((a, b) => {
    if (a.id === _activeLlmBackend) return -1;
    if (b.id === _activeLlmBackend) return 1;
    return 0;
  });

  if (allProviders.length === 0) {
    list.innerHTML = '<div class="empty-state">No providers</div>';
    return;
  }

  list.innerHTML = allProviders.map((p) => {
    const isActive = p.id === _activeLlmBackend;
    const adapterLabel = ADAPTER_LABELS[p.adapter] || p.adapter;
    const activeBadge = isActive
      ? '<span class="provider-badge provider-badge-active">' + I18n.t('status.active') + '</span>'
      : '';
    const builtinBadge = p.builtin
      ? '<span class="provider-badge provider-badge-builtin">' + I18n.t('config.builtin') + '</span>'
      : '';
    const deleteBtn = !p.builtin && !isActive
      ? '<button class="provider-action-btn provider-delete-btn" data-action="delete-custom-provider" data-id="' + escapeHtml(p.id) + '">' + I18n.t('common.delete') + '</button>'
      : '';
    const editBtn = !p.builtin
      ? '<button class="provider-action-btn" data-action="edit-custom-provider" data-id="' + escapeHtml(p.id) + '">' + I18n.t('common.edit') + '</button>'
      : '';
    // Show Configure for built-in providers that support it (not bedrock — uses AWS credential chain)
    const configureBtn = p.builtin && p.id !== 'bedrock'
      ? '<button class="provider-action-btn" data-action="configure-builtin-provider" data-id="' + escapeHtml(p.id) + '">' + I18n.t('config.configureProvider') + '</button>'
      : '';
    const useBtn = !isActive
      ? '<button class="provider-action-btn" data-action="set-active-provider" data-id="' + escapeHtml(p.id) + '">' + I18n.t('config.useProvider') + '</button>'
      : '';
    const overrideBaseUrl = p.builtin && _builtinOverrides[p.id] ? (_builtinOverrides[p.id].base_url || '') : '';
    const effectiveBaseUrl = overrideBaseUrl || p.env_base_url || p.base_url;
    const baseUrlText = effectiveBaseUrl
      ? '<span class="provider-url">' + escapeHtml(effectiveBaseUrl) + '</span>'
      : '';
    // Show configured model: for active provider use _selectedModel, for others check _builtinOverrides then env defaults
    const overrideModel = p.builtin && _builtinOverrides[p.id] ? (_builtinOverrides[p.id].model || '') : '';
    const displayModel = isActive
      ? (_selectedModel || p.env_model || '')
      : (overrideModel || p.env_model || '');
    const modelText = displayModel
      ? '<span class="provider-current-model">' + escapeHtml(I18n.t('config.currentModel', { model: displayModel })) + '</span>'
      : '';

    return '<div class="provider-card' + (isActive ? ' provider-card-active' : '') + '">'
      + '<div class="provider-card-header">'
      +   '<span class="provider-name">' + escapeHtml(p.name || p.id) + '</span>'
      +   '<span class="provider-id-label">' + escapeHtml(p.id) + '</span>'
      +   activeBadge + builtinBadge
      + '</div>'
      + '<div class="provider-card-meta">'
      +   '<span class="provider-adapter">' + escapeHtml(adapterLabel) + '</span>'
      +   baseUrlText
      +   modelText
      + '</div>'
      + '<div class="provider-card-actions">'
      +   useBtn + configureBtn + editBtn + deleteBtn
      + '</div>'
      + '</div>';
  }).join('');
}

function setActiveProvider(id) {
  const provider = [..._builtinProviders, ..._customProviders].find((p) => p.id === id);
  // Restore the last-configured model for this provider, falling back to the provider's default
  const restoredModel =
    (_builtinOverrides[id] && _builtinOverrides[id].model) ||
    (provider && provider.default_model) ||
    null;
  const defaultModel = restoredModel;
  const modelUpdate = () => defaultModel
    ? apiFetchVoid('/api/settings/selected_model', { method: 'PUT', body: { value: defaultModel } })
    : apiFetchVoid('/api/settings/selected_model', { method: 'DELETE' });
  apiFetchVoid('/api/settings/llm_backend', { method: 'PUT', body: { value: id } })
    .then(() => modelUpdate())
    .then(() => {
      _activeLlmBackend = id;
      _selectedModel = defaultModel || '';
      renderProviders();
      loadInferenceSettings();
      scrollToProviders();
      document.getElementById('config-restart-notice').style.display = 'flex';
      var llmNotice = document.getElementById('llm-restart-notice');
      if (llmNotice) llmNotice.style.display = 'flex';
      showToast(I18n.t('config.providerActivated', { name: id }));
    })
    .catch((e) => showToast(I18n.t('error.unknown') + ': ' + e.message, 'error'));
}

function deleteCustomProvider(id) {
  if (id === _activeLlmBackend) {
    showToast(I18n.t('config.cannotDeleteActiveProvider'), 'error');
    return;
  }
  if (!confirm(I18n.t('config.confirmDeleteProvider', { id }))) return;
  const originalProviders = _customProviders;
  _customProviders = _customProviders.filter((p) => p.id !== id);
  saveCustomProviders().then(() => {
    renderProviders();
    showToast(I18n.t('config.providerDeleted'));
  }).catch((e) => {
    _customProviders = originalProviders;
    showToast(I18n.t('error.unknown') + ': ' + e.message, 'error');
  });
}

function saveCustomProviders() {
  return apiFetchVoid('/api/settings/llm_custom_providers', { method: 'PUT', body: { value: _customProviders } });
}

function editCustomProvider(id) {
  const p = _customProviders.find((p) => p.id === id);
  if (!p) return;
  _editingProviderId = id;
  const titleEl = document.getElementById('provider-form-title');
  titleEl.textContent = I18n.t('config.editProvider');
  titleEl.removeAttribute('data-i18n');
  document.getElementById('provider-name').value = p.name || '';
  const idField = document.getElementById('provider-id');
  idField.value = p.id;
  idField.readOnly = true;
  idField.style.opacity = '0.6';
  document.getElementById('provider-adapter').value = p.adapter || 'open_ai_completions';
  document.getElementById('provider-base-url').value = p.base_url || '';
  const editApiKeyInput = document.getElementById('provider-api-key');
  if (p.api_key === API_KEY_UNCHANGED) {
    editApiKeyInput.value = '';
    editApiKeyInput.placeholder = I18n.t('config.apiKeyConfigured');
  } else {
    editApiKeyInput.value = '';
    editApiKeyInput.placeholder = I18n.t('config.apiKeyEnter');
  }
  document.getElementById('provider-model').value = p.default_model || '';
  openProviderDialog(true);
  document.getElementById('provider-name').focus();
}

function configureBuiltinProvider(id) {
  const p = _builtinProviders.find((p) => p.id === id);
  if (!p) return;
  _configuringBuiltinId = id;
  const titleEl = document.getElementById('provider-form-title');
  titleEl.textContent = I18n.t('config.configureProvider') + ': ' + (p.name || id);
  titleEl.removeAttribute('data-i18n');
  // Hide name/id/adapter rows; show base-url as editable
  document.getElementById('provider-name-row').style.display = 'none';
  document.getElementById('provider-id-row').style.display = 'none';
  document.getElementById('provider-adapter-row').style.display = 'none';
  const baseUrlInput = document.getElementById('provider-base-url');
  const override = _builtinOverrides[id] || {};
  // Priority: db override > env > hardcoded default
  const effectiveBaseUrl = override.base_url || p.env_base_url || p.base_url;
  document.getElementById('provider-base-url-row').style.display = '';
  baseUrlInput.value = effectiveBaseUrl || '';
  baseUrlInput.readOnly = false;
  baseUrlInput.style.opacity = '';
  baseUrlInput.placeholder = p.base_url || '';
  document.getElementById('provider-api-key-row').style.display = p.api_key_required !== false ? '' : 'none';
  document.getElementById('fetch-models-btn').style.display = p.can_list_models ? '' : 'none';
  const apiKeyInput = document.getElementById('provider-api-key');
  const hasDbKey = override.api_key === API_KEY_UNCHANGED;
  const hasEnvKey = p.has_api_key === true;
  apiKeyInput.value = '';
  if (hasDbKey) {
    apiKeyInput.placeholder = I18n.t('config.apiKeyConfigured');
  } else if (hasEnvKey) {
    apiKeyInput.placeholder = I18n.t('config.apiKeyFromEnv');
  } else {
    apiKeyInput.placeholder = I18n.t('config.apiKeyEnter');
  }
  document.getElementById('provider-model').value = override.model || p.env_model || p.default_model || '';
  openProviderDialog(true);
  document.getElementById('provider-model').focus();
}

// Add provider form

document.getElementById('add-provider-btn').addEventListener('click', () => {
  openProviderDialog(false);
});

document.getElementById('cancel-provider-btn').addEventListener('click', () => {
  resetProviderForm();
});

document.getElementById('cancel-provider-footer-btn').addEventListener('click', () => {
  resetProviderForm();
});

document.getElementById('provider-dialog-overlay').addEventListener('click', () => {
  resetProviderForm();
});

function openProviderDialog(isEdit) {
  if (!isEdit) {
    // Add mode: ensure all rows visible
    ['provider-name-row', 'provider-id-row', 'provider-adapter-row',
     'provider-base-url-row', 'provider-api-key-row'].forEach((id) => {
      document.getElementById(id).style.display = '';
    });
    document.getElementById('fetch-models-btn').style.display = '';
  }
  document.getElementById('provider-dialog').style.display = 'flex';
  if (!isEdit) {
    document.getElementById('provider-name').focus();
  }
}

document.getElementById('test-provider-btn').addEventListener('click', () => {
  let adapter = document.getElementById('provider-adapter').value;
  let baseUrl = document.getElementById('provider-base-url').value.trim();
  const apiKey = document.getElementById('provider-api-key').value.trim();
  const model = document.getElementById('provider-model').value.trim();

  // For built-in providers, use the adapter from the registry.
  // base_url comes from the form which already reflects: env > hardcoded default.
  if (_configuringBuiltinId) {
    const p = _builtinProviders.find((x) => x.id === _configuringBuiltinId);
    if (p) {
      adapter = p.adapter;
      if (!baseUrl) baseUrl = p.base_url;
    }
  }

  const btn = document.getElementById('test-provider-btn');
  const result = document.getElementById('test-connection-result');

  btn.disabled = true;
  btn.textContent = I18n.t('config.testing');
  result.style.display = 'none';
  result.className = 'test-connection-result';

  // Resolve provider_id so the backend can look up vaulted API keys.
  const providerId = _configuringBuiltinId || document.getElementById('provider-id').value.trim();

  if (!model) {
    result.textContent = I18n.t('config.modelRequired') || 'Model is required for connection test';
    result.className = 'test-connection-result test-fail';
    result.style.display = '';
    btn.disabled = false;
    btn.textContent = I18n.t('config.testConnection');
    return;
  }

  apiFetch('/api/llm/test_connection', {
    method: 'POST',
    body: {
      adapter, base_url: baseUrl,
      api_key: apiKey || undefined,
      model,
      provider_id: providerId || undefined,
      provider_type: _configuringBuiltinId ? 'builtin' : 'custom',
    },
  })
    .then((data) => {
      result.textContent = data.message;
      result.className = 'test-connection-result ' + (data.ok ? 'test-ok' : 'test-fail');
      result.style.display = '';
    })
    .catch((e) => {
      result.textContent = e.message;
      result.className = 'test-connection-result test-fail';
      result.style.display = '';
    })
    .finally(() => {
      btn.disabled = false;
      btn.textContent = I18n.t('config.testConnection');
    });
});

document.getElementById('save-provider-btn').addEventListener('click', () => {
  // Built-in configure mode: save api_key + model to llm_builtin_overrides
  if (_configuringBuiltinId) {
    const apiKey = document.getElementById('provider-api-key').value.trim();
    const model = document.getElementById('provider-model').value.trim();
    const baseUrl = document.getElementById('provider-base-url').value.trim();
    const id = _configuringBuiltinId;
    const prevOverride = _builtinOverrides[id] || {};
    const hadKey = prevOverride.api_key === API_KEY_UNCHANGED;
    const override = {};
    if (apiKey) {
      override.api_key = apiKey;  // New key entered — backend will encrypt it
    } else if (hadKey) {
      override.api_key = API_KEY_UNCHANGED;  // Sentinel: keep existing encrypted key
    }
    // If neither — key is cleared (no key configured)
    if (model) override.model = model;
    if (baseUrl) override.base_url = baseUrl;
    const prev = _builtinOverrides[id];
    _builtinOverrides[id] = override;
    const isActive = id === _activeLlmBackend;
    const modelUpdate = () => {
      if (!isActive) return Promise.resolve();
      if (model) {
        return apiFetchVoid('/api/settings/selected_model', { method: 'PUT', body: { value: model } });
      }
      return apiFetchVoid('/api/settings/selected_model', { method: 'DELETE' });
    };
    apiFetchVoid('/api/settings/llm_builtin_overrides', { method: 'PUT', body: { value: _builtinOverrides } })
      .then(() => modelUpdate())
      .then(() => {
        if (isActive) _selectedModel = model;
        renderProviders();
        if (isActive) loadInferenceSettings();
        resetProviderForm();
        scrollToProviders();
        if (isActive) {
          document.getElementById('config-restart-notice').style.display = 'flex';
          var llmNotice = document.getElementById('llm-restart-notice');
          if (llmNotice) llmNotice.style.display = 'flex';
        }
        showToast(I18n.t('config.providerConfigured', { name: id }));
      })
      .catch((e) => {
        if (prev !== undefined) { _builtinOverrides[id] = prev; } else { delete _builtinOverrides[id]; }
        showToast(I18n.t('error.unknown') + ': ' + e.message, 'error');
      });
    return;
  }

  const name = document.getElementById('provider-name').value.trim();
  const id = document.getElementById('provider-id').value.trim();
  const adapter = document.getElementById('provider-adapter').value;
  const baseUrl = document.getElementById('provider-base-url').value.trim();
  const apiKey = document.getElementById('provider-api-key').value.trim();
  const model = document.getElementById('provider-model').value.trim();

  if (!id || !name) {
    showToast(I18n.t('config.providerFieldsRequired'), 'error');
    return;
  }

  if (_editingProviderId) {
    // Update existing provider
    const idx = _customProviders.findIndex((p) => p.id === _editingProviderId);
    if (idx === -1) return;
    const original = _customProviders[idx];
    const hadCustomKey = original.api_key === API_KEY_UNCHANGED;
    let effectiveApiKey;
    if (apiKey) {
      effectiveApiKey = apiKey;  // New key — backend will encrypt it
    } else if (hadCustomKey) {
      effectiveApiKey = API_KEY_UNCHANGED;  // Sentinel: keep existing encrypted key
    } else {
      effectiveApiKey = undefined;  // No key
    }
    _customProviders[idx] = { ...original, name, adapter, base_url: baseUrl, default_model: model || undefined, api_key: effectiveApiKey };
    const isActive = _editingProviderId === _activeLlmBackend;
    const modelUpdate = () => {
      if (!isActive) return Promise.resolve();
      if (model) {
        return apiFetchVoid('/api/settings/selected_model', { method: 'PUT', body: { value: model } });
      }
      return apiFetchVoid('/api/settings/selected_model', { method: 'DELETE' });
    };
    saveCustomProviders().then(() => modelUpdate()).then(() => {
      if (isActive) _selectedModel = model;
      renderProviders();
      if (isActive) loadInferenceSettings();
      resetProviderForm();
      scrollToProviders();
      if (isActive) {
        document.getElementById('config-restart-notice').style.display = 'flex';
        var llmNotice = document.getElementById('llm-restart-notice');
        if (llmNotice) llmNotice.style.display = 'flex';
      }
      showToast(I18n.t('config.providerUpdated', { name }));
    }).catch((e) => {
      _customProviders[idx] = original;
      showToast(I18n.t('error.unknown') + ': ' + e.message, 'error');
    });
    return;
  }

  if (!/^[a-z0-9_-]+$/.test(id)) {
    showToast(I18n.t('config.providerIdInvalid'), 'error');
    return;
  }
  const allIds = [..._builtinProviders.map((p) => p.id), ..._customProviders.map((p) => p.id)];
  if (allIds.includes(id)) {
    showToast(I18n.t('config.providerIdTaken', { id }), 'error');
    return;
  }

  const newProvider = { id, name, adapter, base_url: baseUrl, default_model: model, api_key: apiKey || undefined, builtin: false };
  _customProviders.push(newProvider);

  saveCustomProviders().then(() => {
    renderProviders();
    resetProviderForm();
    scrollToProviders();
    showToast(I18n.t('config.providerAdded', { name }));
  }).catch((e) => {
    _customProviders.pop();
    showToast(I18n.t('error.unknown') + ': ' + e.message, 'error');
  });
});

function resetProviderForm() {
  _editingProviderId = null;
  _configuringBuiltinId = null;
  document.getElementById('provider-dialog').style.display = 'none';
  // Restore all hidden rows and buttons
  ['provider-name-row', 'provider-id-row', 'provider-adapter-row',
   'provider-base-url-row', 'provider-api-key-row'].forEach((id) => {
    document.getElementById(id).style.display = '';
  });
  document.getElementById('fetch-models-btn').style.display = '';
  const titleEl = document.getElementById('provider-form-title');
  titleEl.setAttribute('data-i18n', 'config.newProvider');
  titleEl.textContent = I18n.t('config.newProvider');
  const idField = document.getElementById('provider-id');
  idField.readOnly = false;
  idField.style.opacity = '';
  delete idField.dataset.edited;
  const baseUrlField = document.getElementById('provider-base-url');
  baseUrlField.readOnly = false;
  baseUrlField.style.opacity = '';
  ['provider-name', 'provider-id', 'provider-base-url', 'provider-api-key', 'provider-model'].forEach((id) => {
    document.getElementById(id).value = '';
  });
  document.getElementById('provider-adapter').selectedIndex = 0;
  const sel = document.getElementById('provider-model-select');
  sel.innerHTML = '';
  sel.style.display = 'none';
  document.getElementById('test-connection-result').style.display = 'none';
}

document.getElementById('provider-model-select').addEventListener('change', (e) => {
  document.getElementById('provider-model').value = e.target.value;
});

document.getElementById('fetch-models-btn').addEventListener('click', () => {
  let adapter = document.getElementById('provider-adapter').value;
  let baseUrl = document.getElementById('provider-base-url').value.trim();
  const apiKey = document.getElementById('provider-api-key').value.trim();

  // For built-in providers, use the adapter from the registry.
  // base_url comes from the form which already reflects: env > hardcoded default.
  if (_configuringBuiltinId) {
    const p = _builtinProviders.find((x) => x.id === _configuringBuiltinId);
    if (p) {
      adapter = p.adapter;
      if (!baseUrl) baseUrl = p.base_url;
    }
  }

  if (!baseUrl) {
    showToast(I18n.t('config.providerBaseUrlRequired'), 'error');
    return;
  }

  const btn = document.getElementById('fetch-models-btn');
  btn.disabled = true;
  btn.textContent = I18n.t('config.fetchingModels');

  // Resolve provider_id so the backend can look up vaulted API keys.
  const providerId = _configuringBuiltinId || document.getElementById('provider-id').value.trim();

  apiFetch('/api/llm/list_models', {
    method: 'POST',
    body: {
      adapter, base_url: baseUrl,
      api_key: apiKey || undefined,
      provider_id: providerId || undefined,
      provider_type: _configuringBuiltinId ? 'builtin' : 'custom',
    },
  })
    .then((data) => {
      const select = document.getElementById('provider-model-select');
      if (data.ok && data.models && data.models.length > 0) {
        const currentModel = document.getElementById('provider-model').value;
        select.innerHTML = data.models
          .map((m) => `<option value="${escapeHtml(m)}"${m === currentModel ? ' selected' : ''}>${escapeHtml(m)}</option>`)
          .join('');
        select.style.display = '';
        btn.style.display = 'none';
        showToast(I18n.t('config.modelsFetched', { count: data.models.length }));
      } else {
        showToast(data.message || I18n.t('config.modelsFetchFailed'), 'error');
      }
    })
    .catch((e) => showToast(e.message, 'error'))
    .finally(() => {
      btn.disabled = false;
      btn.textContent = I18n.t('config.fetchModels');
    });
});

// Auto-fill provider ID from name
document.getElementById('provider-name').addEventListener('input', (e) => {
  const idField = document.getElementById('provider-id');
  if (!idField.dataset.edited) {
    idField.value = e.target.value.toLowerCase().replace(/[^a-z0-9_]+/g, '-').replace(/^-|-$/g, '');
  }
});

document.getElementById('provider-id').addEventListener('input', (e) => {
  e.target.dataset.edited = e.target.value ? '1' : '';
});

// ==================== Widget Extension System ====================
//
// Provides a registration API for frontend widgets. Widgets are self-contained
// components that plug into named slots in the UI (tabs, sidebar, status bar, etc.).
//
// Widget authors call IronClaw.registerWidget({ id, name, slot, init, ... })
// from their module script. The init() function receives a container DOM element
// and the IronClaw.api object for authenticated fetch, event subscription, etc.

// Define `window.IronClaw` as a non-writable, non-configurable property
// rather than `window.IronClaw = window.IronClaw || {}`. The `|| {}` form
// would honor any pre-existing value on `window.IronClaw`, which in
// principle could be set by an inline script that ran before app.js — a
// hostile pre-init could install a fake `registerWidget` trap and
// intercept every widget registration. In practice the gateway HTML
// loads app.js before any deferred `type="module"` widget script and
// has no inline scripts that touch `window.IronClaw`, so this is
// defense-in-depth against future template changes (or a stray browser
// extension), not a fix for an exploitable bug. Using
// `Object.defineProperty` with `writable: false` / `configurable: false`
// also locks the binding so a hostile widget can't replace the entire
// `IronClaw` object after the fact — its only path is to mutate properties
// on the fixed object, which is the same authority every other widget has.
Object.defineProperty(window, 'IronClaw', {
  value: {},
  writable: false,
  configurable: false,
  enumerable: true,
});
IronClaw.widgets = new Map();
IronClaw._widgetInitQueue = [];
IronClaw._chatRenderers = [];

/**
 * Register a widget component.
 * @param {Object} def - Widget definition
 * @param {string} def.id - Unique widget identifier
 * @param {string} def.name - Display name
 * @param {string} def.slot - Target slot ('tab', 'chat_header', etc.)
 * @param {string} [def.icon] - Icon identifier
 * @param {Function} def.init - Called with (container, api) when widget activates
 * @param {Function} [def.activate] - Called when widget becomes visible
 * @param {Function} [def.deactivate] - Called when widget is hidden
 * @param {Function} [def.destroy] - Called when widget is removed
 */
IronClaw.registerWidget = function(def) {
  if (!def.id || !def.init) {
    console.error('[IronClaw] Widget registration requires id and init:', def);
    return;
  }
  IronClaw.widgets.set(def.id, def);

  if (def.slot === 'tab') {
    _addWidgetTab(def);
  }
};

/**
 * Register a chat renderer for custom inline rendering of structured data.
 *
 * Chat renderers run against each assistant message. The first renderer
 * whose `match()` returns true gets to transform the content.
 *
 * @param {Object} def - Renderer definition
 * @param {string} def.id - Unique identifier
 * @param {Function} def.match - (textContent, element) => boolean
 * @param {Function} def.render - (element, textContent) => void (mutate element in place)
 * @param {number} [def.priority=0] - Higher priority runs first
 */
IronClaw.registerChatRenderer = function(def) {
  if (!def.id || !def.match || !def.render) {
    console.error('[IronClaw] Chat renderer requires id, match, and render:', def);
    return;
  }
  IronClaw._chatRenderers.push(def);
  // Sort by priority (higher first)
  IronClaw._chatRenderers.sort(function(a, b) {
    return (b.priority || 0) - (a.priority || 0);
  });
};

/**
 * API object exposed to widgets for safe interaction with the app.
 */
IronClaw.api = {
  /**
   * Authenticated fetch wrapper — injects the session token.
   *
   * **Same-origin enforcement.** The session token is injected into the
   * `Authorization` header on every call, so a cross-origin URL would
   * leak the token to an attacker-controlled host. Resolve the requested
   * path against the page's own origin and reject anything that lands on
   * a different origin. Site-relative paths (`/api/foo`) and same-origin
   * absolute URLs are still allowed; everything else (`https://evil.example/...`,
   * protocol-relative `//evil.example/...`, `javascript:`, `data:`) is
   * rejected with a clear `TypeError` so the widget author sees the
   * misuse at the offending call site instead of having the request fly
   * silently to a hostile host.
   */
  fetch: function(path, opts) {
    var resolved;
    try {
      resolved = new URL(path, window.location.origin);
    } catch (e) {
      return Promise.reject(
        new TypeError('IronClaw.api.fetch: invalid URL ' + JSON.stringify(path))
      );
    }
    if (resolved.origin !== window.location.origin) {
      return Promise.reject(
        new TypeError(
          'IronClaw.api.fetch: cross-origin requests are not allowed (got ' +
          resolved.origin + ', expected ' + window.location.origin +
          '). Use a relative path or a same-origin absolute URL.'
        )
      );
    }
    opts = opts || {};
    opts.headers = Object.assign({}, opts.headers || {}, {
      'Authorization': 'Bearer ' + token
    });
    return fetch(resolved.toString(), opts);
  },

  /** Subscribe to an SSE/WebSocket event type. Returns an unsubscribe function. */
  subscribe: function(eventType, handler) {
    if (!window._widgetEventHandlers) window._widgetEventHandlers = {};
    if (!window._widgetEventHandlers[eventType]) window._widgetEventHandlers[eventType] = [];
    window._widgetEventHandlers[eventType].push(handler);
    return function() {
      var handlers = window._widgetEventHandlers[eventType];
      if (handlers) {
        var idx = handlers.indexOf(handler);
        if (idx !== -1) handlers.splice(idx, 1);
      }
    };
  },

  /**
   * Dispatch an SSE event to registered widget handlers.
   * Called internally by SSE event listeners — not for widget use.
   * @private
   */
  _dispatch: function(eventType, data) {
    var handlers = window._widgetEventHandlers && window._widgetEventHandlers[eventType];
    if (!handlers || handlers.length === 0) return;
    for (var i = 0; i < handlers.length; i++) {
      try { handlers[i](data); } catch (e) {
        console.error('[IronClaw] Widget event handler error (' + eventType + '):', e);
      }
    }
  },

  /** Current theme information. */
  theme: {
    get current() { return document.documentElement.dataset.theme || 'dark'; }
  },

  /** Internationalization helper. */
  i18n: {
    t: function(key) { return (window.I18n && window.I18n.t) ? window.I18n.t(key) : key; }
  },

  /** Navigate to a tab by ID. */
  navigate: function(tabId) {
    if (typeof switchTab === 'function') switchTab(tabId);
  }
};

/**
 * Add a widget as a new tab in the tab bar.
 * @private
 */
