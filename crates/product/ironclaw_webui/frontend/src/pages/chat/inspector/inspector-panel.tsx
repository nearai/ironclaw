import React from "react";
import { createPortal } from "react-dom";

import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";
import { ActivityKind } from "./activity-kind";
import { fetchInspectorTool } from "./inspector-api";
import {
  reduceInspectorActivity,
  rememberInspectorRun,
  type InspectorActivityRow,
} from "./inspector-activity";
import {
  INSPECTOR_HEALTH,
  INSPECTOR_TABS,
  inspectorViewportMode,
  readInspectorPreferences,
  writeInspectorPreferences,
  type InspectorPreferences,
  type InspectorTab,
} from "./inspector-state";
import {
  type DiagnosticMetricTotal,
  type SessionDiagnosticStats,
} from "./inspector-session-stats";
import { useInspector } from "./useInspector";
import "./inspector-translations";

const HEALTH_LABEL_KEYS = {
  [INSPECTOR_HEALTH.IDLE]: "inspector.health.idle",
  [INSPECTOR_HEALTH.LOADING]: "inspector.health.loading",
  [INSPECTOR_HEALTH.CONNECTING]: "inspector.health.connecting",
  [INSPECTOR_HEALTH.CONNECTED]: "inspector.health.connected",
  [INSPECTOR_HEALTH.RECONNECTING]: "inspector.health.reconnecting",
  [INSPECTOR_HEALTH.DISCONNECTED]: "inspector.health.disconnected",
  [INSPECTOR_HEALTH.FORBIDDEN]: "inspector.health.forbidden",
  [INSPECTOR_HEALTH.UNAVAILABLE]: "inspector.health.unavailable",
};

const PAGE_HEADER_INSPECTOR_ACTION_ID = "page-header-inspector-action";

