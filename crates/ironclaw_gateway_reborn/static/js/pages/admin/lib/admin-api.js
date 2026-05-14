import { apiFetch } from "../../../lib/api.js";

export function fetchAdminUsers() {
  return apiFetch("/api/admin/users");
}

export function fetchAdminUser(id) {
  return apiFetch(`/api/admin/users/${encodeURIComponent(id)}`);
}

export function createAdminUser(payload) {
  return apiFetch("/api/admin/users", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function updateAdminUser(id, payload) {
  return apiFetch(`/api/admin/users/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: JSON.stringify(payload),
  });
}

export function deleteAdminUser(id) {
  return apiFetch(`/api/admin/users/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
}

export function suspendAdminUser(id) {
  return apiFetch(`/api/admin/users/${encodeURIComponent(id)}/suspend`, {
    method: "POST",
  });
}

export function activateAdminUser(id) {
  return apiFetch(`/api/admin/users/${encodeURIComponent(id)}/activate`, {
    method: "POST",
  });
}

export function createUserToken(userId, name) {
  return apiFetch("/api/tokens", {
    method: "POST",
    body: JSON.stringify({ name, user_id: userId }),
  });
}

export function fetchUsageSummary() {
  return apiFetch("/api/admin/usage/summary");
}

export function fetchUsage(period = "day", userId) {
  const url = new URL("/api/admin/usage", window.location.origin);
  url.searchParams.set("period", period);
  if (userId) url.searchParams.set("user_id", userId);
  return apiFetch(url.pathname + url.search);
}
