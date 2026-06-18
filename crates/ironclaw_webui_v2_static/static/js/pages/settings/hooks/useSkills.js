import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchSkillContent,
  fetchSkills,
  installSkill as installSkillRequest,
  removeSkill as removeSkillRequest,
  updateSkill as updateSkillRequest,
} from "../lib/settings-api.js";

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

  const skills = query.data?.skills || [];

  return {
    skills,
    query,
    fetchSkillContent,
    installSkill: installMutation.mutateAsync,
    removeSkill: removeMutation.mutateAsync,
    updateSkill: updateMutation.mutateAsync,
    isInstalling: installMutation.isPending,
    isRemoving: removeMutation.isPending,
    isUpdating: updateMutation.isPending,
  };
}