function useViewportMode(): "mobile" | "overlay" | "sidebar" {
  const [mode, setMode] = React.useState(() =>
    inspectorViewportMode(typeof window === "undefined" ? 0 : window.innerWidth),
  );
  React.useEffect(() => {
    const update = () => setMode(inspectorViewportMode(window.innerWidth));
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);
  return mode;
}

function EmptyTab({ title, description }: { title: string; description: string }) {
  return (
    <div className="grid min-h-48 place-items-center px-5 py-8 text-center">
      <div>
        <p className="text-sm font-medium text-[var(--v2-text-strong)]">{title}</p>
        <p className="mt-2 max-w-64 text-xs leading-5 text-[var(--v2-text-muted)]">
          {description}
        </p>
      </div>
    </div>
  );
}

interface BoundedDiagnosticText {
  content: string;
  original_bytes: number;
  truncated: boolean;
}

interface PromptComponent {
  kind: string;
  label: BoundedDiagnosticText;
  content: BoundedDiagnosticText;
  estimated_tokens: number | null;
}

interface PromptDiagnostic {
  components: PromptComponent[];
  components_truncated: boolean;
  reconstructed_prompt: BoundedDiagnosticText;
  total_estimated_tokens: number | null;
  message_count: number;
  identity_message_count: number;
  instruction_snippet_count: number;
  active_skills: BoundedDiagnosticText[];
  active_skills_truncated: boolean;
  capability_count: number;
  requested_model: BoundedDiagnosticText | null;
  effective_model: BoundedDiagnosticText | null;
  context_limit: number | null;
}

function formatNumber(value: number | null | undefined, unavailable: string): string {
  return typeof value === "number" ? value.toLocaleString() : unavailable;
}

function PromptShell({
  snapshot,
  health,
}: {
  snapshot: Record<string, unknown> | null;
  health: string;
}) {
  const t = useT();
  const prompt = snapshot?.prompt as PromptDiagnostic | null | undefined;
  if (!prompt) {
    if (health === INSPECTOR_HEALTH.LOADING || health === INSPECTOR_HEALTH.CONNECTING) {
      return (
        <EmptyTab
          title={t("inspector.prompt.loadingTitle")}
          description={t("inspector.prompt.loadingDescription")}
        />
      );
    }
    if (health === INSPECTOR_HEALTH.FORBIDDEN || health === INSPECTOR_HEALTH.UNAVAILABLE) {
      return (
        <EmptyTab
          title={t("inspector.prompt.unavailableTitle")}
          description={t("inspector.prompt.unavailableDescription")}
        />
      );
    }
    return (
      <EmptyTab
        title={t("inspector.prompt.emptyTitle")}
        description={t("inspector.prompt.emptyDescription")}
      />
    );
  }
  const contextPercent = prompt.context_limit && prompt.total_estimated_tokens != null
    ? Math.min(100, (prompt.total_estimated_tokens / prompt.context_limit) * 100)
    : null;
  const anyTruncated = prompt.components_truncated
    || prompt.reconstructed_prompt.truncated
    || prompt.active_skills_truncated
    || prompt.active_skills.some((skill) => skill.truncated)
    || prompt.requested_model?.truncated === true
    || prompt.effective_model?.truncated === true
    || prompt.components.some(
      (component) => component.label.truncated || component.content.truncated,
    );
  return (
    <div className="space-y-4 p-4" data-testid="inspector-prompt-content">
      <div className="grid grid-cols-2 gap-3">
        <div className="rounded-xl border border-[var(--v2-panel-border)] p-3">
          <p className="text-xs text-[var(--v2-text-muted)]">{t("inspector.prompt.estimatedTokens")}</p>
          <p className="mt-1 text-xl font-semibold text-[var(--v2-text-strong)]">
            {formatNumber(prompt.total_estimated_tokens, t("inspector.unavailable"))}
          </p>
        </div>
        <div className="rounded-xl border border-[var(--v2-panel-border)] p-3">
          <p className="text-xs text-[var(--v2-text-muted)]">{t("inspector.prompt.contextLimit")}</p>
          <p className="mt-1 text-xl font-semibold text-[var(--v2-text-strong)]">
            {formatNumber(prompt.context_limit, t("inspector.unavailable"))}
          </p>
        </div>
      </div>
      {contextPercent != null && (
        <div>
          <div className="mb-1 flex justify-between text-[11px] text-[var(--v2-text-muted)]">
            <span>{t("inspector.prompt.contextUsage")}</span>
            <span>{contextPercent.toFixed(1)}%</span>
          </div>
          <div className="h-1.5 overflow-hidden rounded-full bg-[var(--v2-surface-soft)]">
            <div
              className="h-full rounded-full bg-[var(--v2-accent)]"
              style={{ width: `${contextPercent}%` }}
            />
          </div>
        </div>
      )}
      <dl className="grid grid-cols-2 gap-x-3 gap-y-2 text-xs">
        <div><dt className="text-[var(--v2-text-faint)]">{t("inspector.prompt.effectiveModel")}</dt><dd>{prompt.effective_model?.content || t("inspector.unavailable")}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">{t("inspector.prompt.requestedModel")}</dt><dd>{prompt.requested_model?.content || t("inspector.default")}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">{t("inspector.prompt.messages")}</dt><dd>{prompt.message_count}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">{t("inspector.prompt.identityMessages")}</dt><dd>{prompt.identity_message_count}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">{t("inspector.prompt.instructionSnippets")}</dt><dd>{prompt.instruction_snippet_count}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">{t("inspector.prompt.capabilities")}</dt><dd>{prompt.capability_count}</dd></div>
      </dl>
      {prompt.active_skills.length > 0 && (
        <div>
          <p className="text-xs text-[var(--v2-text-faint)]">{t("inspector.prompt.activeSkills")}</p>
          <div className="mt-2 flex flex-wrap gap-1.5">
            {prompt.active_skills.map((skill, index) => (
              <span key={`${skill.content}-${index}`} className="rounded-full bg-[var(--v2-surface-soft)] px-2 py-1 text-[11px]">
                {skill.content}{skill.truncated ? "…" : ""}
              </span>
            ))}
          </div>
        </div>
      )}
      {anyTruncated && (
        <p role="status" className="rounded-lg bg-[var(--v2-surface-soft)] px-3 py-2 text-xs text-[var(--v2-warning-text)]">
          {t("inspector.prompt.truncatedNotice")}
        </p>
      )}
      <div className="space-y-2">
        {prompt.components.map((component, index) => (
          <details key={`${component.label.content}-${index}`} className="rounded-xl border border-[var(--v2-panel-border)]">
            <summary className="cursor-pointer list-none px-3 py-2 text-xs font-medium text-[var(--v2-text-strong)]">
              <span>{component.label.content}</span>
              <span className="ml-2 font-normal text-[var(--v2-text-faint)]">
                {component.kind} · {t("inspector.prompt.tokenCount", {
                  count: formatNumber(component.estimated_tokens, t("inspector.unavailable")),
                })}
                {component.content.truncated ? ` · ${t("inspector.truncated")}` : ""}
              </span>
            </summary>
            <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-words border-t border-[var(--v2-panel-border)] p-3 text-[11px] leading-5 text-[var(--v2-text-muted)]">
              {component.content.content}
            </pre>
          </details>
        ))}
      </div>
      <details className="rounded-xl border border-[var(--v2-panel-border)]">
        <summary className="cursor-pointer px-3 py-2 text-xs font-medium">{t("inspector.prompt.fullReconstruction")}</summary>
        <div className="border-t border-[var(--v2-panel-border)] p-3">
          <p className="mb-3 text-[11px] leading-5 text-[var(--v2-text-faint)]">
            {t("inspector.prompt.reconstructionNotice")}
          </p>
          <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-words text-[11px] leading-5 text-[var(--v2-text-muted)]">
            {prompt.reconstructed_prompt.content}
          </pre>
        </div>
      </details>
    </div>
  );
}

function ActivityShell({
  snapshot,
  updates,
  runHistory,
  selectedRunId,
  onSelectRun,
  threadId,
}: {
  snapshot: Record<string, unknown> | null;
  updates: Array<Record<string, unknown>>;
  runHistory: string[];
  selectedRunId: string | null;
  onSelectRun: (runId: string) => void;
  threadId: string | null;
}) {
  const t = useT();
  const activity = React.useMemo(
    () => reduceInspectorActivity(snapshot, updates),
    [snapshot, updates],
  );
  const requestedIndex = selectedRunId ? runHistory.indexOf(selectedRunId) : -1;
  // A pinned run can fall out of the bounded history. Fall back to the latest
  // observed turn, matching the default selection, rather than to the oldest.
  const selectedIndex = requestedIndex >= 0
    ? requestedIndex
    : runHistory.length - 1;
  const previousRun = selectedIndex > 0 ? runHistory[selectedIndex - 1] : null;
  const nextRun = selectedIndex >= 0 && selectedIndex < runHistory.length - 1
    ? runHistory[selectedIndex + 1]
    : null;
  if (activity.length === 0) {
    return (
      <div>
        <TurnNavigation
          runHistory={runHistory}
          selectedIndex={selectedIndex}
          previousRun={previousRun}
          nextRun={nextRun}
          onSelectRun={onSelectRun}
        />
        <EmptyTab
          title={t("inspector.activity.emptyTitle")}
          description={t("inspector.activity.emptyDescription")}
        />
      </div>
    );
  }
  return (
    <div className="space-y-3 p-4" data-testid="inspector-activity-content">
      <TurnNavigation
        runHistory={runHistory}
        selectedIndex={selectedIndex}
        previousRun={previousRun}
        nextRun={nextRun}
        onSelectRun={onSelectRun}
      />
      <ol className="space-y-2" aria-label={t("inspector.activity.timelineLabel")}>
        {activity.map((entry) => (
          <ActivityEntry
            key={entry.key}
            entry={entry}
            threadId={threadId}
            runId={selectedRunId}
          />
        ))}
      </ol>
    </div>
  );
}

function TurnNavigation({
  runHistory,
  selectedIndex,
  previousRun,
  nextRun,
  onSelectRun,
}: {
  runHistory: string[];
  selectedIndex: number;
  previousRun: string | null;
  nextRun: string | null;
  onSelectRun: (runId: string) => void;
}) {
  const t = useT();
  const latestRun = selectedIndex >= 0 && selectedIndex < runHistory.length - 1
    ? runHistory.at(-1) || null
    : null;
  return (
    <div className="flex items-center justify-between gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
      <button
        type="button"
        aria-label={t("inspector.navigation.previousLabel")}
        disabled={!previousRun}
        onClick={() => previousRun && onSelectRun(previousRun)}
        className="rounded px-2 py-1 text-xs disabled:opacity-40"
      >
        ← {t("inspector.navigation.previous")}
      </button>
      <p className="text-center text-xs text-[var(--v2-text-muted)]">
        {t("inspector.navigation.position", {
          current: selectedIndex >= 0 ? selectedIndex + 1 : 0,
          total: runHistory.length,
        })}
      </p>
      <div className="flex items-center gap-1">
        {latestRun && (
          <button
            type="button"
            aria-label={t("inspector.navigation.latestLabel")}
            onClick={() => onSelectRun(latestRun)}
            className="rounded px-2 py-1 text-xs font-medium text-[var(--v2-accent-text)]"
          >
            {t("inspector.navigation.latest")}
          </button>
        )}
        <button
          type="button"
          aria-label={t("inspector.navigation.nextLabel")}
          disabled={!nextRun}
          onClick={() => nextRun && onSelectRun(nextRun)}
          className="rounded px-2 py-1 text-xs disabled:opacity-40"
        >
          {t("inspector.navigation.next")} →
        </button>
      </div>
    </div>
  );
}

const ACTIVITY_LABEL_KEYS: Record<ActivityKind, string> = {
  [ActivityKind.TurnStarted]: "inspector.activity.turnStarted",
  [ActivityKind.PromptPrepared]: "inspector.activity.promptPrepared",
  [ActivityKind.ModelCallStarted]: "inspector.activity.modelCallStarted",
  [ActivityKind.ModelCallCompleted]: "inspector.activity.modelCallCompleted",
  [ActivityKind.ModelCallFailed]: "inspector.activity.modelCallFailed",
  [ActivityKind.Progress]: "inspector.activity.progress",
  [ActivityKind.ToolStarted]: "inspector.activity.toolStarted",
  [ActivityKind.ToolCompleted]: "inspector.activity.toolCompleted",
  [ActivityKind.ToolFailed]: "inspector.activity.toolFailed",
  [ActivityKind.GateBlocked]: "inspector.activity.gateBlocked",
  [ActivityKind.FinalResponseCompleted]: "inspector.activity.finalResponseCompleted",
  [ActivityKind.StreamDisconnected]: "inspector.activity.streamDisconnected",
  [ActivityKind.StreamResumed]: "inspector.activity.streamResumed",
};

function shortId(value: string | null): string | null {
  return value && value.length > 12 ? `${value.slice(0, 8)}…` : value;
}

interface ToolDetail {
  capability_name: BoundedDiagnosticText;
  arguments: BoundedDiagnosticText | null;
  result: BoundedDiagnosticText | null;
  status: "started" | "succeeded" | "failed";
  duration_ms: number | null;
  output_bytes: number | null;
  failure_category: BoundedDiagnosticText | null;
  failure_summary: BoundedDiagnosticText | null;
}

const TOOL_STATUS_LABEL_KEYS: Record<ToolDetail["status"], string> = {
  started: "inspector.tool.statusStarted",
  succeeded: "inspector.tool.statusSucceeded",
  failed: "inspector.tool.statusFailed",
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonNegativeSafeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isBoundedDiagnosticText(value: unknown): value is BoundedDiagnosticText {
  return isRecord(value)
    && typeof value.content === "string"
    && isNonNegativeSafeInteger(value.original_bytes)
    && typeof value.truncated === "boolean";
}

function isNullableBoundedDiagnosticText(
  value: unknown,
): value is BoundedDiagnosticText | null {
  return value === null || isBoundedDiagnosticText(value);
}

function isNullableNonNegativeSafeInteger(value: unknown): value is number | null {
  return value === null || isNonNegativeSafeInteger(value);
}

function decodeToolDetailResponse(response: unknown): ToolDetail | null {
  if (!isRecord(response) || !isRecord(response.tool)) return null;
  const tool = response.tool;
  if (
    !isBoundedDiagnosticText(tool.capability_name)
    || !isNullableBoundedDiagnosticText(tool.arguments)
    || !isNullableBoundedDiagnosticText(tool.result)
    || (tool.status !== "started" && tool.status !== "succeeded" && tool.status !== "failed")
    || !isNullableNonNegativeSafeInteger(tool.duration_ms)
    || !isNullableNonNegativeSafeInteger(tool.output_bytes)
    || !isNullableBoundedDiagnosticText(tool.failure_category)
    || !isNullableBoundedDiagnosticText(tool.failure_summary)
  ) {
    return null;
  }
  return {
    capability_name: tool.capability_name,
    arguments: tool.arguments,
    result: tool.result,
    status: tool.status,
    duration_ms: tool.duration_ms,
    output_bytes: tool.output_bytes,
    failure_category: tool.failure_category,
    failure_summary: tool.failure_summary,
  };
}

function ToolDetailDisclosure({
  threadId,
  runId,
  activityId,
}: {
  threadId: string;
  runId: string;
  activityId: string;
}) {
  const t = useT();
  const [open, setOpen] = React.useState(false);
  const [loading, setLoading] = React.useState(false);
  const [tool, setTool] = React.useState<ToolDetail | null>(null);
  const [unavailable, setUnavailable] = React.useState(false);
  const requestRef = React.useRef<AbortController | null>(null);
  React.useEffect(() => () => {
    requestRef.current?.abort();
    requestRef.current = null;
  }, []);
  const load = () => {
    const nextOpen = !open;
    setOpen(nextOpen);
    if (!nextOpen) {
      requestRef.current?.abort();
      requestRef.current = null;
      setLoading(false);
      return;
    }
    if (tool || loading) return;
    const controller = new AbortController();
    requestRef.current = controller;
    setLoading(true);
    setUnavailable(false);
    fetchInspectorTool({ threadId, runId, activityId, signal: controller.signal })
      .then((response) => {
        if (controller.signal.aborted || requestRef.current !== controller) return;
        const detail = decodeToolDetailResponse(response);
        setTool(detail);
        setUnavailable(!detail);
      })
      .catch(() => {
        if (!controller.signal.aborted && requestRef.current === controller) {
          setUnavailable(true);
        }
      })
      .finally(() => {
        if (requestRef.current === controller) {
          requestRef.current = null;
          setLoading(false);
        }
      });
  };
  return (
    <div className="mt-3 border-t border-[var(--v2-panel-border)] pt-3">
      <button
        type="button"
        aria-expanded={open}
        onClick={load}
        className="text-xs font-medium text-[var(--v2-accent-text)]"
      >
        {open ? t("inspector.tool.hideDetails") : t("inspector.tool.showDetails")}
      </button>
      {open && (
        <div className="mt-3 space-y-3 text-xs" data-testid={`inspector-tool-detail-${activityId}`}>
          {loading && <p role="status">{t("inspector.tool.loadingDetails")}</p>}
          {unavailable && <p role="status">{t("inspector.tool.unavailableDetails")}</p>}
          {tool && (
            <>
              <p><span className="font-medium">{t("inspector.tool.capability")}:</span> {tool.capability_name.content}</p>
              <p><span className="font-medium">{t("inspector.tool.status")}:</span> {t(TOOL_STATUS_LABEL_KEYS[tool.status])}</p>
              {tool.duration_ms != null && <p>{t("inspector.tool.duration", { count: tool.duration_ms.toLocaleString() })}</p>}
              <ToolDetailBlock label={t("inspector.tool.arguments")} value={tool.arguments} />
              <ToolDetailBlock label={t("inspector.tool.output")} value={tool.result} />
              {tool.output_bytes != null && <p>{t("inspector.tool.outputSize", { count: tool.output_bytes.toLocaleString() })}</p>}
              <ToolDetailBlock label={t("inspector.tool.failureCategory")} value={tool.failure_category} />
              <ToolDetailBlock label={t("inspector.tool.failure")} value={tool.failure_summary} />
            </>
          )}
        </div>
      )}
    </div>
  );
}

function ToolDetailBlock({ label, value }: { label: string; value: BoundedDiagnosticText | null }) {
  const t = useT();
  if (!value) return null;
  return (
    <div>
      <p className="mb-1 font-medium">
        {label}{value.truncated ? ` · ${t("inspector.tool.truncatedFrom", { count: value.original_bytes.toLocaleString() })}` : ""}
      </p>
      <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-[var(--v2-surface-soft)] p-3 text-[11px]">
        {value.content}
      </pre>
    </div>
  );
}

function ActivityEntry({
  entry,
  threadId,
  runId,
}: {
  entry: InspectorActivityRow;
  threadId: string | null;
  runId: string | null;
}) {
  const t = useT();
  const failed = entry.kind === ActivityKind.ModelCallFailed
    || entry.kind === ActivityKind.ToolFailed
    || entry.kind === ActivityKind.GateBlocked;
  const hasToolDetails = entry.kind === ActivityKind.ToolStarted
    || entry.kind === ActivityKind.ToolCompleted
    || entry.kind === ActivityKind.ToolFailed;
  const correlation = shortId(entry.activity_id || entry.model_call_id);
  const timestamp = new Date(entry.occurred_at);
  return (
    <li className="rounded-xl border border-[var(--v2-panel-border)] p-3" data-activity-kind={entry.kind}>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-xs font-medium text-[var(--v2-text-strong)]">
            {ACTIVITY_LABEL_KEYS[entry.kind]
              ? t(ACTIVITY_LABEL_KEYS[entry.kind])
              : entry.kind.replaceAll("_", " ")}
          </p>
          {entry.summary?.content && (
            <p className="mt-1 break-words text-xs text-[var(--v2-text-muted)]">
              {entry.summary.content}{entry.summary.truncated ? "…" : ""}
            </p>
          )}
        </div>
        <span className={cn(
          "shrink-0 rounded-full px-2 py-0.5 text-[10px]",
          failed
            ? "bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]"
            : entry.pending
              ? "bg-[var(--v2-surface-soft)] text-[var(--v2-warning-text)]"
              : "bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]",
        )}>
          {failed
            ? t("inspector.activity.failed")
            : entry.pending
              ? t("inspector.activity.pending")
              : t("inspector.activity.recorded")}
        </span>
      </div>
      <p className="mt-2 font-mono text-[10px] text-[var(--v2-text-faint)]">
        {Number.isNaN(timestamp.getTime()) ? entry.occurred_at : timestamp.toLocaleTimeString()}
        {entry.iteration != null ? ` · ${t("inspector.activity.iteration", { count: entry.iteration })}` : ""}
        {correlation ? ` · ${correlation}` : ""}
      </p>
      {hasToolDetails && entry.activity_id && threadId && runId && (
        <ToolDetailDisclosure
          key={`${threadId}:${runId}:${entry.activity_id}`}
          threadId={threadId}
          runId={runId}
          activityId={entry.activity_id}
        />
      )}
    </li>
  );
}

function metricValue(
  metric: DiagnosticMetricTotal,
  sampleCount: number,
  unavailable: string,
  suffix = "",
): string {
  if (sampleCount > 0 && metric.unavailable_samples >= sampleCount) return unavailable;
  return `${metric.known_total.toLocaleString()}${suffix}`;
}

function MetricCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl border border-[var(--v2-panel-border)] p-3">
      <p className="text-xs text-[var(--v2-text-muted)]">{label}</p>
      <p className="mt-1 text-xl font-semibold text-[var(--v2-text-strong)]">{value}</p>
    </div>
  );
}

