// @ts-nocheck
import React from "react";
import { useParams } from "react-router";
import { useT } from "../../lib/i18n";
import { fetchIntentDetail, fetchSigningContext } from "../../lib/api";
import { CEREMONY_OUTCOME, runCeremony } from "../../lib/ledger/ceremony";
import { createDevicePort } from "../../lib/ledger/device-adapter";

/**
 * The transaction-review page (attested-signing Phase C).
 *
 * A review link in chat lands on the server's public `/intent/{token}` route,
 * which reveals nothing and redirects here. Everything below is rendered from
 * an authenticated read the server authorized against the session — the caller
 * must BE the intent's bound approver.
 *
 * ## The hash is the point of this page
 *
 * A Ledger displays the transaction hash it is about to sign. The human's job
 * is to compare that against what IronClaw says it asked for. That comparison
 * only works if the hash is shown in full: an abbreviated `0xab…cd` matches a
 * tampered transaction just as happily as the real one, because an attacker
 * who can choose the transaction can usually grind the visible ends. So the
 * hash renders complete, in a monospace face, broken for readability but never
 * truncated — and the tests pin that.
 *
 * ## No descriptor, no signing
 *
 * The device can only render a transaction's fields if an ERC-7730 descriptor
 * covers it. Without one it would fall back to showing a bare hash, which the
 * human cannot meaningfully check — that is blind signing wearing a hardware
 * wallet. So when the backend says `clear_signing: "unavailable"` this page
 * renders a blocked state and offers NO path to the device. There is no
 * override, no "sign anyway", and no flag: the affordance simply does not
 * exist, which is the only version of fail-closed that survives a user in a
 * hurry.
 */

/** Lifecycle states the server may project onto an intent. */
const TERMINAL_STATES = ["approved", "rejected", "expired"];

/** Split a long hex string into readable groups WITHOUT dropping any of it. */
export function groupHex(value, groupSize = 8) {
  if (typeof value !== "string" || value.length === 0) return [];
  const groups = [];
  for (let index = 0; index < value.length; index += groupSize) {
    groups.push(value.slice(index, index + groupSize));
  }
  return groups;
}

/** Milliseconds until `expiresAtMs`, floored at zero. */
export function millisRemaining(expiresAtMs, nowMs) {
  if (typeof expiresAtMs !== "number" || Number.isNaN(expiresAtMs)) return 0;
  return Math.max(0, expiresAtMs - nowMs);
}

/** Whole minutes remaining, for the countdown label. */
export function minutesRemaining(expiresAtMs, nowMs) {
  return Math.floor(millisRemaining(expiresAtMs, nowMs) / 60000);
}

/**
 * The full hash, grouped but complete.
 *
 * `data-testid` is stable so the truncation regression test can assert the
 * rendered text equals the hash exactly.
 */
export function TransactionHash({ hash }) {
  const t = useT();
  return (
    <section className="mt-6">
      <h2 className="text-sm font-medium text-[var(--v2-text-muted)]">
        {t("review.hash.label")}
      </h2>
      <p className="mt-1 text-sm text-[var(--v2-text-muted)]">
        {t("review.hash.compare")}
      </p>
      <p
        data-testid="review-approved-tx-hash"
        className="mt-2 select-all break-all font-mono text-base leading-relaxed text-[var(--v2-text-strong)]"
      >
        {hash}
      </p>
    </section>
  );
}

/** One decoded-transaction field. */
export function DetailRow({ label, value }) {
  return (
    <div className="flex justify-between gap-4 border-b border-[var(--v2-border)] py-2">
      <span className="text-sm text-[var(--v2-text-muted)]">{label}</span>
      <span className="break-all text-right font-mono text-sm text-[var(--v2-text-strong)]">
        {value}
      </span>
    </div>
  );
}

/**
 * Flatten the decoded transaction into displayable rows.
 *
 * Deliberately generic: the server sends whatever the authoritative decode
 * produced, and this page renders it rather than re-deriving or interpreting
 * anything. A field this page does not understand must still be shown — a
 * silently dropped field is a field an attacker gets to set for free.
 */
export function decodedRows(decodedTx) {
  if (!decodedTx || typeof decodedTx !== "object") return [];
  return Object.entries(decodedTx)
    .filter(([, value]) => value !== null && value !== undefined && value !== "")
    .map(([key, value]) => ({
      key,
      value: typeof value === "object" ? JSON.stringify(value) : String(value),
    }));
}

/**
 * The device half of the page.
 *
 * Renders exactly one of three things and never two: still checking, blocked,
 * or ready to connect. `data-testid="review-sign-action"` exists ONLY on the
 * ready branch, so a test can assert the affordance is absent — not merely
 * disabled — whenever clear signing is unavailable.
 */
