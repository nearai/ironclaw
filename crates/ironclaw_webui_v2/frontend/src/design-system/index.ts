/**
 * IronClaw design-system barrel.
 *
 * Components follow shadcn patterns (Radix primitives + CVA/cn + tokens)
 * while keeping IronClaw `--v2-*` look/feel.
 */

export { Avatar, AvatarFallback, AvatarImage } from "./avatar";
export { Badge, StatusPill } from "./badge";
export { Button } from "./button";
export { Card, CardBody, CardFooter, CardHeader, CardLabel } from "./card";
export { Checkbox } from "./checkbox";
export { ConfirmDialog } from "./confirm-dialog";
export {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPortal,
  DropdownMenuRadioGroup,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuTrigger,
} from "./dropdown-menu";
export { Icon } from "./icons";
export { FormField, Input, Label, Select, Textarea } from "./input";
export { Modal, ModalBody, ModalFooter, ModalHeader } from "./modal";
export {
  MOTION_DURATION,
  MOTION_EASE_IN_OUT,
  MOTION_EASE_OUT,
  useReducedMotion,
} from "./motion";
export { Popover, PopoverAnchor, PopoverClose, PopoverContent, PopoverTrigger } from "./popover";
export {
  EmptyPanel,
  FlowList,
  Panel,
  SectionHeader,
  StatCard,
  SubLabel,
  cx,
} from "./primitives";
export { RadioGroup, RadioGroupItem } from "./radio-group";
export { ScrollArea, ScrollBar } from "./scroll-area";
export { SelectMenu } from "./select-menu";
export type { SelectMenuAlign, SelectMenuOption, SelectMenuTone } from "./select-menu";
export { Separator } from "./separator";
export { Skeleton } from "./skeleton";
export { Slider } from "./slider";
export { Spinner } from "./spinner";
export { Switch } from "./switch";
export { Tabs } from "./tabs";
export type { TabItem } from "./tabs";
export { useInterfaceTheme } from "./theme";
export type { InterfaceTheme } from "./theme";
export { Tooltip, TooltipProvider } from "./tooltip";
