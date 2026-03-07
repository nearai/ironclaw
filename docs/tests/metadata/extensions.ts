// Documentation metadata for Extensions UI

export const docMetadata = {
  page: 'extensions',
  title: 'Extensions Tab',
  description: 'Manage MCP and WASM extensions that add new tools and capabilities',
  screenshots: [
    {
      filename: 'web-extensions-overview.png',
      title: 'Extensions Overview',
      description: 'View and manage installed extensions. Extensions add new tools and capabilities to IronClaw through the MCP protocol or WASM runtime.',
      elements: [
        {
          selector: '.extensions-list',
          name: 'Extensions List',
          description: 'Displays all installed MCP and WASM extensions',
          interaction: 'Scroll to view all extensions; click to view details',
        },
        {
          selector: '[data-testid="mcp-extensions"]',
          name: 'MCP Extensions',
          description: 'Model Context Protocol extensions that provide tools',
          interaction: 'View tools provided; toggle enabled/disabled',
        },
        {
          selector: '[data-testid="wasm-extensions"]',
          name: 'WASM Extensions',
          description: 'Sandboxed WebAssembly extensions',
          interaction: 'View capabilities; manage permissions',
        },
        {
          selector: '[data-testid="install-extension-btn"]',
          name: 'Install Extension Button',
          description: 'Opens the extension installation dialog',
          interaction: 'Click to browse and install new extensions from the registry',
        },
      ],
      usage: 'Use Extensions to add new tools to IronClaw. Install MCP servers for ecosystem integrations or WASM tools for sandboxed custom functionality.',
      related: ['settings', 'skills'],
    },
  ],
};
