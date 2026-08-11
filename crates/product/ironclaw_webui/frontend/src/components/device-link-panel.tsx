// @ts-nocheck
import React from "react";
import { Button } from "../design-system/button";
import { LinkPayloadPanel } from "./link-payload-panel";
import { useT } from "../lib/i18n";
import {
  cancelDeviceLink,
  deviceLinkError,
  pollDeviceLink,
  startDeviceLink,
  submitDeviceLinkInput,
} from "../lib/device-link-api";
import {
  DEVICE_LINK_ERROR_CODES,
  DEVICE_LINK_INPUT_KINDS,
  DEVICE_LINK_MODES,
  DEVICE_LINK_STEPS,
  deviceLinkAlternateMode,
  deviceLinkFrameFromWire,
  deviceLinkPollDelayMs,
} from "../lib/device-link-frame";

// Input affordances per `DeviceLinkInputKind`. A one-time code and a standing
// account password are not the same field: only the second is masked, and
// only the first should be offered to an SMS autofill.
const INPUT_ATTRIBUTES = Object.freeze({
  [DEVICE_LINK_INPUT_KINDS.identifier]: {
    type: "tel",
    inputMode: "tel",
    autoComplete: "tel",
    labelKey: "deviceLink.identifierLabel",
  },
  [DEVICE_LINK_INPUT_KINDS.code]: {
    type: "text",
    inputMode: "numeric",
    autoComplete: "one-time-code",
    labelKey: "deviceLink.codeLabel",
  },
  [DEVICE_LINK_INPUT_KINDS.password]: {
    type: "password",
    inputMode: "text",
    autoComplete: "current-password",
    labelKey: "deviceLink.passwordLabel",
  },
});

const INPUT_CLASS =
  "w-full rounded-md border border-white/12 bg-white/[0.04] px-3 py-2 text-sm " +
  "text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45";

/**
 * DeviceLinkPanel — the whole multi-step device link, from first payload to a
 * linked account.
 *
 * Shared deliberately: the in-chat gate card (`auth-device-link-card`) and the
 * Extensions configure modal render the same flow, and a device link is far
 * too much state to implement twice.
 *
 * What this component owns, and why each rule exists:
 *
 * - **Stale revisions are dropped.** Every frame carries the flow revision it
 *   was rendered from. A poll that overlaps a submit can resolve *after* the
 *   newer frame is already painted; adopting it would walk the card backwards
 *   onto a step the user has already left. Anything below the highest revision
 *   already adopted is discarded.
 * - **Stale generations are dropped.** Switching modes or starting again
 *   abandons the old flow. Responses still in flight from it describe a link
 *   nobody is on any more.
 * - **Polling stops on every terminal step.** A completed or failed link will
 *   never advance again, and a card left open on one must hold no timer.
 * - **Secrets are not retained.** The typed value is cleared the moment it is
 *   handed to the host; the session it produces is host custody and never
 *   reaches the browser.
 *
 * Props
 *   provider       vendor id the link belongs to (the auth provider).
 *   extensionName  installed extension declaring the device-link recipe.
 *   displayName    human name for the account being linked.
 *   initialFrame   normalized frame to paint while `start` is in flight
 *                  (a chat gate already carries one).
 *   threadId/runId/gateRef/invocationId  caller scope for the start call.
 *   onCompleted    fired once when the link completes.
 */
