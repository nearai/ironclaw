function isCurrentThread(threadId) {
  if (!threadId) return false;
  if (!currentThreadId) return true;
  return threadId === currentThreadId;
}

// --- Suggestion Chips ---

function showSuggestionChips(suggestions) {
  // Clear previous chips/ghost without restoring placeholder (we'll set it below)
  _ghostSuggestion = '';
  const container = document.getElementById('suggestion-chips');
  container.innerHTML = '';
  const ghost = document.getElementById('ghost-text');
  ghost.style.display = 'none';
  const wrapper = document.querySelector('.chat-input-wrapper');
  if (wrapper) wrapper.classList.remove('has-ghost');

  _ghostSuggestion = suggestions[0] || '';
  const input = document.getElementById('chat-input');
  suggestions.forEach(text => {
    const chip = document.createElement('button');
    chip.className = 'suggestion-chip';
    chip.textContent = text;
    chip.addEventListener('click', () => {
      input.value = text;
      clearSuggestionChips();
      autoResizeTextarea(input);
      input.focus();
      sendMessage();
    });
    container.appendChild(chip);
  });
  container.style.display = 'flex';
  // Show first suggestion as ghost text in the input so user knows Tab works
  if (_ghostSuggestion && input.value === '') {
    ghost.textContent = _ghostSuggestion;
    ghost.style.display = 'block';
    input.closest('.chat-input-wrapper').classList.add('has-ghost');
  }
}

function clearSuggestionChips() {
  _ghostSuggestion = '';
  const container = document.getElementById('suggestion-chips');
  if (container) {
    container.innerHTML = '';
    container.style.display = 'none';
  }
  const ghost = document.getElementById('ghost-text');
  if (ghost) ghost.style.display = 'none';
  const wrapper = document.querySelector('.chat-input-wrapper');
  if (wrapper) wrapper.classList.remove('has-ghost');
}

// --- Flow cards (in-thread onboarding widgets) ---
//
// Rendered from the `flow_card` SSE event: connect CTA, stack cascade,
// "reading your world" stats, and draft-automation proposal cards. Actions
// post to /api/flows/action (approve creates a real task on the board).

const FLOW_CARD_ICONS = {
  'bell': '<path d="M10.268 21a2 2 0 0 0 3.464 0"/><path d="M3.262 15.326A1 1 0 0 0 4 17h16a1 1 0 0 0 .74-1.673C19.41 13.956 18 12.499 18 8A6 6 0 0 0 6 8c0 4.499-1.411 5.956-2.738 7.326"/>',
  'inbox': '<polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>',
  'calendar-check': '<path d="M8 2v4"/><path d="M16 2v4"/><rect width="18" height="18" x="3" y="4" rx="2"/><path d="M3 10h18"/><path d="m9 16 2 2 4-4"/>',
  'users': '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>',
  'trending-up': '<polyline points="22 7 13.5 15.5 8.5 10.5 2 17"/><polyline points="16 7 22 7 22 13"/>',
  'radar': '<path d="M19.07 4.93A10 10 0 0 0 6.99 3.34"/><path d="M4 6h.01"/><path d="M2.29 9.62a10 10 0 1 0 19.02-1.27"/><path d="M16.24 7.76a6 6 0 1 0-8.01 8.91"/><path d="M12 18h.01"/><path d="M17.99 11.66a6 6 0 0 1-2.22 4.75"/><circle cx="12" cy="12" r="2"/><path d="m13.41 10.59 5.66-5.66"/>',
  'git-branch': '<line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/>',
  'git-pull-request': '<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M13 6h3a2 2 0 0 1 2 2v7"/><line x1="6" x2="6" y1="9" y2="21"/>',
  'bug': '<path d="m8 2 1.88 1.88"/><path d="M14.12 3.88 16 2"/><path d="M9 7.13v-1a3.003 3.003 0 1 1 6 0v1"/><path d="M12 20c-3.3 0-6-2.7-6-6v-3a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v3c0 3.3-2.7 6-6 6"/><path d="M12 20v-9"/><path d="M6.53 9C4.6 8.8 3 7.1 3 5"/><path d="M6 13H2"/><path d="M3 21c0-2.1 1.7-3.9 3.8-4"/><path d="M20.97 5c0 2.1-1.6 3.8-3.5 4"/><path d="M22 13h-4"/><path d="M17.2 17c2.1.1 3.8 1.9 3.8 4"/>',
  'messages-square': '<path d="M14 9a2 2 0 0 1-2 2H6l-4 4V4a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2z"/><path d="M18 9h2a2 2 0 0 1 2 2v11l-4-4h-6a2 2 0 0 1-2-2v-1"/>',
  'newspaper': '<path d="M4 22h16a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2H8a2 2 0 0 0-2 2v16a2 2 0 0 1-2 2Zm0 0a2 2 0 0 1-2-2v-9c0-1.1.9-2 2-2h2"/><path d="M18 14h-8"/><path d="M15 18h-5"/><path d="M10 6h8v4h-8V6Z"/>',
  'check-square': '<path d="m9 11 3 3L22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/>',
  'activity': '<path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2"/>',
  'alert-triangle': '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>',
};

function flowIconSvg(name, size) {
  const path = FLOW_CARD_ICONS[name] || FLOW_CARD_ICONS['bell'];
  return '<svg width="' + (size || 15) + '" height="' + (size || 15) + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + path + '</svg>';
}

// MOCK: per-provider consent scopes shown in the OAuth-style connect modal
// (illustrative demo copy — a real OAuth screen lives with the provider).
const FLOW_CONSENT_SCOPES = {
  gmail: ['Read, compose, and send email', 'Manage labels and drafts'],
  google_calendar: ['View and edit events on your calendars', 'See your availability'],
  google_sheets: ['See and manage your spreadsheets'],
  google_drive: ['View and manage files you open with this app'],
  slack: ['Send messages as you', 'Read channels you choose'],
  telegram: ['Message you on Telegram'],
  github: ['Read repos, issues, and pull requests'],
};

function flowProviderLabel(provider, title) {
  if (provider === 'gmail' || String(provider).indexOf('google_') === 0) return 'Google';
  return title || provider;
}

