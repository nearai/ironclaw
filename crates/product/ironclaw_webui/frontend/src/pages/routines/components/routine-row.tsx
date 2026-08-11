import { Button } from "../../../design-system/button";
import { StatusPill } from "../../../design-system/primitives";
import {
  formatRoutineDate,
  routineStatusTone,
  verificationTone,
} from "../lib/routines-presenters";

export function RoutineRow({
  routine,
  selectedRoutineId,
  onSelectRoutine,
  onTriggerRoutine,
  onToggleRoutine,
  isBusy,
}) {
  const selected = selectedRoutineId === routine.id;

  return (
    <article
      className={[
        "group flex flex-col gap-4 rounded-[18px] border p-5",
        selected
          ? "border-signal/35 bg-signal/10"
          : "border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80",
      ].join(" ")}
    >
      <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
        <button onClick={() => onSelectRoutine(routine.id)} className="min-w-0 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-lg font-semibold text-iron-100">{routine.name}</h3>
            <StatusPill
              tone={routineStatusTone(routine.status, routine.enabled)}
              label={routine.enabled ? routine.status : "disabled"}
            />
            <StatusPill
              tone={verificationTone(routine.verification_status)}
              label={routine.verification_status || "unknown"}
            />
          </div>
          <p className="mt-2 line-clamp-2 text-sm leading-6 text-iron-300">
            {routine.description || routine.trigger_summary || "No description"}
          </p>
          <div className="mt-3 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
            <span>{routine.trigger_type}</span>
            <span>{routine.action_type}</span>
            <span>runs {routine.run_count || 0}</span>
            <span>next {formatRoutineDate(routine.next_fire_at)}</span>
          </div>
        </button>

        <div className="flex shrink-0 flex-wrap gap-2">
          <Button
            variant="secondary"
            className="h-9 px-3 text-xs"
            disabled={isBusy}
            onClick={() => onTriggerRoutine(routine.id)}
          >
            Run
          </Button>
          <Button
            variant="ghost"
            className="h-9 px-3 text-xs"
            disabled={isBusy}
            onClick={() => onToggleRoutine(routine.id)}
          >
            {routine.enabled ? "Disable" : "Enable"}
          </Button>
          <Button
            variant="ghost"
            className="h-9 px-3 text-xs"
            onClick={() => onSelectRoutine(routine.id)}
          >
            Open
          </Button>
        </div>
      </div>
    </article>
  );
}
