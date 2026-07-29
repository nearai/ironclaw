// Extensions fixtures: the installed-extension projection, the registry
// catalog, per-extension setup descriptors, WebGeneratedCode pairing state,
// and the Telegram channel setup endpoints.

import { DAY, MINUTE, iso, isoAhead } from "./helpers";

type Surface = Record<string, unknown>;

type AuthAccount = { state: string; last_error?: string };

export type InstalledExtension = {
  package_ref: { id: string };
  display_name: string;
  description: string;
  version: string;
  runtime: string;
  install_scope: "shared" | "private";
  installation_state: "active" | "setup_needed";
  surfaces: Surface[];
  tools: string[];
  auth_accounts: { vendor: string; accounts: AuthAccount[] }[];
  onboarding?: Record<string, unknown>;
  activation_error?: string;
  installed_at: string;
};

type SetupSecret = {
  name: string;
  prompt: string;
  optional?: boolean;
  provided: boolean;
  setup: { kind: "manual_token" | "oauth" };
};

type SetupDescriptor = {
  secrets: SetupSecret[];
  onboarding: Record<string, unknown> | null;
};

const channelSurface = (strategy: string, instructions?: string): Surface => ({
  kind: "channel",
  direction: "bidirectional",
  connection: { strategy, ...(instructions ? { instructions } : {}) },
});

const toolSurface: Surface = { kind: "tool" };
const authSurface: Surface = { kind: "auth" };

/* ── Installed extensions ──────────────────────────────────────────── */

export const installedExtensions: InstalledExtension[] = [
  {
    package_ref: { id: "nearai.slack" },
    display_name: "Slack",
    description:
      "Connect Slack channels and DMs: route alerts, reply in threads, and deliver final replies to #ops.",
    version: "0.4.2",
    runtime: "wasm",
    install_scope: "shared",
    installation_state: "active",
    surfaces: [channelSurface("oauth"), authSurface],
    tools: ["slack.send_message", "slack.read_channel", "slack.list_channels"],
    auth_accounts: [{ vendor: "slack", accounts: [{ state: "connected" }] }],
    installed_at: iso(45 * DAY),
  },
  {
    package_ref: { id: "nearai.telegram" },
    display_name: "Telegram",
    description:
      "Pair your Telegram account with a one-time code and chat with the agent from anywhere.",
    version: "0.3.0",
    runtime: "wasm",
    install_scope: "shared",
    installation_state: "active",
    surfaces: [
      channelSurface(
        "web_generated_code",
        "Open the bot in Telegram and send the pairing code below."
      ),
    ],
    tools: ["telegram.send_message"],
    auth_accounts: [{ vendor: "telegram", accounts: [{ state: "connected" }] }],
    installed_at: iso(30 * DAY),
  },
  {
    package_ref: { id: "nearai.github" },
    display_name: "GitHub",
    description:
      "Repositories, issues, pull requests, and workflow runs through the deployment GitHub App.",
    version: "1.1.5",
    runtime: "wasm",
    install_scope: "shared",
    installation_state: "active",
    surfaces: [toolSurface, authSurface],
    tools: [
      "github.list_merged_prs",
      "github.create_pr",
      "github.get_issue",
      "github.comment",
      "github.workflow_runs",
    ],
    auth_accounts: [{ vendor: "github", accounts: [{ state: "connected" }] }],
    installed_at: iso(60 * DAY),
  },
  {
    package_ref: { id: "community.postgres-mcp" },
    display_name: "PostgreSQL",
    description:
      "Query the analytics warehouse over MCP: schema browsing, read-only SQL, and EXPLAIN plans.",
    version: "0.9.1",
    runtime: "mcp",
    install_scope: "private",
    installation_state: "setup_needed",
    surfaces: [toolSurface],
    tools: ["postgres.query", "postgres.schema", "postgres.explain"],
    auth_accounts: [],
    onboarding: {
      credential_instructions:
        "Provide a read-only connection string for the warehouse replica (postgres://…).",
      credential_next_step:
        "Ask your DBA for the reporting-replica credentials if you don't have them.",
    },
    installed_at: iso(2 * DAY),
  },
  {
    package_ref: { id: "nearai.browser" },
    display_name: "Browser",
    description:
      "Headless browsing for research tasks: open pages, extract content, and capture screenshots.",
    version: "2.0.0",
    runtime: "first_party",
    install_scope: "shared",
    installation_state: "active",
    surfaces: [toolSurface],
    tools: ["browser.open", "browser.extract", "browser.screenshot"],
    auth_accounts: [],
    installed_at: iso(90 * DAY),
  },
];

