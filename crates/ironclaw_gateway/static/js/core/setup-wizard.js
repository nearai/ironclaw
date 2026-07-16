// --- Guided setup wizard ---
//
// A three-step path from "fresh agent" to "working integration":
//   1. Pick a use case (curated, from NUX_DATA)
//   2. Connect a channel (real install/configure flows via /api/extensions)
//   3. Start the first conversation (chat input pre-filled with the
//      use case's starter prompt)
//
// The wizard intentionally reuses the existing extension lifecycle
// (`/api/extensions/install`, showConfigureModal, pairing/onboarding SSE)
// rather than duplicating it — it is a guided front door to the same flows
// that live under Settings.

let _nuxWizardState = null;

function openNuxSetupWizard() {
  closeNuxSetupWizard();

  _nuxWizardState = {
    step: 0,
    useCase: null,
    channels: [],
    refreshTimer: null,
  };

  const overlay = document.createElement('div');
  overlay.className = 'nux-wizard-overlay';
  overlay.id = 'nux-wizard-overlay';
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) closeNuxSetupWizard();
  });

  const modal = document.createElement('div');
  modal.className = 'nux-wizard';
  modal.setAttribute('role', 'dialog');
  modal.setAttribute('aria-modal', 'true');
  modal.setAttribute('aria-labelledby', 'nux-wizard-title');

  const header = document.createElement('div');
  header.className = 'nux-wizard-header';

  const steps = document.createElement('div');
  steps.className = 'nux-wizard-steps';
  steps.id = 'nux-wizard-steps';
  header.appendChild(steps);

  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'nux-wizard-close';
  closeBtn.setAttribute('aria-label', I18n.t('common.close'));
  closeBtn.textContent = '\u00d7';
  closeBtn.addEventListener('click', closeNuxSetupWizard);
  header.appendChild(closeBtn);

  modal.appendChild(header);

  const body = document.createElement('div');
  body.className = 'nux-wizard-body';
  body.id = 'nux-wizard-body';
  modal.appendChild(body);

  const footer = document.createElement('div');
  footer.className = 'nux-wizard-footer';
  footer.id = 'nux-wizard-footer';
  modal.appendChild(footer);

  overlay.appendChild(modal);
  document.body.appendChild(overlay);

  renderNuxWizardStep();
}

function closeNuxSetupWizard() {
  if (_nuxWizardState && _nuxWizardState.refreshTimer) {
    clearInterval(_nuxWizardState.refreshTimer);
  }
  _nuxWizardState = null;
  const overlay = document.getElementById('nux-wizard-overlay');
  if (overlay) overlay.remove();
}

const NUX_WIZARD_STEP_KEYS = ['nux.stepUseCase', 'nux.stepConnect', 'nux.stepChat'];

function renderNuxWizardStepIndicator() {
  const steps = document.getElementById('nux-wizard-steps');
  if (!steps || !_nuxWizardState) return;
  steps.innerHTML = '';
  NUX_WIZARD_STEP_KEYS.forEach((key, i) => {
    const dot = document.createElement('span');
    dot.className = 'nux-wizard-step'
      + (i === _nuxWizardState.step ? ' current' : '')
      + (i < _nuxWizardState.step ? ' done' : '');
    const num = document.createElement('span');
    num.className = 'nux-wizard-step-num';
    num.textContent = i < _nuxWizardState.step ? '\u2713' : String(i + 1);
    dot.appendChild(num);
    const label = document.createElement('span');
    label.className = 'nux-wizard-step-label';
    label.textContent = I18n.t(key);
    dot.appendChild(label);
    steps.appendChild(dot);
  });
}

function renderNuxWizardStep() {
  if (!_nuxWizardState) return;
  renderNuxWizardStepIndicator();
  const body = document.getElementById('nux-wizard-body');
  const footer = document.getElementById('nux-wizard-footer');
  if (!body || !footer) return;
  body.innerHTML = '';
  footer.innerHTML = '';

  if (_nuxWizardState.refreshTimer) {
    clearInterval(_nuxWizardState.refreshTimer);
    _nuxWizardState.refreshTimer = null;
  }

  if (_nuxWizardState.step === 0) renderNuxWizardUseCaseStep(body, footer);
  else if (_nuxWizardState.step === 1) renderNuxWizardConnectStep(body, footer);
  else renderNuxWizardChatStep(body, footer);
}

function nuxWizardFooterButton(label, className, onClick) {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = className;
  btn.textContent = label;
  btn.addEventListener('click', onClick);
  return btn;
}

function nuxWizardGoTo(step) {
  if (!_nuxWizardState) return;
  _nuxWizardState.step = step;
  renderNuxWizardStep();
}

// --- Step 1: pick a use case ---

