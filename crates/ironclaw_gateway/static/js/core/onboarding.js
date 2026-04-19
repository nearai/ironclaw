function showApproval(data) {
  // Avoid duplicate cards on reconnect/history refresh.
  const existing = document.querySelector('.approval-card[data-request-id="' + CSS.escape(data.request_id) + '"]');
  if (existing) return;

  const container = document.getElementById('chat-messages');
  const cardThreadId = data.thread_id || currentThreadId;

  // Outer wrapper with avatar
  const wrapper = document.createElement('div');
  wrapper.className = 'gw-msg gw-msg--assistant';

  const avatar = document.createElement('div');
  avatar.className = 'gw-msg__avatar gw-msg__avatar--warn';
  avatar.textContent = '!';

  const msgContent = document.createElement('div');
  msgContent.className = 'gw-msg__content';

  const card = document.createElement('div');
  card.className = 'approval-card gw-approval';
  card.setAttribute('data-request-id', data.request_id);
  if (cardThreadId) {
    card.setAttribute('data-thread-id', cardThreadId);
  }

  // Header with title and tag
  const header = document.createElement('div');
  header.className = 'gw-approval__header';
  const title = document.createElement('span');
  title.className = 'gw-approval__title';
  title.textContent = humanizeToolName(data.tool_name);
  header.appendChild(title);
  const tag = document.createElement('span');
  tag.className = 'gw-approval__tag';
  tag.textContent = I18n.t('approval.tag');
  header.appendChild(tag);
  card.appendChild(header);

  if (data.description) {
    const desc = document.createElement('div');
    desc.className = 'gw-approval__summary';
    desc.textContent = data.description;
    card.appendChild(desc);
  }

  // Detail grid for parameters
  if (data.parameters) {
    const details = document.createElement('dl');
    details.className = 'gw-approval__details';
    const dt = document.createElement('dt');
    dt.textContent = 'params';
    const dd = document.createElement('dd');
    const code = document.createElement('code');
    code.textContent = data.parameters.length > 200
      ? data.parameters.substring(0, 200) + '...'
      : data.parameters;
    dd.appendChild(code);
    details.appendChild(dt);
    details.appendChild(dd);
    card.appendChild(details);
  }

  // Actions: Deny (ghost), Approve once (secondary), Always allow (primary)
  const actions = document.createElement('div');
  actions.className = 'approval-actions gw-approval__actions';

  const denyBtn = document.createElement('button');
  denyBtn.className = 'gw-btn gw-btn--ghost gw-btn--sm deny';
  denyBtn.textContent = I18n.t('approval.deny');
  denyBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'deny', cardThreadId));

  const approveBtn = document.createElement('button');
  approveBtn.className = 'gw-btn gw-btn--secondary gw-btn--sm approve';
  approveBtn.textContent = I18n.t('approval.approveOnce');
  approveBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'approve', cardThreadId));

  actions.appendChild(denyBtn);
  actions.appendChild(approveBtn);

  if (data.allow_always !== false) {
    const alwaysBtn = document.createElement('button');
    alwaysBtn.className = 'gw-btn gw-btn--primary gw-btn--sm always';
    alwaysBtn.textContent = I18n.t('approval.alwaysAllow');
    alwaysBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'always', cardThreadId));
    actions.appendChild(alwaysBtn);
  }

  card.appendChild(actions);
  msgContent.appendChild(card);
  wrapper.appendChild(avatar);
  wrapper.appendChild(msgContent);

  container.appendChild(wrapper);
  container.scrollTop = container.scrollHeight;
}

// --- Plan Checklist ---

