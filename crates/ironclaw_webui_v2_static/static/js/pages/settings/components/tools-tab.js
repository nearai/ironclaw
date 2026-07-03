import { Icon } from "../../../design-system/icons.js";
import { Badge } from "../../../design-system/badge.js";
import { Card } from "../../../design-system/card.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { useTools } from "../hooks/useTools.js";
import { matchesSearch } from "../lib/settings-search.js";

const AUTO_APPROVE_KEY = "agent.auto_approve_tools";

function translatedToolDescription(t, tool) {
  const key = `tools.description.${tool.name}`;
  const translated = t(key);
  return translated && translated !== key ? translated : tool.description || "";
}

function SavedIndicator({ visible }) {
  const t = useT();
  if (!visible) return null;
  return html`
    <span className="font-mono text-[11px] text-[var(--v2-accent-text)]" role="status">
      ${t("tools.saved")}
    </span>
  `;
}

function Switch({ checked, disabled = false, label, onChange }) {
  return html`
    <button
      type="button"
      role="switch"
      aria-checked=${checked}
      aria-label=${label}
      disabled=${disabled}
      onClick=${() => !disabled && onChange(!checked)}
      className=${[
        "relative inline-flex h-7 w-12 shrink-0 items-center rounded-full border transition",
        disabled ? "cursor-not-allowed opacity-60" : "cursor-pointer",
        checked
          ? "border-[color-mix(in_srgb,var(--v2-accent)_45%,transparent)] bg-[color-mix(in_srgb,var(--v2-accent)_22%,transparent)]"
          : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
      ].join(" ")}
    >
      <span
        className=${[
          "pointer-events-none inline-block h-5 w-5 rounded-full transition",
          checked
            ? "translate-x-5 bg-[var(--v2-accent-text)]"
            : "translate-x-1 bg-[var(--v2-text-muted)]",
        ].join(" ")}
      />
    </button>
  `;
}

function AutoApproveCard({ settings, onSave, savedKeys, isLoading }) {
  const t = useT();
  const label = t("settings.field.autoApproveEligibleTools");
  // Absent → default ON (mirrors backend AUTO_APPROVE_DEFAULT_ENABLED).
  const raw = settings?.[AUTO_APPROVE_KEY];
  const checked = raw == null ? true : raw === true || raw === "true";

  return html`
    <${Card} padding="md" className="flex items-center justify-between gap-6">
      <div className="min-w-0">
        <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
          ${label}
        </h3>
        <p className="mt-1 text-sm text-[var(--v2-text-muted)]">
          ${t("settings.field.autoApproveEligibleToolsDesc")}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <${SavedIndicator} visible=${savedKeys?.[AUTO_APPROVE_KEY]} />
        <${Switch}
          checked=${checked}
          disabled=${isLoading}
          label=${label}
          onChange=${(value) => onSave(AUTO_APPROVE_KEY, value)}
        />
      </div>
    <//>
  `;
}

