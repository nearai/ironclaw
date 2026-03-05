// Documentation metadata for Routines UI

export const docMetadata = {
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
};
