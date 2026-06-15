import * as SheetPrimitive from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import type * as React from "react";

import { cn } from "@/lib/utils";

function Sheet({ ...props }: React.ComponentProps<typeof SheetPrimitive.Root>) {
  return <SheetPrimitive.Root data-slot="sheet" {...props} />;
}

function SheetTrigger({ ...props }: React.ComponentProps<typeof SheetPrimitive.Trigger>) {
  return <SheetPrimitive.Trigger data-slot="sheet-trigger" {...props} />;
}

function SheetClose({ ...props }: React.ComponentProps<typeof SheetPrimitive.Close>) {
  return <SheetPrimitive.Close data-slot="sheet-close" {...props} />;
}

function SheetPortal({ ...props }: React.ComponentProps<typeof SheetPrimitive.Portal>) {
  return <SheetPrimitive.Portal data-slot="sheet-portal" {...props} />;
}

function SheetOverlay({
  className,
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Overlay>) {
  return (
    <SheetPrimitive.Overlay
      data-slot="sheet-overlay"
      className={cn(
        "fixed inset-0 z-50 bg-foreground/30 backdrop-blur-[2px]",
        "data-[state=open]:animate-in data-[state=open]:fade-in-0",
        "data-[state=closed]:animate-out data-[state=closed]:fade-out-0",
        "data-[state=closed]:duration-250 data-[state=open]:duration-300",
        className,
      )}
      {...props}
    />
  );
}

function SheetContent({
  className,
  children,
  side = "bottom",
  hideCloseButton = false,
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Content> & {
  side?: "top" | "right" | "bottom" | "left";
  hideCloseButton?: boolean;
}) {
  return (
    <SheetPortal>
      <SheetOverlay />
      <SheetPrimitive.Content
        data-slot="sheet-content"
        className={cn(
          "fixed z-50 flex flex-col",
          "bg-card text-foreground",
          "border-2 border-outset border-border-strong",
          "shadow-[0_-8px_40px_rgba(0,0,0,0.18)]",
          "transition ease-in-out",
          "data-[state=closed]:duration-250 data-[state=open]:duration-350",
          "data-[state=open]:animate-in data-[state=closed]:animate-out",

          side === "bottom" && [
            "inset-x-0 bottom-0",
            "max-h-[92svh] min-h-[40svh]",
            "rounded-t-2xl",
            "border-b-0",
            "data-[state=open]:slide-in-from-bottom data-[state=closed]:slide-out-to-bottom",
          ],
          side === "top" && [
            "inset-x-0 top-0",
            "max-h-[85svh]",
            "rounded-b-2xl",
            "border-t-0",
            "data-[state=open]:slide-in-from-top data-[state=closed]:slide-out-to-top",
          ],
          side === "right" && [
            "inset-y-0 right-0",
            "w-full max-w-sm",
            "border-r-0",
            "rounded-l-2xl",
            "data-[state=open]:slide-in-from-right data-[state=closed]:slide-out-to-right",
          ],
          side === "left" && [
            "inset-y-0 left-0",
            "w-full max-w-sm",
            "border-l-0",
            "rounded-r-2xl",
            "data-[state=open]:slide-in-from-left data-[state=closed]:slide-out-to-left",
          ],
          className,
        )}
        {...props}
      >
        {side === "bottom" && (
          <div className="flex shrink-0 justify-center pt-3 pb-1">
            <div className="h-1 w-10 rounded-full bg-border-strong opacity-60" />
          </div>
        )}
        {children}
        {!hideCloseButton && (
          <SheetPrimitive.Close
            className={cn(
              "absolute flex h-9 w-9 items-center justify-center rounded-xl",
              "border-2 border-outset border-border-strong bg-card",
              "text-muted-foreground transition-colors hover:text-foreground",
              side === "bottom" ? "right-4 top-3" : "right-4 top-4",
            )}
          >
            <X size={16} />
            <span className="sr-only">Close</span>
          </SheetPrimitive.Close>
        )}
      </SheetPrimitive.Content>
    </SheetPortal>
  );
}

function SheetHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sheet-header"
      className={cn("flex shrink-0 flex-col gap-1 px-5 pb-3 pt-1", className)}
      {...props}
    />
  );
}

function SheetFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sheet-footer"
      className={cn(
        "mt-auto flex shrink-0 flex-col gap-2 px-5 pb-[calc(1rem+env(safe-area-inset-bottom,0px))] pt-3",
        className,
      )}
      {...props}
    />
  );
}

function SheetTitle({ className, ...props }: React.ComponentProps<typeof SheetPrimitive.Title>) {
  return (
    <SheetPrimitive.Title
      data-slot="sheet-title"
      className={cn("text-base font-semibold text-foreground", className)}
      {...props}
    />
  );
}

function SheetDescription({
  className,
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Description>) {
  return (
    <SheetPrimitive.Description
      data-slot="sheet-description"
      className={cn("text-sm text-muted-foreground", className)}
      {...props}
    />
  );
}

export {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
};
