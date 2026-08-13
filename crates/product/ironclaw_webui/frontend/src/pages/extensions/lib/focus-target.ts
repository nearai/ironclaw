export type FocusTargetResolver = () => HTMLElement | null;

export type FocusTarget = HTMLElement | FocusTargetResolver;

export type ConfigureFocusHandler<T = unknown> = (
  extension: T,
  returnFocusTo?: FocusTarget | null,
) => void;

export type ExtensionInstallPayload = {
  packageRef: string | { id?: string };
  displayName?: string;
};

export type InstallFocusHandler = (
  payload: ExtensionInstallPayload,
  installTrigger: HTMLElement,
) => void;

export function resolveFocusTarget(
  target: FocusTarget | null | undefined,
): HTMLElement | null {
  return typeof target === "function" ? target() : target ?? null;
}
