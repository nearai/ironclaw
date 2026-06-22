import { useQuery } from "@tanstack/react-query";
import { useApiClient, type ApiClient } from "@/app";

export function mountsQueryOptions(apiClient: ApiClient) {
  return {
    queryKey: ["fs", "mounts"] as const,
    queryFn: async () => {
      const result = await apiClient.ironclaw.fs.mounts();
      return result.mounts;
    },
    staleTime: 60_000,
  } as const;
}

export function fsListQueryOptions(apiClient: ApiClient, mount: string, path: string) {
  return {
    queryKey: ["fs", "list", mount, path] as const,
    queryFn: async () => {
      const result = await apiClient.ironclaw.fs.list({ mount, path });
      return result;
    },
    staleTime: 10_000,
  } as const;
}

export function fsContentQueryOptions(apiClient: ApiClient, mount: string, path: string) {
  return {
    queryKey: ["fs", "content", mount, path] as const,
    queryFn: async () => {
      const result = await apiClient.ironclaw.fs.content({ mount, path });
      return result;
    },
    staleTime: 60_000,
  } as const;
}

export function useFilesystemMounts() {
  const apiClient = useApiClient();
  return useQuery(mountsQueryOptions(apiClient));
}

export function useFsList(mount: string, path: string) {
  const apiClient = useApiClient();
  return useQuery(fsListQueryOptions(apiClient, mount, path));
}

export function useFsContent(mount: string, path: string) {
  const apiClient = useApiClient();
  return useQuery({
    ...fsContentQueryOptions(apiClient, mount, path),
    enabled: !!mount && !!path,
  });
}
