/**
 * @ironclaw/ui — public surface.
 *
 * Layered bottom-up:
 *   tokens      src/tokens/tokens.css (import via "@ironclaw/ui/tokens.css")
 *   primitives  leaf building blocks with no intra-package dependencies
 *   components  themed interactive controls built on primitives
 *   composites  higher-order assemblies built on components
 *   theme       runtime theming + i18n bridging hooks/providers
 */

/* ── Primitives ────────────────────────────────────────────────────── */
export { cn, type ClassValue } from "./primitives/cn";
export { Icon, ICON_NAMES, type IconName } from "./primitives/icon";
export { Spinner } from "./primitives/spinner";
export { Skeleton } from "./primitives/skeleton";

/* ── Components ────────────────────────────────────────────────────── */
export { Badge, StatusPill, type BadgeTone, type BadgeSize } from "./components/badge";
export { Button } from "./components/button";
export { Callout } from "./components/callout";
export { Card, CardBody, CardFooter, CardHeader, CardLabel, Panel } from "./components/card";
export { IconButton, iconButtonClasses, type IconButtonProps } from "./components/icon-button";
export { FormField, Input, Label, Select, Textarea } from "./components/input";
export { Modal, ModalBody, ModalFooter, ModalHeader } from "./components/modal";
export {
  SelectMenu,
  type SelectMenuAlign,
  type SelectMenuOption,
  type SelectMenuTone,
} from "./components/select-menu";

/* ── Composites ────────────────────────────────────────────────────── */
export { Breadcrumb, type BreadcrumbItem } from "./composites/breadcrumb";
export { ConfirmDialog } from "./composites/confirm-dialog";
export { EmptyPanel } from "./composites/empty-panel";
export { FlowList, type FlowListItem } from "./composites/flow-list";
export { SectionHeader, SubLabel } from "./composites/section-header";
export { StatCard } from "./composites/stat-card";

/* ── Theme ─────────────────────────────────────────────────────────── */
export { useInterfaceTheme, type InterfaceTheme } from "./theme/theme";
export {
  UiTextProvider,
  useUiText,
  DEFAULT_UI_TEXT,
  type UiText,
} from "./theme/ui-text";
