import { apiFetch } from "./api.js";

export function listConnectableChannels() {
  return apiFetch("/api/webchat/v2/channels/connectable");
}

export function resolveChannelConnectCommand(input, channels) {
  if (!looksLikeChannelConnectCommand(input)) return null;
  const text = normalizedWords(input);
  return (channels || []).find((channel) =>
    channelAliases(channel).some((alias) => includesWordPhrase(text, normalizedWords(alias)))
  ) || null;
}

export function looksLikeChannelConnectCommand(input) {
  const text = normalizedWords(input);
  if (!text) return false;
  const intent = /(^|\s)(connect|link|pair|setup|set up)(\s|$)/.test(text);
  const target = /(^|\s)(account|channel|app|integration|slack|telegram|whatsapp)(\s|$)/.test(text);
  return intent && target;
}

function channelAliases(channel) {
  return [
    channel?.channel,
    channel?.display_name,
    ...(Array.isArray(channel?.command_aliases) ? channel.command_aliases : []),
  ].filter(Boolean);
}

function normalizedWords(value) {
  return String(value || "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim()
    .replace(/\s+/g, " ");
}

function includesWordPhrase(text, phrase) {
  if (!phrase) return false;
  return ` ${text} `.includes(` ${phrase} `);
}
