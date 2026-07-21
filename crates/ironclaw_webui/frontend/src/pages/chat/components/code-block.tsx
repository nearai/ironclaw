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
    <div className="group relative my-3 overflow-hidden rounded-lg border border-[var(--v2-panel-border)] bg-[var(--v2-code-bg)]">
      <div className="flex items-center justify-between border-b border-[var(--v2-panel-border)] px-3 py-1.5">
        <span className="font-mono text-[11px] text-[var(--v2-text)]">{language || "text"}</span>
        <button
          onClick={handleCopy}
          className="rounded px-2 py-0.5 text-[11px] text-[var(--v2-text)] opacity-0 hover:bg-[var(--v2-surface-muted)] group-hover:opacity-100"
        >
          {t("common.copy")}
        </button>
      </div>
      <pre className="overflow-x-auto p-3 text-sm"><code className="font-mono text-[var(--v2-text-strong)]">{code}</code></pre>
    </div>
  );
}
