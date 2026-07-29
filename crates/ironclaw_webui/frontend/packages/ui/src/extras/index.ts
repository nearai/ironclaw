/**
 * @ironclaw/ui/extras — pre-built component kit.
 *
 * shadcn/ui gap coverage for the design system: token-faithful components
 * that no product surface uses yet. Deliberately NOT re-exported from the
 * main barrel — import from "@ironclaw/ui/extras" so unused extras never
 * enter the app bundle, and promote a component into src/components +
 * src/index.ts when a surface adopts it. See README.md in this directory.
 */

export {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "./accordion";
export { AspectRatio } from "./aspect-ratio";
export { Avatar, AvatarFallback, AvatarImage, type AvatarSize } from "./avatar";
export { Checkbox } from "./checkbox";
export {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "./collapsible";
export { Combobox, type ComboboxOption } from "./combobox";
export {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from "./command";
export {
  ContextMenu,
  ContextMenuCheckboxItem,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuRadioGroup,
  ContextMenuRadioItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "./context-menu";
export { Calendar, DatePicker } from "./date-picker";
export {
  Drawer,
  DrawerBody,
  DrawerFooter,
  Sheet,
  SheetBody,
  SheetFooter,
  type DrawerSide,
  type DrawerSize,
} from "./drawer";
export {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "./dropdown-menu";
export { HoverCard, HoverCardContent, HoverCardTrigger } from "./hover-card";
export { InputOTP } from "./input-otp";
export {
  Menubar,
  MenubarCheckboxItem,
  MenubarContent,
  MenubarGroup,
  MenubarItem,
  MenubarLabel,
  MenubarMenu,
  MenubarRadioGroup,
  MenubarRadioItem,
  MenubarSeparator,
  MenubarShortcut,
  MenubarSub,
  MenubarSubContent,
  MenubarSubTrigger,
  MenubarTrigger,
} from "./menubar";
export {
  NavigationMenu,
  NavigationMenuContent,
  NavigationMenuIndicator,
  NavigationMenuItem,
  NavigationMenuLink,
  NavigationMenuList,
  NavigationMenuTrigger,
  NavigationMenuViewport,
} from "./navigation-menu";
export {
  Pagination,
  PaginationButton,
  PaginationContent,
  PaginationEllipsis,
  PaginationItem,
  PaginationNext,
  PaginationPrevious,
  SimplePagination,
} from "./pagination";
export {
  Popover,
  PopoverAnchor,
  PopoverClose,
  PopoverContent,
  PopoverTrigger,
} from "./popover";
export { Progress } from "./progress";
export { RadioGroup, RadioGroupItem } from "./radio-group";
export {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "./resizable";
export { ScrollArea, ScrollBar } from "./scroll-area";
export { Separator } from "./separator";
export { Slider } from "./slider";
export { Switch } from "./switch";
export {
  DataTable,
  Table,
  TableBody,
  TableCaption,
  TableCell,
  TableFooter,
  TableHead,
  TableHeader,
  TableRow,
  type DataTableColumn,
} from "./table";
export { Tabs, TabsContent, TabsList, TabsTrigger } from "./tabs";
export {
  Toast,
  ToastAction,
  ToastClose,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
  Toaster,
  dismissToast,
  toast,
  type ToastOptions,
  type ToastTone,
} from "./toast";
export { Toggle, type ToggleSize } from "./toggle";
export { ToggleGroup, ToggleGroupItem } from "./toggle-group";
export {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "./tooltip";
