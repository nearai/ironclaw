// @ts-nocheck
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchExtensionAdminConfiguration,
  replaceExtensionAdminConfiguration,
} from "../lib/admin-api";

const queryKey = ["admin", "extension-configuration"];

export function useAdminConfiguration() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey,
    queryFn: fetchExtensionAdminConfiguration,
  });
  const mutation = useMutation({
    mutationFn: ({ groupId, values }) =>
      replaceExtensionAdminConfiguration(groupId, values),
    onSuccess: (saved) => {
      queryClient.setQueryData(queryKey, (groups = []) =>
        groups.map((group) => group.group_id === saved.group_id ? saved : group),
      );
    },
  });
  return {
    groups: query.data || [],
    query,
    save: mutation.mutateAsync,
    isSaving: mutation.isPending,
    savingGroupId: mutation.variables?.groupId,
    saveError: mutation.error,
    resetSave: mutation.reset,
  };
}
