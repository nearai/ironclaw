import { Badge } from "../../../design-system/badge.js";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

// One learned skill awaiting review: a held new skill, or a proposed update to a
// skill the user has since edited. The browser previews the content (or, for an
// update, the user's current version next to the proposal) and approves or
// discards by name.
export function PendingSkillCard({ skill, onApprove, onDiscard, isApproving, isDiscarding }) {
  const t = useT();
  const [expanded, setExpanded] = React.useState(false);
  const isEvolution = skill.kind === "evolution";
  const name = skill.name;
  const busy = isApproving || isDiscarding;

  return html`
    <div className="border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">${name}</span>
            <${Badge}
              tone=${isEvolution ? "muted" : "positive"}
              label=${isEvolution
                ? t("skills.learning.review.proposedBadge")
                : t("skills.learning.review.newBadge")}
              size="sm"
            />
          </div>
          ${skill.description &&
          html`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${skill.description}</div>`}
          <div className="mt-1 text-xs text-[var(--v2-text-muted)]">
            ${isEvolution
              ? t("skills.learning.review.evolutionDescription")
              : t("skills.learning.review.newDescription")}
          </div>
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          <${Button}
            type="button"
            variant="secondary"
            size="sm"
            onClick=${() => setExpanded((value) => !value)}
          >
            <${Icon} name="file" className="h-4 w-4" />
            ${expanded ? t("skills.learning.review.hide") : t("skills.learning.review.preview")}
          <//>
          <${Button}
            type="button"
            variant="primary"
            size="sm"
            disabled=${busy}
            onClick=${() => onApprove(name)}
          >
            <${Icon} name="check" className="h-4 w-4" />
            ${isEvolution
              ? t("skills.learning.review.applyUpdate")
              : t("skills.learning.review.approve")}
          <//>
          <${Button}
            type="button"
            variant="danger"
            size="sm"
            disabled=${busy}
            onClick=${() => onDiscard(name)}
          >
            <${Icon} name="trash" className="h-4 w-4" />
            ${t("skills.learning.review.discard")}
          <//>
        </div>
      </div>

      ${expanded &&
      html`
        <div className="mt-3 space-y-3">
          ${isEvolution
            ? html`
                <${PreviewBlock}
                  label=${t("skills.learning.review.yourVersion")}
                  value=${skill.current_content}
                />
                <${PreviewBlock}
                  label=${t("skills.learning.review.proposedVersion")}
                  value=${skill.proposed_content || ""}
                />
              `
            : html`<${PreviewBlock}
                label=${t("skills.learning.review.skillContent")}
                value=${skill.current_content}
              />`}
        </div>
      `}
    </div>
  `;
}

function PreviewBlock({ label, value }) {
  const preClass =
    "max-h-64 overflow-auto whitespace-pre-wrap rounded-lg border " +
    "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3 font-mono " +
    "text-xs leading-5 text-[var(--v2-text-muted)]";
  return html`
    <div>
      <div className="mb-1 font-mono text-[10px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${label}
      </div>
      <pre className=${preClass}>${value}</pre>
    </div>
  `;
}
