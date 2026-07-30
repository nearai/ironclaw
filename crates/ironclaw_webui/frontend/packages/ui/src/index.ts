/**
 * @ironclaw/ui — public surface.
 *
 * Layered bottom-up:
 *   tokens      src/tokens/tokens.css (import via "@ironclaw/ui/tokens.css")
 *   primitives  leaf building blocks with no intra-package dependencies
 *   components  themed interactive controls built on primitives
 *   composites  higher-order assemblies built on components
 *   theme       runtime theming + i18n bridging hooks/providers
 *
 * The extras kit ("@ironclaw/ui/extras") sits beside components; when a
 * product surface adopts an extra, promote the file into components/.
 */

/* ── Primitives ────────────────────────────────────────────────────── */
export { cn, type ClassValue } from "./primitives/cn";
export { Icon, ICON_NAMES, type IconName, type IconProps } from "./primitives/icon";
export { Skeleton } from "./primitives/skeleton";
export { Spinner } from "./primitives/spinner";
export { StatusDot, type StatusDotTone } from "./primitives/status-dot";

/* ── Components ────────────────────────────────────────────────────── */
export { Badge, type BadgeTone, type BadgeSize } from "./components/badge";
export { Breadcrumb, type BreadcrumbItem } from "./components/breadcrumb";
export { Button } from "./components/button";
export { Callout, type CalloutTone } from "./components/callout";
export { Card, CardBody, CardFooter, CardHeader, CardLabel } from "./components/card";
export { FlowList, type FlowListItem } from "./components/flow-list";
export { IconButton, iconButtonClasses, type IconButtonProps } from "./components/icon-button";
export { Input, Label, Select, Textarea, type InputSize } from "./components/input";
export { Modal, ModalBody, ModalFooter, ModalHeader } from "./components/modal";
export { SearchInput } from "./components/search-input";
export {
  SelectMenu,
  type SelectMenuAlign,
  type SelectMenuOption,
  type SelectMenuTone,
} from "./components/select-menu";
export { Switch } from "./components/switch";

/* ── Composites ────────────────────────────────────────────────────── */
export { CodePanel } from "./composites/code-panel";
export { ConfirmDialog } from "./composites/confirm-dialog";
export { DetailList, DetailRow } from "./composites/detail-list";
export { EmptyPanel } from "./composites/empty-panel";
export { FormField } from "./composites/form-field";
export { SectionHeader, SubLabel } from "./composites/section-header";
export {
  SegmentedControl,
  type SegmentedControlOption,
} from "./composites/segmented-control";
export { SkeletonList } from "./composites/skeleton-list";
export { StatCard } from "./composites/stat-card";
export { StatStrip, StatTile } from "./composites/stat-strip";
export { Toolbar, ToolbarGroup } from "./composites/toolbar";
export {
  VerticalTabs,
  VerticalTabsMobile,
  type VerticalTabItem,
} from "./composites/vertical-tabs";

/* ── Theme ─────────────────────────────────────────────────────────── */
export { useInterfaceTheme, type InterfaceTheme } from "./theme/theme";
export {
  UiTextProvider,
  useUiText,
  DEFAULT_UI_TEXT,
  type UiText,
} from "./theme/ui-text";
