import type { Meta, StoryObj } from "@storybook/react-vite";

import { withQueryClient } from "../test-support/storybook-decorators";
import { TelegramSetupPanel } from "./telegram-setup-panel";

// TelegramSetupPanel takes the setup query as an injected prop, so stories pass
// a plain mock query result — no network. It still needs a QueryClient for its
// save/remove mutations.
function mockQuery(data: unknown, overrides: Record<string, unknown> = {}) {
  return { data, isError: false, isLoading: false, error: null, ...overrides };
}

const meta = {
  title: "Components/TelegramSetupPanel",
  component: TelegramSetupPanel,
  decorators: [
    withQueryClient(),
    (Story) => (
      <div className="max-w-xl">
        <Story />
      </div>
    ),
  ],
  args: {
    action: null,
    setupQuery: mockQuery({ configured: false, bot_token_configured: false }),
  },
  tags: ["ai-generated"],
} satisfies Meta<typeof TelegramSetupPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Unconfigured: Story = {};

export const Configured: Story = {
  args: {
    setupQuery: mockQuery({
      configured: true,
      bot_token_configured: true,
      bot_username: "ironclaw_bot",
      webhook_url: "https://example.com/telegram/webhook",
      revision: 2,
    }),
  },
};

export const LoadError: Story = {
  args: {
    setupQuery: mockQuery(undefined, { isError: true, error: new Error("Failed to load setup") }),
  },
};
