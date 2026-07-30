import type { Meta, StoryObj } from "@storybook/react-vite";
import { CodePanel } from "../src/composites/code-panel";

const meta: Meta = { title: "Composites/CodePanel" };
export default meta;

type Story = StoryObj;

const PAYLOAD = `{
  "job_id": "job-7f3a",
  "state": "in_progress",
  "attempts": 1,
  "runner": "runner-2"
}`;

export const Default: Story = {
  render: () => (
    <div className="w-[32rem]">
      <CodePanel>{PAYLOAD}</CodePanel>
    </div>
  ),
};

export const Wrapped: Story = {
  render: () => (
    <div className="w-[24rem]">
      <CodePanel wrap>
        {"https://gateway.demo.ironclaw.dev/hooks/telegram?token=verylongtokenvaluethatwouldotherwiseoverflowthecontainer"}
      </CodePanel>
    </div>
  ),
};
