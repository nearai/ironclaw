import { Button } from "../../../design-system/button.js";
import { FormField, Input } from "../../../design-system/input.js";
import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { useSharedCredential } from "../hooks/useSharedCredential.js";

// Operator-only affordance for setting a tenant-shared credential (#5459),
// embedded inside the extension Configure modal.
//
// The admin sets one key for the whole tenant; every user then resolves it
// without being prompted individually — distinct from the per-user secret
// fields above it in the modal. The raw value is a write-only secret: it lives
// only in this component's local `value` state and the request body, is cleared
// from state on success, and is never logged to the console or persisted
// client-side.
//
// `defaultHandle` is the credential handle declared by the extension manifest
// (`runtime_credentials.handle`) and is always supplied by the caller — the
// modal renders one section per declared shared credential. The handle is
// therefore FIXED: shown read-only as context, never an editable input. The
// form asks only for the value.
export function SharedCredentialSection({ defaultHandle = "" }) {
  const t = useT();
  const { setCredential, isSaving } = useSharedCredential();
  const [value, setValue] = React.useState("");
  const [fieldErrors, setFieldErrors] = React.useState({ value: "" });
  const [formError, setFormError] = React.useState("");
  const [savedHandle, setSavedHandle] = React.useState("");

  const clearFieldErrorIfValid = React.useCallback((field, next) => {
    setFieldErrors((current) => {
      if (!current[field] || !next.trim()) return current;
      return { ...current, [field]: "" };
    });
  }, []);

  const submit = React.useCallback(async () => {
    const validationErrors = {
      // The value is intentionally not trimmed — a secret may legitimately
      // contain leading/trailing whitespace — but it must be non-empty.
      value: value ? "" : t("sharedCredential.valueRequired"),
    };
    if (validationErrors.value) {
      setFieldErrors(validationErrors);
      setFormError("");
      setSavedHandle("");
      return;
    }

    setFieldErrors({ value: "" });
    setFormError("");
    setSavedHandle("");
    try {
      const result = await setCredential(defaultHandle, value);
      // Clear the raw secret from local state immediately on success so it
      // does not linger in the form after it has been stored.
      setValue("");
      setSavedHandle(result?.handle || defaultHandle);
    } catch (err) {
      if (err?.status === 403) {
        setFormError(t("sharedCredential.adminOnly"));
      } else {
        setFormError(err?.message || t("sharedCredential.failed"));
      }
    }
  }, [defaultHandle, value, setCredential, t]);

  return html`
    <div className="mt-6 border-t border-white/12 pt-5">
      <h4 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        ${t("sharedCredential.title")}
      </h4>
      <p className="mt-1 text-xs leading-5 text-iron-300">
        ${t("sharedCredential.description")}
      </p>

      <p className="mt-3 text-xs text-iron-300">
        <span className="text-iron-400">${t("sharedCredential.handleLabel")}</span>
        <span className="ml-1.5 font-mono text-iron-100">${defaultHandle}</span>
      </p>
      <p className="mt-1 text-xs leading-5 text-iron-500">
        ${t("sharedCredential.handleFixedHint")}
      </p>

      <div className="mt-4 space-y-3">
        <${FormField}
          label=${t("sharedCredential.value")}
          error=${fieldErrors.value}
          hint=${t("sharedCredential.valueHint")}
          required
        >
          <${Input}
            type="password"
            size="sm"
            autoComplete="off"
            error=${Boolean(fieldErrors.value)}
            aria-invalid=${fieldErrors.value ? "true" : undefined}
            value=${value}
            placeholder=${t("sharedCredential.valuePlaceholder")}
            onInput=${(event) => {
              const next = event.currentTarget.value;
              setValue(next);
              clearFieldErrorIfValid("value", next);
            }}
          />
        <//>
      </div>

      ${formError &&
      html`<p className="mt-3 text-sm text-[var(--v2-danger-text)]" role="alert">${formError}</p>`}
      ${savedHandle &&
      html`<p className="mt-3 text-sm text-mint" role="status">
        ${t("sharedCredential.saved", { handle: savedHandle })}
      </p>`}

      <div className="mt-4 flex justify-end">
        <${Button} type="button" size="sm" disabled=${isSaving} onClick=${submit}>
          <${Icon} name="shield" className="h-4 w-4" />
          ${isSaving ? t("sharedCredential.saving") : t("sharedCredential.submit")}
        <//>
      </div>
    </div>
  `;
}
