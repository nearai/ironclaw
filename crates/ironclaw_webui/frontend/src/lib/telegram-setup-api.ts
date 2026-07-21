// @ts-nocheck
import { apiFetch } from "./api";
import { channelSetupError, optionalString } from "./channel-setup-api";

export const TELEGRAM_SETUP_PATH = "/api/webchat/v2/channels/telegram/setup";
export const TELEGRAM_PAIRING_PATH = "/api/webchat/v2/channels/telegram/pairing";

// -> { configured, bot_username, bot_token_configured, webhook_url, revision }
export function getTelegramSetup() {
  return apiFetch(TELEGRAM_SETUP_PATH);
}

// PUT body: { bot_token?, webhook_url? }. A blank/omitted bot_token keeps the
// stored secret, so the form never has to echo it back; the webhook override
// rides as an explicit null when cleared, mirroring the optional Slack fields.
export function saveTelegramSetup(setup) {
  const body = {
    webhook_url: optionalString(setup.webhook_url),
  };
  const botToken = String(setup.bot_token || "").trim();
  if (botToken) body.bot_token = botToken;
  return apiFetch(TELEGRAM_SETUP_PATH, {
    method: "PUT",
    body: JSON.stringify(body),
  });
}

// -> 204; removes the bot token + webhook registration.
export function clearTelegramSetup() {
  return apiFetch(TELEGRAM_SETUP_PATH, { method: "DELETE" });
}

// -> { code, deep_link, expires_at }; mints (or rotates) the caller's code.
export function startTelegramPairing() {
  return apiFetch(TELEGRAM_PAIRING_PATH, { method: "POST" });
}

// -> { connected, pending: { code, deep_link, expires_at } | null }
export function getTelegramPairing() {
  return apiFetch(TELEGRAM_PAIRING_PATH);
}

// -> 204; unpairs the caller's Telegram account.
export function disconnectTelegramPairing() {
  return apiFetch(TELEGRAM_PAIRING_PATH, { method: "DELETE" });
}

export const telegramSetupError = channelSetupError;