// OAuth-style consent modal — the deferred-auth moment of the chat-first
// journey. "Allow" mocks the OAuth round-trip (short beat), then resolves
// the paused flow via /api/flows/action.
function showFlowConsentModal(card, onAllowed) {
  closeFlowModal();
  const provider = card.provider;
  const name = card.title.replace(/^Connect\s+/i, '');
  const scopes = FLOW_CONSENT_SCOPES[provider] || ['Access your ' + name + ' data'];

  const overlay = document.createElement('div');
  overlay.className = 'flow-modal-overlay';
  overlay.id = 'flow-modal';
  let scopeRows = '';
  scopes.forEach((scope) => {
    scopeRows += '<li class="flow-consent-scope">'
      + '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12"/></svg>'
      + escapeHtml(scope) + '</li>';
  });
  overlay.innerHTML =
    '<div class="flow-modal flow-consent" role="dialog" aria-modal="true" aria-label="' + escapeHtml(card.title) + '">'
    + '<div class="flow-consent-head">'
    + '<img class="flow-consent-icon" src="' + escapeHtml(card.icon || '') + '" alt="" aria-hidden="true">'
    + '<div class="flow-consent-titles">'
    + '<div class="flow-consent-title">' + escapeHtml(card.title) + '</div>'
    + '<div class="flow-consent-subtitle">' + escapeHtml(I18n.t('flow.consentSubtitle', { provider: flowProviderLabel(provider, name) })) + '</div>'
    + '</div>'
    + '</div>'
    + '<div class="flow-consent-scopes">'
    + '<div class="flow-consent-scopes-label">' + escapeHtml(I18n.t('flow.consentScopesLabel')) + '</div>'
    + '<ul>' + scopeRows + '</ul>'
    + '</div>'
    + '<div class="flow-consent-vault">'
    + '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/></svg>'
    + '<span>' + escapeHtml(I18n.t('flow.consentVault')) + '</span>'
    + '</div>'
    + '<div class="flow-modal-actions">'
    + '<button type="button" class="flow-modal-secondary flow-consent-cancel">' + escapeHtml(I18n.t('flow.notNow')) + '</button>'
    + '<button type="button" class="btn-primary flow-consent-allow">' + escapeHtml(I18n.t('flow.allow')) + '</button>'
    + '</div>'
    + '</div>';

  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) closeFlowModal();
  });
  overlay.querySelector('.flow-consent-cancel').addEventListener('click', () => closeFlowModal());
  const allowBtn = overlay.querySelector('.flow-consent-allow');
  allowBtn.addEventListener('click', () => {
    allowBtn.disabled = true;
    overlay.querySelector('.flow-consent-cancel').disabled = true;
    allowBtn.innerHTML = '<span class="spinner"></span> ' + escapeHtml(I18n.t('flow.connecting'));
    // Mock the OAuth round-trip, then resolve the paused flow.
    setTimeout(() => {
      closeFlowModal();
      onAllowed();
    }, 900);
  });
  document.body.appendChild(overlay);
  // Move focus into the dialog so keyboard/screen-reader users land on it.
  overlay.querySelector('.flow-consent-cancel').focus();
}

// Proposal detail modal — what would actually run: trigger, steps, and a
// readable automation spec (Suggest-only, nothing runs without approval).
function showFlowProposalModal(card) {
  closeFlowModal();
  const uses = (card.uses && card.uses.length > 0) ? card.uses : ['chat'];
  const specLines = [
    'name: ' + card.proposal,
    'on: ' + (card.runsWhen || 'schedule \u00b7 daily'),
    'uses: [' + uses.join(', ') + ']',
    'mode: suggest        # drafts for your approval \u2014 nothing runs without you',
    'steps:',
  ];
  (card.details || []).forEach((d) => {
    specLines.push('  - ' + d.toLowerCase().replace(/\.$/, ''));
  });
  specLines.push('deliver: chat + connected channels');
  specLines.push('vault: credentials injected at the boundary \u2014 the model never sees them');

  let detailRows = '';
  (card.details || []).forEach((d) => {
    detailRows += '<li>' + escapeHtml(d) + '</li>';
  });

  const overlay = document.createElement('div');
  overlay.className = 'flow-modal-overlay';
  overlay.id = 'flow-modal';
  overlay.innerHTML =
    '<div class="flow-modal flow-proposal-detail" role="dialog" aria-modal="true" aria-label="' + escapeHtml(card.title) + '">'
    + '<div class="flow-consent-head">'
    + '<span class="flow-proposal-icon flow-detail-icon" aria-hidden="true">' + flowIconSvg(card.icon, 18) + '</span>'
    + '<div class="flow-consent-titles">'
    + '<div class="flow-consent-title">' + escapeHtml(card.title) + '</div>'
    + '<div class="flow-detail-body">' + escapeHtml(card.body || '') + '</div>'
    + '</div>'
    + '</div>'
    + '<div class="flow-detail-section">'
    + '<div class="flow-detail-label">' + escapeHtml(I18n.t('flow.detailRunsWhen')) + '</div>'
    + '<div class="flow-detail-trigger">' + escapeHtml(card.runsWhen || '') + '</div>'
    + '</div>'
    + '<div class="flow-detail-section">'
    + '<div class="flow-detail-label">' + escapeHtml(I18n.t('flow.detailWhatItDoes')) + '</div>'
    + '<ul class="flow-detail-list">' + detailRows + '</ul>'
    + '</div>'
    + '<div class="flow-detail-section">'
    + '<div class="flow-detail-label">' + escapeHtml(I18n.t('flow.detailAutomation')) + '</div>'
    + '<pre class="flow-detail-spec">' + escapeHtml(specLines.join('\n')) + '</pre>'
    + '</div>'
    + '<div class="flow-modal-actions">'
    + '<button type="button" class="flow-modal-secondary flow-detail-close">' + escapeHtml(I18n.t('common.close')) + '</button>'
    + '</div>'
    + '</div>';

  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) closeFlowModal();
  });
  overlay.querySelector('.flow-detail-close').addEventListener('click', () => closeFlowModal());
  document.body.appendChild(overlay);
}

function closeFlowModal() {
  document.getElementById('flow-modal')?.remove();
}

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') closeFlowModal();
});

// One-shot sparkle burst — the "magic moment" when the scripted first hour
// completes (prototype parity, tuned down for the workspace).
function flowFlourish() {
  if (document.getElementById('flow-flourish')) return;
  const host = document.createElement('div');
  host.id = 'flow-flourish';
  host.className = 'flow-flourish';
  host.setAttribute('aria-hidden', 'true');
  let sparks = '';
  for (let i = 0; i < 14; i++) {
    const left = 8 + ((i * 6.3) % 84);
    const delay = (i % 7) * 90;
    const size = 9 + (i % 4) * 4;
    sparks += '<svg class="flow-flourish-spark" style="left:' + left + '%;width:' + size + 'px;height:' + size + 'px;animation-delay:' + delay + 'ms" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/></svg>';
  }
  host.innerHTML = sparks;
  document.body.appendChild(host);
  setTimeout(() => host.remove(), 2400);
}

