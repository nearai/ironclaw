import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useApiClient, type ApiClient } from "@/app";

export function projectsQueryOptions(apiClient: ApiClient) {
  return {
    queryKey: ["projects"] as const,
    queryFn: async () => {
      const result = await apiClient.ironclaw.projects.list();
      return result.projects;
    },
    staleTime: 30_000,
  } as const;
}

export function projectQueryOptions(apiClient: ApiClient, projectId: string) {
  return {
    queryKey: ["projects", projectId] as const,
    queryFn: async () => {
      const result = await apiClient.ironclaw.projects.get({ id: projectId });
      return result;
    },
    staleTime: 30_000,
  } as const;
}

export function projectMembersQueryOptions(apiClient: ApiClient, projectId: string) {
  return {
    queryKey: ["projects", projectId, "members"] as const,
    queryFn: async () => {
      const result = await apiClient.ironclaw.projects.listMembers({ id: projectId });
      return result.members;
    },
    staleTime: 30_000,
  } as const;
}

export function useProjects() {
  const apiClient = useApiClient();
  return useQuery(projectsQueryOptions(apiClient));
}

export function useProject(projectId: string) {
  const apiClient = useApiClient();
  return useQuery(projectQueryOptions(apiClient, projectId));
}

export function useProjectMembers(projectId: string) {
  const apiClient = useApiClient();
  return useQuery(projectMembersQueryOptions(apiClient, projectId));
}

export function useCreateProject() {
  const apiClient = useApiClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: { name: string; description?: string }) => {
      return apiClient.ironclaw.projects.create(input);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}

export function useDeleteProject() {
  const apiClient = useApiClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (projectId: string) => {
      await apiClient.ironclaw.projects.delete({ id: projectId });
    },
    onSuccess: (_data, projectId) => {
      queryClient.invalidateQueries({ queryKey: ["projects"] });
      queryClient.invalidateQueries({ queryKey: ["projects", projectId] });
      queryClient.invalidateQueries({ queryKey: ["projects", projectId, "members"] });
    },
  });
}

export function useAddProjectMember(projectId: string) {
  const apiClient = useApiClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: { userId: string; role: string }) => {
      return apiClient.ironclaw.projects.addMember({ id: projectId, ...input });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["projects", projectId, "members"] });
    },
  });
}

export function useUpdateProjectMember(projectId: string) {
  const apiClient = useApiClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: { userId: string; role: string }) => {
      return apiClient.ironclaw.projects.updateMember({ id: projectId, ...input });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["projects", projectId, "members"] });
    },
  });
}

export function useRemoveProjectMember(projectId: string) {
  const apiClient = useApiClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (userId: string) => {
      await apiClient.ironclaw.projects.removeMember({ id: projectId, userId });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["projects", projectId, "members"] });
    },
  });
}
