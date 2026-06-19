/**
 * AuthTokenCard — rendered when `gate.challengeKind === "manual_token"`.
 *
 * Status Pill + Drawer presentation (AuthGateShell). The drawer holds the
 * masked token input and submit/cancel actions. The token never leaves the
 * submit handler and is cleared on success.
 */
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { Input } from "../../../design-system/input.js";
import { AuthGateShell } from "./auth-gate-shell.js";

export function AuthTokenCard({ gate, onSubmit, onCancel }) {
  const t = useT();
  const [token, setToken] = React.useState("");
  const [error, setError] = React.useState("");
  const [isSubmitting, setIsSubmitting] = React.useState(false);

  const submit = React.useCallback(
    async (event) => {
      event.preventDefault();
      const value = token.trim();
      if (!value) {
        setError(t("authGate.tokenRequired"));
        return;
      }
      setError("");
      setIsSubmitting(true);
      try {
        await onSubmit(value);
        setToken("");
      } catch (err) {
        setError(
          err?.safeAuthGateCode === "credential_stored_gate_resolution_failed"
            ? t("authGate.resolveFailedAfterTokenSaved")
            : t("authGate.submitFailed"),
        );
      } finally {
        setIsSubmitting(false);
      }
    },
    [onSubmit, t, token],
  );

  return html`
    <${AuthGateShell}
      icon="lock"
      headline=${gate?.headline || t("authGate.title")}
      provider=${gate?.provider || ""}
      accountLabel=${gate?.accountLabel || ""}
      body=${gate?.body || ""}
      pillHint=${t("authGate.pillEnterToken")}
    >
      <form onSubmit=${submit}>
        <div className="mb-3">
          <${Input}
            type="password"
            autoComplete="off"
            spellCheck=${false}
            value=${token}
            disabled=${isSubmitting}
            placeholder=${t("authGate.tokenPlaceholder")}
            aria-label=${t("authGate.tokenLabel")}
            error=${Boolean(error)}
            onInput=${(event) => setToken(event.currentTarget.value)}
          />
          ${error &&
          html`
            <p className="mt-2 text-xs text-[var(--v2-danger-text)]" role="alert">
              ${error}
            </p>
          `}
        </div>
        <div className="flex flex-wrap gap-2">
          <${Button} type="submit" variant="primary" disabled=${isSubmitting}>
            ${isSubmitting ? t("authGate.submitting") : t("authGate.submit")}
          <//>
          <${Button}
            type="button"
            variant="secondary"
            disabled=${isSubmitting}
            onClick=${() => onCancel?.()}
          >
            ${t("authGate.cancel")}
          <//>
        </div>
      </form>
    <//>
  `;
}
