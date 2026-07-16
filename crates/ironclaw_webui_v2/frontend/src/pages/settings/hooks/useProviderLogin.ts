// @ts-nocheck
import { useQueryClient } from "@tanstack/react-query";
import React from "react";
import { useT } from "../../../lib/i18n";
import {
  completeNearaiWalletLogin,
  fetchLlmProviders,
  startCodexLogin,
  startNearaiLogin,
} from "../lib/settings-api";

const WALLET_LOGIN_TIMEOUT_MS = 300_000;

// NEAR AI's hosted auth (private.near.ai) rejects `frontend_callback` URLs that
// point at a loopback host, so its browser sign-in (GitHub / Google / NEAR
// Wallet) cannot complete from a local dev origin. Detect that origin so we can
// fail fast with a clear message on click — instead of opening a doomed tab and
// polling for five minutes only to hit the opaque error (issue #4705).
export function isLocalDevOrigin() {
  if (typeof window === "undefined" || !window.location) return false;
  const host = window.location.hostname;
  // `window.location.hostname` exposes IPv6 hosts without brackets (e.g.
  // `http://[::1]:3000/` -> `"::1"`), so a bracketed `"[::1]"` form never
  // appears here.
  //
  // The entire `127.0.0.0/8` block is loopback, not just `127.0.0.1` — some
  // setups serve the dev UI on `127.0.1.1` (Debian's default for the hostname)
  // or other `127.*` addresses. Matching only `127.0.0.1` would let those
  // origins open the doomed hosted-SSO flow and wait out the full timeout
  // instead of failing fast.
  return (
    host === "localhost" ||
    host === "0.0.0.0" ||
    host === "::1" ||
    /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host) ||
    host.endsWith(".localhost")
  );
}

function walletLoginChannelName() {
  const suffix =
    typeof window.crypto?.randomUUID === "function"
      ? window.crypto.randomUUID()
      : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return `nearai-wallet-login:${suffix}`;
}

// Isolated popup that connects a NEAR wallet and signs the NEAR AI login
// message. Resolves with the BroadcastChannel payload, or null if the user
// cancels, closes the window, or the deadline passes.
function awaitWalletSignature(popup, channelName) {
  return new Promise((resolve) => {
    if (typeof window.BroadcastChannel !== "function") {
      resolve(null);
      return;
    }
    const channel = new window.BroadcastChannel(channelName);
    const onMessage = (event) => {
      const data = event.data;
      if (!data || data.type !== "nearai-wallet-login") return;
      cleanup();
      resolve(data.ok ? data : null);
    };
    const closedTimer = setInterval(() => {
      if (popup && popup.closed) {
        cleanup();
        resolve(null);
      }
    }, 500);
    const timeout = setTimeout(() => {
      cleanup();
      resolve(null);
    }, WALLET_LOGIN_TIMEOUT_MS);
    function cleanup() {
      clearInterval(closedTimer);
      clearTimeout(timeout);
      channel.removeEventListener("message", onMessage);
      channel.close();
    }
    channel.addEventListener("message", onMessage);
  });
}

// How long to keep polling the snapshot for a login to land before giving up.
// NEAR AI is a quick browser redirect; Codex device codes live ~15 minutes.
const NEARAI_POLL_DEADLINE_MS = 300_000;
const CODEX_POLL_DEADLINE_MS = 900_000;
const POLL_INTERVAL_MS = 2000;

// Poll the LLM snapshot until `providerId` becomes the active provider, the
// login popup is closed, or the deadline passes. When a `popup` handle is
// given, a closed window short-circuits the wait so the UI recovers the instant
// the user cancels instead of staying disabled until the full deadline.
// Returns "active", "closed", or "timeout".
async function pollUntilActive(providerId, deadlineMs, popup) {
  const deadline = Date.now() + deadlineMs;
  // The popup can auto-close a beat before the snapshot flips active on a
  // successful sign-in, so keep confirming activation for a short grace window
  // after a close before concluding the user actually cancelled.
  let graceChecksAfterClose = 2;
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
    const snapshot = await fetchLlmProviders().catch(() => null);
    if (snapshot?.active?.provider_id === providerId) {
      return "active";
    }
    if (popup && popup.closed) {
      if (graceChecksAfterClose <= 0) {
        return "closed";
      }
      graceChecksAfterClose -= 1;
    }
  }
  return "timeout";
}