function countValue(value: number | undefined, unavailable: string): string {
  return typeof value === "number" ? value.toLocaleString() : unavailable;
}

function StreamHealth({
  health,
  reconnectCount,
  receivedUpdateCount,
  lastUpdateAt,
}: {
  health: keyof typeof HEALTH_LABEL_KEYS;
  reconnectCount: number;
  receivedUpdateCount: number;
  lastUpdateAt: string | null;
}) {
  const t = useT();
  const observed = lastUpdateAt ? new Date(lastUpdateAt) : null;
  const lastUpdate = observed && !Number.isNaN(observed.getTime())
    ? observed.toLocaleTimeString()
    : t("inspector.stream.noUpdates");
  return (
    <div className="rounded-xl border border-[var(--v2-panel-border)] p-3 text-xs" data-testid="inspector-stream-health">
      <p className="font-medium text-[var(--v2-text-strong)]">{t("inspector.stream.title")}</p>
      <dl className="mt-2 grid grid-cols-2 gap-x-3 gap-y-2">
        <div><dt className="text-[var(--v2-text-faint)]">{t("inspector.stream.state")}</dt><dd data-testid="inspector-stream-state">{t(HEALTH_LABEL_KEYS[health])}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">{t("inspector.stream.reconnects")}</dt><dd data-testid="inspector-stream-reconnects">{reconnectCount.toLocaleString()}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">{t("inspector.stream.updates")}</dt><dd data-testid="inspector-stream-updates">{receivedUpdateCount.toLocaleString()}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">{t("inspector.stream.lastUpdate")}</dt><dd data-testid="inspector-stream-last-update">{lastUpdate}</dd></div>
      </dl>
    </div>
  );
}

