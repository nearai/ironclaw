import { apiFetch } from "./api.js";
import { notifyChannelConnected } from "./channel-connection-events.js";

export const SLACK_PAIRING_REDEEM_PATH =
  "/api/webchat/v2/extensions/pairing/redeem";

export function redeemSlackPairingCode(code, options = {}) {
  const body = { channel: "slack", code };
  if (options.threadId) body.thread_id = options.threadId;
  if (options.requestId) body.request_id = options.requestId;
  return apiFetch(SLACK_PAIRING_REDEEM_PATH, {
    method: "POST",
    body: JSON.stringify(body),
  }).then((response) => {
    notifyChannelConnected({
      channel: "slack",
      provider: response.provider,
      providerUserId: response.provider_user_id,
      sourceThreadId: options.threadId || null,
      source: options.source || "webui",
    });
    return {
      success: true,
      provider: response.provider,
      provider_user_id: response.provider_user_id,
      message: "Slack account connected.",
    };
  });
}
