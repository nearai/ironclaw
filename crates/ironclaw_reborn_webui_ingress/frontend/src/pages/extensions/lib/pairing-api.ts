// @ts-nocheck
import { apiFetch } from "../../../lib/api";
import { notifyChannelConnected } from "../../../lib/channel-connection-events";

// SCAFFOLDING: no Reborn backend mounts this route today. The only mount was
// the Slack-only pairing redeem removed by PR #5604 (Slack now connects via
// OAuth), and no shipped non-Slack channel reaches the proof-code UI — the
// backend cannot emit a `pairing`/`pairing_required` onboarding state and only
// Slack declares an inbound product adapter. This generic proof-code path is
// retained for the first non-Slack inbound channel, which must mount a generic
// redeem route here when it lands; until then a POST 404s.
export const PAIRING_REDEEM_PATH = "/api/webchat/v2/extensions/pairing/redeem";

export function redeemPairingCode(channel, code, options = {}) {
  // The redeem body carries only what the server reads. Channel connection is a
  // per-user gate: the server resumes every run this caller parked on the
  // channel, so it does not scope resume by thread/request. `options.threadId`
  // is used below only for the client-side connection broadcast — sending it in
  // the request body would be silently ignored, so we don't.
  const body = { channel, code };
  return apiFetch(PAIRING_REDEEM_PATH, {
    method: "POST",
    body: JSON.stringify(body),
  }).then(async (response) => {
    await notifyChannelConnected({
      channel,
      provider: response.provider,
      providerUserId: response.provider_user_id,
      sourceThreadId: options.threadId || null,
      source: options.source || "webui",
    });
    return {
      success: true,
      provider: response.provider,
      provider_user_id: response.provider_user_id,
      // The binding is durable, so the connection succeeded. `resumeError` is
      // true when the follow-up resume faulted (a still-parked chat couldn't be
      // continued); surfaced, not dropped, so the caller can react.
      resumeError: Boolean(response.resume_error),
      resumedRunCount: response.resumed_run_count ?? 0,
    };
  });
}
