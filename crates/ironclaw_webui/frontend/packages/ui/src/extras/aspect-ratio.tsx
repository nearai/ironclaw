/**
 * AspectRatio
 *
 * Constrains content to a fixed width/height ratio (16/9, 1, 4/3, …).
 * Thin pass-through over @radix-ui/react-aspect-ratio — purely structural,
 * no visual styling of its own.
 *
 * Usage
 *   <AspectRatio ratio={16 / 9}>
 *     <img src="…" className="h-full w-full rounded-[10px] object-cover" />
 *   </AspectRatio>
 */
import * as AspectRatioPrimitive from "@radix-ui/react-aspect-ratio";
import type { ComponentProps } from "react";

export function AspectRatio(
  props: ComponentProps<typeof AspectRatioPrimitive.Root>
) {
  return <AspectRatioPrimitive.Root {...props} />;
}
