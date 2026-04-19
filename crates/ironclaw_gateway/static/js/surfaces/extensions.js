function loadExtensions() {
  const extList = document.getElementById('extensions-list');
  const wasmList = document.getElementById('available-wasm-list');
  extList.innerHTML = renderCardsSkeleton(3);

  // Fetch extensions and registry in parallel
  Promise.all([
    apiFetch('/api/extensions').catch(() => ({ extensions: [] })),
    apiFetch('/api/extensions/registry').catch(function(err) { console.warn('registry fetch failed:', err); return { entries: [] }; }),
  ]).then(([extData, registryData]) => {
    // Render installed extensions (exclude wasm_channel and mcp_server — shown in their own tabs)
    var nonChannelExts = extData.extensions.filter(function(e) {
      return e.kind !== 'wasm_channel' && e.kind !== 'mcp_server';
    });
    if (nonChannelExts.length === 0) {
      extList.innerHTML = '<div class="empty-state">' + I18n.t('extensions.noInstalled') + '</div>';
    } else {
      extList.innerHTML = '';
      for (const ext of nonChannelExts) {
        extList.appendChild(renderExtensionCard(ext));
      }
    }

    // Available extensions (exclude MCP servers and channels — they have their own tabs)
    var wasmEntries = registryData.entries.filter(function(e) {
      return e.kind !== 'mcp_server' && e.kind !== 'wasm_channel' && e.kind !== 'channel' && !e.installed;
    });

    var wasmSection = document.getElementById('available-wasm-section');
    if (wasmEntries.length === 0) {
      if (wasmSection) wasmSection.style.display = 'none';
    } else {
      if (wasmSection) wasmSection.style.display = '';
      wasmList.innerHTML = '';
      for (const entry of wasmEntries) {
        wasmList.appendChild(renderAvailableExtensionCard(entry));
      }
    }

  });
}

function renderAvailableExtensionCard(entry) {
  const card = document.createElement('div');
  card.className = 'ext-card ext-available';

  const header = document.createElement('div');
  header.className = 'ext-header';

  const name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = entry.display_name;
  header.appendChild(name);

  const kind = document.createElement('span');
  kind.className = 'ext-kind kind-' + entry.kind;
  kind.textContent = kindLabels[entry.kind] || entry.kind;
  header.appendChild(kind);

  if (entry.version) {
    const ver = document.createElement('span');
    ver.className = 'ext-version';
    ver.textContent = 'v' + entry.version;
    header.appendChild(ver);
  }

  card.appendChild(header);

  const desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = entry.description;
  card.appendChild(desc);

  if (entry.keywords && entry.keywords.length > 0) {
    const kw = document.createElement('div');
    kw.className = 'ext-keywords';
    kw.textContent = entry.keywords.join(', ');
    card.appendChild(kw);
  }

  const actions = document.createElement('div');
  actions.className = 'ext-actions';

  const installBtn = document.createElement('button');
  installBtn.className = 'btn-ext install';
  installBtn.textContent = I18n.t('extensions.install');
  installBtn.addEventListener('click', function() {
    installBtn.disabled = true;
    installBtn.textContent = I18n.t('extensions.installing');
    apiFetch('/api/extensions/install', {
      method: 'POST',
      body: { name: entry.name, kind: entry.kind },
    }).then(function(res) {
      if (res.success) {
        showToast(I18n.t('extensions.installedSuccess', {name: entry.display_name}), 'success');
        // OAuth popup if auth started during install (builtin creds)
        if (res.auth_url) {
          handleAuthRequired({
            extension_name: entry.name,
            auth_url: res.auth_url,
            display_name: entry.display_name || entry.name,
            block_chat: false,
          });
          showToast(I18n.t('extensions.openingAuth', { name: entry.display_name }), 'info');
          openOAuthUrl(res.auth_url);
        }
        refreshCurrentSettingsTab();
        // Auto-open configure for WASM channels
        if (entry.kind === 'wasm_channel') {
          showConfigureModal(entry.name);
        }
      } else {
        showToast(I18n.t('extensions.installFailed', { message: res.message || 'unknown error' }), 'error');
        refreshCurrentSettingsTab();
      }
    }).catch(function(err) {
      showToast(I18n.t('extensions.installFailed', { message: err.message }), 'error');
      refreshCurrentSettingsTab();
    });
  });
  actions.appendChild(installBtn);

  card.appendChild(actions);
  return card;
}

