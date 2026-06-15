/// <reference types="@rsbuild/core/types" />

declare module "*.md" {
  const content: string;
  export default content;
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
