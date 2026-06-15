import Plugin from "@/index";
import pluginDevConfig from "../plugin.dev";

declare module "every-plugin" {
  interface RegisteredPlugins {
    [pluginDevConfig.pluginId]: typeof Plugin;
  }
}