function renderMcpServerCard(entry, installedExt) {
  var card = document.createElement('div');
  card.className = 'ext-card' + (installedExt ? '' : ' ext-available');

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = entry.display_name;
  header.appendChild(name);

  var kind = document.createElement('span');
  kind.className = 'ext-kind kind-mcp_server';
  kind.textContent = kindLabels['mcp_server'] || 'mcp_server';
  header.appendChild(kind);

  if (installedExt) {
    var authDot = document.createElement('span');
    authDot.className = 'ext-auth-dot ' + (installedExt.authenticated ? 'authed' : 'unauthed');
    authDot.title = installedExt.authenticated ? I18n.t('auth.authenticated') : I18n.t('auth.notAuthenticated');
    header.appendChild(authDot);
  }

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = entry.description;
  card.appendChild(desc);

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  if (installedExt) {
    if (!installedExt.active) {
      var activateBtn = document.createElement('button');
      activateBtn.className = 'btn-ext activate';
      activateBtn.textContent = I18n.t('common.activate');
      activateBtn.addEventListener('click', function() { activateExtension(installedExt.name); });
      actions.appendChild(activateBtn);
    } else {
      var activeLabel = document.createElement('span');
      activeLabel.className = 'ext-active-label';
      activeLabel.textContent = I18n.t('ext.active');
      actions.appendChild(activeLabel);
    }
    if (installedExt.needs_setup || (installedExt.has_auth && installedExt.authenticated)) {
      var configBtn = document.createElement('button');
      configBtn.className = 'btn-ext configure';
      configBtn.textContent = installedExt.authenticated ? I18n.t('ext.reconfigure') : I18n.t('ext.configure');
      configBtn.addEventListener('click', function() { showConfigureModal(installedExt.name); });
      actions.appendChild(configBtn);
    }
    var removeBtn = document.createElement('button');
    removeBtn.className = 'btn-ext remove';
    removeBtn.textContent = I18n.t('ext.remove');
    removeBtn.addEventListener('click', function() { removeExtension(installedExt.name); });
    actions.appendChild(removeBtn);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-ext install';
    installBtn.textContent = I18n.t('ext.install');
    installBtn.addEventListener('click', function() {
      installBtn.disabled = true;
      installBtn.textContent = I18n.t('ext.installing');
      apiFetch('/api/extensions/install', {
        method: 'POST',
        body: { name: entry.name, kind: entry.kind },
      }).then(function(res) {
        if (res.success) {
          showToast(I18n.t('extensions.installedSuccess', { name: entry.display_name }), 'success');
        } else {
          showToast(I18n.t('ext.install') + ': ' + (res.message || 'unknown error'), 'error');
        }
        loadMcpServers();
      }).catch(function(err) {
        showToast(I18n.t('ext.installFailed', { message: err.message }), 'error');
        loadMcpServers();
      });
    });
    actions.appendChild(installBtn);
  }

  card.appendChild(actions);
  return card;
}

function createReconfigureButton(extName) {
  var btn = document.createElement('button');
  btn.className = 'btn-ext configure';
  btn.textContent = I18n.t('ext.reconfigure');
  btn.addEventListener('click', function() { showConfigureModal(extName); });
  return btn;
}

