// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function visit(node, fn, visited = new WeakSet()) {
  if (!node || typeof node !== "object" || visited.has(node)) return;
  visited.add(node);
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn, visited);
    return;
  }
  fn(node);
  visit(node.values, fn, visited);
}

function componentProps(root, component) {
  const props = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (let index = 0; index < node.values.length; index += 1) {
      if (node.values[index] !== component) continue;
      const current = {};
      for (let propIndex = index + 1; propIndex < node.values.length; propIndex += 1) {
        const name = node.strings[propIndex]?.match(/([A-Za-z][A-Za-z0-9-]*)=\s*$/)?.[1];
        if (name) current[name] = node.values[propIndex];
      }
      props.push(current);
    }
  });
  return props;
}

function createHarness({ queryError = null, removeResult = { success: true } } = {}) {
  const hookValues = [];
  let hookCursor = 0;
  const removeCalls = [];
  function ConfirmDialog() {}
  function InlineNotice() {}
  function SkillCard() {}
  const context = {
    Button() {},
    Card() {},
    ConfirmDialog,
    InlineNotice,
    React: {
      useCallback: (fn) => fn,
      useState: (initial) => {
        const index = hookCursor;
        hookCursor += 1;
        if (!(index in hookValues)) {
          hookValues[index] = typeof initial === "function" ? initial() : initial;
        }
        return [
          hookValues[index],
          (next) => {
            hookValues[index] =
              typeof next === "function" ? next(hookValues[index]) : next;
          },
        ];
      },
    },
    SettingsSearchEmpty() {},
    Skeleton() {},
    SkillCard,
    SkillInstallPanel() {},
    matchesSearch: () => true,
    useSkills: () => ({
      skills: [{
        name: "markdown-helper",
        source_kind: "installed",
        can_delete: true,
      }],
      query: { isLoading: false, error: queryError },
      autoActivateLearned: true,
      fetchSkillContent: () => {},
      installSkill: () => {},
      removeSkill: async (name) => {
        removeCalls.push(name);
        return removeResult;
      },
      updateSkill: () => {},
      setSkillAutoActivate: () => {},
      setAutoActivateLearned: () => {},
      isInstalling: false,
      isRemoving: false,
      isUpdating: false,
      isSettingAutoActivate: false,
      isSettingAutoActivateLearned: false,
    }),
    useT: () => (key, params) =>
      params?.name ? `${key}:${params.name}` : key,
  };
  const exports = runVmModuleForTest(
    "./skills-tab.tsx",
    ["SkillsTab", "SkillActionResult", "SkillGroup"],
    context,
    import.meta.url
  );
  return {
    ConfirmDialog,
    InlineNotice,
    SkillActionResult: exports.SkillActionResult,
    SkillGroup: exports.SkillGroup,
    removeCalls,
    render() {
      hookCursor = 0;
      return exports.SkillsTab({});
    },
  };
}

test("template visitor ignores circular references", () => {
  const node = { values: [] };
  node.values.push(node);
  let visits = 0;

  visit(node, () => {
    visits += 1;
  });

  assert.equal(visits, 1);
});

test("SkillsTab removes a skill only after confirming the shared dialog", async () => {
  const harness = createHarness();
  let rendered = harness.render();
  const [group] = componentProps(rendered, harness.SkillGroup);

  group.onRemove("markdown-helper");
  assert.deepEqual(harness.removeCalls, []);

  rendered = harness.render();
  let [dialog] = componentProps(rendered, harness.ConfirmDialog);
  assert.equal(dialog.open, true);
  assert.equal(dialog.title, "skills.confirmDelete:markdown-helper");

  dialog.onCancel();
  rendered = harness.render();
  [dialog] = componentProps(rendered, harness.ConfirmDialog);
  assert.equal(dialog.open, false);
  assert.deepEqual(harness.removeCalls, []);

  componentProps(rendered, harness.SkillGroup)[0].onRemove("markdown-helper");
  rendered = harness.render();
  [dialog] = componentProps(rendered, harness.ConfirmDialog);
  await dialog.onConfirm();
  assert.deepEqual(harness.removeCalls, ["markdown-helper"]);

  rendered = harness.render();
  [dialog] = componentProps(rendered, harness.ConfirmDialog);
  assert.equal(dialog.open, false);
});

test("SkillsTab renders query failures through a danger alert notice", () => {
  const harness = createHarness({ queryError: new Error("offline") });
  const rendered = harness.render();
  const [notice] = componentProps(rendered, harness.InlineNotice);

  assert.ok(notice, "expected query errors to render InlineNotice");
  assert.equal(notice.tone, "danger");
  assert.equal(notice.role, "alert");
  assert.equal(notice["data-testid"], "skills-load-error");
});

for (const scenario of [
  {
    name: "failed",
    removeResult: { success: false, message: "remove denied" },
    tone: "danger",
    role: "alert",
  },
  {
    name: "successful",
    removeResult: { success: true, message: "skill removed" },
    tone: "success",
    role: "status",
  },
]) {
  test(`SkillsTab renders a ${scenario.name} action result with semantic notice props`, async () => {
    const harness = createHarness({ removeResult: scenario.removeResult });
    let rendered = harness.render();

    componentProps(rendered, harness.SkillGroup)[0].onRemove("markdown-helper");
    rendered = harness.render();
    const [dialog] = componentProps(rendered, harness.ConfirmDialog);
    await dialog.onConfirm();

    rendered = harness.render();
    const [resultProps] = componentProps(rendered, harness.SkillActionResult);
    assert.ok(resultProps, "expected SkillsTab to render SkillActionResult");
    const result = harness.SkillActionResult(resultProps);
    const [notice] = componentProps(result, harness.InlineNotice);
    assert.ok(notice, "expected action results to render InlineNotice");
    assert.equal(notice.tone, scenario.tone);
    assert.equal(notice.role, scenario.role);
    assert.equal(notice["data-testid"], "skill-action-result");
  });
}
