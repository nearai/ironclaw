import { Button } from "../../../design-system/button.js";
import { Card } from "../../../design-system/card.js";
import { FormField, Input, Textarea } from "../../../design-system/input.js";
import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

export function SkillInstallPanel({ onInstall, isInstalling, isAdmin = false }) {
  const t = useT();
  const [name, setName] = React.useState("");
  const [content, setContent] = React.useState("");
  const [shared, setShared] = React.useState(false);
  const [fieldErrors, setFieldErrors] = React.useState({ name: "", content: "" });
  const [formError, setFormError] = React.useState("");
  const [result, setResult] = React.useState("");

  const clearFieldErrorIfValid = React.useCallback((field, value) => {
    setFieldErrors((current) => {
      if (!current[field] || !value.trim()) return current;
      return { ...current, [field]: "" };
    });
  }, []);

  const submit = React.useCallback(async () => {
    const payload = buildPayload({ name, content, shared: isAdmin && shared });
    const validationErrors = validatePayload(payload, t);
    if (validationErrors.name || validationErrors.content) {
      setFieldErrors(validationErrors);
      setFormError("");
      setResult("");
      return;
    }

    setFieldErrors({ name: "", content: "" });
    setFormError("");
    setResult("");
    try {
      const response = await onInstall(payload);
      if (!response?.success) {
        setFormError(response?.message || t("skills.installFailed"));
        return;
      }

      setName("");
      setContent("");
      setShared(false);
      setResult(response.message || t("skills.installedSuccess", { name: payload.name }));
    } catch (err) {
      setFormError(err.message || t("skills.installFailed"));
    }
  }, [content, isAdmin, name, onInstall, shared, t]);

  return html`
    <${Card} padding="md">
      <div className="mb-4 flex items-start justify-between gap-4">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${t("skills.import")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">
            ${t("skills.importDesc")}
          </p>
        </div>
      </div>

      <${FormField} label=${t("skills.name")} error=${fieldErrors.name} required>
        <${Input}
          size="sm"
          error=${Boolean(fieldErrors.name)}
          aria-invalid=${fieldErrors.name ? "true" : undefined}
          value=${name}
          placeholder=${t("skills.namePlaceholder")}
          onInput=${(event) => {
            const nextName = event.currentTarget.value;
            setName(nextName);
            clearFieldErrorIfValid("name", nextName);
          }}
        />
      <//>

      <${FormField}
        className="mt-3"
        label=${t("skills.content")}
        error=${fieldErrors.content}
        hint=${t("skills.contentHint")}
        required
      >
        <${Textarea}
          rows=${5}
          error=${Boolean(fieldErrors.content)}
          aria-invalid=${fieldErrors.content ? "true" : undefined}
          value=${content}
          placeholder=${t("skills.contentPlaceholder")}
          onInput=${(event) => {
            const nextContent = event.currentTarget.value;
            setContent(nextContent);
            clearFieldErrorIfValid("content", nextContent);
          }}
        />
      <//>

      ${isAdmin &&
      html`
        <label className="mt-3 flex items-start gap-2 text-sm text-[var(--v2-text)]">
          <input
            type="checkbox"
            className="mt-0.5"
            checked=${shared}
            onChange=${(event) => setShared(event.currentTarget.checked)}
          />
          <span>
            ${t("skills.shareWithAllUsers")}
            <span className="block text-xs text-[var(--v2-text-muted)]">
              ${t("skills.shareWithAllUsersHint")}
            </span>
          </span>
        </label>
      `}

      ${formError &&
      html`<p className="mt-3 text-sm text-[var(--v2-danger-text)]">${formError}</p>`}
      ${result &&
      html`<p className="mt-3 text-sm text-[var(--v2-positive-text)]">${result}</p>`}

      <div className="mt-4 flex justify-end">
        <${Button} type="button" size="sm" disabled=${isInstalling} onClick=${submit}>
          <${Icon} name="upload" className="h-4 w-4" />
          ${isInstalling ? t("skills.installing") : t("skills.install")}
        <//>
      </div>
    <//>
  `;
}

function buildPayload({ name, content, shared }) {
  const payload = { name: name.trim() };
  if (content.trim()) payload.content = content.trim();
  // Omitted (not `false`) when unset so pre-existing backends see the exact
  // pre-#5459 request shape.
  if (shared) payload.shared = true;
  return payload;
}

function validatePayload(payload, t) {
  return {
    name: payload.name ? "" : t("skills.nameRequired"),
    content: payload.content ? "" : t("skills.contentRequired"),
  };
}
