import React from "react";

import { listChatCommands } from "../../../lib/api";

// The inventory is static per deployment; fetch it once per loaded app and
// share across mounts. Failure leaves the list empty — the composer intercept
// and menu simply stay off and slash text submits as an ordinary message.
let cachedCommands = null;

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
