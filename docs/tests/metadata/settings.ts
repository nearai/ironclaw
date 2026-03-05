// Documentation metadata for Settings UI

export const docMetadata = {
  page: 'settings',
  title: 'Settings Tab',
  description: 'Configure IronClaw providers, extensions, and system preferences',
  screenshots: [
    {
      filename: 'web-settings-overview.png',
      title: 'Settings Overview',
      description: 'Configure your IronClaw instance. Set up LLM providers, manage extensions, and adjust system preferences.',
      elements: [
        {
          selector: '.settings-sections',
          name: 'Settings Sections',
          description: 'Organized categories of configuration options',
          interaction: 'Click a section to expand and view its settings',
        },
        {
          selector: '[data-testid="provider-settings"]',
          name: 'Provider Configuration',
          description: 'Configure LLM providers (NEAR AI, OpenAI, Anthropic, etc.)',
          interaction: 'Select provider, enter API key, test connection',
        },
        {
          selector: '[data-testid="extensions-list"]',
          name: 'Extensions',
          description: 'Installed MCP and WASM extensions',
          interaction: 'View status, configure, or remove extensions',
        },
        {
          selector: '[data-testid="connection-status"]',
          name: 'Connection Status',
          description: 'Shows health of connected services',
          interaction: 'Click to view detailed connection diagnostics',
        },
      ],
      usage: 'Use Settings to configure IronClaw to work with your preferred providers and extensions. Start by setting up at least one LLM provider, then add extensions for additional capabilities.',
      related: ['skills', 'chat'],
    },
  ],
};
