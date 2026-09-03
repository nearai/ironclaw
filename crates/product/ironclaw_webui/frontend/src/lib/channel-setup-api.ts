// Shared helpers for the generic extension-setup API modules: sanitized
// error extraction and optional-field normalization. The per-channel
// setup/pairing modules these once served are gone — the unified channel
// model routes both through `{extension_id}`-parameterized endpoints
// (`extension-pairing-api.ts`, `WEBUI_V2_PATTERN_SETUP_EXTENSION`), and no
// route may name a channel.

type ChannelSetupErrorLike = {
  message?: unknown;
  payload?: {
    error?: unknown;
    message?: unknown;
  };
};

export function channelSetupError(error: unknown, fallback: string): string {
  const candidate = error as ChannelSetupErrorLike | null | undefined;
  return String(
    candidate?.payload?.error ||
      candidate?.payload?.message ||
      candidate?.message ||
      fallback,
  );
}

export function optionalString(value: unknown): string | null {
  const normalized = String(value || "").trim();
  return normalized ? normalized : null;
}
