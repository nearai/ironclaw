import { React, html } from "../../../lib/html.js";
import { Card } from "../../../design-system/card.js";
import { useT } from "../../../lib/i18n.js";
import { useSkills } from "../hooks/useSkills.js";
import { matchesSearch } from "../lib/settings-search.js";
import { SkillCard } from "./skill-card.js";
import { SkillInstallPanel } from "./skill-install-panel.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";

export function SkillsTab({ searchQuery = "" }) {
  const t = useT();
  const {
    skills,
    query,
    fetchSkillContent,
    installSkill,
    removeSkill,
    updateSkill,
    isInstalling,
    isRemoving,
    isUpdating,
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
                onEdit=${fetchSkillContent}
                onRemove=${handleRemove}
                onUpdate=${handleUpdate}
                isRemoving=${isRemoving}
                isUpdating=${isUpdating}
              />
            `
          )}
        </div>
      `;
    }
  }

  return html`
    <div className="space-y-4">
      <${SkillInstallPanel} onInstall=${installSkill} isInstalling=${isInstalling} />
      <${SkillActionResult} error=${actionError} result=${actionResult} />
      ${body}
    </div>
  `;
}

function SkillGroup({ title, skills, onEdit, onRemove, onUpdate, isRemoving, isUpdating }) {
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
            onEdit=${onEdit}
            onRemove=${onRemove}
            onUpdate=${onUpdate}
            isRemoving=${isRemoving}
            isUpdating=${isUpdating}
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
  return html`
    <div
      className=${error
        ? "rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
        : "rounded-xl border border-emerald-400/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200"}
    >
      ${error || result}
    </div>
  `;
}
