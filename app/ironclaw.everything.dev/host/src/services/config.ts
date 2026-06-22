import { Context } from "every-plugin/effect";
import type {
  ClientRuntimeConfig,
  RuntimeConfig,
  SharedConfig,
  SourceMode,
} from "everything-dev/types";

export type { ClientRuntimeConfig, RuntimeConfig, SharedConfig, SourceMode };

export class ConfigService extends Context.Tag("host/ConfigService")<
  ConfigService,
  RuntimeConfig
>() {}
