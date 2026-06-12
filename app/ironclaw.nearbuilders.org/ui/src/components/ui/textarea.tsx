import type * as React from "react";
import { cn } from "@/lib/utils";

function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "flex min-h-[80px] w-full border-2 border-inset border-border-strong bg-card px-3 py-2 text-sm shadow-inner placeholder:text-muted-foreground transition-all duration-200 ease-out focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-0 disabled:cursor-not-allowed disabled:opacity-50 rounded-[8px]",
        className,
      )}
      {...props}
    />
  );
}

export { Textarea };