function renderNuxWizardUseCaseStep(body, footer) {
  const title = document.createElement('h2');
  title.className = 'nux-wizard-title';
  title.id = 'nux-wizard-title';
  title.textContent = I18n.t('nux.useCaseTitle');
  body.appendChild(title);

  const subtitle = document.createElement('p');
  subtitle.className = 'nux-wizard-subtitle';
  subtitle.textContent = I18n.t('nux.useCaseSubtitle');
  body.appendChild(subtitle);

  const grid = document.createElement('div');
  grid.className = 'nux-usecase-grid';

  (NUX_DATA.useCases || []).forEach((useCase) => {
    const card = document.createElement('button');
    card.type = 'button';
    card.className = 'nux-usecase-card'
      + (_nuxWizardState.useCase && _nuxWizardState.useCase.id === useCase.id ? ' selected' : '');

    const glyph = document.createElement('span');
    glyph.className = 'nux-usecase-glyph';
    glyph.setAttribute('aria-hidden', 'true');
    glyph.textContent = useCase.glyph;
    card.appendChild(glyph);

    const text = document.createElement('span');
    text.className = 'nux-usecase-text';
    const titleEl = document.createElement('span');
    titleEl.className = 'nux-usecase-title';
    titleEl.textContent = useCase.title;
    text.appendChild(titleEl);
    const descEl = document.createElement('span');
    descEl.className = 'nux-usecase-desc';
    descEl.textContent = useCase.description;
    text.appendChild(descEl);
    card.appendChild(text);

    card.addEventListener('click', () => {
      _nuxWizardState.useCase =
        _nuxWizardState.useCase && _nuxWizardState.useCase.id === useCase.id ? null : useCase;
      nuxWizardGoTo(_nuxWizardState.useCase ? 1 : 0);
    });

    grid.appendChild(card);
  });

  body.appendChild(grid);

  footer.appendChild(nuxWizardFooterButton(
    I18n.t('nux.skipStep'), 'btn-secondary', () => nuxWizardGoTo(1)
  ));
}

// --- Step 2: connect a channel ---

function nuxChannelStatus(installedExt) {
  if (!installedExt) return 'available';
  const state = installedExt.onboarding_state || installedExt.activation_status || 'installed';
  if (state === 'active' || state === 'ready') return 'connected';
  if (state === 'pairing' || state === 'pairing_required') return 'pairing';
  return 'setup';
}

function loadNuxWizardChannels() {
  return Promise.all([
    apiFetch('/api/extensions').catch(() => ({ extensions: [] })),
    apiFetch('/api/extensions/registry').catch(() => ({ entries: [] })),
  ]).then(([extData, registryData]) => {
    const installedByName = new Map();
    (extData.extensions || []).forEach((ext) => installedByName.set(ext.name, ext));
    const registryByName = new Map();
    (registryData.entries || []).forEach((entry) => registryByName.set(entry.name, entry));

    return (NUX_DATA.setupChannels || [])
      .filter((ch) => installedByName.has(ch.name) || registryByName.has(ch.name))
      .map((ch) => {
        const installed = installedByName.get(ch.name) || null;
        const registry = registryByName.get(ch.name) || null;
        return {
          name: ch.name,
          label: ch.label,
          blurb: ch.blurb,
          icon: ch.icon,
          kind: (registry && registry.kind) || (installed && installed.kind) || 'wasm_channel',
          installed,
          status: nuxChannelStatus(installed),
        };
      });
  });
}

function renderNuxWizardConnectStep(body, footer) {
  const title = document.createElement('h2');
  title.className = 'nux-wizard-title';
  title.id = 'nux-wizard-title';
  title.textContent = I18n.t('nux.connectTitle');
  body.appendChild(title);

  const subtitle = document.createElement('p');
  subtitle.className = 'nux-wizard-subtitle';
  subtitle.textContent = I18n.t('nux.connectSubtitle');
  body.appendChild(subtitle);

  const list = document.createElement('div');
  list.className = 'nux-channel-list';
  list.innerHTML = '<div class="empty-state">' + I18n.t('common.loading') + '</div>';
  body.appendChild(list);

  const hasConnected = () =>
    _nuxWizardState && _nuxWizardState.channels.some((ch) => ch.status === 'connected');
  const refresh = () => {
    loadNuxWizardChannels().then((channels) => {
      if (!_nuxWizardState || _nuxWizardState.step !== 1) return;
      _nuxWizardState.channels = channels;
      renderNuxWizardChannelList(list, channels);
      // Relabel the footer button: a channel may have connected via the
      // poll (e.g. OAuth completed in another tab) after first render.
      const btn = document.getElementById('nux-wizard-continue');
      if (btn) {
        btn.textContent = hasConnected()
          ? I18n.t('common.continue')
          : I18n.t('nux.skipStep');
      }
    });
  };
  refresh();
  // Setup/pairing completes out-of-band (configure modal, OAuth tab, DM
  // pairing) — poll while this step is visible so status flips to Connected
  // without a manual refresh. Mirrors the Settings pairing poll cadence.
  _nuxWizardState.refreshTimer = setInterval(refresh, 4000);

  footer.appendChild(nuxWizardFooterButton(
    I18n.t('common.back'), 'btn-secondary', () => nuxWizardGoTo(0)
  ));
  const continueBtn = nuxWizardFooterButton(
    hasConnected() ? I18n.t('common.continue') : I18n.t('nux.skipStep'),
    'btn-primary',
    () => nuxWizardGoTo(2)
  );
  continueBtn.id = 'nux-wizard-continue';
  footer.appendChild(continueBtn);
}

