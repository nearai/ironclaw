// --- Billing / credits (MOCK) ---
//
// Everything in this module is a client-side mock driven by the onboarding
// handoff (`?billing=<plan-id|skipped>`, see landing.js) and usage:
//   - $5.00 of free credits by default; each agent message burns a small
//     amount so the sidebar ring visibly moves while using the agent.
//   - The Billing surface shows plan, usage breakdown, and per-day spend.
//   - When billing was skipped during onboarding, a one-per-session
//     reminder modal appears after the 3rd message or when credits run low.
// No backend billing API exists yet; state lives in localStorage.
//
// All mock billing CONTENT (plan cards, credit amounts, thresholds) lives in
// js/core/mock-backend.js (`window.NUX_BILLING`) — the single mock-data file.
// This module is only the rendering/state logic.

const BILLING_CREDITS_KEY = 'ironclaw_nux_credits';
const BILLING_REMINDER_SHOWN_KEY = 'ironclaw_nux_billing_reminder_shown';
const BILLING_SESSION_MSGS_KEY = 'ironclaw_nux_billing_session_msgs';
const BILLING_FREE_TOTAL = window.NUX_BILLING.freeCreditsUsd;
const BILLING_LOW_THRESHOLD = window.NUX_BILLING.lowBalanceThresholdUsd;
const BILLING_REMINDER_AFTER_MSGS = window.NUX_BILLING.reminderAfterMessages;
const BILLING_PLANS = window.NUX_BILLING.plans;

function billingGetState() {
  let state = null;
  try {
    state = JSON.parse(localStorage.getItem(BILLING_CREDITS_KEY) || 'null');
  } catch (e) {}
  if (!state || typeof state.used !== 'number') {
    state = { total: BILLING_FREE_TOTAL, used: 0, byDay: {} };
  }
  if (typeof state.total !== 'number') state.total = BILLING_FREE_TOTAL;
  if (!state.byDay) state.byDay = {};
  return state;
}

function billingSaveState(state) {
  try { localStorage.setItem(BILLING_CREDITS_KEY, JSON.stringify(state)); } catch (e) {}
}

function billingRemaining(state) {
  const s = state || billingGetState();
  return Math.max(0, s.total - s.used);
}

function billingPlan() {
  const billing = typeof getHandoffBillingState === 'function' ? getHandoffBillingState() : null;
  if (!billing || billing === 'skipped') return null;
  return BILLING_PLANS.find((p) => p.id === billing) || null;
}

function billingFormatUsd(value) {
  return '$' + value.toFixed(2);
}

// Called from sendMessage (chat.js) on every agent message. MOCK: burns a
// small pseudo-random amount so the ring moves at human-visible speed.
function billingRecordUsage() {
  const state = billingGetState();
  const cost = 0.05 + Math.random() * 0.09;
  state.used = Math.min(state.total, state.used + cost);
  const day = new Date().toISOString().slice(0, 10);
  state.byDay[day] = (state.byDay[day] || 0) + cost;
  billingSaveState(state);
  renderSidebarCredits();
  if (currentTab === 'billing') renderBillingSurface();

  // Payment reminder (once per session, only when billing was skipped).
  let msgs = 0;
  try {
    msgs = parseInt(sessionStorage.getItem(BILLING_SESSION_MSGS_KEY) || '0', 10) + 1;
    sessionStorage.setItem(BILLING_SESSION_MSGS_KEY, String(msgs));
  } catch (e) {}
  const skipped = typeof getHandoffBillingState === 'function'
    && getHandoffBillingState() === 'skipped';
  const low = billingRemaining(state) < BILLING_LOW_THRESHOLD;
  if (skipped && (msgs >= BILLING_REMINDER_AFTER_MSGS || low)) {
    maybeShowBillingReminder();
  }
}

// --- Radial progress ring (shared by sidebar + billing page + modal) ---

