import React from "react";

import { cn } from "../../../utils/cn";
import {
  INSPECTOR_HEALTH,
  INSPECTOR_TABS,
  inspectorViewportMode,
  readInspectorPreferences,
  writeInspectorPreferences,
  type InspectorPreferences,
  type InspectorTab,
} from "./inspector-state";
import { useInspector } from "./useInspector";

const HEALTH_LABELS = {
  [INSPECTOR_HEALTH.IDLE]: "Idle",
  [INSPECTOR_HEALTH.LOADING]: "Loading",
  [INSPECTOR_HEALTH.CONNECTING]: "Connecting",
  [INSPECTOR_HEALTH.CONNECTED]: "Live",
  [INSPECTOR_HEALTH.RECONNECTING]: "Reconnecting",
  [INSPECTOR_HEALTH.DISCONNECTED]: "Disconnected",
  [INSPECTOR_HEALTH.FORBIDDEN]: "Forbidden",
  [INSPECTOR_HEALTH.UNAVAILABLE]: "Unavailable",
};

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

function formatNumber(value: number | null | undefined): string {
  return typeof value === "number" ? value.toLocaleString() : "Unavailable";
}

function PromptShell({
  snapshot,
  health,
}: {
  snapshot: Record<string, unknown> | null;
  health: string;
}) {
  const prompt = snapshot?.prompt as PromptDiagnostic | null | undefined;
  if (!prompt) {
    if (health === INSPECTOR_HEALTH.LOADING || health === INSPECTOR_HEALTH.CONNECTING) {
      return (
        <EmptyTab
          title="Loading prompt diagnostics"
          description="The inspector is loading the latest bounded prompt snapshot."
        />
      );
    }
    if (health === INSPECTOR_HEALTH.FORBIDDEN || health === INSPECTOR_HEALTH.UNAVAILABLE) {
      return (
        <EmptyTab
          title="Prompt diagnostics unavailable"
          description="This session cannot access prompt diagnostics. Chat remains available."
        />
      );
    }
    return (
      <EmptyTab
        title="No prompt captured"
        description="Prompt components will appear here when diagnostics are available for this run."
      />
    );
  }
  const contextPercent = prompt.context_limit && prompt.total_estimated_tokens != null
    ? Math.min(100, (prompt.total_estimated_tokens / prompt.context_limit) * 100)
    : null;
  const anyTruncated = prompt.components_truncated
    || prompt.reconstructed_prompt.truncated
    || prompt.active_skills_truncated
    || prompt.components.some((component) => component.content.truncated);
  return (
    <div className="space-y-4 p-4" data-testid="inspector-prompt-content">
      <div className="grid grid-cols-2 gap-3">
        <div className="rounded-xl border border-[var(--v2-panel-border)] p-3">
          <p className="text-xs text-[var(--v2-text-muted)]">Estimated prompt tokens</p>
          <p className="mt-1 text-xl font-semibold text-[var(--v2-text-strong)]">
            {formatNumber(prompt.total_estimated_tokens)}
          </p>
        </div>
        <div className="rounded-xl border border-[var(--v2-panel-border)] p-3">
          <p className="text-xs text-[var(--v2-text-muted)]">Context limit</p>
          <p className="mt-1 text-xl font-semibold text-[var(--v2-text-strong)]">
            {formatNumber(prompt.context_limit)}
          </p>
        </div>
      </div>
      {contextPercent != null && (
        <div>
          <div className="mb-1 flex justify-between text-[11px] text-[var(--v2-text-muted)]">
            <span>Estimated context usage</span>
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
        <div><dt className="text-[var(--v2-text-faint)]">Effective model</dt><dd>{prompt.effective_model?.content || "Unavailable"}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">Requested model</dt><dd>{prompt.requested_model?.content || "Default"}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">Messages</dt><dd>{prompt.message_count}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">Identity messages</dt><dd>{prompt.identity_message_count}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">Instruction snippets</dt><dd>{prompt.instruction_snippet_count}</dd></div>
        <div><dt className="text-[var(--v2-text-faint)]">Capabilities</dt><dd>{prompt.capability_count}</dd></div>
      </dl>
      {prompt.active_skills.length > 0 && (
        <div>
          <p className="text-xs text-[var(--v2-text-faint)]">Active skills</p>
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
          Some prompt content was safely truncated before display.
        </p>
      )}
      <div className="space-y-2">
        {prompt.components.map((component, index) => (
          <details key={`${component.label.content}-${index}`} className="rounded-xl border border-[var(--v2-panel-border)]">
            <summary className="cursor-pointer list-none px-3 py-2 text-xs font-medium text-[var(--v2-text-strong)]">
              <span>{component.label.content}</span>
              <span className="ml-2 font-normal text-[var(--v2-text-faint)]">
                {component.kind} · {formatNumber(component.estimated_tokens)} tokens
                {component.content.truncated ? " · truncated" : ""}
              </span>
            </summary>
            <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-words border-t border-[var(--v2-panel-border)] p-3 text-[11px] leading-5 text-[var(--v2-text-muted)]">
              {component.content.content}
            </pre>
          </details>
        ))}
      </div>
      <details className="rounded-xl border border-[var(--v2-panel-border)]">
        <summary className="cursor-pointer px-3 py-2 text-xs font-medium">Full reconstructed prompt</summary>
        <div className="border-t border-[var(--v2-panel-border)] p-3">
          <p className="mb-3 text-[11px] leading-5 text-[var(--v2-text-faint)]">
            Reconstructed content reflects the latest host prompt boundary and may differ from a specific historical model call.
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
  updateCount,
}: {
  snapshot: Record<string, unknown> | null;
  updateCount: number;
}) {
  const retained = Array.isArray(snapshot?.activity) ? snapshot.activity.length : 0;
  if (retained === 0 && updateCount === 0) {
    return (
      <EmptyTab
        title="No activity yet"
        description="Ordered model and tool activity will appear here as the run progresses."
      />
    );
  }
  return (
    <div className="space-y-3 p-4">
      <div className="rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
        <p className="text-xs uppercase tracking-wide text-[var(--v2-text-faint)]">Activity</p>
        <p className="mt-1 text-sm text-[var(--v2-text-strong)]">
          {retained} retained · {updateCount} live updates
        </p>
      </div>
    </div>
  );
}

function StatsShell({ snapshot }: { snapshot: Record<string, unknown> | null }) {
  const stats = snapshot?.stats as { total_model_calls?: unknown } | undefined;
  if (!stats) {
    return (
      <EmptyTab
        title="No statistics yet"
        description="Session totals will appear after the run records model or tool activity."
      />
    );
  }
  const totalCalls = typeof stats.total_model_calls === "number" ? stats.total_model_calls : 0;
  return (
    <div className="grid grid-cols-2 gap-3 p-4">
      <div className="rounded-xl border border-[var(--v2-panel-border)] p-3">
        <p className="text-xs text-[var(--v2-text-muted)]">Model calls</p>
        <p className="mt-1 text-xl font-semibold text-[var(--v2-text-strong)]">{totalCalls}</p>
      </div>
      <div className="rounded-xl border border-[var(--v2-panel-border)] p-3">
        <p className="text-xs text-[var(--v2-text-muted)]">Live updates</p>
        <p className="mt-1 text-xl font-semibold text-[var(--v2-text-strong)]">Available</p>
      </div>
    </div>
  );
}

function StatusNotice({ health, error }: { health: string; error: string | null }) {
  if (!error && health !== INSPECTOR_HEALTH.DISCONNECTED) return null;
  return (
    <div
      role="status"
      data-testid="inspector-status-notice"
      className="m-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-text-muted)]"
    >
      {error || "The diagnostics stream is disconnected. Chat remains available."}
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
  const viewportMode = useViewportMode();
  const [preferences, setPreferences] = React.useState<InspectorPreferences>(() =>
    readInspectorPreferences(),
  );
  const inspector = useInspector({
    enabled: preferences.open && viewportMode !== "mobile",
    threadId,
    runId,
  });

  const updatePreferences = React.useCallback((next: InspectorPreferences) => {
    setPreferences(next);
    writeInspectorPreferences(next);
  }, []);
  const setActiveTab = (activeTab: InspectorTab) =>
    updatePreferences({ ...preferences, activeTab });
  const setOpen = (open: boolean) => updatePreferences({ ...preferences, open });

  if (viewportMode === "mobile") return null;
  if (!preferences.open) {
    return (
      <button
        type="button"
        data-testid="inspector-open"
        onClick={() => setOpen(true)}
        className="fixed bottom-5 right-5 z-40 hidden rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-4 py-2 text-xs font-semibold text-[var(--v2-text-strong)] shadow-lg sm:block"
      >
        Open Inspector
      </button>
    );
  }

  const snapshot = inspector.snapshot as Record<string, unknown> | null;
  return (
    <aside
      aria-label="Web Debug Inspector"
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
              Web Debug Inspector
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
              <span data-testid="inspector-health">{HEALTH_LABELS[inspector.health]}</span>
            </div>
          </div>
          <button
            type="button"
            aria-label="Close inspector"
            data-testid="inspector-close"
            onClick={() => setOpen(false)}
            className="rounded-lg px-2 py-1 text-lg leading-none text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)]"
          >
            ×
          </button>
        </div>
        <p className="mt-2 truncate font-mono text-[11px] text-[var(--v2-text-faint)]">
          {threadId && runId ? `${threadId} · ${runId}` : "Waiting for an active run"}
        </p>
      </header>

      <nav aria-label="Inspector tabs" className="flex border-b border-[var(--v2-panel-border)] px-2">
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
            {tab}
          </button>
        ))}
      </nav>

      <StatusNotice health={inspector.health} error={inspector.error} />
      <section role="tabpanel" className="min-h-0 flex-1 overflow-y-auto">
        {preferences.activeTab === "prompt" && (
          <PromptShell snapshot={snapshot} health={inspector.health} />
        )}
        {preferences.activeTab === "activity" && (
          <ActivityShell snapshot={snapshot} updateCount={inspector.updates.length} />
        )}
        {preferences.activeTab === "stats" && <StatsShell snapshot={snapshot} />}
      </section>
    </aside>
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
