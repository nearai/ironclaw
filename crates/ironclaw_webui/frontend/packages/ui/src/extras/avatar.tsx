/**
 * Avatar
 *
 * User/agent identity chip built on @radix-ui/react-avatar: image with a
 * token-tinted initials fallback that shows while the image loads or when it
 * fails. Sizes follow the control scale used by IconButton.
 *
 * Usage
 *   <Avatar size="md">
 *     <AvatarImage src="…" alt="Ada Lovelace" />
 *     <AvatarFallback>AL</AvatarFallback>
 *   </Avatar>
 */
import * as AvatarPrimitive from "@radix-ui/react-avatar";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";

const AVATAR_SIZES = {
  sm: "h-7 w-7 text-ui-sm",
  md: "h-9 w-9 text-ui-sm",
  lg: "h-12 w-12 text-ui",
};

export type AvatarSize = keyof typeof AVATAR_SIZES;

type AvatarProps = ComponentProps<typeof AvatarPrimitive.Root> & {
  size?: AvatarSize;
};

export function Avatar({ className, size = "md", ...props }: AvatarProps) {
  return (
    <AvatarPrimitive.Root
      className={cn(
        "relative flex shrink-0 overflow-hidden rounded-full",
        "border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
        AVATAR_SIZES[size] ?? AVATAR_SIZES.md,
        className
      )}
      {...props}
    />
  );
}

export function AvatarImage({
  className,
  ...props
}: ComponentProps<typeof AvatarPrimitive.Image>) {
  return (
    <AvatarPrimitive.Image
      className={cn("aspect-square h-full w-full object-cover", className)}
      {...props}
    />
  );
}

export function AvatarFallback({
  className,
  ...props
}: ComponentProps<typeof AvatarPrimitive.Fallback>) {
  return (
    <AvatarPrimitive.Fallback
      className={cn(
        "flex h-full w-full items-center justify-center rounded-full",
        "bg-[var(--v2-accent-soft)] font-medium text-[var(--v2-accent-text)]",
        className
      )}
      {...props}
    />
  );
}
