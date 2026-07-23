// @ts-nocheck
export function primaryExtensionAction(ext) {
  const state = extensionLifecycleState(ext);

  if (!ext?.package_ref || state === "active") {
    return null;
  }
  return "configure";
}

export function extensionLifecycleState(ext) {
  const installationState = ext?.installation_state || ext?.installationState;
  if (installationState) {
    return installationState === "active" ? "active" : "setup_needed";
  }
  const onboardingState = ext?.onboarding_state || ext?.onboardingState;
  if (onboardingState) {
    return onboardingState === "active" ? "active" : "setup_needed";
  }
  return ext?.active && ext?.needs_setup !== true ? "active" : "setup_needed";
}

export function extensionIsActive(ext) {
  const state = extensionLifecycleState(ext);
  return state === "active";
}
