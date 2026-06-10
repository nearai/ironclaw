import { useQueryClient } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import {
  completeNearaiWalletLogin,
  fetchLlmProviders,
  startCodexLogin,
  startNearaiLogin,
} from "../lib/settings-api.js";

const WALLET_LOGIN_TIMEOUT_MS = 300_000;

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

// Poll the LLM snapshot until `providerId` becomes the active provider, or the
// deadline passes. Returns true on success.
async function pollUntilActive(providerId, deadlineMs) {
  const deadline = Date.now() + deadlineMs;
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
    const snapshot = await fetchLlmProviders().catch(() => null);
    if (snapshot?.active?.provider_id === providerId) {
      return true;
    }
  }
  return false;
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

  const finishActive = React.useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
    if (onSuccess) {
      onSuccess();
    }
  }, [queryClient, onSuccess]);

  const startNearai = React.useCallback(
    async (provider) => {
      setNearaiError("");
      setNearaiBusy(true);
      try {
        const { auth_url: authUrl } = await startNearaiLogin({
          provider,
          origin: window.location.origin,
        });
        window.open(authUrl, "_blank", "noopener");
        if (await pollUntilActive("nearai", NEARAI_POLL_DEADLINE_MS)) {
          await finishActive();
          return;
        }
        setNearaiError(t("onboarding.nearaiTimeout"));
      } catch (_err) {
        setNearaiError(t("onboarding.nearaiFailed"));
      } finally {
        setNearaiBusy(false);
      }
    },
    [finishActive, t]
  );

  // NEAR wallet login can't reuse the GitHub/Google redirect: NEP-413 signing
  // happens in the browser. Open the isolated wallet popup, wait for the signed
  // message, then relay it to the backend (which exchanges it for a NEAR AI
  // session token, makes NEAR AI active, and hot-swaps the provider).
  const startNearaiWallet = React.useCallback(async () => {
    setNearaiError("");
    setNearaiBusy(true);
    try {
      const channelName = walletLoginChannelName();
      // Keep the window handle (no `noopener`/`noreferrer`, which would make
      // `window.open` return null) so `awaitWalletSignature` can detect the
      // user closing the popup instead of waiting out the full timeout. The
      // popup is a same-origin route we control, so the handle is safe.
      const popup = window.open(
        `/v2/wallet/connect?channel=${encodeURIComponent(channelName)}`,
        "_blank",
        "width=460,height=640"
      );
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
  }, [finishActive, t]);

  const startCodex = React.useCallback(async () => {
    setCodexError("");
    setCodexCode(null);
    setCodexBusy(true);
    try {
      const { user_code: userCode, verification_uri: verificationUri } =
        await startCodexLogin();
      setCodexCode({ userCode, verificationUri });
      window.open(verificationUri, "_blank", "noopener");
      if (await pollUntilActive("openai_codex", CODEX_POLL_DEADLINE_MS)) {
        await finishActive();
        return;
      }
      setCodexError(t("onboarding.codexTimeout"));
    } catch (_err) {
      setCodexError(t("onboarding.codexFailed"));
    } finally {
      setCodexBusy(false);
    }
  }, [finishActive, t]);

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
