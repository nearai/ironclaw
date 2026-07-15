import { readdirSync, readFileSync } from "node:fs";
import { dirname, extname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import ts from "typescript";

const JAVASCRIPT_FAMILY_EXTENSIONS = new Set([
  ".cjs",
  ".cts",
  ".js",
  ".jsx",
  ".mjs",
  ".mts",
  ".ts",
  ".tsx",
]);
const TYPESCRIPT_EXTENSIONS = new Set([".ts", ".tsx"]);
const EXPLICIT_RELATIVE_EXTENSION = /\.(?:[cm]?[jt]sx?)(?:[?#].*)?$/i;

export type ConventionViolationKind =
  | "explicit-relative-extension"
  | "html-tagged-template"
  | "invalid-module-extension";

export type ConventionViolation = {
  file: string;
  kind: ConventionViolationKind;
  line: number;
};

function isRelativeModuleSpecifier(value: string): boolean {
  return value.startsWith("./") || value.startsWith("../");
}

function scriptKindForExtension(extension: string): ts.ScriptKind {
  if (extension === ".tsx") return ts.ScriptKind.TSX;
  if (extension === ".jsx") return ts.ScriptKind.JSX;
  if ([".js", ".mjs", ".cjs"].includes(extension)) return ts.ScriptKind.JS;
  return ts.ScriptKind.TS;
}

function lineForNode(sourceFile: ts.SourceFile, node: ts.Node): number {
  return sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1;
}

function moduleSpecifierForNode(node: ts.Node): ts.StringLiteralLike | undefined {
  if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
    return node.moduleSpecifier && ts.isStringLiteralLike(node.moduleSpecifier)
      ? node.moduleSpecifier
      : undefined;
  }
  if (
    ts.isCallExpression(node) &&
    node.expression.kind === ts.SyntaxKind.ImportKeyword &&
    node.arguments.length >= 1 &&
    ts.isStringLiteralLike(node.arguments[0])
  ) {
    return node.arguments[0];
  }
  if (
    ts.isImportEqualsDeclaration(node) &&
    ts.isExternalModuleReference(node.moduleReference) &&
    node.moduleReference.expression &&
    ts.isStringLiteralLike(node.moduleReference.expression)
  ) {
    return node.moduleReference.expression;
  }
  if (
    ts.isImportTypeNode(node) &&
    ts.isLiteralTypeNode(node.argument) &&
    ts.isStringLiteralLike(node.argument.literal)
  ) {
    return node.argument.literal;
  }
  return undefined;
}

function compareViolations(left: ConventionViolation, right: ConventionViolation): number {
  return (
    left.file.localeCompare(right.file) ||
    left.line - right.line ||
    left.kind.localeCompare(right.kind)
  );
}

export function checkSourceFile(filePath: string, sourceText: string): ConventionViolation[] {
  const extension = extname(filePath).toLowerCase();
  if (!JAVASCRIPT_FAMILY_EXTENSIONS.has(extension)) return [];

  const violations: ConventionViolation[] = [];
  if (!TYPESCRIPT_EXTENSIONS.has(extension)) {
    violations.push({ file: filePath, kind: "invalid-module-extension", line: 1 });
  }

  const sourceFile = ts.createSourceFile(
    filePath,
    sourceText,
    ts.ScriptTarget.Latest,
    true,
    scriptKindForExtension(extension),
  );

  function visit(node: ts.Node): void {
    const moduleSpecifier = moduleSpecifierForNode(node);
    if (
      moduleSpecifier &&
      isRelativeModuleSpecifier(moduleSpecifier.text) &&
      EXPLICIT_RELATIVE_EXTENSION.test(moduleSpecifier.text)
    ) {
      violations.push({
        file: filePath,
        kind: "explicit-relative-extension",
        line: lineForNode(sourceFile, moduleSpecifier),
      });
    }
    if (
      ts.isTaggedTemplateExpression(node) &&
      ts.isIdentifier(node.tag) &&
      node.tag.text === "html"
    ) {
      violations.push({
        file: filePath,
        kind: "html-tagged-template",
        line: lineForNode(sourceFile, node),
      });
    }
    ts.forEachChild(node, visit);
  }

  visit(sourceFile);
  return violations.sort(compareViolations);
}

function sourceFilesUnder(root: string): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = resolve(root, entry.name);
    if (entry.isDirectory()) {
      files.push(...sourceFilesUnder(path));
    } else if (entry.isFile()) {
      files.push(path);
    }
  }
  return files.sort();
}

export function checkSourceTree(sourceRoot: string): ConventionViolation[] {
  const root = resolve(sourceRoot);
  const violations = sourceFilesUnder(root).flatMap((absolutePath) => {
    const file = relative(root, absolutePath).split(sep).join("/");
    return checkSourceFile(file, readFileSync(absolutePath, "utf8"));
  });
  return violations.sort(compareViolations);
}

const VIOLATION_MESSAGES: Record<ConventionViolationKind, string> = {
  "explicit-relative-extension": "relative module imports must be extensionless",
  "html-tagged-template": "React markup must use JSX instead of html tagged templates",
  "invalid-module-extension": "authored modules must use .ts or .tsx",
};

export function formatViolation(violation: ConventionViolation): string {
  return `${violation.file}:${violation.line}: ${VIOLATION_MESSAGES[violation.kind]}`;
}

function runCli(): void {
  const scriptDirectory = dirname(fileURLToPath(import.meta.url));
  const sourceRoot = resolve(scriptDirectory, "../src");
  const violations = checkSourceTree(sourceRoot);
  if (violations.length === 0) return;

  for (const violation of violations) {
    console.error(formatViolation(violation));
  }
  console.error(`Found ${violations.length} frontend source convention violation(s).`);
  process.exitCode = 1;
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : undefined;
if (invokedPath === fileURLToPath(import.meta.url)) runCli();
