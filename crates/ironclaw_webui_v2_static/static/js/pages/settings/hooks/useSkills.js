import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  approvePendingSkill as approvePendingSkillRequest,
  discardPendingSkill as discardPendingSkillRequest,
  fetchPendingSkills,
  fetchSkillContent,
  fetchSkills,
  installSkill as installSkillRequest,
  removeSkill as removeSkillRequest,
  setAutoActivateLearned as setAutoActivateLearnedRequest,
  setLearningEnabled as setLearningEnabledRequest,
  setRequireReview as setRequireReviewRequest,
  setSkillAutoActivate as setSkillAutoActivateRequest,
  updateSkill as updateSkillRequest,
} from "../lib/settings-api.js";

export function useSkills() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["skills"],
    queryFn: fetchSkills,
  });
  const pendingQuery = useQuery({
    queryKey: ["skills", "pending"],
    queryFn: fetchPendingSkills,
  });

  const invalidateSkills = () => {
    queryClient.invalidateQueries({ queryKey: ["skills"] });
  };
  // Approve/discard change BOTH the skill list and the pending set, so refresh
  // both. The list query key is a prefix of the pending key, so invalidating
  // ["skills"] also covers ["skills", "pending"], but spell it out for clarity.
  const invalidateSkillsAndPending = () => {
    queryClient.invalidateQueries({ queryKey: ["skills"] });
    queryClient.invalidateQueries({ queryKey: ["skills", "pending"] });
  };

  const installMutation = useMutation({
    mutationFn: installSkillRequest,
    onSuccess: invalidateSkills,
  });

  const removeMutation = useMutation({
    mutationFn: removeSkillRequest,
    onSuccess: invalidateSkills,
  });

  const updateMutation = useMutation({
    mutationFn: ({ name, content }) => updateSkillRequest(name, { content }),
    onSuccess: invalidateSkillsAndPending,
  });

  const autoActivateMutation = useMutation({
    mutationFn: ({ name, enabled }) => setSkillAutoActivateRequest(name, enabled),
    onSuccess: invalidateSkills,
  });

  const learnedAutoActivateMutation = useMutation({
    mutationFn: (enabled) => setAutoActivateLearnedRequest(enabled),
    onSuccess: invalidateSkills,
  });

  const learningEnabledMutation = useMutation({
    mutationFn: (enabled) => setLearningEnabledRequest(enabled),
    onSuccess: invalidateSkills,
  });

  const requireReviewMutation = useMutation({
    mutationFn: (enabled) => setRequireReviewRequest(enabled),
    onSuccess: invalidateSkills,
  });

  const approvePendingMutation = useMutation({
    mutationFn: (name) => approvePendingSkillRequest(name),
    onSuccess: invalidateSkillsAndPending,
  });

  const discardPendingMutation = useMutation({
    mutationFn: (name) => discardPendingSkillRequest(name),
    onSuccess: invalidateSkillsAndPending,
  });

  const skills = query.data?.skills || [];
  // Default true so the switch reads "on" before the first load resolves and
  // for older backends that predate the flag.
  const autoActivateLearned = query.data?.auto_activate_learned !== false;
  const learningEnabled = query.data?.learning_enabled !== false;
  // Fail closed: default hold-for-review ON until the backend says otherwise
  // (matches the server default), so the UI never shows new skills as
  // auto-applied before the first load resolves.
  const requireReview = query.data?.require_review !== false;
  const pendingSkills = pendingQuery.data?.pending || [];

  return {
    skills,
    query,
    autoActivateLearned,
    learningEnabled,
    requireReview,
    pendingSkills,
    pendingQuery,
    fetchSkillContent,
    installSkill: installMutation.mutateAsync,
    removeSkill: removeMutation.mutateAsync,
    updateSkill: updateMutation.mutateAsync,
    setSkillAutoActivate: autoActivateMutation.mutateAsync,
    setAutoActivateLearned: learnedAutoActivateMutation.mutateAsync,
    setLearningEnabled: learningEnabledMutation.mutateAsync,
    setRequireReview: requireReviewMutation.mutateAsync,
    approvePendingSkill: approvePendingMutation.mutateAsync,
    discardPendingSkill: discardPendingMutation.mutateAsync,
    isInstalling: installMutation.isPending,
    isRemoving: removeMutation.isPending,
    isUpdating: updateMutation.isPending,
    isSettingAutoActivate: autoActivateMutation.isPending,
    isSettingAutoActivateLearned: learnedAutoActivateMutation.isPending,
    isSettingLearningEnabled: learningEnabledMutation.isPending,
    isSettingRequireReview: requireReviewMutation.isPending,
    isApprovingPending: approvePendingMutation.isPending,
    isDiscardingPending: discardPendingMutation.isPending,
  };
}