// Shared NEAR AI + OpenAI Codex login flows, surface-agnostic. The onboarding
// screen and the Settings → Inference tab both drive the same backend login
// endpoints; this hook owns the open-tab + poll-until-active choreography so the
// two surfaces stay in sync. `onSuccess` runs after the provider goes active
// (the onboarding screen navigates to chat; settings just lets the refreshed
// snapshot re-render the now-active card).
export function useProviderLogin({ onSuccess } = {}) {
  const t = useT();
  const queryClient = useQueryClient();

  const [nearaiBusy, setNearaiBusy] = React.useState(false);
  const [nearaiError, setNearaiError] = React.useState("");
  const [codexBusy, setCodexBusy] = React.useState(false);
  const [codexError, setCodexError] = React.useState("");
  const [codexCode, setCodexCode] = React.useState(null);

  // Clear every login flow's feedback before starting a new attempt. The
  // status surface renders the NEAR AI and Codex errors (plus the Codex device
  // code) together, and the three NEAR AI methods share one error slot, so a
  // failed attempt's message would otherwise linger while the user switches to
  // a different provider/method. Resetting on every start keeps the surface
  // scoped to the attempt in progress.
  const resetLoginFeedback = React.useCallback(() => {
    setNearaiError("");
    setCodexError("");
    setCodexCode(null);
  }, []);

  const finishActive = React.useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
    if (onSuccess) {
      onSuccess();
    }
  }, [queryClient, onSuccess]);

  const startNearai = React.useCallback(
    async (provider) => {
      resetLoginFeedback();
      if (isLocalDevOrigin()) {
        setNearaiError(t("onboarding.nearaiLocalSso"));
        return;
      }
      // Open the popup synchronously inside the click gesture: browsers only
      // allow gesture-time opens, so opening after the awaited backend call
      // would be blocked. Navigate the blank popup to the auth URL once it
      // returns. Sever `opener` (we keep the handle, so no `noopener` flag) as
      // reverse-tabnabbing defense before sending it to the external page.
      const popup = window.open("about:blank", "_blank");
      if (!popup) {
        setNearaiError(t("onboarding.nearaiFailed"));
        return;
      }
      try {
        popup.opener = null;
      } catch (_e) {
        // Ignore: some engines disallow setting opener; navigation still works.
      }
      setNearaiBusy(true);
      try {
        const { auth_url: authUrl } = await startNearaiLogin({
          provider,
          origin: window.location.origin,
        });
        popup.location.href = authUrl;
        const outcome = await pollUntilActive("nearai", NEARAI_POLL_DEADLINE_MS, popup);
        if (outcome === "active") {
          await finishActive();
          return;
        }
        // Cancelled (tab closed) or never completed: close the tab if it is
        // still open and surface a retryable error. `finally` clears the busy
        // flag, so the buttons re-enable for an immediate retry without a
        // page refresh.
        popup.close();
        setNearaiError(
          t(outcome === "closed" ? "onboarding.nearaiFailed" : "onboarding.nearaiTimeout")
        );
      } catch (_err) {
        popup.close();
        setNearaiError(t("onboarding.nearaiFailed"));
      } finally {
        setNearaiBusy(false);
      }
    },
    [finishActive, resetLoginFeedback, t]
  );

  // NEAR wallet login can't reuse the GitHub/Google redirect: NEP-413 signing
  // happens in the browser. Open the isolated wallet popup, wait for the signed
  // message, then relay it to the backend (which exchanges it for a NEAR AI
  // session token, makes NEAR AI active, and hot-swaps the provider).
  const startNearaiWallet = React.useCallback(async () => {
    // Unlike the GitHub/Google hosted SSO flow, wallet login does NOT depend on
    // a NEAR AI `frontend_callback` redirect (which rejects loopback origins):
    // NEP-413 signing happens in a same-origin popup and the signed message is
    // relayed through our own backend. So it works on localhost — no local-dev
    // guard here.
    resetLoginFeedback();
    setNearaiBusy(true);
    try {
      const channelName = walletLoginChannelName();
      // Keep the window handle (no `noopener`/`noreferrer`, which would make
      // `window.open` return null) so `awaitWalletSignature` can detect the
      // user closing the popup instead of waiting out the full timeout. The
      // popup is a same-origin route we control, so the handle is safe.
      const popup = window.open(
        `/wallet/connect?channel=${encodeURIComponent(channelName)}`,
        "_blank",
        "width=460,height=640"
      );
      // A popup blocker makes window.open return null; fail fast instead of
      // waiting out the full signature timeout on a window that never opened.
      if (!popup) {
        setNearaiError(t("onboarding.nearaiFailed"));
        return;
      }
      // Defense-in-depth against reverse tabnabbing: sever the child's back
      // reference to this window. The wallet page reports back over
      // BroadcastChannel, not window.opener, so this doesn't affect the flow.
      popup.opener = null;
      const signed = await awaitWalletSignature(popup, channelName);
      if (!signed) {
        setNearaiError(t("onboarding.nearaiFailed"));
        return;
      }
      await completeNearaiWalletLogin({
        account_id: signed.accountId,
        public_key: signed.publicKey,
        signature: signed.signature,
        message: signed.message,
        recipient: signed.recipient,
        nonce: signed.nonce,
      });
      await finishActive();
    } catch (_err) {
      setNearaiError(t("onboarding.nearaiFailed"));
    } finally {
      setNearaiBusy(false);
    }
  }, [finishActive, resetLoginFeedback, t]);

  const startCodex = React.useCallback(async () => {
    resetLoginFeedback();
    // Open the popup synchronously inside the click gesture: browsers only
    // allow gesture-time opens, so opening after the awaited backend call could
    // be blocked. Keep the handle (no `noopener`, which would null the return)
    // so a closed tab can short-circuit the wait instead of leaving the button
    // disabled for the full device-code deadline. Sever `opener` while the
    // popup is still same-origin `about:blank` — setting it on the cross-origin
    // verification page can be rejected, so nulling it before navigating is
    // what closes the reverse-tabnabbing hole. If the popup is blocked, `popup`
    // is null and the wait falls back to polling alone; the device code is
    // still shown so the user can complete it elsewhere. Mirrors the NEAR AI
    // flow above.
    const popup = window.open("about:blank", "_blank");
    if (popup) {
      try {
        popup.opener = null;
      } catch (_e) {
        // Ignore: some engines disallow setting opener; navigation still works.
      }
    }
    setCodexBusy(true);
    try {
      const { user_code: userCode, verification_uri: verificationUri } =
        await startCodexLogin();
      setCodexCode({ userCode, verificationUri });
      if (popup) {
        popup.location.href = verificationUri;
      }
      const outcome = await pollUntilActive("openai_codex", CODEX_POLL_DEADLINE_MS, popup);
      if (outcome === "active") {
        await finishActive();
        return;
      }
      if (popup) {
        popup.close();
      }
      setCodexError(
        t(outcome === "closed" ? "onboarding.codexFailed" : "onboarding.codexTimeout")
      );
    } catch (_err) {
      if (popup) {
        popup.close();
      }
      setCodexError(t("onboarding.codexFailed"));
    } finally {
      setCodexBusy(false);
    }
  }, [finishActive, resetLoginFeedback, t]);

  return {
    nearaiBusy,
    nearaiError,
    codexBusy,
    codexError,
    codexCode,
    startNearai,
    startNearaiWallet,
    startCodex,
  };
}
