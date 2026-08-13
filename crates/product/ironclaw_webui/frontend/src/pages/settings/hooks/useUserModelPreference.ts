// @ts-nocheck
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchUserModelCatalog,
  fetchUserModelPreference,
  setUserModelPreference,
} from "../lib/settings-api";

const CATALOG_QUERY_KEY = ["user-model-catalog"];
const PREFERENCE_QUERY_KEY = ["user-model-preference"];

export function useUserModelPreference() {
  const queryClient = useQueryClient();
  const catalogQuery = useQuery({
    queryKey: CATALOG_QUERY_KEY,
    queryFn: fetchUserModelCatalog,
    staleTime: 60_000,
  });
  const preferenceQuery = useQuery({
    queryKey: PREFERENCE_QUERY_KEY,
    queryFn: fetchUserModelPreference,
    staleTime: 60_000,
  });
  const mutation = useMutation({
    mutationFn: setUserModelPreference,
    onSuccess: (preference) => {
      queryClient.setQueryData(PREFERENCE_QUERY_KEY, preference);
    },
  });

  return {
    catalog: catalogQuery.data || {
      selection_enabled: false,
      workspace_default: null,
      models: [],
    },
    model: preferenceQuery.data?.model || null,
    isLoading: catalogQuery.isLoading || preferenceQuery.isLoading,
    isSaving: mutation.isPending,
    catalogReadFailed: catalogQuery.isError,
    preferenceReadFailed: preferenceQuery.isError,
    saveError: mutation.error || null,
    setModel: (model) => mutation.mutate(model || null),
  };
}
