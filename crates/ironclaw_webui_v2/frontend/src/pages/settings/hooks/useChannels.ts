import { useQuery } from "@tanstack/react-query";
import { fetchExtensions, fetchExtensionRegistry } from "../lib/settings-api";
import { hasChannelSurface } from "../../extensions/lib/extensions-schema";

// Channel discovery is extension-surface data (the wire carries runtime +
// surfaces; there is no extension `kind`). This settings view groups by the
// declared channel surface exactly like the extensions page; runtime is never
// a grouping axis, so there is no separate MCP rail here.
export function useChannels() {
  const extensionsQuery = useQuery({
    queryKey: ["extensions"],
    queryFn: fetchExtensions,
  });

  const registryQuery = useQuery({
    queryKey: ["extension-registry"],
    queryFn: fetchExtensionRegistry,
  });

  const extensions = extensionsQuery.data?.extensions || [];
  const registry = registryQuery.data?.entries || [];

  const channels = extensions.filter((extension) => hasChannelSurface(extension));
  const channelRegistry = registry.filter(
    (entry) => hasChannelSurface(entry) && !entry.installed
  );

  const isLoading = extensionsQuery.isLoading || registryQuery.isLoading;

  return { channels, channelRegistry, extensions, isLoading };
}
