
import { useT } from "../../../lib/i18n";

export function RecoveryNotice({ notice, onRecover }) {
  const t = useT();
  return (
    <div className="mx-auto flex max-w-xl flex-wrap items-center justify-center gap-3 rounded-lg border border-[color-mix(in_srgb,var(--v2-warning-text)_35%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] px-4 py-3 text-ui text-[var(--v2-warning-text)]">
      <span>{notice.message}</span>
      {notice.status !== "loading" &&
      (
        <button
          type="button"
          onClick={onRecover}
          className="rounded-md border border-copper/40 px-2.5 py-1 text-ui-sm font-medium hover:bg-[var(--v2-warning-soft)]"
        >
          {t("chat.reloadHistory")}
        </button>
      )}
    </div>
  );
}
