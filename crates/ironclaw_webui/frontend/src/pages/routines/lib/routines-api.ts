// Routine endpoints depend on v1 `/api/routines/*`. TODO stubs.

export function fetchRoutines() {
  return Promise.resolve({ routines: [], todo: true });
}
export function fetchRoutinesSummary() {
  return Promise.resolve({ total: 0, active: 0, paused: 0, todo: true });
}
export function fetchRoutineDetail(_routineId) {
  return Promise.resolve(null);
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
