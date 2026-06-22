import { Data } from "every-plugin/effect";

export class FederationError extends Data.TaggedError("FederationError")<{
  readonly remoteName: string;
  readonly remoteUrl?: string;
  readonly cause?: unknown;
}> {
  get message() {
    const raw = this.cause instanceof FederationError ? this.cause.cause : this.cause;
    const detail = raw instanceof Error ? raw.message : String(raw ?? "");
    return `Failed to load ${this.remoteName}${this.remoteUrl ? ` from ${this.remoteUrl}` : ""}: ${detail}`;
  }
}

export class PluginError extends Data.TaggedError("PluginError")<{
  readonly pluginName?: string;
  readonly pluginUrl?: string;
  readonly cause?: unknown;
}> {
  get message() {
    const raw = this.cause instanceof PluginError ? this.cause.cause : this.cause;
    const detail = raw instanceof Error ? raw.message : String(raw ?? "");
    return `Plugin ${this.pluginName ?? "unknown"}${this.pluginUrl ? ` at ${this.pluginUrl}` : ""} failed: ${detail}`;
  }
}
