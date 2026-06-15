import { verifySriForUrl } from "everything-dev/integrity";
import type { RuntimeConfig } from "everything-dev/types";
import { logger } from "../utils/logger";

const DEFAULT_INTERVAL_MS = 5 * 60 * 1000;

interface MonitoredRemote {
  key: string;
  url: string;
  integrity: string;
}

function extractMonitoredRemotes(config: RuntimeConfig): MonitoredRemote[] {
  const remotes: MonitoredRemote[] = [];

  if (config.ui?.integrity && config.ui.url) {
    remotes.push({ key: "ui", url: config.ui.url, integrity: config.ui.integrity });
  }

  if (config.api?.integrity && config.api.url) {
    remotes.push({ key: "api", url: config.api.url, integrity: config.api.integrity });
  }

  if (config.auth?.integrity && config.auth.url) {
    remotes.push({ key: "auth", url: config.auth.url, integrity: config.auth.integrity });
  }

  for (const [key, plugin] of Object.entries(config.plugins ?? {})) {
    if (plugin.integrity && plugin.url) {
      remotes.push({ key, url: plugin.url, integrity: plugin.integrity });
    }
    if (plugin.ui?.integrity && plugin.ui.url) {
      remotes.push({ key: `${key}-ui`, url: plugin.ui.url, integrity: plugin.ui.integrity });
    }
  }

  return remotes;
}

export function startIntegrityMonitor(
  config: RuntimeConfig,
  intervalMs: number = DEFAULT_INTERVAL_MS,
): () => void {
  if (config.env !== "production") {
    return () => {};
  }

  const remotes = extractMonitoredRemotes(config);
  if (remotes.length === 0) {
    return () => {};
  }

  logger.info(
    `[IntegrityMonitor] Monitoring ${remotes.length} remote(s) every ${intervalMs / 1000}s`,
  );

  const timer = setInterval(async () => {
    for (const remote of remotes) {
      try {
        await verifySriForUrl(remote.url, remote.integrity);
      } catch (error) {
        logger.error(
          `[IntegrityMonitor] INTEGRITY FAILURE for ${remote.key} (${remote.url}): ${error instanceof Error ? error.message : String(error)}`,
        );
      }
    }
  }, intervalMs);

  return () => {
    clearInterval(timer);
    logger.info("[IntegrityMonitor] Stopped");
  };
}
