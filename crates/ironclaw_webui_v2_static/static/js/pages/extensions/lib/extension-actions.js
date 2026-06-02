export function primaryExtensionAction(ext) {
  const state =
    ext?.onboarding_state ||
    ext?.activation_status ||
    (ext?.active ? "active" : "installed");

  if (!ext?.package_ref || state === "active" || state === "ready") {
    return null;
  }

  if (state === "auth_required" || state === "setup_required") {
    return "configure";
  }

  if (ext.kind === "wasm_channel") {
    return null;
  }

  return "activate";
}

export function setupReadyForActivation({ secrets = [], fields = [] } = {}) {
  if (fields.length > 0 || secrets.length === 0) {
    return false;
  }
  return secrets.every((secret) => secret.provided);
}