function billingRingSvg(fraction, size, stroke) {
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const filled = Math.max(0, Math.min(1, fraction)) * c;
  return '<svg class="credits-ring" width="' + size + '" height="' + size + '" viewBox="0 0 ' + size + ' ' + size + '">'
    + '<circle class="credits-ring-track" cx="' + (size / 2) + '" cy="' + (size / 2) + '" r="' + r + '" stroke-width="' + stroke + '"/>'
    + '<circle class="credits-ring-fill" cx="' + (size / 2) + '" cy="' + (size / 2) + '" r="' + r + '" stroke-width="' + stroke + '"'
    + ' stroke-dasharray="' + filled.toFixed(2) + ' ' + c.toFixed(2) + '"'
    + ' transform="rotate(-90 ' + (size / 2) + ' ' + (size / 2) + ')"/>'
    + '</svg>';
}

function renderSidebarCredits() {
  const btn = document.getElementById('sidebar-credits');
  if (!btn) return;
  const state = billingGetState();
  const remaining = billingRemaining(state);
  const fractionLeft = state.total > 0 ? remaining / state.total : 0;
  const plan = billingPlan();
  btn.classList.toggle('low', remaining < BILLING_LOW_THRESHOLD);
  // Compact widget: ring + amount, plan detail in the tooltip.
  btn.title = I18n.t('billing.ringTooltip', {
    remaining: billingFormatUsd(remaining),
    total: billingFormatUsd(state.total),
  }) + ' \u00b7 ' + (plan ? plan.name : I18n.t('billing.freeCredits'));
  btn.innerHTML = billingRingSvg(fractionLeft, 16, 2.5)
    + '<span class="sidebar-credits-amount">' + escapeHtml(billingFormatUsd(remaining)) + '</span>';
}

document.getElementById('sidebar-credits')?.addEventListener('click', () => {
  switchTab('billing');
});

// --- Billing surface ---

function renderBillingSurface() {
  const container = document.getElementById('billing-container');
  if (!container) return;
  const state = billingGetState();
  const remaining = billingRemaining(state);
  const fractionLeft = state.total > 0 ? remaining / state.total : 0;
  const usedPct = state.total > 0 ? Math.min(100, (state.used / state.total) * 100) : 0;
  const plan = billingPlan();

  let html = '';

  // Current plan + usage card.
  html += '<div class="billing-grid">'
    + '<div class="billing-card billing-usage-card">'
    + '<div class="billing-card-label">' + escapeHtml(I18n.t('billing.currentPlan')) + '</div>'
    + '<div class="billing-plan-name">'
    + escapeHtml(plan ? (plan.name + ' \u2014 ' + plan.price + plan.period) : I18n.t('billing.noPlan'))
    + '</div>'
    + '<div class="billing-usage-row">'
    + '<div class="billing-usage-ring">' + billingRingSvg(fractionLeft, 84, 8)
    + '<div class="billing-usage-ring-center"><span>' + escapeHtml(billingFormatUsd(remaining)) + '</span><small>' + escapeHtml(I18n.t('billing.left')) + '</small></div>'
    + '</div>'
    + '<div class="billing-usage-detail">'
    + '<div class="billing-usage-line"><span>' + escapeHtml(I18n.t('billing.used')) + '</span><strong>' + escapeHtml(billingFormatUsd(state.used)) + '</strong></div>'
    + '<div class="billing-usage-line"><span>' + escapeHtml(I18n.t('billing.total')) + '</span><strong>' + escapeHtml(billingFormatUsd(state.total)) + '</strong></div>'
    + '<div class="billing-usage-bar"><div class="billing-usage-bar-fill" style="width:' + usedPct.toFixed(1) + '%"></div></div>'
    + '<button type="button" class="billing-add-credits" id="billing-add-credits">' + escapeHtml(I18n.t('billing.addCredits')) + '</button>'
    + '</div>'
    + '</div>'
    + '</div>';

  // Usage by day (MOCK data accumulated client-side).
  const days = Object.keys(state.byDay).sort().slice(-7);
  html += '<div class="billing-card">'
    + '<div class="billing-card-label">' + escapeHtml(I18n.t('billing.usageByDay')) + '</div>';
  if (days.length === 0) {
    html += '<div class="billing-empty">' + escapeHtml(I18n.t('billing.noUsage')) + '</div>';
  } else {
    const max = Math.max(...days.map((d) => state.byDay[d]));
    html += '<div class="billing-spark">' + days.map((d) => {
      const v = state.byDay[d];
      const h = max > 0 ? Math.max(8, (v / max) * 100) : 8;
      return '<div class="billing-spark-col" title="' + escapeHtml(d + ' \u2014 ' + billingFormatUsd(v)) + '">'
        + '<div class="billing-spark-bar" style="height:' + h.toFixed(0) + '%"></div>'
        + '<span class="billing-spark-day">' + escapeHtml(d.slice(5)) + '</span>'
        + '</div>';
    }).join('') + '</div>';
  }
  html += '</div></div>';

  // Upgrade plans.
  html += '<div class="billing-card-label billing-plans-label">' + escapeHtml(I18n.t('billing.upgradeHeading')) + '</div>';
  html += '<div class="billing-plans">' + BILLING_PLANS.map((p) => billingPlanCardHtml(p, plan)).join('') + '</div>';

  container.innerHTML = html;
  container.querySelector('#billing-add-credits')?.addEventListener('click', () => {
    // MOCK: tops the balance up locally.
    const s = billingGetState();
    s.total += 5;
    billingSaveState(s);
    renderSidebarCredits();
    renderBillingSurface();
    showToast(I18n.t('billing.creditsAdded'), 'success');
  });
  bindPlanButtons(container);
}

