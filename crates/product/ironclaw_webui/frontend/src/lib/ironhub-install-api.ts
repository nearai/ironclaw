// @ts-nocheck
import { apiFetch } from "./api";

const IRONHUB_INSTALL_PARAMS = [
  "slug",
  "version",
  "uid",
  "aid",
  "ts",
  "nonce",
  "artifact_digest",
  "sig",
];

export function readIronhubInstallRequest(search) {
  const params = new URLSearchParams(search);
  const missing = IRONHUB_INSTALL_PARAMS.filter((name) => !params.get(name));
  if (missing.length > 0) return { request: null, missing };

  const ts = Number(params.get("ts"));
  if (!Number.isSafeInteger(ts) || ts < 0) {
    return { request: null, missing: ["ts"] };
  }

  const request = {
    slug: params.get("slug"),
    version: params.get("version"),
    uid: params.get("uid"),
    aid: params.get("aid"),
    ts,
    nonce: params.get("nonce"),
    artifact_digest: params.get("artifact_digest"),
    sig: params.get("sig"),
  };

  const privateManifestUrl = params.get("private_manifest_url");
  if (privateManifestUrl) request.private_manifest_url = privateManifestUrl;

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

// -> { installed, slug, message }
export function deliverIronhubInstall(request) {
  return apiFetch("/api/webchat/v2/ironhub/install", {
    method: "POST",
    body: JSON.stringify(request),
  });
}
