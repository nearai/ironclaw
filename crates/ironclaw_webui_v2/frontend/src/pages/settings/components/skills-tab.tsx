// @ts-nocheck
import React from "react";
import { Card } from "../../../design-system/card";
import { Button } from "../../../design-system/button";
import { useT } from "../../../lib/i18n";
import { useSkills } from "../hooks/useSkills";
import { matchesSearch } from "../lib/settings-search";
import { SkillCard } from "./skill-card";
import { SkillInstallPanel } from "./skill-install-panel";
import { SettingsSearchEmpty } from "./settings-search-empty";

export function SkillsTab({ searchQuery = "" }) {
  const t = useT();
  const {
    skills,
    query,
    autoActivateLearned,
    fetchSkillContent,
    installSkill,
    removeSkill,
    updateSkill,
    setSkillAutoActivate,
    setAutoActivateLearned,
    isInstalling,
    isRemoving,
    isUpdating,
    isSettingAutoActivate,
    isSettingAutoActivateLearned,
  } = useSkills();
  const [actionError, setActionError] = React.useState("");
  const [actionResult, setActionResult] = React.useState("");

  const handleRemove = React.useCallback(async (name) => {
    if (!window.confirm(t("skills.confirmDelete", { name }))) return;
    setActionError("");
    setActionResult("");
    try {
      const response = await removeSkill(name);
      if (!response?.success) {
        setActionError(response?.message || t("skills.removeFailed"));
        return;
      }
      setActionResult(response.message || t("skills.removed", { name }));
    } catch (err) {
      setActionError(err.message || t("skills.removeFailed"));
    }
  }, [removeSkill, t]);

  const handleUpdate = React.useCallback(async (name, content) => {
    if (!content.trim()) {
      setActionError(t("skills.contentRequired"));
      setActionResult("");
      return { success: false, message: t("skills.contentRequired") };
    }
    setActionError("");
    setActionResult("");
    try {
      const response = await updateSkill({ name, content });
      if (!response?.success) {
        setActionError(response?.message || t("skills.updateFailed"));
        return response;
      }
      setActionResult(response.message || t("skills.updated", { name }));
      return response;
    } catch (err) {
      const message = err.message || t("skills.updateFailed");
      setActionError(message);
      return { success: false, message };
    }
  }, [t, updateSkill]);

  const handleSetAutoActivate = React.useCallback(async (name, enabled) => {
    setActionError("");
    setActionResult("");
    try {
      const response = await setSkillAutoActivate({ name, enabled });
      if (!response?.success) {
        setActionError(response?.message || t("skills.updateFailed"));
        return;
      }
      setActionResult(response.message);
    } catch (err) {
      setActionError(err.message || t("skills.updateFailed"));
    }
  }, [setSkillAutoActivate, t]);

  const handleSetAutoActivateLearned = React.useCallback(async (enabled) => {
    setActionError("");
    setActionResult("");
    try {
      const response = await setAutoActivateLearned(enabled);
      if (!response?.success) {
        setActionError(response?.message || t("skills.updateFailed"));
        return;
      }
      setActionResult(response.message);
    } catch (err) {
      setActionError(err.message || t("skills.updateFailed"));
    }
  }, [setAutoActivateLearned, t]);

  let body;
  if (query.isLoading) {
    body = (
      <Card padding="md">
          <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          {[1, 2, 3].map((i) => (
            <div key={i} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
              <div>
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          ))}
        </Card>
    );
  } else if (query.error) {
    body = (
      <Card padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">{t("skills.failedLoad", { message: query.error.message })}</p>
        </Card>
    );
  } else {
    const filteredSkills = skills.filter((skill) =>
      matchesSearch(searchQuery, [
        skill.name,
        skill.id,
        skill.description,
        skill.keywords,
        skill.trust_level,
        skill.source_kind,
        skill.version,
      ])
    );

    const skillGroups = groupSkills(filteredSkills);

    if (skills.length === 0) {
      body = (
        <Card padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">{t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            {t("skills.noInstalledDesc")}
          </p>
        </Card>
      );
    } else if (filteredSkills.length === 0) {
      body = (<SettingsSearchEmpty query={searchQuery} />);
    } else {
      body = (
        <div id="skills-list">
          {skillGroups.map(
            (group) => (
              <SkillGroup
                key={group.id}
                title={t(group.labelKey)}
                skills={group.skills}
                onEdit={fetchSkillContent}
                onRemove={handleRemove}
                onUpdate={handleUpdate}
                onSetAutoActivate={handleSetAutoActivate}
                isRemoving={isRemoving}
                isUpdating={isUpdating}
                isSettingAutoActivate={isSettingAutoActivate}
              />
            )
          )}
        </div>
      );
    }
  }

  return (
    <div className="space-y-4">
      <LearnedAutoActivateCard
        enabled={autoActivateLearned}
        isSaving={isSettingAutoActivateLearned}
        onToggle={handleSetAutoActivateLearned}
      />
      <SkillInstallPanel onInstall={installSkill} isInstalling={isInstalling} />
      <SkillActionResult error={actionError} result={actionResult} />
      {body}
    </div>
  );
}

