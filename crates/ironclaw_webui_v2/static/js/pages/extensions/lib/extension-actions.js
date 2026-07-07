import { isChannelExtensionKind } from "./extensions-schema.js";

export function primaryExtensionAction(ext) {
  const state = extensionLifecycleState(ext);

  if (!ext?.package_ref || state === "active" || state === "ready") {
    return null;
  }

  if (state === "auth_required" || state === "setup_required") {
    return "configure";
  }

  // Channel-surface extensions are configured or paired through their channel
  // panels. A generic Activate button bypasses that guidance and can hit the
  // wrong lifecycle endpoint for Slack-style inbound proof-code flows.
  if (isChannelExtensionKind(ext?.kind)) {
    return null;
  }

  return "activate";
}

export function extensionLifecycleState(ext) {
  return (
    ext?.onboarding_state ||
    ext?.onboardingState ||
    ext?.activation_status ||
    ext?.activationStatus ||
    (ext?.active ? "active" : "installed")
  );
}

export function extensionIsActive(ext) {
  const state = extensionLifecycleState(ext);
  return state === "active" || state === "ready";
}

export function setupReadyForActivation({ extension, secrets = [], fields = [] } = {}) {
  if (extensionIsActive(extension)) {
    return false;
  }
  if (fields.length > 0 || secrets.length === 0) {
    return false;
  }
  return secrets.every((secret) => secret.provided);
}