export function DeviceLinkPanel({
  provider = "",
  extensionName = "",
  displayName = "",
  initialFrame = null,
  threadId = "",
  runId = "",
  gateRef = "",
  invocationId = "",
  onCompleted = null,
}) {
  const t = useT();
  const [frame, setFrame] = React.useState(initialFrame);
  const [flowId, setFlowId] = React.useState("");
  const [mode, setMode] = React.useState(initialFrame?.mode || DEVICE_LINK_MODES.default);
  const [attempt, setAttempt] = React.useState(0);
  const [error, setError] = React.useState("");
  const [inputValue, setInputValue] = React.useState("");
  const [isStarting, setIsStarting] = React.useState(false);
  const [isSubmitting, setIsSubmitting] = React.useState(false);
  const [isRenewing, setIsRenewing] = React.useState(false);
  // Bumped by every start. A response tagged with an older generation belongs
  // to a flow that was abandoned (mode switch, start again) and is dropped.
  const generationRef = React.useRef(0);
  // The highest frame revision adopted on the current flow.
  const revisionRef = React.useRef(-1);
  const flowIdRef = React.useRef("");
  const completedRef = React.useRef(false);

  const name = displayName || provider || extensionName;

  const adopt = (response, generation) => {
    if (generation !== generationRef.current) return false;
    const next = deviceLinkFrameFromWire(response?.device_link);
    if (!next) return false;
    const nextFlowId = response?.flow_id || next.flowId || "";
    // A frame for a different flow is another card's business.
    if (flowIdRef.current && nextFlowId && nextFlowId !== flowIdRef.current) return false;
    if (next.revision < revisionRef.current) return false;
    revisionRef.current = next.revision;
    if (nextFlowId && nextFlowId !== flowIdRef.current) {
      flowIdRef.current = nextFlowId;
      setFlowId(nextFlowId);
    }
    setFrame(next);
    if (next.step === DEVICE_LINK_STEPS.completed && !completedRef.current) {
      completedRef.current = true;
      onCompleted?.(next);
    }
    return true;
  };

  // Start (or resume) the flow. Re-runs on a mode switch and on "start again";
  // both are a new generation, so nothing from the previous one can land.
  React.useEffect(() => {
    generationRef.current += 1;
    const generation = generationRef.current;
    revisionRef.current = -1;
    flowIdRef.current = "";
    completedRef.current = false;
    setFlowId("");
    setError("");
    setInputValue("");
    setIsStarting(true);
    let cancelled = false;
    Promise.resolve(
      startDeviceLink({
        provider,
        extensionName,
        mode,
        threadId,
        runId,
        gateRef,
        invocationId,
      }),
    )
      .then((response) => {
        if (cancelled) return;
        adopt(response, generation);
      })
      .catch((startError) => {
        if (cancelled || generation !== generationRef.current) return;
        setError(deviceLinkError(startError, t("deviceLink.startFailed", { name })));
      })
      .then(() => {
        if (!cancelled) setIsStarting(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider, extensionName, mode, attempt]);

  // Poll while the flow can still advance. A terminal step registers no timer
  // at all, so a card left open on success or failure stops talking to the
  // host. Scalar deps only: re-arming on every adopted object would restart
  // the interval on each tick.
  const pollDelayMs = deviceLinkPollDelayMs(frame);
  const stepIsTerminal = Boolean(frame?.terminal);
  React.useEffect(() => {
    if (!flowId || !frame || stepIsTerminal) return undefined;
    const generation = generationRef.current;
    const timer = setInterval(async () => {
      if (generation !== generationRef.current) return;
      try {
        const response = await pollDeviceLink({ flowId, invocationId });
        adopt(response, generation);
      } catch (_) {
        // Poll is best-effort; the next tick retries.
      }
    }, pollDelayMs);
    return () => clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [flowId, stepIsTerminal, pollDelayMs, frame?.revision]);

  const abandonCurrentFlow = () => {
    const abandoned = flowIdRef.current;
    if (!abandoned || stepIsTerminal) return;
    // Best effort: the host logs the device out so an accepted-but-abandoned
    // link is not left as an orphan authorization.
    Promise.resolve(cancelDeviceLink({ flowId: abandoned, invocationId })).catch(() => {});
  };

  const switchMode = () => {
    abandonCurrentFlow();
    setFrame(null);
    setMode(deviceLinkAlternateMode(mode));
  };

  const restart = () => {
    abandonCurrentFlow();
    setFrame(null);
    setAttempt((value) => value + 1);
  };

  const renewPayload = async () => {
    if (!flowId || isRenewing) return;
    const generation = generationRef.current;
    setIsRenewing(true);
    setError("");
    try {
      adopt(await pollDeviceLink({ flowId, invocationId }), generation);
    } catch (renewError) {
      setError(deviceLinkError(renewError, t("deviceLink.pollFailed", { name })));
    } finally {
      setIsRenewing(false);
    }
  };

  const submitInput = async (event) => {
    event?.preventDefault?.();
    const value = inputValue.trim();
    if (!frame || !flowId || !value || isSubmitting) return;
    const generation = generationRef.current;
    setIsSubmitting(true);
    setError("");
    try {
      const response = await submitDeviceLinkInput({
        flowId,
        revision: frame.revision,
        kind: frame.inputKind,
        value,
        invocationId,
      });
      // Drop the code/password from component state the moment the host has
      // it. Nothing here keeps a secret across a step.
      setInputValue("");
      adopt(response, generation);
    } catch (submitError) {
      setError(deviceLinkError(submitError, t("deviceLink.submitFailed")));
    } finally {
      setIsSubmitting(false);
    }
  };

  const modeSwitch = (
    <button
      type="button"
      onClick={switchMode}
      data-testid="device-link-mode-switch"
      data-device-link-target-mode={deviceLinkAlternateMode(mode)}
      className="text-xs text-iron-400 underline underline-offset-2 hover:text-iron-200"
    >
      {mode === DEVICE_LINK_MODES.alternate
        ? t("deviceLink.useQr")
        : t("deviceLink.usePhone")}
    </button>
  );

  const errorNotice = error
    ? (<p role="alert" data-testid="device-link-error" className="mt-3 text-xs leading-5 text-red-300">{error}</p>)
    : null;

  if (!frame) {
    return (
      <div data-testid="device-link-panel">
        {error
          ? (
              <div className="space-y-2">
                <p role="alert" data-testid="device-link-error" className="text-xs leading-5 text-red-300">{error}</p>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={restart}
                  loading={isStarting}
                  data-testid="device-link-restart"
                >
                  {t("deviceLink.startAgain")}
                </Button>
              </div>
            )
          : (<div className="v2-skeleton h-3 w-32 rounded" data-testid="device-link-starting" />)}
      </div>
    );
  }

  return (
    <div data-testid="device-link-panel" data-device-link-step={frame.step} data-device-link-mode={mode}>
      <p data-testid="device-link-instructions" className="mb-3 text-xs leading-5 text-iron-300">
        {frame.instructions}
      </p>
      {frame.step === DEVICE_LINK_STEPS.display &&
      (
        <div className="space-y-3">
          <LinkPayloadPanel
            idPrefix="device-link"
            payload={frame.qrPayload || ""}
            code={frame.code || ""}
            expiresAtMs={frame.expiresAtMs}
            labels={payloadLabels(t, name)}
            onRenew={renewPayload}
            isRenewing={isRenewing}
          />
          {modeSwitch}
        </div>
      )}
      {frame.step === DEVICE_LINK_STEPS.awaitingVendor &&
      (
        <p
          role="status"
          data-testid="device-link-awaiting"
          className="text-xs leading-5 text-iron-300"
        >
          {t("deviceLink.awaiting", { name })}
        </p>
      )}
      {frame.step === DEVICE_LINK_STEPS.inputRequired &&
      (
        <form onSubmit={submitInput} className="space-y-2">
          <label className="block text-xs text-iron-300" htmlFor="device-link-input">
            {frame.secretLabel || t(inputAttributes(frame.inputKind).labelKey)}
          </label>
          <input
            id="device-link-input"
            data-testid="device-link-input"
            data-device-link-input-kind={frame.inputKind}
            className={INPUT_CLASS}
            type={inputAttributes(frame.inputKind).type}
            inputMode={inputAttributes(frame.inputKind).inputMode}
            autoComplete={inputAttributes(frame.inputKind).autoComplete}
            value={inputValue}
            onChange={(event) => setInputValue(event.target.value)}
          />
          <div className="flex flex-wrap items-center gap-3">
            <Button
              type="submit"
              variant="primary"
              size="sm"
              loading={isSubmitting}
              data-testid="device-link-submit"
            >
              {t("deviceLink.submit")}
            </Button>
            {modeSwitch}
          </div>
        </form>
      )}
      {frame.step === DEVICE_LINK_STEPS.completed &&
      (
        <div className="space-y-2">
          <p data-testid="device-link-completed" className="text-sm text-[var(--v2-positive-text)]">
            ✅ {t("deviceLink.linked", { name })}
          </p>
          {/*
            The ADR's one compensating control, and it is DETECTION, not
            prevention: IronClaw cannot verify that the code it displayed came
            from a session it controls, so the resolved account plus a
            count-the-devices check is what makes the crude substituted-login
            variant visible. It does NOT catch a stolen key (the key IS the
            device — no second entry appears) or a session that registered
            under the name "IronClaw". The copy therefore asks the user to
            check a specific, checkable fact and claims nothing beyond it.
          */}
          {frame.code
            ? (
                <p
                  data-testid="device-link-account"
                  className="text-xs leading-5 text-iron-300"
                >
                  {t("deviceLink.confirmDeviceAccount", { account: frame.code })}
                </p>
              )
            : null}
          <p data-testid="device-link-confirm-device" className="text-xs leading-5 text-iron-400">
            {t("deviceLink.confirmDevice", { name })}
          </p>
          <p className="text-xs leading-5 text-iron-400">
            {t("deviceLink.revokeHint", { name })}
          </p>
        </div>
      )}
      {frame.step === DEVICE_LINK_STEPS.failed &&
      (
        <div className="space-y-2">
          <p role="alert" data-testid="device-link-failed" className="text-xs leading-5 text-red-300">
            {failureCopy(t, frame)}
          </p>
          {frame.restartable
            ? (
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={restart}
                  loading={isStarting}
                  data-testid="device-link-restart"
                >
                  {t("deviceLink.startAgain")}
                </Button>
              )
            : (
                <p data-testid="device-link-terminal" className="text-xs leading-5 text-iron-400">
                  {t("deviceLink.cannotRetry", { name })}
                </p>
              )}
        </div>
      )}
      {errorNotice}
    </div>
  );
}

function inputAttributes(inputKind) {
  return INPUT_ATTRIBUTES[inputKind] || INPUT_ATTRIBUTES[DEVICE_LINK_INPUT_KINDS.code];
}

function payloadLabels(t, name) {
  return {
    qrAlt: t("deviceLink.qrAlt", { name }),
    copy: t("deviceLink.copyCode"),
    copied: t("common.copiedToClipboard"),
    open: t("deviceLink.openIn", { name }),
    expiresIn: (time) => t("deviceLink.expiresIn", { time }),
    expired: t("deviceLink.expired"),
    renew: t("deviceLink.refresh"),
  };
}

// The frame's `instructions` are host-authored and already say what happened;
// a typed error code adds the one line a user can act on. An unrecognized code
// (newer host, older browser) contributes nothing rather than a raw key.
function failureCopy(t, frame) {
  if (!DEVICE_LINK_ERROR_CODES.includes(frame.errorCode)) return frame.instructions;
  return `${frame.instructions} ${t(`deviceLink.error.${frame.errorCode}`)}`;
}
