import { apiFetch } from "../../../lib/api.js";

export const PAIRING_REDEEM_PATH = "/api/webchat/v2/extensions/pairing/redeem";

export function redeemPairingCode(channel, code) {
  return apiFetch(PAIRING_REDEEM_PATH, {
    method: "POST",
    body: JSON.stringify({ channel, code }),
  }).then((response) => ({
    success: true,
    provider: response.provider,
    provider_user_id: response.provider_user_id,
  }));
}
