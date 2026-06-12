import type { Context, Next } from "hono";
import type {
  AuthRequestContext,
  AuthSession,
  AuthSessionData,
  AuthSessionUser,
  AuthServices as GeneratedAuthServices,
} from "@/lib/auth-types.gen";
import type { HostPluginEntry, PluginResult } from "./plugins";

export type AuthUser = AuthSessionUser;

interface AuthServices extends GeneratedAuthServices {
  auth: GeneratedAuthServices["auth"];
}

interface AuthClient {
  getSession(): Promise<AuthSession | null>;
  getContext(): Promise<AuthRequestContext>;
}

export interface ApiKeyContext {
  id: string;
  name: string | null;
  permissions: Record<string, string[]> | null;
}

export interface AuthVariables {
  user: AuthUser | null;
  session: AuthSessionData | null;
  reqHeaders: Headers;
  getRawBody: () => Promise<string>;
  walletAddress: string | null;
  apiKey: ApiKeyContext | null;
  organizationId: string | null;
}

export type HonoEnv = { Variables: AuthVariables };

export function toAuthClientContext(headers: Headers): Record<string, string> {
  return Object.fromEntries(headers.entries());
}

function resolveAuthEntry(plugins: PluginResult): HostPluginEntry | null {
  return plugins.auth ?? plugins.plugins.auth ?? null;
}

function getAuthServices(plugins: PluginResult): AuthServices | null {
  const entry = resolveAuthEntry(plugins);
  if (!entry?.initialized?.context) return null;
  return entry.initialized.context as AuthServices;
}

export function registerAuthHandler(app: { on: (...args: any[]) => any }, plugins: PluginResult) {
  const services = getAuthServices(plugins);
  if (!services) return;
  app.on(["POST", "GET"], "/api/auth/*", (c: Context<HonoEnv>) => services.handler(c.req.raw));
}

export function createSessionMiddleware(plugins: PluginResult) {
  const authClientFactory = plugins.authClient;

  return async (c: Context<HonoEnv>, next: Next) => {
    if (c.req.path.startsWith("/api/auth/")) {
      return next();
    }

    c.set("reqHeaders", c.req.raw.headers);

    const rawClone = c.req.method === "GET" || c.req.method === "HEAD" ? null : c.req.raw.clone();
    let cachedRawBody: string | null = null;
    c.set("getRawBody", async () => {
      if (cachedRawBody !== null) return cachedRawBody;
      if (!rawClone) {
        cachedRawBody = "";
        return cachedRawBody;
      }
      cachedRawBody = await rawClone.text();
      return cachedRawBody;
    });

    if (!authClientFactory) {
      c.set("user", null);
      c.set("session", null);
      c.set("walletAddress", null);
      c.set("apiKey", null);
      c.set("organizationId", null);
      await next();
      return;
    }

    try {
      const authClient = authClientFactory({
        reqHeaders: toAuthClientContext(c.get("reqHeaders")),
      }) as AuthClient;
      const [sessionResult, contextResult] = await Promise.all([
        authClient.getSession(),
        authClient.getContext(),
      ]);
      c.set("user", sessionResult?.user ?? contextResult.user ?? null);
      c.set("session", sessionResult?.session ?? null);
      c.set("walletAddress", contextResult.near.primaryAccountId ?? null);
      c.set("apiKey", contextResult.apiKey ?? null);
      c.set("organizationId", contextResult.organization?.activeOrganizationId ?? null);
    } catch (error) {
      console.warn(
        `[Auth] Session resolution failed: ${error instanceof Error ? error.message : String(error)}`,
      );
      c.set("user", null);
      c.set("session", null);
      c.set("walletAddress", null);
      c.set("apiKey", null);
      c.set("organizationId", null);
    }

    await next();
  };
}

export function buildPluginContext(c: Context<HonoEnv>) {
  const user = c.get("user");
  const session = c.get("session");
  const walletAddress = c.get("walletAddress");
  const apiKey = c.get("apiKey");
  const organizationId = c.get("organizationId");

  return {
    userId: user?.id,
    user: user ?? undefined,
    walletAddress: walletAddress ?? undefined,
    organizationId: organizationId ?? session?.activeOrganizationId ?? undefined,
    apiKey: apiKey ?? undefined,
    reqHeaders: c.get("reqHeaders"),
    getRawBody: c.get("getRawBody"),
  };
}