// Global master switch for keyword/criteria skill activation. When off, EVERY
// skill (learned, user-authored, and bundled) stays invokable via an explicit
// /name mention but no longer auto-activates by keyword. This is a true global
// default, not learned-only; the per-skill toggle on `SkillCard` is the
// per-skill counterpart.
function LearnedAutoActivateCard({ enabled, isSaving, onToggle }) {
  const t = useT();
  // When auto-activation is off, give the whole card a light-red background as a
  // persistent "default is off" cue. Inline style overrides the Card variant's
  // own background reliably (no Tailwind class-ordering ambiguity).
  const cardStyle = enabled ? undefined : { background: "var(--v2-danger-soft)" };
  return (
    <Card padding="md" style={cardStyle}>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="text-sm font-medium text-[var(--v2-text-strong)]">
            {enabled
              ? t("skills.defaultAutoActivationEnabled")
              : t("skills.defaultAutoActivationDisabled")}
          </div>
          <div className="mt-1 text-xs text-[var(--v2-text-muted)]">
            {enabled
              ? t("skills.defaultAutoActivationOnDesc")
              : t("skills.defaultAutoActivationOffDesc")}
          </div>
        </div>
        <div className="shrink-0">
          <Button
            type="button"
            variant={enabled ? "secondary" : "ghost"}
            size="sm"
            disabled={isSaving}
            onClick={() => onToggle(!enabled)}
          >
            {enabled
              ? t("skills.defaultAutoActivationOnButton")
              : t("skills.defaultAutoActivationOffButton")}
          </Button>
        </div>
      </div>
    </Card>
  );
}

function SkillGroup({
  title,
  skills,
  onEdit,
  onRemove,
  onUpdate,
  onSetAutoActivate,
  isRemoving,
  isUpdating,
  isSettingAutoActivate,
}) {
  if (skills.length === 0) return null;
  return (
    <Card padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        {title}
      </h3>
      {skills.map(
        (skill) => (
          <SkillCard
            key={`${skill.source_kind || "skill"}:${skill.name || skill.id}`}
            skill={skill}
            onEdit={onEdit}
            onRemove={onRemove}
            onUpdate={onUpdate}
            onSetAutoActivate={onSetAutoActivate}
            isRemoving={isRemoving}
            isUpdating={isUpdating}
            isSettingAutoActivate={isSettingAutoActivate}
          />
        )
      )}
    </Card>
  );
}

function groupSkills(skills) {
  const groups = [
    { id: "user", labelKey: "skills.group.user", skills: [] },
    { id: "system", labelKey: "skills.group.system", skills: [] },
    { id: "workspace", labelKey: "skills.group.workspace", skills: [] },
  ];
  const fallback = groups[0];

  for (const skill of skills) {
    const sourceKind = skill.source_kind || "";
    const group =
      sourceKind === "system"
        ? groups[1]
        : sourceKind === "workspace"
          ? groups[2]
          : fallback;
    group.skills.push(skill);
  }

  return groups.filter((group) => group.skills.length > 0);
}

function SkillActionResult({ error, result }) {
  if (!error && !result) return null;
  return (
    <div
      data-testid="skill-action-result"
      className={error
        ? "rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
        : "rounded-xl border border-[color-mix(in_srgb,var(--v2-positive-text)_35%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] px-4 py-3 text-sm text-[var(--v2-positive-text)]"}
    >
      {error || result}
    </div>
  );
}
