export type HostedInstance = {
  id: string;
  name?: string;
  service_type?: string;
  status?: string;
  dashboard_url?: string;
};

type HostedInstancesResponse = {
  items?: HostedInstance[];
};

export function hostedControlOrigin(productOrigin: string): string {
  return productOrigin.includes("stg")
    ? "https://private-chat-stg.near.ai"
    : "https://private.near.ai";
}

export function hostedLoginUrl(
  controlOrigin: string,
  provider: string,
  returnUrl: string
): string {
  const url = new URL(`/v1/auth/${encodeURIComponent(provider)}`, controlOrigin);
  url.searchParams.set("frontend_callback", returnUrl);
  url.searchParams.set("oauth_channel", "mobile");
  return url.toString();
}

export function callbackLoginTicket(url: string): string {
  const callback = new URL(url);
  const hash = new URLSearchParams(callback.hash.replace(/^#/, ""));
  return callback.searchParams.get("login_ticket") ?? hash.get("login_ticket") ?? "";
}

export class HostedControlApi {
  constructor(
    private readonly origin: string,
    private readonly token: string
  ) {}

  async listInstances(): Promise<HostedInstance[]> {
    const response = await fetch(`${this.origin}/v1/agents/instances?limit=100&offset=0`, {
      headers: {
        Accept: "application/json",
        Authorization: `Bearer ${this.token}`
      }
    });
    const payload = (await response.json().catch(() => undefined)) as
      | HostedInstancesResponse
      | { detail?: string }
      | undefined;
    if (!response.ok) {
      const message = payload && "detail" in payload ? payload.detail : undefined;
      throw new Error(message || "Could not load your hosted agents");
    }
    return payload && "items" in payload ? payload.items ?? [] : [];
  }

  async exchangeLoginTicket(ticket: string): Promise<string> {
    const headers = new Headers({
      Accept: "application/json",
      "Content-Type": "application/json"
    });
    if (this.token) headers.set("Authorization", `Bearer ${this.token}`);
    const response = await fetch(`${this.origin}/auth/session/exchange`, {
      method: "POST",
      headers,
      body: JSON.stringify({ ticket })
    });
    const payload = (await response.json().catch(() => undefined)) as
      | { token?: string; detail?: string }
      | undefined;
    if (!response.ok) {
      throw new Error(payload?.detail || "Could not complete hosted login");
    }
    const nextToken = payload?.token?.trim() ?? "";
    if (!nextToken) throw new Error("Hosted login did not return a session token");
    return nextToken;
  }
}

export function preferredIronClawInstance(instances: HostedInstance[]): HostedInstance | undefined {
  const ironclaw = instances.filter(
    (instance) =>
      instance.dashboard_url &&
      (instance.service_type === "ironclaw" || instance.service_type === "ironclaw-dind")
  );
  return ironclaw.find((instance) => instance.status === "running") ?? ironclaw[0];
}
