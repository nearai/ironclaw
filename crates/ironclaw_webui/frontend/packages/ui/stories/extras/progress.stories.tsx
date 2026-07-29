import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Progress } from "../../src/extras/progress";

const meta: Meta = { title: "Extras/Progress" };
export default meta;

type Story = StoryObj;

function Animated() {
  const [value, setValue] = React.useState(15);
  React.useEffect(() => {
    const timer = setInterval(
      () => setValue((current) => (current >= 100 ? 0 : current + 12)),
      900
    );
    return () => clearInterval(timer);
  }, []);
  return <Progress value={value} aria-label="Sync progress" className="w-72" />;
}

export const Default: Story = {
  render: () => <Progress value={64} aria-label="Upload" className="w-72" />,
};

export const Live: Story = { render: () => <Animated /> };

export const Tones: Story = {
  render: () => (
    <div className="flex w-72 flex-col gap-3">
      <Progress value={80} tone="accent" aria-label="Accent" />
      <Progress value={100} tone="positive" aria-label="Positive" />
      <Progress value={55} tone="warning" aria-label="Warning" />
      <Progress value={25} tone="danger" aria-label="Danger" />
    </div>
  ),
};