function StatsShell({
  stats,
  health,
  reconnectCount,
  receivedUpdateCount,
  lastUpdateAt,
}: {
  stats: SessionDiagnosticStats | null;
  health: keyof typeof HEALTH_LABEL_KEYS;
  reconnectCount: number;
  receivedUpdateCount: number;
  lastUpdateAt: string | null;
}) {
  const t = useT();
  if (!stats) {
    return (
      <div className="space-y-4 p-4" data-testid="inspector-stats-content">
        <StreamHealth {...{ health, reconnectCount, receivedUpdateCount, lastUpdateAt }} />
        <EmptyTab
          title={t("inspector.stats.emptyTitle")}
          description={t("inspector.stats.emptyDescription")}
        />
      </div>
    );
  }
  const knownLatencySamples = Math.max(
    0,
    stats.total_model_calls - stats.total_latency_ms.unavailable_samples,
  );
  const averageLatency = knownLatencySamples > 0
    ? `${Math.round(stats.total_latency_ms.known_total / knownLatencySamples).toLocaleString()} ms`
    : t("inspector.unavailable");
  const partialMetricCount = [
    stats.input_tokens,
    stats.output_tokens,
    stats.cache_read_input_tokens,
    stats.cache_creation_input_tokens,
    stats.total_latency_ms,
  ].reduce((total, metric) => total + metric.unavailable_samples, 0);
  return (
    <div className="space-y-4 p-4" data-testid="inspector-stats-content">
      <div className="grid grid-cols-2 gap-3">
        <MetricCard label={t("inspector.stats.modelCalls")} value={stats.total_model_calls.toLocaleString()} />
        <MetricCard label={t("inspector.stats.toolCalls")} value={countValue(stats.total_tool_calls, t("inspector.unavailable"))} />
        <MetricCard label={t("inspector.stats.successfulToolCalls")} value={countValue(stats.successful_tool_calls, t("inspector.unavailable"))} />
        <MetricCard label={t("inspector.stats.failedToolCalls")} value={countValue(stats.failed_tool_calls, t("inspector.unavailable"))} />
        <MetricCard label={t("inspector.stats.averageLatency")} value={averageLatency} />
        <MetricCard label={t("inspector.stats.inputTokens")} value={metricValue(stats.input_tokens, stats.total_model_calls, t("inspector.unavailable"))} />
        <MetricCard label={t("inspector.stats.outputTokens")} value={metricValue(stats.output_tokens, stats.total_model_calls, t("inspector.unavailable"))} />
        <MetricCard label={t("inspector.stats.cacheReadTokens")} value={metricValue(stats.cache_read_input_tokens, stats.total_model_calls, t("inspector.unavailable"))} />
        <MetricCard label={t("inspector.stats.cacheCreatedTokens")} value={metricValue(stats.cache_creation_input_tokens, stats.total_model_calls, t("inspector.unavailable"))} />
        <MetricCard label={t("inspector.stats.totalLatency")} value={metricValue(stats.total_latency_ms, stats.total_model_calls, t("inspector.unavailable"), " ms")} />
      </div>
      <StreamHealth {...{ health, reconnectCount, receivedUpdateCount, lastUpdateAt }} />
      <div className="rounded-xl border border-[var(--v2-panel-border)] p-3 text-xs">
        <p className="font-medium text-[var(--v2-text-strong)]">{t("inspector.stats.callsPerModel")}</p>
        {stats.calls_per_model.length === 0 ? (
          <p className="mt-2 text-[var(--v2-text-muted)]">{t("inspector.stats.noModelBreakdown")}</p>
        ) : (
          <dl className="mt-2 space-y-1.5">
            {stats.calls_per_model.map((entry, index) => (
              <div key={index} className="flex justify-between gap-3">
                <dt className="min-w-0 truncate text-[var(--v2-text-muted)]">{entry.model.content}</dt>
                <dd>{entry.calls.toLocaleString()}</dd>
              </div>
            ))}
          </dl>
        )}
      </div>
      {(partialMetricCount > 0 || stats.calls_per_model_truncated) && (
        <p role="status" className="rounded-lg bg-[var(--v2-surface-soft)] px-3 py-2 text-xs text-[var(--v2-warning-text)]">
          {t("inspector.stats.partial", { count: partialMetricCount.toLocaleString() })}
          {stats.calls_per_model_truncated ? ` ${t("inspector.stats.modelBreakdownTruncated")}` : ""}
        </p>
      )}
    </div>
  );
}

