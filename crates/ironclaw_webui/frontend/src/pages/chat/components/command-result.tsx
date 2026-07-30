import React from "react";
import { useT } from "../../../lib/i18n";
import { Icon } from "../../../design-system/icons";
import { toast } from "../../../lib/toast";
import { formatThreadActivityTooltip } from "../../../lib/thread-meta";
import { COMMAND_RESULT_KIND, classifyCommandResponse } from "../lib/chat-commands";

/* Command-result presentation — the sibling of the ⌘K palette and the
   composer's command menu (command-palette.tsx / chat-input.tsx's dropdown),
   not the old amber "system notice" bubble. Design:
   - `result` renders as a proper card: a heading, `fields` as left-aligned
     definition rows, `lines` as a compact, scrollable list.
   - The "available commands" rejection renders the SERVER inventory
     (`useChatCommands()`, threaded down as `commands`) as rows that echo the
     dropdown's own `/name` + title + description layout, falling back to the
     backend's plain-text help blob only when the inventory hasn't loaded.
   - Every other rejection (e.g. "requires an admin account") renders as a
     calm, low-alarm inline notice — never the amber blob.
   Every token here is an existing `--v2-*` CSS variable, so light/dark
   theming is automatic (see design-system/card.tsx's own comment). */

// ISO 8601 timestamp shape the backend actually emits for time-valued fields
// (e.g. `/status`'s "Since": `DateTime::to_rfc3339_opts(SecondsFormat::Secs,
// true)` — always `Z`-suffixed UTC, never a bare offset-less string). Lives
// here (not chat-commands.ts) because this presentation layer is its only
// caller — see the comment on that module.
const ISO_TIMESTAMP_RE =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})$/;

// Whether a field VALUE (never the label) is an ISO timestamp that should
// render through the app's existing human-readable date formatting instead
// of as raw text.
export function isIsoTimestampValue(value) {
  if (typeof value !== "string") return false;
  const trimmed = value.trim();
  if (!ISO_TIMESTAMP_RE.test(trimmed)) return false;
  return Number.isFinite(Date.parse(trimmed));
}

const MIN_IDENTIFIER_LENGTH = 8;
// Opaque-token charset: letters, digits, and the punctuation that shows up
// in run ids (UUIDs), package ids (dotted/slashed/hyphenated names), and
// hashes. Deliberately excludes "_" — backend `State` values are snake_case
// WORDS (e.g. `setup_needed`, `LifecyclePublicState::as_str()`), not opaque
// identifiers, and must keep rendering as plain prose, not monospace.
const IDENTIFIER_CHARSET_RE = /^[A-Za-z0-9](?:[A-Za-z0-9.:@/-]*[A-Za-z0-9])?$/;

// Whether a field VALUE looks like an identifier (run id, package id, hash)
// rather than prose — a value-shape heuristic, not a label allowlist, so it
// stays correct for any command's fields without hardcoding that command's
// name or field labels here. Short plain words ("idle", "yes") and
// underscored state labels ("setup_needed") are deliberately excluded; see
// the constants above for why. This means a short human-chosen slug (e.g. a
// 5-letter package id) will not get the identifier treatment — an accepted
// trade-off given a value-only heuristic cannot otherwise distinguish it from
// an ordinary short word (see the design report).
export function isIdentifierValue(value) {
  if (typeof value !== "string") return false;
  const trimmed = value.trim();
  if (trimmed.length < MIN_IDENTIFIER_LENGTH) return false;
  if (/\s/.test(trimmed)) return false;
  if (!IDENTIFIER_CHARSET_RE.test(trimmed)) return false;
  if (!/[A-Za-z]/.test(trimmed)) return false;
  return /[0-9._:@/-]/.test(trimmed);
}

function CopyValueButton({ value }) {
  const t = useT();
  const [copied, setCopied] = React.useState(false);
  const timerRef = React.useRef(null);
  React.useEffect(() => () => window.clearTimeout(timerRef.current), []);

  const onCopy = React.useCallback(async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      toast(t("common.copiedToClipboard"), { tone: "success" });
      window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => setCopied(false), 1400);
    } catch {
      // clipboard unavailable — no-op, matching message-bubble.tsx's copy affordance
    }
  }, [value, t]);

  return (
    <button
      type="button"
      onClick={onCopy}
      title={copied ? t("common.copied") : t("common.copy")}
      aria-label={copied ? t("common.copied") : t("common.copy")}
      className="v2-button inline-grid h-5 w-5 shrink-0 place-items-center rounded text-[var(--v2-text-faint)] hover:text-[var(--v2-text-strong)]"
    >
      <Icon name={copied ? "check" : "copy"} className="h-3 w-3" />
    </button>
  );
}

function FieldValue({ value }) {
  if (isIsoTimestampValue(value)) {
    return (
      <time dateTime={value} className="text-[var(--v2-text-strong)]">
        {formatThreadActivityTooltip(value)}
      </time>
    );
  }
  if (isIdentifierValue(value)) {
    return (
      <span className="flex min-w-0 items-center gap-1.5">
        <span
          title={value}
          className="min-w-0 truncate font-mono text-[13px] text-[var(--v2-text-strong)]"
        >
          {value}
        </span>
        <CopyValueButton value={value} />
      </span>
    );
  }
  return <span className="text-left text-[var(--v2-text-strong)]">{value}</span>;
}

