export function validateDeploymentOrigin(origin: string, production: boolean): string {
  const normalized = origin.trim().replace(/\/+$/, "");
  let url: URL;
  try {
    url = new URL(normalized);
  } catch {
    throw new Error("Enter a valid deployment URL");
  }

  const loopback =
    url.hostname === "localhost" || url.hostname === "127.0.0.1" || url.hostname === "[::1]";
  if (url.protocol !== "https:" && !(url.protocol === "http:" && loopback && !production)) {
    throw new Error(
      production
        ? "Deployment URLs must use HTTPS"
        : "Use HTTPS, or HTTP with localhost, 127.0.0.1, or [::1]"
    );
  }
  if (url.username || url.password || url.pathname !== "/" || url.search || url.hash) {
    throw new Error("Enter only the deployment origin, without credentials, a path, or parameters");
  }
  return normalized;
}
