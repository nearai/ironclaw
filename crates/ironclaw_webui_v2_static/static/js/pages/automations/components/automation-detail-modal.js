import { useNavigate } from "react-router";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { Modal, ModalBody, ModalFooter } from "../../../design-system/modal.js";
import { StatusPill } from "../../../design-system/primitives.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";
import { AutomationDetailBody } from "./automation-detail-body.js";

// The automation id (a ULID) labelled and copyable, so it's obvious what the
// long token is and easy to grab for support/debugging.
function AutomationId({ id }) {
  const t = useT();
  const [copied, setCopied] = React.useState(false);
  const timerRef = React.useRef(null);
  React.useEffect(() => () => clearTimeout(timerRef.current), []);

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(id);
      setCopied(true);
      clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setCopied(false), 1500);
    } catch (_) {
      // Clipboard can be blocked (insecure context / denied permission).
    }
  };

  return html`
    <div className="mt-2 flex items-center gap-2">
      <span className="shrink-0 font-mono text-[10px] uppercase tracking-[0.16em] text-iron-500">
        ${t("automations.detail.idLabel")}
      </span>
      <span
        className="truncate font-mono text-[11px] tracking-[0.04em] text-iron-400"
        title=${id}
      >
        ${id}
      </span>
      <button
        type="button"
        onClick=${onCopy}
        aria-label=${copied ? t("automations.empty.copied") : t("automations.detail.copyId")}
        title=${copied ? t("automations.empty.copied") : t("automations.detail.copyId")}
        className=${cn(
          "inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-md border border-[var(--v2-panel-border)] text-iron-400",
          "hover:border-white/20 hover:text-iron-200",
          copied && "border-emerald-400/40 text-emerald-300"
        )}
      >
        <${Icon} name=${copied ? "check" : "copy"} className="h-3.5 w-3.5" />
      </button>
    </div>
  `;
}

// Per-automation read-only detail, shown as a modal over the list. This is the
// "quick look": schedule, success rate, current run, and recent-run history.
// For the persistent, full-logs view the footer pops out to
// `/automations/:automationId`.
export function AutomationDetailModal({
  automation,
  open,
  onClose,
  isMutating = false,
  onPauseAutomation,
  onResumeAutomation,
  onDeleteAutomation,
}) {
  const t = useT();
  const navigate = useNavigate();

  if (!automation) return null;

  const canResume = automation.state === "paused";
  const canPause = automation.state === "active" || automation.state === "scheduled";
  const actionTitle = `${
    canResume ? t("missions.action.resume") : t("missions.action.pause")
  }: ${automation.display_name}`;
  const handleAction = () => {
    if (canResume) onResumeAutomation?.(automation.automation_id);
    else if (canPause) onPauseAutomation?.(automation.automation_id);
  };
  const deleteTitle = `${t("common.delete")}: ${automation.display_name}`;
  const handleDelete = () => {
    if (window.confirm(deleteTitle)) {
      onDeleteAutomation?.(automation.automation_id);
      onClose?.();
    }
  };

  const openFullView = () => {
    onClose?.();
    navigate(`/automations/${encodeURIComponent(automation.automation_id)}`);
  };

  return html`
    <${Modal} open=${open} onClose=${onClose} size="lg">
      <div
        className="flex shrink-0 items-start justify-between gap-4 border-b border-[var(--v2-panel-border)] px-5 py-4 md:px-7 md:py-5"
      >
        <div className="min-w-0">
          <div className="flex items-center gap-3">
            <span
              className="grid h-11 w-11 shrink-0 place-items-center rounded-[12px] border border-[color-mix(in_srgb,var(--v2-accent)_25%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
            >
              <${Icon} name=${automation.icon} className="h-[1.4rem] w-[1.4rem]" />
            </span>
            <h2 className="truncate text-2xl font-semibold tracking-[-0.02em] text-iron-100">
              ${automation.display_name}
            </h2>
          </div>
          <${AutomationId} id=${automation.automation_id} />
        </div>
        <div className="flex shrink-0 items-center gap-3">
          <${StatusPill}
            tone=${automation.primary_status_tone}
            label=${automation.primary_status_label}
          />
          <button
            type="button"
            onClick=${onClose}
            aria-label=${t("nav.close")}
            className="grid h-8 w-8 shrink-0 place-items-center rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          >
            <${Icon} name="close" className="h-4 w-4" />
          </button>
        </div>
      </div>

      <${ModalBody}>
        <${AutomationDetailBody}
          automation=${automation}
          onOpenRun=${navigate}
          onOpenLogs=${navigate}
        />
      <//>

      <${ModalFooter}>
        <div className="flex w-full flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            ${(canPause || canResume) &&
            html`
              <${Button}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${isMutating}
                onClick=${handleAction}
              >
                <${Icon} name=${canResume ? "play" : "pause"} className="mr-1.5 h-4 w-4" />
                ${canResume ? t("missions.action.resume") : t("missions.action.pause")}
              <//>
            `}
            <${Button}
              type="button"
              variant="danger"
              size="sm"
              disabled=${isMutating}
              onClick=${handleDelete}
            >
              <${Icon} name="trash" className="mr-1.5 h-4 w-4" />
              ${t("common.delete")}
            <//>
          </div>
          <div className="flex items-center gap-2">
            <${Button} variant="secondary" size="sm" onClick=${onClose}>
              ${t("nav.close")}
            <//>
            <${Button} variant="primary" size="sm" className="text-white" onClick=${openFullView}>
              <${Icon} name="layers" className="mr-1.5 h-4 w-4" />
              ${t("automations.detail.openFullView")}
            <//>
          </div>
        </div>
      <//>
    <//>
  `;
}
