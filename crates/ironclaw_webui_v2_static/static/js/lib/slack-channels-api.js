import { apiFetch } from "./api.js";

export const SLACK_ALLOWED_CHANNELS_PATH = "/api/webchat/v2/channels/slack/allowed";

export function normalizeSlackChannelIds(channelIds = []) {
  return Array.from(
    new Set(
      channelIds
        .map((channelId) => String(channelId || "").trim())
        .filter(Boolean),
    ),
  ).sort();
}

export function listSlackAllowedChannels() {
  return apiFetch(SLACK_ALLOWED_CHANNELS_PATH);
}

export function saveSlackAllowedChannels(channelIds) {
  return apiFetch(SLACK_ALLOWED_CHANNELS_PATH, {
    method: "PUT",
    body: JSON.stringify({
      channel_ids: channelIds,
    }),
  });
}

export function slackChannelPickerError(error, fallback) {
  return error?.payload?.error || error?.payload?.message || error?.message || fallback;
}
