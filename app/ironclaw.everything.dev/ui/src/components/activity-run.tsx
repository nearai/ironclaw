import type { ToolCallPart, ToolResultPart } from "@tanstack/ai";
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  FileText,
  Globe,
  Loader2,
  Monitor,
  Terminal,
  Wrench,
} from "lucide-react";
import { useState } from "react";
import { parseIronclawToolResultEnvelope } from "@/lib/ironclaw-message-parts";
import { cn } from "@/lib/utils";

type ToolItem = {
  call: ToolCallPart;
  result?: ToolResultPart;
};

const TOOL_RUN_COLLAPSE_AFTER = 2;

const DOT_STYLE: Record<string, string> = {
  running: "bg-blue-400 animate-pulse",
  success: "bg-[color:var(--near-green)]",
  error: "bg-destructive",
};

const STATUS_WORD: Record<string, string> = { success: "ok", error: "err", running: "run" };

function toolIcon(name: string) {
  const n = name.toLowerCase();
  if (/(grep|search|find|lookup|query|web|http|fetch|url)/.test(n)) return Globe;
  if (/(bash|shell|exec|run|command|terminal|spawn|code)/.test(n)) return Terminal;
  if (/(read|file|content|cat|view|open|glob|list|ls|tree|diff)/.test(n)) return FileText;
  if (/(browser|screenshot|page|click|navigate)/.test(n)) return Monitor;
  return Wrench;
}

