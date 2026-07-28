import Constants from "expo-constants";
import * as SecureStore from "expo-secure-store";
import * as WebBrowser from "expo-web-browser";
import * as Linking from "expo-linking";
import React from "react";
import { Platform } from "react-native";
import { IronClawApi } from "@/lib/api";
import { validateDeploymentOrigin } from "@/lib/deployment-url";
import {
  callbackLoginTicket,
  HostedControlApi,
  hostedControlOrigin,
  hostedLoginUrl,
  preferredIronClawInstance
} from "@/lib/hosted";
import type { Deployment, Session } from "@/types";

WebBrowser.maybeCompleteAuthSession();

const TOKEN_KEY = "ironclaw.mobile.token.v1";
const ORIGIN_KEY = "ironclaw.mobile.origin.v1";
const SESSION_KEY = "ironclaw.mobile.session.v1";
const DEFAULT_ORIGIN =
  (Constants.expoConfig?.extra?.hostedOrigin as string | undefined) ??
  "https://agent-stg.near.ai";
const PRODUCTION = Constants.expoConfig?.extra?.buildProfile === "production";

type SessionContextValue = {
  loading: boolean;
  token: string;
  deployment: Deployment;
  session: Session | null;
  api: IronClawApi;
  error: string;
  connectWithToken: (origin: string, token: string) => Promise<void>;
  loginWithProvider: (provider: string) => Promise<void>;
  signOut: () => Promise<void>;
  refreshSession: () => Promise<void>;
};

const hostedDeployment = (origin: string): Deployment => ({
  id: origin,
  name: origin.includes("stg") ? "IronClaw Staging" : "IronClaw",
  origin,
  hosted: origin.endsWith("agent.near.ai") || origin.endsWith("agent-stg.near.ai")
});

const SessionContext = React.createContext<SessionContextValue | null>(null);

async function readSecret(key: string): Promise<string> {
  if (Platform.OS === "web") return globalThis.localStorage?.getItem(key) ?? "";
  return (await SecureStore.getItemAsync(key)) ?? "";
}

async function writeSecret(key: string, value: string): Promise<void> {
  if (Platform.OS === "web") {
    if (value) globalThis.localStorage?.setItem(key, value);
    else globalThis.localStorage?.removeItem(key);
    return;
  }
  if (value) await SecureStore.setItemAsync(key, value);
  else await SecureStore.deleteItemAsync(key);
}

function parseSavedSession(value: string): Session | null {
  if (!value) return null;
  try {
    const candidate = JSON.parse(value) as Partial<Session>;
    return typeof candidate.user_id === "string" && typeof candidate.tenant_id === "string"
      ? (candidate as Session)
      : null;
  } catch {
    return null;
  }
}

export function SessionProvider({ children }: React.PropsWithChildren) {
  const [loading, setLoading] = React.useState(true);
  const [token, setToken] = React.useState("");
  const [origin, setOrigin] = React.useState(DEFAULT_ORIGIN);
  const [session, setSession] = React.useState<Session | null>(null);
  const [error, setError] = React.useState("");
  const api = React.useMemo(() => new IronClawApi(origin, token), [origin, token]);

  const validate = React.useCallback(async (nextOrigin: string, nextToken: string) => {
    const validatedOrigin = validateDeploymentOrigin(nextOrigin, PRODUCTION);
    const nextApi = new IronClawApi(validatedOrigin, nextToken);
    const nextSession = await nextApi.session();
    await Promise.all([
      writeSecret(TOKEN_KEY, nextToken),
      writeSecret(ORIGIN_KEY, validatedOrigin),
      writeSecret(SESSION_KEY, JSON.stringify(nextSession))
    ]);
    setOrigin(validatedOrigin);
    setToken(nextToken);
    setSession(nextSession);
    setError("");
  }, []);

  React.useEffect(() => {
    Promise.all([readSecret(TOKEN_KEY), readSecret(ORIGIN_KEY), readSecret(SESSION_KEY)])
      .then(async ([savedToken, savedOrigin, savedSession]) => {
        const nextOrigin = savedOrigin || DEFAULT_ORIGIN;
        setOrigin(nextOrigin);
        setToken(savedToken);
        setSession(parseSavedSession(savedSession));
        if (!savedToken) return;
        try {
          const refreshedSession = await new IronClawApi(nextOrigin, savedToken).session();
          setSession(refreshedSession);
          await writeSecret(SESSION_KEY, JSON.stringify(refreshedSession));
        } catch (reason) {
          setError(reason instanceof Error ? reason.message : "Could not reach your agent");
        }
      })
      .finally(() => setLoading(false));
  }, []);

  const loginWithProvider = React.useCallback(
    async (provider: string) => {
      const returnUrl = Linking.createURL("auth/callback");
      const controlOrigin = hostedControlOrigin(DEFAULT_ORIGIN);
      const result = await WebBrowser.openAuthSessionAsync(
        hostedLoginUrl(controlOrigin, provider, returnUrl),
        returnUrl
      );
      if (result.type !== "success") return;
      const loginTicket = callbackLoginTicket(result.url);
      if (!loginTicket) throw new Error("The hosted login did not return a login ticket");
      const bootstrapApi = new HostedControlApi(controlOrigin, "");
      const accountToken = await bootstrapApi.exchangeLoginTicket(loginTicket);
      const instances = await new HostedControlApi(controlOrigin, accountToken).listInstances();
      const instance = preferredIronClawInstance(instances);
      if (!instance?.dashboard_url) {
        throw new Error(
          "No hosted IronClaw deployment is ready. Create or start one at agent.near.ai, then try again."
        );
      }
      await validate(instance.dashboard_url, accountToken);
    },
    [validate]
  );

  const signOut = React.useCallback(async () => {
    await Promise.all([
      writeSecret(TOKEN_KEY, ""),
      writeSecret(ORIGIN_KEY, ""),
      writeSecret(SESSION_KEY, "")
    ]);
    setToken("");
    setSession(null);
    setOrigin(DEFAULT_ORIGIN);
    setError("");
  }, []);

  const refreshSession = React.useCallback(async () => {
    if (!token) return;
    const refreshedSession = await api.session();
    setSession(refreshedSession);
    await writeSecret(SESSION_KEY, JSON.stringify(refreshedSession));
  }, [api, token]);

  const value: SessionContextValue = {
    loading,
    token,
    deployment: hostedDeployment(origin),
    session,
    api,
    error,
    connectWithToken: validate,
    loginWithProvider,
    signOut,
    refreshSession
  };
  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}

export function useSession(): SessionContextValue {
  const context = React.useContext(SessionContext);
  if (!context) throw new Error("useSession must be used inside SessionProvider");
  return context;
}
