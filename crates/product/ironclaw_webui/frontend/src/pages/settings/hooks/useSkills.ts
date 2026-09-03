import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchSkillContent,
  fetchSkills,
  installSkill as installSkillRequest,
  removeSkill as removeSkillRequest,
  setAutoActivateLearned as setAutoActivateLearnedRequest,
  setSkillAutoActivate as setSkillAutoActivateRequest,
  updateSkill as updateSkillRequest,
} from "../lib/settings-api";

type SkillMutationResult = { success?: boolean; message?: string };
type SkillInstallVariables = { name: string; content?: string };
type SkillUpdateVariables = { name: string; content: string };
type SkillAutoActivateVariables = { name: string; enabled: boolean };
type SkillsSnapshot = {
  auto_activate_learned?: boolean;
  [key: string]: unknown;
};

export function useSkills() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["skills"],
    queryFn: fetchSkills,
  });

  const installMutation = useMutation<
    SkillMutationResult,
    Error,
    SkillInstallVariables
  >({
    mutationFn: installSkillRequest,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  const removeMutation = useMutation<SkillMutationResult, Error, string>({
    mutationFn: removeSkillRequest,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  const updateMutation = useMutation<
    SkillMutationResult,
    Error,
    SkillUpdateVariables
  >({
    mutationFn: ({ name, content }) => updateSkillRequest(name, { content }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  const autoActivateMutation = useMutation<
    SkillMutationResult,
    Error,
    SkillAutoActivateVariables
  >({
    mutationFn: ({ name, enabled }) => setSkillAutoActivateRequest(name, enabled),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  const learnedAutoActivateMutation = useMutation<
    SkillMutationResult,
    Error,
    boolean
  >({
    mutationFn: (enabled) => setAutoActivateLearnedRequest(enabled),
    onSuccess: (_response, enabled) => {
      queryClient.setQueryData<SkillsSnapshot>(["skills"], (current) => {
        if (!current) return current;
        return {
          ...current,
          auto_activate_learned: enabled,
        };
      });
      // Keep the active view on the mutation-confirmed value while marking the
      // cached list stale for the next normal refresh.
      queryClient.invalidateQueries({ queryKey: ["skills"], refetchType: "none" });
    },
  });

  const skills = query.data?.skills || [];
  // Default true so the switch reads "on" before the first load resolves and
  // for older backends that predate the flag.
  const autoActivateLearned = query.data?.auto_activate_learned !== false;

  return {
    skills,
    query,
    autoActivateLearned,
    fetchSkillContent,
    installSkill: installMutation.mutateAsync,
    removeSkill: removeMutation.mutateAsync,
    updateSkill: updateMutation.mutateAsync,
    setSkillAutoActivate: autoActivateMutation.mutateAsync,
    setAutoActivateLearned: learnedAutoActivateMutation.mutateAsync,
    isInstalling: installMutation.isPending,
    isRemoving: removeMutation.isPending,
    isUpdating: updateMutation.isPending,
    isSettingAutoActivate: autoActivateMutation.isPending,
    // The card renders before the initial skills request settles. Keep its
    // toggle disabled until that authoritative value arrives so an older GET
    // cannot overwrite a mutation-confirmed cache update.
    isSettingAutoActivateLearned:
      query.isLoading || learnedAutoActivateMutation.isPending,
  };
}
