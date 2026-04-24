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

/** Check whether a provider has all required credentials (API key + base URL if required). */
function isProviderConfigured(provider) {
  // ── API key check ──────────────────────────────────────────────────────
  // Built-in providers carry `api_key_required` from the backend registry.
  // Custom providers don't — derive the requirement from the adapter instead:
  // ollama runs locally and needs no key; other adapters do.
  const needsKey = provider.builtin
    ? provider.api_key_required !== false
    : provider.adapter !== 'ollama';
  const hasEnvKey = provider.has_api_key === true;
  const overrideKey = provider.builtin && _builtinOverrides[provider.id]
    ? _builtinOverrides[provider.id].api_key
    : undefined;
  // For custom providers, `api_key` is either the sentinel (vaulted on the
  // server) OR a freshly-entered plaintext string that hasn't been swapped
  // for the sentinel yet. Both mean the provider is configured.
  const customKey = !provider.builtin ? provider.api_key : undefined;
  const hasDbKey = provider.builtin
    ? (overrideKey === API_KEY_UNCHANGED || (typeof overrideKey === 'string' && overrideKey.length > 0))
    : (customKey === API_KEY_UNCHANGED || (typeof customKey === 'string' && customKey.length > 0));
  const keyOk = !needsKey || hasEnvKey || hasDbKey;
  if (!keyOk) return false;

  // ── Base URL check ─────────────────────────────────────────────────────
  // Built-ins with `base_url_required` (e.g. openai_compatible) have no
  // hardcoded fallback in the client layer, so activation must be gated on
  // having a URL from SOME source. Custom providers always need a URL
  // because they have no default at all.
  const needsBaseUrl = provider.builtin
    ? provider.base_url_required === true
    : true;
  if (!needsBaseUrl) return true;

  const overrideBaseUrl = provider.builtin && _builtinOverrides[provider.id]
    ? _builtinOverrides[provider.id].base_url
    : undefined;
  const hasOverrideBaseUrl = typeof overrideBaseUrl === 'string' && overrideBaseUrl.trim().length > 0;
  const hasEnvBaseUrl = typeof provider.env_base_url === 'string' && provider.env_base_url.trim().length > 0;
  // `provider.base_url` is the registry default for built-ins (may be empty
  // when base_url_required=true and there's no default) OR the user-set URL
  // for custom providers.
  const hasProviderBaseUrl = typeof provider.base_url === 'string' && provider.base_url.trim().length > 0;
  return hasOverrideBaseUrl || hasEnvBaseUrl || hasProviderBaseUrl;
}

/**
 * Determine what's missing on an unconfigured provider for a precise toast.
 * Returns 'base_url' if the base URL is missing, 'api_key' if the key is
 * missing, or 'ok' if nothing is missing. Mirrors the checks in
 * isProviderConfigured — keep the two in sync.
 */
function providerMissingReason(provider) {
  // API key check — matches isProviderConfigured above.
  const needsKey = provider.builtin
    ? provider.api_key_required !== false
    : provider.adapter !== 'ollama';
  if (needsKey) {
    const hasEnvKey = provider.has_api_key === true;
    const overrideKey = provider.builtin && _builtinOverrides[provider.id]
      ? _builtinOverrides[provider.id].api_key
      : undefined;
    const customKey = !provider.builtin ? provider.api_key : undefined;
    const hasDbKey = provider.builtin
      ? (overrideKey === API_KEY_UNCHANGED || (typeof overrideKey === 'string' && overrideKey.length > 0))
      : (customKey === API_KEY_UNCHANGED || (typeof customKey === 'string' && customKey.length > 0));
    if (!hasEnvKey && !hasDbKey) return 'api_key';
  }
  // Base URL check — matches isProviderConfigured above.
  const needsBaseUrl = provider.builtin
    ? provider.base_url_required === true
    : true;
  if (needsBaseUrl) {
    const overrideBaseUrl = provider.builtin && _builtinOverrides[provider.id]
      ? _builtinOverrides[provider.id].base_url
      : undefined;
    const hasOverrideBaseUrl = typeof overrideBaseUrl === 'string' && overrideBaseUrl.trim().length > 0;
    const hasEnvBaseUrl = typeof provider.env_base_url === 'string' && provider.env_base_url.trim().length > 0;
    const hasProviderBaseUrl = typeof provider.base_url === 'string' && provider.base_url.trim().length > 0;
    if (!hasOverrideBaseUrl && !hasEnvBaseUrl && !hasProviderBaseUrl) return 'base_url';
  }
  return 'ok';
}

