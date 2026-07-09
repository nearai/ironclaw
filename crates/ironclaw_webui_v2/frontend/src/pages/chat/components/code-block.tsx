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
    <div className="group relative my-3 overflow-hidden rounded-lg border border-iron-700 bg-iron-950">
      <div className="flex items-center justify-between border-b border-iron-800 px-3 py-1.5">
        <span className="font-mono text-[11px] text-iron-200">{language || "text"}</span>
        <button
          onClick={handleCopy}
          className="rounded px-2 py-0.5 text-[11px] text-iron-200 opacity-0 hover:bg-white/10 group-hover:opacity-100"
        >
          {t("common.copy")}
        </button>
      </div>
      <pre className="overflow-x-auto p-3 text-sm"><code className="font-mono text-iron-100">{code}</code></pre>
    </div>
  );
}
