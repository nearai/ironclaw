import { Button } from "../../../design-system/button.js";
import { Card } from "../../../design-system/card.js";
import { FormField, Input, Textarea } from "../../../design-system/input.js";
import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

export function SkillInstallPanel({ onInstall, isInstalling }) {
  const t = useT();
  const [name, setName] = React.useState("");
  const [url, setUrl] = React.useState("");
  const [content, setContent] = React.useState("");
  const [error, setError] = React.useState("");
  const [result, setResult] = React.useState("");

  const submit = React.useCallback(async () => {
    const payload = buildPayload({ name, url, content });
    if (!payload.name) {
      setError(t("skills.nameRequired"));
      return;
    }
    if (!payload.url && !payload.content) {
      setError(t("skills.importSourceRequired"));
      return;
    }
    if (payload.url && !payload.url.startsWith("https://")) {
      setError(t("skills.httpsRequired"));
      return;
    }

    setError("");
    setResult("");
    try {
      const response = await onInstall(payload);
      if (!response?.success) {
        setError(response?.message || t("skills.installFailed"));
        return;
      }

      setName("");
      setUrl("");
      setContent("");
      setResult(response.message || t("skills.installedSuccess", { name: payload.name }));
    } catch (err) {
      setError(err.message || t("skills.installFailed"));
    }
  }, [content, name, onInstall, t, url]);

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

      <div className="grid gap-3 lg:grid-cols-[minmax(0,0.72fr)_minmax(0,1fr)]">
        <${FormField} label=${t("skills.name")} error=${error && !name.trim() ? error : ""}>
          <${Input}
            size="sm"
            value=${name}
            placeholder=${t("skills.namePlaceholder")}
            onInput=${(event) => setName(event.currentTarget.value)}
          />
        <//>
        <${FormField} label=${t("skills.url")} hint=${t("skills.urlHint")}>
          <${Input}
            size="sm"
            value=${url}
            placeholder=${t("skills.urlPlaceholder")}
            onInput=${(event) => setUrl(event.currentTarget.value)}
          />
        <//>
      </div>

      <${FormField} className="mt-3" label=${t("skills.content")} hint=${t("skills.contentHint")}>
        <${Textarea}
          rows=${5}
          value=${content}
          placeholder=${t("skills.contentPlaceholder")}
          onInput=${(event) => setContent(event.currentTarget.value)}
        />
      <//>

      ${error &&
      html`<p className="mt-3 text-sm text-[var(--v2-danger-text)]">${error}</p>`}
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

function buildPayload({ name, url, content }) {
  const payload = { name: name.trim() };
  if (url.trim()) payload.url = url.trim();
  if (content.trim()) payload.content = content.trim();
  return payload;
}
