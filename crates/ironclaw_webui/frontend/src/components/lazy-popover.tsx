// @ts-nocheck
// Interaction-gated facade for the design-system Popover.
//
// Always-mounted chrome (sidebar footer, TEE shield) sits in the initial
// import graph of every route, and Radix Popover drags the
// floating-ui + react-remove-scroll stack (~18KB gzip) with it — for a
// surface that only appears on click. Until the first activation this
// renders just the trigger markup; the first click lazy-loads the real
// Popover and opens it. After that the Radix popover owns the trigger,
// so subsequent opens/closes behave exactly like a direct usage.
import React, { useCallback, useState } from "react";

const LazyPopoverImpl = React.lazy(() =>
  import("./lazy-popover-impl").then((mod) => ({
    default: mod.LazyPopoverImpl,
  }))
);

export function LazyPopover({
  trigger = null,
  triggerProps = {},
  side = undefined,
  align = undefined,
  sideOffset = undefined,
  contentClassName = "",
  onOpenChange = undefined,
  children = null,
}) {
  const [activated, setActivated] = useState(false);

  const activate = useCallback(() => {
    setActivated(true);
    onOpenChange?.(true);
  }, [onOpenChange]);

  const triggerMarkup = (extra) => (
    <button
      type="button"
      aria-haspopup="dialog"
      {...triggerProps}
      {...extra}
    >
      {trigger}
    </button>
  );

  if (!activated) {
    return triggerMarkup({ "aria-expanded": "false", onClick: activate });
  }

  return (
    <React.Suspense fallback={triggerMarkup({ "aria-expanded": "true" })}>
      <LazyPopoverImpl
        defaultOpen
        trigger={trigger}
        triggerProps={triggerProps}
        side={side}
        align={align}
        sideOffset={sideOffset}
        contentClassName={contentClassName}
        onOpenChange={onOpenChange}
      >
        {children}
      </LazyPopoverImpl>
    </React.Suspense>
  );
}
