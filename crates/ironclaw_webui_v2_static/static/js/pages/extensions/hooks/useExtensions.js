import { React } from "../../../lib/html.js";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { gatewayStatus } from "../../../lib/api.js";
import {
  fetchExtensions,
  fetchExtensionRegistry,
  installExtension,
  activateExtension,
  removeExtension,
  fetchExtensionSetup,
  submitExtensionSetup,
  fetchPairingRequests,
  approvePairingCode,
} from "../lib/extensions-api.js";

export function useExtensions() {
  const queryClient = useQueryClient();

  const statusQuery = useQuery({
    queryKey: ["gateway-status-extensions"],
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

  const invalidate = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["extensions"] });
    queryClient.invalidateQueries({ queryKey: ["extension-registry"] });
    queryClient.invalidateQueries({ queryKey: ["gateway-status-extensions"] });
  }, [queryClient]);

  const [actionResult, setActionResult] = React.useState(null);

  const clearResult = React.useCallback(() => setActionResult(null), []);

  const installMutation = useMutation({
    mutationFn: ({ name, kind }) => installExtension(name, kind),
    onSuccess: (res, { displayName }) => {
      if (res.success) {
        setActionResult({ type: "success", message: `${displayName || "Extension"} installed` });
        if (res.auth_url) {
          window.open(res.auth_url, "_blank", "noopener,noreferrer");
        }
      } else {
        setActionResult({ type: "error", message: res.message || "Install failed" });
      }
      invalidate();
    },
    onError: (err) => {
      setActionResult({ type: "error", message: err.message });
      invalidate();
    },
  });

  const activateMutation = useMutation({
    mutationFn: ({ name }) => activateExtension(name),
    onSuccess: (res, { name }) => {
      if (res.success) {
        setActionResult({ type: "success", message: `${name} activated` });
        if (res.auth_url) {
          window.open(res.auth_url, "_blank", "noopener,noreferrer");
        }
      } else if (res.auth_url) {
        window.open(res.auth_url, "_blank", "noopener,noreferrer");
        setActionResult({ type: "info", message: "Opening authentication…" });
      } else if (res.awaiting_token) {
        setActionResult({ type: "info", message: "Configuration required" });
      } else {
        setActionResult({ type: "error", message: res.message || "Activation failed" });
      }
      invalidate();
    },
    onError: (err) => {
      setActionResult({ type: "error", message: err.message });
    },
  });

  const removeMutation = useMutation({
    mutationFn: ({ name }) => removeExtension(name),
    onSuccess: (res, { name }) => {
      if (res.success) {
        setActionResult({ type: "success", message: `${name} removed` });
      } else {
        setActionResult({ type: "error", message: res.message || "Remove failed" });
      }
      invalidate();
    },
    onError: (err) => {
      setActionResult({ type: "error", message: err.message });
    },
  });

  const status = statusQuery.data || {};
  const extensions = extensionsQuery.data?.extensions || [];
  const registry = registryQuery.data?.entries || [];

  const installedNames = new Set(extensions.map((e) => e.name));

  const channels = extensions.filter((e) => e.kind === "wasm_channel");
  const mcpServers = extensions.filter((e) => e.kind === "mcp_server");
  const tools = extensions.filter((e) => e.kind !== "wasm_channel" && e.kind !== "mcp_server");

  const channelRegistry = registry.filter((e) => (e.kind === "wasm_channel" || e.kind === "channel") && !installedNames.has(e.name));
  const mcpRegistry = registry.filter((e) => e.kind === "mcp_server" && !installedNames.has(e.name));
  const toolRegistry = registry.filter(
    (e) => e.kind !== "mcp_server" && e.kind !== "wasm_channel" && e.kind !== "channel" && !installedNames.has(e.name)
  );

  const isLoading = extensionsQuery.isLoading || registryQuery.isLoading;
  const isBusy = installMutation.isPending || activateMutation.isPending || removeMutation.isPending;

  return {
    status,
    extensions,
    channels,
    mcpServers,
    tools,
    channelRegistry,
    mcpRegistry,
    toolRegistry,
    registry,
    isLoading,
    isBusy,
    actionResult,
    clearResult,
    install: installMutation.mutate,
    activate: activateMutation.mutate,
    remove: removeMutation.mutate,
    invalidate,
  };
}

export function useExtensionSetup(name) {
  const query = useQuery({
    queryKey: ["extension-setup", name],
    queryFn: () => fetchExtensionSetup(name),
    enabled: Boolean(name),
  });

  return {
    secrets: query.data?.secrets || [],
    fields: query.data?.fields || [],
    onboarding: query.data?.onboarding || null,
    isLoading: query.isLoading,
    error: query.error,
  };
}

export function useSetupSubmit(name, onSuccess) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ secrets, fields }) => submitExtensionSetup(name, secrets, fields),
    onSuccess: (res) => {
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
      queryClient.invalidateQueries({ queryKey: ["extension-setup", name] });
      if (onSuccess) onSuccess(res);
    },
  });
}

export function usePairing(channel) {
  const query = useQuery({
    queryKey: ["pairing", channel],
    queryFn: () => fetchPairingRequests(channel),
    enabled: Boolean(channel),
    refetchInterval: 5000,
  });

  const queryClient = useQueryClient();

  const approveMutation = useMutation({
    mutationFn: ({ code }) => approvePairingCode(channel, code),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["pairing", channel] });
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
    },
  });

  return {
    requests: query.data?.requests || [],
    isLoading: query.isLoading,
    approve: approveMutation.mutate,
    isApproving: approveMutation.isPending,
  };
}
