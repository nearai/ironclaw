import type { Meta, StoryObj } from "@storybook/react-vite";
import { FlowList } from "../src/components/flow-list";

const meta: Meta = { title: "Components/FlowList" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <div className="w-[28rem]">
      <FlowList
        items={[
          { title: "Connect a provider", description: "Add an LLM provider key in settings." },
          { title: "Start a chat", description: "The agent gets a workspace and tool access." },
          { title: "Automate", description: "Promote a recurring prompt into an automation." },
        ]}
      />
    </div>
  ),
};