function renderPlanChecklist(data) {
  const chatContainer = document.getElementById('chat-messages');
  const planId = data.plan_id;

  // Find or create the plan container
  let container = chatContainer.querySelector('.plan-container[data-plan-id="' + CSS.escape(planId) + '"]');
  if (!container) {
    container = document.createElement('div');
    container.className = 'plan-container';
    container.setAttribute('data-plan-id', planId);
    chatContainer.appendChild(container);
  }

  // Clear and rebuild
  container.innerHTML = '';

  // Header
  const header = document.createElement('div');
  header.className = 'plan-header';

  const title = document.createElement('span');
  title.className = 'plan-title';
  title.textContent = data.title || planId;
  header.appendChild(title);

  const badge = document.createElement('span');
  badge.className = 'plan-status-badge plan-status-' + (data.status || 'draft');
  badge.textContent = data.status || 'draft';
  header.appendChild(badge);

  container.appendChild(header);

  // Steps
  if (data.steps && data.steps.length > 0) {
    const stepsList = document.createElement('div');
    stepsList.className = 'plan-steps';

    let completed = 0;
    for (const step of data.steps) {
      const stepEl = document.createElement('div');
      stepEl.className = 'plan-step';
      stepEl.setAttribute('data-status', step.status || 'pending');

      const icon = document.createElement('span');
      icon.className = 'plan-step-icon';
      if (step.status === 'completed') {
        icon.textContent = '\u2713'; // checkmark
        completed++;
      } else if (step.status === 'failed') {
        icon.textContent = '\u2717'; // X
      } else if (step.status === 'in_progress') {
        icon.innerHTML = '<span class="plan-spinner"></span>';
      } else {
        icon.textContent = '\u25CB'; // circle
      }
      stepEl.appendChild(icon);

      const text = document.createElement('span');
      text.className = 'plan-step-text';
      text.textContent = step.title;
      stepEl.appendChild(text);

      if (step.result) {
        const result = document.createElement('span');
        result.className = 'plan-step-result';
        result.textContent = step.result;
        stepEl.appendChild(result);
      }

      stepsList.appendChild(stepEl);
    }
    container.appendChild(stepsList);

    // Summary
    const summary = document.createElement('div');
    summary.className = 'plan-summary';
    summary.textContent = completed + ' of ' + data.steps.length + ' steps completed';
    if (data.mission_id) {
      summary.textContent += ' \u00b7 Mission: ' + data.mission_id.substring(0, 8);
    }
    container.appendChild(summary);
  }

  chatContainer.scrollTop = chatContainer.scrollHeight;
}

function showJobCard(data) {
  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'job-card';

  const icon = document.createElement('span');
  icon.className = 'job-card-icon';
  icon.textContent = '\u2692';
  card.appendChild(icon);

  const info = document.createElement('div');
  info.className = 'job-card-info';

  const title = document.createElement('div');
  title.className = 'job-card-title';
  title.textContent = data.title || I18n.t('sandbox.job');
  info.appendChild(title);

  const id = document.createElement('div');
  id.className = 'job-card-id';
  id.textContent = (data.job_id || '').substring(0, 8);
  info.appendChild(id);

  card.appendChild(info);

  const viewBtn = document.createElement('button');
  viewBtn.className = 'job-card-view';
  viewBtn.textContent = I18n.t('jobs.viewJob');
  viewBtn.addEventListener('click', () => {
    switchTab('jobs');
    openJobDetail(data.job_id);
  });
  card.appendChild(viewBtn);

  if (data.browse_url) {
    const browseBtn = document.createElement('a');
    browseBtn.className = 'job-card-browse';
    browseBtn.href = data.browse_url;
    browseBtn.target = '_blank';
    browseBtn.rel = 'noopener noreferrer';
    browseBtn.textContent = I18n.t('jobs.browse');
    card.appendChild(browseBtn);
  }

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
}

// --- Auth card ---

function handleAuthRequired(data) {
  const shouldBlockChat = data.block_chat !== false;
  if (data.thread_id && !isCurrentThread(data.thread_id)) {
    unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
    debouncedLoadThreads();
    return;
  }
  if (data.extension_name && getConfigureOverlay(data.extension_name)) {
    if (shouldBlockChat) setAuthFlowPending(true, data.instructions);
    return;
  }
  if (shouldBlockChat) setAuthFlowPending(true, data.instructions);
  if (data.auth_url || !data.extension_name || !data.request_id) {
    showAuthCard(data);
  } else {
    if (getConfigureOverlay(data.extension_name)) return;
    showSetupCardForExtension(data);
  }
}

