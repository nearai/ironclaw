// @ts-nocheck
import React from "react";
import {
  channelConnectionContinuationMessage,
  connectionEventMatchesOnboarding,
  forgetChannelConnectionWaiter,
  normalizeConnectionChannel,
  notifyChannelConnected,
  rememberChannelConnectionWaiter,
  subscribeChannelConnected,
} from "../../../lib/channel-connection-events";
import {
  completionMatchesFlow,
  failureMatchesFlow,
  openAuthPopup,
  readLatestProductAuthOAuthCompletion,
  subscribeProductAuthOAuthCompletion,
} from "../../../lib/product-auth-oauth-events";
import { queryClient } from "../../../lib/query-client";
import {
  fetchExtensionSetup,
  fetchExtensions,
  fetchOauthFlowStatus,
  startExtensionOauth,
} from "../../extensions/lib/extensions-api";
import { redeemPairingCode } from "../../extensions/lib/pairing-api";

const DISMISSED_ONBOARDING_STORAGE_PREFIX =
  "ironclaw.chat.dismissedOnboarding.v1:";
const DISMISSED_ONBOARDING_STORAGE_LIMIT = 100;

// In-chat OAuth watcher bounds. Mirror the Extensions page watcher
// (`OAUTH_SETUP_TIMEOUT_MS` / `OAUTH_SETUP_REFRESH_MS` in useExtensions.ts) so
// an abandoned popup cannot leave the card polling the server forever.
const CHAT_OAUTH_TIMEOUT_MS = 10 * 60 * 1000;
const CHAT_OAUTH_POLL_MS = 2000;
const CHAT_OAUTH_FAILED_MESSAGE = "Authorization failed. Try connecting again.";
const CHAT_OAUTH_TIMED_OUT_MESSAGE =
  "Authorization timed out. Try connecting again.";

// A pairing panel is per-thread. Exported because `useChat`'s send-admission and
// visible-panel computation share this exact rule with the onboarding hook.
export function onboardingBelongsToThread(onboarding, threadId) {
  if (!onboarding) return false;
  const onboardingThreadId = String(onboarding.threadId || "").trim();
  // An onboarding with no thread id belongs to no thread, so it must not leak
  // onto every chat the user opens — the live derive path always stamps the
  // current thread, so this only guards a malformed writer.
  if (!onboardingThreadId) return false;
  const currentThreadId = String(threadId || "").trim();
  return Boolean(currentThreadId) && onboardingThreadId === currentThreadId;
}

function dismissedOnboardingStorageKey(threadId) {
  const normalized = String(threadId || "").trim();
  return normalized ? `${DISMISSED_ONBOARDING_STORAGE_PREFIX}${normalized}` : null;
}

function onboardingStorage() {
  try {
    return globalThis?.localStorage || null;
  } catch (_) {
    return null;
  }
}

function loadDismissedOnboardingIds(threadId) {
  const key = dismissedOnboardingStorageKey(threadId);
  const storage = key ? onboardingStorage() : null;
  if (!storage) return new Set();
  try {
    const parsed = JSON.parse(storage.getItem(key) || "[]");
    if (!Array.isArray(parsed)) return new Set();
    return new Set(parsed.filter((value) => typeof value === "string"));
  } catch (_) {
    return new Set();
  }
}

function persistDismissedOnboardingId(threadId, sourceMessageId) {
  const key = dismissedOnboardingStorageKey(threadId);
  const storage = key ? onboardingStorage() : null;
  if (!storage || !sourceMessageId) return;
  const dismissed = loadDismissedOnboardingIds(threadId);
  dismissed.add(sourceMessageId);
  const values = Array.from(dismissed).slice(-DISMISSED_ONBOARDING_STORAGE_LIMIT);
  try {
    storage.setItem(key, JSON.stringify(values));
  } catch (_) {
    // Best-effort UX preference; storage failures should not block chat.
  }
}

// The backend marks an `extension_activate` result for a connectable channel
// with this `output_kind`; the structured connect action rides on the card's
// `toolResultPreview` JSON. This is the structured replacement for the deleted
// prose regex — the panel is derived from typed fields, never from message text.
const CHANNEL_CONNECTION_REQUIRED_OUTPUT_KIND = "channel_connection_required";

