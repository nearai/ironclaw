// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { listWorkspace, readWorkspaceFile } from "./workspace-api";

// `/fs/*` is caller-scoped server-side: the authenticated caller's tenant/user
// prefix is resolved by the backend, and the frontend consumes only
// mount-relative paths. `CURRENT_USER` is a fail-closed identity gate, never a
// path component.
const CURRENT_USER = { tenant_id: "tenant-a", user_id: "alice" };
const MEMORY_AGENT_PATH = "agents/reborn-cli-agent";
const MEMORY_PROJECT_PATH = `${MEMORY_AGENT_PATH}/projects/_none`;
const MEMORY_AGENT_QUERY = encodeURIComponent(MEMORY_AGENT_PATH);
const MEMORY_PROJECTS_QUERY = encodeURIComponent(`${MEMORY_AGENT_PATH}/projects`);
const MEMORY_PROJECT_QUERY = encodeURIComponent(MEMORY_PROJECT_PATH);
const MEMORY_HELLO_ENTRY = {
  name: "hello.md",
  path: `${MEMORY_PROJECT_PATH}/hello.md`,
  kind: "file",
};

function jsonResponse(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function installFetch(handler) {
  const originalFetch = globalThis.fetch;
  const originalSessionStorage = globalThis.sessionStorage;
  const originalWindow = globalThis.window;
  const calls = [];

  globalThis.window = { location: { origin: "http://localhost" } };
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return handler(path, options);
  };

  return {
    calls,
    restore() {
      globalThis.fetch = originalFetch;
      globalThis.sessionStorage = originalSessionStorage;
      globalThis.window = originalWindow;
    },
  };
}

// Memory mount-relative collapse walk. The server returns the caller's own
// memory tree; the frontend collapses the internal `agents/<agent>/projects/_none`
// wrapper directories so the user-facing root jumps straight to content.
function memoryScopeListResponse(path, projectEntries = [MEMORY_HELLO_ENTRY]) {
  if (path === "/api/webchat/v2/fs/list?mount=memory") {
    return jsonResponse({
      entries: [
        { name: "agents", path: "agents", kind: "directory" },
      ],
    });
  }
  if (path === "/api/webchat/v2/fs/list?mount=memory&path=agents") {
    return jsonResponse({
      entries: [
        { name: "reborn-cli-agent", path: MEMORY_AGENT_PATH, kind: "directory" },
      ],
    });
  }
  if (path === `/api/webchat/v2/fs/list?mount=memory&path=${MEMORY_AGENT_QUERY}`) {
    return jsonResponse({
      entries: [
        { name: "projects", path: `${MEMORY_AGENT_PATH}/projects`, kind: "directory" },
      ],
    });
  }
  if (path === `/api/webchat/v2/fs/list?mount=memory&path=${MEMORY_PROJECTS_QUERY}`) {
    return jsonResponse({
      entries: [
        { name: "_none", path: MEMORY_PROJECT_PATH, kind: "directory" },
      ],
    });
  }
  if (path === `/api/webchat/v2/fs/list?mount=memory&path=${MEMORY_PROJECT_QUERY}`) {
    return jsonResponse({ entries: projectEntries });
  }
  return null;
}

test("workspace root keeps workspace visible alongside memory", async () => {
  const harness = installFetch(() =>
    jsonResponse({
      mounts: [
        { mount: "workspace", label: "workspace" },
        { mount: "memory", label: "memory" },
      ],
    })
  );

  try {
    const response = await listWorkspace("");

    assert.deepEqual(response, {
      entries: [
        { name: "workspace", path: "workspace", is_dir: true },
        { name: "memory", path: "memory", is_dir: true },
      ],
    });
    assert.equal(harness.calls.length, 1);
    assert.equal(harness.calls[0].path, "/api/webchat/v2/fs/mounts");
    assert.equal(harness.calls[0].options.headers.get("Authorization"), "Bearer token-1");
  } finally {
    harness.restore();
  }
});

test("hosted workspace lists the caller's own mount-relative root", async () => {
  const harness = installFetch((path) => {
    assert.equal(path, "/api/webchat/v2/fs/list?mount=workspace");
    return jsonResponse({
      entries: [
        { name: "mine.txt", path: "mine.txt", kind: "file" },
      ],
    });
  });

  try {
    const response = await listWorkspace("workspace", {
      currentUser: CURRENT_USER,
      requireScopedWorkspace: true,
    });

    assert.deepEqual(response, {
      entries: [{ name: "mine.txt", path: "workspace/mine.txt", is_dir: false }],
    });
  } finally {
    harness.restore();
  }
});

