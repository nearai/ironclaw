import { apiFetch } from "../../../lib/api.js";

function withPathParam(path, value) {
  return path.replace(":routineId", encodeURIComponent(value));
}

export function fetchRoutines() {
  return apiFetch("/api/routines");
}

export function fetchRoutinesSummary() {
  return apiFetch("/api/routines/summary");
}

export function fetchRoutineDetail(routineId) {
  return apiFetch(withPathParam("/api/routines/:routineId", routineId));
}

export function triggerRoutine(routineId) {
  return apiFetch(withPathParam("/api/routines/:routineId/trigger", routineId), {
    method: "POST",
  });
}

export function toggleRoutine(routineId) {
  return apiFetch(withPathParam("/api/routines/:routineId/toggle", routineId), {
    method: "POST",
  });
}

export function deleteRoutine(routineId) {
  return apiFetch(withPathParam("/api/routines/:routineId", routineId), {
    method: "DELETE",
  });
}
