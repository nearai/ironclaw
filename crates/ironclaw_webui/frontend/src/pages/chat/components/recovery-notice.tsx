import { Button } from "@ironclaw/design-system";

export function RecoveryNotice({ notice, onRecover }) {
  return (
    <div className="mx-auto flex max-w-xl flex-wrap items-center justify-center gap-3 rounded-lg border border-[var(--v2-warning-text)]/30 bg-[var(--v2-warning-soft)] px-4 py-3 text-sm text-[var(--v2-warning-text)]">
      <span>{notice.message}</span>
      {notice.status !== "loading" &&
      (
        <Button
          variant="ghost"
          size="sm"
          onClick={onRecover}
          className="border border-[var(--v2-warning-text)]/40 text-[var(--v2-warning-text)] hover:bg-[var(--v2-warning-soft)] hover:text-[var(--v2-warning-text)]"
        >
          Reload history
        </Button>
      )}
    </div>
  );
}
