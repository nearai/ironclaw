import { React, html } from "../../../lib/html.js";
import { Card } from "../../../design-system/card.js";
import { Button } from "../../../design-system/button.js";
import { useT } from "../../../lib/i18n.js";
import { useSkills } from "../hooks/useSkills.js";
import { matchesSearch } from "../lib/settings-search.js";
import { SkillCard } from "./skill-card.js";
import { PendingSkillCard } from "./pending-skill-card.js";
import { SkillInstallPanel } from "./skill-install-panel.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";

export function SkillsTab({ searchQuery = "" }) {
  const t = useT();
  const {
    skills,
    query,
    autoActivateLearned,
    learningEnabled,
    requireReview,
    pendingSkills,
    fetchSkillContent,
    installSkill,
    removeSkill,
    updateSkill,
    setSkillAutoActivate,
    setAutoActivateLearned,
    setLearningEnabled,
    setRequireReview,
    approvePendingSkill,
    discardPendingSkill,
    isInstalling,
    isRemoving,
    isUpdating,
    isSettingAutoActivate,
    isSettingAutoActivateLearned,
    isSettingLearningEnabled,
    isSettingRequireReview,
    isApprovingPending,
    isDiscardingPending,
  } = useSkills();
  const [actionError, setActionError] = React.useState("");
  const [actionResult, setActionResult] = React.useState("");

  // Shared shape for the toggle handlers: clear banners, run the mutation,
  // surface the facade's message (or its error) back to the user.
  const runToggle = React.useCallback(
    async (mutation, value) => {
      setActionError("");
      setActionResult("");
      try {
        const response = await mutation(value);
        if (!response?.success) {
          setActionError(response?.message || t("skills.updateFailed"));
          return;
        }
        setActionResult(response.message);
      } catch (err) {
        setActionError(err.message || t("skills.updateFailed"));
      }
    },
    [t]
  );

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

  const handleSetLearningEnabled = React.useCallback(
    (enabled) => runToggle(setLearningEnabled, enabled),
    [runToggle, setLearningEnabled]
  );
  const handleSetRequireReview = React.useCallback(
    (enabled) => runToggle(setRequireReview, enabled),
    [runToggle, setRequireReview]
  );
  const handleSetAutoActivateLearned = React.useCallback(
    (enabled) => runToggle(setAutoActivateLearned, enabled),
    [runToggle, setAutoActivateLearned]
  );

  const handleApprovePending = React.useCallback(async (name) => {
    setActionError("");
    setActionResult("");
    try {
      const response = await approvePendingSkill(name);
      if (!response?.success) {
        setActionError(response?.message || t("skills.updateFailed"));
        return;
      }
      setActionResult(response.message);
    } catch (err) {
      setActionError(err.message || t("skills.updateFailed"));
    }
  }, [approvePendingSkill, t]);

  const handleDiscardPending = React.useCallback(async (name) => {
    if (!window.confirm(t("skills.confirmDiscardPending", { name }))) {
      return;
    }
    setActionError("");
    setActionResult("");
    try {
      const response = await discardPendingSkill(name);
      if (!response?.success) {
        setActionError(response?.message || t("skills.updateFailed"));
        return;
      }
      setActionResult(response.message);
    } catch (err) {
      setActionError(err.message || t("skills.updateFailed"));
    }
  }, [discardPendingSkill, t]);

  let body;
  if (query.isLoading) {
    body = html`
      <${Card} padding="md">
          <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1, 2, 3].map((i) => html`
            <div key=${i} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
              <div>
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `)}
        <//>
    `;
  } else if (query.error) {
    body = html`
      <${Card} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">${t("skills.failedLoad", { message: query.error.message })}</p>
        <//>
    `;
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
      body = html`
        <${Card} padding="lg">
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">${t("skills.noInstalled")}</h3>
          <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("skills.noInstalledDesc")}
          </p>
        <//>
      `;
    } else if (filteredSkills.length === 0) {
      body = html`<${SettingsSearchEmpty} query=${searchQuery} />`;
    } else {
      body = html`
        <div id="skills-list">
          ${skillGroups.map(
            (group) => html`
              <${SkillGroup}
                key=${group.id}
                title=${t(group.labelKey)}
                skills=${group.skills}
                globalAutoActivate=${autoActivateLearned}
                onEdit=${fetchSkillContent}
                onRemove=${handleRemove}
                onUpdate=${handleUpdate}
                onSetAutoActivate=${handleSetAutoActivate}
                isRemoving=${isRemoving}
                isUpdating=${isUpdating}
                isSettingAutoActivate=${isSettingAutoActivate}
              />
            `
          )}
        </div>
      `;
    }
  }

  return html`
    <div className="space-y-4">
      <${SkillLearningControls}
        learningEnabled=${learningEnabled}
        requireReview=${requireReview}
        autoActivateLearned=${autoActivateLearned}
        isSettingLearningEnabled=${isSettingLearningEnabled}
        isSettingRequireReview=${isSettingRequireReview}
        isSettingAutoActivateLearned=${isSettingAutoActivateLearned}
        onSetLearningEnabled=${handleSetLearningEnabled}
        onSetRequireReview=${handleSetRequireReview}
        onSetAutoActivateLearned=${handleSetAutoActivateLearned}
      />
      <${PendingReviewSection}
        pending=${pendingSkills}
        onApprove=${handleApprovePending}
        onDiscard=${handleDiscardPending}
        isApproving=${isApprovingPending}
        isDiscarding=${isDiscardingPending}
      />
      <${SkillInstallPanel} onInstall=${installSkill} isInstalling=${isInstalling} />
      <${SkillActionResult} error=${actionError} result=${actionResult} />
      ${body}
    </div>
  `;
}

