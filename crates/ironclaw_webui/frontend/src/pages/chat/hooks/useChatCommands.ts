import React from "react";

import { authScope } from "../../../lib/auth-scope";
import { listChatCommands } from "../../../lib/api";

// The inventory is ROLE-FILTERED per caller (an admin sees a different
// command set than a member); fetch it once per resolved identity and share
// across mounts of this hook for that identity. Failure leaves the list
// empty — the composer intercept and menu simply stay off and slash text
// submits as an ordinary message.
let cachedCommands = null;

// Drop the cached inventory. Called from the auth identity-change purge
// effect (`app/auth.ts`, beside `clearHistoryCache()` / `clearAllDrafts()` /
// `clearAllPins()`) so an in-tab admin<->member swap never serves the
// previous identity's role-filtered command list. `RequireAuth`
// (`app/app.tsx`) renders `<AuthLoading />` while the new session resolves
// (`auth.isChecking`), unmounting the authenticated subtree — including
// whatever mounted this hook — and remounting it once the new session lands,
// so clearing here is enough for the next mount to refetch under the new
// identity instead of reading the stale cache.
export function clearChatCommandsCache() {
  cachedCommands = null;
}

export function useChatCommands() {
  const [commands, setCommands] = React.useState(cachedCommands || []);

  React.useEffect(() => {
    if (cachedCommands) return undefined;
    let cancelled = false;
    // Capture the issuing identity before the fetch (same idiom as
    // useHistory.ts's `issuingScope` / chat-input.tsx's `flushDraft` scope
    // check). The purge in the auth identity-change effect only helps when
    // it runs BEFORE this resolves; a previous identity's slow in-flight
    // fetch can still land after a swap-and-purge already remounted this
    // hook under a new identity. Without this guard that late response would
    // silently overwrite the new identity's already-correct cache with the
    // previous identity's role-filtered list — the purge can't catch this
    // because it already ran.
    const issuingScope = authScope();
    listChatCommands()
      .then((response) => {
        if (authScope() !== issuingScope) return;
        cachedCommands = response?.commands || [];
        if (!cancelled) setCommands(cachedCommands);
      })
      .catch(() => {
        // Inventory unavailable: degrade to plain messaging. Nothing is
        // written on this path, so no scope guard is needed here.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return commands;
}
