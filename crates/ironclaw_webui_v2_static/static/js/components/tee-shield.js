import { Button } from "../design-system/button.js";
import { Icon } from "../design-system/icons.js";
import { React, html } from "../lib/html.js";
import { useT } from "../lib/i18n.js";
import { cn } from "../utils/cn.js";
import { useTeeAttestation } from "../hooks/useTeeAttestation.js";

const SUMMARY_FIELDS = [
  ["image_digest", "tee.imageDigest"],
  ["tls_certificate_fingerprint", "tee.tlsFingerprint"],
  ["report_data", "tee.reportData"],
  ["vm_config", "tee.vmConfig"],
];

export function TeeShield() {
  const t = useT();
  const tee = useTeeAttestation();
  const [open, setOpen] = React.useState(false);

  const toggleOpen = React.useCallback(() => {
    setOpen((next) => {
      const nextOpen = !next;
      if (nextOpen) tee.loadReport();
      return nextOpen;
    });
  }, [tee]);

  const copy = React.useCallback(() => {
    tee.copyReport().catch(() => {});
  }, [tee]);

  if (!tee.available) return null;

  const rows = buildRows({ teeInfo: tee.teeInfo, report: tee.report, t });

  return html`
    <div className="relative">
      <button
        type="button"
        onClick=${toggleOpen}
        aria-expanded=${open}
        title=${t("tee.title")}
        className=${cn(
          "grid h-8 w-8 place-items-center rounded-[8px]",
          "border border-[color-mix(in_srgb,var(--v2-positive-text)_28%,transparent)]",
          "bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",
          "hover:border-[color-mix(in_srgb,var(--v2-positive-text)_52%,transparent)]"
        )}
      >
        <${Icon} name="shield" className="h-4 w-4" />
      </button>

      ${open &&
      html`
        <div
          className=${cn(
            "absolute right-0 top-full z-40 mt-2 w-[min(22rem,calc(100vw-2rem))]",
            "rounded-[14px] border border-[var(--v2-panel-border)]",
            "bg-[var(--v2-surface)] p-3 shadow-[0_18px_48px_rgba(0,0,0,0.35)]"
          )}
        >
          <div className="flex items-center gap-2">
            <span className="grid h-8 w-8 place-items-center rounded-[10px] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]">
              <${Icon} name="shield" className="h-4 w-4" />
            </span>
            <div className="min-w-0">
              <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                ${t("tee.title")}
              </div>
              <div className="text-xs text-[var(--v2-text-muted)]">
                ${t("tee.verified")}
              </div>
            </div>
          </div>

          <div className="mt-3 space-y-2">
            ${rows.map(
              (row) => html`
                <div className="rounded-[10px] bg-[var(--v2-surface-soft)] px-3 py-2">
                  <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-[var(--v2-text-faint)]">
                    ${row.label}
                  </div>
                  <div className="mt-1 break-all font-mono text-[11px] text-[var(--v2-text)]">
                    ${row.value}
                  </div>
                </div>
              `
            )}
            ${tee.reportLoading &&
            html`<div className="text-xs text-[var(--v2-text-muted)]">${t("tee.loading")}</div>`}
            ${tee.reportError &&
            html`<div className="text-xs text-[var(--v2-danger-text)]">${t("tee.loadFailed")}</div>`}
          </div>

          <div className="mt-3 flex justify-end">
            <${Button}
              type="button"
              variant="secondary"
              size="sm"
              disabled=${tee.reportLoading}
              onClick=${copy}
            >
              <${Icon} name="check" className="h-4 w-4" />
              ${tee.copied ? t("tee.copied") : t("tee.copyReport")}
            <//>
          </div>
        </div>
      `}
    </div>
  `;
}

function buildRows({ teeInfo, report, t }) {
  const source = { ...report, image_digest: teeInfo?.image_digest };
  return SUMMARY_FIELDS.map(([key, labelKey]) => ({
    label: t(labelKey),
    value: summarizeValue(source[key]) || t("common.unknown"),
  }));
}

function summarizeValue(value) {
  if (!value) return "";
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length > 72 ? `${text.slice(0, 72)}...` : text;
}