function handleOnboardingState(data) {
  if (data.thread_id && !isCurrentThread(data.thread_id)) {
    if (data.state === 'auth_required' || data.state === 'setup_required' || data.state === 'pairing_required') {
      unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
    }
    debouncedLoadThreads();
    return;
  }

  if (data.state === 'auth_required') {
    handleAuthRequired({
      extension_name: data.extension_name,
      display_name: data.display_name || data.extension_name,
      request_id: data.request_id || null,
      instructions: data.instructions,
      auth_url: data.auth_url || null,
      setup_url: data.setup_url || null,
      thread_id: data.thread_id || currentThreadId,
    });
    return;
  }

  if (data.state === 'setup_required') {
    setAuthFlowPending(true, data.instructions || null);
    showSetupCardForExtension({
      extension_name: data.extension_name,
      display_name: data.display_name || data.extension_name,
      instructions: data.instructions,
      auth_url: data.auth_url || null,
      setup_url: data.setup_url || null,
      onboarding: data.onboarding || null,
      thread_id: data.thread_id || currentThreadId,
    });
    return;
  }

  if (data.state === 'pairing_required') {
    removeAuthCard(data.extension_name);
    removeSetupCard(data.extension_name);
    closeConfigureModal(data.extension_name);
    showPairingCard({
      channel: data.extension_name,
      request_id: data.request_id || null,
      instructions: data.instructions,
      onboarding: data.onboarding || null,
      thread_id: data.thread_id || currentThreadId,
    });
    if (currentTab === 'settings') refreshCurrentSettingsTab();
    return;
  }

  if (data.state === 'ready' || data.state === 'failed') {
    const recentPairingApprovalAt = _recentLocalPairingApprovals.get(data.extension_name);
    const skipToast = !!recentPairingApprovalAt
      && data.state === 'ready'
      && Date.now() - recentPairingApprovalAt <= 5000;
    if (data.message && !skipToast) {
      showToast(data.message, data.state === 'ready' ? 'success' : 'error');
    }
    _recentLocalPairingApprovals.delete(data.extension_name);
    removePairingCard(data.extension_name);
    removeAuthCard(data.extension_name);
    removeSetupCard(data.extension_name);
    closeConfigureModal(data.extension_name);
    setAuthFlowPending(false);
    if (data.state === 'ready' && shouldShowChannelConnectedMessage(data.extension_name, true)) {
      addMessage('system', `${data.display_name || data.extension_name} is now connected.`);
    }
    if (currentTab === 'settings') refreshCurrentSettingsTab();
    enableChatInput();
  }
}

function parseGateResumeKind(resumeKind) {
  if (!resumeKind || typeof resumeKind !== 'object') return null;
  if (resumeKind.Approval) return { type: 'approval', ...resumeKind.Approval };
  if (resumeKind.Authentication) return { type: 'authentication', ...resumeKind.Authentication };
  if (resumeKind.External) return { type: 'external', ...resumeKind.External };
  return null;
}

function handleGateRequired(data) {
  const hasThread = !!data.thread_id;
  const forCurrentThread = !hasThread || isCurrentThread(data.thread_id);
  const resume = parseGateResumeKind(data.resume_kind);
  if (!forCurrentThread) {
    unreadThreads.set(data.thread_id, (unreadThreads.get(data.thread_id) || 0) + 1);
    debouncedLoadThreads();
    return;
  }
  if (resume && resume.type === 'authentication') {
    handleOnboardingState({
      state: 'auth_required',
      extension_name: data.extension_name || null,
      display_name: data.display_name || data.extension_name || resume.credential_name,
      request_id: data.request_id,
      instructions: resume.instructions,
      auth_url: resume.auth_url || null,
      thread_id: data.thread_id || currentThreadId,
    });
    return;
  }
  showApproval({
    request_id: data.request_id,
    tool_name: data.tool_name,
    description: data.description,
    parameters: data.parameters,
    allow_always: !(resume && resume.type === 'approval' && resume.allow_always === false),
    thread_id: data.thread_id || currentThreadId,
  });
}

function handleGateResolved(data) {
  const hasThread = !!data.thread_id;
  if (hasThread && !isCurrentThread(data.thread_id)) {
    debouncedLoadThreads();
    return;
  }
  document.querySelectorAll('.approval-card[data-request-id="' + CSS.escape(data.request_id) + '"]').forEach((el) => el.remove());
  if (
    data.resolution === 'credential_provided'
    || data.resolution === 'cancelled'
    || data.resolution === 'external_callback'
  ) {
    removeAuthCard();
    enableChatInput();
  }
}

function queryByDataAttribute(selector, attributeName, attributeValue) {
  if (typeof attributeValue !== 'string') return document.querySelector(selector);

  if (window.CSS && typeof window.CSS.escape === 'function') {
    return document.querySelector(
      selector + '[' + attributeName + '="' + window.CSS.escape(attributeValue) + '"]'
    );
  }

  const candidates = document.querySelectorAll(selector);
  for (const candidate of candidates) {
    if (candidate.getAttribute(attributeName) === attributeValue) return candidate;
  }
  return null;
}

