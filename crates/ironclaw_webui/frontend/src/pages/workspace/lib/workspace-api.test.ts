// @vitest-environment happy-dom
import assert from "node:assert/strict";
import { beforeEach, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  apiFetch: vi.fn(),
  fetchAttachmentBlob: vi.fn(),
  fetchAttachmentDataUrl: vi.fn(),
  listProjectFiles: vi.fn(),
  projectFileContentUrl: vi.fn(),
  statProjectFile: vi.fn(),
}));

vi.mock("../../../lib/api", () => ({
  apiFetch: mocks.apiFetch,
  fetchAttachmentBlob: mocks.fetchAttachmentBlob,
  fetchAttachmentDataUrl: mocks.fetchAttachmentDataUrl,
  listProjectFiles: mocks.listProjectFiles,
  projectFileContentUrl: mocks.projectFileContentUrl,
  statProjectFile: mocks.statProjectFile,
}));

import { listWorkspace, readWorkspaceFile } from "./workspace-api";

beforeEach(() => {
  vi.clearAllMocks();
});

test("thread-scoped workspace roots expose only the authorized project mount", async () => {
  const result = await listWorkspace("", { threadId: "thread-project-beta" });

  assert.deepEqual(result, {
    entries: [{ name: "workspace", path: "workspace", is_dir: true }],
  });
  assert.equal(mocks.apiFetch.mock.calls.length, 0);
  assert.equal(mocks.listProjectFiles.mock.calls.length, 0);
});

test("thread-scoped directory listings preserve the source thread", async () => {
  mocks.listProjectFiles.mockResolvedValueOnce({
    entries: [
      { name: "report.md", path: "/workspace/reports/report.md", kind: "file" },
      { name: "archive", path: "/workspace/reports/archive", kind: "directory" },
      { name: "escape", path: "/workspace/../private.md", kind: "file" },
    ],
  });

  const result = await listWorkspace("workspace/reports", {
    threadId: "thread-project-beta",
  });

  assert.deepEqual(mocks.listProjectFiles.mock.calls, [[{
    threadId: "thread-project-beta",
    path: "/workspace/reports",
  }]]);
  assert.deepEqual(result, {
    entries: [
      { name: "report.md", path: "workspace/reports/report.md", is_dir: false },
      { name: "archive", path: "workspace/reports/archive", is_dir: true },
    ],
  });
  assert.equal(mocks.apiFetch.mock.calls.length, 0);
});

test("thread-scoped file reads use the same authorized content endpoint", async () => {
  mocks.statProjectFile.mockResolvedValueOnce({
    stat: { kind: "file", mime_type: "text/plain", size_bytes: 4 },
  });
  mocks.projectFileContentUrl.mockReturnValueOnce(
    "/api/webchat/v2/threads/thread-project-beta/files/content?path=%2Fworkspace%2Freport.txt",
  );
  mocks.fetchAttachmentBlob.mockResolvedValueOnce(
    new Blob(["test"], { type: "text/plain" }),
  );

  const result = await readWorkspaceFile("workspace/report.txt", {
    threadId: "thread-project-beta",
  });

  const scopedFile = {
    threadId: "thread-project-beta",
    path: "/workspace/report.txt",
  };
  assert.deepEqual(mocks.statProjectFile.mock.calls, [[scopedFile]]);
  assert.deepEqual(mocks.projectFileContentUrl.mock.calls, [[scopedFile]]);
  assert.deepEqual(mocks.fetchAttachmentBlob.mock.calls, [[
    "/api/webchat/v2/threads/thread-project-beta/files/content?path=%2Fworkspace%2Freport.txt",
  ]]);
  assert.equal(result.kind, "text");
  assert.equal("content" in result ? result.content : null, "test");
  assert.equal(mocks.apiFetch.mock.calls.length, 0);
});

test("thread-scoped browsing rejects other mounts instead of falling back", async () => {
  await assert.rejects(
    listWorkspace("memory/private.md", { threadId: "thread-project-beta" }),
    /limited to the workspace mount/,
  );
  await assert.rejects(
    readWorkspaceFile("memory/private.md", { threadId: "thread-project-beta" }),
    /limited to the workspace mount/,
  );
  await assert.rejects(
    readWorkspaceFile("workspace/../private.md", {
      threadId: "thread-project-beta",
    }),
    /Invalid thread-scoped workspace path/,
  );

  assert.equal(mocks.apiFetch.mock.calls.length, 0);
  assert.equal(mocks.listProjectFiles.mock.calls.length, 0);
  assert.equal(mocks.statProjectFile.mock.calls.length, 0);
});
