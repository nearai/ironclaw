import { apiFetch } from "./api.js";

export const SLACK_PAIRING_REDEEM_PATH =
  "/api/webchat/v2/extensions/pairing/redeem";

export function redeemSlackPairingCode(code) {
  return apiFetch(SLACK_PAIRING_REDEEM_PATH, {
    method: "POST",
    body: JSON.stringify({ channel: "slack", code }),
  }).then((response) => ({
    success: true,
    provider: response.provider,
    provider_user_id: response.provider_user_id,
    message: "Slack account connected.",
  }));
}