function getAuthOverlay(extensionName) {
  return queryByDataAttribute('.auth-overlay', 'data-extension-name', extensionName);
}

function getAuthCard(extensionName) {
  return queryByDataAttribute('.auth-card', 'data-extension-name', extensionName);
}

function getPairingCard(channel) {
  return queryByDataAttribute('.pairing-card', 'data-channel', channel);
}

function getConfigureOverlay(extensionName) {
  return queryByDataAttribute('.configure-overlay', 'data-extension-name', extensionName);
}

function removeSetupCard(extensionName) {
  removeAuthCard(extensionName);
}

function buildSetupFields(form, extensionName, secrets, submitFn) {
  const fields = [];
  (secrets || []).forEach((secret) => {
    const field = document.createElement('label');
    field.className = 'setup-field';

    const label = document.createElement('span');
    label.className = 'setup-label';
    label.textContent = secret.prompt;
    field.appendChild(label);

    const inputRow = document.createElement('div');
    inputRow.className = 'setup-input-row';

    const input = document.createElement('input');
    input.className = 'setup-input';
    input.type = 'password';
    input.name = secret.name;
    input.placeholder = secret.provided ? I18n.t('config.alreadySet') : secret.prompt;
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submitFn();
    });
    inputRow.appendChild(input);
    field.appendChild(inputRow);
    form.appendChild(field);
    fields.push({ name: secret.name, input });
  });
  return fields;
}

function showSetupCardForExtension(data) {
  // Dedup: don't open if a configure modal is already showing for this extension
  if (getConfigureOverlay(data.extension_name)) return;
  showConfigureModal(data.extension_name, { authData: data });
}

function showAuthCard(data) {
  if (data.thread_id && !isCurrentThread(data.thread_id)) return;
  // Keep a single global auth prompt so the experience is consistent across tabs.
  const existing = getAuthOverlay();
  if (existing) existing.remove();
  // Temporary compatibility boundary: legacy web auth prompts (engine v1
  // `pending_auth`) do not carry a gate `request_id`, but they still need
  // manual token entry until that path is retired. Real v2 gates keep using
  // `/api/chat/gate/resolve`.
  const allowTokenSubmit = !data.auth_url;
  const displayName = data.display_name || data.extension_name || 'this integration';

  const overlay = document.createElement('div');
  overlay.className = 'auth-overlay';
  if (data.extension_name) {
    overlay.setAttribute('data-extension-name', data.extension_name);
  }
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) cancelAuth(data.extension_name);
  });

  const card = document.createElement('div');
  card.className = 'auth-card auth-modal';
  if (data.extension_name) {
    card.setAttribute('data-extension-name', data.extension_name);
  }
  if (data.thread_id) {
    card.setAttribute('data-thread-id', data.thread_id);
  }
  if (data.request_id) {
    card.setAttribute('data-request-id', data.request_id);
  }

  const header = document.createElement('div');
  header.className = 'auth-header';
  header.textContent = I18n.t('authRequired.title', {name: displayName});
  card.appendChild(header);

  if (data.instructions) {
    const instr = document.createElement('div');
    instr.className = 'auth-instructions';
    instr.textContent = data.instructions;
    card.appendChild(instr);
  }

  const links = document.createElement('div');
  links.className = 'auth-links';

  if (data.auth_url) {
    const parsedAuthUrl = parseHttpsOAuthUrl(data.auth_url);
    if (parsedAuthUrl) {
      const oauthLink = document.createElement('a');
      oauthLink.className = 'auth-oauth';
      oauthLink.href = parsedAuthUrl.href;
      oauthLink.target = '_blank';
      // Match the other external links: include `noreferrer` so the
      // OAuth provider does not see the in-app Referer header.
      oauthLink.rel = 'noopener noreferrer';
      oauthLink.textContent = I18n.t('authRequired.authenticateWith', {name: displayName});
      oauthLink.setAttribute('aria-label', 'Authenticate with ' + displayName + ' in a new tab');
      oauthLink.title = 'Opens authentication in a new tab';
      links.appendChild(oauthLink);
    }
  }

  if (data.setup_url) {
    const parsedSetupUrl = parseHttpsExternalUrl(data.setup_url, 'setup');
    if (parsedSetupUrl) {
      const setupLink = document.createElement('a');
      setupLink.className = 'auth-setup-link';
      setupLink.href = parsedSetupUrl.href;
      setupLink.target = '_blank';
      setupLink.rel = 'noopener noreferrer';
      setupLink.textContent = I18n.t('authRequired.getToken');
      setupLink.setAttribute('aria-label', 'Open token setup instructions for ' + displayName + ' in a new tab');
      setupLink.title = 'Opens setup instructions in a new tab';
      links.appendChild(setupLink);
    }
  }

  if (links.children.length > 0) {
    card.appendChild(links);
  }

  let tokenInput = null;
  if (allowTokenSubmit) {
    const tokenRow = document.createElement('div');
    tokenRow.className = 'auth-token-input';

    tokenInput = document.createElement('input');
    tokenInput.type = 'password';
    tokenInput.placeholder = data.instructions
      || I18n.t('auth.tokenPlaceholder');
    tokenInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submitAuthToken(data.extension_name, tokenInput.value);
    });
    tokenRow.appendChild(tokenInput);
    card.appendChild(tokenRow);
  }

  // Error display (hidden initially)
  const errorEl = document.createElement('div');
  errorEl.className = 'auth-error';
  errorEl.style.display = 'none';
  card.appendChild(errorEl);

  // Action buttons
  const actions = document.createElement('div');
  actions.className = 'auth-actions';

  if (allowTokenSubmit) {
    const submitBtn = document.createElement('button');
    submitBtn.className = 'auth-submit';
    submitBtn.textContent = I18n.t('btn.submit');
    submitBtn.addEventListener('click', () => submitAuthToken(data.extension_name, tokenInput.value));
    actions.appendChild(submitBtn);
  }

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'auth-cancel';
  cancelBtn.textContent = allowTokenSubmit ? I18n.t('btn.cancel') : I18n.t('common.close');
  cancelBtn.addEventListener('click', () => cancelAuth(data.extension_name));
  actions.appendChild(cancelBtn);
  card.appendChild(actions);

  overlay.appendChild(card);
  document.body.appendChild(overlay);
  if (tokenInput) tokenInput.focus();
}