// The three stages of skill self-evolution, shown in pipeline order so the
// controls read as one flow: learn (extract) -> review -> use (activate).
function SkillLearningControls({
  learningEnabled,
  requireReview,
  autoActivateLearned,
  isSettingLearningEnabled,
  isSettingRequireReview,
  isSettingAutoActivateLearned,
  onSetLearningEnabled,
  onSetRequireReview,
  onSetAutoActivateLearned,
}) {
  const t = useT();
  return html`
    <div className="space-y-3">
      <${SkillSwitchCard}
        title=${t("skills.learning.selfLearning.title")}
        summary=${learningEnabled
          ? t("skills.learning.selfLearning.summaryOn")
          : t("skills.learning.selfLearning.summaryOff")}
        labelOn=${t("skills.learning.selfLearning.labelOn")}
        labelOff=${t("skills.learning.selfLearning.labelOff")}
        enabled=${learningEnabled}
        isSaving=${isSettingLearningEnabled}
        onToggle=${onSetLearningEnabled}
      />
      <${SkillSwitchCard}
        title=${t("skills.learning.review.title")}
        summary=${requireReview
          ? t("skills.learning.review.summaryOn")
          : t("skills.learning.review.summaryOff")}
        labelOn=${t("skills.learning.review.labelOn")}
        labelOff=${t("skills.learning.review.labelOff")}
        enabled=${requireReview}
        isSaving=${isSettingRequireReview}
        onToggle=${onSetRequireReview}
      />
      <${SkillSwitchCard}
        title=${t("skills.learning.activation.title")}
        summary=${autoActivateLearned
          ? t("skills.learning.activation.summaryOn")
          : t("skills.learning.activation.summaryOff")}
        labelOn=${t("skills.learning.activation.labelOn")}
        labelOff=${t("skills.learning.activation.labelOff")}
        enabled=${autoActivateLearned}
        isSaving=${isSettingAutoActivateLearned}
        onToggle=${onSetAutoActivateLearned}
        dangerWhenOff=${true}
      />
    </div>
  `;
}

// A single binary skill control. The button shows the CURRENT choice and
// clicking it flips to the other. `dangerWhenOff` paints the card with a soft
// red background while off, a persistent cue for a restrictive default (used by
// the "use" switch, where off means nothing auto-activates).
function SkillSwitchCard({
  title,
  summary,
  labelOn,
  labelOff,
  enabled,
  isSaving,
  onToggle,
  dangerWhenOff = false,
}) {
  const cardStyle = dangerWhenOff && !enabled ? { background: "var(--v2-danger-soft)" } : undefined;
  return html`
    <${Card} padding="md" style=${cardStyle}>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="text-sm font-medium text-[var(--v2-text-strong)]">${title}</div>
          <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${summary}</div>
        </div>
        <div className="shrink-0">
          <${Button}
            type="button"
            variant=${enabled ? "secondary" : "ghost"}
            size="sm"
            disabled=${isSaving}
            onClick=${() => onToggle(!enabled)}
          >
            ${enabled ? labelOn : labelOff}
          <//>
        </div>
      </div>
    <//>
  `;
}

function PendingReviewSection({ pending, onApprove, onDiscard, isApproving, isDiscarding }) {
  const t = useT();
  if (!pending || pending.length === 0) return null;
  return html`
    <${Card} padding="md">
      <h3 className="mb-1 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${t("skills.learning.review.pendingTitle", { count: pending.length })}
      </h3>
      <p className="mb-3 text-xs text-[var(--v2-text-muted)]">
        ${t("skills.learning.review.pendingDescription")}
      </p>
      ${pending.map(
        (skill) => html`
          <${PendingSkillCard}
            key=${`${skill.kind}:${skill.name}`}
            skill=${skill}
            onApprove=${onApprove}
            onDiscard=${onDiscard}
            isApproving=${isApproving}
            isDiscarding=${isDiscarding}
          />
        `
      )}
    <//>
  `;
}

function SkillGroup({
  title,
  skills,
  globalAutoActivate,
  onEdit,
  onRemove,
  onUpdate,
  onSetAutoActivate,
  isRemoving,
  isUpdating,
  isSettingAutoActivate,
}) {
  if (skills.length === 0) return null;
  return html`
    <${Card} padding="md">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${title}
      </h3>
      ${skills.map(
        (skill) => html`
          <${SkillCard}
            key=${`${skill.source_kind || "skill"}:${skill.name || skill.id}`}
            skill=${skill}
            globalAutoActivate=${globalAutoActivate}
            onEdit=${onEdit}
            onRemove=${onRemove}
            onUpdate=${onUpdate}
            onSetAutoActivate=${onSetAutoActivate}
            isRemoving=${isRemoving}
            isUpdating=${isUpdating}
            isSettingAutoActivate=${isSettingAutoActivate}
          />
        `
      )}
    <//>
  `;
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
  // Theme-token colors (not raw Tailwind palette) so the text stays legible in
  // both the light and dark v2 themes. Errors use the danger token; plain
  // confirmations use the positive token on a subtle surface — kept neutral
  // rather than alarming red, since the persistent "reduced-safety" signal
  // lives on the auto-activation card (its red `dangerWhenOff` background).
  const style = error
    ? {
        background: "var(--v2-danger-soft)",
        color: "var(--v2-danger-text)",
        border: "1px solid var(--v2-danger-soft)",
      }
    : {
        background: "var(--v2-surface-soft)",
        color: "var(--v2-positive-text)",
        border: "1px solid var(--v2-card-border)",
      };
  return html`
    <div className="rounded-xl px-4 py-3 text-sm" style=${style}>
      ${error || result}
    </div>
  `;
}
