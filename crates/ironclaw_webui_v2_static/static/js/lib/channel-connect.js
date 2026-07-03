import { apiFetch } from "./api.js";

export function listConnectableChannels() {
  return apiFetch("/api/webchat/v2/channels/connectable");
}

export function resolveChannelConnectCommand(input, channels) {
  if (!looksLikeChannelConnectCommand(input)) return null;
  const text = normalizedWords(input);
  const requireExplicitCommandAlias = explicitlyTargetsSlackChannelManagement(text);
  let best = null;
  for (const channel of channels || []) {
    if (!isChatCommandResolvable(channel)) continue;
    const matchLength = bestMatchingAliasLength(text, channel, {
      commandAliasesOnly: requireExplicitCommandAlias,
    });
    if (matchLength > (best?.matchLength || 0)) {
      best = { channel, matchLength };
    }
  }
  return best?.channel || null;
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

function commandAliases(channel, options = {}) {
  const aliases = Array.isArray(channel?.command_aliases)
    ? channel.command_aliases.filter(Boolean)
    : [];
  if (!options.channelManagementOnly) return aliases;
  return aliases.filter((alias) => explicitlyTargetsChannelManagement(normalizedWords(alias)));
}

function isChatCommandResolvable(channel) {
  return channel?.strategy !== "admin_managed_channels";
}

function explicitlyTargetsSlackChannelManagement(text) {
  return includesWordPhrase(text, "slack") && explicitlyTargetsChannelManagement(text);
}

function explicitlyTargetsChannelManagement(text) {
  return /(^|\s)(channel|channels|allowlist)(\s|$)/.test(text);
}

function normalizedWords(value) {
  return String(value || "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim()
    .replace(/\s+/g, " ");
}

function bestMatchingAliasLength(text, channel, options = {}) {
  const aliases = options.commandAliasesOnly
    ? commandAliases(channel, { channelManagementOnly: true })
    : channelAliases(channel);
  return aliases.reduce((best, alias) => {
    const phrase = normalizedWords(alias);
    return includesWordPhrase(text, phrase) ? Math.max(best, phrase.length) : best;
  }, 0);
}

function includesWordPhrase(text, phrase) {
  if (!phrase) return false;
  return ` ${text} `.includes(` ${phrase} `);
}
