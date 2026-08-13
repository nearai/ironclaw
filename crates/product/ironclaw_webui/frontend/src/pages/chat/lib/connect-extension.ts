/**
 * resolveConnectExtension — map an OOBE suggested task's `app` id (e.g. "gmail",
 * "google_calendar") to a REAL catalog extension so the card's Connect can open
 * the existing extensions setup/OAuth modal (`ConfigureModal`) instead of
 * cloning the connect flow (PROPOSAL §2A, slice 3).
 *
 * It returns the `configurePayload` shape ConfigureModal expects — the raw
 * extension spread with `packageRef`/`displayName` normalized — exactly as
 * `extension-card.tsx` builds it, so both call sites drive the one real path.
 *
 * Resolution is tolerant on purpose: static demo `app` ids won't byte-match a
 * live package ref, and the authoritative id set only exists once the backend
 * suggestion producer (slice 6) emits real extension identities. Until then we
 * normalize both sides (lowercased, alphanumeric-only) and accept a
 * containment match, preferring an already-installed extension over a registry
 * entry. Returns `null` when nothing plausibly matches — the caller then simply
 * does not open a modal rather than opening one with no package ref.
 */

interface PackageRef {
  id?: string;
}

interface CatalogExtension {
  package_ref?: PackageRef | string | null;
  display_name?: string | null;
  [key: string]: unknown;
}

export interface ConnectExtension extends CatalogExtension {
  packageRef: PackageRef | string | null | undefined;
  displayName: string;
}

function packageRefId(ref: PackageRef | string | null | undefined): string {
  if (!ref) return "";
  return typeof ref === "string" ? ref : ref.id || "";
}

function normalize(value: string | null | undefined): string {
  return (value || "").toLowerCase().replace(/[^a-z0-9]/g, "");
}

function matches(app: string, extension: CatalogExtension): boolean {
  const target = normalize(app);
  if (!target) return false;
  const candidates = [
    normalize(packageRefId(extension.package_ref)),
    normalize(extension.display_name),
  ].filter(Boolean);
  return candidates.some(
    (candidate) => candidate === target || candidate.includes(target) || target.includes(candidate),
  );
}

function toConnectExtension(extension: CatalogExtension): ConnectExtension {
  const ref = extension.package_ref ?? null;
  return {
    ...extension,
    packageRef: ref,
    displayName: extension.display_name || packageRefId(ref) || "",
  };
}

export function resolveConnectExtension(
  app: string,
  extensions: CatalogExtension[] = [],
  registry: CatalogExtension[] = [],
): ConnectExtension | null {
  // Installed extensions win: a live package ref + already-provisioned setup
  // state is the most faithful thing to hand ConfigureModal.
  const installed = extensions.find((extension) => matches(app, extension));
  if (installed) return toConnectExtension(installed);
  const known = registry.find((entry) => matches(app, entry));
  if (known) return toConnectExtension(known);
  return null;
}
