import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchExtensionAdminConfiguration,
  replaceExtensionAdminConfiguration,
} from "../lib/admin-api";

const queryKey = ["admin", "extension-configuration"];

type ConfigurationField = {
  handle: string;
  label?: string;
  value?: string;
  description?: string;
  secret?: boolean;
  provided?: boolean;
  required?: boolean;
};

type ConfigurationGroup = {
  group_id: string;
  revision: number;
  display_name?: string;
  description?: string;
  complete?: boolean;
  fields: ConfigurationField[];
  used_by: Array<{
    package_id: string;
    display_name?: string;
    installed?: boolean;
  }>;
};

type SaveConfigurationVariables = {
  groupId: string;
  values: Array<{ handle: string; value: string }>;
  expectedRevision: number;
  idempotencyKey: string;
};

function apiErrorStatus(error: unknown): number | undefined {
  if (!error || typeof error !== "object") return undefined;
  const status = Reflect.get(error, "status");
  return typeof status === "number" ? status : undefined;
}

export function useAdminConfiguration() {
  const queryClient = useQueryClient();
  const query = useQuery<ConfigurationGroup[]>({
    queryKey,
    queryFn: fetchExtensionAdminConfiguration,
  });
  const mutation = useMutation<
    ConfigurationGroup,
    Error,
    SaveConfigurationVariables
  >({
    mutationFn: ({ groupId, values, expectedRevision, idempotencyKey }) =>
      replaceExtensionAdminConfiguration(
        groupId,
        values,
        expectedRevision,
        idempotencyKey,
      ),
    onSuccess: (saved) => {
      queryClient.setQueryData<ConfigurationGroup[]>(queryKey, (groups = []) =>
        groups.map((group) => group.group_id === saved.group_id ? saved : group),
      );
    },
    onError: (error) => {
      if (apiErrorStatus(error) === 409) {
        return queryClient.invalidateQueries({ queryKey });
      }
      return undefined;
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
