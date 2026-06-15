import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import type * as React from "react";

import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap text-sm font-medium cursor-pointer transition-all duration-200 ease-out disabled:pointer-events-none disabled:opacity-50 disabled:cursor-not-allowed [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4 shrink-0 [&_svg]:shrink-0 outline-none focus-visible:ring-2 focus-visible:ring-brand-accent focus-visible:ring-offset-2 border-2 border-outset border-border-strong shadow-sm hover:shadow-md active:border-inset active:shadow-none rounded-[12px]",
  {
    variants: {
      variant: {
        default: "bg-foreground text-background hover:opacity-90",
        destructive:
          "bg-destructive text-destructive-foreground border-destructive hover:opacity-90",
        outline: "bg-card text-foreground hover:bg-muted",
        secondary: "bg-secondary text-secondary-foreground hover:opacity-90",
        ghost:
          "border-transparent shadow-none hover:bg-muted hover:shadow-sm active:shadow-none active:border-transparent",
        link: "border-transparent shadow-none text-link underline-offset-4 hover:underline hover:shadow-none active:shadow-none active:border-transparent",
      },
      size: {
        default: "h-10 px-4 py-2 has-[>svg]:px-3",
        sm: "h-8 px-3 py-1.5 has-[>svg]:px-2.5 text-xs",
        lg: "h-12 px-6 py-3 has-[>svg]:px-4",
        icon: "size-10",
        "icon-sm": "size-8",
        "icon-lg": "size-12",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

function Button({
  className,
  variant,
  size,
  asChild = false,
  ...props
}: React.ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean;
  }) {
  const Comp = asChild ? Slot : "button";

  return (
    <Comp
      data-slot="button"
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  );
}

export { Button, buttonVariants };
