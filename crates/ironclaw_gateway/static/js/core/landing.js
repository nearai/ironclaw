// --- Landing use-case showcase (pre-auth) ---
//
// Renders the use-case gallery on the auth screen from NUX_DATA (see
// js/core/mock-backend.js). Selecting a card remembers its starter prompt so the
// chat input can be pre-filled the moment the user lands in the app — the
// shortest path from "this looks useful" to a working first conversation.

const NUX_PENDING_PROMPT_KEY = 'ironclaw_nux_pending_prompt';
// Onboarding-handoff state (PROTOTYPE / MOCK): the marketing-site onboarding
// flow deep-links to the gateway with `?integrations=gmail,slack` to simulate
// integrations the user "connected" during onboarding. They are stored here
// as mock connected state only — no real install/auth happens — and surface
// as "Connected" badges on the Integrations surface and the chat empty state.
const NUX_CONNECTED_INTEGRATIONS_KEY = 'ironclaw_nux_connected_integrations';
// The use case picked during onboarding, kept for the session so the chat
// empty state can derive actionable suggestion chips from it.
const NUX_PENDING_USECASE_KEY = 'ironclaw_nux_usecase';
// MOCK billing state from the onboarding handoff: `?billing=<plan-id>` (the
// plan picked during onboarding) or `?billing=skipped` (user deferred
// payment → free credits + a one-per-session reminder, see billing.js).
const NUX_BILLING_KEY = 'ironclaw_nux_billing';

// Capture intent handed off from the marketing site (ironclaw.com use-case
// cards / hero prompt box / mocked onboarding flow deep-link here with
// ?usecase=<id>, ?prompt=<text>, and/or ?integrations=<comma,separated,ids>).
// Stored in the same pending-prompt slot the landing cards use, so the chat
// input is pre-filled the moment the user is through auth — the funnel keeps
// the user's intent from first click to first message.
// (`?token=` is handled separately by autoAuth in init-auth.js.)
function captureIntentFromUrl() {
  const params = new URLSearchParams(window.location.search);
  const useCaseId = params.get('usecase');
  const rawPrompt = params.get('prompt');
  const rawIntegrations = params.get('integrations');
  const rawBilling = params.get('billing');
  if (!useCaseId && !rawPrompt && !rawIntegrations && !rawBilling) return null;

  let prompt = (rawPrompt || '').trim().slice(0, 2000);
  let matchedId = null;
  if (useCaseId && typeof NUX_DATA !== 'undefined') {
    const match = (NUX_DATA.useCases || []).find((u) => u.id === useCaseId);
    if (match) {
      matchedId = match.id;
      if (!prompt) prompt = match.prompt;
    }
  }
  if (prompt) sessionStorage.setItem(NUX_PENDING_PROMPT_KEY, prompt);
  if (matchedId) sessionStorage.setItem(NUX_PENDING_USECASE_KEY, matchedId);

  // MOCK connected-integration state from the onboarding handoff (see note
  // on NUX_CONNECTED_INTEGRATIONS_KEY above). Sanitized to known ids.
  if (rawIntegrations) {
    const known = (typeof NUX_DATA !== 'undefined' && NUX_DATA.integrationCatalog || [])
      .map((entry) => entry.id);
    const ids = rawIntegrations.split(',')
      .map((id) => id.trim().toLowerCase())
      .filter((id) => known.indexOf(id) !== -1);
    if (ids.length > 0) {
      sessionStorage.setItem(NUX_CONNECTED_INTEGRATIONS_KEY, JSON.stringify(ids));
    }
  }

  // MOCK billing state (see NUX_BILLING_KEY above). Sanitized to known
  // plan ids + 'skipped'.
  if (rawBilling) {
    const billing = rawBilling.trim().toLowerCase();
    const known = ['skipped', 'starter', 'basic', 'proplus'];
    if (known.indexOf(billing) !== -1) {
      try { localStorage.setItem(NUX_BILLING_KEY, billing); } catch (e) {}
    }
  }

  // Strip the intent params so refreshes/auth redirects don't re-apply them.
  params.delete('usecase');
  params.delete('prompt');
  params.delete('integrations');
  params.delete('billing');
  const query = params.toString();
  window.history.replaceState({}, '', window.location.pathname + (query ? '?' + query : '') + window.location.hash);

  return matchedId;
}

// Read the mock connected-integration ids captured from the onboarding
// handoff. Used by the Integrations surface and the chat empty state.
function getHandoffConnectedIntegrations() {
  try {
    const raw = sessionStorage.getItem(NUX_CONNECTED_INTEGRATIONS_KEY);
    const arr = raw ? JSON.parse(raw) : [];
    return Array.isArray(arr) ? arr : [];
  } catch (e) {
    return [];
  }
}

function getHandoffUseCaseId() {
  return sessionStorage.getItem(NUX_PENDING_USECASE_KEY) || null;
}

// MOCK billing state from the handoff: a plan id, 'skipped', or null when
// the user never came through the onboarding billing step.
function getHandoffBillingState() {
  try {
    return localStorage.getItem(NUX_BILLING_KEY) || null;
  } catch (e) {
    return null;
  }
}

const _nuxIntentUseCaseId = captureIntentFromUrl();