export function findInstalledExtension(id: string): InstalledExtension | undefined {
  return installedExtensions.find((extension) => extension.package_ref.id === id);
}

/* ── Registry catalog ──────────────────────────────────────────────── */

type RegistryEntry = {
  package_ref: { id: string };
  display_name: string;
  description: string;
  version: string;
  runtime: string;
  keywords: string[];
  surfaces: Surface[];
  installed: boolean;
  /** When installed from the demo registry, does it need credentials first? */
  needsSetup?: boolean;
  setupPrompt?: string;
};

const registryEntries: RegistryEntry[] = [
  // Installed extensions echoed with their catalog metadata.
  {
    package_ref: { id: "nearai.slack" },
    display_name: "Slack",
    description: "Route alerts and conversations through Slack channels and DMs.",
    version: "0.4.2",
    runtime: "wasm",
    keywords: ["chat", "channel", "alerts"],
    surfaces: [channelSurface("oauth")],
    installed: true,
  },
  {
    package_ref: { id: "nearai.telegram" },
    display_name: "Telegram",
    description: "Chat with the agent from Telegram via one-time pairing codes.",
    version: "0.3.0",
    runtime: "wasm",
    keywords: ["chat", "channel", "mobile"],
    surfaces: [channelSurface("web_generated_code")],
    installed: true,
  },
  {
    package_ref: { id: "nearai.github" },
    display_name: "GitHub",
    description: "Issues, pull requests, releases, and workflow runs.",
    version: "1.1.5",
    runtime: "wasm",
    keywords: ["git", "code review", "ci"],
    surfaces: [toolSurface, authSurface],
    installed: true,
  },
  {
    package_ref: { id: "community.postgres-mcp" },
    display_name: "PostgreSQL",
    description: "Read-only warehouse SQL over MCP.",
    version: "0.9.1",
    runtime: "mcp",
    keywords: ["sql", "database", "analytics"],
    surfaces: [toolSurface],
    installed: true,
  },
  {
    package_ref: { id: "nearai.browser" },
    display_name: "Browser",
    description: "Headless browsing, extraction, and screenshots.",
    version: "2.0.0",
    runtime: "first_party",
    keywords: ["web", "research", "screenshot"],
    surfaces: [toolSurface],
    installed: true,
  },
  // Available catalog.
  {
    package_ref: { id: "nearai.discord" },
    display_name: "Discord",
    description: "Bring the agent into Discord servers and DM conversations.",
    version: "0.2.4",
    runtime: "wasm",
    keywords: ["chat", "channel", "community"],
    surfaces: [channelSurface("oauth")],
    installed: false,
    needsSetup: true,
    setupPrompt: "Discord bot token",
  },
  {
    package_ref: { id: "nearai.email" },
    display_name: "Email",
    description: "Send and triage email through a connected mailbox.",
    version: "0.5.0",
    runtime: "wasm",
    keywords: ["email", "channel", "inbox"],
    surfaces: [channelSurface("oauth")],
    installed: false,
    needsSetup: true,
    setupPrompt: "Mailbox authorization",
  },
  {
    package_ref: { id: "community.linear-mcp" },
    display_name: "Linear",
    description: "Issues, cycles, and project updates over MCP.",
    version: "1.3.2",
    runtime: "mcp",
    keywords: ["issues", "project management"],
    surfaces: [toolSurface, authSurface],
    installed: false,
    needsSetup: true,
    setupPrompt: "Linear API key",
  },
  {
    package_ref: { id: "community.notion-mcp" },
    display_name: "Notion",
    description: "Search and edit Notion pages and databases.",
    version: "0.8.0",
    runtime: "mcp",
    keywords: ["docs", "wiki", "notes"],
    surfaces: [toolSurface, authSurface],
    installed: false,
    needsSetup: true,
    setupPrompt: "Notion integration secret",
  },
  {
    package_ref: { id: "nearai.google-calendar" },
    display_name: "Google Calendar",
    description: "Read availability and schedule events on connected calendars.",
    version: "0.6.1",
    runtime: "wasm",
    keywords: ["calendar", "scheduling"],
    surfaces: [toolSurface, authSurface],
    installed: false,
    needsSetup: true,
    setupPrompt: "Google account authorization",
  },
  {
    package_ref: { id: "community.jira" },
    display_name: "Jira",
    description: "Create and transition Jira issues from conversations.",
    version: "0.4.7",
    runtime: "wasm",
    keywords: ["issues", "tickets", "atlassian"],
    surfaces: [toolSurface],
    installed: false,
    needsSetup: true,
    setupPrompt: "Jira API token",
  },
  {
    package_ref: { id: "community.sentry" },
    display_name: "Sentry",
    description: "Look up issues, releases, and stack traces from Sentry.",
    version: "0.3.3",
    runtime: "mcp",
    keywords: ["errors", "monitoring", "observability"],
    surfaces: [toolSurface],
    installed: false,
    needsSetup: true,
    setupPrompt: "Sentry auth token",
  },
  {
    package_ref: { id: "community.stripe" },
    display_name: "Stripe",
    description: "Inspect customers, invoices, and payment activity (read-only).",
    version: "0.2.0",
    runtime: "mcp",
    keywords: ["payments", "billing", "finance"],
    surfaces: [toolSurface],
    installed: false,
    needsSetup: true,
    setupPrompt: "Stripe restricted key",
  },
  {
    package_ref: { id: "nearai.weather" },
    display_name: "Weather",
    description: "Forecasts and current conditions, no credentials required.",
    version: "1.0.2",
    runtime: "wasm",
    keywords: ["weather", "forecast"],
    surfaces: [toolSurface],
    installed: false,
  },
  {
    package_ref: { id: "community.elevenlabs" },
    display_name: "ElevenLabs",
    description: "Generate speech audio for outbound voice notes.",
    version: "0.1.9",
    runtime: "script",
    keywords: ["audio", "voice", "tts"],
    surfaces: [toolSurface],
    installed: false,
    needsSetup: true,
    setupPrompt: "ElevenLabs API key",
  },
];

