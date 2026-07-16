// --- Landing helpers (workspace redesign base) ---
// Intent capture, use-case gallery, and auto-send handoff live in the
// chat-first onboarding stack. Base keeps no-op accessors so Integrations /
// billing / welcome chips degrade gracefully without handoff state.

function getHandoffConnectedIntegrations() {
  return [];
}

function getHandoffUseCaseId() {
  return null;
}

function getHandoffBillingState() {
  return null;
}

function applyPendingUseCasePrompt() {
  // no-op in workspace redesign base
}
