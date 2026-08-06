// @ts-nocheck
import React from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { fetchTools, updateToolPermission } from "../lib/settings-api";
import { throwIfApiFailed } from "../lib/api-result";

export function useTools() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["settings-tools"],
    queryFn: fetchTools,
  });

  const tools = query.data?.tools || [];

  const [savedTools, setSavedTools] = React.useState({});
  const [pendingPermissions, setPendingPermissions] = React.useState({});
  const nextRequestId = React.useRef(0);
  const pendingRequestIds = React.useRef({});
  const savedTimeouts = React.useRef({});

  React.useEffect(() => {
    return () => {
      for (const timeoutId of Object.values(savedTimeouts.current)) {
        clearTimeout(timeoutId);
      }
      savedTimeouts.current = {};
    };
  }, []);

  const clearPendingPermission = React.useCallback((name, requestId) => {
    if (pendingRequestIds.current[name] !== requestId) return;
    delete pendingRequestIds.current[name];
    setPendingPermissions((prev) => {
      if (prev[name]?.requestId !== requestId) return prev;
      const next = { ...prev };
      delete next[name];
      return next;
    });
  }, []);

  const mutation = useMutation({
    // Treat `success: false` as a failed save so the UI never shows a fake
    // "Saved" indicator for a permission change that didn't persist.
    mutationFn: async ({ name, state }) =>
      throwIfApiFailed(await updateToolPermission(name, state), "Save failed"),
    onSuccess: (data, { name, state, requestId }) => {
      if (pendingRequestIds.current[name] !== requestId) {
        queryClient.invalidateQueries({ queryKey: ["settings-tools"] });
        return;
      }
      queryClient.setQueryData(["settings-tools"], (old) => {
        if (!old) return old;
        const updatedTool = data?.tool;
        return {
          ...old,
          tools: old.tools.map((t) =>
            t.name === name ? { ...t, state, ...(updatedTool || {}) } : t
          ),
        };
      });
      clearPendingPermission(name, requestId);
      setSavedTools((prev) => ({ ...prev, [name]: true }));
      if (savedTimeouts.current[name]) {
        clearTimeout(savedTimeouts.current[name]);
      }
      savedTimeouts.current[name] = setTimeout(() => {
        setSavedTools((prev) => ({ ...prev, [name]: false }));
        delete savedTimeouts.current[name];
      }, 2000);
    },
    onError: (_error, { name, requestId }) => {
      clearPendingPermission(name, requestId);
      queryClient.invalidateQueries({ queryKey: ["settings-tools"] });
    },
  });

  const setPermission = React.useCallback(
    (name, state) => {
      const requestId = nextRequestId.current + 1;
      nextRequestId.current = requestId;
      pendingRequestIds.current[name] = requestId;
      mutation.reset();
      setPendingPermissions((prev) => ({
        ...prev,
        [name]: { requestId, state },
      }));
      mutation.mutate({ name, state, requestId });
    },
    [mutation.mutate, mutation.reset]
  );

  return {
    tools,
    query,
    setPermission,
    savedTools,
    pendingPermissions,
    error: mutation.error,
  };
}
