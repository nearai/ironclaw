/**
 * OobeRestorePill — the "Show suggestions" pill shown inside the composer once
 * the OOBE suggestion drawer has been section-dismissed (empty-state owns the
 * drawer-visibility state and renders this pill). Clicking the label reopens
 * the drawer; the × dismisses it fully.
 *
 * Lazy-loaded from empty-state so its markup never lands in the eager /chat
 * bundle — it can only ever appear after the (already lazy) suggestion surface
 * has loaded and the user has dismissed it.
 */
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";

export function OobeRestorePill({
  onRestore,
  onDismiss,
}: {
  onRestore: () => void;
  onDismiss: () => void;
}) {
  const t = useT();
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 flex justify-start p-3">
      <span className="pointer-events-auto inline-flex items-center gap-1 rounded-full border border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] py-1 pl-2.5 pr-1 text-[12.5px] font-medium text-[var(--v2-accent-text)]">
        <button
          type="button"
          onClick={onRestore}
          className="inline-flex items-center gap-1.5"
        >
          <Icon name="spark" className="h-3.5 w-3.5" />
          {t("chat.oobe.showSuggestions")}
        </button>
        <button
          type="button"
          onClick={onDismiss}
          aria-label={t("chat.oobe.dismiss")}
          className="grid h-5 w-5 place-items-center rounded-full text-[var(--v2-accent-text)] transition-colors hover:bg-[color-mix(in_srgb,var(--v2-accent)_22%,transparent)]"
        >
          <Icon name="close" className="h-3 w-3" />
        </button>
      </span>
    </div>
  );
}
