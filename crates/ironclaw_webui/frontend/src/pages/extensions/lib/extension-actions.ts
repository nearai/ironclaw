// @ts-nocheck
export function primaryExtensionAction(ext) {
  const state = extensionLifecycleState(ext);

  return ext?.package_ref && state === "setup_needed" ? "configure" : null;
}

export function extensionLifecycleState(ext) {
  const installationState = ext?.installation_state;
  return installationState === "active" || installationState === "setup_needed"
    ? installationState
    : "uninstalled";
}

export function extensionIsActive(ext) {
  const state = extensionLifecycleState(ext);
  return state === "active";
}
