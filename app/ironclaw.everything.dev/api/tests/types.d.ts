import type Plugin from "@/index";
import pluginDevConfig from "../plugin.dev";

declare module "every-plugin" {
  interface RegisteredPlugins {
    [pluginDevConfig.pluginId]: typeof Plugin;
  }
}

declare module "virtual:drizzle-migrations.sql" {
  export interface Migration {
    idx: number;
    when: number;
    tag: string;
    hash: string;
    sql: string[];
  }

  const migrations: Migration[];
  export default migrations;
}
