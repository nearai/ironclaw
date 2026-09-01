import { useT } from "../../../lib/i18n";

export function CodeBlock({ code, language = "" }) {
  const t = useT();
  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(code);
    } catch {
      // ignore
    }
  };

  return (
    <div className="group relative my-3 overflow-hidden rounded-lg border border-[var(--v2-panel-border)] bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)]">
      <div className="flex items-center justify-between border-b border-iron-800 px-3 py-1.5">
        <span className="font-mono text-[11px] text-[var(--v2-text-strong)]">{language || "text"}</span>
        <button
          onClick={handleCopy}
          className="rounded px-2 py-0.5 text-[11px] text-[var(--v2-text-strong)] opacity-0 hover:bg-white/10 group-hover:opacity-100"
        >
          {t("common.copy")}
        </button>
      </div>
      <pre className="overflow-x-auto p-3 text-ui"><code className="font-mono text-[var(--v2-text-strong)]">{code}</code></pre>
    </div>
  );
}
