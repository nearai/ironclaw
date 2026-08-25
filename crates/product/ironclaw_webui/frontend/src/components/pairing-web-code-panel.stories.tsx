import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, waitFor } from "storybook/test";

import { withQueryClient, withStubbedFetch } from "../test-support/storybook-decorators";
import { PairingWebCodePanel } from "./pairing-web-code-panel";

// PairingWebCodePanel fetches imperatively on mount: it reads pairing status and
// mints a code when disconnected. Those go straight through `apiFetch`, which a
// `withQueryClient` cache seed cannot intercept — so the stories stub `fetch`
// with `withStubbedFetch`. Without a stub a shared/deployed Storybook would
// perform a real network round-trip (and could mint a live pairing code).
const PAIRING = "extensions/example-chat/pairing";

// A live pending code keeps the panel in its happy path (renders the code + QR
// + countdown) without ever calling mint — the status effect adopts it. The
// factory keeps `expires_at` in the future on every poll so the countdown stays
// live and deterministic.
const livePending = () => ({
  connected: false,
  pending: {
    code: "IRON-4F2K-9QZ",
    deep_link: "https://example.test/connect?start=IRON-4F2K-9QZ",
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
    extensionId: "example-chat",
    displayName: "Example Chat",
    instructions: "Open Example Chat and send this code to the assistant bot to connect.",
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

// Counts every mint the panel attempts. The stub calls a route's `json` factory
// per matched request, so incrementing here records the POST the retry button
// fires — the difference between "the button re-rendered" and "the panel really
// re-ran its mint path".
let mintAttempts = 0;
const failedMint = () => {
  mintAttempts += 1;
  return { kind: "service_unavailable" };
};

const withFailingPairing = withStubbedFetch([
  { match: `${PAIRING}/status`, status: 503, json: { kind: "service_unavailable" } },
  { match: `${PAIRING}/mint`, method: "POST", status: 503, json: failedMint },
]);

export const Default: Story = { decorators: [withLivePairing] };
export const Compact: Story = { args: { compact: true }, decorators: [withLivePairing] };

// The backend is unavailable, so the panel settles into its error / retry state.
// Bootstrap only reads status (which fails before it ever mints), so the play
// function drives the retry itself: the "get a new code" button is the only
// path to `renew()`, and a failing retry must land back on the alert rather
// than a stuck spinner or a blank panel.
export const MintError: Story = {
  decorators: [withFailingPairing],
  play: async ({ canvas, userEvent }) => {
    await expect(await canvas.findByRole("alert")).toBeVisible();
    const attemptsBefore = mintAttempts;

    await userEvent.click(canvas.getByTestId("pairing-new-code"));

    // `renew()` clears the error, mints, and the stubbed POST fails again.
    await waitFor(() => expect(mintAttempts).toBe(attemptsBefore + 1));
    await waitFor(async () => {
      await expect(canvas.getByRole("alert")).toBeVisible();
      await expect(canvas.getByTestId("pairing-new-code")).toBeEnabled();
    });
  },
};
