// @ts-nocheck
import { Card } from "../../../design-system/card.js";
import { Icon } from "../../../design-system/icons.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { useInterfacePreferences } from "../../../lib/interface-preferences.js";
import { matchesSearch } from "../lib/settings-search.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";

function Switch({ checked, label, onChange }) {
  return html`
    <button
      type="button"
      role="switch"
      aria-checked=${checked}
      aria-label=${label}
      onClick=${() => onChange(!checked)}
      className=${[
        "relative inline-flex h-7 w-12 shrink-0 items-center rounded-full border transition",
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

export function AppearanceTab({ searchQuery = "" }) {
  const t = useT();
  const { showChatLogsShortcut, setShowChatLogsShortcut } =
    useInterfacePreferences();
  const title = t("settings.appearance");
  const label = t("settings.field.showChatTerminalShortcut");
  const description = t("settings.field.showChatTerminalShortcutDesc");

  if (
    !matchesSearch(searchQuery, [
      title,
      label,
      description,
      "appearance",
      "interface",
      "chat",
      "terminal",
      "console",
      "logs",
    ])
  ) {
    return html`<${SettingsSearchEmpty} query=${searchQuery} />`;
  }

  return html`
    <${Card} padding="md">
      <div className="flex items-start justify-between gap-6">
        <div className="flex min-w-0 gap-3">
          <span
            className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-accent-text)]"
          >
            <${Icon} name="terminal" className="h-4 w-4" />
          </span>
          <div className="min-w-0">
            <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
              ${label}
            </h3>
            <p className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
              ${description}
            </p>
          </div>
        </div>
        <${Switch}
          checked=${showChatLogsShortcut}
          label=${label}
          onChange=${setShowChatLogsShortcut}
        />
      </div>
    <//>
  `;
}
