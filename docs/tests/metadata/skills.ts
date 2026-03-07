// Documentation metadata for Skills UI

export const docMetadata = {
  page: 'skills',
  title: 'Skills Tab',
  description: "Manage and discover skills that extend IronClaw's capabilities",
  screenshots: [
    {
      filename: 'web-skills-list.png',
      title: 'Installed Skills',
      description: 'View all installed skills with their trust level, version, and activation status. Skills extend IronClaw\'s capabilities with domain-specific instructions.',
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
};
