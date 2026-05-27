import { Badge } from "../../../design-system/badge.js";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

export function SkillCard({ skill, onRemove, isRemoving }) {
  const t = useT();
  const name = skill.name || skill.id;
  const trust = skill.trust || skill.trust_level || "installed";
  const canRemove = String(trust).toLowerCase() !== "trusted";

  return html`
    <div className="border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">${name}</span>
            <${Badge}
              tone=${String(trust).toLowerCase() === "trusted" ? "positive" : "muted"}
              label=${trust}
              size="sm"
            />
            ${skill.version &&
            html`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${skill.version}</span>`}
          </div>

          ${skill.description &&
          html`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${skill.description}</div>`}

          <${SkillMetadata} skill=${skill} />
        </div>

        ${canRemove &&
        html`
          <${Button}
            type="button"
            variant="danger"
            size="sm"
            disabled=${isRemoving}
            onClick=${() => onRemove(name)}
          >
            <${Icon} name="trash" className="h-4 w-4" />
            ${t("skills.remove")}
          <//>
        `}
      </div>
    </div>
  `;
}

function SkillMetadata({ skill }) {
  const t = useT();

  return html`
    ${skill.keywords?.length > 0 &&
    html`
      <div className="mt-2 text-xs text-[var(--v2-text-muted)]">
        <span className="text-[var(--v2-text-faint)]">${t("skills.activatesOn")}:</span>
        ${skill.keywords.join(", ")}
      </div>
    `}
    ${skill.usage_hint &&
    html`<div className="mt-2 text-xs text-[var(--v2-text-muted)]">${skill.usage_hint}</div>`}
    ${skill.setup_hint &&
    html`<div className="mt-2 text-xs text-[var(--v2-warning-text)]">${skill.setup_hint}</div>`}
    ${(skill.has_requirements || skill.has_scripts || skill.install_source_url) &&
    html`
      <div className="mt-2 flex flex-wrap gap-1.5">
        ${skill.has_requirements && html`<${MetaChip}>requirements.txt<//>`}
        ${skill.has_scripts && html`<${MetaChip}>scripts/<//>`}
        ${skill.install_source_url && html`<${MetaChip}>${t("skills.imported")}<//>`}
      </div>
    `}
  `;
}

function MetaChip({ children }) {
  return html`
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      ${children}
    </span>
  `;
}