function ToolRow({ tool, onPermissionChange, isSaved }) {
  const t = useT();
  const description = translatedToolDescription(t, tool);
  const permissionStates = [
    { value: "default", label: t("tools.followDefault"), tone: "neutral" },
    { value: "always_allow", label: t("tools.alwaysAllow"), tone: "positive" },
    { value: "ask_each_time", label: t("tools.askEachTime"), tone: "warning" },
    { value: "disabled", label: t("tools.disabled"), tone: "danger" },
  ];
  const sourceLabels = {
    default: t("tools.sourceDefault"),
    global: t("tools.sourceGlobal"),
    override: t("tools.sourceOverride"),
  };

  const isLocked = tool.locked;
  const current =
    permissionStates.find((p) => p.value === tool.state) || permissionStates[1];
  const effectiveSource = tool.effective_source || "default";
  const selectedState = effectiveSource === "override" ? tool.state : "default";
  const isDefault = effectiveSource === "default" && tool.state === tool.default_state;

  return html`
    <div
      data-testid="settings-tool-row"
      data-tool-name=${tool.name}
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="flex min-w-0 items-center gap-3">
        ${isLocked &&
        html`<span data-testid="settings-tool-lock" className="shrink-0">
          <${Icon}
            name="lock"
            className="h-3.5 w-3.5 text-[var(--v2-text-faint)]"
          />
        </span>`}
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate font-mono text-sm text-[var(--v2-text)]"
              >${tool.name}</span
            >
            ${isDefault &&
            html`
              <span
                className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]"
              >
                ${t("tools.default")}
              </span>
            `}
            <span
              className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]"
            >
              ${sourceLabels[effectiveSource] || sourceLabels.default}
            </span>
          </div>
          ${description &&
          html`
            <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">
              ${description}
            </div>
          `}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-3">
        ${isLocked
          ? html`<${Badge} tone=${current.tone} label=${current.label} size="sm" />`
          : html`
              <select
                value=${selectedState}
                onChange=${(e) => onPermissionChange(tool.name, e.target.value)}
                aria-label=${t("tools.permissionFor", { name: tool.name })}
                className="v2-select h-8 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2.5 font-mono text-xs text-[var(--v2-text-strong)] outline-none focus:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]"
              >
                ${permissionStates.map(
                  (p) =>
                    html`<option key=${p.value} value=${p.value}>
                      ${p.label}
                    </option>`
                )}
              </select>
            `}
        ${isSaved &&
        html`
          <span className="font-mono text-[11px] text-[var(--v2-accent-text)]"
            >${t("tools.saved")}</span
          >
        `}
      </div>
    </div>
  `;
}

export function ToolsTab({
  settings = {},
  onSave = () => {},
  savedKeys = {},
  isLoading = false,
  searchQuery = "",
}) {
  const t = useT();
  const { tools, query, setPermission, savedTools } = useTools();

  if (query.isLoading) {
    return html`
      <div className="space-y-4">
        <${AutoApproveCard}
          settings=${settings}
          onSave=${onSave}
          savedKeys=${savedKeys}
          isLoading=${isLoading}
        />
        <${Card} padding="md">
          <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1, 2, 3, 4, 5].map(
            (i) => html`
              <div
                key=${i}
                className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-3.5 first:border-0"
              >
                <div className="h-4 w-36 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="h-8 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              </div>
            `
          )}
        <//>
      </div>
    `;
  }

  if (query.error) {
    return html`
      <div className="space-y-4">
        <${AutoApproveCard}
          settings=${settings}
          onSave=${onSave}
          savedKeys=${savedKeys}
          isLoading=${isLoading}
        />
        <${Card} padding="md">
          <p className="text-sm text-[var(--v2-danger-text)]">
            ${t("tools.failedLoad", { message: query.error.message })}
          </p>
        <//>
      </div>
    `;
  }

  const filtered = tools.filter((tool) => {
    const description = translatedToolDescription(t, tool);
    return matchesSearch(searchQuery, [
      tool.name,
      tool.description,
      description,
      tool.state,
      tool.default_state,
      tool.effective_source,
      tool.state === "disabled" ? t("tools.disabled") : "",
    ]);
  });

  return html`
    <div className="space-y-4">
      <${AutoApproveCard}
        settings=${settings}
        onSave=${onSave}
        savedKeys=${savedKeys}
        isLoading=${isLoading}
      />

      ${searchQuery &&
      html`
        <div className="flex justify-end">
          <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">
            ${filtered.length} / ${tools.length}
          </span>
        </div>
      `}

      <${Card} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("tools.permissions")}
        </h3>
        ${filtered.length === 0
          ? html`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("tools.noMatch")}
            </p>`
          : filtered.map(
              (tool) =>
                html`
                  <${ToolRow}
                    key=${tool.name}
                    tool=${tool}
                    onPermissionChange=${setPermission}
                    isSaved=${savedTools[tool.name]}
                  />
                `
            )}
      <//>
    </div>
  `;
}
