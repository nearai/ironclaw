// @ts-nocheck
import React from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { clientActionId, gatewayStatus } from "../../../lib/api";
import {
  completionMatchesFlow,
  failureMatchesFlow,
  isHttpsAuthUrl,
  openAuthPopup,
  readLatestProductAuthOAuthCompletion,
  subscribeProductAuthOAuthCompletion,
} from "../../../lib/product-auth-oauth-events";
import { useT } from "../../../lib/i18n";
import { hasChannelSurface } from "../lib/extensions-schema";
import {
  fetchExtensions,
  fetchExtensionRegistry,
  installExtension,
  activateExtension,
  removeExtension,
  fetchExtensionSetup,
  submitExtensionSetup,
  startExtensionOauth,
  fetchOauthFlowStatus,
  fetchPairingRequests,
  approvePairingCode,
  importExtension,
} from "../lib/extensions-api";

const OAUTH_SETUP_REFRESH_MS = 2000;
const OAUTH_SETUP_TIMEOUT_MS = 10 * 60 * 1000;
const OAUTH_STATUS_ERROR_KEYS = Object.freeze({
  failed: "extensions.oauthFailed",
  canceled: "extensions.oauthCanceled",
  expired: "extensions.oauthExpired",
});

// OAuth callback constants, HTTPS-auth-URL/popup helpers, and completion
// parsing/matching are the shared product-auth OAuth event contract — see
// `lib/product-auth-oauth-events.ts`. This hook keeps only its setup-watcher
// state machine below.

function authPopupFailureMessage(reason, t) {
  return reason === "popup_blocked"
    ? t("authGate.popupBlocked")
    : t("extensions.oauthInvalidAuthorizationUrl");
}

function oauthResponseFlowId(response) {
  return response?.flow_id || response?.flowId || null;
}

// The invocation id the start response minted, carried on the callback-scope
// hint. The origin-independent flow-status poll sends it back so the
// caller-scoped backend can re-derive the exact scope its `get_flow` matched
// on when the flow was created.
function oauthResponseInvocationId(response) {
  const scope = response?.callback_scope || response?.callbackScope || null;
  return scope?.invocation_id || scope?.invocationId || null;
}

