/**
 * DesignPreviewPage — a DEV-only harness for the OOBE / automations design.
 *
 * It composes the *real* chat presentational components (EmptyState with the
 * automations carousel + mode pill, and a mock thread with the inline calendar
 * reschedule card) driven entirely by mock data, so the concepts can be seen
 * and clicked in `pnpm dev` with no backend running.
 *
 * This route is mounted only when `import.meta.env.DEV` is true (see app.tsx),
 * so it never ships in a production bundle. It is a design surface, not a
 * feature — the components it renders are the ones wired into the live Chat.
 */
import React from "react";
import { EmptyState } from "../chat/components/empty-state";
import { ChatInput } from "../chat/components/chat-input";
import { MessageList } from "../chat/components/message-list";
import { CalendarRescheduleCard } from "../chat/components/calendar-reschedule-card";
import { PlanCard } from "../chat/components/plan-card";
import { useAutomationTasks } from "../chat/hooks/useAutomationTasks";
import {
  MOCK_AUTOMATED_RESCHEDULE,
  MOCK_PLAN,
  MOCK_PLAN_TASKS,
  MOCK_SUGGESTED_RESCHEDULE,
} from "../chat/lib/automation-tasks";

const NOOP = () => {};
const NOOP_ASYNC = async () => null;

const MOCK_THREAD_MESSAGES = [
  {
    id: "demo-user-1",
    role: "user",
    content:
      "My Thursday afternoon is packed — can you clear the conflict on my calendar?",
    timestamp: "2026-07-23T14:02:00Z",
  },
  {
    id: "demo-assistant-1",
    role: "assistant",
    content:
      "I checked your calendar and found one overlap. Here's what I'd do — approve it, tweak the time, or skip it:",
    timestamp: "2026-07-23T14:02:04Z",
    isFinalReply: true,
  },
];

const MOCK_THREAD_MESSAGES_AUTO = [
  {
    id: "demo-assistant-2",
    role: "assistant",
    content:
      "Earlier today, while in Auto mode, I already moved another conflict for you. You can still adjust or undo it:",
    timestamp: "2026-07-23T13:10:00Z",
    isFinalReply: true,
  },
];

function Segmented({ value, onChange }) {
  const options = [
    { value: "landing", label: "Landing (carousel + mode)" },
    { value: "thread", label: "Thread (calendar preview)" },
    { value: "plan", label: "Plan (batched approval)" },
  ];
  return (
    <div className="inline-flex rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-1">
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          onClick={() => onChange(option.value)}
          className={[
            "rounded-full px-3 py-1.5 text-xs font-medium transition-colors",
            value === option.value
              ? "bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
              : "text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]",
          ].join(" ")}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function ThreadDemo() {
  // Seed must be stable across renders or the hook resets state every render
  // (its effect keys on the `initialTasks` reference).
  const seed = React.useMemo(
    () => [{ ...MOCK_SUGGESTED_RESCHEDULE }, { ...MOCK_AUTOMATED_RESCHEDULE }],
    [],
  );
  const tasks = useAutomationTasks({ initialTasks: seed });
  const suggested = tasks.tasks.find((t) => t.id === MOCK_SUGGESTED_RESCHEDULE.id);
  const automated = tasks.tasks.find((t) => t.id === MOCK_AUTOMATED_RESCHEDULE.id);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <MessageList
        messages={MOCK_THREAD_MESSAGES}
        isLoading={false}
        hasMore={false}
        onLoadMore={NOOP}
        onRetryMessage={NOOP}
        threadId="design-preview"
        activeRunId={null}
        logsPath={null}
        commands={[]}
      >
        {suggested && (
          <CalendarRescheduleCard
            task={suggested}
            busy={tasks.isBusy(suggested.id)}
            pendingAction={tasks.pendingAction(suggested.id)}
            onApprove={() => tasks.approve(suggested.id)}
            onModify={(patch) => tasks.modify(suggested.id, patch)}
            onCancel={() => tasks.cancel(suggested.id)}
            onRevert={() => tasks.revert(suggested.id)}
          />
        )}
        <div className="mx-auto w-full max-w-3xl space-y-4">
          {MOCK_THREAD_MESSAGES_AUTO.map((message) => (
            <div key={message.id} className="mr-auto px-1 text-iron-100">
              {message.content}
            </div>
          ))}
        </div>
        {automated && (
          <CalendarRescheduleCard
            task={automated}
            busy={tasks.isBusy(automated.id)}
            pendingAction={tasks.pendingAction(automated.id)}
            onApprove={() => tasks.approve(automated.id)}
            onModify={(patch) => tasks.modify(automated.id, patch)}
            onCancel={() => tasks.cancel(automated.id)}
            onRevert={() => tasks.revert(automated.id)}
          />
        )}
      </MessageList>
      <ChatInput
        onSend={NOOP_ASYNC}
        onCancel={NOOP}
        disabled={false}
        sendDisabled={false}
        draftKey="design-preview"
        variant="dock"
      />
    </div>
  );
}

function PlanDemo() {
  const seed = React.useMemo(
    () => MOCK_PLAN_TASKS.map((task) => ({ ...task })),
    [],
  );
  const automations = useAutomationTasks({ initialTasks: seed });
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-4 py-8">
      <div className="mx-auto w-full max-w-3xl">
        <p className="mb-4 px-1 text-iron-100">
          You're in Plan mode, so here's a batch I can run together — approve all,
          or adjust any first:
        </p>
        <PlanCard plan={MOCK_PLAN} automations={automations} />
      </div>
    </div>
  );
}

function LandingDemo() {
  return (
    <EmptyState
      onSuggestion={NOOP}
      onSend={NOOP_ASYNC}
      disabled={false}
      sendDisabled={false}
      initialText=""
      resetKey="design-preview"
      draftKey="design-preview-landing"
      context={{}}
      statusText=""
      canCancel={false}
      onCancel={NOOP}
    />
  );
}

export function DesignPreviewPage() {
  const [view, setView] = React.useState("landing");
  const [demoKey, setDemoKey] = React.useState(0);

  return (
    <div className="flex min-h-[100dvh] flex-col bg-[var(--v2-canvas)] text-[var(--v2-text-base)]">
      <header className="flex flex-wrap items-center gap-3 border-b border-[var(--v2-panel-border)] px-5 py-3">
        <span className="text-sm font-semibold text-[var(--v2-text-strong)]">
          IronClaw · OOBE design preview
        </span>
        <span className="rounded-full border border-[color-mix(in_srgb,var(--v2-warning-text)_40%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] px-2 py-0.5 text-[0.625rem] font-semibold uppercase tracking-[0.14em] text-[var(--v2-warning-text)]">
          mock data
        </span>
        <div className="ml-auto flex items-center gap-3">
          <Segmented value={view} onChange={setView} />
          <button
            type="button"
            onClick={() => setDemoKey((k) => k + 1)}
            className="rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
          >
            Reset demo
          </button>
        </div>
      </header>
      <main key={`${view}-${demoKey}`} className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {view === "landing" && <LandingDemo />}
        {view === "thread" && <ThreadDemo />}
        {view === "plan" && <PlanDemo />}
      </main>
    </div>
  );
}
