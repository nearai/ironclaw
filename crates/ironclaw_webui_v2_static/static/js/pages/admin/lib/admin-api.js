// All admin endpoints depend on v1 `/api/admin/*`, which the v2
// ingress does not expose. Every function below is a TODO stub
// returning an empty shape so the page UI renders without hitting
// any network. Hard non-goal of issue #3886: no v1 paths in browser
// code. Remove this file's stub pattern when the v2 admin contract
// lands.

const ADMIN_TODO_PAYLOAD = Object.freeze({ todo: true });

export function fetchAdminUsers() {
  return Promise.resolve({ users: [], total: 0, ...ADMIN_TODO_PAYLOAD });
}
export function fetchAdminUser(_id) {
  return Promise.resolve(null);
}
export function createAdminUser(_payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 admin endpoint" });
}
export function updateAdminUser(_id, _payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 admin endpoint" });
}
export function deleteAdminUser(_id) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 admin endpoint" });
}
export function suspendAdminUser(_id) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 admin endpoint" });
}
export function activateAdminUser(_id) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 admin endpoint" });
}
export function createUserToken(_userId, _name) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 admin endpoint" });
}
export function fetchUsageSummary() {
  return Promise.resolve({
    total_users: 0,
    active_users: 0,
    suspended_users: 0,
    admin_users: 0,
    total_jobs: 0,
    llm_calls: 0,
    total_cost_usd: 0,
    active_jobs: 0,
    uptime_seconds: 0,
    recent_users: [],
    ...ADMIN_TODO_PAYLOAD,
  });
}
export function fetchUsage(_period = "day", _userId) {
  return Promise.resolve({ entries: [], ...ADMIN_TODO_PAYLOAD });
}
