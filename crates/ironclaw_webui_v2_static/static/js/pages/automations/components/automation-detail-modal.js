import { useNavigate } from "react-router";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { Modal, ModalBody, ModalFooter } from "../../../design-system/modal.js";
import { StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { AutomationDetailBody } from "./automation-detail-body.js";

// Per-automation read-only detail, shown as a modal over the list. This is the
// "quick look": schedule, success rate, current run, and recent-run history.
// For the persistent, full-logs view the footer pops out to
// `/automations/:automationId`.
export function AutomationDetailModal({ automation, open, onClose }) {
  const t = useT();
  const navigate = useNavigate();

  if (!automation) return null;

  const statusTone = automation.has_running_run ? "info" : automation.state_tone;
  const statusLabel = automation.has_running_run
    ? t("automations.status.running")
    : automation.state_label;

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
          <div className="flex items-center gap-2">
            <span
              className="grid h-9 w-9 shrink-0 place-items-center rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-iron-200"
            >
              <${Icon} name=${automation.icon} className="h-4 w-4" />
            </span>
            <h2 className="truncate text-lg font-semibold tracking-[-0.02em] text-iron-100">
              ${automation.display_name}
            </h2>
          </div>
          <div className="mt-2 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-iron-400">
            ${automation.automation_id}
          </div>
        </div>
        <div className="flex items-center gap-3">
          <${StatusPill} tone=${statusTone} label=${statusLabel} />
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
        <${Button} variant="secondary" size="sm" onClick=${openFullView}>
          <${Icon} name="layers" className="mr-1.5 h-4 w-4" />
          ${t("automations.detail.openFullView")}
        <//>
        <${Button} variant="primary" size="sm" className="text-white" onClick=${onClose}>
          ${t("nav.close")}
        <//>
      <//>
    <//>
  `;
}
