import React from "react";
import { useT } from "../../../lib/i18n";
import { Callout, Card, SectionHeader } from "@ironclaw/ui";

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
        ? (<Callout tone="danger">{t("projects.widgets.mountFailed", { name: errorName || t("projects.widgets.fallbackSlot") })}</Callout>)
        : null}
      <div ref={containerRef} className={errorName !== undefined ? "hidden" : ""} />
    </div>
  );
}

export function ProjectWidgets({ widgets, projectId }) {
  const t = useT();
  if (!widgets?.length) return null;

  return (
    <Card className="p-4 sm:p-5">
      <SectionHeader
        className="mb-4"
        eyebrow={t("projects.widgets.title")}
        title={t("projects.widgets.instrumentation")}
      />
      <div className="grid gap-4 xl:grid-cols-2">
        {widgets.map((widget) => (<ProjectWidgetMount key={widget.manifest?.id} widget={widget} projectId={projectId} />))}
      </div>
    </Card>
  );
}
