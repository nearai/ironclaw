/**
 * Resizable
 *
 * Draggable split layouts built on react-resizable-panels v4 (the same dep
 * shadcn uses — v4 renamed PanelGroup/PanelResizeHandle to Group/Separator).
 * The handle renders a token-colored divider with an optional grip pill;
 * keyboard resizing and the separator role come from the library.
 *
 * Usage
 *   <ResizablePanelGroup orientation="horizontal" className="h-64">
 *     <ResizablePanel defaultSize="30%">Sidebar</ResizablePanel>
 *     <ResizableHandle withHandle />
 *     <ResizablePanel>Content</ResizablePanel>
 *   </ResizablePanelGroup>
 */
import React, { type ComponentProps } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { cn } from "../primitives/cn";

type ResizableOrientation = "horizontal" | "vertical";

/** Lets ResizableHandle orient its divider without a repeated prop. */
const OrientationContext =
  React.createContext<ResizableOrientation>("horizontal");

export function ResizablePanelGroup({
  className,
  orientation = "horizontal",
  ...props
}: ComponentProps<typeof Group>) {
  return (
    <OrientationContext.Provider value={orientation}>
      <Group
        orientation={orientation}
        className={cn("h-full w-full", className)}
        {...props}
      />
    </OrientationContext.Provider>
  );
}

export const ResizablePanel = Panel;

type ResizableHandleProps = ComponentProps<typeof Separator> & {
  /** Render a small grip pill on the divider. */
  withHandle?: boolean;
};

export function ResizableHandle({
  className,
  withHandle = false,
  ...props
}: ResizableHandleProps) {
  const orientation = React.useContext(OrientationContext);
  return (
    <Separator
      className={cn(
        "relative flex items-center justify-center bg-[var(--v2-panel-border)] transition-colors",
        orientation === "horizontal" ? "w-px" : "h-px",
        "hover:bg-[var(--v2-accent)] active:bg-[var(--v2-accent)]",
        "focus-visible:outline-none focus-visible:ring-2",
        "focus-visible:ring-[var(--v2-focus-ring)]",
        className
      )}
      {...props}
    >
      {withHandle && (
        <span
          className={cn(
            "z-10 grid place-items-center rounded-[4px]",
            "border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
            orientation === "horizontal" ? "h-6 w-2" : "h-2 w-6"
          )}
        >
          <span
            className={cn(
              "bg-[var(--v2-text-faint)]",
              orientation === "horizontal" ? "h-2.5 w-px" : "h-px w-2.5"
            )}
          />
        </span>
      )}
    </Separator>
  );
}
