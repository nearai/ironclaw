import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import type * as React from "react";

import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center justify-center border-2 px-2.5 py-0.5 text-xs font-medium w-fit whitespace-nowrap shrink-0 [&>svg]:size-3 gap-1 [&>svg]:pointer-events-none shadow-sm transition-all duration-200 ease-out rounded-[6px]",
  {
    variants: {
      variant: {
        default: "border-outset border-border-strong bg-secondary text-foreground",
        secondary:
          "border-outset border-muted-foreground/40 bg-secondary text-secondary-foreground",
        destructive: "border-outset border-destructive bg-destructive text-destructive-foreground",
        outline: "border-outset border-foreground bg-background text-foreground",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

function Badge({
  className,
  variant,
  asChild = false,
  ...props
}: React.ComponentProps<"span"> & VariantProps<typeof badgeVariants> & { asChild?: boolean }) {
  const Comp = asChild ? Slot : "span";

  return (
    <Comp data-slot="badge" className={cn(badgeVariants({ variant }), className)} {...props} />
  );
}

export { Badge, badgeVariants };
