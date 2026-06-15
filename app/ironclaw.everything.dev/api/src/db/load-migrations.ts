import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import type { Migration } from "virtual:drizzle-migrations.sql";

export async function loadMigrations(): Promise<Migration[]> {
  try {
    const mod = await import("virtual:drizzle-migrations.sql");
    return mod.default;
  } catch {
    return loadMigrationsFromDisk();
  }
}

function loadMigrationsFromDisk(): Migration[] {
  const migrationsDir = resolve(import.meta.dirname, "migrations");
  const metaDir = join(migrationsDir, "meta");

  const journal = JSON.parse(readFileSync(join(metaDir, "_journal.json"), "utf8"));

  return journal.entries.map((entry: { idx: number; when: number; tag: string }) => {
    const sqlPath = join(migrationsDir, `${entry.tag}.sql`);
    const raw = readFileSync(sqlPath, "utf8");
    const sql = raw.split("--> statement-breakpoint").map((s: string) => s.trim());
    const hash = createHash("sha256").update(raw).digest("hex").slice(0, 12);

    return {
      idx: entry.idx,
      when: entry.when,
      tag: entry.tag,
      hash,
      sql,
    };
  });
}
