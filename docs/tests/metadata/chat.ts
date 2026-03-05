// Documentation metadata for Chat UI

export const docMetadata = {
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
};