function renderNuxWizardChannelList(list, channels) {
  list.innerHTML = '';
  if (channels.length === 0) {
    list.innerHTML = '<div class="empty-state">' + I18n.t('nux.noChannels') + '</div>';
    return;
  }

  channels.forEach((channel) => {
    const row = document.createElement('div');
    row.className = 'nux-channel-row';

    // Provider app mark (committed asset) next to the channel name.
    if (channel.icon) {
      const icon = document.createElement('img');
      icon.className = 'nux-channel-icon';
      icon.src = channel.icon;
      icon.alt = '';
      icon.setAttribute('aria-hidden', 'true');
      row.appendChild(icon);
    }

    const info = document.createElement('div');
    info.className = 'nux-channel-info';
    const label = document.createElement('div');
    label.className = 'nux-channel-label';
    label.textContent = channel.label;
    if (channel.status === 'connected') {
      const badge = document.createElement('span');
      badge.className = 'nux-channel-badge connected';
      badge.textContent = I18n.t('nux.connected');
      label.appendChild(badge);
    } else if (channel.status === 'pairing') {
      const badge = document.createElement('span');
      badge.className = 'nux-channel-badge pairing';
      badge.textContent = I18n.t('status.awaitingPairing');
      label.appendChild(badge);
    }
    info.appendChild(label);
    const blurb = document.createElement('div');
    blurb.className = 'nux-channel-blurb';
    blurb.textContent = channel.blurb;
    info.appendChild(blurb);
    row.appendChild(info);

    const action = document.createElement('div');
    action.className = 'nux-channel-action';
    if (channel.status === 'connected') {
      const check = document.createElement('span');
      check.className = 'nux-channel-check';
      check.textContent = '\u2713';
      action.appendChild(check);
    } else if (channel.status === 'available') {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'btn-primary nux-channel-btn';
      btn.textContent = I18n.t('nux.connect');
      btn.addEventListener('click', () => installNuxWizardChannel(channel, btn));
      action.appendChild(btn);
    } else {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'btn-secondary nux-channel-btn';
      btn.textContent = channel.status === 'pairing'
        ? I18n.t('nux.enterPairingCode')
        : I18n.t('nux.finishSetup');
      btn.addEventListener('click', () => showConfigureModal(channel.name));
      action.appendChild(btn);
    }
    row.appendChild(action);

    list.appendChild(row);
  });
}

function installNuxWizardChannel(channel, btn) {
  btn.disabled = true;
  btn.textContent = I18n.t('extensions.installing');
  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: channel.name, kind: channel.kind },
  }).then((res) => {
    if (res.success) {
      if (res.auth_url) {
        showToast(I18n.t('extensions.openingAuth', { name: channel.label }), 'info');
        openOAuthUrl(res.auth_url);
      } else {
        // WASM channels need credentials right after install — reuse the
        // canonical configure modal (same one Settings uses).
        showConfigureModal(channel.name);
      }
    } else {
      showToast(I18n.t('extensions.installFailed', { message: res.message || 'unknown error' }), 'error');
    }
    nuxWizardRefreshConnectStep();
  }).catch((err) => {
    showToast(I18n.t('extensions.installFailed', { message: err.message }), 'error');
    nuxWizardRefreshConnectStep();
  });
}

function nuxWizardRefreshConnectStep() {
  if (_nuxWizardState && _nuxWizardState.step === 1) renderNuxWizardStep();
}

// --- Step 3: start the first conversation ---

function renderNuxWizardChatStep(body, footer) {
  const title = document.createElement('h2');
  title.className = 'nux-wizard-title';
  title.id = 'nux-wizard-title';
  title.textContent = I18n.t('nux.chatTitle');
  body.appendChild(title);

  const subtitle = document.createElement('p');
  subtitle.className = 'nux-wizard-subtitle';
  subtitle.textContent = I18n.t('nux.chatSubtitle');
  body.appendChild(subtitle);

  const prompt = (_nuxWizardState.useCase && _nuxWizardState.useCase.prompt)
    || (NUX_DATA.starterPrompts || [])[0]
    || '';

  const preview = document.createElement('div');
  preview.className = 'nux-prompt-preview';
  preview.textContent = prompt;
  body.appendChild(preview);

  const hint = document.createElement('p');
  hint.className = 'nux-wizard-hint';
  hint.textContent = I18n.t('nux.chatHint');
  body.appendChild(hint);

  footer.appendChild(nuxWizardFooterButton(
    I18n.t('common.back'), 'btn-secondary', () => nuxWizardGoTo(1)
  ));
  footer.appendChild(nuxWizardFooterButton(
    I18n.t('nux.startChatting'), 'btn-primary', () => {
      closeNuxSetupWizard();
      switchTab('chat');
      const input = document.getElementById('chat-input');
      if (input && !input.disabled && prompt) {
        input.value = prompt;
        input.dispatchEvent(new Event('input'));
        input.focus();
      }
    }
  ));
}
