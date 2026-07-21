
export function RecoveryNotice({ notice, onRecover }) {
  return (
    <div className="mx-auto flex max-w-xl flex-wrap items-center justify-center gap-3 rounded-lg border border-[var(--v2-warning-text)]/30 bg-[var(--v2-warning-soft)] px-4 py-3 text-sm text-[var(--v2-warning-text)]">
      <span>{notice.message}</span>
      {notice.status !== "loading" &&
      (
        <button
          type="button"
          onClick={onRecover}
          className="rounded-md border border-[var(--v2-warning-text)]/40 px-2.5 py-1 text-xs font-medium hover:bg-[var(--v2-warning-soft)]"
        >
          Reload history
        </button>
      )}
    </div>
  );
}
