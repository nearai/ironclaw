// @ts-nocheck
// The real Radix-backed popover subtree behind LazyPopover. Loaded on
// first activation only — see lazy-popover.tsx for the rationale. This
// file imports named exports from the design-system barrel STATICALLY
// (so tree-shaking still works); only the file itself is dynamic.
import { Popover, PopoverContent, PopoverTrigger } from "@ironclaw/design-system";

export function LazyPopoverImpl({
  defaultOpen = false,
  trigger,
  triggerProps = {},
  side,
  align,
  sideOffset,
  contentClassName = "",
  onOpenChange,
  children,
}) {
  return (
    <Popover defaultOpen={defaultOpen} onOpenChange={onOpenChange}>
      <PopoverTrigger type="button" {...triggerProps}>
        {trigger}
      </PopoverTrigger>
      <PopoverContent
        side={side}
        align={align}
        sideOffset={sideOffset}
        className={contentClassName}
      >
        {children}
      </PopoverContent>
    </Popover>
  );
}
