// @ts-nocheck
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

export function useSkills() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["skills"],
    queryFn: fetchSkills,
  });

  const installMutation = useMutation({
    mutationFn: installSkillRequest,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  const removeMutation = useMutation({
    mutationFn: removeSkillRequest,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({ name, content }) => updateSkillRequest(name, { content }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  const autoActivateMutation = useMutation({
    mutationFn: ({ name, enabled }) => setSkillAutoActivateRequest(name, enabled),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  const learnedAutoActivateMutation = useMutation({
    mutationFn: (enabled) => setAutoActivateLearnedRequest(enabled),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
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
    isSettingAutoActivateLearned: learnedAutoActivateMutation.isPending,
  };
}
