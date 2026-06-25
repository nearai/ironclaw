/**
 * ApprovalCard v2 — tool-approval gate.
 *
 * Adds a heuristic risk badge derived client-side from the tool name (the
 * backend does not currently send a risk classification), a cleaner parameter
 * block, and a clearer "always allow" affordance. When the operator ticks
 * "always allow", the primary action calls `onAlways` instead of `onApprove`.
 *
 * No fabricated diff or scope claims: parameters are rendered as supplied,
 * and "always allow" wording stays generic because the backend owns the
 * actual persistence scope.
 */
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { Badge } from "../../../design-system/badge.js";
import { Icon } from "../../../design-system/icons.js";
import { classifyRisk } from "../lib/approval-risk.js";

const APPROVAL_PAYLOAD_PREVIEW_LIMIT = 480;

function approvalPayloadIsLong(parameters, approvalDetails) {
  if (approvalDetails && approvalDetails.length > 0) {
    return approvalDetails.some(
      (detail) =>
        typeof detail?.value === "string" && detail.value.length > APPROVAL_PAYLOAD_PREVIEW_LIMIT,
    );
  }
  return typeof parameters === "string" && parameters.length > APPROVAL_PAYLOAD_PREVIEW_LIMIT;
}

function approvalPayloadPreview(value, expanded) {
  if (typeof value !== "string") return value;
  if (expanded || value.length <= APPROVAL_PAYLOAD_PREVIEW_LIMIT) return value;
  return `${value.slice(0, APPROVAL_PAYLOAD_PREVIEW_LIMIT).trimEnd()}\n...`;
}

export function ApprovalCard({ gate, onApprove, onDeny, onAlways }) {
  const t = useT();
  const { toolName, description, parameters, allowAlways, approvalDetails = [] } = gate;
  const [always, setAlways] = React.useState(false);
  const [expandedPayload, setExpandedPayload] = React.useState(false);

  React.useEffect(() => {
    setExpandedPayload(false);
  }, [gate]);

  const risk = React.useMemo(
    () => classifyRisk(toolName, description, parameters),
    [toolName, description, parameters]
  );
  const toolLabel = toolName || t("approval.thisTool");
  const longPayload = approvalPayloadIsLong(parameters, approvalDetails);
  const payloadMaxHeight = expandedPayload ? "max-h-72" : "max-h-36";

  const onPrimary = React.useCallback(() => {
    if (always && allowAlways) {
      onAlways?.();
    } else {
      onApprove?.();
    }
  }, [always, allowAlways, onAlways, onApprove]);

  return html`
    <div
      data-testid="approval-card"
      className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4"
    >
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${Icon} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">${t("approval.title")}</span>
        <${Badge}
          tone=${risk.tone}
          label=${t(risk.key)}
          dot=${false}
          size="sm"
          className="ml-auto"
        />
      </div>
      ${toolName &&
      html`<div className="mb-1 break-all font-mono text-sm font-medium text-iron-100">${toolName}</div>`}
      ${description &&
      html`<div className="mb-3 break-words text-sm text-iron-200">${description}</div>`}
      ${approvalDetails.length > 0
        ? html`
            <dl className=${`mb-2 ${payloadMaxHeight} overflow-y-auto rounded-md border border-iron-800 bg-iron-950/80 text-xs`}>
              ${approvalDetails.map(
                (detail) => html`
                  <div className="grid gap-1 border-b border-iron-800/70 px-3 py-2 last:border-b-0 sm:grid-cols-[7rem_1fr]">
                    <dt className="font-medium text-iron-400">${detail.label}</dt>
                    <dd className="min-w-0 whitespace-pre-wrap break-all font-mono text-iron-100">${approvalPayloadPreview(detail.value, expandedPayload)}</dd>
                  </div>
                `,
              )}
            </dl>
          `
        : parameters &&
          html`<pre className=${`mb-2 ${payloadMaxHeight} overflow-auto whitespace-pre-wrap break-all rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100`}>${approvalPayloadPreview(parameters, expandedPayload)}</pre>`}

      ${longPayload &&
      html`
        <${Button}
          variant="ghost"
          size="sm"
          className="mb-3 px-0 text-[var(--v2-accent)] hover:bg-transparent"
          onClick=${() => setExpandedPayload((current) => !current)}
          type="button"
        >
          ${expandedPayload ? t("approval.showCommandPreview") : t("approval.viewFullCommand")}
        <//>
      `}

      ${allowAlways &&
      html`
        <label className="mb-3 flex items-center gap-2 text-xs text-iron-200">
          <input
            type="checkbox"
            checked=${always}
            onChange=${(event) => setAlways(event.currentTarget.checked)}
            className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
          />
          ${t("approval.alwaysAllowToolLabel", { tool: toolLabel })}
        </label>
      `}

      <div className="flex flex-wrap gap-2">
        <${Button} variant="primary" onClick=${onPrimary}>
          ${always && allowAlways ? t("approval.approveAndAlways") : t("approval.approve")}
        <//>
        <${Button} variant="secondary" onClick=${() => onDeny?.()}>
          ${t("approval.deny")}
        <//>
      </div>
    </div>
  `;
}
