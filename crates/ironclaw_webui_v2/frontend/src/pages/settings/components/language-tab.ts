import { html } from "../../../lib/html.js";
import { Card } from "../../../design-system/card.js";
import { AVAILABLE_LANGUAGES, useI18n, useT } from "../../../lib/i18n.js";
import { matchesSearch } from "../lib/settings-search.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";

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
    return html`<${SettingsSearchEmpty} query=${searchQuery} />`;
  }

  return html`
    <${Card} padding="md">
      <h3 className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
        ${t("lang.title")}
      </h3>
      <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
        ${t("lang.description")}
      </p>

      <div className="mt-5 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
        <div className="text-xs text-[var(--v2-text-muted)]">${t("lang.current")}</div>
        <div className="mt-1 flex items-baseline gap-2">
          <span className="text-lg font-semibold text-[var(--v2-text-strong)]">${current.native}</span>
          <span className="font-mono text-xs text-[var(--v2-text-faint)]">${current.name}</span>
        </div>
      </div>

      <div className="mt-4 grid gap-2 sm:grid-cols-2">
        ${languages.map(
          (l) => html`
            <button
              key=${l.code}
              type="button"
              onClick=${() => setLang(l.code)}
              className=${[
                "flex items-center justify-between gap-3 rounded-xl border px-4 py-3 text-left",
                l.code === lang
                  ? "border-[color-mix(in_srgb,var(--v2-accent)_35%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
                  : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_20%,var(--v2-panel-border))] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
              ].join(" ")}
            >
              <div className="min-w-0">
                <div className="truncate text-sm font-medium">${l.native}</div>
                <div className="truncate font-mono text-[11px] text-[var(--v2-text-faint)]">${l.name}</div>
              </div>
              <div className="shrink-0 font-mono text-[11px] text-[var(--v2-text-faint)]">${l.code}</div>
            </button>
          `
        )}
      </div>
    <//>
  `;
}