// Continuity cue on the auth card: show the captured intent so users see
// their request survives sign-in (and can discard it). Reduces the "did my
// click do anything?" doubt between marketing page and product.
function renderPendingIntentBanner() {
  const existing = document.getElementById('landing-intent');
  if (existing) existing.remove();
  const prompt = sessionStorage.getItem(NUX_PENDING_PROMPT_KEY);
  if (!prompt) return;
  const card = document.querySelector('.auth-card-login');
  if (!card) return;

  const banner = document.createElement('div');
  banner.className = 'landing-intent';
  banner.id = 'landing-intent';

  const label = document.createElement('span');
  label.className = 'landing-intent-label';
  label.textContent = I18n.t('landing.intentLabel');
  banner.appendChild(label);

  const text = document.createElement('span');
  text.className = 'landing-intent-text';
  text.textContent = prompt;
  text.title = prompt;
  banner.appendChild(text);

  const clear = document.createElement('button');
  clear.type = 'button';
  clear.className = 'landing-intent-clear';
  clear.setAttribute('aria-label', I18n.t('btn.cancel'));
  clear.textContent = '\u00d7';
  clear.addEventListener('click', () => {
    sessionStorage.removeItem(NUX_PENDING_PROMPT_KEY);
    banner.remove();
    document.querySelectorAll('.landing-usecase-card.selected').forEach((el) => {
      el.classList.remove('selected');
    });
  });
  banner.appendChild(clear);

  card.insertBefore(banner, card.firstChild);
}

function renderLandingUseCases() {
  const grid = document.getElementById('landing-usecases');
  if (!grid || typeof NUX_DATA === 'undefined') return;

  NUX_DATA.useCases.slice(0, 6).forEach((useCase) => {
    const card = document.createElement('button');
    card.type = 'button';
    card.className = 'landing-usecase-card'
      + (useCase.id === _nuxIntentUseCaseId ? ' selected' : '');
    card.setAttribute('data-usecase-id', useCase.id);

    const glyph = document.createElement('span');
    glyph.className = 'landing-usecase-glyph';
    glyph.setAttribute('aria-hidden', 'true');
    glyph.textContent = useCase.glyph;
    card.appendChild(glyph);

    const body = document.createElement('span');
    body.className = 'landing-usecase-body';

    const title = document.createElement('span');
    title.className = 'landing-usecase-title';
    title.textContent = useCase.title;
    body.appendChild(title);

    const desc = document.createElement('span');
    desc.className = 'landing-usecase-desc';
    desc.textContent = useCase.description;
    body.appendChild(desc);

    card.appendChild(body);

    card.addEventListener('click', () => {
      const wasSelected = card.classList.contains('selected');
      grid.querySelectorAll('.landing-usecase-card.selected').forEach((el) => {
        el.classList.remove('selected');
      });
      if (wasSelected) {
        sessionStorage.removeItem(NUX_PENDING_PROMPT_KEY);
        renderPendingIntentBanner();
        return;
      }
      card.classList.add('selected');
      sessionStorage.setItem(NUX_PENDING_PROMPT_KEY, useCase.prompt);
      renderPendingIntentBanner();
      // Nudge the user toward sign-in now that they've picked a starting point.
      const tokenInput = document.getElementById('token-input');
      const firstSocialBtn = document.querySelector('.auth-social-btn:not([hidden])');
      if (firstSocialBtn) {
        firstSocialBtn.focus();
      } else if (tokenInput && tokenInput.offsetParent !== null) {
        tokenInput.focus();
      }
    });

    grid.appendChild(card);
  });
}

renderLandingUseCases();
renderPendingIntentBanner();

// Called from initApp() after authentication: carries the prompt of the
// use case picked on the landing page into the chat input and sends it
// automatically, so the handoff flows straight into a working first
// conversation without an extra click. The seed is consumed here, so a
// refresh never re-sends it.
function applyPendingUseCasePrompt() {
  const prompt = sessionStorage.getItem(NUX_PENDING_PROMPT_KEY);
  if (!prompt) return;
  sessionStorage.removeItem(NUX_PENDING_PROMPT_KEY);
  const input = document.getElementById('chat-input');
  if (!input || input.disabled) return;
  input.value = prompt;
  // Reuse the existing input pipeline so autosize + send-button state update.
  input.dispatchEvent(new Event('input'));
  input.focus();
  autoSendPendingPrompt(input, prompt);
}

// Auto-send the handoff prompt once the chat is actually ready to accept it.
// Thread selection happens asynchronously (loadThreads → switchThread), so we
// poll briefly for a selected, writable thread. Bails out if the user edits
// or clears the composer in the meantime — their keystrokes win — or if the
// chat never becomes ready (the prompt then stays pre-filled as before).
function autoSendPendingPrompt(input, prompt) {
  const deadline = Date.now() + 10000;
  (function attempt() {
    if (input.value !== prompt) return;
    const ready = typeof currentThreadId !== 'undefined' && currentThreadId
      && !input.disabled
      && !(typeof authFlowPending !== 'undefined' && authFlowPending);
    if (ready && typeof sendMessage === 'function') {
      sendMessage();
      return;
    }
    if (Date.now() < deadline) setTimeout(attempt, 150);
  })();
}