function renderExtensionCard(ext) {
  const card = document.createElement('div');
  var stateClass = 'state-inactive';
  if (ext.kind === 'wasm_channel') {
    var s = ext.onboarding_state || ext.activation_status || 'installed';
    if (s === 'active') stateClass = 'state-active';
    else if (s === 'ready') stateClass = 'state-active';
    else if (s === 'failed') stateClass = 'state-error';
    else if (s === 'pairing') stateClass = 'state-pairing';
    else if (s === 'pairing_required') stateClass = 'state-pairing';
  } else if (ext.active) {
    stateClass = 'state-active';
  }
  card.className = 'ext-card ' + stateClass;

  const header = document.createElement('div');
  header.className = 'ext-header';

  const name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = ext.display_name || ext.name;
  header.appendChild(name);

  const kind = document.createElement('span');
  kind.className = 'ext-kind kind-' + ext.kind;
  kind.textContent = kindLabels[ext.kind] || ext.kind;
  header.appendChild(kind);

  if (ext.version) {
    const ver = document.createElement('span');
    ver.className = 'ext-version';
    ver.textContent = 'v' + ext.version;
    header.appendChild(ver);
  }

  // Auth dot only for non-WASM-channel extensions (channels use the stepper instead)
  if (ext.kind !== 'wasm_channel') {
    const authDot = document.createElement('span');
    authDot.className = 'ext-auth-dot ' + (ext.authenticated ? 'authed' : 'unauthed');
    authDot.title = ext.authenticated ? I18n.t('auth.authenticated') : I18n.t('auth.notAuthenticated');
    header.appendChild(authDot);
  }

  card.appendChild(header);

  // WASM channels get a progress stepper
  if (ext.kind === 'wasm_channel') {
    card.appendChild(renderWasmChannelStepper(ext));
  }

  if (ext.description) {
    const desc = document.createElement('div');
    desc.className = 'ext-desc';
    desc.textContent = ext.description;
    card.appendChild(desc);
  }

  if (ext.url) {
    const url = document.createElement('div');
    url.className = 'ext-url';
    url.textContent = ext.url;
    url.title = ext.url;
    card.appendChild(url);
  }

  if (ext.tools && ext.tools.length > 0) {
    const tools = document.createElement('div');
    tools.className = 'ext-tools';
    tools.textContent = I18n.t('extensions.toolsLabel', { list: ext.tools.join(', ') });
    card.appendChild(tools);
  }

  // Show activation error for WASM channels
  if (ext.kind === 'wasm_channel' && ext.activation_error) {
    const errorDiv = document.createElement('div');
    errorDiv.className = 'ext-error';
    errorDiv.textContent = ext.activation_error;
    card.appendChild(errorDiv);
  }


  const actions = document.createElement('div');
  actions.className = 'ext-actions';

  if (ext.kind === 'wasm_channel') {
    // WASM channels: state-based buttons (no generic Activate)
    var status = ext.onboarding_state || ext.activation_status || 'installed';
    if (status === 'active' || status === 'ready') {
      var activeLabel = document.createElement('span');
      activeLabel.className = 'ext-active-label';
      activeLabel.textContent = I18n.t('ext.active');
      actions.appendChild(activeLabel);
      actions.appendChild(createReconfigureButton(ext.name));
    } else if (status === 'pairing' || status === 'pairing_required') {
      var pairingLabel = document.createElement('span');
      pairingLabel.className = 'ext-pairing-label';
      pairingLabel.textContent = I18n.t('status.awaitingPairing');
      actions.appendChild(pairingLabel);
      actions.appendChild(createReconfigureButton(ext.name));
    } else if (status === 'failed') {
      actions.appendChild(createReconfigureButton(ext.name));
    } else {
      var reconfigureBtn = document.createElement('button');
      reconfigureBtn.className = 'btn-ext configure';
      reconfigureBtn.textContent = I18n.t('extensions.reconfigure');
      reconfigureBtn.addEventListener('click', function() { showConfigureModal(ext.name); });
      actions.appendChild(reconfigureBtn);
    }
  } else {
    // WASM tools / MCP servers
    const activeLabel = document.createElement('span');
    activeLabel.className = 'ext-active-label';
    activeLabel.textContent = ext.active ? I18n.t('ext.active') : I18n.t('status.installed');
    actions.appendChild(activeLabel);

    // MCP servers and channel-relay extensions may be installed but inactive — show Activate button
    if ((ext.kind === 'mcp_server' || ext.kind === 'channel_relay') && !ext.active) {
      const activateBtn = document.createElement('button');
      activateBtn.className = 'btn-ext activate';
      activateBtn.textContent = I18n.t('common.activate');
      activateBtn.addEventListener('click', () => activateExtension(ext.name));
      actions.appendChild(activateBtn);
    }

    // Show Configure/Reconfigure button when there are secrets to enter.
    // Skip when has_auth is true but needs_setup is false and not yet authenticated —
    // this means OAuth credentials resolve automatically (builtin/env) and the user
    // just needs to complete the OAuth flow, not fill in a config form.
    if (ext.needs_setup || (ext.has_auth && ext.authenticated)) {
      const configBtn = document.createElement('button');
      configBtn.className = 'btn-ext configure';
      configBtn.textContent = ext.authenticated ? I18n.t('ext.reconfigure') : I18n.t('ext.configure');
      configBtn.addEventListener('click', () => showConfigureModal(ext.name));
      actions.appendChild(configBtn);
    }
  }

  const removeBtn = document.createElement('button');
  removeBtn.className = 'btn-ext remove';
  removeBtn.textContent = I18n.t('ext.remove');
  removeBtn.addEventListener('click', () => removeExtension(ext.name));
  actions.appendChild(removeBtn);

  card.appendChild(actions);

  // For WASM channels, check for pending pairing requests.
  if (ext.kind === 'wasm_channel') {
    if ((ext.onboarding_state || ext.activation_status || 'installed') === 'setup_required') {
      const setupSection = document.createElement('div');
      setupSection.className = 'ext-onboarding';
      card.appendChild(setupSection);
      loadInlineChannelSetup(ext, setupSection);
    }
    if ((ext.onboarding_state || ext.activation_status || 'installed') === 'pairing_required'
      || (ext.onboarding_state || ext.activation_status || 'installed') === 'pairing') {
      const pairingSection = document.createElement('div');
      pairingSection.className = 'ext-pairing';
      pairingSection.setAttribute('data-channel', ext.name);
      pairingSection.__onboarding = ext.onboarding || null;
      card.appendChild(pairingSection);
      if (currentUserIsAdmin()) {
        loadPairingRequests(ext.name, pairingSection, ext.onboarding || null);
      } else {
        renderMemberPairingClaim(ext, pairingSection, ext.onboarding || null);
      }
    }
  }

  return card;
}

