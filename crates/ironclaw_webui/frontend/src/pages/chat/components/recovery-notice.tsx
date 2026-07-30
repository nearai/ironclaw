import { Callout } from "@ironclaw/ui";

export function RecoveryNotice({ notice, onRecover }) {
  return (
    <Callout
      tone="warning"
      className="mx-auto max-w-xl justify-center"
      actions={
        notice.status !== "loading" && (
          <button
            type="button"
            onClick={onRecover}
            className="rounded-md border border-current px-2.5 py-1 text-xs font-medium transition-colors hover:bg-[var(--v2-warning-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]"
          >
            Reload history
          </button>
        )
      }
    >
      {notice.message}
    </Callout>
  );
}
