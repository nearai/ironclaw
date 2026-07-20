import React from "react";
import { sortRoutines } from "../lib/routines-presenters";

export function useRoutineFilters(routines) {
  const [search, setSearch] = React.useState("");
  const [statusFilter, setStatusFilter] = React.useState("all");

  const filteredRoutines = React.useMemo(() => {
    const query = search.trim().toLowerCase();
    return sortRoutines(routines).filter((routine) => {
      const haystack = [
        routine.name,
        routine.description,
        routine.trigger_summary,
        routine.trigger_type,
        routine.action_type,
        routine.status,
      ]
        .join(" ")
        .toLowerCase();
      const matchesSearch = !query || haystack.includes(query);
      const matchesStatus =
        statusFilter === "all" ||
        (statusFilter === "enabled" && routine.enabled) ||
        (statusFilter === "disabled" && !routine.enabled) ||
        (statusFilter === "unverified" &&
          routine.verification_status === "unverified") ||
        (statusFilter === "failing" && routine.status === "failing");
      return matchesSearch && matchesStatus;
    });
  }, [routines, search, statusFilter]);

  return {
    filteredRoutines,
    search,
    setSearch,
    statusFilter,
    setStatusFilter,
  };
}
