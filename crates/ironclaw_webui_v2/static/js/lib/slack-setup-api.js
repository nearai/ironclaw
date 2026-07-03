import { apiFetch } from "./api.js";

export const SLACK_SETUP_PATH = "/api/webchat/v2/channels/slack/setup";

export function getSlackSetup() {
  return apiFetch(SLACK_SETUP_PATH);
}

export function saveSlackSetup(setup) {
  const body = {
    installation_id: String(setup.installation_id || "").trim(),
    team_id: String(setup.team_id || "").trim(),
    api_app_id: String(setup.api_app_id || "").trim(),
    user_id: optionalString(setup.user_id),
    shared_subject_user_id: optionalString(setup.shared_subject_user_id),
  };
  const botToken = String(setup.bot_token || "").trim();
  const signingSecret = String(setup.signing_secret || "").trim();
  if (botToken) body.bot_token = botToken;
  if (signingSecret) body.signing_secret = signingSecret;
  return apiFetch(SLACK_SETUP_PATH, {
    method: "PUT",
    body: JSON.stringify(body),
  });
}

export function slackSetupError(error, fallback) {
  return error?.payload?.error || error?.payload?.message || error?.message || fallback;
}

function optionalString(value) {
  const normalized = String(value || "").trim();
  return normalized ? normalized : null;
}
