import { apiFetch } from "../../../lib/api.js";

export const SLACK_PAIRING_REDEEM_PATH =
  "/api/reborn/slack/personal-binding/pairing/redeem";

export function redeemSlackPairingCode(code) {
  return apiFetch(SLACK_PAIRING_REDEEM_PATH, {
    method: "POST",
    body: JSON.stringify({ code }),
  }).then((response) => ({
    success: true,
    provider: response.provider,
    provider_user_id: response.provider_user_id,
    message: "Slack account connected.",
  }));
}