function renderFlowCard(card) {
  if (!card || !card.kind) return;
  const container = document.getElementById('chat-messages');
  if (!container) return;
  finalizeActivityGroup();

  const el = document.createElement('div');
  el.className = 'flow-card flow-card-' + card.kind;

  if (card.kind === 'connect') {
    el.innerHTML =
      '<img class="flow-connect-icon" src="' + escapeHtml(card.icon || '') + '" alt="" aria-hidden="true">'
      + '<span class="flow-connect-body">'
      + '<span class="flow-connect-title">' + escapeHtml(card.title) + '</span>'
      + '<span class="flow-connect-caption">' + escapeHtml(card.caption || '') + '</span>'
      + '</span>'
      + '<button type="button" class="btn-primary flow-connect-btn">' + escapeHtml(I18n.t('flow.connect')) + '</button>';
    const btn = el.querySelector('.flow-connect-btn');
    // The OAuth-style consent modal is the deferred-auth beat: Connect opens
    // it, Allow resolves the mock OAuth and resumes the paused flow.
    btn.addEventListener('click', () => {
      showFlowConsentModal(card, () => {
        btn.disabled = true;
        btn.textContent = I18n.t('flow.connecting');
        apiFetch('/api/flows/action', {
          method: 'POST',
          body: { action: 'connect', provider: card.provider },
        }).then(() => {
          el.classList.add('connected');
          btn.outerHTML = '<span class="flow-connect-done">'
            + '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg> '
            + escapeHtml(I18n.t('flow.connected')) + '</span>';
        }).catch(() => {
          btn.disabled = false;
          btn.textContent = I18n.t('flow.connect');
        });
      });
    });
  }

  if (card.kind === 'cascade') {
    let chips = '';
    (card.chips || []).forEach((chip, i) => {
      chips += '<span class="flow-cascade-chip" style="animation-delay:' + (i * 380) + 'ms"'
        + (chip.blurb ? ' title="' + escapeHtml(chip.blurb) + '"' : '') + '>'
        + (chip.icon ? '<img src="' + escapeHtml(chip.icon) + '" alt="" aria-hidden="true">' : '')
        + escapeHtml(chip.label)
        + '<svg class="flow-cascade-check" style="animation-delay:' + (i * 380 + 240) + 'ms" width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>'
        + (chip.via ? '<span class="flow-cascade-via">' + escapeHtml(chip.via) + '</span>' : '')
        + '</span>';
    });
    el.innerHTML =
      '<div class="flow-cascade-label">' + escapeHtml(card.label || '') + '</div>'
      + '<div class="flow-cascade-chips">' + chips + '</div>';
  }

  if (card.kind === 'reading') {
    let stats = '';
    (card.stats || []).forEach((s) => {
      stats += '<div class="flow-stat"><span class="flow-stat-value">' + escapeHtml(String(s.value)) + '</span>'
        + '<span class="flow-stat-label">' + escapeHtml(s.label) + '</span></div>';
    });
    let learned = '';
    (card.learned || []).forEach((l, i) => {
      learned += '<span class="flow-learned-chip" style="animation-delay:' + (i * 140) + 'ms">' + escapeHtml(l) + '</span>';
    });
    el.innerHTML =
      '<div class="flow-stats">' + stats + '</div>'
      + '<div class="flow-learned">' + learned + '</div>';
  }

  if (card.kind === 'proposal') {
    let rows = '';
    (card.details || []).forEach((d) => {
      rows += '<div class="flow-proposal-row">'
        + '<span class="flow-proposal-row-circle" aria-hidden="true"></span>'
        + '<span class="flow-proposal-row-icon" aria-hidden="true">' + flowIconSvg(card.icon, 12) + '</span>'
        + '<span class="flow-proposal-row-text">' + escapeHtml(d) + '</span>'
        + '</div>';
    });
    el.innerHTML =
      '<div class="flow-proposal-head">'
      + '<span class="flow-proposal-icon" aria-hidden="true">' + flowIconSvg(card.icon, 15) + '</span>'
      + '<span class="flow-proposal-title">' + escapeHtml(card.title) + '</span>'
      + '<span class="flow-proposal-badge draft">' + escapeHtml(I18n.t('flow.draft')) + '</span>'
      + '</div>'
      + '<div class="flow-proposal-body">' + escapeHtml(card.body || '') + '</div>'
      + '<div class="flow-proposal-rows">' + rows + '</div>'
      + '<div class="flow-proposal-hint">' + escapeHtml(card.suggestLine || '') + '</div>'
      + '<div class="flow-proposal-footer">'
      + '<button type="button" class="flow-proposal-details">' + escapeHtml(I18n.t('flow.viewDetails'))
      + '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 18 15 12 9 6"/></svg>'
      + '</button>'
      + '<span class="flow-proposal-actions">'
      + '<button type="button" class="flow-proposal-dismiss">' + escapeHtml(I18n.t('flow.dismiss')) + '</button>'
      + '<button type="button" class="btn-primary flow-proposal-approve">' + escapeHtml(I18n.t('flow.approve')) + '</button>'
      + '</span>'
      + '</div>';
    const badge = el.querySelector('.flow-proposal-badge');
    const approve = el.querySelector('.flow-proposal-approve');
    const dismiss = el.querySelector('.flow-proposal-dismiss');
    el.querySelector('.flow-proposal-details').addEventListener('click', () => showFlowProposalModal(card));
    approve.addEventListener('click', () => {
      approve.disabled = true;
      dismiss.disabled = true;
      apiFetch('/api/flows/action', {
        method: 'POST',
        body: { action: 'approve', proposal: card.proposal },
      }).then(() => {
        el.classList.add('approved');
        badge.textContent = I18n.t('flow.live');
        badge.classList.remove('draft');
        badge.classList.add('live');
        el.querySelectorAll('.flow-proposal-row').forEach((row) => row.classList.add('done'));
        el.querySelector('.flow-proposal-actions').innerHTML =
          '<span class="flow-proposal-live-note">' + escapeHtml(I18n.t('flow.approvedNote')) + '</span>';
        showToast(I18n.t('flow.automationCreated'), 'success');
      }).catch(() => {
        approve.disabled = false;
        dismiss.disabled = false;
      });
    });
    dismiss.addEventListener('click', () => {
      apiFetch('/api/flows/action', { method: 'POST', body: { action: 'dismiss', proposal: card.proposal } }).catch(() => {});
      el.classList.add('dismissed');
      badge.textContent = I18n.t('flow.dismissed');
      el.querySelector('.flow-proposal-actions').innerHTML = '';
    });
  }

  // Plan-upgrade CTA — the last beat of the scripted first hour. Rides with
  // the one-shot sparkle flourish (the setup-complete "magic moment").
  if (card.kind === 'upgrade') {
    flowFlourish();
    el.innerHTML =
      '<button type="button" class="btn-primary flow-upgrade-btn">'
      + '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect width="20" height="14" x="2" y="5" rx="2"/><line x1="2" x2="22" y1="10" y2="10"/></svg>'
      + escapeHtml(I18n.t('flow.pickPlan'))
      + '</button>';
    el.querySelector('.flow-upgrade-btn').addEventListener('click', () => {
      showPlanPickerModal((plan) => {
        el.innerHTML = '<span class="flow-upgrade-done">'
          + '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12"/></svg>'
          + escapeHtml(I18n.t('flow.onPlan', { plan: plan.name }))
          + '</span>';
        // The agent confirms the upgrade in-thread (streamed bubble).
        apiFetch('/api/flows/action', {
          method: 'POST',
          body: { action: 'upgrade', plan: plan.id, thread_id: currentThreadId },
        }).catch(() => {});
      });
    });
  }

  container.appendChild(el);
  container.scrollTop = container.scrollHeight;
}