export function registrySnapshot() {
  return { entries: registryEntries };
}

/* ── Setup descriptors ─────────────────────────────────────────────── */

const setupDescriptors = new Map<string, SetupDescriptor>([
  [
    "nearai.slack",
    {
      secrets: [
        {
          name: "slack.workspace",
          prompt: "Slack workspace authorization",
          provided: true,
          setup: { kind: "oauth" },
        },
      ],
      onboarding: {
        credential_instructions:
          "Authorize the IronClaw Slack app for your workspace. The deployment app credentials are managed under Admin → Configuration.",
      },
    },
  ],
  [
    "nearai.github",
    {
      secrets: [
        {
          name: "github.account",
          prompt: "GitHub account authorization",
          provided: true,
          setup: { kind: "oauth" },
        },
      ],
      onboarding: {
        credential_instructions:
          "Authorize the deployment GitHub App for the repositories you want the agent to work with.",
      },
    },
  ],
  [
    "community.postgres-mcp",
    {
      secrets: [
        {
          name: "database_url",
          prompt: "PostgreSQL connection string",
          provided: false,
          setup: { kind: "manual_token" },
        },
      ],
      onboarding: {
        credential_instructions:
          "Paste a read-only connection string for the warehouse replica (postgres://…).",
        credential_next_step:
          "Tip: use the reporting replica, not the primary — the agent only ever needs SELECT.",
        setup_url: "https://docs.near.ai/ironclaw/extensions/postgres",
      },
    },
  ],
  ["nearai.telegram", { secrets: [], onboarding: null }],
  ["nearai.browser", { secrets: [], onboarding: null }],
]);

export function setupDescriptorFor(id: string): SetupDescriptor {
  let descriptor = setupDescriptors.get(id);
  if (descriptor) return descriptor;
  const entry = registryEntries.find((item) => item.package_ref.id === id);
  descriptor = {
    secrets: entry?.needsSetup
      ? [
          {
            name: "api_key",
            prompt: entry.setupPrompt || "API key",
            provided: false,
            setup: { kind: "manual_token" },
          },
        ]
      : [],
    onboarding: entry?.needsSetup
      ? {
          credential_instructions: `Provide the ${entry.setupPrompt || "credential"} to finish connecting ${entry.display_name}.`,
        }
      : null,
  };
  setupDescriptors.set(id, descriptor);
  return descriptor;
}

