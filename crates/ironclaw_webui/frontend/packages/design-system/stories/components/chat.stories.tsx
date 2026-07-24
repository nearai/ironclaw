import type { Meta, StoryObj } from "@storybook/react-vite";
import { Badge } from "../../src/badge";
import { Button } from "../../src/button";
import { Card, CardBody, CardFooter, CardHeader } from "../../src/card";
import { ChatMessage } from "../../src/chat";
import { Icon } from "../../src/icons";

const meta = {
  title: "Components/Chat",
  component: ChatMessage,
  parameters: {
    docs: {
      description: {
        component:
          "`ChatMessage` covers both turn shapes: `role=\"agent\"` renders the avatar with " +
          "open text (cards and other components nest as children), `role=\"user\"` renders " +
          "the right-aligned bubble. Agent copy follows the receipt pattern from Voice & " +
          "copy: past tense, the reason, the escape hatch. `AgentAvatar` is exported " +
          "separately for headers and lists.",
      },
    },
  },
  argTypes: {
    role: { control: "select", options: ["agent", "user"] },
  },
  args: { role: "agent", children: "Done. Moved the digest to 7:30am and added release notes." },
} satisfies Meta<typeof ChatMessage>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => <div className="w-[34rem]">{<ChatMessage {...args} />}</div>,
};

export const Exchange: Story = {
  render: () => (
    <div className="grid w-[34rem] gap-4">
      <ChatMessage role="agent">
        Morning. While you slept I went through the inbox: 34 newsletter threads were
        burying your real mail, so I set up a routine for them.
        <Card variant="subtle" radius="sm" padding="none">
          <CardHeader className="!py-3">
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-2">
                <Icon name="bolt" className="h-4 w-4 text-[var(--v2-accent-text)]" />
                <span className="text-sm font-medium text-[var(--v2-text-strong)]">
                  Morning digest
                </span>
              </div>
              <Badge tone="success" label="Scheduled" size="sm" />
            </div>
          </CardHeader>
          <CardBody className="!py-0 text-xs leading-5 text-[var(--v2-text-muted)]">
            First run tomorrow · <span className="font-mono">8:00am</span>.
          </CardBody>
          <CardFooter divider={false} className="!pt-3 !pb-3">
            <div className="flex gap-2">
              <Button variant="secondary" size="sm">
                Adjust
              </Button>
              <Button variant="ghost" size="sm">
                Undo
              </Button>
            </div>
          </CardFooter>
        </Card>
      </ChatMessage>
      <ChatMessage role="user">Move it to 7:30 and include GitHub release notes too.</ChatMessage>
      <ChatMessage role="agent">
        Done. Moved to <span className="font-mono text-xs">7:30am</span> and watching 3 repos
        for releases.
      </ChatMessage>
    </div>
  ),
};