function StatusNotice({ health, error }: { health: string; error: string | null }) {
  const t = useT();
  if (!error && health !== INSPECTOR_HEALTH.DISCONNECTED) return null;
  return (
    <div
      role="status"
      data-testid="inspector-status-notice"
      className="m-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-text-muted)]"
    >
      {error ? t(error) : t("inspector.error.disconnected")}
    </div>
  );
}

function InspectorPanelCore({
  threadId,
  runId,
}: {
  threadId: string | null;
  runId: string | null;
}) {
  const t = useT();
  const viewportMode = useViewportMode();
  const [preferences, setPreferences] = React.useState<InspectorPreferences>(() =>
    readInspectorPreferences(),
  );
  const [runHistory, setRunHistory] = React.useState<string[]>([]);
  const [selectedRunId, setSelectedRunId] = React.useState<string | null>(null);
  const selectedThreadRef = React.useRef(threadId);
  const selectionPinnedRef = React.useRef(false);
  React.useEffect(() => {
    const history = rememberInspectorRun(threadId, runId);
    const threadChanged = selectedThreadRef.current !== threadId;
    selectedThreadRef.current = threadId;
    if (threadChanged) selectionPinnedRef.current = false;
    setRunHistory(history);
    setSelectedRunId((current) => {
      if (!selectionPinnedRef.current) return runId || history.at(-1) || null;
      if (current && history.includes(current)) return current;
      // The pinned run was evicted from the bounded history: rejoin the
      // latest observed turn rather than jumping to the oldest retained one.
      return history.at(-1) || runId || null;
    });
  }, [threadId, runId]);
  const selectRun = React.useCallback((nextRunId: string) => {
    selectionPinnedRef.current = true;
    setSelectedRunId(nextRunId);
  }, []);
  const inspector = useInspector({
    enabled: true,
    threadId,
    runId: selectedRunId,
  });

  const updatePreferences = React.useCallback((next: InspectorPreferences) => {
    setPreferences(next);
    writeInspectorPreferences(next);
  }, []);
  const setActiveTab = (activeTab: InspectorTab) =>
    updatePreferences({ ...preferences, activeTab });
  const setOpen = (open: boolean) => updatePreferences({ ...preferences, open });
  const toggleLabel = preferences.open ? t("inspector.closeLabel") : t("inspector.open");
  const headerTarget = typeof document === "undefined"
    ? null
    : document.getElementById(PAGE_HEADER_INSPECTOR_ACTION_ID);
  const headerToggle = headerTarget
    ? createPortal(
        <button
          type="button"
          aria-label={toggleLabel}
          aria-pressed={preferences.open}
          data-testid="inspector-open"
          onClick={() => setOpen(!preferences.open)}
          className={cn(
            "hidden h-8 w-8 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] sm:grid",
            preferences.open && "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",
          )}
          title={toggleLabel}
        >
          <Icon name="code" className="h-4 w-4" />
        </button>,
        headerTarget,
      )
    : null;

  if (viewportMode === "mobile" || !preferences.open) return headerToggle;

  const snapshot = inspector.snapshot as Record<string, unknown> | null;
  return (
    <>
      {headerToggle}
      <aside
        aria-label={t("inspector.panelLabel")}
        data-testid="inspector-panel"
        data-layout={viewportMode}
        className={cn(
          "flex min-h-0 w-[min(420px,72vw)] flex-col border-l border-[var(--v2-panel-border)] bg-[var(--v2-surface)]",
          viewportMode === "overlay"
            ? "fixed inset-y-0 right-0 z-50 shadow-2xl"
            : "relative shrink-0 shadow-none",
        )}
      >
      <header className="border-b border-[var(--v2-panel-border)] px-4 py-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <h2 className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">
              {t("inspector.title")}
            </h2>
            <div className="mt-1 flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span
                className={cn(
                  "h-2 w-2 rounded-full",
                  inspector.health === INSPECTOR_HEALTH.CONNECTED
                    ? "bg-[var(--v2-positive-text)]"
                    : inspector.health === INSPECTOR_HEALTH.RECONNECTING
                      ? "bg-[var(--v2-warning-text)]"
                      : "bg-[var(--v2-text-faint)]",
                )}
              />
              <span data-testid="inspector-health">{t(HEALTH_LABEL_KEYS[inspector.health])}</span>
            </div>
          </div>
          <button
            type="button"
            aria-label={t("inspector.closeLabel")}
            data-testid="inspector-close"
            onClick={() => setOpen(false)}
            className="rounded-lg px-2 py-1 text-lg leading-none text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)]"
          >
            ×
          </button>
        </div>
        <p className="mt-2 truncate font-mono text-[11px] text-[var(--v2-text-faint)]">
          {threadId && selectedRunId
            ? `${threadId} · ${selectedRunId}`
            : t("inspector.waitingForRun")}
        </p>
      </header>

      <nav aria-label={t("inspector.tabsLabel")} className="flex border-b border-[var(--v2-panel-border)] px-2">
        {INSPECTOR_TABS.map((tab) => (
          <button
            key={tab}
            type="button"
            role="tab"
            aria-selected={preferences.activeTab === tab}
            data-testid={`inspector-tab-${tab}`}
            onClick={() => setActiveTab(tab)}
            className={cn(
              "flex-1 border-b-2 px-2 py-3 text-xs font-medium capitalize",
              preferences.activeTab === tab
                ? "border-[var(--v2-accent)] text-[var(--v2-accent-text)]"
                : "border-transparent text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]",
            )}
          >
            {t(`inspector.tab.${tab}`)}
          </button>
        ))}
      </nav>

      <StatusNotice health={inspector.health} error={inspector.error} />
      <section role="tabpanel" className="min-h-0 flex-1 overflow-y-auto">
        {preferences.activeTab === "prompt" && (
          <PromptShell snapshot={snapshot} health={inspector.health} />
        )}
        {preferences.activeTab === "activity" && (
          <ActivityShell
            snapshot={snapshot}
            updates={inspector.updates}
            runHistory={runHistory}
            selectedRunId={selectedRunId}
            onSelectRun={selectRun}
            threadId={threadId}
          />
        )}
        {preferences.activeTab === "stats" && (
          <StatsShell
            stats={inspector.sessionStats}
            health={inspector.health}
            reconnectCount={inspector.reconnectCount}
            receivedUpdateCount={inspector.receivedUpdateCount}
            lastUpdateAt={inspector.lastUpdateAt}
          />
        )}
      </section>
      </aside>
    </>
  );
}

class InspectorErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: unknown) {
    console.warn("Inspector disabled after a rendering failure", {
      category: error instanceof Error ? error.name : "unknown",
    });
  }

  render() {
    return this.state.failed ? null : this.props.children;
  }
}

export function InspectorPanel(props: { threadId: string | null; runId: string | null }) {
  return (
    <InspectorErrorBoundary>
      <InspectorPanelCore {...props} />
    </InspectorErrorBoundary>
  );
}
