import { writeFileSync, mkdirSync } from 'fs';
import { join } from 'path';

interface ElementMetadata {
  selector: string;
  name: string;
  description: string;
  interaction: string;
}

interface ScreenshotMetadata {
  filename: string;
  title: string;
  description: string;
  elements: ElementMetadata[];
  usage: string;
  related: string[];
}

interface DocMetadata {
  page: string;
  title: string;
  description: string;
  screenshots: ScreenshotMetadata[];
}

import { fileURLToPath } from 'url';
const __dirname = fileURLToPath(new URL('.', import.meta.url));

// Load metadata from spec files
// In a real implementation, we'd dynamically import the spec files
// For now, we define the metadata here to match the spec exports
const allMetadata: DocMetadata[] = [
  {
    page: 'chat',
    title: 'Chat Interface',
    description: 'The main chat interface for interacting with IronClaw',
    screenshots: [
      {
        filename: 'web-chat-overview.png',
        title: 'Chat Overview',
        description: 'The primary interface for communicating with IronClaw. View your conversation history, send new messages, and see responses stream in real-time.',
        elements: [
          {
            selector: '.message-list',
            name: 'Message History',
            description: 'Displays the conversation history between you and IronClaw',
            interaction: 'Scroll to view older messages; click to select text',
          },
          {
            selector: '[data-testid="chat-input"]',
            name: 'Message Input',
            description: 'Type your messages or commands here',
            interaction: 'Click to focus, type your message, press Enter to send',
          },
          {
            selector: '[data-testid="streaming-indicator"]',
            name: 'Streaming Indicator',
            description: 'Shows when IronClaw is generating a response',
            interaction: 'Wait for the indicator to disappear before sending follow-up',
          },
        ],
        usage: 'Use the chat interface to ask questions, run commands, or have IronClaw perform tasks. Type your request and press Enter.',
        related: ['skills', 'routines'],
      },
    ],
  },
  {
    page: 'skills',
    title: 'Skills Tab',
    description: "Manage and discover skills that extend IronClaw's capabilities",
    screenshots: [
      {
        filename: 'web-skills-list.png',
        title: 'Installed Skills',
        description: "View all installed skills with their trust level, version, and activation status. Skills extend IronClaw's capabilities with domain-specific instructions.",
        elements: [
          {
            selector: '.skills-list',
            name: 'Skills List',
            description: 'Displays all installed skills with metadata',
            interaction: 'Scroll to view all skills; click a skill to view details',
          },
          {
            selector: '[data-testid="skill-search"]',
            name: 'Skill Search',
            description: 'Search for skills by name or description',
            interaction: 'Type to filter the skills list in real-time',
          },
          {
            selector: '[data-testid="install-skill-btn"]',
            name: 'Install Skill Button',
            description: 'Opens the skill installation dialog',
            interaction: 'Click to browse and install new skills from the registry',
          },
        ],
        usage: "Use the Skills tab to manage what IronClaw knows. Install skills from the registry to add new capabilities, or view installed skills to understand what's available.",
        related: ['settings', 'chat'],
      },
    ],
  },
  {
    page: 'routines',
    title: 'Routines Tab',
    description: 'Manage scheduled and event-triggered routines for automated task execution',
    screenshots: [
      {
        filename: 'web-routines-overview.png',
        title: 'Routines Overview',
        description: 'View and manage all your routines. Routines are automated workflows that trigger on a schedule or in response to events.',
        elements: [
          {
            selector: '.routines-list',
            name: 'Routines List',
            description: 'Lists all cron and event-triggered routines',
            interaction: 'Scroll to view all routines; click to edit',
          },
          {
            selector: '[data-testid="cron-routines"]',
            name: 'Cron Routines',
            description: 'Scheduled routines that run at specific times',
            interaction: 'View next run time; toggle enabled/disabled',
          },
          {
            selector: '[data-testid="event-routines"]',
            name: 'Event Routines',
            description: 'Reactive routines triggered by system events',
            interaction: 'View trigger conditions; edit actions',
          },
          {
            selector: '[data-testid="create-routine-btn"]',
            name: 'Create Routine Button',
            description: 'Starts the routine creation workflow',
            interaction: 'Click to define a new scheduled or event-triggered routine',
          },
        ],
        usage: 'Use Routines to automate repetitive tasks. Create cron routines for periodic checks (like every 6 hours) or event routines that respond to system changes.',
        related: ['settings', 'chat'],
      },
    ],
  },
  {
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
  },
  {
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
  },
  {
    page: 'memory',
    title: 'Memory Tab',
    description: 'Search and manage persistent memory and workspace documents',
    screenshots: [
      {
        filename: 'web-memory-overview.png',
        title: 'Memory Overview',
        description: 'Search through your persistent memory using hybrid search (full-text + semantic). Access workspace documents and conversation history.',
        elements: [
          {
            selector: '.memory-search',
            name: 'Memory Search',
            description: 'Hybrid search across all memory documents',
            interaction: 'Type to search; results ranked by relevance',
          },
          {
            selector: '[data-testid="memory-tree"]',
            name: 'Memory Tree',
            description: 'Hierarchical view of memory documents',
            interaction: 'Click folders to expand; click documents to view',
          },
          {
            selector: '[data-testid="search-results"]',
            name: 'Search Results',
            description: 'Matching documents with relevance scores',
            interaction: 'Click a result to view the full document',
          },
          {
            selector: '[data-testid="new-document-btn"]',
            name: 'New Document Button',
            description: 'Creates a new memory document',
            interaction: 'Click to add a new document to your workspace',
          },
        ],
        usage: 'Use Memory to recall past conversations and access stored documents. The hybrid search combines full-text and semantic matching to find relevant information.',
        related: ['chat', 'skills'],
      },
    ],
  },
];