function removeAuthCard(extensionName) {
  const overlay = getAuthOverlay(extensionName);
  if (overlay) {
    overlay.remove();
    return;
  }
  const card = getAuthCard(extensionName);
  if (card) {
    const parentOverlay = card.closest('.auth-overlay');
    if (parentOverlay) parentOverlay.remove();
    else card.remove();
  }
}

function showPairingCard(data) {
  if (data.thread_id && !isCurrentThread(data.thread_id)) return;
  removePairingCard(data.channel);

  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'auth-card pairing-card';
  card.setAttribute('data-channel', data.channel);
  if (data.request_id) {
    card.setAttribute('data-request-id', data.request_id);
  }
  if (data.thread_id) {
    card.setAttribute('data-thread-id', data.thread_id);
  }

  const header = document.createElement('div');
  header.className = 'auth-header';
  header.textContent = (data.onboarding && data.onboarding.pairing_title) || ('Claim ownership for ' + data.channel);
  card.appendChild(header);

  const instr = document.createElement('div');
  instr.className = 'auth-instructions';
  instr.textContent = (data.onboarding && data.onboarding.pairing_instructions)
    || data.instructions
    || ('Paste the pairing code from ' + data.channel + '.');
  card.appendChild(instr);

  if (data.onboarding && data.onboarding.restart_instructions) {
    const restart = document.createElement('div');
    restart.className = 'setup-next-step pairing-restart';
    restart.textContent = data.onboarding.restart_instructions;
    card.appendChild(restart);
  }

  const inputRow = document.createElement('div');
  inputRow.className = 'auth-token-input';

  const codeInput = document.createElement('input');
  codeInput.type = 'text';
  codeInput.placeholder = I18n.t('extensions.pairingCodePlaceholder');
  codeInput.autocomplete = 'off';
  codeInput.spellcheck = false;
  codeInput.autocapitalize = 'characters';
  codeInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') submitPairingCode(data.channel, codeInput.value, card);
  });
  inputRow.appendChild(codeInput);
  card.appendChild(inputRow);

  const errorEl = document.createElement('div');
  errorEl.className = 'auth-error';
  errorEl.style.display = 'none';
  card.appendChild(errorEl);

  const actions = document.createElement('div');
  actions.className = 'auth-actions';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'auth-submit pairing-submit';
  submitBtn.textContent = I18n.t('approval.approve');
  submitBtn.addEventListener('click', () => submitPairingCode(data.channel, codeInput.value, card));

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'auth-cancel pairing-cancel';
  cancelBtn.textContent = I18n.t('btn.cancel');
  cancelBtn.addEventListener('click', () => cancelPairingCard(data.channel, data.onboarding));

  actions.appendChild(submitBtn);
  actions.appendChild(cancelBtn);
  card.appendChild(actions);

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
  codeInput.focus();
}

