import React from "react";
import { useT } from "../../../lib/i18n";
import { Panel } from "../../../design-system/primitives";

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
    <div className="rounded-[20px] border border-white/10 bg-white/[0.03] p-4">
      <div className="mb-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{widget.manifest?.slot || t("projects.widgets.fallbackSlot")}</div>
        <div className="mt-1 text-lg font-semibold tracking-tight text-white">{widget.manifest?.name || widget.manifest?.id}</div>
      </div>
      {errorName !== undefined
        ? (<p className="rounded-xl border border-red-400/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{t("projects.widgets.mountFailed", { name: errorName || t("projects.widgets.fallbackSlot") })}</p>)
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
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">{t("projects.widgets.title")}</div>
        <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">{t("projects.widgets.instrumentation")}</h2>
      </div>
      <div className="grid gap-4 xl:grid-cols-2">
        {widgets.map((widget) => (<ProjectWidgetMount key={widget.manifest?.id} widget={widget} projectId={projectId} />))}
      </div>
    </Panel>
  );
}
