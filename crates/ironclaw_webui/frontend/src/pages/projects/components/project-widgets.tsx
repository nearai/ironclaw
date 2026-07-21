import React from "react";
import { useT } from "../../../lib/i18n";
import { Panel } from "@ironclaw/design-system";

declare global {
  interface Window {
    IronClaw?: {
      api?: unknown;
    };
  }
}

function ProjectWidgetMount({ widget, projectId }) {
  const t = useT();
  const containerRef = React.useRef(null);
  const [errorName, setErrorName] = React.useState(undefined);

  React.useEffect(() => {
    const container = containerRef.current;
    if (!container || !widget) return undefined;

    let styleEl = null;

    try {
      container.innerHTML = "";
      if (widget.css) {
        styleEl = document.createElement("style");
        styleEl.textContent = widget.css;
        document.head.appendChild(styleEl);
      }

      const api = window.IronClaw?.api || null;
      const mount = new Function("container", "api", "projectId", widget.js);
      mount(container, api, projectId);
      setErrorName(undefined);
    } catch (mountError) {
      console.error("[v2-projects] failed to mount widget", widget?.manifest?.id, mountError);
      setErrorName(widget?.manifest?.name || "");
    }

    return () => {
      container.innerHTML = "";
      if (styleEl) styleEl.remove();
    };
  }, [projectId, widget]);

  return (
    <div className="rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
      <div className="mb-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{widget.manifest?.slot || t("projects.widgets.fallbackSlot")}</div>
        <div className="mt-1 text-lg font-medium tracking-tight text-[var(--v2-text-strong)]">{widget.manifest?.name || widget.manifest?.id}</div>
      </div>
      {errorName !== undefined
        ? (<p className="rounded-xl border border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-2 text-sm text-[var(--v2-danger-text)]">{t("projects.widgets.mountFailed", { name: errorName || t("projects.widgets.fallbackSlot") })}</p>)
        : null}
      <div ref={containerRef} className={errorName !== undefined ? "hidden" : ""} />
    </div>
  );
}

export function ProjectWidgets({ widgets, projectId }) {
  const t = useT();
  if (!widgets?.length) return null;

  return (
    <Panel className="p-4 sm:p-5">
      <div className="mb-4">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{t("projects.widgets.title")}</div>
        <h2 className="mt-2 text-2xl font-medium tracking-tight text-[var(--v2-text-strong)]">{t("projects.widgets.instrumentation")}</h2>
      </div>
      <div className="grid gap-4 xl:grid-cols-2">
        {widgets.map((widget) => (<ProjectWidgetMount key={widget.manifest?.id} widget={widget} projectId={projectId} />))}
      </div>
    </Panel>
  );
}
