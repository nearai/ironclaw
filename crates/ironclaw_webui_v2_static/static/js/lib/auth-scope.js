/* Authenticated-user scope for namespacing per-session client state.
 *
 * The WebChat v2 SPA supports bearer-authenticated multi-user sessions in a
 * single browser tab. Client-side caches that hold user content — the
 * in-memory thread-history cache and the localStorage composer drafts — must
 * be keyed by the authenticated identity so one user's content can never
 * surface for another after a session change. This covers changes that do
 * NOT go through an explicit sign-out (token swap, re-auth, 401 expiry):
 * the scope tracks the resolved session, so a different identity reads under
 * a different key and simply misses the previous user's entries.
 *
 * Set from the auth layer whenever the session is (re)resolved; defaults to
 * "anon" before/without a session.
 */

let currentScope = "anon";

/** Update the active scope from the resolved session (or null to reset). */
export function setAuthScope(session) {
  currentScope =
    session && session.tenant_id && session.user_id
      ? `${session.tenant_id}:${session.user_id}`
      : "anon";
}

/** The active scope string, e.g. `"<tenant_id>:<user_id>"` or `"anon"`. */
export function authScope() {
  return currentScope;
}