// --- Chat ---

async function sendMessage() {
  // Wait for any in-flight FileReader decode so an Enter-press mid-upload
  // still includes the attachment in the next /api/chat/send body.
  if (pendingAttachmentReads.length > 0) {
    await Promise.all([...pendingAttachmentReads]);
  }
  clearSuggestionChips();
  removeWelcomeCard();
  _turnResponseReceived = false;
  if (_doneWithoutResponseTimer) {
    clearTimeout(_doneWithoutResponseTimer);
    _doneWithoutResponseTimer = null;
  }
  const input = document.getElementById('chat-input');
  if (authFlowPending) {
    showToast(I18n.t('chat.authRequiredBeforeSend'), 'info');
    const tokenField = document.querySelector('.auth-card .auth-token-input input');
    if (tokenField) tokenField.focus();
    return;
  }
  if (!currentThreadId) {
    console.warn('sendMessage: no thread selected, ignoring');
    return;
  }
  if (_sendCooldown) return;
  const content = input.value.trim();
  if (!content && stagedImages.length === 0 && stagedAttachments.length === 0) return;

  // Intercept approval keywords when an unresolved approval card is pending.
  // Find the most recent unresolved card for the current thread (resolved cards
  // linger 1.5s before removal; cards from other threads must not be matched).
  const approvalCards = Array.from(document.querySelectorAll('.approval-card'));
  const approvalCard = approvalCards.reverse().find(card => {
    if (card.querySelector('.approval-resolved')) return false;
    const cardThreadId = card.getAttribute('data-thread-id');
    return !cardThreadId || cardThreadId === currentThreadId;
  });
  if (approvalCard && content) {
    const lower = content.toLowerCase();
    let action = null;
    if (['yes', 'y', 'approve', 'ok', '/approve', '/yes', '/y'].includes(lower)) {
      action = 'approve';
    } else if (['always', 'a', 'yes always', 'approve always', '/always', '/a'].includes(lower)) {
      action = 'always';
    } else if (['no', 'n', 'deny', 'reject', 'cancel', '/deny', '/no', '/n'].includes(lower)) {
      action = 'deny';
    }
    if (action) {
      input.value = '';
      autoResizeTextarea(input);
      input.focus();
      const requestId = approvalCard.getAttribute('data-request-id');
      const threadId = approvalCard.getAttribute('data-thread-id');
      if (requestId) {
        sendApprovalAction(requestId, action, threadId);
      }
      return;
    }
  }

  // Snapshot attached images + attachments before the body block clears them,
  // so the optimistic display, pending entry, and retry handler all see the
  // same view the user pressed Enter on.
  const attachedImageDataUrls = stagedImages.map(img => img.dataUrl);
  const pendingAttachmentsForDisplay = stagedAttachments.map(att => ({
    kind: att.kind || (att.mime_type && att.mime_type.startsWith('image/') ? 'image' : 'document'),
    filename: att.filename || 'attachment',
    mime_type: att.mime_type || '',
    size_label: att.size_label || '',
    preview_url: att.preview_url || null,
    preview_text: '',
  }));
  const displayContent = content
    || (pendingAttachmentsForDisplay.length > 0 ? '(files attached)' : '(images attached)');
  const pendingCopyTextParts = [];
  if (displayContent) pendingCopyTextParts.push(displayContent);
  pendingAttachmentsForDisplay.forEach((att) => {
    const suffix = [att.mime_type, att.size_label].filter(Boolean).join(' • ');
    pendingCopyTextParts.push(
      suffix
        ? `[Attachment] ${att.filename || 'attachment'} (${suffix})`
        : `[Attachment] ${att.filename || 'attachment'}`
    );
  });
  const pendingCopyText = pendingCopyTextParts.join('\n');
  const userMsg = addMessage('user', displayContent, {
    attachments: pendingAttachmentsForDisplay,
    copyText: pendingCopyText,
  });
  if (attachedImageDataUrls.length > 0) {
    appendImagesToMessage(userMsg, attachedImageDataUrls);
  }
  pruneOldMessages();
  if (currentThreadId) {
    activeWorkStore.updateThread(currentThreadId, {
      statusText: ActivityEntry.t('activity.starting', 'Starting'),
    });
  }
  input.value = '';
  autoResizeTextarea(input);
  input.focus();

  // Track as pending so loadHistory() can re-inject if DB hasn't persisted yet (#2409)
  let pendingId = null;
  const pendingThreadId = currentThreadId;
  if (currentThreadId) {
    if (!_pendingUserMessages.has(currentThreadId)) {
      _pendingUserMessages.set(currentThreadId, []);
    }
    pendingId = _nextPendingId++;
    _pendingUserMessages.get(currentThreadId).push({
      id: pendingId,
      content: displayContent,
      copyText: pendingCopyText,
      attachments: pendingAttachmentsForDisplay.map((att) => ({ ...att })),
      images: attachedImageDataUrls,
      timestamp: Date.now(),
    });
  }

  const body = { content, thread_id: currentThreadId || undefined, timezone: Intl.DateTimeFormat().resolvedOptions().timeZone };
  if (stagedImages.length > 0) {
    body.images = stagedImages.map(img => ({ media_type: img.media_type, data: img.data }));
    stagedImages = [];
    renderImagePreviews();
  }
  // Clone attachments so the retry handler can restore them if send fails
  // without getting mutated by subsequent stagedAttachments clears.
  const pendingAttachments = stagedAttachments.map(att => ({ ...att }));
  if (stagedAttachments.length > 0) {
    body.attachments = stagedAttachments.map(att => ({
      mime_type: att.mime_type,
      filename: att.filename,
      data_base64: att.data_base64,
    }));
    stagedAttachments = [];
    if (typeof renderAttachmentPreviews === 'function') {
      renderAttachmentPreviews();
    }
  }

  // MOCK billing: each agent message burns a little of the credit balance
  // so the sidebar ring visibly moves (see js/core/billing.js).
  if (typeof billingRecordUsage === 'function') billingRecordUsage();

  apiFetch('/api/chat/send', {
    method: 'POST',
    body: body,
  }).catch((err) => {
    // Remove the pending entry so it won't be re-injected on thread switch (#2498)
    if (pendingId !== null && pendingThreadId) {
      const arr = _pendingUserMessages.get(pendingThreadId);
      if (arr) {
        const filtered = arr.filter(p => p.id !== pendingId);
        if (filtered.length > 0) {
          _pendingUserMessages.set(pendingThreadId, filtered);
        } else {
          _pendingUserMessages.delete(pendingThreadId);
        }
      }
    }
    // Handle rate limiting (429)
    if (err.status === 429) {
      showToast(I18n.t('chat.rateLimited'), 'error');
      _sendCooldown = true;
      const sendBtn = document.getElementById('send-btn');
      if (sendBtn) sendBtn.disabled = true;
      setTimeout(() => {
        _sendCooldown = false;
        if (sendBtn) sendBtn.disabled = false;
      }, 2000);
    }
    // Keep the user message in DOM, add a retry link
    if (userMsg) {
      userMsg.classList.add('send-failed');
      userMsg.style.borderStyle = 'dashed';
      const retryLink = document.createElement('a');
      retryLink.className = 'retry-link';
      retryLink.href = '#';
      retryLink.textContent = I18n.t('common.retry');
      retryLink.addEventListener('click', (e) => {
        e.preventDefault();
        if (userMsg.parentNode) userMsg.parentNode.removeChild(userMsg);
        // Restore the attachments we just cleared so the retry carries the
        // same payload the failed send attempted. `stagedImages` is kept
        // separately by the existing preview machinery.
        if (pendingAttachments.length > 0) {
          stagedAttachments = pendingAttachments.map(att => ({ ...att }));
          if (typeof renderAttachmentPreviews === 'function') {
            renderAttachmentPreviews();
          }
        }
        input.value = content;
        sendMessage();
      });
      userMsg.appendChild(retryLink);
    }
  });
}

