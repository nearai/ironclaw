// @ts-nocheck
// Shared helpers for the per-channel setup API modules (slack-setup-api,
// telegram-setup-api): sanitized error extraction and optional-field
// normalization. Channel-specific endpoints stay in their own modules.

export function channelSetupError(error, fallback) {
  return error?.payload?.error || error?.payload?.message || error?.message || fallback;
}

export function optionalString(value) {
  const normalized = String(value || "").trim();
  return normalized ? normalized : null;
}
