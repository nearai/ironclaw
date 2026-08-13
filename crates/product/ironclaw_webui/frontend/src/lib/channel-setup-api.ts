// @ts-nocheck
// Shared helpers for the generic extension-setup API modules: sanitized
// error extraction and optional-field normalization. The per-channel
// setup/pairing modules these once served are gone — the unified channel
// model routes both through `{extension_id}`-parameterized endpoints
// (`extension-pairing-api.ts`, `WEBUI_V2_PATTERN_SETUP_EXTENSION`), and no
// route may name a channel.

export function channelSetupError(error, fallback) {
  return error?.payload?.error || error?.payload?.message || error?.message || fallback;
}

export function optionalString(value) {
  const normalized = String(value || "").trim();
  return normalized ? normalized : null;
}
