import { useQuery } from "@tanstack/react-query";
import { gatewayStatus } from "../../../lib/api";
import { fetchExtensions, fetchExtensionRegistry } from "../lib/settings-api";
import { hasChannelSurface } from "../../extensions/lib/extensions-schema";

// Channel discovery is extension-surface data (the wire carries runtime +
// surfaces; there is no extension `kind`). This settings view groups by the
// declared channel surface exactly like the extensions page; runtime is never
// a grouping axis, so there is no separate MCP rail here.
export function useChannels() {
  const statusQuery = useQuery({
    queryKey: ["gateway-status-settings"],
    queryFn: gatewayStatus,
    staleTime: 10_000,
  });

  const extensionsQuery = useQuery({
    queryKey: ["extensions"],
    queryFn: fetchExtensions,
  });

  const registryQuery = useQuery({
    queryKey: ["extension-registry"],
    queryFn: fetchExtensionRegistry,
  });

  const status = statusQuery.data || {};
  const extensions = extensionsQuery.data?.extensions || [];
  const registry = registryQuery.data?.entries || [];

  const channels = extensions.filter((extension) => hasChannelSurface(extension));
  const channelRegistry = registry.filter(
    (entry) => hasChannelSurface(entry) && !entry.installed
  );

  const isLoading = statusQuery.isLoading || extensionsQuery.isLoading;

  return { status, channels, channelRegistry, extensions, isLoading };
}
