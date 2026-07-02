import { React } from "../../../lib/html.js";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { fetchUsers, createUser, updateUser } from "../lib/settings-api.js";

export function useUsers() {
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: ["admin-users"],
    queryFn: fetchUsers,
    retry: false,
  });

  const users = query.data?.users || [];
  const isForbidden = query.error?.message?.includes("403") || query.error?.message?.includes("Forbidden");

  const createMutation = useMutation({
    mutationFn: createUser,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["admin-users"] }),
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, payload }) => updateUser(id, payload),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["admin-users"] }),
  });

  return {
    users,
    query,
    isForbidden,
    createUser: createMutation.mutate,
    updateUser: (id, payload) => updateMutation.mutate({ id, payload }),
    createError: createMutation.error,
    isCreating: createMutation.isPending,
  };
}
