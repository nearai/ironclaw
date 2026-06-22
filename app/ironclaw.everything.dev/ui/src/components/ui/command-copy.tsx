import { Check, Copy } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

function CommandCopy({ command, label }: { command: string; label?: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    await navigator.clipboard.writeText(command);
    setCopied(true);
    toast.success(label ?? "Copied");
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="group flex items-center gap-2 rounded-lg border border-border bg-muted px-3.5 py-2.5 text-sm font-mono text-foreground">
      <span className="flex-1 truncate">{command}</span>
      <button
        type="button"
        onClick={handleCopy}
        className="shrink-0 text-muted-foreground hover:text-foreground transition-colors"
        title={label ?? "Copy command"}
      >
        {copied ? <Check size={14} /> : <Copy size={14} />}
      </button>
    </div>
  );
}

export { CommandCopy };
