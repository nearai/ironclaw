import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Badge } from "../../../design-system/badge.js";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import {
  KIND_LABELS,
  STATE_TONES,
  STATE_LABELS,
  isChannelExtensionKind,
} from "../lib/extensions-schema.js";
import { primaryExtensionAction } from "../lib/extension-actions.js";

/* Card layout (Option B): self-contained bordered card. Capabilities collapse
   behind a count disclosure; secondary actions (Configure / Setup / Remove)
   live in an overflow menu so the resting card stays calm. */

const CARD =
  "flex self-start flex-col rounded-[14px] border border-[var(--v2-panel-border)] " +
  "bg-[var(--v2-surface-soft)] p-4";
const META = "mt-1.5 flex flex-wrap items-center gap-x-2 font-mono text-[10px] text-[var(--v2-text-faint)]";
const DESC = "mt-2 line-clamp-2 min-h-[2.5rem] text-xs leading-5 text-[var(--v2-text-muted)]";
const FOOTER = "mt-3 flex items-center gap-2 border-t border-[var(--v2-panel-border)] pt-3";
const DISCLOSURE =
  "v2-button inline-flex items-center gap-1.5 border-0 bg-transparent p-0 " +
  "font-mono text-[11px] text-[var(--v2-text-faint)] hover:text-[var(--v2-accent-text)]";
const CHIP =
  "rounded border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] " +
  "px-1.5 py-0.5 font-mono text-[10px] text-[var(--v2-text-muted)]";

function packageId(item) {
  return item.package_ref?.id || "";
}