function enableChatInput() {
  if (currentThreadIsReadOnly || authFlowPending) return;
  const input = document.getElementById('chat-input');
  const btn = document.getElementById('send-btn');
  if (input) {
    input.disabled = false;
    input.placeholder = I18n.t('chat.inputPlaceholder');
  }
  if (btn) btn.disabled = false;
}

// --- Image Upload ---

function renderImagePreviews() {
  const strip = document.getElementById('image-preview-strip');
  strip.innerHTML = '';
  stagedImages.forEach((img, idx) => {
    const container = document.createElement('div');
    container.className = 'image-preview-container';

    const preview = document.createElement('img');
    preview.className = 'image-preview';
    preview.src = img.dataUrl;
    preview.alt = 'Attached image';

    const removeBtn = document.createElement('button');
    removeBtn.className = 'image-preview-remove';
    removeBtn.textContent = '\u00d7';
    removeBtn.addEventListener('click', () => {
      stagedImages.splice(idx, 1);
      renderImagePreviews();
    });

    container.appendChild(preview);
    container.appendChild(removeBtn);
    strip.appendChild(container);
  });
}

const MAX_IMAGE_SIZE_BYTES = 5 * 1024 * 1024; // 5 MB per image
const MAX_STAGED_IMAGES = 5;

function handleImageFiles(files) {
  Array.from(files).forEach(file => {
    if (!file.type.startsWith('image/')) return;
    if (file.size > MAX_IMAGE_SIZE_BYTES) {
      alert(I18n.t('chat.imageTooBig', { name: file.name, size: (file.size / 1024 / 1024).toFixed(1) }));
      return;
    }
    if (stagedImages.length >= MAX_STAGED_IMAGES) {
      alert(I18n.t('chat.maxImages', { n: MAX_STAGED_IMAGES }));
      return;
    }
    const reader = new FileReader();
    reader.onload = function(e) {
      const dataUrl = e.target.result;
      const commaIdx = dataUrl.indexOf(',');
      const meta = dataUrl.substring(0, commaIdx); // e.g. "data:image/png;base64"
      const base64 = dataUrl.substring(commaIdx + 1);
      const mediaType = meta.replace('data:', '').replace(';base64', '');
      stagedImages.push({ media_type: mediaType, data: base64, dataUrl: dataUrl });
      renderImagePreviews();
    };
    reader.readAsDataURL(file);
  });
}

// The click/change/paste wiring for #attach-btn + #image-file-input lives in
// the `wireAttachmentUI` IIFE below (next to the unified handleAttachmentFiles
// flow). A duplicate set of listeners used to live here and fire first,
// clearing `e.target.value` before the unified listener ran — which emptied
// the FileList and silently dropped every uploaded attachment.

const chatMessagesEl = document.getElementById('chat-messages');
chatMessagesEl.addEventListener('copy', (e) => {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed) return;
  const anchorNode = selection.anchorNode;
  const focusNode = selection.focusNode;
  if (!anchorNode || !focusNode) return;
  if (!chatMessagesEl.contains(anchorNode) || !chatMessagesEl.contains(focusNode)) return;
  const text = selection.toString();
  if (!text || !e.clipboardData) return;
  // Force plain-text clipboard output so dark-theme styling never leaks on paste.
  e.preventDefault();
  e.clipboardData.clearData();
  e.clipboardData.setData('text/plain', text);
});

