/**
 * NavigationMenu
 *
 * Site-level navigation with expanding panels, built on
 * @radix-ui/react-navigation-menu. Triggers look like ghost buttons; open
 * panels use the shared overlay surface. The viewport renders the active
 * panel beneath the list, sized by Radix's CSS variables.
 *
 * Usage
 *   <NavigationMenu>
 *     <NavigationMenuList>
 *       <NavigationMenuItem>
 *         <NavigationMenuTrigger>Product</NavigationMenuTrigger>
 *         <NavigationMenuContent>…links…</NavigationMenuContent>
 *       </NavigationMenuItem>
 *       <NavigationMenuItem>
 *         <NavigationMenuLink href="/docs">Docs</NavigationMenuLink>
 *       </NavigationMenuItem>
 *     </NavigationMenuList>
 *   </NavigationMenu>
 */
import * as NavigationMenuPrimitive from "@radix-ui/react-navigation-menu";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";
import { Icon } from "../primitives/icon";
import { OVERLAY_SURFACE_CLASSES } from "./overlay";

export const NavigationMenuItem = NavigationMenuPrimitive.Item;

const TRIGGER_CLASSES =
  "inline-flex items-center gap-1.5 rounded-[8px] px-3 py-1.5 text-ui font-medium " +
  "text-[var(--v2-text-muted)] outline-none transition-colors " +
  "hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)] " +
  "focus-visible:ring-2 focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)] " +
  "data-[state=open]:bg-[var(--v2-surface-soft)] data-[state=open]:text-[var(--v2-text-strong)]";

export function NavigationMenu({
  className,
  children,
  ...props
}: ComponentProps<typeof NavigationMenuPrimitive.Root>) {
  return (
    <NavigationMenuPrimitive.Root
      className={cn("relative z-10 flex max-w-max items-center", className)}
      {...props}
    >
      {children}
      <NavigationMenuViewport />
    </NavigationMenuPrimitive.Root>
  );
}

export function NavigationMenuList({
  className,
  ...props
}: ComponentProps<typeof NavigationMenuPrimitive.List>) {
  return (
    <NavigationMenuPrimitive.List
      className={cn("flex list-none items-center gap-1", className)}
      {...props}
    />
  );
}

export function NavigationMenuTrigger({
  className,
  children,
  ...props
}: ComponentProps<typeof NavigationMenuPrimitive.Trigger>) {
  return (
    <NavigationMenuPrimitive.Trigger
      className={cn(TRIGGER_CLASSES, "group", className)}
      {...props}
    >
      {children}
      <Icon
        name="chevron"
        className="h-3 w-3 text-[var(--v2-text-faint)] transition-transform group-data-[state=open]:rotate-180"
      />
    </NavigationMenuPrimitive.Trigger>
  );
}

export function NavigationMenuContent({
  className,
  ...props
}: ComponentProps<typeof NavigationMenuPrimitive.Content>) {
  return (
    <NavigationMenuPrimitive.Content
      className={cn("left-0 top-0 w-full p-3 md:absolute md:w-auto", className)}
      {...props}
    />
  );
}

export function NavigationMenuLink({
  className,
  ...props
}: ComponentProps<typeof NavigationMenuPrimitive.Link>) {
  return (
    <NavigationMenuPrimitive.Link
      className={cn(
        TRIGGER_CLASSES,
        "data-[active]:bg-[var(--v2-accent-soft)] data-[active]:text-[var(--v2-accent-text)]",
        className
      )}
      {...props}
    />
  );
}

export function NavigationMenuViewport({
  className,
  ...props
}: ComponentProps<typeof NavigationMenuPrimitive.Viewport>) {
  return (
    <div className="absolute left-0 top-full flex justify-center">
      <NavigationMenuPrimitive.Viewport
        className={cn(
          OVERLAY_SURFACE_CLASSES,
          "relative mt-1.5 w-full p-0 md:w-[var(--radix-navigation-menu-viewport-width)]",
          "h-[var(--radix-navigation-menu-viewport-height)] overflow-hidden transition-[width,height]",
          className
        )}
        {...props}
      />
    </div>
  );
}

export function NavigationMenuIndicator({
  className,
  ...props
}: ComponentProps<typeof NavigationMenuPrimitive.Indicator>) {
  return (
    <NavigationMenuPrimitive.Indicator
      className={cn("top-full z-[1] flex h-1.5 items-end justify-center", className)}
      {...props}
    >
      <span className="relative top-[60%] h-2 w-2 rotate-45 rounded-tl-sm bg-[var(--v2-panel-border)]" />
    </NavigationMenuPrimitive.Indicator>
  );
}
