/* Canonical helpers for talking about a thread's activity in the UI.
 *
 * `byActivityDesc` is the project-wide thread ordering. Today the sidebar's
 * Recent group is the only consumer; future surfaces (resume cards, palette,
 * etc.) should reuse it so threads sort consistently everywhere.
 *
 * `formatThreadActivityLabel` and `formatThreadActivityTooltip` are the
 * project-wide thread time-label functions. Anything that renders "when did
 * this thread last move" should go through here so the wording stays
 * consistent.
 */

/** ISO timestamp the UI considers most recent for sorting/labelling. */
export function threadActivityIso(thread) {
  return thread.updated_at || thread.created_at || null;
}

/**
 * Comparator: newest-first by updated_at || created_at.
 *
 * Lexicographic ISO compare is chronological for the timestamp formats
 * both DB backends emit (RFC3339 + libSQL CURRENT_TIMESTAMP). Tie-breaks
 * on id for a stable sort.
 */
export function byActivityDesc(a, b) {
  const aIso = threadActivityIso(a) || "";
  const bIso = threadActivityIso(b) || "";
  if (aIso === bIso) return (a.id || "").localeCompare(b.id || "");
  return bIso.localeCompare(aIso);
}

/* Naming note: a separate `formatRelativeTime` lives in
 * pages/admin/lib/admin-presenters.ts with completely different semantics
 * ("5m ago" duration vs this file's "HH:MM | Mon D" stamp). The exports
 * here are deliberately namespaced to make the difference obvious to a
 * grep. A future canonical lib/time.ts could consolidate both, but that
 * refactor is out of scope here. */

/** Sidebar row label: HH:MM if today, otherwise "Mon D". */
export function formatThreadActivityLabel(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  const now = new Date();
  if (d.toDateString() === now.toDateString())
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

/** Tooltip-friendly absolute stamp: "Mon D, HH:MM". */
export function formatThreadActivityTooltip(iso) {
  if (!iso) return "";
  return new Date(iso).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
