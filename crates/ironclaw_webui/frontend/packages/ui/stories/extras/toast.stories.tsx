import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Toast,
  ToastAction,
  ToastClose,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
  Toaster,
  toast,
} from "../../src/extras/toast";
import { Button } from "../../src/components/button";

const meta: Meta = { title: "Extras/Toast" };
export default meta;

type Story = StoryObj;

function ImperativeDemo() {
  return (
    <>
      <div className="flex gap-2">
        <Button
          variant="secondary"
          size="sm"
          onClick={() => toast({ title: "Run started", description: "researcher · run-4822" })}
        >
          Default
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => toast({ title: "Run complete", tone: "positive" })}
        >
          Positive
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={() =>
            toast({ title: "Budget at 80%", tone: "warning", description: "Consider pausing." })}
        >
          Warning
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={() =>
            toast({ title: "Run failed", tone: "danger", description: "Tool call timed out." })}
        >
          Danger
        </Button>
      </div>
      <Toaster />
    </>
  );
}

export const Imperative: Story = { render: () => <ImperativeDemo /> };

function ComposedDemo() {
  const [open, setOpen] = React.useState(true);
  return (
    <ToastProvider>
      <Button variant="secondary" size="sm" onClick={() => setOpen(true)}>
        Show composed toast
      </Button>
      <Toast open={open} onOpenChange={setOpen} tone="positive" duration={60000}>
        <div className="flex min-w-0 flex-col gap-0.5 pr-5">
          <ToastTitle>Deploy finished</ToastTitle>
          <ToastDescription>webui-v2 → production</ToastDescription>
        </div>
        <ToastAction altText="View deploy logs" onClick={() => setOpen(false)}>
          View
        </ToastAction>
        <ToastClose />
      </Toast>
      <ToastViewport />
    </ToastProvider>
  );
}

export const Composed: Story = { render: () => <ComposedDemo /> };
