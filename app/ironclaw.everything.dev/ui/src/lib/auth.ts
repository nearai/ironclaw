import { apiKeyClient } from "@better-auth/api-key/client";
import { passkeyClient } from "@better-auth/passkey/client";
import { useQuery } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import {
  adminClient,
  anonymousClient,
  inferAdditionalFields,
  organizationClient,
  phoneNumberClient,
} from "better-auth/client/plugins";
import { createAuthClient as createBetterAuthClient } from "better-auth/react";
import type { RelayedTransactionT } from "better-near-auth";
import { siwnClient } from "better-near-auth/client";
import type { ClientRuntimeConfig } from "everything-dev/types";
import { getRuntimeConfig } from "everything-dev/ui/runtime";
import type { Auth } from "./auth-types.gen";

type RuntimeAuthVariables = {
  siwn: {
    recipient?: string;
    recipients?: {
      mainnet?: string;
      testnet?: string;
    };
  };
};

type CreateAuthClientOptions = {
  runtimeConfig?: Partial<ClientRuntimeConfig>;
  headers?: HeadersInit;
  cspNonce?: string;
};

type SiwnClientConfig = Parameters<typeof siwnClient>[0] & {
  cspNonce?: string;
};

function hasAuthVariables(auth: ClientRuntimeConfig["auth"] | undefined): auth is NonNullable<
  ClientRuntimeConfig["auth"]
> & {
  variables: RuntimeAuthVariables;
} {
  return !!auth && typeof auth === "object" && typeof auth.variables === "object";
}

function readRuntimeConfig(config?: Partial<ClientRuntimeConfig>) {
  if (config) return config;
  if (typeof window === "undefined") return undefined;
  try {
    return getRuntimeConfig();
  } catch {
    return undefined;
  }
}

function getAuthVariables(config?: Partial<ClientRuntimeConfig>): RuntimeAuthVariables {
  const runtimeConfig = readRuntimeConfig(config);
  if (!runtimeConfig || !hasAuthVariables(runtimeConfig.auth)) {
    throw new Error("Missing auth runtime configuration");
  }
  return runtimeConfig.auth.variables;
}

function getProviderId(account: {
  providerId?: unknown;
  accountId?: unknown;
  network?: unknown;
}): string {
  if (typeof account.providerId === "string" && account.providerId.length > 0) {
    return account.providerId;
  }

  if (
    typeof account.accountId === "string" &&
    (account.network === "mainnet" || account.network === "testnet")
  ) {
    return "siwn";
  }

  return "unknown";
}

export function getAccountProviderId(account: {
  providerId?: unknown;
  accountId?: unknown;
  network?: unknown;
}): string {
  return getProviderId(account);
}

export function getNearAccountId(
  linkedAccounts: Array<{ providerId?: unknown; accountId?: unknown; network?: unknown }>,
): string | null {
  if (!Array.isArray(linkedAccounts)) {
    return null;
  }

  const nearAccount = linkedAccounts.find((account) => getProviderId(account) === "siwn");
  if (typeof nearAccount?.accountId !== "string") {
    return null;
  }

  return nearAccount.accountId.split(":")[0] || null;
}

export function getLinkedProviders(
  linkedAccounts: Array<{ providerId?: unknown; accountId?: unknown; network?: unknown }>,
): string[] {
  if (!Array.isArray(linkedAccounts)) {
    return [];
  }

  return [...new Set(linkedAccounts.map((account) => getProviderId(account)))];
}

function getSiwnClientConfig(options: CreateAuthClientOptions): SiwnClientConfig {
  const runtimeConfig = readRuntimeConfig(options.runtimeConfig);
  const variables = getAuthVariables(options.runtimeConfig);
  const siwn = variables.siwn;

  const mainnetRecipient = siwn.recipients?.mainnet ?? siwn.recipient;
  if (!mainnetRecipient) {
    throw new Error("Missing auth SIWN recipient");
  }

  const networkId =
    runtimeConfig?.networkId ?? (mainnetRecipient.endsWith(".testnet") ? "testnet" : "mainnet");
  const testnetRecipient = siwn.recipients?.testnet;
  const recipient =
    networkId === "testnet" && testnetRecipient ? testnetRecipient : mainnetRecipient;

  return { recipient, networkId, cspNonce: options.cspNonce };
}

function getHostUrl(config?: Partial<ClientRuntimeConfig>) {
  const runtimeConfig = readRuntimeConfig(config);
  if (runtimeConfig?.hostUrl) return runtimeConfig.hostUrl;
  if (typeof window !== "undefined") return window.location.origin;
  return "";
}

export function createAuthClient(options: CreateAuthClientOptions = {}) {
  const nearAuthConfig = getSiwnClientConfig(options);

  return createBetterAuthClient({
    baseURL: getHostUrl(options.runtimeConfig),
    fetchOptions: {
      credentials: "include",
      ...(options.headers ? { headers: options.headers } : {}),
    },
    plugins: [
      inferAdditionalFields<Auth>(),
      siwnClient(nearAuthConfig),
      adminClient(),
      anonymousClient(),
      phoneNumberClient(),
      passkeyClient(),
      organizationClient(),
      apiKeyClient(),
    ],
  });
}

export type AuthClient = ReturnType<typeof createAuthClient>;
type OrganizationListResult = Awaited<ReturnType<AuthClient["organization"]["list"]>>;
type PasskeyListResult = Awaited<ReturnType<AuthClient["passkey"]["listUserPasskeys"]>>;

export type SessionData = AuthClient["$Infer"]["Session"];
export type Organization = NonNullable<OrganizationListResult["data"]>[number];
export type Passkey = NonNullable<PasskeyListResult["data"]>[number];

export function useAuthClient(): AuthClient {
  return useRouter().options.context.authClient;
}

export const sessionQueryKey = ["session"] as const;

export function sessionQueryOptions(authClient: AuthClient, initialSession?: SessionData | null) {
  const baseOptions = {
    queryKey: sessionQueryKey,
    queryFn: async () => {
      const { data: session } = await authClient.getSession();
      return session ?? null;
    },
    staleTime: 60 * 1000,
    gcTime: 10 * 60 * 1000,
  };

  return initialSession === undefined
    ? baseOptions
    : { ...baseOptions, initialData: initialSession };
}

export function useRelayHistory(session: SessionData | null | undefined, authClient: AuthClient) {
  return useQuery({
    queryKey: ["relay-history"],
    queryFn: async (): Promise<RelayedTransactionT[]> => {
      const res = await authClient.near.relayHistory();
      return res?.data?.transactions ?? [];
    },
    enabled: !!session,
    refetchInterval: 2000,
  });
}
