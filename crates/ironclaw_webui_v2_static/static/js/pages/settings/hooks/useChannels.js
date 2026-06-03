import { useQuery } from "@tanstack/react-query";
import { gatewayStatus } from "../../../lib/api.js";
import { fetchExtensions, fetchExtensionRegistry } from "../lib/settings-api.js";

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

  const channels = extensions.filter((e) => e.kind === "wasm_channel" || e.kind === "channel");
  const channelRegistry = registry.filter(
    (e) => (e.kind === "wasm_channel" || e.kind === "channel") && !e.installed
  );
  const mcpServers = extensions.filter((e) => e.kind === "mcp_server");
  const mcpRegistry = registry.filter((e) => e.kind === "mcp_server" && !e.installed);

  const isLoading = statusQuery.isLoading || extensionsQuery.isLoading;

  return { status, channels, channelRegistry, mcpServers, mcpRegistry, extensions, isLoading };
}