function billingPlanCardHtml(p, currentPlan) {
  const isCurrent = currentPlan && currentPlan.id === p.id;
  return '<div class="billing-plan-card' + (p.recommended ? ' recommended' : '') + '">'
    + (p.recommended ? '<span class="billing-plan-flag">' + escapeHtml(I18n.t('billing.recommended')) + '</span>' : '')
    + '<div class="billing-plan-title">' + escapeHtml(p.name) + '</div>'
    + '<div class="billing-plan-price">' + escapeHtml(p.price) + '<small>' + escapeHtml(p.period) + '</small></div>'
    + '<div class="billing-plan-credits">' + escapeHtml(p.credits) + '</div>'
    + '<div class="billing-plan-blurb">' + escapeHtml(p.blurb) + '</div>'
    + '<button type="button" class="billing-plan-btn' + (p.recommended ? ' primary' : '') + '" data-plan-id="' + escapeHtml(p.id) + '"'
    + (isCurrent ? ' disabled' : '') + '>'
    + escapeHtml(isCurrent ? I18n.t('billing.currentPlanBtn') : I18n.t('billing.upgrade'))
    + '</button>'
    + '</div>';
}

function bindPlanButtons(scope, onUpgraded) {
  scope.querySelectorAll('.billing-plan-btn[data-plan-id]').forEach((btn) => {
    btn.addEventListener('click', () => {
      // MOCK: "upgrading" just stores the plan id locally.
      const planId = btn.getAttribute('data-plan-id');
      try { localStorage.setItem('ironclaw_nux_billing', planId); } catch (e) {}
      closeBillingReminder();
      closePlanPicker();
      renderSidebarCredits();
      if (currentTab === 'billing') renderBillingSurface();
      showToast(I18n.t('billing.upgraded'), 'success');
      if (typeof onUpgraded === 'function') {
        const plan = BILLING_PLANS.find((p) => p.id === planId);
        if (plan) onUpgraded(plan);
      }
    });
  });
}

// --- Plan picker modal (in-flow upgrade CTA from the chat-first journey) ---