export function SigningCeremony({ state, terminal, onSign, running, outcome }) {
  const t = useT();

  // A decided intent has nothing left to sign.
  if (terminal) return null;

  if (state.status === "loading") {
    return (
      <section className="mt-6" data-testid="review-ceremony-loading">
        <p className="text-sm text-[var(--v2-text-muted)]">
          {t("review.ceremony.checking")}
        </p>
      </section>
    );
  }

  if (state.status !== "available") {
    return (
      <section className="mt-6" data-testid="review-ceremony-blocked">
        <h2 className="text-sm font-medium text-[var(--v2-text-strong)]">
          {t("review.ceremony.blocked.title")}
        </h2>
        <p className="mt-1 text-sm text-[var(--v2-text-muted)]">
          {t("review.ceremony.blocked.body")}
        </p>
      </section>
    );
  }

  return (
    <section className="mt-6" data-testid="review-ceremony-ready">
      <button
        type="button"
        data-testid="review-sign-action"
        disabled={running}
        onClick={onSign}
        className="rounded-lg bg-[var(--v2-accent)] px-4 py-2 text-sm font-medium"
      >
        {t(running ? "review.ceremony.signing" : "review.ceremony.connect")}
      </button>
      {outcome && outcome !== CEREMONY_OUTCOME.signed && (
        <p data-testid="review-ceremony-outcome" className="mt-2 text-sm text-[var(--v2-text-muted)]">
          {t(`review.ceremony.outcome.${outcome}`)}
        </p>
      )}
      {outcome === CEREMONY_OUTCOME.signed && (
        <p data-testid="review-ceremony-signed" className="mt-2 text-sm text-[var(--v2-text-strong)]">
          {t("review.ceremony.outcome.signed")}
        </p>
      )}
    </section>
  );
}

export function ReviewPage() {
  const t = useT();
  const { intentId } = useParams();
  const [state, setState] = React.useState({ status: "loading" });
  const [signingContext, setSigningContext] = React.useState({ status: "loading" });

  React.useEffect(() => {
    let cancelled = false;
    setState({ status: "loading" });
    fetchIntentDetail({ intentId })
      .then((intent) => {
        if (!cancelled) setState({ status: "ready", intent });
      })
      .catch((error) => {
        if (cancelled) return;
        // A 404 is the server's uniform refusal: unknown, not yours, or
        // expired — all one answer by design. Rendering a distinguishing
        // message would undo that on the client.
        setState(
          error?.status === 404
            ? { status: "unavailable" }
            : { status: "error", message: error?.message },
        );
      });
    return () => {
      cancelled = true;
    };
  }, [intentId]);

  const [ceremony, setCeremony] = React.useState({ running: false, outcome: null });

  // Declared unconditionally, above every early return, so the hook order is
  // identical on each render regardless of load state.
  const runDeviceCeremony = React.useCallback(async () => {
    setCeremony({ running: true, outcome: null });
    const result = await runCeremony({
      // Async: the adapter is a dynamic import so DMK never loads for chat.
      device: await createDevicePort(undefined, { intentId }),
      intent: state.status === "ready" ? state.intent : null,
      // The backend's answer, never a local guess.
      clearSigningAvailable: signingContext.status === "available",
      descriptor: signingContext.descriptor,
    });
    setCeremony({ running: false, outcome: result.outcome });
  }, [state, signingContext]);

  React.useEffect(() => {
    let cancelled = false;
    setSigningContext({ status: "loading" });
    fetchSigningContext({ intentId })
      .then((context) => {
        if (cancelled) return;
        setSigningContext(
          context?.clear_signing === "available"
            ? { status: "available", descriptor: context.descriptor }
            : { status: "unavailable" },
        );
      })
      .catch(() => {
        // A failed fetch is not a reason to proceed. Any outcome that is not
        // an explicit "available" blocks.
        if (!cancelled) setSigningContext({ status: "unavailable" });
      });
    return () => {
      cancelled = true;
    };
  }, [intentId]);

  if (state.status === "loading") {
    return (
      <main className="mx-auto max-w-2xl px-6 py-10" data-testid="review-loading">
        <p className="text-[var(--v2-text-muted)]">{t("common.loading")}</p>
      </main>
    );
  }

  if (state.status === "unavailable") {
    return (
      <main className="mx-auto max-w-2xl px-6 py-10" data-testid="review-unavailable">
        <h1 className="text-xl font-semibold text-[var(--v2-text-strong)]">
          {t("review.unavailable.title")}
        </h1>
        <p className="mt-2 text-[var(--v2-text-muted)]">
          {t("review.unavailable.body")}
        </p>
      </main>
    );
  }

  if (state.status === "error") {
    return (
      <main className="mx-auto max-w-2xl px-6 py-10" data-testid="review-error">
        <h1 className="text-xl font-semibold text-[var(--v2-text-strong)]">
          {t("review.error.title")}
        </h1>
        <p className="mt-2 text-[var(--v2-text-muted)]">{state.message}</p>
      </main>
    );
  }

  const { intent } = state;
  const terminal = TERMINAL_STATES.includes(intent.state);
  const minutes = minutesRemaining(intent.expires_at_ms, Date.now());

  return (
    <main className="mx-auto max-w-2xl px-6 py-10" data-testid="review-page">
      <h1 className="text-xl font-semibold text-[var(--v2-text-strong)]">
        {t("review.title")}
      </h1>

      <p
        data-testid="review-state"
        className="mt-1 text-sm text-[var(--v2-text-muted)]"
      >
        {t(`review.state.${intent.state}`)}
      </p>

      {!terminal && (
        <p data-testid="review-countdown" className="mt-1 text-sm text-[var(--v2-text-muted)]">
          {t("review.expiresIn", { minutes })}
        </p>
      )}

      <TransactionHash hash={intent.approved_tx_hash} />

      <SigningCeremony
        state={signingContext}
        terminal={terminal}
        running={ceremony.running}
        outcome={ceremony.outcome}
        onSign={runDeviceCeremony}
      />

      <section className="mt-6">
        <h2 className="text-sm font-medium text-[var(--v2-text-muted)]">
          {t("review.details.label")}
        </h2>
        <div className="mt-2">
          <DetailRow label={t("review.details.chain")} value={intent.chain_id} />
          {decodedRows(intent.decoded_tx).map((row) => (
            <DetailRow key={row.key} label={row.key} value={row.value} />
          ))}
        </div>
      </section>
    </main>
  );
}
