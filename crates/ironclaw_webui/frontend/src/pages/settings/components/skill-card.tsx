import { Badge } from "../../../design-system/badge";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { Textarea } from "../../../design-system/input";
import React from "react";
import { useT } from "../../../lib/i18n";

export function SkillCard({
  skill,
  onEdit,
  onRemove,
  onUpdate,
  onSetAutoActivate,
  isRemoving,
  isUpdating,
  isSettingAutoActivate,
}) {
  const t = useT();
  const name = skill.name || skill.id;
  const trust = skill.trust || skill.trust_level || "installed";
  const sourceKind = skill.source_kind || "installed";
  const canEdit = Boolean(skill.can_edit);
  const canDelete = Boolean(skill.can_delete);
  // Defaults true: a skill without the field auto-activates.
  const autoActivate = skill.auto_activate !== false;
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

  return (
    <div className="ext-card border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text)]">{name}</span>
            <Badge
              tone={String(trust).toLowerCase() === "trusted" ? "positive" : "muted"}
              label={trust}
              size="sm"
            />
            <Badge
              tone={sourceKind === "system" ? "positive" : "muted"}
              label={t(`skills.source.${sourceKind}`)}
              size="sm"
            />
            {skill.version &&
            (<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">v{skill.version}</span>)}
          </div>

          {skill.description &&
          (<div className="mt-1 text-xs text-[var(--v2-text-muted)]">{skill.description}</div>)}

          {isEditing
            ? (
                <div className="mt-3">
                  <Textarea
                    rows={12}
                    value={draft}
                    className="font-mono text-xs leading-5"
                    onInput={(event) => setDraft(event.currentTarget.value)}
                  />
                </div>
              )
            : (<SkillMetadata skill={skill} />)}
        </div>

        <div className="flex shrink-0 flex-wrap justify-end gap-2">
          {canEdit && !isEditing &&
          (
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={isUpdating || isLoadingContent}
              title={t("skills.edit")}
              onClick={startEdit}
            >
              <Icon name="file" className="h-4 w-4" />
              {isLoadingContent ? t("skills.loading") : t("skills.edit")}
            </Button>
          )}
          {isEditing &&
          (
            <>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={isUpdating}
              onClick={() => {
                setDraft("");
                setIsEditing(false);
              }}
            >
              <Icon name="close" className="h-4 w-4" />
              {t("skills.cancel")}
            </Button>
            <Button
              type="button"
              variant="primary"
              size="sm"
              disabled={isUpdating}
              onClick={saveEdit}
            >
              <Icon name="check" className="h-4 w-4" />
              {isUpdating ? t("skills.saving") : t("skills.save")}
            </Button>
            </>
          )}
          {canEdit && !isEditing &&
          (
            <Button
              type="button"
              variant={autoActivate ? "secondary" : "ghost"}
              size="sm"
              disabled={isSettingAutoActivate}
              title={autoActivate
                ? t("skills.autoActivateOnTitle")
                : t("skills.autoActivateOffTitle")}
              onClick={() => onSetAutoActivate(name, !autoActivate)}
            >
              <Icon name={autoActivate ? "check" : "close"} className="h-4 w-4" />
              {autoActivate
                ? t("skills.autoActivateOnLabel")
                : t("skills.autoActivateOffLabel")}
            </Button>
          )}
          {canDelete && !isEditing &&
          (
            <Button
              type="button"
              variant="danger"
              size="sm"
              disabled={isRemoving}
              title={t("skills.delete")}
              onClick={() => onRemove(name)}
            >
              <Icon name="trash" className="h-4 w-4" />
              {t("skills.delete")}
            </Button>
          )}
        </div>
      </div>
      {editError &&
      (<p className="mt-2 text-xs text-[var(--v2-danger-text)]">{editError}</p>)}
    </div>
  );
}

function SkillMetadata({ skill }) {
  const t = useT();

  return (
    <>
    {skill.keywords?.length > 0 &&
    (
      <div className="mt-2 text-xs text-[var(--v2-text-muted)]">
        <span className="text-[var(--v2-text-faint)]">{t("skills.activatesOn")}:</span>
        {skill.keywords.join(", ")}
      </div>
    )}
    {skill.usage_hint &&
    (<div className="mt-2 text-xs text-[var(--v2-text-muted)]">{skill.usage_hint}</div>)}
    {skill.setup_hint &&
    (<div className="mt-2 text-xs text-[var(--v2-warning-text)]">{skill.setup_hint}</div>)}
    {(skill.has_requirements || skill.has_scripts || skill.install_source_url) &&
    (
      <div className="mt-2 flex flex-wrap gap-1.5">
        {skill.has_requirements && (<MetaChip>requirements.txt</MetaChip>)}
        {skill.has_scripts && (<MetaChip>scripts/</MetaChip>)}
        {skill.install_source_url && (<MetaChip>{t("skills.imported")}</MetaChip>)}
      </div>
    )}
    </>
  );
}

function MetaChip({ children }) {
  return (
    <span className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]">
      {children}
    </span>
  );
}