function ResultFields({ fields }) {
  if (!fields || fields.length === 0) return null;
  return (
    <dl className="divide-y divide-[var(--v2-panel-border)]">
      {fields.map((field) => (
        <div
          key={field.label}
          className="grid grid-cols-[6.5rem_1fr] items-start gap-3 px-4 py-2 text-sm"
        >
          <dt className="text-left text-[var(--v2-text-muted)]">{field.label}</dt>
          <dd className="min-w-0 text-left">
            <FieldValue value={field.value} />
          </dd>
        </div>
      ))}
    </dl>
  );
}

// Compact, bounded-height rows for backend prose lines (e.g. /extension_list
// with a dozen rows) — never re-parsed/re-formatted, just left-aligned and
// scrollable instead of a wall of centered text.
function ResultLines({ lines }) {
  if (!lines || lines.length === 0) return null;
  return (
    <ul
      role="list"
      className="max-h-64 list-none divide-y divide-[var(--v2-panel-border)] overflow-y-auto px-4 py-1"
    >
      {lines.map((line, index) => (
        <li
          key={index}
          className="whitespace-pre-wrap break-words py-1.5 text-left text-sm leading-6 text-[var(--v2-text)]"
        >
          {line}
        </li>
      ))}
    </ul>
  );
}

function CommandResultShell({ children }) {
  return (
    <div
      data-testid="command-result"
      className="mx-auto w-full max-w-lg overflow-hidden rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] text-left shadow-[0_20px_45px_-24px_rgba(0,0,0,0.6)]"
    >
      {children}
    </div>
  );
}

function CommandResultHeader({ title, badge = null }) {
  return (
    <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-4 py-2.5">
      <Icon name="terminal" className="h-3.5 w-3.5 shrink-0 text-[var(--v2-text-faint)]" />
      <h3 className="min-w-0 flex-1 truncate text-left text-sm font-semibold text-[var(--v2-text-strong)]">
        {title}
      </h3>
      {badge != null && (
        <span
          aria-hidden="true"
          className="ml-auto shrink-0 rounded-full bg-[var(--v2-surface-soft)] px-2 py-0.5 font-mono text-[10px] text-[var(--v2-text-faint)]"
        >
          {badge}
        </span>
      )}
    </div>
  );
}

function CommandSuccessResult({ result }) {
  return (
    <CommandResultShell>
      <CommandResultHeader title={result.title} />
      <ResultFields fields={result.fields} />
      <ResultLines lines={result.lines} />
    </CommandResultShell>
  );
}

// Shared shell for both a genuine denial and the "available commands" help
// text when the inventory hasn't loaded — a calm inline notice (role=status,
// muted tones), not a card and not the old amber centered blob.
function CommandNotice({ icon, message, testId }) {
  return (
    <div
      data-testid={testId}
      role="status"
      className="mx-auto flex w-full max-w-lg items-start gap-2.5 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-left text-sm leading-6 text-[var(--v2-text-muted)]"
    >
      <Icon name={icon} className="mt-0.5 h-4 w-4 shrink-0 text-[var(--v2-text-faint)]" />
      <span className="whitespace-pre-wrap break-words">{message}</span>
    </div>
  );
}

function CommandListRows({ commands }) {
  return (
    <ul role="list" className="max-h-72 overflow-y-auto p-1.5">
      {commands.map((command) => (
        <li key={command.name}>
          <div className="flex w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-left">
            <span className="shrink-0 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-xs leading-4 text-[var(--v2-text-strong)]">
              /{command.name}
            </span>
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate text-sm font-medium text-[var(--v2-text-strong)]">
                {command.title}
              </span>
              {command.description && (
                <span className="truncate text-xs text-[var(--v2-text-muted)]">
                  {command.description}
                </span>
              )}
            </span>
          </div>
        </li>
      ))}
    </ul>
  );
}

// The unknown/undeclared-command rejection: prefer the live server inventory
// (rendered as dropdown-echoing rows) over reformatting the backend's plain
// help-text blob. The blob remains the fallback when the inventory hasn't
// loaded yet (composer mounted before `useChatCommands()` resolved, or the
// fetch failed) — see useChatCommands.ts.
function CommandListResult({ rejection, commands }) {
  const t = useT();
  if (!commands || commands.length === 0) {
    return (
      <CommandNotice
        icon="list"
        message={rejection.message}
        testId="command-result-list-fallback"
      />
    );
  }
  return (
    <CommandResultShell>
      <CommandResultHeader title={t("chat.commandListTitle")} badge={commands.length} />
      <CommandListRows commands={commands} />
    </CommandResultShell>
  );
}

export function CommandResult({ response, commands = [] }) {
  const kind = classifyCommandResponse(response);
  if (kind === COMMAND_RESULT_KIND.SUCCESS) {
    return <CommandSuccessResult result={response.result} />;
  }
  if (kind === COMMAND_RESULT_KIND.COMMAND_LIST) {
    return <CommandListResult rejection={response.rejection} commands={commands} />;
  }
  if (kind === COMMAND_RESULT_KIND.DENIAL) {
    return (
      <CommandNotice
        icon="lock"
        message={response.rejection.message}
        testId="command-result-denial"
      />
    );
  }
  return null;
}
