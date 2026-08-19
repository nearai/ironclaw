import type { Meta, StoryObj } from "@storybook/react-vite";

import { withQueryClient, withStubbedFetch } from "../test-support/storybook-decorators";
import { PairingWebCodePanel } from "./pairing-web-code-panel";

// PairingWebCodePanel fetches imperatively on mount: it reads pairing status and
// mints a code when disconnected. Those go straight through `apiFetch`, which a
// `withQueryClient` cache seed cannot intercept — so the stories stub `fetch`
// with `withStubbedFetch`. Without a stub a shared/deployed Storybook would
// perform a real network round-trip (and could mint a live pairing code).
const PAIRING = "extensions/telegram/pairing";

// A live pending code keeps the panel in its happy path (renders the code + QR
// + countdown) without ever calling mint — the status effect adopts it. The
// factory keeps `expires_at` in the future on every poll so the countdown stays
// live and deterministic.
const livePending = () => ({
  connected: false,
  pending: {
    code: "IRON-4F2K-9QZ",
    deep_link: "https://t.me/ironclaw_bot?start=IRON-4F2K-9QZ",
    expires_at: new Date(Date.now() + 10 * 60_000).toISOString(),
  },
});

const meta = {
  title: "Components/PairingWebCodePanel",
  component: PairingWebCodePanel,
  decorators: [
    (Story) => (
      <div className="max-w-md">
        <Story />
      </div>
    ),
    withQueryClient(),
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
          "Stories stub `fetch` so the panel never reaches a real backend: the default stories " +
          "serve a deterministic pending code, and `MintError` serves a failure so the panel " +
          "settles into its 'get a new code' error state.",
      },
    },
  },
  tags: ["ai-generated"],
} satisfies Meta<typeof PairingWebCodePanel>;

export default meta;
type Story = StoryObj<typeof meta>;

const withLivePairing = withStubbedFetch([
  { match: `${PAIRING}/status`, json: livePending },
  { match: `${PAIRING}/mint`, method: "POST", json: () => livePending().pending },
]);

const withFailingPairing = withStubbedFetch([
  { match: `${PAIRING}/status`, status: 503, json: { kind: "service_unavailable" } },
  { match: `${PAIRING}/mint`, method: "POST", status: 503, json: { kind: "service_unavailable" } },
]);

export const Default: Story = { decorators: [withLivePairing] };
export const Compact: Story = { args: { compact: true }, decorators: [withLivePairing] };

// The backend is unavailable, so the panel settles into its error / retry state.
export const MintError: Story = { decorators: [withFailingPairing] };