function createGeneratedImageElement(dataUrl, path, eventId) {
  const card = document.createElement('div');
  card.className = 'generated-image-card';
  if (eventId) {
    card.dataset.imageEventId = eventId;
  }

  if (isSafeGeneratedImageDataUrl(dataUrl)) {
    const img = document.createElement('img');
    img.className = 'generated-image';
    img.src = dataUrl;
    img.alt = 'Generated image';
    card.appendChild(img);
  } else {
    const placeholder = document.createElement('div');
    placeholder.className = 'generated-image-placeholder';
    placeholder.textContent = 'Generated image unavailable in history payload';
    card.appendChild(placeholder);
  }

  if (path) {
    const pathLabel = document.createElement('div');
    pathLabel.className = 'generated-image-path';
    pathLabel.textContent = path;
    card.appendChild(pathLabel);
  }

  return card;
}

function isSafeGeneratedImageDataUrl(dataUrl) {
  return typeof dataUrl === 'string' && /^data:image\//i.test(dataUrl);
}

function hasRenderedGeneratedImage(container, eventId) {
  if (!eventId) return false;
  return Array.from(container.querySelectorAll('.generated-image-card')).some((card) => {
    return card.dataset.imageEventId === eventId;
  });
}

function addGeneratedImage(dataUrl, path, eventId, shouldScroll = true) {
  const container = document.getElementById('chat-messages');
  if (hasRenderedGeneratedImage(container, eventId)) {
    return;
  }
  const card = createGeneratedImageElement(dataUrl, path, eventId);
  container.appendChild(card);
  if (shouldScroll) {
    container.scrollTop = container.scrollHeight;
  }
}

function rememberGeneratedImage(threadId, eventId, dataUrl, path) {
  if (!threadId || !eventId || !isSafeGeneratedImageDataUrl(dataUrl)) return;
  const normalizedPath = path || null;
  let images = generatedImagesByThread.get(threadId);
  if (!images) {
    if (generatedImagesByThread.size >= GENERATED_IMAGE_THREAD_CACHE_CAP) {
      const oldestThreadId = generatedImagesByThread.keys().next().value;
      if (oldestThreadId) {
        generatedImagesByThread.delete(oldestThreadId);
      }
    }
    images = [];
    generatedImagesByThread.set(threadId, images);
  } else {
    // Refresh insertion order so recently viewed/updated threads stay cached.
    generatedImagesByThread.delete(threadId);
    generatedImagesByThread.set(threadId, images);
  }
  if (images.some(img => img.eventId === eventId)) {
    return;
  }
  images.push({ eventId, dataUrl, path: normalizedPath });
  while (images.length > GENERATED_IMAGES_PER_THREAD_CAP) {
    images.shift();
  }
}

function getRememberedGeneratedImage(threadId, eventId) {
  if (!threadId || !eventId) return null;
  const images = generatedImagesByThread.get(threadId);
  if (!images) return null;
  return images.find(img => img.eventId === eventId) || null;
}

function resolveGeneratedImageForRender(threadId, image) {
  const normalizedPath = image.path || null;
  if (image.data_url) {
    return { dataUrl: image.data_url, path: normalizedPath };
  }
  const remembered = getRememberedGeneratedImage(threadId, image.event_id);
  if (remembered) {
    return { dataUrl: remembered.dataUrl, path: remembered.path };
  }
  return { dataUrl: null, path: normalizedPath };
}

// --- Slash Autocomplete ---

let _slashSkillEntries = [];

function showSlashAutocomplete(matches) {
  const el = document.getElementById('slash-autocomplete');
  if (!el || matches.length === 0) { hideSlashAutocomplete(); return; }
  _slashMatches = matches;
  _slashSelected = -1;
  el.innerHTML = '';
  matches.forEach((item, i) => {
    const row = document.createElement('div');
    row.className = 'slash-ac-item';
    row.dataset.index = i;
    var cmdSpan = document.createElement('span');
    cmdSpan.className = 'slash-ac-cmd';
    cmdSpan.textContent = item.cmd;
    var descSpan = document.createElement('span');
    descSpan.className = 'slash-ac-desc';
    descSpan.textContent = item.desc;
    row.appendChild(cmdSpan);
    row.appendChild(descSpan);
    row.addEventListener('mousedown', (e) => {
      e.preventDefault(); // prevent blur
      selectSlashItem(item.cmd);
    });
    el.appendChild(row);
  });
  el.style.display = 'block';
}

function setSlashSkillEntries(skills) {
  if (!Array.isArray(skills)) {
    _slashSkillEntries = [];
    const input = document.getElementById('chat-input');
    if (input && input.value.startsWith('/')) filterSlashCommands(input.value);
    return;
  }
  _slashSkillEntries = skills
    .filter((skill) => skill && typeof skill.name === 'string' && skill.name.trim() !== '')
    .map((skill) => ({
      cmd: '/' + skill.name.trim(),
      desc: (skill.description || '').trim() || 'Skill',
      kind: 'skill',
    }))
    .sort((a, b) => a.cmd.localeCompare(b.cmd));
  const input = document.getElementById('chat-input');
  if (input && input.value.startsWith('/')) filterSlashCommands(input.value);
}

function getSlashAutocompleteItems() {
  const items = SLASH_COMMANDS.map((cmd) => ({
    cmd: cmd.cmd,
    desc: cmd.desc,
    kind: 'command',
  }));
  const seen = new Set(items.map((item) => item.cmd.toLowerCase()));
  _slashSkillEntries.forEach((item) => {
    const key = item.cmd.toLowerCase();
    if (seen.has(key)) return;
    seen.add(key);
    items.push(item);
  });
  return items;
}

function refreshSlashSkillEntries() {
  return apiFetch('/api/skills')
    .then(function(data) {
      setSlashSkillEntries((data && data.skills) || []);
    })
    .catch(function() {
      // Preserve the last known skill list on transient fetch failures.
    });
}

function hideSlashAutocomplete() {
  const el = document.getElementById('slash-autocomplete');
  if (el) el.style.display = 'none';
  _slashSelected = -1;
  _slashMatches = [];
}

function selectSlashItem(cmd) {
  const input = document.getElementById('chat-input');
  input.value = cmd + ' ';
  input.focus();
  hideSlashAutocomplete();
  autoResizeTextarea(input);
}

function updateSlashHighlight() {
  const items = document.querySelectorAll('#slash-autocomplete .slash-ac-item');
  items.forEach((el, i) => el.classList.toggle('selected', i === _slashSelected));
  if (_slashSelected >= 0 && items[_slashSelected]) {
    items[_slashSelected].scrollIntoView({ block: 'nearest' });
  }
}