function loadInlineChannelSetup(ext, container) {
  apiFetch('/api/extensions/' + encodeURIComponent(ext.name) + '/setup')
    .then((setup) => {
      const onboarding = setup.onboarding || ext.onboarding || {};
      const secrets = Array.isArray(setup.secrets) ? setup.secrets : [];
      if (secrets.length === 0) {
        container.innerHTML = '';
        return;
      }

      container.innerHTML = '';

      const title = document.createElement('div');
      title.className = 'ext-onboarding-title';
      title.textContent = onboarding.credential_title || ('Configure credentials for ' + (ext.display_name || ext.name));
      container.appendChild(title);

      if (onboarding.credential_instructions) {
        const text = document.createElement('div');
        text.className = 'ext-onboarding-text';
        text.textContent = onboarding.credential_instructions;
        container.appendChild(text);
      }

      if (onboarding.setup_url) {
        // Strict HTTPS validation via shared helper.
        const parsedSetupUrl2 = parseHttpsExternalUrl(onboarding.setup_url, 'setup');
        if (parsedSetupUrl2) {
          const links = document.createElement('div');
          links.className = 'auth-links';
          const link = document.createElement('a');
          link.href = parsedSetupUrl2.href;
          link.target = '_blank';
          link.rel = 'noopener noreferrer';
          link.textContent = I18n.t('authRequired.getToken');
          links.appendChild(link);
          container.appendChild(links);
        }
      }

      const form = document.createElement('div');
      form.className = 'setup-form inline';
      container.appendChild(form);

      let fields = [];
      const submit = () => submitInlineChannelSetup(ext.name, fields, container);
      fields = buildSetupFields(form, ext.name, secrets, submit);

      if (onboarding.credential_next_step) {
        const nextStep = document.createElement('div');
        nextStep.className = 'setup-next-step';
        nextStep.textContent = onboarding.credential_next_step;
        container.appendChild(nextStep);
      }

      const actions = document.createElement('div');
      actions.className = 'ext-actions';
      const submitBtn = document.createElement('button');
      submitBtn.className = 'btn-ext activate';
      submitBtn.textContent = I18n.t('config.save');
      submitBtn.addEventListener('click', submit);
      actions.appendChild(submitBtn);
      container.appendChild(actions);
    })
    .catch(() => {
      container.innerHTML = '';
    });
}

function submitInlineChannelSetup(name, fields, container) {
  const secrets = {};
  (fields || []).forEach((field) => {
    const value = (field.input.value || '').trim();
    if (value) secrets[field.name] = value;
  });

  const buttons = container.querySelectorAll('button');
  buttons.forEach((btn) => { btn.disabled = true; });

  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup', {
    method: 'POST',
    body: { secrets, fields: {} },
  }).then((res) => {
    if (!res.success) {
      showToast(res.message || 'Configuration failed', 'error');
      buttons.forEach((btn) => { btn.disabled = false; });
      return;
    }
    if (res.onboarding_state === 'pairing_required') {
      showPairingCard({
        channel: name,
        instructions: res.onboarding && res.onboarding.pairing_instructions,
        onboarding: res.onboarding || null,
      });
    }
    refreshCurrentSettingsTab();
  }).catch((err) => {
    buttons.forEach((btn) => { btn.disabled = false; });
    showToast(I18n.t('extensions.configFailed', { message: err.message }), 'error');
  });
}

function refreshCurrentSettingsTab() {
  if (currentSettingsSubtab === 'extensions') loadExtensions();
  if (currentSettingsSubtab === 'channels') loadChannelsStatus();
  if (currentSettingsSubtab === 'mcp') loadMcpServers();
}

function activateExtension(name) {
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/activate', { method: 'POST' })
    .then((res) => {
      if (res.success) {
        // Even on success, the tool may need OAuth (e.g., WASM loaded but no token yet)
        if (res.auth_url) {
          handleAuthRequired({
            extension_name: name,
            auth_url: res.auth_url,
            display_name: name,
            block_chat: false,
          });
          showToast(I18n.t('extensions.openingAuth', { name: name }), 'info');
          openOAuthUrl(res.auth_url);
        }
        refreshCurrentSettingsTab();
        return;
      }

      if (res.auth_url) {
        handleAuthRequired({
          extension_name: name,
          auth_url: res.auth_url,
          display_name: name,
          block_chat: false,
        });
        showToast(I18n.t('extensions.openingAuth', { name: name }), 'info');
        openOAuthUrl(res.auth_url);
      } else if (res.awaiting_token) {
        showConfigureModal(name);
      } else {
        showToast(I18n.t('extensions.activateFailed', { message: res.message }), 'error');
      }
      refreshCurrentSettingsTab();
    })
    .catch((err) => showToast(I18n.t('extensions.activateFailed', { message: err.message }), 'error'));
}

function removeExtension(name) {
  showConfirmModal(I18n.t('ext.confirmRemove', { name: name }), '', function() {
    apiFetch('/api/extensions/' + encodeURIComponent(name) + '/remove', { method: 'POST' })
      .then((res) => {
        if (!res.success) {
          showToast(I18n.t('ext.removeFailed', { message: res.message }), 'error');
        } else {
          showToast(I18n.t('ext.removed', { name: name }), 'success');
        }
        refreshCurrentSettingsTab();
      })
      .catch((err) => showToast(I18n.t('ext.removeFailed', { message: err.message }), 'error'));
  }, I18n.t('common.remove'), 'btn-danger');
}

function showConfigureModal(name, options) {
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup')
    .then((setup) => {
      const secrets = Array.isArray(setup.secrets) ? setup.secrets : [];
      const setupFields = Array.isArray(setup.fields) ? setup.fields : [];
      if (secrets.length === 0 && setupFields.length === 0) {
        if (options && options.authData) {
          showAuthCard(options.authData);
        } else {
          showToast(I18n.t('extensions.noConfigNeeded', { name: name }), 'info');
        }
        return;
      }
      renderConfigureModal(name, secrets, setupFields, setup.onboarding || null, options);
    })
    .catch((err) => {
      showToast(I18n.t('extensions.setupLoadFailed', { message: err.message }), 'error');
      if (options && options.authData) {
        showAuthCard(options.authData);
      }
    });
}

