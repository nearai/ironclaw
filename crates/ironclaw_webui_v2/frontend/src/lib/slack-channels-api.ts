import { apiFetch } from "./api.js";

export const SLACK_ALLOWED_CHANNELS_PATH = "/api/webchat/v2/channels/slack/allowed";
export const SLACK_ROUTABLE_SUBJECTS_PATH = "/api/webchat/v2/channels/slack/subjects";

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

export function listSlackRoutableSubjects() {
  return apiFetch(SLACK_ROUTABLE_SUBJECTS_PATH);
}

export function saveSlackAllowedChannels(channels) {
  const hasStructuredChannels = channels.some((channel) => typeof channel !== "string");
  const normalized = channels.map((channel) => {
    if (typeof channel === "string") return { channel_id: channel };
    return {
      channel_id: channel.channel_id,
      subject_user_id: channel.subject_user_id,
    };
  });
  const body = hasStructuredChannels
    ? { channels: normalized }
    : { channel_ids: normalized.map((channel) => channel.channel_id) };
  return apiFetch(SLACK_ALLOWED_CHANNELS_PATH, {
    method: "PUT",
    body: JSON.stringify(body),
  });
}

export function slackChannelPickerError(error, fallback) {
  return error?.payload?.error || error?.payload?.message || error?.message || fallback;
}
