/**
 * ContextMenu
 *
 * Right-click menu built on @radix-ui/react-context-menu. Mirrors the
 * DropdownMenu API and shares the same overlay surface styling; only the
 * invocation gesture differs (contextmenu event on the trigger area).
 *
 * Usage
 *   <ContextMenu>
 *     <ContextMenuTrigger className="…">Right-click me</ContextMenuTrigger>
 *     <ContextMenuContent>
 *       <ContextMenuItem>Copy</ContextMenuItem>
 *     </ContextMenuContent>
 *   </ContextMenu>
 */
import * as ContextMenuPrimitive from "@radix-ui/react-context-menu";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";
import { Icon } from "../icons/icon";
import {
  MENU_ITEM_CLASSES,
  MENU_LABEL_CLASSES,
  MENU_SEPARATOR_CLASSES,
  MENU_SHORTCUT_CLASSES,
  OVERLAY_SURFACE_CLASSES,
} from "../primitives/overlay";

export const ContextMenu = ContextMenuPrimitive.Root;
export const ContextMenuTrigger = ContextMenuPrimitive.Trigger;
export const ContextMenuGroup = ContextMenuPrimitive.Group;
export const ContextMenuSub = ContextMenuPrimitive.Sub;
export const ContextMenuRadioGroup = ContextMenuPrimitive.RadioGroup;

export function ContextMenuContent({
  className,
  ...props
}: ComponentProps<typeof ContextMenuPrimitive.Content>) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.Content
        className={cn(OVERLAY_SURFACE_CLASSES, "min-w-[10rem]", className)}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  );
}

type ContextMenuItemProps = ComponentProps<
  typeof ContextMenuPrimitive.Item
> & {
  /** "danger" renders the item in the destructive text color. */
  tone?: "default" | "danger";
};

export function ContextMenuItem({
  className,
  tone = "default",
  ...props
}: ContextMenuItemProps) {
  return (
    <ContextMenuPrimitive.Item
      className={cn(
        MENU_ITEM_CLASSES,
        tone === "danger" &&
          "text-[var(--v2-danger-text)] data-[highlighted]:bg-[var(--v2-danger-soft)] data-[highlighted]:text-[var(--v2-danger-text)]",
        className
      )}
      {...props}
    />
  );
}

export function ContextMenuCheckboxItem({
  className,
  children,
  ...props
}: ComponentProps<typeof ContextMenuPrimitive.CheckboxItem>) {
  return (
    <ContextMenuPrimitive.CheckboxItem
      className={cn(MENU_ITEM_CLASSES, "pl-8", className)}
      {...props}
    >
      <span className="absolute left-2.5 flex h-3.5 w-3.5 items-center justify-center">
        <ContextMenuPrimitive.ItemIndicator>
          <Icon name="check" className="h-3.5 w-3.5 text-[var(--v2-accent-text)]" />
        </ContextMenuPrimitive.ItemIndicator>
      </span>
      {children}
    </ContextMenuPrimitive.CheckboxItem>
  );
}

export function ContextMenuRadioItem({
  className,
  children,
  ...props
}: ComponentProps<typeof ContextMenuPrimitive.RadioItem>) {
  return (
    <ContextMenuPrimitive.RadioItem
      className={cn(MENU_ITEM_CLASSES, "pl-8", className)}
      {...props}
    >
      <span className="absolute left-2.5 flex h-3.5 w-3.5 items-center justify-center">
        <ContextMenuPrimitive.ItemIndicator>
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--v2-accent-text)]" />
        </ContextMenuPrimitive.ItemIndicator>
      </span>
      {children}
    </ContextMenuPrimitive.RadioItem>
  );
}

export function ContextMenuLabel({
  className,
  ...props
}: ComponentProps<typeof ContextMenuPrimitive.Label>) {
  return (
    <ContextMenuPrimitive.Label
      className={cn(MENU_LABEL_CLASSES, className)}
      {...props}
    />
  );
}

export function ContextMenuSeparator({
  className,
  ...props
}: ComponentProps<typeof ContextMenuPrimitive.Separator>) {
  return (
    <ContextMenuPrimitive.Separator
      className={cn(MENU_SEPARATOR_CLASSES, className)}
      {...props}
    />
  );
}

export function ContextMenuShortcut({
  className,
  ...props
}: ComponentProps<"span">) {
  return <span className={cn(MENU_SHORTCUT_CLASSES, className)} {...props} />;
}

export function ContextMenuSubTrigger({
  className,
  children,
  ...props
}: ComponentProps<typeof ContextMenuPrimitive.SubTrigger>) {
  return (
    <ContextMenuPrimitive.SubTrigger
      className={cn(
        MENU_ITEM_CLASSES,
        "data-[state=open]:bg-[var(--v2-surface-muted)]",
        className
      )}
      {...props}
    >
      {children}
      <Icon
        name="chevron"
        className="ml-auto h-3.5 w-3.5 -rotate-90 text-[var(--v2-text-faint)]"
      />
    </ContextMenuPrimitive.SubTrigger>
  );
}

export function ContextMenuSubContent({
  className,
  ...props
}: ComponentProps<typeof ContextMenuPrimitive.SubContent>) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.SubContent
        className={cn(OVERLAY_SURFACE_CLASSES, "min-w-[9rem]", className)}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  );
}