function renderConfigureModal(name, secrets, setupFields, onboarding, options) {
  // Cancel any existing auth-flow overlay before replacing it.
  // Remove directly (don't clear authFlowPending) since a new overlay is about to be appended.
  var existingOverlay = document.querySelector('.configure-overlay');
  if (existingOverlay && existingOverlay.getAttribute('data-auth-flow')) {
    existingOverlay.remove();
  } else {
    closeConfigureModal();
  }
  const overlay = document.createElement('div');
  overlay.className = 'configure-overlay';
  overlay.setAttribute('data-extension-name', name);
  if (options && options.authData) {
    overlay.setAttribute('data-auth-flow', 'true');
    overlay.setAttribute('data-auth-extension', options.authData.extension_name || name);
    if (options.authData.request_id) overlay.setAttribute('data-request-id', options.authData.request_id);
    if (options.authData.thread_id) overlay.setAttribute('data-thread-id', options.authData.thread_id);
  }
  overlay.addEventListener('click', (e) => {
    if (e.target !== overlay) return;
    if (overlay.getAttribute('data-auth-flow')) {
      cancelAuthFromConfigureModal(overlay);
    } else {
      closeConfigureModal();
    }
  });

  const modal = document.createElement('div');
  modal.className = 'configure-modal';

  const header = document.createElement('h3');
  header.textContent = I18n.t('config.title', { name: name });
  modal.appendChild(header);

  if (onboarding && onboarding.credential_instructions) {
    const hint = document.createElement('div');
    hint.className = 'configure-hint';
    hint.textContent = onboarding.credential_instructions;
    modal.appendChild(hint);
  }

  const form = document.createElement('div');
  form.className = 'configure-form';

  const fields = [];
  for (const secret of secrets) {
    const field = document.createElement('div');
    field.className = 'configure-field';
    field.dataset.secretName = secret.name;

    const label = document.createElement('label');
    label.textContent = secret.prompt;
    if (secret.optional) {
      const opt = document.createElement('span');
      opt.className = 'field-optional';
      opt.textContent = I18n.t('config.optional');
      label.appendChild(opt);
    }
    field.appendChild(label);

    const inputRow = document.createElement('div');
    inputRow.className = 'configure-input-row';

    const input = document.createElement('input');
    input.type = 'password';
    input.name = secret.name;
    input.placeholder = secret.provided ? I18n.t('config.alreadySet') : '';
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submitConfigureModal(name, fields);
    });
    inputRow.appendChild(input);

    if (secret.provided) {
      const badge = document.createElement('span');
      badge.className = 'field-provided';
      badge.textContent = '\u2713';
      badge.title = I18n.t('config.alreadyConfigured');
      inputRow.appendChild(badge);
    }
    if (secret.auto_generate && !secret.provided) {
      const hint = document.createElement('span');
      hint.className = 'field-autogen';
      hint.textContent = I18n.t('config.autoGenerate');
      inputRow.appendChild(hint);
    }

    field.appendChild(inputRow);
    form.appendChild(field);
    fields.push({ kind: 'secret', name: secret.name, input: input });
  }

  for (const setupField of setupFields) {
    const field = document.createElement('div');
    field.className = 'configure-field';

    const label = document.createElement('label');
    label.textContent = setupField.prompt;
    if (setupField.optional) {
      const opt = document.createElement('span');
      opt.className = 'field-optional';
      opt.textContent = I18n.t('config.optional');
      label.appendChild(opt);
    }
    field.appendChild(label);

    const inputRow = document.createElement('div');
    inputRow.className = 'configure-input-row';

    const input = document.createElement('input');
    input.type = setupField.input_type === 'password' ? 'password' : 'text';
    input.name = setupField.name;
    input.placeholder = setupField.provided ? I18n.t('config.alreadySet') : '';
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submitConfigureModal(name, fields);
    });
    inputRow.appendChild(input);

    if (setupField.provided) {
      const badge = document.createElement('span');
      badge.className = 'field-provided';
      badge.textContent = '\u2713';
      badge.title = I18n.t('config.alreadyConfigured');
      inputRow.appendChild(badge);
    }

    field.appendChild(inputRow);
    form.appendChild(field);
    fields.push({ kind: 'field', name: setupField.name, input: input });
  }

  modal.appendChild(form);

  const error = document.createElement('div');
  error.className = 'configure-inline-error';
  error.style.display = 'none';
  modal.appendChild(error);

  const actions = document.createElement('div');
  actions.className = 'configure-actions';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'btn-ext activate';
  submitBtn.textContent = I18n.t('config.save');
  submitBtn.addEventListener('click', () => submitConfigureModal(name, fields));
  actions.appendChild(submitBtn);

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'btn-ext remove';
  cancelBtn.textContent = I18n.t('config.cancel');
  cancelBtn.addEventListener('click', function() {
    if (overlay.getAttribute('data-auth-flow')) {
      cancelAuthFromConfigureModal(overlay);
    } else {
      closeConfigureModal();
    }
  });
  actions.appendChild(cancelBtn);

  modal.appendChild(actions);
  overlay.appendChild(modal);
  document.body.appendChild(overlay);

  if (fields.length > 0) fields[0].input.focus();
}

