import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { test } from "vitest";

const ALLOWED_NOTICE_TONES = new Set(["info", "success", "warning", "danger"]);
const ALLOWED_NOTICE_ROLES = new Set(["alert", "status"]);

const PAGE_FILES = [
  "./jobs/jobs-page.tsx",
  "./projects/projects-page.tsx",
  "./workspace/workspace-page.tsx",
  "./extensions/extensions-page.tsx",
] as const;

const SETTINGS_ADMIN_NOTICE_CONSUMERS = [
  ["./settings/settings-page.tsx", 1],
  ["./settings/components/settings-toolbar.tsx", 1],
  ["./settings/components/restart-banner.tsx", 3],
  ["./settings/components/skills-tab.tsx", 2],
  ["./settings/components/provider-management.tsx", 2],
  ["./settings/components/tools-tab.tsx", 2],
  ["./settings/components/trace-commons-tab.tsx", 3],
  ["./admin/components/configuration-tab.tsx", 3],
  ["./admin/components/users-tab.tsx", 2],
  ["./admin/components/user-detail.tsx", 2],
] as const;

function expressionValues(expression: ts.Expression): string[] | null {
  if (ts.isStringLiteralLike(expression)) return [expression.text];
  if (ts.isParenthesizedExpression(expression)) {
    return expressionValues(expression.expression);
  }
  if (ts.isConditionalExpression(expression)) {
    const whenTrue = expressionValues(expression.whenTrue);
    const whenFalse = expressionValues(expression.whenFalse);
    if (!whenTrue || !whenFalse) return null;
    return [...whenTrue, ...whenFalse];
  }
  return null;
}

function attributeValues(attribute: ts.JsxAttribute | undefined): string[] | null {
  if (!attribute?.initializer) return null;
  if (ts.isStringLiteral(attribute.initializer)) return [attribute.initializer.text];
  if (ts.isJsxExpression(attribute.initializer) && attribute.initializer.expression) {
    return expressionValues(attribute.initializer.expression);
  }
  return null;
}

function inlineNoticeAttributes(source: string, consumerFile: string) {
  const sourceFile = ts.createSourceFile(
    consumerFile,
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const notices: Array<{ roles: string[] | null; tones: string[] | null }> = [];

  const visit = (node: ts.Node) => {
    if (
      (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) &&
      node.tagName.getText(sourceFile) === "InlineNotice"
    ) {
      const attribute = (name: string) =>
        node.attributes.properties.find(
          (property): property is ts.JsxAttribute =>
            ts.isJsxAttribute(property) && property.name.getText(sourceFile) === name,
        );
      notices.push({
        roles: attributeValues(attribute("role")),
        tones: attributeValues(attribute("tone")),
      });
    }
    ts.forEachChild(node, visit);
  };

  visit(sourceFile);
  return notices;
}

for (const pageFile of PAGE_FILES) {
  test(`${pageFile} routes page feedback through InlineNotice`, () => {
    const source = readFileSync(new URL(pageFile, import.meta.url), "utf8");

    assert.match(source, /design-system\/inline-notice/);
    assert.match(source, /<InlineNotice/);
    assert.match(source, /role=(?:"(?:alert|status)"|\{)/);
  });
}

for (const [consumerFile, minimumNoticeCount] of SETTINGS_ADMIN_NOTICE_CONSUMERS) {
  test(`${consumerFile} routes page feedback through InlineNotice`, () => {
    const source = readFileSync(new URL(consumerFile, import.meta.url), "utf8");
    const notices = inlineNoticeAttributes(source, consumerFile);

    assert.match(source, /design-system\/inline-notice/);
    assert.ok(
      notices.length >= minimumNoticeCount,
      `expected at least ${minimumNoticeCount} InlineNotice consumers, found ${notices.length}`,
    );
    for (const [index, notice] of notices.entries()) {
      assert.ok(notice.tones?.length, `${consumerFile} notice ${index + 1} must declare tone`);
      assert.ok(notice.roles?.length, `${consumerFile} notice ${index + 1} must declare role`);
      for (const tone of notice.tones ?? []) {
        assert.ok(
          ALLOWED_NOTICE_TONES.has(tone),
          `${consumerFile} notice ${index + 1} has unsupported tone ${tone}`,
        );
      }
      for (const role of notice.roles ?? []) {
        assert.ok(
          ALLOWED_NOTICE_ROLES.has(role),
          `${consumerFile} notice ${index + 1} has unsupported role ${role}`,
        );
      }
    }
  });
}

test("legacy page feedback components are retired", () => {
  for (const legacyFile of [
    "./projects/components/feedback-banner.tsx",
    "./extensions/components/action-toast.tsx",
  ]) {
    assert.throws(
      () => readFileSync(new URL(legacyFile, import.meta.url), "utf8"),
      /ENOENT/,
    );
  }
});