function filterSlashCommands(value) {
  if (!value.startsWith('/')) { hideSlashAutocomplete(); return; }
  // Only show autocomplete when the input is just a slash command prefix (no spaces except /thread new)
  const lower = value.toLowerCase();
  const exactLower = lower.trimEnd();
  const matches = getSlashAutocompleteItems().filter((c) => c.cmd.toLowerCase().startsWith(lower));
  if (matches.length === 0 || (matches.length === 1 && matches[0].cmd.toLowerCase() === exactLower)) {
    hideSlashAutocomplete();
  } else {
    showSlashAutocomplete(matches);
  }
}

function sendApprovalAction(requestId, action, threadId) {
  const card = document.querySelector('.approval-card[data-request-id="' + requestId + '"]');
  const targetThreadId = threadId || (card ? card.getAttribute('data-thread-id') : null) || currentThreadId;
  apiFetch('/api/chat/gate/resolve', {
    method: 'POST',
    body: {
      request_id: requestId,
      thread_id: targetThreadId,
      resolution: action === 'deny' ? 'denied' : 'approved',
      always: action === 'always',
    },
  }).catch((err) => {
    addMessage('system', 'Failed to send approval: ' + err.message);
  });

  // Disable buttons and show confirmation on the card
  if (card) {
    const buttons = card.querySelectorAll('.approval-actions button');
    buttons.forEach((btn) => {
      btn.disabled = true;
    });
    const actions = card.querySelector('.approval-actions');
    const label = document.createElement('span');
    label.className = 'approval-resolved';
    const labelText = action === 'approve' ? I18n.t('approval.approved') : action === 'always' ? I18n.t('approval.alwaysApproved') : I18n.t('approval.denied');
    label.textContent = labelText;
    actions.appendChild(label);
    // Remove the card after showing the confirmation briefly
    setTimeout(() => { card.remove(); }, 1500);
  }
}


// --- Attachment Upload ---

function inferAttachmentMimeType(file) {
  if (file.type) return file.type;
  const name = (file.name || '').toLowerCase();
  if (name.endsWith('.pdf')) return 'application/pdf';
  if (name.endsWith('.pptx')) return 'application/vnd.openxmlformats-officedocument.presentationml.presentation';
  if (name.endsWith('.ppt')) return 'application/vnd.ms-powerpoint';
  if (name.endsWith('.docx')) return 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';
  if (name.endsWith('.doc')) return 'application/msword';
  if (name.endsWith('.xlsx')) return 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';
  if (name.endsWith('.xls')) return 'application/vnd.ms-excel';
  if (name.endsWith('.md')) return 'text/markdown';
  if (name.endsWith('.csv')) return 'text/csv';
  if (name.endsWith('.json')) return 'application/json';
  if (name.endsWith('.xml')) return 'application/xml';
  if (name.endsWith('.rtf')) return 'application/rtf';
  if (name.endsWith('.txt')) return 'text/plain';
  if (name.endsWith('.mp3')) return 'audio/mpeg';
  if (name.endsWith('.ogg')) return 'audio/ogg';
  if (name.endsWith('.wav')) return 'audio/wav';
  if (name.endsWith('.m4a')) return 'audio/x-m4a';
  if (name.endsWith('.mp4')) return 'audio/mp4';
  if (name.endsWith('.aac')) return 'audio/aac';
  if (name.endsWith('.flac')) return 'audio/flac';
  if (name.endsWith('.webm')) return 'audio/webm';
  return 'application/octet-stream';
}