function setConfigureInlineError(overlay, message) {
  const error = overlay && overlay.querySelector('.configure-inline-error');
  if (!error) return;
  error.textContent = message || '';
  error.style.display = message ? 'block' : 'none';
}

function clearConfigureInlineError(overlay) {
  setConfigureInlineError(overlay, '');
}

function submitConfigureModal(name, fields, options) {
  options = options || {};
  const secrets = {};
  const setupFields = {};
  for (const f of fields) {
    const value = f.input.value.trim();
    if (!value) {
      continue;
    }
    if (f.kind === 'secret') {
      secrets[f.name] = value;
    } else {
      setupFields[f.name] = value;
    }
  }

  const overlay = getConfigureOverlay(name) || document.querySelector('.configure-overlay');
  const requestId = overlay ? overlay.getAttribute('data-request-id') : null;
  const threadId = overlay ? overlay.getAttribute('data-thread-id') : null;
  clearConfigureInlineError(overlay);

  // Disable buttons to prevent double-submit
  var btns = overlay ? overlay.querySelectorAll('.configure-actions button') : [];
  btns.forEach(function(b) { b.disabled = true; });

  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup', {
    method: 'POST',
    body: {
      request_id: requestId || undefined,
      thread_id: threadId || undefined,
      secrets,
      fields: setupFields,
    },
  })
    .then((res) => {
      if (res.success) {
        // Strip auth-flow flag before closing so closeConfigureModal
        // does not trigger a spurious gate cancellation.
        if (overlay) overlay.removeAttribute('data-auth-flow');
        closeConfigureModal();
        if (res.auth_url) {
          handleAuthRequired({
            extension_name: name,
            auth_url: res.auth_url,
            display_name: name,
            block_chat: false,
          });
          showToast(I18n.t('extensions.openingOAuth', { name: name }), 'info');
          openOAuthUrl(res.auth_url);
          refreshCurrentSettingsTab();
        }
        // For non-OAuth success: the server always broadcasts onboarding_state SSE,
        // which will show the toast and refresh extensions — no need to do it here too.
      } else {
        // Keep modal open so the user can correct their input and retry.
        btns.forEach(function(b) { b.disabled = false; });
        setConfigureInlineError(overlay, res.message || 'Configuration failed');
        showToast(res.message || 'Configuration failed', 'error');
      }
    })
    .catch((err) => {
      btns.forEach(function(b) { b.disabled = false; });
      setConfigureInlineError(overlay, 'Configuration failed: ' + err.message);
      showToast(I18n.t('extensions.configFailed', { message: err.message }), 'error');
    });
}

function closeConfigureModal(extensionName) {
  if (typeof extensionName !== 'string') extensionName = null;
  const existing = getConfigureOverlay(extensionName);
  if (existing) existing.remove();
  if (!document.querySelector('.configure-overlay') && !document.querySelector('.auth-card')) {
    setAuthFlowPending(false);
    enableChatInput();
  }
}

function requestAuthCancellation(requestId, threadId) {
  const targetThreadId = threadId || currentThreadId || undefined;
  if (requestId) {
    return apiFetch('/api/chat/gate/resolve', {
      method: 'POST',
      body: {
        request_id: requestId,
        thread_id: targetThreadId,
        resolution: 'cancelled'
      }
    });
  }

  // Legacy `pending_auth` cancel path. Remove this when web auth no longer
  // uses thread-level auth mode and every prompt is gate-backed.
  return apiFetch('/api/chat/auth-cancel', {
    method: 'POST',
    body: {
      thread_id: targetThreadId,
    }
  });
}

function cancelAuthFromConfigureModal(overlay) {
  var requestId = overlay.getAttribute('data-request-id');
  var threadId = overlay.getAttribute('data-thread-id');
  requestAuthCancellation(requestId, threadId).catch(function() {});
  overlay.remove();
  if (!document.querySelector('.configure-overlay') && !document.querySelector('.auth-card')) {
    setAuthFlowPending(false);
    enableChatInput();
  }
}

function currentUserIsAdmin() {
  return !!(window._currentUser && window._currentUser.role === 'admin');
}

// Validate that a server-supplied OAuth URL is HTTPS before opening a popup.
// Rejects javascript:, data:, and other non-HTTPS schemes to prevent URL-injection.
// Uses the URL constructor to safely parse and validate the scheme, which also
// handles non-string values (objects, null, etc.) that would throw on .startsWith().
function parseHttpsExternalUrl(url, label) {
  let parsed;
  try {
    parsed = new URL(url);
    if (parsed.protocol !== 'https:') {
      throw new Error('non-HTTPS protocol: ' + parsed.protocol);
    }
  } catch (e) {
    console.warn(`Blocked invalid/non-HTTPS ${label} URL:`, url, e.message);
    showToast(I18n.t('extensions.invalidOAuthUrl'), 'error');
    return null;
  }
  return parsed;
}

