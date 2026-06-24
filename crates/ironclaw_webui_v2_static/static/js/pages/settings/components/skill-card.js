import { Badge } from "../../../design-system/badge.js";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { Textarea } from "../../../design-system/input.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

export function SkillCard({
  skill,
  onEdit,
  onRemove,
  onUpdate,
  onSetAutoActivate,
  isRemoving,
  isUpdating,
  isSettingAutoActivate,
  // The global "auto-activate learned skills" master switch. When off, only
  // machine-LEARNED skills stop auto-firing by keyword (hand-written skills are
  // unaffected); a learned skill's per-skill toggle is then shown disabled
  // (greyed) — its saved value is preserved, just overridden until the switch is
  // back on. See `pausedByGlobal` below.
  globalAutoActivate = true,
}) {
  const t = useT();
  const name = skill.name || skill.id;
  const trust = skill.trust || skill.trust_level || "installed";
  const sourceKind = skill.source_kind || "installed";
  const canEdit = Boolean(skill.can_edit);
  const canDelete = Boolean(skill.can_delete);
  // Defaults true: a skill without the field auto-activates.
  const autoActivate = skill.auto_activate !== false;
  // Decision B: the global learned-auto-activation switch governs ONLY
  // machine-learned skills. A learned skill is "paused" (kept, but not
  // auto-firing) when that switch is off; hand-written skills never pause.
  const isLearned = Boolean(skill.is_learned);
  const pausedByGlobal = isLearned && !globalAutoActivate;
  const [isEditing, setIsEditing] = React.useState(false);
  const [draft, setDraft] = React.useState("");
  const [editError, setEditError] = React.useState("");
  const [isLoadingContent, setIsLoadingContent] = React.useState(false);

  React.useEffect(() => {
    if (!isEditing) {
      setDraft("");
      setEditError("");
    }
  }, [isEditing]);

  const startEdit = React.useCallback(async () => {
    setIsLoadingContent(true);
    setEditError("");
    try {
      const response = await onEdit(name);
      setDraft(response?.content || "");
      setIsEditing(true);
    } catch (err) {
      setEditError(err.message || t("skills.contentLoadFailed"));
    } finally {
      setIsLoadingContent(false);
    }
  }, [name, onEdit, t]);

  const saveEdit = React.useCallback(async () => {
    const response = await onUpdate(name, draft);
    if (response?.success) setIsEditing(false);
  }, [draft, name, onUpdate]);

  return html`
    <div className="ext-card border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">${name}</span>
            <${Badge}
              tone=${String(trust).toLowerCase() === "trusted" ? "positive" : "muted"}
              label=${trust}
              size="sm"
            />
            <${Badge}
              tone=${sourceKind === "system" ? "positive" : "muted"}
              label=${t(`skills.source.${sourceKind}`)}
              size="sm"
            />
            ${skill.pending_review &&
            html`<${Badge}
              tone="warning"
              label=${t("skills.pendingReview")}
              size="sm"
            />`}
            ${skill.version &&
            html`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v${skill.version}</span>`}
          </div>

          ${skill.description &&
          html`<div className="mt-1 text-xs text-[var(--v2-text-muted)]">${skill.description}</div>`}

          ${isEditing
            ? html`
                <div className="mt-3">
                  <${Textarea}
                    rows=${12}
                    value=${draft}
                    className="font-mono text-xs leading-5"
                    onInput=${(event) => setDraft(event.currentTarget.value)}
                  />
                </div>
              `
            : html`<${SkillMetadata} skill=${skill} />`}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          ${canEdit && !isEditing &&
          html`
            <${Button}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${isUpdating || isLoadingContent}
              title=${t("skills.edit")}
              onClick=${startEdit}
            >
              <${Icon} name="file" className="h-4 w-4" />
              ${isLoadingContent ? t("skills.loading") : t("skills.edit")}
            <//>
          `}
          ${isEditing &&
          html`
            <${Button}
              type="button"
              variant="ghost"
              size="sm"
              disabled=${isUpdating}
              onClick=${() => {
                setDraft("");
                setIsEditing(false);
              }}
            >
              <${Icon} name="close" className="h-4 w-4" />
              ${t("skills.cancel")}
            <//>
            <${Button}
              type="button"
              variant="primary"
              size="sm"
              disabled=${isUpdating}
              onClick=${saveEdit}
            >
              <${Icon} name="check" className="h-4 w-4" />
              ${isUpdating ? t("skills.saving") : t("skills.save")}
            <//>
          `}
          ${canEdit && !isEditing &&
          html`
            <${Button}
              type="button"
              variant=${autoActivate ? "secondary" : "ghost"}
              size="sm"
              disabled=${isSettingAutoActivate || pausedByGlobal}
              title=${pausedByGlobal
                ? t("skills.autoActivate.titlePaused")
                : autoActivate
                  ? t("skills.autoActivate.titleOn")
                  : t("skills.autoActivate.titleOff")}
              onClick=${() => onSetAutoActivate(name, !autoActivate)}
            >
              <${Icon} name=${autoActivate ? "check" : "close"} className="h-4 w-4" />
              ${autoActivate
                ? pausedByGlobal
                  ? t("skills.autoActivate.onPaused")
                  : t("skills.autoActivate.on")
                : t("skills.autoActivate.off")}
            <//>
          `}
          ${canDelete && !isEditing &&
          html`
            <${Button}
              type="button"
              variant="danger"
              size="sm"
              disabled=${isRemoving}
              title=${t("skills.delete")}
              onClick=${() => onRemove(name)}
            >
              <${Icon} name="trash" className="h-4 w-4" />
              ${t("skills.delete")}
            <//>
          `}
        </div>
      </div>
      ${editError &&
      html`<p className="mt-2 text-xs text-[var(--v2-danger-text)]">${editError}</p>`}
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
