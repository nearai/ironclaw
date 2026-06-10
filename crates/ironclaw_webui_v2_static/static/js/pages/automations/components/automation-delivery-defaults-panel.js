import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { Panel, StatusPill } from "../../../design-system/primitives.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";

function statusTone(status) {
  if (status === "available") return "success";
  if (status === "unavailable") return "warning";
  return "muted";
}

function targetLabel(option) {
  return option?.target?.display_name || option?.target?.target_id || "";
}

export function AutomationDeliveryDefaultsPanel({ deliveryState }) {
  const t = useT();
  const currentTargetId = deliveryState.currentTarget?.target_id || "";
  const [draftTargetId, setDraftTargetId] = React.useState(currentTargetId);

  React.useEffect(() => {
    setDraftTargetId(currentTargetId);
  }, [currentTargetId]);

  const isDirty = draftTargetId !== currentTargetId;
  const isBusy = deliveryState.isLoading || deliveryState.isSaving;
  const canSave = isDirty && !isBusy;
  const canClear = Boolean(currentTargetId || draftTargetId) && !isBusy;
  const error = deliveryState.error || deliveryState.saveError;
  const currentLabel =
    deliveryState.currentTarget?.display_name ||
    t("automations.delivery.none");
  const selectedOption = deliveryState.finalReplyTargets.find(
    (option) => option?.target?.target_id === draftTargetId
  );

  const handleSave = () => {
    if (!canSave) return;
    deliveryState.saveFinalReplyTarget(draftTargetId || null).catch(() => {});
  };

  const handleClear = () => {
    if (!canClear) return;
    setDraftTargetId("");
    deliveryState.saveFinalReplyTarget(null).catch(() => {});
  };

  return html`
    <${Panel} className="p-4 sm:p-5">
      <div className="flex flex-col gap-4 xl:flex-row xl:items-end xl:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-3">
            <div
              className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300"
            >
              ${t("automations.delivery.eyebrow")}
            </div>
            <${StatusPill}
              tone=${statusTone(deliveryState.currentStatus)}
              label=${t(`automations.delivery.status.${deliveryState.currentStatus}`)}
            />
          </div>
          <div className="mt-3 text-lg font-semibold text-iron-100">
            ${currentLabel}
          </div>
          <div className="mt-1 max-w-2xl text-sm leading-6 text-iron-300">
            ${deliveryState.currentTarget?.description ||
            t("automations.delivery.noneDescription")}
          </div>
        </div>

        <div className="grid gap-3 sm:grid-cols-[minmax(14rem,1fr)_auto_auto] sm:items-end">
          <label className="min-w-0">
            <span
              className="mb-1.5 block font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400"
            >
              ${t("automations.delivery.targetLabel")}
            </span>
            <select
              value=${draftTargetId}
              disabled=${isBusy || deliveryState.finalReplyTargets.length === 0}
              onChange=${(event) => setDraftTargetId(event.target.value)}
              className=${cn(
                "h-10 w-full rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 text-sm text-iron-100",
                "focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",
                "disabled:cursor-not-allowed disabled:opacity-60"
              )}
            >
              <option value="">
                ${t("automations.delivery.noTargetOption")}
              </option>
              ${deliveryState.finalReplyTargets.map(
                (option) => html`
                  <option
                    key=${option.target.target_id}
                    value=${option.target.target_id}
                  >
                    ${targetLabel(option)}
                  </option>
                `
              )}
            </select>
          </label>

          <${Button}
            variant="primary"
            size="sm"
            disabled=${!canSave}
            onClick=${handleSave}
          >
            <${Icon} name=${deliveryState.isSaving ? "retry" : "check"} className="h-4 w-4" />
            ${deliveryState.isSaving
              ? t("automations.delivery.saving")
              : t("automations.delivery.save")}
          <//>

          <${Button}
            variant="secondary"
            size="sm"
            disabled=${!canClear}
            onClick=${handleClear}
          >
            ${t("automations.delivery.clear")}
          <//>
        </div>
      </div>

      ${(selectedOption?.target?.description || error) &&
      html`
        <div className="mt-3 text-xs leading-5 text-iron-400">
          ${error
            ? t("automations.delivery.error")
            : selectedOption.target.description}
        </div>
      `}
    <//>
  `;
}