function parseHttpsOAuthUrl(url) {
  return parseHttpsExternalUrl(url, 'OAuth');
}

function openOAuthUrl(url) {
  const parsed = parseHttpsOAuthUrl(url);
  if (!parsed) return;
  // `noopener,noreferrer` defends against tabnabbing — without these the
  // OAuth provider page can read `window.opener` and reach back into the
  // app tab. `noreferrer` also strips the Referer header.
  const opened = window.open(
    parsed.href,
    '_blank',
    'width=600,height=700,noopener,noreferrer',
  );
  // Some browsers ignore the noopener feature flag in window.open's third
  // argument when the window is non-null; explicitly null the opener as a
  // belt-and-suspenders defense.
  if (opened) {
    try {
      opened.opener = null;
    } catch (_) {
      /* opener may already be null in cross-origin contexts */
    }
  }
}

// --- Pairing ---

function loadPairingRequests(channel, container, onboarding) {
  if (!currentUserIsAdmin()) return;

  apiFetch('/api/pairing/' + encodeURIComponent(channel))
    .then(data => {
      container.innerHTML = '';

      const info = onboarding || {};

      const heading = document.createElement('div');
      heading.className = 'pairing-heading';
      heading.textContent = info.pairing_title || I18n.t('extensions.claimPairing');
      container.appendChild(heading);

      const help = document.createElement('div');
      help.className = 'pairing-help';
      help.textContent = info.pairing_instructions || I18n.t('extensions.claimPairingHelp');
      container.appendChild(help);

      const manual = document.createElement('div');
      manual.className = 'pairing-row pairing-manual';

      const input = document.createElement('input');
      input.className = 'pairing-manual-input';
      input.type = 'text';
      input.placeholder = I18n.t('extensions.pairingCodePlaceholder');
      input.autocomplete = 'off';
      input.spellcheck = false;
      input.autocapitalize = 'characters';
      input.maxLength = 64;
      input.addEventListener('keydown', function(event) {
        if (event.key === 'Enter') {
          event.preventDefault();
          approvePairing(channel, input.value, {
            onSuccess: function() {
              input.value = '';
              loadPairingRequests(channel, container, onboarding);
            }
          });
        }
      });
      manual.appendChild(input);

      const manualBtn = document.createElement('button');
      manualBtn.className = 'btn-ext activate pairing-manual-submit';
      manualBtn.textContent = I18n.t('approval.approve');
      manualBtn.addEventListener('click', function() {
        approvePairing(channel, input.value, {
          onSuccess: function() {
            input.value = '';
            loadPairingRequests(channel, container, onboarding);
          }
        });
      });
      manual.appendChild(manualBtn);
      container.appendChild(manual);

      if (info.restart_instructions) {
        const restart = document.createElement('div');
        restart.className = 'pairing-help pairing-restart';
        restart.textContent = info.restart_instructions;
        container.appendChild(restart);
      }

      if (!data.requests || data.requests.length === 0) return;

      const pendingHeading = document.createElement('div');
      pendingHeading.className = 'pairing-heading';
      pendingHeading.textContent = I18n.t('extensions.pendingPairing');
      container.appendChild(pendingHeading);

      data.requests.forEach(req => {
        const row = document.createElement('div');
        row.className = 'pairing-row';

        const code = document.createElement('span');
        code.className = 'pairing-code';
        code.textContent = req.code;
        row.appendChild(code);

        const sender = document.createElement('span');
        sender.className = 'pairing-sender';
        sender.textContent = I18n.t('extensions.from') + ' ' + req.sender_id;
        row.appendChild(sender);

        const btn = document.createElement('button');
        btn.className = 'btn-ext activate';
        btn.textContent = I18n.t('common.approve');
        btn.addEventListener('click', function() {
          approvePairing(channel, req.code, {
            onSuccess: function() {
              loadPairingRequests(channel, container, onboarding);
            }
          });
        });
        row.appendChild(btn);

        container.appendChild(row);
      });
    })
    .catch(() => {});
}

function renderMemberPairingClaim(ext, container, onboarding) {
  const info = onboarding || {};
  const heading = document.createElement('div');
  heading.className = 'pairing-heading';
  heading.textContent = info.pairing_title || I18n.t('extensions.claimPairing');
  container.appendChild(heading);

  const help = document.createElement('div');
  help.className = 'pairing-help';
  help.textContent = info.pairing_instructions || I18n.t('extensions.claimPairingHelp');
  container.appendChild(help);

  const row = document.createElement('div');
  row.className = 'pairing-row';

  const input = document.createElement('input');
  input.className = 'pairing-input';
  input.type = 'text';
  input.placeholder = I18n.t('extensions.pairingCodePlaceholder');
  input.autocomplete = 'off';
  input.spellcheck = false;
  input.maxLength = 64;
  row.appendChild(input);

  const btn = document.createElement('button');
  btn.className = 'btn-ext activate';
  btn.textContent = I18n.t('extensions.claimPairingAction');
  btn.addEventListener('click', function() {
    approvePairing(ext.name, input.value, {
      onSuccess: function() {
        input.value = '';
      }
    });
  });
  row.appendChild(btn);

  input.addEventListener('keydown', function(event) {
    if (event.key === 'Enter') {
      event.preventDefault();
      btn.click();
    }
  });

  container.appendChild(row);

  if (info.restart_instructions) {
    const restart = document.createElement('div');
    restart.className = 'pairing-help pairing-restart';
    restart.textContent = info.restart_instructions;
    container.appendChild(restart);
  }
}

