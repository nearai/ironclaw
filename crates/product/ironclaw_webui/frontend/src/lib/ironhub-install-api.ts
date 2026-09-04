// @ts-nocheck
import { apiFetch } from "./api";

const IRONHUB_INSTALL_PARAMS = [
  { name: "slug", max: 128, pattern: /^[a-z0-9_-]+$/ },
  { name: "version", max: 64, pattern: /^[A-Za-z0-9.+_-]+$/ },
  { name: "uid", max: 128, pattern: /^[A-Za-z0-9._-]+$/ },
  { name: "aid", max: 128, pattern: /^[A-Za-z0-9._-]+$/ },
  { name: "nonce", max: 128, pattern: /^[A-Za-z0-9._-]+$/ },
  { name: "artifact_digest", max: 160, pattern: /^[A-Za-z0-9:_-]+$/ },
  { name: "sig", max: 512, pattern: /^[A-Za-z0-9+/=_-]+$/ },
];

const MAX_PRIVATE_MANIFEST_URL = 2048;

function isHttpUrl(value) {
  try {
    const { protocol } = new URL(value);
    return protocol === "https:" || protocol === "http:";
  } catch {
    return false;
  }
}

export function readIronhubInstallRequest(search) {
  const params = new URLSearchParams(search);
  const request = {};
  const missing = [];

  for (const { name, max, pattern } of IRONHUB_INSTALL_PARAMS) {
    const value = params.get(name);
    if (!value || value.length > max || !pattern.test(value)) {
      missing.push(name);
      continue;
    }
    request[name] = value;
  }

  const rawTs = params.get("ts");
  if (!rawTs || !/^[0-9]{1,15}$/.test(rawTs)) {
    missing.push("ts");
  } else {
    request.ts = Number(rawTs);
  }

  const privateManifestUrl = params.get("private_manifest_url");
  if (privateManifestUrl) {
    if (privateManifestUrl.length > MAX_PRIVATE_MANIFEST_URL || !isHttpUrl(privateManifestUrl)) {
      missing.push("private_manifest_url");
    } else {
      request.private_manifest_url = privateManifestUrl;
    }
  }

  if (missing.length > 0) return { request: null, missing };
  return { request, missing: [] };
}

export function installErrorKey(error) {
  if (error?.status !== 403) return "ironhub.install.failed";

  switch (error?.payload?.kind) {
    case "expired":
      return "ironhub.install.expired";
    case "duplicate":
      return "ironhub.install.alreadyUsed";
    default:
      return "ironhub.install.rejected";
  }
}

export function deliverIronhubInstall(request) {
  return apiFetch("/api/webchat/v2/ironhub/install", {
    method: "POST",
    body: JSON.stringify(request),
  });
}