function summarizeTools(tools: ToolItem[]): string {
  let files = 0;
  let searches = 0;
  let commands = 0;
  let others = 0;
  for (const t of tools) {
    const name = t.call.name.toLowerCase();
    if (/(grep|search|find|lookup|query)/.test(name)) searches++;
    else if (/(bash|shell|exec|run|command|terminal|spawn|process)/.test(name)) commands++;
    else if (
      /(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(name)
    )
      files++;
    else others++;
  }
  const segs: string[] = [];
  if (files) segs.push(`${files} ${files === 1 ? "file" : "files"}`);
  if (searches) segs.push(`${searches} ${searches === 1 ? "search" : "searches"}`);
  if (commands) segs.push(`${commands} ${commands === 1 ? "command" : "commands"}`);
  if (others) segs.push(`${others} ${others === 1 ? "other" : "others"}`);
  const text = segs.join(", ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function toolStatus(item: ToolItem): "running" | "success" | "error" {
  if (item.result?.state === "error") return "error";
  if (!item.result) return "running";
  const state = item.call.state;
  switch (state) {
    case "awaiting-input":
    case "input-streaming":
    case "approval-requested":
      return "running";
    case "input-complete":
    case "approval-responded":
    case "complete":
      return "success";
    default:
      return "running";
  }
}

function RichResult({ text }: { text: string }) {
  const value = text.trim();

  if (/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(value)) {
    return (
      <img
        src={value}
        alt="Tool result"
        className="max-h-48 rounded-lg border border-border object-contain"
      />
    );
  }

  let parsed: unknown;
  if ((value.startsWith("{") || value.startsWith("[")) && value.length < 200000) {
    try {
      parsed = JSON.parse(value);
    } catch {
      parsed = undefined;
    }
  }

  if (
    Array.isArray(parsed) &&
    parsed.length > 0 &&
    parsed.every(
      (r): r is Record<string, unknown> => r !== null && typeof r === "object" && !Array.isArray(r),
    )
  ) {
    const columns = Array.from(
      parsed.reduce((set: Set<string>, row) => {
        Object.keys(row).forEach((k) => set.add(k));
        return set;
      }, new Set<string>()),
    );
    return (
      <div className="overflow-x-auto rounded border border-border/60">
        <table className="w-full border-collapse text-left font-mono text-[11px]">
          <thead>
            <tr>
              {columns.map((col) => (
                <th
                  key={col}
                  className="border-b border-border/60 bg-muted/50 px-2 py-1 font-semibold text-foreground"
                >
                  {col}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {parsed.map((row, i) => (
              <tr key={i}>
                {columns.map((col) => (
                  <td
                    key={col}
                    className="border-b border-border/40 px-2 py-1 text-muted-foreground"
                  >
                    {String(row[col] ?? "")}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  }

  if (parsed !== undefined && typeof parsed === "object") {
    return (
      <pre className="overflow-x-auto whitespace-pre-wrap rounded bg-muted/50 p-2 font-mono text-xs text-foreground/80">
        {JSON.stringify(parsed, null, 2)}
      </pre>
    );
  }

  return (
    <pre className="overflow-x-auto whitespace-pre-wrap rounded bg-muted/50 p-2 font-mono text-xs text-foreground/80">
      {text}
    </pre>
  );
}

function ToolDetailPanel({
  envelope,
  resultContent,
  verbose,
}: {
  envelope: ReturnType<typeof parseIronclawToolResultEnvelope>;
  resultContent: string | null;
  verbose?: boolean;
}) {
  const tabs: { id: string; label: string; content: React.ReactNode }[] = [];

  if (resultContent) {
    const displayText = envelope?.output ?? resultContent;
    tabs.push({ id: "result", label: "Result", content: <RichResult text={displayText} /> });
  }
  if (envelope?.inputSummary) {
    tabs.push({
      id: "input",
      label: "Input",
      content: <p className="text-xs text-muted-foreground/80">{envelope.inputSummary}</p>,
    });
  }
  if (verbose) {
    tabs.push({
      id: "meta",
      label: "Meta",
      content: (
        <div className="space-y-1 text-[10px] text-muted-foreground/50 font-mono">
          {envelope?.outputKind && <div>Kind: {envelope.outputKind}</div>}
          {envelope?.truncated && <div className="text-amber-600">Truncated</div>}
        </div>
      ),
    });
  }

  const [activeTab, setActiveTab] = useState(tabs[0]?.id ?? null);
  const active = tabs.find((t) => t.id === activeTab) ?? tabs[0];

  if (tabs.length === 0) return null;

  return (
    <div className="rounded-b-lg border-x border-b border-border bg-card mt-1">
      <div className="flex items-center gap-1 border-b border-border px-2 pt-1.5">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            onClick={() => setActiveTab(tab.id)}
            className={cn(
              "rounded-t-md px-2.5 py-1 font-mono text-[10px]",
              active?.id === tab.id
                ? "bg-muted text-foreground"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {tab.label}
          </button>
        ))}
      </div>
      <div className="p-2 text-xs">{active?.content ?? null}</div>
    </div>
  );
}

function ToolRunRow({ item, verbose }: { item: ToolItem; verbose?: boolean }) {
  const status = toolStatus(item);
  const dotClass = DOT_STYLE[status] || DOT_STYLE.running;
  const statusWord = STATUS_WORD[status] || "run";
  const Icon = toolIcon(item.call.name);
  const [expanded, setExpanded] = useState(status === "error");

  const resultContent =
    item.result && typeof item.result.content === "string" ? item.result.content : null;
  const envelope = parseIronclawToolResultEnvelope(item.result?.content ?? item.call.output);

  const inputFromArgs = (() => {
    if (envelope?.inputSummary) return envelope.inputSummary;
    try {
      const args = JSON.parse(item.call.arguments);
      return typeof args.input === "string" && args.input.length > 0 ? args.input : null;
    } catch {
      return null;
    }
  })();

  const detailEnvelope =
    envelope ??
    (inputFromArgs
      ? {
          title: item.call.name,
          inputSummary: inputFromArgs,
          output: "",
          outputKind: null as string | null,
          truncated: false,
        }
      : null);

  const displayName = detailEnvelope?.title ?? item.call.name;
  const hasDetails = !!(resultContent || detailEnvelope?.inputSummary || verbose);

  return (
    <div className="flex flex-col min-w-[240px]">
      <button
        type="button"
        onClick={() => hasDetails && setExpanded(!expanded)}
        aria-expanded={expanded}
        className="v2-button flex w-full min-h-[28px] items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-xs"
      >
        <span className={cn("h-2 w-2 shrink-0 rounded-full", dotClass)} />
        <span className="shrink-0 font-mono text-[10px] uppercase tracking-wide text-muted-foreground">
          {statusWord}
        </span>
        <Icon size={10} className="shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate font-medium text-foreground/80">{displayName}</span>
        <span className="ml-auto flex shrink-0 items-center gap-1">
          {status === "running" && (
            <Loader2 size={10} className="animate-spin text-muted-foreground" />
          )}
          {status === "success" && (
            <CheckCircle2 size={10} className="shrink-0 text-[color:var(--near-green)]" />
          )}
          {status === "error" && <AlertCircle size={10} className="shrink-0 text-destructive" />}
          <span className="inline-flex w-3 shrink-0 justify-center">
            {hasDetails ? (
              expanded ? (
                <ChevronDown size={10} className="text-muted-foreground/50" />
              ) : (
                <ChevronRight size={10} className="text-muted-foreground/50" />
              )
            ) : (
              <span className="w-[10px]" />
            )}
          </span>
        </span>
      </button>
      {expanded && hasDetails && (
        <ToolDetailPanel
          envelope={detailEnvelope}
          resultContent={resultContent}
          verbose={verbose}
        />
      )}
    </div>
  );
}

export function ActivityRun({ tools, verbose }: { tools: ToolItem[]; verbose?: boolean }) {
  const hasError = tools.some((t) => toolStatus(t) === "error");
  const [expanded, setExpanded] = useState(hasError);

  if (tools.length === 0) return null;

  if (tools.length <= TOOL_RUN_COLLAPSE_AFTER) {
    return (
      <div className="w-full max-w-[85%] min-w-[240px] space-y-0.5">
        {tools.map((item) => (
          <ToolRunRow key={item.call.id} item={item} verbose={verbose} />
        ))}
      </div>
    );
  }

  const summary = summarizeTools(tools);

  return (
    <div className="w-full max-w-[85%] min-w-[240px]">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
        className={cn(
          "v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-xs",
          hasError ? "text-destructive" : "text-muted-foreground hover:text-foreground",
        )}
      >
        <span className="truncate font-medium">{summary}</span>
        {expanded ? (
          <ChevronDown size={12} className="ml-auto shrink-0 text-muted-foreground/50" />
        ) : (
          <ChevronRight size={12} className="ml-auto shrink-0 text-muted-foreground/50" />
        )}
      </button>
      {expanded && (
        <div className="mt-1 space-y-0.5">
          {tools.map((item) => (
            <ToolRunRow key={item.call.id} item={item} verbose={verbose} />
          ))}
        </div>
      )}
    </div>
  );
}
