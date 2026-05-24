import { React } from "../../../lib/html.js";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { fetchTools, updateToolPermission } from "../lib/settings-api.js";

export function useTools() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["settings-tools"],
    queryFn: fetchTools,
  });

  const tools = query.data?.tools || [];

  const [savedTools, setSavedTools] = React.useState({});

  const mutation = useMutation({
    mutationFn: ({ name, state }) => updateToolPermission(name, state),
    onSuccess: (_data, { name, state }) => {
      queryClient.setQueryData(["settings-tools"], (old) => {
        if (!old) return old;
        return {
          ...old,
          tools: old.tools.map((t) => (t.name === name ? { ...t, state } : t)),
        };
      });
      setSavedTools((prev) => ({ ...prev, [name]: true }));
      setTimeout(() => setSavedTools((prev) => ({ ...prev, [name]: false })), 2000);
    },
  });

  const setPermission = React.useCallback(
    (name, state) => mutation.mutate({ name, state }),
    [mutation]
  );

  return { tools, query, setPermission, savedTools, error: mutation.error };
}