function approvePairing(channel, code, options) {
  options = options || {};
  const normalizedCode = (code || '').trim().toUpperCase();
  if (!normalizedCode) {
    const message = I18n.t('extensions.pairingCodeRequired');
    if (typeof options.onError === 'function') {
      options.onError(message);
    } else {
      showToast(message, 'error');
    }
    return Promise.resolve();
  }

  const card = getPairingCard(channel);
  const threadId = card ? card.getAttribute('data-thread-id') : null;
  const requestId = card ? card.getAttribute('data-request-id') : null;

  return apiFetch('/api/pairing/' + encodeURIComponent(channel) + '/approve', {
    method: 'POST',
    body: {
      code: normalizedCode,
      thread_id: threadId || currentThreadId || undefined,
      request_id: requestId || undefined,
    },
  }).then(res => {
    if (res.success) {
      _recentLocalPairingApprovals.set(channel, Date.now());
      if (!options.skipSuccessToast) {
        showToast(I18n.t('extensions.pairingApproved'), 'success');
      }
      if (typeof options.onSuccess === 'function') options.onSuccess(res);
      if (!options.skipRefresh && currentTab === 'settings') refreshCurrentSettingsTab();
    } else {
      const message = res.message || I18n.t('extensions.approveFailed');
      if (typeof options.onError === 'function') {
        options.onError(message);
      } else {
        showToast(message, 'error');
      }
    }
  }).catch(err => {
    const message = I18n.t('extensions.pairingError', { message: err.message });
    if (typeof options.onError === 'function') {
      options.onError(message);
    } else {
      showToast(message, 'error');
    }
  });
}

function startPairingPoll() {
  stopPairingPoll();
  pairingPollInterval = setInterval(function() {
    document.querySelectorAll('.ext-pairing[data-channel]').forEach(function(el) {
      loadPairingRequests(el.getAttribute('data-channel'), el, el.__onboarding || null);
    });
  }, 10000);
}

function stopPairingPoll() {
  if (pairingPollInterval) {
    clearInterval(pairingPollInterval);
    pairingPollInterval = null;
  }
}

// --- WASM channel stepper ---

function renderWasmChannelStepper(ext) {
  var stepper = document.createElement('div');
  stepper.className = 'ext-stepper';

  var status = ext.onboarding_state || ext.activation_status || 'installed';
  var requiresPairing = !!(ext.onboarding && ext.onboarding.requires_pairing);

  var steps = [
    { label: I18n.t('missions.stepConfigured'), key: 'setup_required' },
    { label: requiresPairing ? I18n.t('missions.stepAwaitingPairing') : I18n.t('extensions.activate'), key: 'pairing_required' },
    { label: I18n.t('missions.stepActive'), key: 'ready' },
  ];

  var reachedIdx;
  if (status === 'active' || status === 'ready') reachedIdx = 2;
  else if (status === 'pairing' || status === 'pairing_required') reachedIdx = 1;
  else if (status === 'failed') reachedIdx = 2;
  else if (status === 'configured' || status === 'activation_in_progress') reachedIdx = 1;
  else reachedIdx = 0;

  for (var i = 0; i < steps.length; i++) {
    if (i > 0) {
      var connector = document.createElement('div');
      connector.className = 'stepper-connector' + (i <= reachedIdx ? ' completed' : '');
      stepper.appendChild(connector);
    }

    var step = document.createElement('div');
    var stepState;
    if (i < reachedIdx) {
      stepState = 'completed';
    } else if (i === reachedIdx) {
      if (status === 'failed') {
        stepState = 'failed';
      } else if (status === 'pairing' || status === 'pairing_required' || status === 'activation_in_progress') {
        stepState = 'in-progress';
      } else if (status === 'setup_required') {
        stepState = 'in-progress';
      } else if (status === 'active' || status === 'ready' || status === 'configured' || status === 'installed') {
        stepState = 'completed';
      } else {
        stepState = 'pending';
      }
    } else {
      stepState = 'pending';
    }
    step.className = 'stepper-step ' + stepState;

    var circle = document.createElement('span');
    circle.className = 'stepper-circle';
    if (stepState === 'completed') circle.textContent = '\u2713';
    else if (stepState === 'failed') circle.textContent = '\u2717';
    step.appendChild(circle);

    var label = document.createElement('span');
    label.className = 'stepper-label';
    label.textContent = steps[i].label;
    step.appendChild(label);

    stepper.appendChild(step);
  }

  return stepper;
}

// --- Jobs ---