function formatAttachmentSize(bytes) {
  if (typeof bytes !== 'number') return '';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function appendAttachmentFileCard(container, itemClassName, nameClassName, metaClassName, filename, metaText) {
  const item = document.createElement('div');
  item.className = itemClassName;
  const nameEl = document.createElement('div');
  nameEl.className = nameClassName;
  nameEl.textContent = filename || 'attachment';
  item.appendChild(nameEl);
  if (metaText) {
    const metaEl = document.createElement('div');
    metaEl.className = metaClassName;
    metaEl.textContent = metaText;
    item.appendChild(metaEl);
  }
  container.appendChild(item);
}

function renderAttachmentPreviews() {
  const strip = document.getElementById('image-preview-strip');
  if (!strip) return;
  strip.innerHTML = '';
  stagedAttachments.forEach((att, idx) => {
    const container = document.createElement('div');
    container.className = 'attachment-preview-container';

    if (att.kind === 'image' && att.preview_url) {
      const preview = document.createElement('img');
      preview.className = 'image-preview';
      preview.src = att.preview_url;
      preview.alt = att.filename || 'Attached image';
      container.appendChild(preview);
    } else {
      container.classList.add('attachment-preview-file');
      const icon = document.createElement('div');
      icon.className = 'attachment-preview-file-icon';
      icon.textContent = (att.filename || 'FILE').split('.').pop().toUpperCase().slice(0, 4);
      container.appendChild(icon);
      const meta = document.createElement('div');
      meta.className = 'attachment-preview-file-meta';
      const nameEl = document.createElement('div');
      nameEl.className = 'attachment-preview-file-name';
      nameEl.textContent = att.filename || 'Attached file';
      meta.appendChild(nameEl);
      const typeEl = document.createElement('div');
      typeEl.className = 'attachment-preview-file-type';
      typeEl.textContent = att.mime_type;
      meta.appendChild(typeEl);
      container.appendChild(meta);
    }

    const removeBtn = document.createElement('button');
    removeBtn.className = 'image-preview-remove';
    removeBtn.textContent = '\u00d7';
    removeBtn.addEventListener('click', () => {
      stagedAttachments.splice(idx, 1);
      renderAttachmentPreviews();
    });

    container.appendChild(removeBtn);
    strip.appendChild(container);
  });
}

const MAX_ATTACHMENT_SIZE_BYTES = 5 * 1024 * 1024; // 5 MB per attachment
const MAX_TOTAL_ATTACHMENT_BYTES = 10 * 1024 * 1024; // 10 MB decoded per message
const MAX_STAGED_ATTACHMENTS = 5;

function handleAttachmentFiles(files) {
  let projectedCount = stagedAttachments.length + pendingAttachmentCount;
  let projectedTotalBytes = stagedAttachments.reduce((sum, att) => sum + (att.size_bytes || 0), 0) + pendingAttachmentBytes;
  Array.from(files).forEach(file => {
    const mimeType = inferAttachmentMimeType(file);
    if (file.size > MAX_ATTACHMENT_SIZE_BYTES) {
      alert(I18n.t('chat.fileTooBig', { name: file.name, size: (file.size / 1024 / 1024).toFixed(1) }));
      return;
    }
    if (projectedCount >= MAX_STAGED_ATTACHMENTS) {
      alert(I18n.t('chat.maxAttachments', { n: MAX_STAGED_ATTACHMENTS }));
      return;
    }
    if (projectedTotalBytes + file.size > MAX_TOTAL_ATTACHMENT_BYTES) {
      alert(I18n.t('chat.totalAttachmentsTooBig', { size: (MAX_TOTAL_ATTACHMENT_BYTES / 1024 / 1024).toFixed(0) }));
      return;
    }
    projectedCount += 1;
    projectedTotalBytes += file.size;
    pendingAttachmentCount += 1;
    pendingAttachmentBytes += file.size;

    const reader = new FileReader();
    let resolveRead;
    const readPromise = new Promise((resolve) => { resolveRead = resolve; });
    pendingAttachmentReads.push(readPromise);
    const finalizeRead = () => {
      pendingAttachmentCount = Math.max(0, pendingAttachmentCount - 1);
      pendingAttachmentBytes = Math.max(0, pendingAttachmentBytes - file.size);
      const idx = pendingAttachmentReads.indexOf(readPromise);
      if (idx !== -1) pendingAttachmentReads.splice(idx, 1);
      resolveRead();
    };
    reader.onload = function(e) {
      const dataUrl = e.target.result;
      const commaIdx = dataUrl.indexOf(',');
      const meta = dataUrl.substring(0, commaIdx);
      const base64 = dataUrl.substring(commaIdx + 1);
      const parsedType = meta.replace('data:', '').replace(';base64', '');
      const mediaType = (!parsedType || parsedType === 'application/octet-stream') ? mimeType : parsedType;
      stagedAttachments.push({
        kind: mediaType.startsWith('image/') ? 'image' : 'document',
        mime_type: mediaType,
        filename: file.name || null,
        data_base64: base64,
        preview_url: mediaType.startsWith('image/') ? dataUrl : null,
        size_bytes: file.size,
        size_label: formatAttachmentSize(file.size),
      });
      renderAttachmentPreviews();
      finalizeRead();
    };
    reader.onerror = function() {
      alert(I18n.t('error.unknown'));
      finalizeRead();
    };
    reader.readAsDataURL(file);
  });
}

(function wireAttachmentUI() {
  const attachBtn = document.getElementById('attach-btn');
  if (attachBtn) {
    attachBtn.addEventListener('click', () => {
      const input = document.getElementById('image-file-input');
      if (input) input.click();
    });
  }
  const fileInput = document.getElementById('image-file-input');
  if (fileInput) {
    fileInput.addEventListener('change', (e) => {
      // Snapshot the FileList into an array *before* clearing the input.
      // Some drivers (e.g. Playwright's set_input_files) expose a live
      // FileList that turns empty mid-listener-chain; reading it later
      // silently loses every file. Array.from fixes this by creating a
      // stable copy while the FileList is still populated.
      const files = Array.from(e.target.files || []);
      handleAttachmentFiles(files);
      e.target.value = '';
    });
  }
  const chatInputEl = document.getElementById('chat-input');
  if (chatInputEl) {
    chatInputEl.addEventListener('paste', (e) => {
      const items = (e.clipboardData || e.originalEvent.clipboardData).items;
      for (let i = 0; i < items.length; i++) {
        if (items[i].kind === 'file' && items[i].type.startsWith('image/')) {
          const file = items[i].getAsFile();
          if (file) handleAttachmentFiles([file]);
        }
      }
    });
  }
})();

// --- User message attachment parsing/rendering ---

function decodeXmlText(text) {
  return text
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&');
}

function parseAttachmentAttributes(rawAttrs) {
  const attrs = {};
  const attrRegex = /(\w+)="([^"]*)"/g;
  let match;
  while ((match = attrRegex.exec(rawAttrs)) !== null) {
    attrs[match[1]] = decodeXmlText(match[2]);
  }
  return attrs;
}

// Extract the plain text body and any `<attachments>…</attachments>` payload
// from a user turn's `user_input`. Messages carry their persisted attachment
// index inline so chat history can re-render file cards without a DB roundtrip.
// Only strip the trailing block when at least one `<attachment …>` element is
// parsed out of it — otherwise the user's raw text happens to end in
// `<attachments>…</attachments>` and we must leave it intact.
function parseUserMessageContent(content) {
  const match = content.match(/^([\s\S]*?)(?:\n\n)?<attachments>([\s\S]*?)<\/attachments>\s*$/);
  if (!match) {
    return { text: content, attachments: [], copyText: content };
  }

  const block = match[2];
  const attachments = [];
  const attachmentRegex = /<attachment\b([^>]*)>([\s\S]*?)<\/attachment>/g;
  let attachmentMatch;
  while ((attachmentMatch = attachmentRegex.exec(block)) !== null) {
    const attrs = parseAttachmentAttributes(attachmentMatch[1]);
    attachments.push({
      kind: attrs.type === 'image' ? 'image' : 'document',
      filename: attrs.filename || 'attachment',
      mime_type: attrs.mime || '',
      size_label: attrs.size || '',
      preview_text: decodeXmlText(attachmentMatch[2].trim()),
      preview_url: null,
    });
  }

  if (attachments.length === 0) {
    return { text: content, attachments: [], copyText: content };
  }

  const text = match[1].replace(/\s+$/, '');
  const copyParts = [];
  if (text) copyParts.push(text);
  attachments.forEach((att) => {
    const suffix = [att.mime_type, att.size_label].filter(Boolean).join(' • ');
    copyParts.push(suffix ? `[Attachment] ${att.filename} (${suffix})` : `[Attachment] ${att.filename}`);
  });

  return { text, attachments, copyText: copyParts.join('\n') };
}

function renderMessageAttachments(container, attachments) {
  if (!attachments || attachments.length === 0) return;
  const strip = document.createElement('div');
  strip.className = 'message-attachments';
  attachments.forEach((att) => {
    if (att.kind === 'image' && att.preview_url) {
      const image = document.createElement('img');
      image.className = 'message-attachment-image';
      image.src = att.preview_url;
      image.alt = att.filename || 'Attached image';
      strip.appendChild(image);
      return;
    }
    appendAttachmentFileCard(
      strip,
      'message-attachment-file',
      'message-attachment-file-name',
      'message-attachment-file-meta',
      att.filename || 'attachment',
      [att.mime_type, att.size_label].filter(Boolean).join(' • ')
    );
  });
  container.appendChild(strip);
}
