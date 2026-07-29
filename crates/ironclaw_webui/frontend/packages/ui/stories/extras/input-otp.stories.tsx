import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { InputOTP } from "../../src/extras/input-otp";

const meta: Meta = { title: "Extras/InputOTP" };
export default meta;

type Story = StoryObj;

function Demo(props: { length?: number; disabled?: boolean }) {
  const [code, setCode] = React.useState("");
  const [done, setDone] = React.useState(false);
  return (
    <div className="flex flex-col items-center gap-3">
      <InputOTP
        value={code}
        onChange={(next) => {
          setCode(next);
          setDone(false);
        }}
        onComplete={() => setDone(true)}
        {...props}
      />
      <span className="text-ui-sm text-[var(--v2-text-muted)]">
        {done ? "Code complete ✓" : `Typed: ${code || "—"}`}
      </span>
    </div>
  );
}

export const SixDigits: Story = { render: () => <Demo /> };
export const FourDigits: Story = { render: () => <Demo length={4} /> };
export const Disabled: Story = {
  render: () => <InputOTP value="42" onChange={() => {}} disabled />,
};