test("local workspace lists the raw mount root when scoped projection is off", async () => {
  const harness = installFetch((path) => {
    assert.equal(path, "/api/webchat/v2/fs/list?mount=workspace");
    return jsonResponse({
      entries: [{ name: "local.txt", path: "local.txt", kind: "file" }],
    });
  });

  try {
    const response = await listWorkspace("workspace", {
      currentUser: CURRENT_USER,
      requireScopedWorkspace: false,
    });

    assert.deepEqual(response, {
      entries: [{ name: "local.txt", path: "workspace/local.txt", is_dir: false }],
    });
  } finally {
    harness.restore();
  }
});

test("hosted workspace stays empty before caller identity is resolved", async () => {
  const harness = installFetch((path) => {
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
    const response = await listWorkspace("workspace", {
      requireScopedWorkspace: true,
    });

    assert.deepEqual(response, { entries: [] });
    assert.deepEqual(harness.calls, []);
  } finally {
    harness.restore();
  }
});

test("workspace file preview reads through the mount-relative path", async () => {
  const harness = installFetch((path) => {
    if (path === "/api/webchat/v2/fs/stat?mount=workspace&path=mine.txt") {
      return jsonResponse({
        stat: { kind: "file", mime_type: "text/plain", size_bytes: 5 },
      });
    }
    if (path === "/api/webchat/v2/fs/content?mount=workspace&path=mine.txt") {
      return new Response("hello", {
        status: 200,
        headers: { "content-type": "text/plain" },
      });
    }
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
    const response = await readWorkspaceFile("workspace/mine.txt", {
      currentUser: CURRENT_USER,
      requireScopedWorkspace: true,
    });

    assert.equal(response.kind, "text");
    assert.equal(response.path, "workspace/mine.txt");
    assert.equal(response.content, "hello");
    assert.equal(
      response.download_path,
      "/api/webchat/v2/fs/content?mount=workspace&path=mine.txt"
    );
  } finally {
    harness.restore();
  }
});

test("hosted workspace file preview never requests another user's path", async () => {
  const forbidden = [];
  const forbiddenQuery = encodeURIComponent("tenants/tenant-a/users/bob/secret.txt");
  const harness = installFetch((path) => {
    if (path.includes(forbiddenQuery)) forbidden.push(path);
    if (path === "/api/webchat/v2/fs/list?mount=workspace") {
      return jsonResponse({ entries: [] });
    }
    if (path === "/api/webchat/v2/fs/stat?mount=workspace&path=secret.txt") {
      return jsonResponse({ error: "not_found" }, 404);
    }
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
    await assert.rejects(
      readWorkspaceFile("workspace/secret.txt", {
        currentUser: CURRENT_USER,
        requireScopedWorkspace: true,
      }),
      /not_found|404|error/i,
    );
    assert.deepEqual(forbidden, []);
  } finally {
    harness.restore();
  }
});

test("hosted workspace file preview waits for caller identity instead of statting raw root", async () => {
  const harness = installFetch((path) => {
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
    const response = await readWorkspaceFile("workspace/shared.txt", {
      requireScopedWorkspace: true,
    });

    assert.deepEqual(response, {
      kind: "directory",
      path: "workspace/shared.txt",
    });
    assert.deepEqual(harness.calls, []);
  } finally {
    harness.restore();
  }
});

test("memory lists the caller subtree without exposing storage wrapper folders", async () => {
  const harness = installFetch((path) => {
    const response = memoryScopeListResponse(path, [
      { name: "hello.md.chunks", path: `${MEMORY_PROJECT_PATH}/hello.md.chunks`, kind: "directory" },
      MEMORY_HELLO_ENTRY,
    ]);
    if (response) return response;
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
    const response = await listWorkspace("memory", {
      currentUser: CURRENT_USER,
      requireScopedWorkspace: true,
    });

    assert.deepEqual(response, {
      entries: [{ name: "hello.md", path: "memory/hello.md", is_dir: false }],
    });
  } finally {
    harness.restore();
  }
});

test("hosted memory stays empty before caller identity is resolved", async () => {
  const harness = installFetch((path) => {
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
    const response = await listWorkspace("memory", {
      requireScopedWorkspace: true,
    });

    assert.deepEqual(response, { entries: [] });
    assert.deepEqual(harness.calls, []);
  } finally {
    harness.restore();
  }
});

test("memory returns an empty scoped view when the caller subtree is missing", async () => {
  const harness = installFetch((path) => {
    assert.equal(path, "/api/webchat/v2/fs/list?mount=memory");
    return jsonResponse({ error: "not_found" }, 404);
  });

  try {
    const response = await listWorkspace("memory", {
      currentUser: CURRENT_USER,
      requireScopedWorkspace: true,
    });

    assert.deepEqual(response, { entries: [] });
  } finally {
    harness.restore();
  }
});

test("memory file preview reads through the collapsed mount-relative path", async () => {
  const harness = installFetch((path) => {
    const response = memoryScopeListResponse(path);
    if (response) return response;
    if (
      path ===
      `/api/webchat/v2/fs/stat?mount=memory&path=${MEMORY_PROJECT_QUERY}%2Fhello.md`
    ) {
      return jsonResponse({
        stat: { kind: "file", mime_type: "text/markdown", size_bytes: 5 },
      });
    }
    if (
      path ===
      `/api/webchat/v2/fs/content?mount=memory&path=${MEMORY_PROJECT_QUERY}%2Fhello.md`
    ) {
      return new Response("hello", {
        status: 200,
        headers: { "content-type": "text/plain" },
      });
    }
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
    const response = await readWorkspaceFile("memory/hello.md", {
      currentUser: CURRENT_USER,
      requireScopedWorkspace: true,
    });

    assert.equal(response.kind, "text");
    assert.equal(response.path, "memory/hello.md");
    assert.equal(response.content, "hello");
    assert.equal(
      response.download_path,
      `/api/webchat/v2/fs/content?mount=memory&path=${MEMORY_PROJECT_QUERY}%2Fhello.md`
    );
  } finally {
    harness.restore();
  }
});

test("memory file preview does not honor raw storage paths for scoped users", async () => {
  // The frontend passes the path mount-relative; the server is the authority
  // for caller scope and rejects a cross-user path with 404. The preview must
  // surface that rejection rather than silently rendering another user's file.
  const rawOtherUserPath = "tenants/tenant-a/users/bob/agents/reborn-cli-agent/projects/_none/secret.md";
  const rawOtherUserQuery = encodeURIComponent(rawOtherUserPath);
  const harness = installFetch((path) => {
    const response = memoryScopeListResponse(path);
    if (response) return response;
    if (path.includes(rawOtherUserQuery)) {
      return jsonResponse({ error: "not_found" }, 404);
    }
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
    await assert.rejects(
      readWorkspaceFile(`memory/${rawOtherUserPath}`, {
        currentUser: CURRENT_USER,
        requireScopedWorkspace: true,
      }),
      /not_found|404|error/i,
    );
  } finally {
    harness.restore();
  }
});

test("thread-scoped workspace roots expose only the authorized project mount", async () => {
  const harness = installFetch((path) => {
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
    const result = await listWorkspace("", { threadId: "thread-project-beta" });

    assert.deepEqual(result, {
      entries: [{ name: "workspace", path: "workspace", is_dir: true }],
    });
    assert.deepEqual(harness.calls, []);
  } finally {
    harness.restore();
  }
});

test("thread-scoped directory listings preserve the source thread", async () => {
  const harness = installFetch((path) => {
    assert.equal(
      path,
      "/api/webchat/v2/threads/thread-project-beta/files?path=%2Fworkspace%2Freports",
    );
    return jsonResponse({
      entries: [
        { name: "report.md", path: "/workspace/reports/report.md", kind: "file" },
        { name: "archive", path: "/workspace/reports/archive", kind: "directory" },
        { name: "escape", path: "/workspace/../private.md", kind: "file" },
      ],
    });
  });

  try {
    const result = await listWorkspace("workspace/reports", {
      threadId: "thread-project-beta",
    });

    assert.deepEqual(result, {
      entries: [
        { name: "report.md", path: "workspace/reports/report.md", is_dir: false },
        { name: "archive", path: "workspace/reports/archive", is_dir: true },
      ],
    });
    assert.equal(harness.calls.length, 1);
  } finally {
    harness.restore();
  }
});

test("thread-scoped file reads use the same authorized content endpoint", async () => {
  const statPath =
    "/api/webchat/v2/threads/thread-project-beta/files/stat?path=%2Fworkspace%2Freport.txt";
  const contentPath =
    "/api/webchat/v2/threads/thread-project-beta/files/content?path=%2Fworkspace%2Freport.txt";
  const harness = installFetch((path) => {
    if (path === statPath) {
      return jsonResponse({
        stat: { kind: "file", mime_type: "text/plain", size_bytes: 4 },
      });
    }
    if (path === contentPath) {
      return new Response("test", {
        status: 200,
        headers: { "content-type": "text/plain" },
      });
    }
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
    const result = await readWorkspaceFile("workspace/report.txt", {
      threadId: "thread-project-beta",
    });

    assert.equal(result.kind, "text");
    assert.equal(result.content, "test");
    assert.equal(result.download_path, contentPath);
    assert.deepEqual(
      harness.calls.map(({ path }) => path),
      [statPath, contentPath],
    );
  } finally {
    harness.restore();
  }
});

test("thread-scoped browsing rejects other mounts instead of falling back", async () => {
  const harness = installFetch((path) => {
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
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
    assert.deepEqual(harness.calls, []);
  } finally {
    harness.restore();
  }
});
