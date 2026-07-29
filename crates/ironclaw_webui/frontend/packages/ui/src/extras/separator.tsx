/**
 * Separator
 *
 * Semantic divider line built on @radix-ui/react-separator. Horizontal by
 * default; decorative unless told otherwise (matching shadcn's default).
 *
 * Usage
 *   <Separator />
 *   <Separator orientation="vertical" className="h-4" />
 */
import * as SeparatorPrimitive from "@radix-ui/react-separator";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

export function Separator({
  className,
  orientation = "horizontal",
  decorative = true,
  ...props
}: ComponentProps<typeof SeparatorPrimitive.Root>) {
  return (
    <SeparatorPrimitive.Root
      orientation={orientation}
      decorative={decorative}
      className={cn(
        "shrink-0 bg-[var(--v2-panel-border)]",
        orientation === "horizontal" ? "h-px w-full" : "h-full w-px",
        className
      )}
      {...props}
    />
  );
}
