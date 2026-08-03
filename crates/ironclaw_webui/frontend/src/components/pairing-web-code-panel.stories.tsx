import type { Meta, StoryObj } from "@storybook/react-vite";

import { withQueryClient } from "../test-support/storybook-decorators";
import { PairingWebCodePanel } from "./pairing-web-code-panel";

const meta = {
  title: "Components/PairingWebCodePanel",
  component: PairingWebCodePanel,
  decorators: [
    withQueryClient(),
    (Story) => (
      <div className="max-w-md">
        <Story />
      </div>
    ),
  ],
  args: {
    extensionId: "telegram",
    displayName: "Telegram",
    instructions: "Open Telegram and send this code to @ironclaw_bot to connect.",
    compact: false,
  },
  parameters: {
    docs: {
      description: {
        component:
          "Mints a pairing code, renders it (with QR + countdown), and polls until connected. " +
          "In Storybook there is no pairing backend, so it settles into its error / 'get a new code' " +
          "state — the happy path requires a live `/extensions/{id}/pairing` endpoint (or MSW).",
      },
    },
  },
  tags: ["ai-generated"],
} satisfies Meta<typeof PairingWebCodePanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const Compact: Story = { args: { compact: true } };
