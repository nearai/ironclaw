import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchSkills,
  installSkill as installSkillRequest,
  removeSkill as removeSkillRequest,
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

  const skills = query.data?.skills || [];

  return {
    skills,
    query,
    installSkill: installMutation.mutateAsync,
    removeSkill: removeMutation.mutateAsync,
    isInstalling: installMutation.isPending,
    isRemoving: removeMutation.isPending,
  };
}
