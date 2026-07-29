/**
 * Menubar
 *
 * Horizontal application menu bar built on @radix-ui/react-menubar
 * (File / Edit / View…). Menus share the overlay surface + item styling
 * with DropdownMenu; the bar itself is a soft surface rail.
 *
 * Usage
 *   <Menubar>
 *     <MenubarMenu>
 *       <MenubarTrigger>File</MenubarTrigger>
 *       <MenubarContent>
 *         <MenubarItem>New…<MenubarShortcut>⌘N</MenubarShortcut></MenubarItem>
 *       </MenubarContent>
 *     </MenubarMenu>
 *   </Menubar>
 */
import * as MenubarPrimitive from "@radix-ui/react-menubar";
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

export const MenubarMenu = MenubarPrimitive.Menu;
export const MenubarGroup = MenubarPrimitive.Group;
export const MenubarSub = MenubarPrimitive.Sub;
export const MenubarRadioGroup = MenubarPrimitive.RadioGroup;

export function Menubar({
  className,
  ...props
}: ComponentProps<typeof MenubarPrimitive.Root>) {
  return (
    <MenubarPrimitive.Root
      className={cn(
        "flex items-center gap-1 rounded-[10px] p-1",
        "border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
        className
      )}
      {...props}
    />
  );
}

export function MenubarTrigger({
  className,
  ...props
}: ComponentProps<typeof MenubarPrimitive.Trigger>) {
  return (
    <MenubarPrimitive.Trigger
      className={cn(
        "select-none rounded-[7px] px-2.5 py-1 text-ui font-medium",
        "text-[var(--v2-text-muted)] outline-none transition-colors",
        "hover:text-[var(--v2-text-strong)]",
        "active:bg-[var(--v2-surface-muted)] active:text-[var(--v2-text-strong)]",
        "focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]",
        "data-[highlighted]:bg-[var(--v2-surface-muted)] data-[highlighted]:text-[var(--v2-text-strong)]",
        "data-[state=open]:bg-[var(--v2-surface-muted)] data-[state=open]:text-[var(--v2-text-strong)]",
        "data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
        className
      )}
      {...props}
    />
  );
}

export function MenubarContent({
  className,
  align = "start",
  sideOffset = 6,
  ...props
}: ComponentProps<typeof MenubarPrimitive.Content>) {
  return (
    <MenubarPrimitive.Portal>
      <MenubarPrimitive.Content
        align={align}
        sideOffset={sideOffset}
        className={cn(OVERLAY_SURFACE_CLASSES, "min-w-[11rem]", className)}
        {...props}
      />
    </MenubarPrimitive.Portal>
  );
}

export function MenubarItem({
  className,
  ...props
}: ComponentProps<typeof MenubarPrimitive.Item>) {
  return (
    <MenubarPrimitive.Item
      className={cn(MENU_ITEM_CLASSES, className)}
      {...props}
    />
  );
}

export function MenubarCheckboxItem({
  className,
  children,
  ...props
}: ComponentProps<typeof MenubarPrimitive.CheckboxItem>) {
  return (
    <MenubarPrimitive.CheckboxItem
      className={cn(MENU_ITEM_CLASSES, "pl-8", className)}
      {...props}
    >
      <span className="absolute left-2.5 flex h-3.5 w-3.5 items-center justify-center">
        <MenubarPrimitive.ItemIndicator>
          <Icon name="check" className="h-3.5 w-3.5 text-[var(--v2-accent-text)]" />
        </MenubarPrimitive.ItemIndicator>
      </span>
      {children}
    </MenubarPrimitive.CheckboxItem>
  );
}

export function MenubarRadioItem({
  className,
  children,
  ...props
}: ComponentProps<typeof MenubarPrimitive.RadioItem>) {
  return (
    <MenubarPrimitive.RadioItem
      className={cn(MENU_ITEM_CLASSES, "pl-8", className)}
      {...props}
    >
      <span className="absolute left-2.5 flex h-3.5 w-3.5 items-center justify-center">
        <MenubarPrimitive.ItemIndicator>
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--v2-accent-text)]" />
        </MenubarPrimitive.ItemIndicator>
      </span>
      {children}
    </MenubarPrimitive.RadioItem>
  );
}

export function MenubarLabel({
  className,
  ...props
}: ComponentProps<typeof MenubarPrimitive.Label>) {
  return (
    <MenubarPrimitive.Label
      className={cn(MENU_LABEL_CLASSES, className)}
      {...props}
    />
  );
}

export function MenubarSeparator({
  className,
  ...props
}: ComponentProps<typeof MenubarPrimitive.Separator>) {
  return (
    <MenubarPrimitive.Separator
      className={cn(MENU_SEPARATOR_CLASSES, className)}
      {...props}
    />
  );
}

export function MenubarShortcut({
  className,
  ...props
}: ComponentProps<"span">) {
  return <span className={cn(MENU_SHORTCUT_CLASSES, className)} {...props} />;
}

export function MenubarSubTrigger({
  className,
  children,
  ...props
}: ComponentProps<typeof MenubarPrimitive.SubTrigger>) {
  return (
    <MenubarPrimitive.SubTrigger
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
    </MenubarPrimitive.SubTrigger>
  );
}

export function MenubarSubContent({
  className,
  ...props
}: ComponentProps<typeof MenubarPrimitive.SubContent>) {
  return (
    <MenubarPrimitive.Portal>
      <MenubarPrimitive.SubContent
        className={cn(OVERLAY_SURFACE_CLASSES, "min-w-[9rem]", className)}
        {...props}
      />
    </MenubarPrimitive.Portal>
  );
}
