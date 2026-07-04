import { React } from "../../../lib/html.js";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { gatewayStatus } from "../../../lib/api.js";
import { listConnectableChannels } from "../../../lib/channel-connect.js";
import { useT } from "../../../lib/i18n.js";
import { isChannelExtensionKind } from "../lib/extensions-schema.js";
import {
  fetchExtensions,
  fetchExtensionRegistry,
  installExtension,
  activateExtension,
  removeExtension,
  fetchExtensionSetup,
  submitExtensionSetup,
  startExtensionOauth,
  fetchPairingRequests,
  approvePairingCode,
} from "../lib/extensions-api.js";

const OAUTH_SETUP_REFRESH_MS = 2000;
const OAUTH_SETUP_TIMEOUT_MS = 10 * 60 * 1000;
const OAUTH_CALLBACK_CHANNEL = "ironclaw-product-auth";
const OAUTH_CALLBACK_STORAGE_KEY = "ironclaw:product-auth:oauth-complete";
const OAUTH_CALLBACK_MESSAGE_TYPE = "ironclaw:product-auth:oauth-complete";

function isHttpsAuthUrl(url) {
  try {
    return new URL(url).protocol === "https:";
  } catch (_) {
    return false;
  }
}

function openAuthUrl(url, popup = null) {
  if (!isHttpsAuthUrl(url)) return { ok: false, popup: null };
  if (popup && !popup.closed) {
    popup.location.href = url;
    return { ok: true, popup };
  }
  const opened = window.open(url, "_blank", "noopener,noreferrer");
  return {
    ok: Boolean(opened),
    popup: opened || null,
    reason: opened ? null : "popup_blocked",
  };
}

function parseOAuthCallbackStoragePayload(value) {
  if (!value) return null;
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}

function isOAuthCallbackCompletion(payload) {
  return payload?.type === OAUTH_CALLBACK_MESSAGE_TYPE && payload?.status === "completed";
}

function oauthCompletionMatchesFlow(payload, flowId) {
  if (!isOAuthCallbackCompletion(payload)) return false;
  // Require a matching flow id. A completion carrying no flow id — or one from a
  // DIFFERENT extension's flow in another tab — must NOT satisfy this modal;
  // treating a missing flow id as a match let a stale cross-tab callback
  // prematurely activate/close the modal. When the OAuth response carried no
  // flow id, this callback fast-path stays disabled and the polling path
  // (`setupIsConfigured`) is the sole completion signal. Mirrors the stricter
  // in-chat gate in useChat.js, which also keys on the flow id.
  if (!flowId) return false;
  return payload.flowId === flowId || payload.flow_id === flowId;
}

function oauthResponseFlowId(response) {
  return response?.flow_id || response?.flowId || null;
}

function extensionListItemIsConfigured(extension) {
  if (!extension) return false;
  if (extension.needs_setup === false && (extension.authenticated || extension.active)) {
    return true;
  }
  const state =
    extension.onboarding_state ||
    extension.activation_status ||
    (extension.active ? "active" : null);
  return (state === "active" || state === "ready") && extension.needs_setup !== true;
}

function packageId(item) {
  return item?.package_ref?.id || null;
}

function displayName(item) {
  return item?.display_name || packageId(item) || "";
}

function catalogId(prefix, item, index) {
  return packageId(item) || `${prefix}:${displayName(item) || "unknown"}:${index}`;
}

function catalogSort(a, b) {
  if (a.installed !== b.installed) return a.installed ? -1 : 1;
  return displayName(a.entry || a.extension).localeCompare(
    displayName(b.entry || b.extension)
  );
}

