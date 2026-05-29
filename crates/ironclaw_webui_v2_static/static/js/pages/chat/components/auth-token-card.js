import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { Input } from "../../../design-system/input.js";
import { Icon } from "../../../design-system/icons.js";

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
    <form
      className="mx-auto w-full max-w-lg rounded-xl border border-[rgba(76,167,230,0.34)] bg-[rgba(76,167,230,0.08)] p-4"
      onSubmit=${submit}
    >
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-[rgba(76,167,230,0.28)] bg-[rgba(76,167,230,0.1)] text-[#8fc8f2]">
          <${Icon} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">
          ${gate?.headline || t("authGate.title")}
        </span>
      </div>
      ${gate?.body &&
      html`<div className="mb-3 text-sm text-iron-200">${gate.body}</div>`}
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
        <${Button}
          type="submit"
          variant="primary"
          disabled=${isSubmitting}
        >
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
  `;
}
