/**
 * AuthGateShell — shared chrome for auth gates (Status Pill + Drawer pattern).
 *
 * Presentation only: renders a collapsible pill header and an expandable
 * drawer. It owns NO security logic and handles NO credentials — the action
 * area (CTA / token input / cancel) is supplied by the calling card via
 * `children`, so each card keeps its own form semantics and security guards.
 *
 * Props
 *   icon          design-system icon name for the header chip (default "lock")
 *   headline      bold pill title
 *   provider      optional provider name (subtitle)
 *   accountLabel  optional account label (subtitle; takes precedence)
 *   body          optional descriptive text shown at the top of the drawer
 *   expiresAt     optional ISO timestamp rendered as an expiry hint
 *   pillHint      short call-to-action text on the right of the pill
 *   defaultExpanded boolean (default true — active gates block the run)
 *   controlsId    id used for aria-controls / drawer id
 *   children      drawer action area (CTA, input, cancel)
 */
import React from "react";
import { useT } from "../../../lib/i18n";
import { Icon } from "@ironclaw/design-system";

export function AuthGateShell({
  icon = "lock",
  headline,
  provider = "",
  accountLabel = "",
  body = "",
  expiresAt = null,
  pillHint = "",
  defaultExpanded = true,
  testId = "auth-gate",
  challengeKind = "",
  children = null,
}) {
  const t = useT();
  const [expanded, setExpanded] = React.useState(defaultExpanded);
  const controlsId = React.useId();
  const subtitle = accountLabel || provider || "";

  return (
    <div
      data-testid={testId}
      data-auth-challenge={challengeKind || undefined}
      className="mx-auto w-full max-w-lg rounded-xl border border-[var(--v2-accent)]/35 bg-[var(--v2-accent)]/10"
    >
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        aria-expanded={expanded ? "true" : "false"}
        aria-controls={controlsId}
        className="flex w-full items-center gap-3 rounded-xl border-0 bg-transparent px-4 py-3 text-left"
      >
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-md border border-[var(--v2-accent)]/30 bg-[var(--v2-accent)]/10 text-[var(--v2-accent-text)]">
          <Icon name={icon} className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium text-[var(--v2-text-strong)]">
            {headline || t("authGate.title")}
          </span>
          {subtitle &&
          (<span className="block truncate text-xs text-[var(--v2-text-muted)]">{subtitle}</span>)}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs font-medium text-[var(--v2-accent-text)]">
          {pillHint && (<span className="hidden sm:inline">{pillHint}</span>)}
          <Icon
            name="chevron"
            className={["h-4 w-4", expanded ? "rotate-180" : ""].join(" ")}
          />
        </span>
      </button>

      {expanded &&
      (
        <div
          id={controlsId}
          className="border-t border-[var(--v2-accent)]/20 px-4 pb-4 pt-3"
        >
          {body &&
          (<div className="mb-3 text-sm text-[var(--v2-text)]">{body}</div>)}
          {children}
          {expiresAt &&
          (
            <p className="mt-2 text-xs text-[var(--v2-text-muted)]">
              {t("authGate.expiresAt")}: {new Date(expiresAt).toLocaleString()}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