export function useExtensions() {
  const t = useT();
  const queryClient = useQueryClient();

  const statusQuery = useQuery({
    queryKey: ["gateway-status-extensions"],
    queryFn: gatewayStatus,
    staleTime: 10_000,
  });

  const extensionsQuery = useQuery({
    queryKey: ["extensions"],
    queryFn: fetchExtensions,
    refetchOnMount: "always",
  });

  const registryQuery = useQuery({
    queryKey: ["extension-registry"],
    queryFn: fetchExtensionRegistry,
    refetchOnMount: "always",
  });

  const connectableChannelsQuery = useQuery({
    queryKey: ["connectable-channels"],
    queryFn: listConnectableChannels,
    refetchOnMount: "always",
  });

  const invalidate = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["extensions"] });
    queryClient.invalidateQueries({ queryKey: ["extension-registry"] });
    queryClient.invalidateQueries({ queryKey: ["gateway-status-extensions"] });
    queryClient.invalidateQueries({ queryKey: ["connectable-channels"] });
  }, [queryClient]);

  const [actionResult, setActionResult] = React.useState(null);

  const clearResult = React.useCallback(() => setActionResult(null), []);

  const installMutation = useMutation({
    mutationFn: ({ packageRef }) => installExtension(packageRef),
    onSuccess: (res, { displayName, kind, configureAfterInstall, onNeedsSetup, packageRef }) => {
      if (res.success) {
        const message = isChannelExtensionKind(kind)
          ? `${displayName || "Channel"} installed. Use Configure to connect the account.`
          : res.message ||
            res.instructions ||
            t("extensions.installedSuccess", {
              name: displayName || t("extensions.defaultName"),
            });
        setActionResult({
          type: "success",
          message,
        });
        if (res.auth_url && !openAuthUrl(res.auth_url).ok) {
          setActionResult({
            type: "error",
            message: "Authentication URL must use HTTPS.",
          });
        } else if (
          !res.auth_url &&
          configureAfterInstall &&
          typeof onNeedsSetup === "function"
        ) {
          onNeedsSetup({
            packageRef,
            displayName,
            // Carry `kind` so the modal can route a connectable channel to the
            // Connect (pairing) panel — without it the modal can't tell this is
            // a channel and falls through to "No configuration required".
            kind,
            // Freshly installed: the caller has not connected/paired yet.
            authenticated: false,
            active: false,
            activationStatus: "setup_required",
            onboardingState: "setup_required",
          });
        }
      } else {
        setActionResult({ type: "error", message: res.message || t("extensions.installFailed") });
      }
      invalidate();
    },
    onError: (err) => {
      setActionResult({ type: "error", message: err.message });
      invalidate();
    },
  });

  const activateMutation = useMutation({
    mutationFn: ({ packageRef }) => activateExtension(packageRef),
    onSuccess: (res, { displayName }) => {
      if (res.success) {
        setActionResult({
          type: "success",
          message:
            res.message ||
            res.instructions ||
            t("extensions.activatedSuccess", {
              name: displayName || t("extensions.defaultName"),
            }),
        });
        if (res.auth_url && !openAuthUrl(res.auth_url).ok) {
          setActionResult({
            type: "error",
            message: "Authentication URL must use HTTPS.",
          });
        }
      } else if (res.auth_url) {
        if (openAuthUrl(res.auth_url).ok) {
          setActionResult({ type: "info", message: t("extensions.openingAuth") });
        } else {
          setActionResult({
            type: "error",
            message: "Authentication URL must use HTTPS.",
          });
        }
      } else if (res.awaiting_token) {
        setActionResult({ type: "info", message: t("extensions.configurationRequired") });
      } else {
        setActionResult({ type: "error", message: res.message || t("extensions.activationFailed") });
      }
      invalidate();
    },
    onError: (err) => {
      setActionResult({ type: "error", message: err.message });
    },
  });

  const removeMutation = useMutation({
    mutationFn: ({ packageRef }) => removeExtension(packageRef),
    onSuccess: (res, { displayName }) => {
      if (res.success) {
        setActionResult({
          type: "success",
          message: t("extensions.removedSuccess", {
            name: displayName || t("extensions.defaultName"),
          }),
        });
      } else {
        setActionResult({ type: "error", message: res.message || t("extensions.removeFailed") });
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
  const connectableChannels = connectableChannelsQuery.data?.channels || [];
  const extensionById = new Map(
    extensions
      .map((extension) => [packageId(extension), extension])
      .filter(([id]) => Boolean(id))
  );
  const registryIds = new Set(registry.map((entry) => packageId(entry)).filter(Boolean));
  const catalogEntries = [
    ...registry.map((entry, index) => {
      const id = packageId(entry);
      const extension = id ? extensionById.get(id) || null : null;
      return {
        id: catalogId("registry", entry, index),
        installed: Boolean(extension || entry.installed),
        entry,
        extension,
      };
    }),
    ...extensions
      .filter((extension) => {
        const id = packageId(extension);
        return !id || !registryIds.has(id);
      })
      .map((extension, index) => ({
        id: catalogId("installed", extension, index),
        installed: true,
        entry: null,
        extension,
      })),
  ].sort(catalogSort);

  const isChannel = (entry) => isChannelExtensionKind(entry.kind);
  const channels = extensions.filter(isChannel);
  const mcpServers = extensions.filter((e) => e.kind === "mcp_server");
  const tools = extensions.filter((e) => !isChannel(e) && e.kind !== "mcp_server");

  const channelRegistry = registry.filter((e) => isChannel(e) && !e.installed);
  const mcpRegistry = registry.filter((e) => e.kind === "mcp_server" && !e.installed);
  const toolRegistry = registry.filter(
    (e) =>
      e.kind !== "mcp_server" &&
      !isChannel(e) &&
      !e.installed
  );

  const isLoading = extensionsQuery.isLoading || registryQuery.isLoading;
  const isBusy = installMutation.isPending || activateMutation.isPending || removeMutation.isPending;
  const remove = React.useCallback(
    (extension) => {
      const name = extension?.displayName || extension?.packageRef?.id || "this extension";
      if (!window.confirm(`Remove ${name}?`)) return;
      removeMutation.mutate(extension);
    },
    [removeMutation]
  );

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
    catalogEntries,
    connectableChannels,
    isLoading,
    isBusy,
    actionResult,
    clearResult,
    install: installMutation.mutate,
    activate: activateMutation.mutate,
    remove,
    invalidate,
  };
}

export function useExtensionSetup(packageRef) {
  const query = useQuery({
    queryKey: ["extension-setup", packageRef?.id || packageRef],
    queryFn: () => fetchExtensionSetup(packageRef),
    enabled: Boolean(packageRef),
  });

  return {
    secrets: query.data?.secrets || [],
    fields: query.data?.fields || [],
    onboarding: query.data?.onboarding || null,
    isLoading: query.isLoading,
    error: query.error,
  };
}

export function useSetupSubmit(packageRef, onSuccess) {
  const queryClient = useQueryClient();
  const packageKey = packageRef?.id || packageRef;

  return useMutation({
    mutationFn: ({ secrets, fields }) =>
      submitExtensionSetup(packageRef, secrets, fields).then((res) => {
        if (res.success === false) {
          throw new Error(res.message || "Setup failed");
        }
        return res;
      }),
    onSuccess: (res) => {
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
      queryClient.invalidateQueries({ queryKey: ["extension-setup", packageKey] });
      if (onSuccess) onSuccess(res);
    },
  });
}

export function useOauthSetup(packageRef, { onConfigured } = {}) {
  const queryClient = useQueryClient();
  const packageKey = packageRef?.id || packageRef;
  const watcherRef = React.useRef(null);
  const configuredRef = React.useRef(false);
  const [isAuthorizing, setIsAuthorizing] = React.useState(false);

  const clearWatcher = React.useCallback(() => {
    const cleanup = watcherRef.current;
    watcherRef.current = null;
    if (typeof cleanup === "function") {
      cleanup();
    } else if (cleanup) {
      window.clearInterval(cleanup);
      setIsAuthorizing(false);
    }
  }, []);

  const refreshSetupState = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["extensions"] });
    queryClient.invalidateQueries({ queryKey: ["extension-registry"] });
    queryClient.invalidateQueries({ queryKey: ["extension-setup", packageKey] });
  }, [packageKey, queryClient]);

  const setupIsConfigured = React.useCallback(({ allowProvidedSecrets = true } = {}) => {
    const setup = queryClient.getQueryData(["extension-setup", packageKey]);
    if (
      allowProvidedSecrets &&
      setup?.secrets?.length > 0 &&
      setup.secrets.every((secret) => secret.provided)
    ) {
      return true;
    }
    const extensions = queryClient.getQueryData(["extensions"])?.extensions || [];
    const extension = extensions.find((item) => item.package_ref?.id === packageKey);
    return extensionListItemIsConfigured(extension);
  }, [packageKey, queryClient]);

  const watchOauthProgress = React.useCallback(
    (popup, { flowId = null, requireCallbackCompletion = false } = {}) => {
      clearWatcher();
      configuredRef.current = false;
      const browserWindow =
        typeof window !== "undefined" ? window : globalThis?.window || null;
      if (!browserWindow) return;
      setIsAuthorizing(true);
      const startedAt = Date.now();
      let stopped = false;
      let channel = null;
      let timer = null;

      function cleanup() {
        if (stopped) return;
        stopped = true;
        if (timer) browserWindow.clearInterval(timer);
        if (channel) channel.close();
        browserWindow.removeEventListener?.("storage", onStorage);
        setIsAuthorizing(false);
      }

      function stopWatcher() {
        if (watcherRef.current === cleanup) watcherRef.current = null;
        cleanup();
      }

      function complete() {
        if (stopped) return;
        if (!configuredRef.current) {
          configuredRef.current = true;
          Promise.resolve(onConfigured?.()).catch(() => {});
        }
        stopWatcher();
        refreshSetupState();
      }

      function handleCompletion(payload) {
        if (!oauthCompletionMatchesFlow(payload, flowId)) return false;
        complete();
        return true;
      }

      function onStorage(event) {
        if (event.key !== OAUTH_CALLBACK_STORAGE_KEY) return;
        handleCompletion(parseOAuthCallbackStoragePayload(event.newValue));
      }

      if (typeof browserWindow.BroadcastChannel === "function") {
        channel = new browserWindow.BroadcastChannel(OAUTH_CALLBACK_CHANNEL);
        channel.onmessage = (event) => handleCompletion(event.data);
      }
      browserWindow.addEventListener?.("storage", onStorage);

      timer = browserWindow.setInterval(() => {
        refreshSetupState();
        if (
          handleCompletion(
            parseOAuthCallbackStoragePayload(
              browserWindow.localStorage?.getItem?.(OAUTH_CALLBACK_STORAGE_KEY),
            ),
          )
        ) {
          return;
        }
        const configured = setupIsConfigured({
          allowProvidedSecrets: !requireCallbackCompletion,
        });
        if (configured) {
          complete();
          return;
        }
        const timedOut = Date.now() - startedAt > OAUTH_SETUP_TIMEOUT_MS;
        const popupClosedBeforeCallback = popup && popup.closed && !requireCallbackCompletion;
        if (popupClosedBeforeCallback || timedOut) {
          stopWatcher();
          refreshSetupState();
        }
      }, OAUTH_SETUP_REFRESH_MS);
      watcherRef.current = cleanup;
      handleCompletion(
        parseOAuthCallbackStoragePayload(
          browserWindow.localStorage?.getItem?.(OAUTH_CALLBACK_STORAGE_KEY),
        ),
      );
    },
    [clearWatcher, onConfigured, refreshSetupState, setupIsConfigured]
  );

  React.useEffect(() => clearWatcher, [clearWatcher]);

  const mutation = useMutation({
    mutationFn: ({ secret, popup }) =>
      startExtensionOauth(packageRef, secret).then((res) => {
        if (res.success === false) {
          throw new Error(res.message || "OAuth setup failed");
        }
        if (res.authorization_url && !isHttpsAuthUrl(res.authorization_url)) {
          throw new Error("Authorization URL must use HTTPS.");
        }
        return { res, popup };
      }),
    onSuccess: ({ res, popup }, variables) => {
      let authPopup = popup;
      if (res.authorization_url) {
        const opened = openAuthUrl(res.authorization_url, popup);
        authPopup = opened.popup;
        if (!opened.ok) {
          throw new Error(
            opened.reason === "popup_blocked"
              ? "Authorization popup was blocked."
              : "Authorization URL must use HTTPS.",
          );
        }
      } else if (popup && !popup.closed) {
        popup.close();
      }
      refreshSetupState();
      if (authPopup) {
        const flowId = oauthResponseFlowId(res);
        watchOauthProgress(authPopup, {
          flowId,
          requireCallbackCompletion: Boolean(flowId && variables?.secret?.provided),
        });
      }
    },
    onError: (_err, variables) => {
      clearWatcher();
      const popup = variables?.popup;
      if (popup && !popup.closed) popup.close();
    },
  });
  return { ...mutation, isAuthorizing };
}

export function usePairing(channel, options = {}) {
  const query = useQuery({
    queryKey: ["pairing", channel],
    queryFn: () => fetchPairingRequests(channel),
    enabled: Boolean(channel) && options.enabled !== false,
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
    result: approveMutation.isSuccess ? approveMutation.data : null,
    error: approveMutation.isError ? approveMutation.error : null,
  };
}
