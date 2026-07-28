import { useNavigate } from "react-router";
import { Button } from "../../../design-system/button";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives";
import { useT } from "../../../lib/i18n";
import {
  formatRoutineDate,
  routineStatusTone,
  summarizeRoutineAction,
  verificationTone,
} from "../lib/routines-presenters";
import { RoutineRecentRuns } from "./routine-recent-runs";

function MetaItem({ label, value }) {
  return (
    <div className="rounded-xl border border-iron-700 bg-iron-950/50 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        {label}
      </div>
      <div className="mt-2 min-w-0 break-words text-sm text-iron-100">
        {value || "—"}
      </div>
    </div>
  );
}

function JsonBlock({ title, value }) {
  return (
    <div>
      <h3 className="text-sm font-semibold text-iron-100">{title}</h3>
      <pre
        className="mt-3 max-h-72 overflow-auto rounded-xl border border-iron-700 bg-iron-950/70 p-4 text-xs leading-5 text-iron-200"
      >{JSON.stringify(value || {}, null, 2)}</pre>
    </div>
  );
}

export function RoutineDetailPanel({
  routine,
  isLoading,
  error,
  isBusy,
  onTriggerRoutine,
  onToggleRoutine,
  onDeleteRoutine,
}) {
  const navigate = useNavigate();
  const t = useT();

  if (isLoading) {
    return (
      <div className="space-y-4">
        {[1, 2, 3].map(
          (index) =>
            (<div key={index} className="v2-skeleton h-32 rounded-xl" />)
        )}
      </div>
    );
  }

  if (error || !routine) {
    return (
      <EmptyPanel
        title={t("routine.unavailable")}
        description={error?.message || t("routine.unavailableDesc")}
      />
    );
  }

  return (
    <Panel className="p-4 sm:p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="truncate text-2xl font-semibold tracking-tight text-iron-100">
              {routine.name}
            </h2>
            <StatusPill
              tone={routineStatusTone(routine.status, routine.enabled)}
              label={routine.enabled ? routine.status : "disabled"}
            />
            <StatusPill
              tone={verificationTone(routine.verification_status)}
              label={routine.verification_status || "unknown"}
            />
          </div>
          <p className="mt-2 max-w-3xl text-sm leading-6 text-iron-300">
            {routine.description || routine.trigger_summary || "No description"}
          </p>
        </div>

        <div className="flex flex-wrap gap-2">
          <Button variant="secondary" disabled={isBusy} onClick={onTriggerRoutine}>Run</Button>
          <Button variant="ghost" disabled={isBusy} onClick={onToggleRoutine}>
            {routine.enabled ? "Disable" : "Enable"}
          </Button>
          <Button variant="ghost" onClick={onDeleteRoutine}>Delete</Button>
        </div>
      </div>

      <div className="mt-5 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <MetaItem label="Trigger" value={routine.trigger_summary || routine.trigger_type} />
        <MetaItem label="Action" value={summarizeRoutineAction(routine.action)} />
        <MetaItem label="Next fire" value={formatRoutineDate(routine.next_fire_at)} />
        <MetaItem label="Last run" value={formatRoutineDate(routine.last_run_at)} />
        <MetaItem label="Run count" value={routine.run_count} />
        <MetaItem label="Failures" value={routine.consecutive_failures} />
        <MetaItem label="Created" value={formatRoutineDate(routine.created_at)} />
        <MetaItem label="Routine ID" value={routine.id} />
      </div>

      {routine.conversation_id &&
      (
        <div className="mt-5">
          <Button variant="secondary" onClick={() => navigate(`/chat/${routine.conversation_id}`)}>
            Open routine thread
          </Button>
        </div>
      )}

      <div className="mt-6 grid gap-6 xl:grid-cols-2">
        <JsonBlock title={t("routine.triggerPayload")} value={routine.trigger} />
        <JsonBlock title={t("routine.actionPayload")} value={routine.action} />
      </div>

      <div className="mt-6">
        <h3 className="mb-3 text-sm font-semibold text-iron-100">Recent runs</h3>
        <RoutineRecentRuns runs={routine.recent_runs} />
      </div>
    </Panel>
  );
}