/* Lightweight overflow menu. Real <button>s; closes on outside click. */
function OverflowMenu({ actions, isBusy }) {
  const t = useT();
  const [open, setOpen] = React.useState(false);
  const ref = React.useRef(null);

  React.useEffect(() => {
    if (!open) return undefined;
    const onDoc = (event) => {
      if (ref.current && !ref.current.contains(event.target)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  return html`
    <div ref=${ref} className="relative shrink-0">
      <button
        type="button"
        aria-label=${t("extensions.moreActions")}
        aria-haspopup="true"
        aria-expanded=${open ? "true" : "false"}
        disabled=${isBusy}
        onClick=${() => setOpen((v) => !v)}
        className="grid h-7 w-7 place-items-center rounded-md border border-transparent text-[var(--v2-text-faint)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] disabled:cursor-not-allowed disabled:opacity-50"
      >
        <${Icon} name="more" className="h-4 w-4" strokeWidth=${2.4} />
      </button>
      ${open &&
      html`
        <div
          role="menu"
          className="absolute right-0 top-8 z-10 min-w-[156px] rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-1 shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]"
        >
          ${actions.map(
            (action) => html`
              <button
                key=${action.id}
                type="button"
                role="menuitem"
                disabled=${isBusy}
                onClick=${() => {
                  setOpen(false);
                  action.run();
                }}
                className=${[
                  "flex w-full items-center gap-2.5 rounded-[7px] px-2.5 py-1.5 text-left text-[13px] disabled:cursor-not-allowed disabled:opacity-50",
                  action.danger
                    ? "text-[var(--v2-danger-text)] hover:bg-[var(--v2-danger-soft)]"
                    : "text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]",
                ].join(" ")}
              >
                <${Icon} name=${action.icon || "settings"} className="h-3.5 w-3.5" />
                ${action.label}
              </button>
            `
          )}
        </div>
      `}
    </div>
  `;
}

function ChipGrid({ items }) {
  if (!items || items.length === 0) return null;
  return html`
    <div className="mt-3 flex flex-wrap gap-1">
      ${items.map((item) => html`<span key=${item} className=${CHIP}>${item}</span>`)}
    </div>
  `;
}

export function ExtensionCard({ ext, onActivate, onConfigure, onRemove, isBusy }) {
  const t = useT();
  const state = ext.onboarding_state || ext.activation_status || (ext.active ? "active" : "installed");
  const tone = STATE_TONES[state] || "muted";
  const label = t(`extensions.state.${state}`) || STATE_LABELS[state] || state;
  const kindLabel = t(`extensions.kind.${ext.kind}`) || KIND_LABELS[ext.kind] || ext.kind;
  const displayName = ext.display_name || packageId(ext);
  const canManage = Boolean(ext.package_ref);
  const tools = ext.tools || [];
  const [capsOpen, setCapsOpen] = React.useState(false);

  const setupState = state === "setup_required" || state === "auth_required";
  const onboardingHint =
    (setupState
      ? ext.onboarding?.credential_instructions || ext.onboarding?.credential_next_step
      : ext.onboarding?.credential_next_step || ext.onboarding?.credential_instructions) ||
    null;

  const configurePayload = {
    packageRef: ext.package_ref,
    displayName,
    kind: ext.kind,
    active: ext.active,
    authenticated: ext.authenticated,
    needs_setup: ext.needs_setup,
    activationStatus: ext.activation_status,
    onboardingState: ext.onboarding_state,
  };

  // Connectable channels are configured by pairing (Connect/Reconnect), not by
  // an operator credential form (Configure/Reconfigure). Pick the label by kind.
  const configureLabel = isChannelExtensionKind(ext.kind)
    ? ext.authenticated
      ? t("extensions.reconnect")
      : t("extensions.connect")
    : ext.authenticated
      ? t("extensions.reconfigure")
      : t("extensions.configure");

  const primaryActions = [];
  const overflowActions = [];
  const primaryAction = primaryExtensionAction(ext);

  if (primaryAction === "configure") {
    primaryActions.push({
      id: "configure",
      label: configureLabel,
      run: () => onConfigure(configurePayload),
    });
  } else if (primaryAction === "activate") {
    primaryActions.push({
      id: "activate",
      label: t("extensions.activate"),
      run: () => onActivate(configurePayload),
    });
  }
  if (canManage && (ext.needs_setup || ext.has_auth) && primaryAction !== "configure") {
    overflowActions.push({
      id: "configure",
      label: configureLabel,
      icon: "settings",
      run: () => onConfigure(configurePayload),
    });
  }
  const hasOverflowConfigureAction = overflowActions.some(
    (action) => action.id === "configure"
  );
  if (
    canManage &&
    primaryAction !== "configure" &&
    isChannelExtensionKind(ext.kind) &&
    (state === "setup_required" || state === "failed")
  ) {
    overflowActions.push({
      id: "setup",
      label: t("extensions.setup"),
      icon: "settings",
      run: () => onConfigure(configurePayload),
    });
  }
  if (
    canManage &&
    isChannelExtensionKind(ext.kind) &&
    !hasOverflowConfigureAction &&
    (state === "active" || state === "ready" || state === "pairing_required" || state === "pairing")
  ) {
    overflowActions.push({
      id: "reconfigure",
      label: configureLabel,
      icon: "settings",
      run: () => onConfigure(configurePayload),
    });
  }
  if (canManage) {
    overflowActions.push({
      id: "remove",
      label: t("common.remove"),
      icon: "trash",
      danger: true,
      run: () => onRemove(configurePayload),
    });
  }

  const primary = primaryActions[0];

  return html`
    <div className=${CARD}>
      <div className="flex items-start gap-2">
        <${Badge} tone=${tone} label=${label} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${displayName}
        </span>
        ${overflowActions.length > 0 &&
        html`<${OverflowMenu} actions=${overflowActions} isBusy=${isBusy} />`}
      </div>

      <div className=${META}>
        <span>${kindLabel}</span>
        ${ext.version && html`<span>· v${ext.version}</span>`}
      </div>

      ${ext.description && html`<p className=${DESC}>${ext.description}</p>`}

      ${ext.activation_error &&
      html`
        <div
          className="mt-2 rounded-[10px] border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-1.5 text-xs text-[var(--v2-danger-text)]"
        >
          ${ext.activation_error}
        </div>
      `}

      ${onboardingHint &&
      html`
        <div className="mt-2 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2 text-xs leading-5 text-[var(--v2-text-muted)]">
          ${onboardingHint}
        </div>
      `}

      <div className=${FOOTER}>
        ${tools.length > 0
          ? html`
              <button
                type="button"
                aria-expanded=${capsOpen ? "true" : "false"}
                onClick=${() => setCapsOpen((v) => !v)}
                className=${DISCLOSURE}
              >
                <${Icon} name="layers" className="h-3.5 w-3.5" />
                <span>${tools.length === 1 ? t("extensions.oneCapability") : t("extensions.pluralCapabilities", {count: tools.length})}</span>
                <${Icon}
                  name="chevron"
                  className=${["h-3 w-3", capsOpen ? "rotate-180" : ""].join(" ")}
                />
              </button>
            `
          : html`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${t("extensions.noCapabilities")}</span>`}
        <span className="flex-1"></span>
        ${primary &&
        html`
          <${Button} variant="secondary" size="sm" onClick=${primary.run} disabled=${isBusy}>
            ${primary.label}
          <//>
        `}
      </div>

      ${capsOpen && html`<${ChipGrid} items=${tools} />`}
    </div>
  `;
}

export function RegistryCard({ entry, onInstall, isBusy, statusLabel }) {
  const t = useT();
  const kindLabel = t(`extensions.kind.${entry.kind}`) || KIND_LABELS[entry.kind] || entry.kind;
  const displayName = entry.display_name || packageId(entry);
  const canInstall = Boolean(entry.package_ref && onInstall);
  const configureAfterInstall = Boolean(
    entry.needs_setup || entry.has_auth || isChannelExtensionKind(entry.kind)
  );
  const keywords = entry.keywords || [];
  const [kwOpen, setKwOpen] = React.useState(false);

  return html`
    <div className=${CARD}>
      <div className="flex items-start gap-2">
        <${Badge}
          tone="muted"
          label=${statusLabel || t("extensions.state.available") || "available"}
          size="sm"
        />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
          ${displayName}
        </span>
      </div>

      <div className=${META}>
        <span>${kindLabel}</span>
        ${entry.version && html`<span>· v${entry.version}</span>`}
      </div>

      ${entry.description && html`<p className=${DESC}>${entry.description}</p>`}

      <div className=${FOOTER}>
        ${keywords.length > 0
          ? html`
              <button
                type="button"
                aria-expanded=${kwOpen ? "true" : "false"}
                onClick=${() => setKwOpen((v) => !v)}
                className=${DISCLOSURE}
              >
                <${Icon} name="list" className="h-3.5 w-3.5" />
                <span>${keywords.length === 1 ? t("extensions.oneKeyword") : t("extensions.pluralKeywords", {count: keywords.length})}</span>
                <${Icon}
                  name="chevron"
                  className=${["h-3 w-3", kwOpen ? "rotate-180" : ""].join(" ")}
                />
              </button>
            `
          : html`<span className="font-mono text-[11px] text-[var(--v2-text-faint)]"></span>`}
        <span className="flex-1"></span>
        ${canInstall &&
        html`
          <${Button}
            variant="outline"
            size="sm"
            onClick=${() =>
              onInstall({
                packageRef: entry.package_ref,
                displayName,
                kind: entry.kind,
                configureAfterInstall,
              })}
            disabled=${isBusy}
          >
            <${Icon} name="plus" className="mr-1.5 h-3.5 w-3.5" />
            ${t("extensions.install")}
          <//>
        `}
      </div>

      ${kwOpen && html`<${ChipGrid} items=${keywords} />`}
    </div>
  `;
}