function cancelPairingCard(channel, onboarding) {
  removePairingCard(channel);
  showToast(
    (onboarding && onboarding.restart_instructions) || I18n.t('extensions.pairingRestartHint'),
    'info'
  );
}

function removePairingCard(channel) {
  const card = getPairingCard(channel);
  if (card) card.remove();
}

function showPairingCardError(channel, message) {
  const card = getPairingCard(channel);
  if (!card) return;
  card.querySelectorAll('button').forEach((btn) => {
    btn.disabled = false;
  });
  const errorEl = card.querySelector('.auth-error');
  if (errorEl) {
    errorEl.textContent = message;
    errorEl.style.display = 'block';
  }
}

function submitPairingCode(channel, codeValue, cardEl) {
  approvePairing(channel, codeValue, {
    skipSuccessToast: true,
    skipRefresh: true,
    onSuccess: function() {
      removePairingCard(channel);
    },
    onError: function(message) {
      showPairingCardError(channel, message);
      const card = cardEl || getPairingCard(channel);
      if (card) {
        const input = card.querySelector('.auth-token-input input');
        if (input) input.focus();
      }
    }
  });
}

function submitAuthToken(extensionName, tokenValue) {
  if (!tokenValue || !tokenValue.trim()) return;

  // Disable submit button while in flight
  const card = getAuthCard(extensionName);
  const threadId = card ? card.getAttribute('data-thread-id') : null;
  if (card) {
    const btns = card.querySelectorAll('button');
    btns.forEach((b) => { b.disabled = true; });
  }

  const requestId = card ? card.getAttribute('data-request-id') : null;
  const targetThreadId = threadId || currentThreadId || undefined;
  // Keep the v1 fallback scoped to prompts without a gate request id. This is
  // the only browser-side compatibility shim we want left once v1 auth mode is
  // removed.
  const request = requestId
    ? apiFetch('/api/chat/gate/resolve', {
      method: 'POST',
      body: {
        request_id: requestId,
        thread_id: targetThreadId,
        resolution: 'credential_provided',
        token: tokenValue.trim(),
      },
    })
    : apiFetch('/api/chat/auth-token', {
      method: 'POST',
      body: {
        token: tokenValue.trim(),
        thread_id: targetThreadId,
      },
    });

  request.then((result) => {
    if (result.success) {
      // Close immediately for responsiveness; the authoritative success UX
      // (toast + settings refresh) still comes from the onboarding_state SSE.
      removeAuthCard(extensionName);
      enableChatInput();
    } else {
      showAuthCardError(extensionName, result.message);
    }
  }).catch((err) => {
    showAuthCardError(extensionName, 'Failed: ' + err.message);
  });
}

function cancelAuth(extensionName) {
  const card = getAuthCard(extensionName);
  const threadId = card ? card.getAttribute('data-thread-id') : null;
  const requestId = card ? card.getAttribute('data-request-id') : null;
  requestAuthCancellation(requestId, threadId).catch(() => {});
  removeAuthCard(extensionName);
  setAuthFlowPending(false);
  enableChatInput();
}

function showAuthCardError(extensionName, message) {
  const card = getAuthCard(extensionName);
  if (!card) return;
  // Re-enable buttons
  const btns = card.querySelectorAll('button');
  btns.forEach((b) => { b.disabled = false; });
  // Show error
  const errorEl = card.querySelector('.auth-error');
  if (errorEl) {
    errorEl.textContent = message;
    errorEl.style.display = 'block';
  }
}

function setAuthFlowPending(pending, instructions) {
  authFlowPending = !!pending;
  const input = document.getElementById('chat-input');
  const btn = document.getElementById('send-btn');
  if (!input || !btn) return;
  if (authFlowPending) {
    input.disabled = true;
    btn.disabled = true;
    return;
  }
  if (!currentThreadIsReadOnly) {
    input.disabled = false;
    btn.disabled = false;
  }
}

