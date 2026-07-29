import { Card } from "@ironclaw/design-system";
import { Text } from "@ironclaw/design-system";
import { AVAILABLE_LANGUAGES, useI18n, useT } from "../../../lib/i18n";
import { matchesSearch } from "../lib/settings-search";
import { SettingsSearchEmpty } from "./settings-search-empty";

export function LanguageTab({ searchQuery = "" }) {
  const t = useT();
  const { lang, setLang } = useI18n();

  const current = AVAILABLE_LANGUAGES.find((l) => l.code === lang) || AVAILABLE_LANGUAGES[0];
  const languages = AVAILABLE_LANGUAGES.filter((language) =>
    matchesSearch(searchQuery, [
      language.code,
      language.name,
      language.native,
    ])
  );

  if (languages.length === 0) {
    return (<SettingsSearchEmpty query={searchQuery} />);
  }

  return (
    <Card padding="md">
      <Text as="h3" variant="eyebrow" tone="accent" className="mb-2">
        {t("lang.title")}
      </Text>
      <Text variant="body" tone="muted">
        {t("lang.description")}
      </Text>

      <div className="mt-5 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
        <Text as="div" variant="caption" tone="muted">{t("lang.current")}</Text>
        <div className="mt-1 flex items-baseline gap-2">
          <span className="text-lg font-medium text-[var(--v2-text-strong)]">{current.native}</span>
          <Text variant="mono" tone="faint">{current.name}</Text>
        </div>
      </div>

      <div className="mt-4 grid gap-2 sm:grid-cols-2">
        {languages.map(
          (l) => (
            <button
              key={l.code}
              type="button"
              onClick={() => setLang(l.code)}
              className={[
                "flex items-center justify-between gap-3 rounded-xl border px-4 py-3 text-left",
                l.code === lang
                  ? "border-[color-mix(in_srgb,var(--v2-accent)_35%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
                  : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_20%,var(--v2-panel-border))] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
              ].join(" ")}
            >
              <div className="min-w-0">
                <div className="truncate text-sm font-medium">{l.native}</div>
                <div className="truncate font-mono text-[11px] text-[var(--v2-text-faint)]">{l.name}</div>
              </div>
              <div className="shrink-0 font-mono text-[11px] text-[var(--v2-text-faint)]">{l.code}</div>
            </button>
          )
        )}
      </div>
    </Card>
  );
}
