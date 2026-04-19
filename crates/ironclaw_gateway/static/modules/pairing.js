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
