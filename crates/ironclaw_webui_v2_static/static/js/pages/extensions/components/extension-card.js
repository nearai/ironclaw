import { React, html } from "../../../lib/html.js";
import { Badge } from "../../../design-system/badge.js";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { KIND_LABELS, STATE_TONES, STATE_LABELS } from "../lib/extensions-schema.js";

export function ExtensionCard({ ext, onActivate, onConfigure, onRemove, isBusy }) {
  const state    = ext.onboarding_state || ext.activation_status || (ext.active ? "active" : "installed");
  const tone     = STATE_TONES[state] || "muted";
  const label    = STATE_LABELS[state] || state;
  const kindLabel = KIND_LABELS[ext.kind] || ext.kind;

  return html`
    <div
      className="flex flex-col gap-3 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text-strong)]">
              ${ext.display_name || ext.name}
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
        </div>

        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
          ${state !== "active" && state !== "ready" && ext.kind !== "wasm_channel" && html`
            <${Button}
              variant="secondary"
              size="sm"
              onClick=${() => onActivate({ name: ext.name })}
              disabled=${isBusy}
            >Activate<//>
          `}
          ${(ext.needs_setup || ext.has_auth) && html`
            <${Button}
              variant="ghost"
              size="sm"
              onClick=${() => onConfigure(ext.name)}
              disabled=${isBusy}
            >${ext.authenticated ? "Reconfigure" : "Configure"}<//>
          `}
          ${ext.kind === "wasm_channel" && (state === "setup_required" || state === "failed") && html`
            <${Button}
              variant="secondary"
              size="sm"
              onClick=${() => onConfigure(ext.name)}
              disabled=${isBusy}
            >Setup<//>
          `}
          ${ext.kind === "wasm_channel" &&
            (state === "active" || state === "ready" || state === "pairing_required" || state === "pairing") && html`
            <${Button}
              variant="ghost"
              size="sm"
              onClick=${() => onConfigure(ext.name)}
              disabled=${isBusy}
            >Reconfigure<//>
          `}
          <${Button}
            variant="danger"
            size="sm"
            onClick=${() => onRemove({ name: ext.name })}
            disabled=${isBusy}
          >Remove<//>
        </div>
      </div>
    </div>
  `;
}

export function RegistryCard({ entry, onInstall, isBusy }) {
  const kindLabel = KIND_LABELS[entry.kind] || entry.kind;

  return html`
    <div
      className="flex flex-col gap-3 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-[var(--v2-text-strong)]">
              ${entry.display_name || entry.name}
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
          <${Button}
            variant="outline"
            size="sm"
            onClick=${() => onInstall({ name: entry.name, kind: entry.kind, displayName: entry.display_name })}
            disabled=${isBusy}
          >
            <${Icon} name="plus" className="mr-1.5 h-3.5 w-3.5" />
            Install
          <//>
        </div>
      </div>
    </div>
  `;
}
