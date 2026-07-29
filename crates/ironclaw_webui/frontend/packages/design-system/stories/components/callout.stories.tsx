import type { Meta, StoryObj } from "@storybook/react-vite";
import { Callout } from "../../src/callout";

const meta = {
  title: "Components/Feedback/Callout",
  component: Callout,
  parameters: {
    docs: {
      description: {
        component:
          "Inline notice panel for guidance, caveats, and status context. Tones follow " +
          "`STATUS_CANON` plus `accent` for product highlights; the icon is derived from " +
          "the tone and can be overridden or removed (`icon={null}`). Copy inside follows " +
          "the Voice & copy rules: calm, plain, no alarm-speak. The docs pages themselves " +
          "use this component for their editorial callouts.",
      },
    },
  },
  argTypes: {
    tone: {
      control: "select",
      options: ["info", "accent", "success", "warning", "danger", "muted"],
    },
    title: { control: "text" },
  },
  args: {
    tone: "info",
    title: "Heads up",
    children: "Connected tools sync every 15 minutes. Trigger a manual sync from Settings.",
  },
} satisfies Meta<typeof Callout>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Tones: Story = {
  render: () => (
    <div className="grid w-[34rem] gap-3">
      <Callout tone="info" title="Scheduled maintenance">
        Runs pause on Sunday between 2:00am and 2:30am UTC. Nothing is lost; queued work
        resumes automatically.
      </Callout>
      <Callout tone="accent" title="New: derived routines">
        The agent now proposes routines from your connected tools. Every one arrives with
        Adjust, Pause, and Undo.
      </Callout>
      <Callout tone="success" title="Backup verified">
        Last vault snapshot completed and passed integrity checks.
      </Callout>
      <Callout tone="warning" title="Token expires soon">
        The GitHub token expires in 3 days. Rotate it in Settings to avoid paused runs.
      </Callout>
      <Callout tone="danger" title="Connection lost">
        Gmail stopped responding at 9:14am. Runs that need it are paused, not failed.
      </Callout>
    </div>
  ),
};

export const WithoutIconOrTitle: Story = {
  render: () => (
    <div className="grid w-[34rem] gap-3">
      <Callout tone="muted" icon={null}>
        Plain body-only callout for low-emphasis notes.
      </Callout>
    </div>
  ),
};