/** Manual-token submit: mark secrets provided and flip the extension active. */
export function submitExtensionSetup(id: string, secretValues: Record<string, unknown>) {
  const descriptor = setupDescriptorFor(id);
  for (const secret of descriptor.secrets) {
    const value = secretValues?.[secret.name];
    if (typeof value === "string" && value.trim()) secret.provided = true;
  }
  const allProvided = descriptor.secrets.every(
    (secret) => secret.provided || secret.optional
  );
  const extension = findInstalledExtension(id);
  if (extension && allProvided) {
    extension.installation_state = "active";
    delete extension.activation_error;
  }
  return allProvided;
}

/** OAuth "start" in demo mode completes instantly: connected + active. */
export function completeExtensionOauth(id: string) {
  const descriptor = setupDescriptorFor(id);
  for (const secret of descriptor.secrets) {
    if (secret.setup.kind === "oauth") secret.provided = true;
  }
  const extension = findInstalledExtension(id);
  if (extension) {
    extension.installation_state = "active";
    const vendor = extension.auth_accounts[0];
    if (vendor?.accounts[0]) {
      vendor.accounts[0].state = "connected";
      delete vendor.accounts[0].last_error;
    } else {
      extension.auth_accounts = [
        { vendor: id.split(".").pop() || id, accounts: [{ state: "connected" }] },
      ];
    }
  }
}

export function installExtension(id: string): { name: string } | null {
  const entry = registryEntries.find((item) => item.package_ref.id === id);
  if (!entry) return null;
  entry.installed = true;
  if (!findInstalledExtension(id)) {
    installedExtensions.push({
      package_ref: { id },
      display_name: entry.display_name,
      description: entry.description,
      version: entry.version,
      runtime: entry.runtime,
      install_scope: "private",
      installation_state: entry.needsSetup ? "setup_needed" : "active",
      surfaces: entry.surfaces,
      tools: [],
      auth_accounts: [],
      ...(entry.needsSetup
        ? {
            onboarding: {
              credential_instructions: `Provide the ${entry.setupPrompt || "credential"} to finish connecting ${entry.display_name}.`,
            },
          }
        : {}),
      installed_at: new Date().toISOString(),
    });
  }
  return { name: entry.display_name };
}

export function removeExtension(id: string): boolean {
  const index = installedExtensions.findIndex(
    (extension) => extension.package_ref.id === id
  );
  if (index >= 0) installedExtensions.splice(index, 1);
  const entry = registryEntries.find((item) => item.package_ref.id === id);
  if (entry) entry.installed = false;
  return index >= 0;
}

/* ── WebGeneratedCode pairing (generic + Telegram channel routes) ──── */

type PairingState = {
  connected: boolean;
  pending: { code: string; deep_link?: string; expires_at: string } | null;
};

const pairingStates = new Map<string, PairingState>([
  ["nearai.telegram", { connected: true, pending: null }],
]);

function pairingState(id: string): PairingState {
  let state = pairingStates.get(id);
  if (!state) {
    state = { connected: false, pending: null };
    pairingStates.set(id, state);
  }
  return state;
}

export function mintPairingCode(id: string) {
  const state = pairingState(id);
  const code = `IRON-${Math.random().toString(36).slice(2, 6).toUpperCase()}`;
  state.pending = {
    code,
    deep_link: `https://t.me/ironclaw_demo_bot?start=${code}`,
    expires_at: isoAhead(10 * MINUTE),
  };
  return state.pending;
}

export function pairingStatus(id: string): PairingState {
  return pairingState(id);
}

export function unpair(id: string) {
  const state = pairingState(id);
  state.connected = false;
  state.pending = null;
}

/* ── Telegram channel setup (`/channels/telegram/*`) ───────────────── */

export const telegramSetup = {
  configured: true,
  bot_username: "ironclaw_demo_bot",
  bot_token_configured: true,
  webhook_url: "https://gateway.demo.ironclaw.dev/hooks/telegram",
  revision: 3,
};

export function saveTelegramSetup(body: Record<string, unknown>) {
  if (typeof body.bot_token === "string" && body.bot_token.trim()) {
    telegramSetup.bot_token_configured = true;
  }
  if (body.webhook_url === null) {
    telegramSetup.webhook_url = "";
  } else if (typeof body.webhook_url === "string") {
    telegramSetup.webhook_url = body.webhook_url;
  }
  telegramSetup.configured = telegramSetup.bot_token_configured;
  telegramSetup.revision += 1;
  return telegramSetup;
}

export function clearTelegramSetup() {
  telegramSetup.configured = false;
  telegramSetup.bot_token_configured = false;
  telegramSetup.webhook_url = "";
  telegramSetup.revision += 1;
}
