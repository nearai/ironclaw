import assert from "node:assert/strict";
import test from "node:test";

import {
  listWorkspace,
  readWorkspaceFile,
} from "./workspace-api.js";

function installBrowserStubs({ responses, token = "workspace-token" } = {}) {
  const calls = [];
  globalThis.window = { location: { origin: "https://app.test" } };
  globalThis.sessionStorage = {
    getItem: () => token,
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    const response = responses.shift();
    if (!response) throw new Error(`unexpected fetch: ${path}`);
    return response;
  };
  return calls;
}

function jsonResponse(body) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

test("listWorkspace root projects mounts as read-only top-level folders", async () => {
  const calls = installBrowserStubs({
    responses: [
      jsonResponse({
        mounts: [
          { mount: "workspace", label: "Workspace" },
          { mount: "memory", label: "Memory" },
        ],
      }),
    ],
  });

  const result = await listWorkspace("");

  assert.deepEqual(result, {
    entries: [
      { name: "home", path: "workspace", is_dir: true },
      { name: "memory", path: "memory", is_dir: true },
    ],
  });
  assert.equal(calls[0].path, "/api/webchat/v2/fs/mounts");
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer workspace-token");
});

test("listWorkspace qualifies child paths by mount", async () => {
  const calls = installBrowserStubs({
    responses: [
      jsonResponse({
        entries: [
          { name: "src", path: "src", kind: "directory" },
          { name: "README.md", path: "README.md", kind: "file" },
        ],
      }),
    ],
  });

  const result = await listWorkspace("workspace");

  assert.deepEqual(result, {
    entries: [
      { name: "src", path: "workspace/src", is_dir: true },
      { name: "README.md", path: "workspace/README.md", is_dir: false },
    ],
  });
  assert.equal(calls[0].path, "/api/webchat/v2/fs/list?mount=workspace");
});

test("readWorkspaceFile treats mount roots as directories without fetching", async () => {
  const calls = installBrowserStubs({ responses: [] });

  assert.deepEqual(await readWorkspaceFile("workspace"), {
    kind: "directory",
    path: "workspace",
  });
  assert.equal(calls.length, 0);
});

test("readWorkspaceFile previews bounded unknown text through authed bytes", async () => {
  const calls = installBrowserStubs({
    responses: [
      jsonResponse({
        stat: {
          kind: "file",
          mime_type: "application/octet-stream",
          size_bytes: 11,
        },
      }),
      new Response("hello world", {
        status: 200,
        headers: { "content-type": "application/octet-stream" },
      }),
    ],
  });

  const result = await readWorkspaceFile("workspace/notes/Dockerfile.worker");

  assert.equal(result.kind, "text");
  assert.equal(result.content, "hello world");
  assert.equal(result.download_path, "/api/webchat/v2/fs/content?mount=workspace&path=notes%2FDockerfile.worker");
  assert.equal(
    calls[0].path,
    "/api/webchat/v2/fs/stat?mount=workspace&path=notes%2FDockerfile.worker",
  );
  assert.equal(
    calls[1].path,
    "/api/webchat/v2/fs/content?mount=workspace&path=notes%2FDockerfile.worker",
  );
  assert.equal(calls[1].options.headers.get("Authorization"), "Bearer workspace-token");
});

test("readWorkspaceFile offers known binary files as downloads without byte fetch", async () => {
  const calls = installBrowserStubs({
    responses: [
      jsonResponse({
        stat: {
          kind: "file",
          mime_type: "application/pdf",
          size_bytes: 128,
        },
      }),
    ],
  });

  const result = await readWorkspaceFile("workspace/reports/plan.pdf");

  assert.equal(result.kind, "binary");
  assert.equal(result.download_path, "/api/webchat/v2/fs/content?mount=workspace&path=reports%2Fplan.pdf");
  assert.equal(calls.length, 1);
});
