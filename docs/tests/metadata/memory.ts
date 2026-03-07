// Documentation metadata for Memory UI

export const docMetadata = {
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
};
