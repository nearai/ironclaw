import type { ComponentProps } from "react";

import { Badge } from "./badge";
import { Card } from "./card";
import { Input, Label, Select, Textarea } from "./input";
import { Modal } from "./modal";
import { SelectMenu } from "./select-menu";
import { Spinner } from "./spinner";
import { Switch } from "./switch";

type InputProps = ComponentProps<typeof Input>;
type TextareaProps = ComponentProps<typeof Textarea>;
type SelectProps = ComponentProps<typeof Select>;
type LabelProps = ComponentProps<typeof Label>;
type ModalProps = ComponentProps<typeof Modal>;
type BadgeProps = ComponentProps<typeof Badge>;
type CardProps = ComponentProps<typeof Card>;
type SelectMenuProps = ComponentProps<typeof SelectMenu>;
type SwitchProps = ComponentProps<typeof Switch>;
type SpinnerProps = ComponentProps<typeof Spinner>;

const validInputProps: InputProps = {
  "aria-label": "Name",
  autoComplete: "name",
  name: "name",
  onChange: (event) => event.currentTarget.value,
  size: "sm",
};
const invalidInputProps: InputProps = {
  // @ts-expect-error unsupported design-system size
  size: "oversized",
};

const validTextareaProps: TextareaProps = {
  maxLength: 500,
  onChange: (event) => event.currentTarget.value,
  rows: 6,
};
const invalidTextareaProps: TextareaProps = {
  // @ts-expect-error textarea rows must use the native numeric prop
  rows: "six",
};

const validSelectProps: SelectProps = {
  children: <option value="all">All</option>,
  onChange: (event) => event.currentTarget.value,
  size: "lg",
};
const invalidSelectProps: SelectProps = {
  // @ts-expect-error unsupported design-system size
  size: "compact",
};

const validLabelProps: LabelProps = {
  children: "Name",
  htmlFor: "name",
  onClick: (event) => event.currentTarget.htmlFor,
};

const validModalProps: ModalProps = {
  children: <div>Body</div>,
  onClose: () => undefined,
  open: true,
  size: "full",
  title: "Settings",
};
const invalidModalProps: ModalProps = {
  ...validModalProps,
  // @ts-expect-error unsupported modal size
  size: "screen",
};

const validBadgeProps: BadgeProps = {
  "data-testid": "status",
  label: "Ready",
  onClick: (event) => event.currentTarget.dataset.testid,
  size: "sm",
  tone: "positive",
};
const invalidBadgeProps: BadgeProps = {
  label: "Ready",
  // @ts-expect-error unsupported badge tone
  tone: "brand",
};

const validCardProps: CardProps = {
  children: "Card body",
  onClick: (event) => event.currentTarget.dataset.state,
  padding: "md",
  variant: "bordered",
};
const validAnchorCardProps: ComponentProps<typeof Card<"a">> = {
  as: "a",
  children: "Documentation",
  href: "/docs",
};
const invalidCardProps: CardProps = {
  // @ts-expect-error unsupported card variant
  variant: "raised",
};

const validSelectMenuProps: SelectMenuProps = {
  "aria-label": "Permission",
  "data-testid": "permission-menu",
  onChange: (value) => value.toUpperCase(),
  options: [{ label: "Allow", tone: "positive", value: "allow" }],
  size: "sm",
  value: "allow",
};
const invalidSelectMenuProps: SelectMenuProps = {
  ...validSelectMenuProps,
  // @ts-expect-error unsupported select-menu alignment
  align: "center",
};

const validSwitchProps: SwitchProps = {
  "aria-label": "Enabled",
  checked: true,
  name: "enabled",
  onChange: () => undefined,
};
const invalidSwitchProps: SwitchProps = {
  ...validSwitchProps,
  // @ts-expect-error unsupported switch size
  size: "lg",
};

const validSpinnerProps: SpinnerProps = { className: "h-5 w-5" };

void [
  invalidBadgeProps,
  invalidCardProps,
  invalidInputProps,
  invalidModalProps,
  invalidSelectMenuProps,
  invalidSelectProps,
  invalidSwitchProps,
  invalidTextareaProps,
  validBadgeProps,
  validAnchorCardProps,
  validCardProps,
  validInputProps,
  validLabelProps,
  validModalProps,
  validSelectMenuProps,
  validSelectProps,
  validSpinnerProps,
  validSwitchProps,
  validTextareaProps,
];