function generateFrontmatter(metadata: DocMetadata): string {
  // Escape double quotes in content to prevent frontmatter parsing issues
  const escapedTitle = metadata.title.replace(/"/g, '\\"');
  const escapedDescription = metadata.description.replace(/"/g, '\\"');
  return `---
title: "${escapedTitle}"
description: "${escapedDescription}"
---

`;
}

function generateScreenshotSection(screenshot: ScreenshotMetadata): string {
  const elementsList = screenshot.elements
    .map((el) => `- **${el.name}**: ${el.description}`)
    .join('\n');

  const interactionsList = screenshot.elements
    .map((el) => `- **${el.name}**: ${el.interaction}`)
    .join('\n');

  const relatedSection =
    screenshot.related.length > 0
      ? `\n### Related Features\n\n${screenshot.related.map((r) => `- [${r}](/ui-reference/${r})`).join('\n')}`
      : '';

  return `## ${screenshot.title}

${screenshot.description}

<Frame>
  <img src="/assets/screenshots/${screenshot.filename}" alt="${screenshot.title}" />
</Frame>

### UI Elements

${elementsList}

### How to Interact

${interactionsList}

### Usage

${screenshot.usage}${relatedSection}

---

`;
}

function generateDoc(metadata: DocMetadata): string {
  let doc = generateFrontmatter(metadata);

  for (const screenshot of metadata.screenshots) {
    doc += generateScreenshotSection(screenshot);
  }

  return doc;
}

// Main execution
async function main() {
  const outputDir = join(__dirname, '..', 'ui-reference');

  // Ensure output directory exists
  mkdirSync(outputDir, { recursive: true });

  console.log('Generating UI reference documentation...\n');

  for (const meta of allMetadata) {
    const doc = generateDoc(meta);
    const outputPath = join(outputDir, `${meta.page}.mdx`);
    writeFileSync(outputPath, doc);
    console.log(`Generated: ${outputPath}`);
  }

  console.log('\nDocumentation generation complete!');
  console.log('Review the generated files in docs/ui-reference/ before committing.');
}

main().catch(console.error);
