import {
  EmptyPanel,
  Card,
  SearchInput,
  SectionHeader,
  Select,
  Toolbar,
} from "@ironclaw/ui";
import { useT } from "../../../lib/i18n";
import { RoutineRow } from "./routine-row";

const FILTERS = [
  { value: "all", label: "All routines" },
  { value: "enabled", label: "Enabled" },
  { value: "disabled", label: "Disabled" },
  { value: "unverified", label: "Unverified" },
  { value: "failing", label: "Failing" },
];

export function RoutinesList({
  routines,
  totalRoutines,
  selectedRoutineId,
  search,
  onSearchChange,
  statusFilter,
  onStatusFilterChange,
  onSelectRoutine,
  onTriggerRoutine,
  onToggleRoutine,
  isBusy,
  isRefreshing,
}) {
  const t = useT();

  if (!routines.length) {
    const hasFilters = Boolean(search.trim()) || statusFilter !== "all";
    return (
      <EmptyPanel
        title={totalRoutines && hasFilters ? "No routines match" : "No routines yet"}
        description={totalRoutines && hasFilters
          ? "Adjust the search or status filter to find a saved routine."
          : "Routines created from chat will appear here after they are saved."}
      />
    );
  }

  return (
    <div className="space-y-5">
      <Card className="p-4 sm:p-5">
        <SectionHeader
          eyebrow={t("routines.explorer")}
          title={t("routines.title")}
          description={t("routines.description")}
          actions={
            <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]">
              <span>{routines.length} visible</span>
              <span>/</span>
              <span>{isRefreshing ? "refreshing" : "live"}</span>
            </div>
          }
        />

        <Toolbar className="mt-5">
          <SearchInput
            label="Search routine name, trigger, or action"
            value={search}
            onChange={(event) => onSearchChange(event.currentTarget.value)}
            onClear={() => onSearchChange("")}
            clearLabel={t("routines.clearSearch")}
            placeholder="Search routine name, trigger, or action"
            className="md:flex-1"
          />
          <div className="md:w-[220px]">
            <Select
              size="sm"
              value={statusFilter}
              onChange={(event) => onStatusFilterChange(event.currentTarget.value)}
              aria-label={t("routines.filterLabel")}
            >
              {FILTERS.map((filter) => (<option key={filter.value} value={filter.value}>{filter.label}</option>))}
            </Select>
          </div>
        </Toolbar>
      </Card>

      <div className="grid gap-3">
        {routines.map(
          (routine) => (
            <RoutineRow
              key={routine.id}
              routine={routine}
              selectedRoutineId={selectedRoutineId}
              onSelectRoutine={onSelectRoutine}
              onTriggerRoutine={onTriggerRoutine}
              onToggleRoutine={onToggleRoutine}
              isBusy={isBusy}
            />
          )
        )}
      </div>
    </div>
  );
}
