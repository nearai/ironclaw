import React from "react";

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
    listChatCommands()
      .then((response) => {
        cachedCommands = response?.commands || [];
        if (!cancelled) setCommands(cachedCommands);
      })
      .catch(() => {
        // Inventory unavailable: degrade to plain messaging.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return commands;
}