/** Open the appropriate configuration dialog for a provider. */
function openProviderConfigDialog(provider) {
  if (provider.builtin && provider.id !== 'bedrock') {
    configureBuiltinProvider(provider.id);
  } else if (!provider.builtin) {
    editCustomProvider(provider.id);
  }
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
    const isConfigured = isProviderConfigured(p);
    const activeBadge = isActive
      ? '<span class="provider-badge provider-badge-active">' + I18n.t('status.active') + '</span>'
      : '';
    const builtinBadge = p.builtin
      ? '<span class="provider-badge provider-badge-builtin">' + I18n.t('config.builtin') + '</span>'
      : '';
    const unconfiguredBadge = !isActive && !isConfigured
      ? '<span class="provider-badge provider-badge-unconfigured">' + I18n.t('config.notConfigured') + '</span>'
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
    // Only show "Use" if provider is configured; unconfigured providers must be configured first
    const useBtn = !isActive && isConfigured
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
      +   activeBadge + builtinBadge + unconfiguredBadge
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
  if (provider && !isProviderConfigured(provider)) {
    // Pick a specific message so the user knows WHAT is missing, not just
    // "configure the provider". Check base URL first because a provider
    // that needs both a key and a URL typically surfaces URL entry first
    // in the dialog layout.
    const reason = providerMissingReason(provider);
    const toastKey = reason === 'base_url' ? 'config.baseUrlRequired' : 'config.configureToUse';
    showToast(I18n.t(toastKey), 'error');
    openProviderConfigDialog(provider);
    return;
  }
  // Restore the last-configured model for this provider, falling back to the provider's default
  const overrideModel = _builtinOverrides[id] && _builtinOverrides[id].model;
  const envModel = provider && provider.env_model;
  const restoredModel = overrideModel || envModel || (provider && provider.default_model) || null;
  const defaultModel = restoredModel;
  // Guard: a model must be available
  if (!defaultModel) {
    showToast(I18n.t('config.modelRequired') || 'Model is required', 'error');
    if (provider) openProviderConfigDialog(provider);
    return;
  }
  // Write backend + model atomically. Two sequential PUTs would hot-reload
  // the chain between them with the new backend but the previous model
  // (selected_model wins over provider defaults), leaving a mixed state
  // if the second request fails. Import writes the set and reloads once.
  apiFetchVoid('/api/settings/import', {
    method: 'POST',
    body: { settings: { llm_backend: id, selected_model: defaultModel } },
  })
    .then(() => {
      _activeLlmBackend = id;
      _selectedModel = defaultModel || '';
      renderProviders();
      loadInferenceSettings();
      scrollToProviders();
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
  },

  /**
   * Open a share modal with social buttons.
   *
   * @param {Object} opts
   * @param {string} opts.imageDataUrl - PNG data URL of the card image
   * @param {string} opts.text         - Pre-filled share text
   * @param {string} [opts.hashtags]   - Comma-separated hashtags (no #)
   */
  share: function(opts) {
    if (!opts || typeof opts.imageDataUrl !== 'string' ||
        !opts.imageDataUrl.startsWith('data:image/png')) {
      return;
    }
    var overlay = document.getElementById('share-modal-overlay');
    if (!overlay) {
      overlay = document.createElement('div');
      overlay.id = 'share-modal-overlay';
      overlay.className = 'share-overlay';
      overlay.innerHTML =
        '<div class="share-modal" role="dialog" aria-modal="true" aria-labelledby="share-modal-title">' +
        '  <div class="share-header">' +
        '    <span class="share-title" id="share-modal-title">Share your gains</span>' +
        '    <button class="share-close" aria-label="Close share dialog">&times;</button>' +
        '  </div>' +
        '  <div class="share-preview"></div>' +
        '  <div class="share-actions">' +
        '    <button class="share-btn share-x" aria-label="Share on X" title="Share on X">' +
        '      <svg aria-hidden="true" focusable="false" width="18" height="18" viewBox="0 0 24 24" fill="currentColor"><path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z"/></svg>' +
        '    </button>' +
        '    <button class="share-btn share-linkedin" aria-label="Share on LinkedIn" title="Share on LinkedIn">' +
        '      <svg aria-hidden="true" focusable="false" width="18" height="18" viewBox="0 0 24 24" fill="currentColor"><path d="M20.447 20.452h-3.554v-5.569c0-1.328-.027-3.037-1.852-3.037-1.853 0-2.136 1.445-2.136 2.939v5.667H9.351V9h3.414v1.561h.046c.477-.9 1.637-1.85 3.37-1.85 3.601 0 4.267 2.37 4.267 5.455v6.286zM5.337 7.433a2.062 2.062 0 01-2.063-2.065 2.064 2.064 0 112.063 2.065zm1.782 13.019H3.555V9h3.564v11.452zM22.225 0H1.771C.792 0 0 .774 0 1.729v20.542C0 23.227.792 24 1.771 24h20.451C23.2 24 24 23.227 24 22.271V1.729C24 .774 23.2 0 22.222 0h.003z"/></svg>' +
        '    </button>' +
        '    <button class="share-btn share-facebook" aria-label="Share on Facebook" title="Share on Facebook">' +
        '      <svg aria-hidden="true" focusable="false" width="18" height="18" viewBox="0 0 24 24" fill="currentColor"><path d="M24 12.073c0-6.627-5.373-12-12-12s-12 5.373-12 12c0 5.99 4.388 10.954 10.125 11.854v-8.385H7.078v-3.47h3.047V9.43c0-3.007 1.792-4.669 4.533-4.669 1.312 0 2.686.235 2.686.235v2.953H15.83c-1.491 0-1.956.925-1.956 1.874v2.25h3.328l-.532 3.47h-2.796v8.385C19.612 23.027 24 18.062 24 12.073z"/></svg>' +
        '    </button>' +
        '    <button class="share-btn share-copy" aria-label="Copy image to clipboard" title="Copy image">' +
        '      <svg aria-hidden="true" focusable="false" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>' +
        '    </button>' +
        '    <button class="share-btn share-download" aria-label="Download image" title="Download image">' +
        '      <svg aria-hidden="true" focusable="false" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>' +
        '    </button>' +
        '  </div>' +
        '  <div class="share-toast" role="status" aria-live="polite"></div>' +
        '</div>';
      document.body.appendChild(overlay);
      overlay.querySelector('.share-close').addEventListener('click', function() {
        overlay.style.display = 'none';
      });
      overlay.addEventListener('click', function(e) {
        if (e.target === overlay) overlay.style.display = 'none';
      });
    }

    var text = opts.text || '';
    var hashtags = opts.hashtags || 'DeFi,IronClaw';
    var encodedText = encodeURIComponent(text);
    var popupFeatures = 'noopener,noreferrer,width=550,height=';

    var preview = overlay.querySelector('.share-preview');
    preview.innerHTML = '';
    var cardImg = document.createElement('img');
    cardImg.className = 'share-card-img';
    cardImg.alt = 'Share card';
    cardImg.src = opts.imageDataUrl;
    preview.appendChild(cardImg);

    var toast = overlay.querySelector('.share-toast');
    function showToast(msg) {
      toast.textContent = msg;
      toast.classList.add('visible');
      setTimeout(function() { toast.classList.remove('visible'); }, 2000);
    }

    var xBtn = overlay.querySelector('.share-x');
    xBtn.onclick = function() {
      window.open(
        'https://twitter.com/intent/tweet?text=' + encodedText +
        '&hashtags=' + encodeURIComponent(hashtags),
        '_blank', popupFeatures + '420'
      );
    };
    var liBtn = overlay.querySelector('.share-linkedin');
    liBtn.onclick = function() {
      window.open(
        'https://www.linkedin.com/sharing/share-offsite/?mini=true&title=' + encodedText,
        '_blank', popupFeatures + '520'
      );
    };
    var fbBtn = overlay.querySelector('.share-facebook');
    fbBtn.onclick = function() {
      window.open(
        'https://www.facebook.com/sharer/sharer.php?quote=' + encodedText,
        '_blank', popupFeatures + '420'
      );
    };
    var copyBtn = overlay.querySelector('.share-copy');
    copyBtn.onclick = function() {
      var img = preview.querySelector('img');
      if (!img) return;
      var canvas = document.createElement('canvas');
      canvas.width = img.naturalWidth;
      canvas.height = img.naturalHeight;
      canvas.getContext('2d').drawImage(img, 0, 0);
      canvas.toBlob(function(blob) {
        if (navigator.clipboard && navigator.clipboard.write && typeof ClipboardItem !== 'undefined') {
          try {
            navigator.clipboard.write([new ClipboardItem({'image/png': blob})]).then(function() {
              showToast('Image copied!');
            }).catch(function() { showToast('Copy failed'); });
          } catch (_) { showToast('Clipboard not supported'); }
        } else {
          showToast('Clipboard not supported');
        }
      }, 'image/png');
    };
    var dlBtn = overlay.querySelector('.share-download');
    dlBtn.onclick = function() {
      var a = document.createElement('a');
      a.href = opts.imageDataUrl;
      a.download = 'ironclaw-portfolio-gains.png';
      a.click();
      showToast('Downloaded!');
    };

    overlay.style.display = 'flex';
  }
};

/**
 * Add a widget as a new tab in the tab bar.
 * @private
 */
