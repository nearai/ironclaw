import { Card } from "../../../design-system/card";
import { Icon } from "../../../design-system/icons";
import type { InterfaceTheme } from "../../../design-system/theme";
import { useT } from "../../../lib/i18n";
import { useInterfacePreferences } from "../../../lib/interface-preferences";
import { matchesSearch } from "../lib/settings-search";
import { SettingsSearchEmpty } from "./settings-search-empty";

type SwitchProps = {
  checked: boolean;
  label: string;
  onChange: (checked: boolean) => void;
};

type ThemeOptionProps = {
  checked: boolean;
  icon: "sun" | "moon";
  label: string;
  onSelect: () => void;
  value: InterfaceTheme;
};

type AppearanceTabProps = {
  searchQuery?: string;
  theme: InterfaceTheme;
  onThemeChange: (theme: InterfaceTheme) => void;
};

function Switch({ checked, label, onChange }: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={[
        "relative inline-flex h-7 w-12 shrink-0 items-center rounded-full border transition",
        checked
          ? "border-[color-mix(in_srgb,var(--v2-accent)_45%,transparent)] bg-[color-mix(in_srgb,var(--v2-accent)_22%,transparent)]"
          : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
      ].join(" ")}
    >
      <span
        className={[
          "pointer-events-none inline-block h-5 w-5 rounded-full transition",
          checked
            ? "translate-x-5 bg-[var(--v2-accent-text)]"
            : "translate-x-1 bg-[var(--v2-text-muted)]",
        ].join(" ")}
      />
    </button>
  );
}

export function ThemeOption({
  checked,
  icon,
  label,
  onSelect,
  value,
}: ThemeOptionProps) {
  return (
    <label
      className={[
        "flex cursor-pointer items-center gap-3 rounded-xl border px-4 py-3 text-left transition",
        "has-[:focus-visible]:outline has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-[var(--v2-accent)]",
        checked
          ? "border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
          : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_20%,var(--v2-panel-border))] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
      ].join(" ")}
    >
      <input
        type="radio"
        name="appearance-theme"
        value={value}
        checked={checked}
        onChange={onSelect}
        data-testid={`appearance-theme-${value}`}
        className="h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
      />
      <span className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] text-[var(--v2-accent-text)]">
        <Icon name={icon} className="h-4 w-4" />
      </span>
      <span className="min-w-0 flex-1 text-sm font-semibold">{label}</span>
      {checked && (<Icon name="check" className="h-4 w-4 shrink-0 text-[var(--v2-accent-text)]" />)}
    </label>
  );
}

export function AppearanceTab({
  searchQuery = "",
  theme,
  onThemeChange,
}: AppearanceTabProps) {
  const t = useT();
  const { showChatLogsShortcut, setShowChatLogsShortcut } =
    useInterfacePreferences();
  const title = t("settings.appearance");
  const lightThemeLabel = t("theme.light");
  const darkThemeLabel = t("theme.dark");
  const label = t("settings.field.showChatTerminalShortcut");
  const description = t("settings.field.showChatTerminalShortcutDesc");

  if (
    !matchesSearch(searchQuery, [
      title,
      lightThemeLabel,
      darkThemeLabel,
      label,
      description,
      "appearance",
      "interface",
      "theme",
      "light",
      "dark",
      "chat",
      "terminal",
      "console",
      "logs",
    ])
  ) {
    return <SettingsSearchEmpty query={searchQuery} />;
  }

  return (
    <div className="space-y-5">
      <Card padding="md">
        <h2
          id="appearance-theme-title"
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          {title}
        </h2>
        <div
          className="grid gap-3 sm:grid-cols-2"
          role="radiogroup"
          aria-labelledby="appearance-theme-title"
        >
          <ThemeOption
            checked={theme === "light"}
            icon="sun"
            label={lightThemeLabel}
            onSelect={() => onThemeChange("light")}
            value="light"
          />
          <ThemeOption
            checked={theme === "dark"}
            icon="moon"
            label={darkThemeLabel}
            onSelect={() => onThemeChange("dark")}
            value="dark"
          />
        </div>
      </Card>

      <Card padding="md">
        <div className="flex items-start justify-between gap-6">
          <div className="flex min-w-0 gap-3">
            <span className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-accent-text)]">
              <Icon name="terminal" className="h-4 w-4" />
            </span>
            <div className="min-w-0">
              <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
                {label}
              </h3>
              <p className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
                {description}
              </p>
            </div>
          </div>
          <Switch
            checked={showChatLogsShortcut}
            label={label}
            onChange={setShowChatLogsShortcut}
          />
        </div>
      </Card>
    </div>
  );
}