function channelConnectionRequirementFromCard(card) {
  if (!card || card.outputKind !== CHANNEL_CONNECTION_REQUIRED_OUTPUT_KIND) return null;
  if (card.toolStatus && card.toolStatus !== "success") return null;
  if (typeof card.toolResultPreview !== "string" || !card.toolResultPreview.trim()) {
    return null;
  }
  let parsed;
  try {
    parsed = JSON.parse(card.toolResultPreview);
  } catch (_) {
    return null;
  }
  const channel = String(parsed?.channel || "").trim();
  if (!channel) return null;
  return {
    sourceMessageId: card.id || null,
    extensionName: channel,
    strategy: typeof parsed.strategy === "string" ? parsed.strategy : null,
    instructions: typeof parsed.instructions === "string" ? parsed.instructions : null,
    inputPlaceholder:
      typeof parsed.input_placeholder === "string" ? parsed.input_placeholder : null,
    submitLabel: typeof parsed.submit_label === "string" ? parsed.submit_label : null,
    errorMessage: typeof parsed.error_message === "string" ? parsed.error_message : null,
  };
}

// The most recent channel-connection-required card in the timeline, unless the
// user already dismissed it (the dismissal is keyed by the durable tool id, so a
// closed panel does not re-derive from the still-present card on reload).
function latestChannelConnectionRequirement(messages, dismissedIds) {
  if (!Array.isArray(messages)) return null;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const requirement = channelConnectionRequirementFromCard(messages[index]);
    if (!requirement) continue;
    if (requirement.sourceMessageId && dismissedIds?.has?.(requirement.sourceMessageId)) {
      return null;
    }
    if (threadResumedAfterConnection(messages, index, requirement.extensionName)) {
      return null;
    }
    return requirement;
  }
  return null;
}

// A connection card is consumed once the durable timeline already carries the
// channel-connected continuation after it: pairing completed (here, in
// another chat, or in another tab) and this thread was resumed. Unlike the
// localStorage dismissal set — which only records panels dismissed or
// submitted in *this* browser — the continuation is part of the thread's own
// data, so the verdict holds across browsers and after the extension is
// later removed. A fresh requirement on such a thread arrives as a new
// backend card on the next send.
function threadResumedAfterConnection(messages, cardIndex, channel) {
  const expected = channelConnectionContinuationMessage(channel);
  for (let index = cardIndex + 1; index < messages.length; index += 1) {
    const message = messages[index];
    if (message?.role !== "user") continue;
    const content = typeof message?.content === "string" ? message.content : "";
    if (content.trim() === expected) return true;
  }
  return false;
}

// True when the caller's account for `channel` is already connected, per the
// per-user extensions snapshot. The card is gated on this so a durable
// activation card can never re-open the panel for an already-connected account.
function channelConnectionIsSatisfied(extensions, channel) {
  // Normalize both operands the same way the waiter bus does
  // (lib/channel-connection-events.ts) so a multi-word channel id (e.g.
  // `telegram_bot`) can't satisfy the gate here while the bus keys on a different
  // normalized string — which would re-open the panel for a connected account.
  const expected = normalizeConnectionChannel(channel);
  const extension = (extensions || []).find(
    (item) =>
      normalizeConnectionChannel(
        item?.package_ref?.id || item?.packageRef?.id || item?.id || "",
      ) === expected,
  );
  if (!extension) return false;
  if (extension.authenticated === true && extension.needs_setup !== true) return true;
  if (extension.authenticated === false || extension.needs_setup === true) return false;
  const state =
    extension.onboarding_state ||
    extension.onboardingState ||
    extension.activation_status ||
    extension.activationStatus;
  // Fail closed: an explicit backend connect card must not be suppressed by a
  // missing or unrecognized onboarding state. Treat the account as connected only
  // when it reports a state that is not a "needs connection" one.
  if (typeof state !== "string" || !state) return false;
  return !["setup_required", "pairing_required", "pairing"].includes(state);
}

