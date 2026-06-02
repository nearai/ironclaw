import { React, html } from "../../../lib/html.js";
import { Badge } from "../../../design-system/badge.js";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { KIND_LABELS, STATE_TONES, STATE_LABELS } from "../lib/extensions-schema.js";

function packageId(item) {
  return item.package_ref?.id || "";
}

export function ExtensionCard({ ext, onActivate, onConfigure, onRemove, isBusy }) {
  const state    = ext.onboarding_state || ext.activation_status || (ext.active ? "active" : "installed");
  const tone     = STATE_TONES[state] || "muted";
  const label    = STATE_LABELS[state] || state;
  const kindLabel = KIND_LABELS[ext.kind] || ext.kind;
  const displayName = ext.display_name || packageId(ext);
  const canManage = Boolean(ext.package_ref);
  const setupState = state === "setup_required" || state === "auth_required";
  const onboardingHint =
    (setupState
      ? ext.onboarding?.credential_instructions || ext.onboarding?.credential_next_step
      : ext.onboarding?.credential_next_step || ext.onboarding?.credential_instructions) ||
    null;

  return html`
    <div
      className="flex flex-col gap-3 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text-strong)]">
              ${displayName}
            </span>
            <${Badge} tone=${tone} label=${label} size="sm" />
            <span
              className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]"
            >
              ${kindLabel}
            </span>
            ${ext.version && html`
              <span className="font-mono text-[10px] text-[var(--v2-text-muted)]">
                v${ext.version}
              </span>
            `}
          </div>
          ${ext.description && html`
            <div className="mt-1 text-xs leading-5 text-[var(--v2-text-muted)]">
              ${ext.description}
            </div>
          `}
          ${ext.tools && ext.tools.length > 0 && html`
            <div className="mt-2 flex flex-wrap gap-1">
              ${ext.tools.map((t) => html`
                <span
                  key=${t}
                  className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]"
                >
                  ${t}
                </span>
              `)}
            </div>
          `}
          ${ext.activation_error && html`
            <div
              className="mt-2 rounded-[10px] border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-1.5 text-xs text-[var(--v2-danger-text)]"
            >
              ${ext.activation_error}
            </div>
          `}
          ${onboardingHint && html`
            <div
              className="mt-2 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2 text-xs leading-5 text-[var(--v2-text-muted)]"
            >
              ${onboardingHint}
            </div>
          `}
        </div>

        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
          ${canManage && state !== "active" && state !== "ready" && ext.kind !== "wasm_channel" && html`
            <${Button}
              variant="secondary"
              size="sm"
              onClick=${() => onActivate({ packageRef: ext.package_ref, displayName })}
              disabled=${isBusy}
            >Activate<//>
          `}
          ${canManage && (ext.needs_setup || ext.has_auth) && html`
            <${Button}
              variant="ghost"
              size="sm"
              onClick=${() => onConfigure({ packageRef: ext.package_ref, displayName })}
              disabled=${isBusy}
            >${ext.authenticated ? "Reconfigure" : "Configure"}<//>
          `}
          ${canManage && ext.kind === "wasm_channel" && (state === "setup_required" || state === "failed") && html`
            <${Button}
              variant="secondary"
              size="sm"
              onClick=${() => onConfigure({ packageRef: ext.package_ref, displayName })}
              disabled=${isBusy}
            >Setup<//>
          `}
          ${canManage && ext.kind === "wasm_channel" &&
            (state === "active" || state === "ready" || state === "pairing_required" || state === "pairing") && html`
            <${Button}
              variant="ghost"
              size="sm"
              onClick=${() => onConfigure({ packageRef: ext.package_ref, displayName })}
              disabled=${isBusy}
            >Reconfigure<//>
          `}
          ${canManage && html`
            <${Button}
              variant="danger"
              size="sm"
              onClick=${() => onRemove({ packageRef: ext.package_ref, displayName })}
              disabled=${isBusy}
            >Remove<//>
          `}
        </div>
      </div>
    </div>
  `;
}

export function RegistryCard({ entry, onInstall, isBusy }) {
  const kindLabel = KIND_LABELS[entry.kind] || entry.kind;
  const displayName = entry.display_name || packageId(entry);
  const canInstall = Boolean(entry.package_ref);

  return html`
    <div
      className="flex flex-col gap-3 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text-strong)]">
              ${displayName}
            </span>
            <${Badge} tone="muted" label="available" size="sm" />
            <span
              className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]"
            >
              ${kindLabel}
            </span>
            ${entry.version && html`
              <span className="font-mono text-[10px] text-[var(--v2-text-muted)]">
                v${entry.version}
              </span>
            `}
          </div>
          ${entry.description && html`
            <div className="mt-1 text-xs leading-5 text-[var(--v2-text-muted)]">
              ${entry.description}
            </div>
          `}
          ${entry.keywords && entry.keywords.length > 0 && html`
            <div className="mt-2 flex flex-wrap gap-1">
              ${entry.keywords.map((kw) => html`
                <span
                  key=${kw}
                  className="rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]"
                >
                  ${kw}
                </span>
              `)}
            </div>
          `}
        </div>

        <div className="flex shrink-0 items-center justify-end">
          ${canInstall && html`
            <${Button}
              variant="outline"
              size="sm"
              onClick=${() => onInstall({ packageRef: entry.package_ref, displayName })}
              disabled=${isBusy}
            >
              <${Icon} name="plus" className="mr-1.5 h-3.5 w-3.5" />
              Install
            <//>
          `}
        </div>
      </div>
    </div>
  `;
}
