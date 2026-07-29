// Routine endpoints depend on v1 `/api/routines/*`. TODO stubs.
//
// The payloads route through `demoJson` so DEMO-mode staging builds can serve
// fixtures from these same paths; production keeps the empty fallbacks (the
// flag is a build-time constant).

import { demoJson } from "../../../demo/bridge";

export function fetchRoutines() {
  return demoJson("/api/routines", { routines: [], todo: true });
}
export function fetchRoutinesSummary() {
  return demoJson("/api/routines/summary", { total: 0, active: 0, paused: 0, todo: true });
}
export function fetchRoutineDetail(routineId) {
  return demoJson(`/api/routines/${encodeURIComponent(routineId)}`, null);
}
export function triggerRoutine(_routineId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 routines endpoint" });
}
export function toggleRoutine(_routineId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 routines endpoint" });
}
export function deleteRoutine(_routineId) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 routines endpoint" });
}