// The in-chat channel-connection ("pairing") panel state machine, extracted from
// `useChat`. Owns `pendingOnboarding` and its refs, derives the panel from the
// durable connection-required tool card, drives OAuth/pairing redemption, and
// resumes the parked chat on channel connect. `useChat` keeps the gate/send
// state and threads the handles it shares (gate, send) in here; the values this
// returns are re-exposed verbatim from `useChat` for `chat.tsx`.
//
// `pendingOnboarding` MUST be the hook's (and this composition's) first
// `useState` so it stays the sixth overall in `useChat` — the WebUI has no live
// per-slot coupling, but the vm test harness seeds/reads it positionally.
export function useChannelOnboarding(
  threadId,
  { messages, messagesThreadId, pendingGate, pendingGateRef, setPendingGate, setIsProcessing, sendRef },
) {
  const [pendingOnboarding, setPendingOnboardingState] = React.useState(null);
  const pendingOnboardingRef = React.useRef(pendingOnboarding);
  // This surface owns one current onboarding OAuth flow. A monotonically
  // increasing generation fences every async setup/start/status response, so
  // a late response from flow A cannot complete, fail, or clear newer flow B.
  const pendingOnboardingOauthFlowRef = React.useRef(null);
  const onboardingOauthGenerationRef = React.useRef(0);
  // Source tool-message ids whose pairing panel the user dismissed. Keyed by
  // the durable `tool-<invocation_id>`, so a dismissal survives re-renders and
  // timeline reloads and the still-present activation tool-result does not
  // re-derive a panel the user already closed.
  const dismissedOnboardingIdsRef = React.useRef(new Set());
  const setPendingOnboarding = React.useCallback((next) => {
    const current = pendingOnboardingRef.current;
    const value =
      typeof next === "function" ? next(current) : next;
    if (Object.is(value, current)) return;
    pendingOnboardingRef.current = value;
    setPendingOnboardingState(value);
  }, []);

  React.useEffect(() => {
    pendingOnboardingRef.current = pendingOnboarding;
  }, [pendingOnboarding]);

  React.useEffect(() => {
    dismissedOnboardingIdsRef.current = loadDismissedOnboardingIds(threadId);
  }, [threadId]);

  React.useEffect(() => {
    if (pendingOnboarding?.threadId && pendingOnboarding?.extensionName) {
      rememberChannelConnectionWaiter({
        channel: pendingOnboarding.extensionName,
        threadId: pendingOnboarding.threadId,
        sourceMessageId: pendingOnboarding.sourceMessageId || null,
      });
    }
  }, [
    pendingOnboarding?.extensionName,
    pendingOnboarding?.threadId,
    pendingOnboarding?.sourceMessageId,
  ]);

  const clearOnboardingAfterChannelConnected = React.useCallback(
    (onboarding) => {
      const threadForResume = onboarding?.threadId || threadId || null;
      if (!threadForResume || !onboarding?.extensionName) return;
      if (onboarding.sourceMessageId) {
        dismissedOnboardingIdsRef.current.add(onboarding.sourceMessageId);
        persistDismissedOnboardingId(threadForResume, onboarding.sourceMessageId);
      }
      forgetChannelConnectionWaiter({
        channel: onboarding.extensionName,
        threadId: threadForResume,
        sourceMessageId: onboarding.sourceMessageId || null,
      });
      setPendingOnboarding(null);
    },
    [threadId, setPendingOnboarding],
  );

  // Derive the in-chat pairing panel from the durable, structured
  // channel-connection-required tool card the backend attaches to an
  // `extension_activate` result for a connectable channel. The trigger is a
  // concrete per-thread card (present only after the agent engaged the channel
  // here), so — unlike the removed global poll — it never fires on an empty or
  // unrelated chat and never races history loading. It is gated on the live
  // per-user connection state so a card that outlived the user's connect can
  // never re-open the panel for an already-connected account.
  React.useEffect(() => {
    if (!threadId || pendingGate || pendingOnboardingRef.current) return;
    // Only derive from messages that belong to the active thread. On a thread
    // switch `threadId` advances a render before useHistory swaps `messages` to
    // the new thread's timeline; without this guard the previous thread's durable
    // connection card would open — and be stamped onto — the newly-viewed chat.
    if (messagesThreadId && messagesThreadId !== threadId) return;
    const requirement = latestChannelConnectionRequirement(
      messages,
      dismissedOnboardingIdsRef.current,
    );
    if (!requirement) return;
    let cancelled = false;
    Promise.resolve(
      typeof queryClient.fetchQuery === "function"
        ? queryClient.fetchQuery({ queryKey: ["extensions"], queryFn: fetchExtensions })
        : fetchExtensions(),
    )
      .then((data) => {
        if (cancelled || pendingOnboardingRef.current) return;
        if (channelConnectionIsSatisfied(data?.extensions || [], requirement.extensionName)) {
          return;
        }
        setPendingOnboarding({
          extensionName: requirement.extensionName,
          state: "pairing_required",
          threadId,
          sourceMessageId: requirement.sourceMessageId,
          strategy: requirement.strategy,
          instructions: requirement.instructions,
          inputPlaceholder: requirement.inputPlaceholder,
          submitLabel: requirement.submitLabel,
          errorMessage: requirement.errorMessage,
        });
      })
      .catch(() => {
        if (cancelled || pendingOnboardingRef.current) return;
        setPendingOnboarding({
          extensionName: requirement.extensionName,
          state: "pairing_required",
          threadId,
          sourceMessageId: requirement.sourceMessageId,
          strategy: requirement.strategy,
          instructions: requirement.instructions,
          inputPlaceholder: requirement.inputPlaceholder,
          submitLabel: requirement.submitLabel,
          errorMessage: requirement.errorMessage,
        });
      });
    return () => {
      cancelled = true;
    };
  }, [messages, messagesThreadId, threadId, pendingGate, setPendingOnboarding]);

  React.useEffect(() => {
    const browserWindow =
      typeof window !== "undefined" ? window : globalThis?.window || null;
    if (!browserWindow) return;
    let serverCheckInFlight = false;
    const flowSnapshotIsCurrent = (snapshot) => {
      const current = pendingOnboardingOauthFlowRef.current;
      return Boolean(
        snapshot &&
          current &&
          snapshot.generation === current.generation &&
          snapshot.flowId === current.flowId,
      );
    };
    // Reads the pending flow from the REF (not a caller-captured copy) so a
    // completion arriving via the broadcast subscription and one arriving via
    // the interval poll cannot both pass the `completing` guard with stale
    // objects and double-send the continuation.
    const finishCompletion = async (expectedFlow = null) => {
      const pending = pendingOnboardingOauthFlowRef.current;
      if (
        !pending ||
        pending.completing ||
        (expectedFlow && !flowSnapshotIsCurrent(expectedFlow))
      ) {
        return;
      }
      pendingOnboardingOauthFlowRef.current = { ...pending, completing: true };
      const onboarding = pendingOnboardingRef.current;
      const threadForResume =
        pending.threadId || onboarding?.threadId || threadId || null;
      let sourceCleared = false;
      if (connectionEventMatchesOnboarding({ channel: pending.channel }, onboarding)) {
        if (threadForResume && !onboarding?.requestId) {
          const sendForResume = sendRef.current;
          if (typeof sendForResume !== "function") {
            pendingOnboardingOauthFlowRef.current = pending;
            return;
          }
          const continuation = await sendForResume(
            channelConnectionContinuationMessage(pending.channel),
            {
              threadId: threadForResume,
              bypassPendingOnboarding: true,
            },
          );
          if (!flowSnapshotIsCurrent(pending)) return;
          if (!continuation || continuation.outcome === "rejected_busy") {
            if (flowSnapshotIsCurrent(pending)) {
              pendingOnboardingOauthFlowRef.current = pending;
            }
            return;
          }
        }
        clearOnboardingAfterChannelConnected(onboarding);
        sourceCleared = true;
      }
      if (!flowSnapshotIsCurrent(pending)) return;
      pendingOnboardingOauthFlowRef.current = null;
      await notifyChannelConnected({
        channel: pending.channel,
        sourceThreadId: sourceCleared ? threadForResume : null,
        source: "chat-oauth",
      });
    };
    // A failed or expired flow clears the pending ref (stopping the poll) and
    // stamps a retryable error onto the still-mounted card, so the user gets a
    // visible way out instead of a spinner the popup can no longer resolve.
    const failOauthFlow = (pending, message) => {
      if (!flowSnapshotIsCurrent(pending)) return;
      pendingOnboardingOauthFlowRef.current = null;
      setPendingOnboarding((current) =>
        current &&
        normalizeConnectionChannel(current.extensionName) ===
          normalizeConnectionChannel(pending.channel)
          ? { ...current, oauthError: message }
          : current,
      );
    };
    const handleCompletion = (payload) => {
      const pending = pendingOnboardingOauthFlowRef.current;
      if (!pending) return;
      if (failureMatchesFlow(payload, pending.flowId)) {
        // The callback can report failure while exact lifecycle compensation
        // is still retrying. Keep the watcher alive when a durable flow scope
        // is available; the status poll below is the authoritative terminal
        // signal and also resumes cleanup after a service restart.
        if (!pending.invocationId) {
          failOauthFlow(pending, CHAT_OAUTH_FAILED_MESSAGE);
        }
        return;
      }
      if (!completionMatchesFlow(payload, pending.flowId)) return;
      Promise.resolve(finishCompletion(pending)).catch(() => {
        if (flowSnapshotIsCurrent(pending)) {
          pendingOnboardingOauthFlowRef.current = pending;
        }
      });
    };
    const pollServerState = () => {
      const pending = pendingOnboardingOauthFlowRef.current;
      if (!pending || pending.completing || serverCheckInFlight) return;
      serverCheckInFlight = true;
      const flowStatus = pending.flowId
        ? Promise.resolve(fetchOauthFlowStatus(pending.flowId, pending.invocationId))
        : Promise.resolve(null);
      flowStatus
        .then((result) => {
          if (!flowSnapshotIsCurrent(pending)) return null;
          if (result?.status === "completed") return finishCompletion(pending);
          if (["failed", "canceled", "expired"].includes(result?.status)) {
            failOauthFlow(pending, CHAT_OAUTH_FAILED_MESSAGE);
            return null;
          }
          return fetchExtensions();
        })
        .then((snapshot) => {
          if (!snapshot || !flowSnapshotIsCurrent(pending)) return null;
          const extensions = Array.isArray(snapshot)
            ? snapshot
            : snapshot?.extensions || [];
          if (channelConnectionIsSatisfied(extensions, pending.channel)) {
            return finishCompletion(pending);
          }
          return null;
        })
        .catch(() => null)
        .finally(() => {
          serverCheckInFlight = false;
        });
    };

    const unsubscribe = subscribeProductAuthOAuthCompletion(browserWindow, handleCompletion);
    const timer = browserWindow.setInterval(() => {
      const pending = pendingOnboardingOauthFlowRef.current;
      // No in-flight OAuth flow: skip the storage read + server poll entirely
      // (the interval runs for the lifetime of every chat view).
      if (!pending) return;
      if (
        !pending.completing &&
        pending.startedAt &&
        Date.now() - pending.startedAt > CHAT_OAUTH_TIMEOUT_MS
      ) {
        failOauthFlow(pending, CHAT_OAUTH_TIMED_OUT_MESSAGE);
        return;
      }
      handleCompletion(readLatestProductAuthOAuthCompletion(browserWindow));
      pollServerState();
    }, CHAT_OAUTH_POLL_MS);
    return () => {
      browserWindow.clearInterval(timer);
      unsubscribe();
    };
  }, [clearOnboardingAfterChannelConnected, setPendingOnboarding, threadId]);

  React.useEffect(() => {
    return subscribeChannelConnected((event) => {
      // A channel connected — here, in another tab, or on the extensions page.
      // Refresh the per-user connection snapshot so every chat's panel gate
      // (and the extensions UI) sees "connected" instead of the stale
      // "needs setup" cache; without this a freshly connected account can
      // still surface a "Connect" panel for up to the query staleTime.
      queryClient.invalidateQueries?.({ queryKey: ["extensions"] });
      queryClient.invalidateQueries?.({ queryKey: ["connectable-channels"] });
      const onboarding = pendingOnboardingRef.current;
      if (!connectionEventMatchesOnboarding(event, onboarding)) return;
      // Hide the mounted panel (the account IS connected now), but do NOT
      // persist a dismissal or forget the waiter: the continuation send for
      // this thread happens best-effort in the NOTIFYING tab
      // (resumeWaitingChannelConnections). If that send fails, the waiter it
      // re-persists is the only remaining path to resume the parked request —
      // durably dismissing here would strand it with no UI affordance left.
      setPendingOnboarding(null);
    });
  }, [setPendingOnboarding]);

  const submitOnboardingPairing = React.useCallback(
    async (code) => {
      const onboarding = pendingOnboardingRef.current;
      if (!onboarding) {
        throw new Error("pairing is no longer pending");
      }
      const trimmed = String(code || "").trim();
      if (!trimmed) {
        throw new Error("pairing code is required");
      }
      const threadForResume = onboarding.threadId || threadId || null;
      const options = {
        threadId: threadForResume,
        requestId: onboarding.requestId || null,
      };
      const response = await redeemPairingCode(onboarding.extensionName, trimmed, options);
      if (response?.success === false) {
        throw new Error(response.message || "Pairing failed");
      }
      if (response?.resumeError) {
        // The connection succeeded (binding is durable), but the backend
        // couldn't continue this parked chat. The gate only clears when the
        // turn actually resumes, so it will stay pending — surface a distinct,
        // recoverable error instead of leaving the card spinning forever.
        const error = new Error("channel connection resume did not complete");
        error.resumeFailed = true;
        throw error;
      }
      const clearOnboarding = () => {
        if (onboarding.sourceMessageId) {
          dismissedOnboardingIdsRef.current.add(onboarding.sourceMessageId);
          persistDismissedOnboardingId(threadForResume, onboarding.sourceMessageId);
        }
        forgetChannelConnectionWaiter({
          channel: onboarding.extensionName,
          threadId: threadForResume,
          sourceMessageId: onboarding.sourceMessageId || null,
        });
        setPendingOnboarding(null);
      };
      if (threadForResume && !onboarding.requestId) {
        const continuation = await sendRef.current(
          channelConnectionContinuationMessage(onboarding.extensionName),
          {
            threadId: threadForResume,
            bypassPendingOnboarding: true,
          },
        );
        // `send` returns null (or a rejected_busy outcome) without posting a
        // turn when thread admission blocks it — an in-flight submit or an
        // active run on the resume thread. The account is connected either way,
        // but the blocked request was NOT resumed: keep the waiter and panel
        // so the channel-connected event path (or a manual retry) can still
        // deliver the continuation instead of silently dropping it behind a
        // success response.
        if (continuation && continuation.outcome !== "rejected_busy") {
          clearOnboarding();
        }
        return response;
      }
      clearOnboarding();
      if (onboarding.requestId && threadForResume) {
        setIsProcessing(true);
      }
      return response;
    },
    [threadId, setPendingOnboarding, setIsProcessing],
  );

  // Redeem a channel-pairing code for a LIVE auth gate — a `manual_token`
  // challenge that also carries a `connection` requirement (gates.ts normalizes
  // it onto `pendingGate.connection`). Distinct from `submitOnboardingPairing`,
  // which drives the durable-timeline pairing panel: here the pairing context
  // comes off the gate, and the parked turn resumes through the gate rather than
  // by re-sending a continuation message. The redeem stores a durable binding and
  // the server resumes every run this caller parked on the channel.
  const submitChannelConnectionPairing = React.useCallback(
    async (code) => {
      const gate = pendingGateRef.current || pendingGate;
      const channel = gate?.connection?.channel;
      if (!gate || !channel) {
        throw new Error("channel connection is no longer pending");
      }
      if (!gate.runId || !gate.gateRef) {
        throw new Error("channel connection gate is missing run_id and gate_ref");
      }
      const trimmed = String(code || "").trim();
      if (!trimmed) {
        throw new Error("pairing code is required");
      }
      const response = await redeemPairingCode(channel, trimmed, { threadId });
      if (response?.success === false) {
        throw new Error(response.message || "Pairing failed");
      }
      if (response?.resumeError) {
        // The binding is durable (connected), but the parked turn didn't resume;
        // the gate stays pending. Surface a distinct, recoverable error instead of
        // leaving the pairing card spinning forever.
        const error = new Error("channel connection resume did not complete");
        error.resumeFailed = true;
        throw error;
      }
      // The server resumed the parked run on redeem; clear the gate locally and
      // show processing while the resumed turn streams back over SSE.
      setPendingGate(null);
      setIsProcessing(true);
      return response;
    },
    [pendingGate, threadId, setPendingGate, setIsProcessing],
  );

  const startOnboardingOAuth = React.useCallback(async () => {
    const generation = onboardingOauthGenerationRef.current + 1;
    onboardingOauthGenerationRef.current = generation;
    const onboarding = pendingOnboardingRef.current;
    if (!onboarding) {
      throw new Error("connection is no longer pending");
    }
    if (onboarding.strategy !== "oauth") {
      throw new Error("connection does not use OAuth");
    }
    const packageRef = { kind: "extension", id: onboarding.extensionName };
    const packageKey = onboarding.extensionName;
    // Open the placeholder popup BEFORE any await: a slow setup fetch would
    // otherwise burn the click's user activation and get the real popup
    // blocked. Any failure below closes it again.
    const popup = window.open("about:blank", "_blank", "width=600,height=600");
    if (popup) popup.opener = null;
    // Unlike the noopener fresh-open in `openAuthPopup` (which returns null
    // even on success per spec), a null here reliably means the browser
    // blocked the popup — surface it before burning the flow start.
    if (!popup) {
      throw new Error("Authorization popup was blocked.");
    }
    try {
      const setup =
        typeof queryClient.fetchQuery === "function"
          ? await queryClient.fetchQuery({
              queryKey: ["extension-setup", packageKey],
              queryFn: () => fetchExtensionSetup(packageRef),
            })
          : await fetchExtensionSetup(packageRef);
      if (generation !== onboardingOauthGenerationRef.current) {
        if (popup && !popup.closed) popup.close();
        return null;
      }
      const secret = (setup?.secrets || []).find(
        (item) => (item?.setup?.kind || "manual_token") === "oauth",
      );
      if (!secret) {
        throw new Error("OAuth setup is unavailable for this channel");
      }
      const response = await startExtensionOauth(packageRef, secret);
      if (generation !== onboardingOauthGenerationRef.current) {
        if (popup && !popup.closed) popup.close();
        return null;
      }
      if (response?.success === false) {
        throw new Error(response.message || "OAuth setup failed");
      }
      if (!response?.authorization_url) {
        throw new Error("OAuth setup did not return an authorization URL");
      }
      if (!response?.flow_id) {
        throw new Error("OAuth setup did not return a flow id");
      }
      const opened = openAuthPopup(response.authorization_url, popup);
      if (!opened.ok) {
        throw new Error(
          opened.reason === "popup_blocked"
            ? "Authorization popup was blocked."
            : "Authorization URL must use HTTPS.",
        );
      }
      // A retry after a failed/timed-out attempt clears the stale card error.
      setPendingOnboarding((current) =>
        current?.oauthError ? { ...current, oauthError: null } : current,
      );
      pendingOnboardingOauthFlowRef.current = {
        generation,
        flowId: response.flow_id,
        invocationId:
          response?.callback_scope?.invocation_id ||
          response?.callbackScope?.invocationId ||
          null,
        channel: onboarding.extensionName,
        threadId: onboarding.threadId || threadId || null,
        startedAt: Date.now(),
      };
      return response;
    } catch (error) {
      if (popup && !popup.closed) popup.close();
      throw error;
    }
  }, [threadId, setPendingOnboarding]);

  const dismissOnboardingPairing = React.useCallback(() => {
    // Dismissing the card also abandons any in-flight OAuth flow it started —
    // otherwise the watcher keeps polling the server for a card that is gone.
    onboardingOauthGenerationRef.current += 1;
    pendingOnboardingOauthFlowRef.current = null;
    const onboarding = pendingOnboardingRef.current;
    if (onboarding) {
      const threadForDismiss = onboarding.threadId || threadId;
      if (onboarding.sourceMessageId) {
        dismissedOnboardingIdsRef.current.add(onboarding.sourceMessageId);
        persistDismissedOnboardingId(threadForDismiss, onboarding.sourceMessageId);
      }
      forgetChannelConnectionWaiter({
        channel: onboarding.extensionName,
        threadId: threadForDismiss,
        sourceMessageId: onboarding.sourceMessageId || null,
      });
    }
    setPendingOnboarding(null);
  }, [threadId, setPendingOnboarding]);

  return {
    pendingOnboarding,
    pendingOnboardingRef,
    setPendingOnboardingState,
    submitOnboardingPairing,
    submitChannelConnectionPairing,
    startOnboardingOAuth,
    dismissOnboardingPairing,
  };
}
