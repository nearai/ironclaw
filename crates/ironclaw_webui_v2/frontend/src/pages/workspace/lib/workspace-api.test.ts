// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { listWorkspace, readWorkspaceFile } from "./workspace-api";

const CURRENT_USER = { tenant_id: "tenant-a", user_id: "alice" };
const SCOPED_USER_PATH = "tenants/tenant-a/users/alice";
const SCOPED_USER_QUERY = "tenants%2Ftenant-a%2Fusers%2Falice";
const MEMORY_AGENT_PATH = `${SCOPED_USER_PATH}/agents/reborn-cli-agent`;
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

function memoryScopeListResponse(path, projectEntries = [MEMORY_HELLO_ENTRY]) {
  if (path === `/api/webchat/v2/fs/list?mount=memory&path=${SCOPED_USER_QUERY}`) {
    return jsonResponse({
      entries: [
        { name: "agents", path: `${SCOPED_USER_PATH}/agents`, kind: "directory" },
      ],
    });
  }
  if (path === `/api/webchat/v2/fs/list?mount=memory&path=${SCOPED_USER_QUERY}%2Fagents`) {
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

test("workspace root keeps home visible alongside memory", async () => {
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
        { name: "home", path: "workspace", is_dir: true },
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

test("hosted workspace lists the caller subtree as home without exposing tenant path", async () => {
  const harness = installFetch((path) => {
    assert.equal(
      path,
      `/api/webchat/v2/fs/list?mount=workspace&path=${SCOPED_USER_QUERY}`
    );
    return jsonResponse({
      entries: [
        {
          name: "mine.txt",
          path: `${SCOPED_USER_PATH}/mine.txt`,
          kind: "file",
        },
      ],
    });
  });

  try {
    const response = await listWorkspace("workspace", { currentUser: CURRENT_USER });

    assert.deepEqual(response, {
      entries: [{ name: "mine.txt", path: "workspace/mine.txt", is_dir: false }],
    });
  } finally {
    harness.restore();
  }
});

test("local workspace falls back to the raw workspace root when no caller subtree exists", async () => {
  const harness = installFetch((path) => {
    if (path === `/api/webchat/v2/fs/list?mount=workspace&path=${SCOPED_USER_QUERY}`) {
      return jsonResponse({ error: "not_found" }, 404);
    }
    assert.equal(path, "/api/webchat/v2/fs/list?mount=workspace");
    return jsonResponse({
      entries: [{ name: "local.txt", path: "local.txt", kind: "file" }],
    });
  });

  try {
    const response = await listWorkspace("workspace", { currentUser: CURRENT_USER });

    assert.deepEqual(response, {
      entries: [{ name: "local.txt", path: "workspace/local.txt", is_dir: false }],
    });
  } finally {
    harness.restore();
  }
});

test("hosted workspace does not fall back to the raw tenant root", async () => {
  const harness = installFetch((path) => {
    if (path === `/api/webchat/v2/fs/list?mount=workspace&path=${SCOPED_USER_QUERY}`) {
      return jsonResponse({ error: "not_found" }, 404);
    }
    assert.equal(path, "/api/webchat/v2/fs/list?mount=workspace");
    return jsonResponse({
      entries: [{ name: "tenants", path: "tenants", kind: "directory" }],
    });
  });

  try {
    const response = await listWorkspace("workspace", { currentUser: CURRENT_USER });

    assert.deepEqual(response, { entries: [] });
  } finally {
    harness.restore();
  }
});

test("hosted workspace hides a raw shared root when scoped projection is required", async () => {
  const harness = installFetch((path) => {
    if (path === `/api/webchat/v2/fs/list?mount=workspace&path=${SCOPED_USER_QUERY}`) {
      return jsonResponse({ error: "not_found" }, 404);
    }
    assert.equal(path, "/api/webchat/v2/fs/list?mount=workspace");
    return jsonResponse({
      entries: [{ name: "shared.txt", path: "shared.txt", kind: "file" }],
    });
  });

  try {
    const response = await listWorkspace("workspace", {
      currentUser: CURRENT_USER,
      requireScopedWorkspace: true,
    });

    assert.deepEqual(response, { entries: [] });
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

test("workspace file preview reads through the hidden caller subtree when it exists", async () => {
  const harness = installFetch((path) => {
    if (path === `/api/webchat/v2/fs/list?mount=workspace&path=${SCOPED_USER_QUERY}`) {
      return jsonResponse({ entries: [] });
    }
    if (
      path ===
      `/api/webchat/v2/fs/stat?mount=workspace&path=${SCOPED_USER_QUERY}%2Fmine.txt`
    ) {
      return jsonResponse({
        stat: { kind: "file", mime_type: "text/plain", size_bytes: 5 },
      });
    }
    if (
      path ===
      `/api/webchat/v2/fs/content?mount=workspace&path=${SCOPED_USER_QUERY}%2Fmine.txt`
    ) {
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
    });

    assert.equal(response.kind, "text");
    assert.equal(response.path, "workspace/mine.txt");
    assert.equal(response.content, "hello");
    assert.equal(
      response.download_path,
      `/api/webchat/v2/fs/content?mount=workspace&path=${SCOPED_USER_QUERY}%2Fmine.txt`
    );
  } finally {
    harness.restore();
  }
});

test("hosted workspace file preview does not read the raw shared root", async () => {
  const harness = installFetch((path) => {
    if (path === `/api/webchat/v2/fs/list?mount=workspace&path=${SCOPED_USER_QUERY}`) {
      return jsonResponse({ error: "not_found" }, 404);
    }
    if (path === "/api/webchat/v2/fs/list?mount=workspace") {
      return jsonResponse({
        entries: [{ name: "shared.txt", path: "shared.txt", kind: "file" }],
      });
    }
    assert.notEqual(path, "/api/webchat/v2/fs/stat?mount=workspace&path=shared.txt");
    assert.equal(
      path,
      `/api/webchat/v2/fs/stat?mount=workspace&path=${SCOPED_USER_QUERY}%2Fshared.txt`
    );
    return jsonResponse({ error: "not_found" }, 404);
  });

  try {
    await assert.rejects(
      readWorkspaceFile("workspace/shared.txt", {
        currentUser: CURRENT_USER,
        requireScopedWorkspace: true,
      })
    );
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
    const response = await listWorkspace("memory", { currentUser: CURRENT_USER });

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
    assert.equal(path, `/api/webchat/v2/fs/list?mount=memory&path=${SCOPED_USER_QUERY}`);
    return jsonResponse({ error: "not_found" }, 404);
  });

  try {
    const response = await listWorkspace("memory", { currentUser: CURRENT_USER });

    assert.deepEqual(response, { entries: [] });
  } finally {
    harness.restore();
  }
});

test("memory file preview reads through the hidden caller subtree", async () => {
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
  const rawOtherUserPath = "tenants/tenant-a/users/bob/agents/reborn-cli-agent/projects/_none/secret.md";
  const rawOtherUserQuery = encodeURIComponent(rawOtherUserPath);
  const harness = installFetch((path) => {
    assert.notEqual(
      path,
      `/api/webchat/v2/fs/stat?mount=memory&path=${rawOtherUserQuery}`
    );
    assert.notEqual(
      path,
      `/api/webchat/v2/fs/content?mount=memory&path=${rawOtherUserQuery}`
    );
    const response = memoryScopeListResponse(path);
    if (response) return response;
    if (path === `/api/webchat/v2/fs/list?mount=memory&path=${MEMORY_PROJECT_QUERY}%2Ftenants`) {
      return jsonResponse({ error: "not_found" }, 404);
    }
    throw new Error(`unexpected fetch ${path}`);
  });

  try {
    await assert.rejects(
      readWorkspaceFile(`memory/${rawOtherUserPath}`, { currentUser: CURRENT_USER })
    );
  } finally {
    harness.restore();
  }
});