function extensionListItemIsConfigured(extension) {
  if (!extension) return false;
  if (extension.needs_setup === false && (extension.authenticated || extension.active)) {
    return true;
  }
  // Same snake/camel fallback chain as `extensionLifecycleState`
  // (lib/extension-actions.ts) so a camelCase snapshot cannot read as
  // "not configured" here while the rest of the page treats it as active.
  const state =
    extension.onboarding_state ||
    extension.onboardingState ||
    extension.installation_state ||
    extension.installationState ||
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

function withClientActionId(payload = {}) {
  const source = payload || {};
  return {
    ...source,
    clientActionId: source.clientActionId || clientActionId(),
  };
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
    // The page must distinguish an offline request from a successful empty
    // catalog. TanStack's default online mode pauses without calling queryFn,
    // leaving both data and error empty and reproducing the misleading state.
    networkMode: "always",
    refetchOnMount: "always",
  });

  const registryQuery = useQuery({
    queryKey: ["extension-registry"],
    queryFn: fetchExtensionRegistry,
    networkMode: "always",
    refetchOnMount: "always",
  });

  const refetch = React.useCallback(
    () => Promise.all([extensionsQuery.refetch(), registryQuery.refetch()]),
    [extensionsQuery.refetch, registryQuery.refetch]
  );

  const invalidate = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["extensions"] });
    queryClient.invalidateQueries({ queryKey: ["extension-registry"] });
    queryClient.invalidateQueries({ queryKey: ["gateway-status-extensions"] });
  }, [queryClient]);

  const [actionResult, setActionResult] = React.useState(null);

  const clearResult = React.useCallback(() => setActionResult(null), []);

  const installMutation = useMutation({
    mutationFn: ({ packageRef, clientActionId: actionId }) =>
      installExtension(packageRef, { clientActionId: actionId }),
    onSuccess: (res, { displayName, surfaces, configureAfterInstall, onNeedsSetup, packageRef }) => {
      if (res.success) {
        const message = hasChannelSurface({ surfaces })
          ? t("extensions.channelInstalledSetup", {
              name: displayName || t("extensions.defaultName"),
            })
          : res.message ||
            res.instructions ||
            t("extensions.installedSuccess", {
              name: displayName || t("extensions.defaultName"),
            });
        setActionResult({
          type: "success",
          message,
        });
        let installAuthPopup = null;
        if (res.auth_url) {
          installAuthPopup = openAuthPopup(res.auth_url);
        }
        if (installAuthPopup && !installAuthPopup.ok) {
          setActionResult({
            type: "error",
            message: authPopupFailureMessage(installAuthPopup.reason, t),
          });
        } else if (
          !res.auth_url &&
          configureAfterInstall &&
          typeof onNeedsSetup === "function"
        ) {
          onNeedsSetup({
            packageRef,
            displayName,
            // Carry `surfaces` so the modal can route a channel-surface
            // extension to the Connect panel — without it the modal can't
            // tell this is a channel and falls through to "No configuration
            // required".
            surfaces,
            // Freshly installed: the caller has not connected/paired yet.
            authenticated: false,
            active: false,
            installationState: "installed",
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
    mutationFn: ({ packageRef, clientActionId: actionId }) =>
      activateExtension(packageRef, { clientActionId: actionId }),
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
        if (res.auth_url) {
          const opened = openAuthPopup(res.auth_url);
          if (!opened.ok) {
            setActionResult({
              type: "error",
              message: authPopupFailureMessage(opened.reason, t),
            });
          }
        }
      } else if (res.auth_url) {
        const opened = openAuthPopup(res.auth_url);
        if (opened.ok) {
          setActionResult({ type: "info", message: t("extensions.openingAuth") });
        } else {
          setActionResult({
            type: "error",
            message: authPopupFailureMessage(opened.reason, t),
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
    mutationFn: ({ packageRef, clientActionId: actionId }) =>
      removeExtension(packageRef, { clientActionId: actionId }),
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

  // Views over surfaces (product taxonomy). Runtime never groups: an
  // MCP-backed tool extension sits beside a wasm one, with its runtime shown
  // as a badge on the card.
  const channels = extensions.filter(hasChannelSurface);
  const tools = extensions.filter((e) => !hasChannelSurface(e));

  const channelRegistry = registry.filter((e) => hasChannelSurface(e) && !e.installed);
  const toolRegistry = registry.filter((e) => !hasChannelSurface(e) && !e.installed);

  const importMutation = useMutation({
    mutationFn: ({ file }) => importExtension(file),
    onSuccess: (res) => {
      if (res.success) {
        setActionResult({
          type: "success",
          message: res.message || t("ext.registry.importSuccess"),
        });
      } else {
        setActionResult({ type: "error", message: res.message || t("ext.registry.importFailed") });
      }
      invalidate();
    },
    onError: (err) => {
      setActionResult({ type: "error", message: err.message });
    },
  });

  const isLoading = extensionsQuery.isLoading || registryQuery.isLoading;
  const isBusy = installMutation.isPending || activateMutation.isPending || removeMutation.isPending || importMutation.isPending;

  return {
    status,
    extensions,
    channels,
    tools,
    channelRegistry,
    toolRegistry,
    registry,
    catalogEntries,
    isExtensionsLoading: extensionsQuery.isLoading,
    isRegistryLoading: registryQuery.isLoading,
    isLoading,
    extensionsError: extensionsQuery.error || null,
    registryError: registryQuery.error || null,
    error: extensionsQuery.error || registryQuery.error || null,
    refetch,
    isRefetching: extensionsQuery.isRefetching || registryQuery.isRefetching,
    isBusy,
    actionResult,
    clearResult,
    install: (payload, options = undefined) =>
      installMutation.mutate(withClientActionId(payload), options),
    activate: (payload, options = undefined) =>
      activateMutation.mutate(withClientActionId(payload), options),
    remove: (payload, options = undefined) =>
      removeMutation.mutate(withClientActionId(payload), options),
    isRemoving: removeMutation.isPending,
    importTool: (payload) => importMutation.mutate(payload),
    isImporting: importMutation.isPending,
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
  const t = useT();
  const queryClient = useQueryClient();
  const packageKey = packageRef?.id || packageRef;

  const mutation = useMutation({
    mutationFn: ({ secrets, fields, clientActionId: actionId }) =>
      submitExtensionSetup(packageRef, secrets, fields, {
        clientActionId: actionId,
      }).then((res) => {
        if (res.success === false) {
          throw new Error(res.message || t("extensions.setupFailed"));
        }
        return res;
      }),
    onSuccess: (res) => {
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
      queryClient.invalidateQueries({ queryKey: ["extension-setup", packageKey] });
      if (onSuccess) onSuccess(res);
    },
  });

  return {
    ...mutation,
    mutate: (payload, options = undefined) =>
      mutation.mutate(withClientActionId(payload), options),
    mutateAsync: (payload, options = undefined) =>
      mutation.mutateAsync(withClientActionId(payload), options),
  };
}

export function useOauthSetup(packageRef, { onConfigured } = {}) {
  const t = useT();
  const queryClient = useQueryClient();
  const packageKey = packageRef?.id || packageRef;
  const watcherRef = React.useRef(null);
  const oauthGenerationRef = React.useRef(0);
  const configuredRef = React.useRef(false);
  const [isAuthorizing, setIsAuthorizing] = React.useState(false);
  // Retryable error surfaced when the callback popup reports a flow-matched
  // FAILURE (provider denial, exchange failure). Ref-guarded setter so the
  // reset at watcher start is a no-op unless an error was actually showing.
  const [authError, setAuthErrorState] = React.useState(null);
  const authErrorRef = React.useRef(null);
  const setAuthError = React.useCallback((value) => {
    if (Object.is(authErrorRef.current, value)) return;
    authErrorRef.current = value;
    setAuthErrorState(value);
  }, []);

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
    (
      popup,
      {
        flowId = null,
        invocationId = null,
        requireCallbackCompletion = false,
        generation,
      } = {},
    ) => {
      if (generation !== oauthGenerationRef.current) return;
      clearWatcher();
      configuredRef.current = false;
      const browserWindow =
        typeof window !== "undefined" ? window : globalThis?.window || null;
      if (!browserWindow) return;
      setIsAuthorizing(true);
      setAuthError(null);
      const startedAt = Date.now();
      // A reconnect starts from an already-configured server snapshot, so
      // "configured" alone cannot prove THIS flow finished: only a
      // not-configured → configured transition observed by the poll (or the
      // flow-id-matched callback signal) may complete the watcher.
      const configuredBeforeFlow = setupIsConfigured({
        allowProvidedSecrets: !requireCallbackCompletion,
      });
      const hasFlowCompletionBackstop = Boolean(flowId);
      let stopped = false;
      let timer = null;
      let unsubscribe = () => {};
      // Guard against overlapping in-flight status polls so a slow request
      // cannot stack fetches across interval ticks.
      let flowStatusPending = false;
      const isCurrentGeneration = () => generation === oauthGenerationRef.current;

      function cleanup() {
        if (stopped) return;
        stopped = true;
        if (timer) browserWindow.clearInterval(timer);
        unsubscribe();
        if (isCurrentGeneration()) setIsAuthorizing(false);
      }

      function stopWatcher() {
        if (watcherRef.current === cleanup) watcherRef.current = null;
        cleanup();
      }

      function complete() {
        if (stopped || !isCurrentGeneration()) return;
        if (!configuredRef.current) {
          configuredRef.current = true;
          Promise.resolve(onConfigured?.()).catch(() => {});
        }
        stopWatcher();
        refreshSetupState();
      }

      function handleCompletion(payload) {
        if (!isCurrentGeneration()) return false;
        if (failureMatchesFlow(payload, flowId)) {
          if (invocationId) {
            pollFlowStatus();
            // Durable status owns terminal reconciliation when an invocation
            // id is available. Keep the interval's timeout path alive if the
            // status endpoint remains unavailable.
            return false;
          }
          setAuthError(t("extensions.oauthFailed"));
          stopWatcher();
          refreshSetupState();
          return true;
        }
        if (!completionMatchesFlow(payload, flowId)) return false;
        complete();
        return true;
      }

      // Origin-independent backstop: the callback page's completion signal is
      // same-origin (localStorage/BroadcastChannel), so a cross-origin callback
      // (local ngrok callback vs 127.0.0.1 opener, or split app/callback domains
      // in prod) never reaches this tab. Poll the durable flow status by id so
      // the watcher can still resolve. Fire-and-forget with a pending guard so
      // the interval never blocks or stacks requests; the browser signal above
      // stays the fast path.
      function pollFlowStatus() {
        if (stopped || !flowId || flowStatusPending) return;
        flowStatusPending = true;
        Promise.resolve(fetchOauthFlowStatus(flowId, invocationId))
          .then((result) => {
            if (stopped || !isCurrentGeneration()) return;
            const status = result?.status;
            if (status === "completed") {
              complete();
            } else {
              const errorKey = OAUTH_STATUS_ERROR_KEYS[status];
              if (typeof errorKey !== "string") return;
              setAuthError(t(errorKey));
              stopWatcher();
              refreshSetupState();
            }
          })
          .catch(() => {})
          .finally(() => {
            flowStatusPending = false;
          });
      }

      unsubscribe = subscribeProductAuthOAuthCompletion(browserWindow, handleCompletion);

      timer = browserWindow.setInterval(() => {
        if (!isCurrentGeneration()) {
          cleanup();
          return;
        }
        refreshSetupState();
        if (handleCompletion(readLatestProductAuthOAuthCompletion(browserWindow))) {
          return;
        }
        pollFlowStatus();
        const configured = setupIsConfigured({
          allowProvidedSecrets: !requireCallbackCompletion,
        });
        if (configured && !configuredBeforeFlow) {
          complete();
          return;
        }
        const timedOut = Date.now() - startedAt > OAUTH_SETUP_TIMEOUT_MS;
        // Current product-auth OAuth callbacks close their popup after writing
        // durable flow status. With a flow id, popup.closed is expected and the
        // status/event backstop owns completion.
        const popupClosedBeforeCallback =
          popup && popup.closed && !hasFlowCompletionBackstop && !requireCallbackCompletion;
        if (popupClosedBeforeCallback || timedOut) {
          if (timedOut) {
            // An abandoned reconnect otherwise ends after 10 minutes with no
            // signal at all — the button was disabled the whole time.
            setAuthError(t("extensions.oauthTimedOut"));
          }
          stopWatcher();
          refreshSetupState();
        }
      }, OAUTH_SETUP_REFRESH_MS);
      watcherRef.current = cleanup;
      handleCompletion(readLatestProductAuthOAuthCompletion(browserWindow));
    },
    [clearWatcher, onConfigured, refreshSetupState, setAuthError, setupIsConfigured, t]
  );

  React.useEffect(() => clearWatcher, [clearWatcher]);

  const mutation = useMutation({
    mutationFn: (variables) => {
      const { secret, popup } = variables;
      const generation = oauthGenerationRef.current + 1;
      oauthGenerationRef.current = generation;
      variables.oauthGeneration = generation;
      clearWatcher();
      return startExtensionOauth(packageRef, secret).then((res) => {
        if (res.success === false) {
          throw new Error(res.message || t("extensions.oauthSetupFailed"));
        }
        if (res.authorization_url && !isHttpsAuthUrl(res.authorization_url)) {
          throw new Error(t("extensions.oauthInvalidAuthorizationUrl"));
        }
        return { res, popup, generation };
      });
    },
    onSuccess: ({ res, popup, generation }, variables) => {
      // React Query always receives `generation` from mutationFn. The fallbacks
      // keep direct unit invocation and older cached mutation results harmless
      // without weakening the current-generation fence.
      const resultGeneration =
        generation ?? variables?.oauthGeneration ?? oauthGenerationRef.current;
      if (resultGeneration !== oauthGenerationRef.current) {
        if (popup && !popup.closed) popup.close();
        return;
      }
      let authPopup = popup;
      if (res.authorization_url) {
        const opened = openAuthPopup(res.authorization_url, popup);
        authPopup = opened.popup;
        if (!opened.ok) {
          throw new Error(authPopupFailureMessage(opened.reason, t));
        }
      } else if (popup && !popup.closed) {
        popup.close();
      }
      refreshSetupState();
      if (authPopup) {
        const flowId = oauthResponseFlowId(res);
        watchOauthProgress(authPopup, {
          flowId,
          invocationId: oauthResponseInvocationId(res),
          requireCallbackCompletion: Boolean(flowId && variables?.secret?.provided),
          generation: resultGeneration,
        });
      }
    },
    onError: (_err, variables) => {
      const failedGeneration =
        variables?.oauthGeneration ?? oauthGenerationRef.current;
      if (failedGeneration !== oauthGenerationRef.current) {
        const stalePopup = variables?.popup;
        if (stalePopup && !stalePopup.closed) stalePopup.close();
        return;
      }
      clearWatcher();
      const popup = variables?.popup;
      if (popup && !popup.closed) popup.close();
    },
  });
  return { ...mutation, isAuthorizing, authError };
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
