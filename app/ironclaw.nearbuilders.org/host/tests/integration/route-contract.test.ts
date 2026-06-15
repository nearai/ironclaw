import { describe, expect, it } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const currentDir = path.dirname(fileURLToPath(import.meta.url));
const routeTreePath = path.resolve(currentDir, "../../../ui/src/routeTree.gen.ts");

interface RouteRecord {
  id: string;
  path: string;
  fullPath: string;
}

function parseRouteRecords(content: string): RouteRecord[] {
  const records: RouteRecord[] = [];
  const regex = /\{\s*id:\s*'([^']+)'[\s\S]*?fullPath:\s*'([^']+)'/g;
  let match: RegExpExecArray | null;
  while ((match = regex.exec(content)) !== null) {
    records.push({ id: match[1], path: "", fullPath: match[2] });
  }
  return records;
}

describe("route-contract", () => {
  const content = readFileSync(routeTreePath, "utf8");
  const records = parseRouteRecords(content);
  const fullPaths = records.map((r) => r.fullPath);

  it("generated route tree file exists", () => {
    expect(existsSync(routeTreePath)).toBe(true);
  });

  it("no duplicate fullPaths in the route tree", () => {
    const seen = new Map<string, string[]>();
    for (const r of records) {
      if (!seen.has(r.fullPath)) {
        seen.set(r.fullPath, []);
      }
      seen.get(r.fullPath)!.push(r.id);
    }
    const duplicates = Array.from(seen.entries()).filter(([, ids]) => ids.length > 1);
    const dupes = duplicates.filter(([fp]) => fp !== "/" && fp !== "");
    expect(dupes).toHaveLength(0);
  });

  it("/settings exists exactly once", () => {
    const settings = fullPaths.filter((fp) => fp === "/settings");
    expect(settings).toHaveLength(1);
  });

  it("/settings/ironclaw exists", () => {
    expect(fullPaths).toContain("/settings/ironclaw");
  });

  it("/ironclaw/control exists", () => {
    expect(fullPaths).toContain("/ironclaw/control");
  });

  it("/setup guide exists", () => {
    expect(fullPaths).toContain("/setup");
  });

  it("/home exists", () => {
    expect(fullPaths).toContain("/home");
  });

  it("/login exists", () => {
    expect(fullPaths).toContain("/login");
  });
});
