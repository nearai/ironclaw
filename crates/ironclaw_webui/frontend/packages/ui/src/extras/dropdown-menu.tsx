/**
 * DropdownMenu
 *
 * Action menu attached to a trigger, built on @radix-ui/react-dropdown-menu.
 * Content, items, checkbox/radio items, labels, separators, shortcuts, and
 * submenus all share the overlay surface styling used by the core SelectMenu.
 *
 * Usage
 *   <DropdownMenu>
 *     <DropdownMenuTrigger asChild><Button variant="secondary">Open</Button></DropdownMenuTrigger>
 *     <DropdownMenuContent>
 *       <DropdownMenuItem onSelect={…}>Rename</DropdownMenuItem>
 *       <DropdownMenuSeparator />
 *       <DropdownMenuItem tone="danger">Delete</DropdownMenuItem>
 *     </DropdownMenuContent>
 *   </DropdownMenu>
 */
import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import type { ComponentProps } from "react";
import { cn } from "../primitives/cn";
import { Icon } from "../primitives/icon";
import {
  MENU_ITEM_CLASSES,
  MENU_LABEL_CLASSES,
  MENU_SEPARATOR_CLASSES,
  MENU_SHORTCUT_CLASSES,
  OVERLAY_SURFACE_CLASSES,
} from "./overlay";

export const DropdownMenu = DropdownMenuPrimitive.Root;
export const DropdownMenuTrigger = DropdownMenuPrimitive.Trigger;
export const DropdownMenuGroup = DropdownMenuPrimitive.Group;
export const DropdownMenuSub = DropdownMenuPrimitive.Sub;
export const DropdownMenuRadioGroup = DropdownMenuPrimitive.RadioGroup;

export function DropdownMenuContent({
  className,
  sideOffset = 6,
  ...props
}: ComponentProps<typeof DropdownMenuPrimitive.Content>) {
  return (
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.Content
        sideOffset={sideOffset}
        className={cn(OVERLAY_SURFACE_CLASSES, "min-w-[10rem]", className)}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  );
}

type DropdownMenuItemProps = ComponentProps<
  typeof DropdownMenuPrimitive.Item
> & {
  /** "danger" renders the item in the destructive text color. */
  tone?: "default" | "danger";
};

export function DropdownMenuItem({
  className,
  tone = "default",
  ...props
}: DropdownMenuItemProps) {
  return (
    <DropdownMenuPrimitive.Item
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

export function DropdownMenuCheckboxItem({
  className,
  children,
  ...props
}: ComponentProps<typeof DropdownMenuPrimitive.CheckboxItem>) {
  return (
    <DropdownMenuPrimitive.CheckboxItem
      className={cn(MENU_ITEM_CLASSES, "pl-8", className)}
      {...props}
    >
      <span className="absolute left-2.5 flex h-3.5 w-3.5 items-center justify-center">
        <DropdownMenuPrimitive.ItemIndicator>
          <Icon name="check" className="h-3.5 w-3.5 text-[var(--v2-accent-text)]" />
        </DropdownMenuPrimitive.ItemIndicator>
      </span>
      {children}
    </DropdownMenuPrimitive.CheckboxItem>
  );
}

export function DropdownMenuRadioItem({
  className,
  children,
  ...props
}: ComponentProps<typeof DropdownMenuPrimitive.RadioItem>) {
  return (
    <DropdownMenuPrimitive.RadioItem
      className={cn(MENU_ITEM_CLASSES, "pl-8", className)}
      {...props}
    >
      <span className="absolute left-2.5 flex h-3.5 w-3.5 items-center justify-center">
        <DropdownMenuPrimitive.ItemIndicator>
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--v2-accent-text)]" />
        </DropdownMenuPrimitive.ItemIndicator>
      </span>
      {children}
    </DropdownMenuPrimitive.RadioItem>
  );
}

export function DropdownMenuLabel({
  className,
  ...props
}: ComponentProps<typeof DropdownMenuPrimitive.Label>) {
  return (
    <DropdownMenuPrimitive.Label
      className={cn(MENU_LABEL_CLASSES, className)}
      {...props}
    />
  );
}

export function DropdownMenuSeparator({
  className,
  ...props
}: ComponentProps<typeof DropdownMenuPrimitive.Separator>) {
  return (
    <DropdownMenuPrimitive.Separator
      className={cn(MENU_SEPARATOR_CLASSES, className)}
      {...props}
    />
  );
}

export function DropdownMenuShortcut({
  className,
  ...props
}: ComponentProps<"span">) {
  return <span className={cn(MENU_SHORTCUT_CLASSES, className)} {...props} />;
}

export function DropdownMenuSubTrigger({
  className,
  children,
  ...props
}: ComponentProps<typeof DropdownMenuPrimitive.SubTrigger>) {
  return (
    <DropdownMenuPrimitive.SubTrigger
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
    </DropdownMenuPrimitive.SubTrigger>
  );
}

export function DropdownMenuSubContent({
  className,
  ...props
}: ComponentProps<typeof DropdownMenuPrimitive.SubContent>) {
  return (
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.SubContent
        className={cn(OVERLAY_SURFACE_CLASSES, "min-w-[9rem]", className)}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  );
}
