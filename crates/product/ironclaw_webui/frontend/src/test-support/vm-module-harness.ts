import { readFileSync } from "node:fs";
import vm from "node:vm";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import type { VmModuleExports } from "./dynamic-test-types";

export type { VmComponentProps, VmModuleExports } from "./dynamic-test-types";

/**
 * Values exported by source evaluated in a VM cannot be inferred statically:
 * the harness removes module syntax and discovers the requested names at
 * runtime. Keep that dynamic boundary here instead of casting in every test.
 */
const STATIC_IMPORT_RE =
  /(^|\n)[ \t]*import(?:\s+[\s\S]*?\s+from\s*)?\s*["'][^"'\n]+["'](?:\s+(?:assert|with)\s*\{[\s\S]*?\})?\s*;?[ \t]*(?=\n|$)/g;
const NAMED_EXPORT_DECLARATION_RE =
  /^(\s*)export\s+(?=(?:async\s+)?function\b|class\b|interface\b|type\b|(?:const|let|var)\b)/gm;
const NAMED_EXPORT_LIST_RE = /^(\s*)export\s*\{([\s\S]*?)\}\s*;?[ \t]*$/gm;

function stripTypeScript(source: string): string {
  return ts.transpileModule(source, {
    compilerOptions: {
      jsx: ts.JsxEmit.Preserve,
      module: ts.ModuleKind.None,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText;
}

export function sourceForVmTest(
  path: string,
  exportNames: readonly string[],
  metaUrl: string | URL,
): string {
  return sourceTextForVmTest(
    readFileSync(new URL(path, metaUrl), "utf8"),
    exportNames
  );
}

export function sourceTextForVmTest(
  source: string,
  exportNames: readonly string[],
): string {
  const exportAliases = new Map<string, string>();
  const transformed = source
    .replace(STATIC_IMPORT_RE, "$1")
    .replace(NAMED_EXPORT_LIST_RE, (_match, _indent, specifiers) => {
      for (const specifier of specifiers.split(",")) {
        const trimmed = specifier.trim();
        if (!trimmed) continue;
        const [localName, exportedName = localName] = trimmed.split(/\s+as\s+/);
        exportAliases.set(exportedName.trim(), localName.trim());
      }
      return "";
    })
    .replace(NAMED_EXPORT_DECLARATION_RE, "$1");
  const executableSource = stripTypeScript(transformed);
  const testExports = exportNames
    .map((name) => {
      const localName = exportAliases.get(name) || name;
      return localName === name ? name : `${JSON.stringify(name)}: ${localName}`;
    })
    .join(", ");
  return `${executableSource.trimEnd()}\nglobalThis.__testExports = { ${testExports} };\n`;
}

export function runVmModuleForTest(
  path: string,
  exportNames: readonly string[],
  context: vm.Context,
  metaUrl: string | URL,
): VmModuleExports {
  const moduleUrl = new URL(path, metaUrl);
  // Capture exports on a detached object; the sandbox globals stay explicit.
  const sandbox = context as vm.Context & {
    globalThis: { __testExports?: VmModuleExports };
  };
  sandbox.globalThis = {};
  vm.runInNewContext(sourceForVmTest(path, exportNames, metaUrl), context, {
    filename: fileURLToPath(moduleUrl),
  });
  if (!sandbox.globalThis.__testExports) {
    throw new Error("VM module did not publish __testExports");
  }
  return sandbox.globalThis.__testExports;
}