function showPlanPickerModal(onUpgraded) {
  closePlanPicker();
  const overlay = document.createElement('div');
  overlay.id = 'billing-plan-picker';
  overlay.className = 'billing-reminder-overlay';
  overlay.innerHTML =
    '<div class="billing-reminder billing-plan-picker" role="dialog" aria-modal="true" aria-label="' + escapeHtml(I18n.t('billing.pickerTitle')) + '">'
    + '<h3 class="billing-reminder-title">' + escapeHtml(I18n.t('billing.pickerTitle')) + '</h3>'
    + '<p class="billing-reminder-text">' + escapeHtml(I18n.t('billing.pickerText')) + '</p>'
    + '<div class="billing-plans billing-reminder-plans">' + BILLING_PLANS.map((p) => billingPlanCardHtml(p, billingPlan())).join('') + '</div>'
    + '<div class="billing-reminder-actions">'
    + '<button type="button" class="billing-reminder-later" id="billing-picker-later">' + escapeHtml(I18n.t('billing.notNow')) + '</button>'
    + '</div>'
    + '</div>';
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) closePlanPicker();
  });
  document.body.appendChild(overlay);
  bindPlanButtons(overlay, onUpgraded);
  overlay.querySelector('#billing-picker-later').addEventListener('click', () => closePlanPicker());
}

function closePlanPicker() {
  document.getElementById('billing-plan-picker')?.remove();
}

// --- Payment reminder modal (billing=skipped, max once per session) ---

function maybeShowBillingReminder() {
  try {
    if (sessionStorage.getItem(BILLING_REMINDER_SHOWN_KEY) === '1') return;
    sessionStorage.setItem(BILLING_REMINDER_SHOWN_KEY, '1');
  } catch (e) {}
  showBillingReminder();
}

function showBillingReminder() {
  closeBillingReminder();
  const state = billingGetState();
  const remaining = billingRemaining(state);
  const fractionLeft = state.total > 0 ? remaining / state.total : 0;

  const overlay = document.createElement('div');
  overlay.id = 'billing-reminder-modal';
  overlay.className = 'billing-reminder-overlay';
  overlay.innerHTML =
    '<div class="billing-reminder" role="dialog" aria-modal="true" aria-label="' + escapeHtml(I18n.t('billing.reminderTitle')) + '">'
    + '<div class="billing-reminder-ring">' + billingRingSvg(fractionLeft, 72, 7)
    + '<div class="billing-usage-ring-center"><span>' + escapeHtml(billingFormatUsd(remaining)) + '</span><small>' + escapeHtml(I18n.t('billing.left')) + '</small></div>'
    + '</div>'
    + '<h3 class="billing-reminder-title">' + escapeHtml(I18n.t('billing.reminderTitle')) + '</h3>'
    + '<p class="billing-reminder-text">' + escapeHtml(I18n.t('billing.reminderText', {
      remaining: billingFormatUsd(remaining),
      total: billingFormatUsd(state.total),
    })) + '</p>'
    + '<div class="billing-plans billing-reminder-plans">' + BILLING_PLANS.map((p) => billingPlanCardHtml(p, billingPlan())).join('') + '</div>'
    + '<div class="billing-reminder-actions">'
    + '<button type="button" class="billing-reminder-later" id="billing-reminder-later">' + escapeHtml(I18n.t('billing.notNow')) + '</button>'
    + '<button type="button" class="billing-reminder-upgrade" id="billing-reminder-upgrade">' + escapeHtml(I18n.t('billing.upgrade')) + '</button>'
    + '</div>'
    + '</div>';

  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) closeBillingReminder();
  });
  document.body.appendChild(overlay);
  bindPlanButtons(overlay);
  overlay.querySelector('#billing-reminder-later').addEventListener('click', () => closeBillingReminder());
  overlay.querySelector('#billing-reminder-upgrade').addEventListener('click', () => {
    closeBillingReminder();
    switchTab('billing');
  });
}

function closeBillingReminder() {
  document.getElementById('billing-reminder-modal')?.remove();
}

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    closeBillingReminder();
    closePlanPicker();
  }
});

// Initial sidebar render (deferred a tick so I18n strings are loaded).
setTimeout(() => renderSidebarCredits(), 0);
