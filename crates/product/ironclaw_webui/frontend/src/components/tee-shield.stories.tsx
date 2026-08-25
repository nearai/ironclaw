import type { Meta, StoryObj } from "@storybook/react-vite";

import { TeeShield } from "./tee-shield";

const meta = {
  title: "Components/TeeShield",
  component: TeeShield,
  parameters: {
    layout: "centered",
    docs: {
      description: {
        component:
          "A verified-enclave (TEE) attestation shield shown in the page header. It is host-gated: " +
          "`getTeeEndpoint` returns no endpoint on localhost / IP hosts, so `available` is false and the " +
          "component renders nothing. It only appears on a real attested deployment host.",
      },
    },
  },
  tags: ["ai-generated"],
} satisfies Meta<typeof TeeShield>;

export default meta;
type Story = StoryObj<typeof meta>;

// Renders nothing on localhost (see note above); paired with an explanation so
// the catalog documents its existence and behavior.
export const LocalhostNull: Story = {
  render: () => (
    <div className="flex max-w-md flex-col gap-3">
      <div className="flex items-center gap-3">
        <TeeShield />
        <span className="text-xs text-[var(--v2-text-muted)]">
          ← TeeShield mounts here (empty on localhost)
        </span>
      </div>
      <p className="text-xs leading-5 text-[var(--v2-text-muted)]">
        On a real *.deployment host it shows a green enclave shield that opens an attestation summary
        (image digest, TLS fingerprint, report data). It is intentionally invisible in local/dev
        environments.
      </p>
    </div>
  ),
};
