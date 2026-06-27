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

function installFileReaderStub() {
  globalThis.FileReader = class {
    readAsDataURL(blob) {
      blob.arrayBuffer()
        .then((buffer) => {
          const base64 = Buffer.from(buffer).toString("base64");
          this.result = `data:${blob.type};base64,${base64}`;
          this.onload?.();
        })
        .catch((error) => {
          this.error = error;
          this.onerror?.();
        });
    }
  };
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

test("readWorkspaceFile previews bounded images as data URLs", async () => {
  installFileReaderStub();
  const pngBytes = new Uint8Array([137, 80, 78, 71]);
  const calls = installBrowserStubs({
    responses: [
      jsonResponse({
        stat: {
          kind: "file",
          mime_type: "image/png",
          size_bytes: pngBytes.byteLength,
        },
      }),
      new Response(pngBytes, {
        status: 200,
        headers: { "content-type": "image/png" },
      }),
    ],
  });

  const result = await readWorkspaceFile("workspace/images/logo.png");

  assert.equal(result.kind, "image");
  assert.equal(result.image_data_url, "data:image/png;base64,iVBORw==");
  assert.equal(result.download_path, "/api/webchat/v2/fs/content?mount=workspace&path=images%2Flogo.png");
  assert.equal(
    calls[1].path,
    "/api/webchat/v2/fs/content?mount=workspace&path=images%2Flogo.png",
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

test("readWorkspaceFile does not fetch oversized text or images for inline preview", async () => {
  const calls = installBrowserStubs({
    responses: [
      jsonResponse({
        stat: {
          kind: "file",
          mime_type: "text/plain",
          size_bytes: 1024 * 1024 + 1,
        },
      }),
      jsonResponse({
        stat: {
          kind: "file",
          mime_type: "image/jpeg",
          size_bytes: 8 * 1024 * 1024 + 1,
        },
      }),
    ],
  });

  const textResult = await readWorkspaceFile("workspace/logs/large.log");
  const imageResult = await readWorkspaceFile("workspace/images/huge.jpg");

  assert.equal(textResult.kind, "binary");
  assert.equal(textResult.download_path, "/api/webchat/v2/fs/content?mount=workspace&path=logs%2Flarge.log");
  assert.equal(imageResult.kind, "binary");
  assert.equal(imageResult.download_path, "/api/webchat/v2/fs/content?mount=workspace&path=images%2Fhuge.jpg");
  assert.equal(calls.length, 2);
});

test("readWorkspaceFile sniffs unknown NUL or invalid UTF-8 bytes as binary", async () => {
  const calls = installBrowserStubs({
    responses: [
      jsonResponse({
        stat: {
          kind: "file",
          mime_type: "application/octet-stream",
          size_bytes: 3,
        },
      }),
      new Response(new Uint8Array([65, 0, 66]), {
        status: 200,
        headers: { "content-type": "application/octet-stream" },
      }),
      jsonResponse({
        stat: {
          kind: "file",
          mime_type: "application/octet-stream",
          size_bytes: 1,
        },
      }),
      new Response(new Uint8Array([0xff]), {
        status: 200,
        headers: { "content-type": "application/octet-stream" },
      }),
    ],
  });

  const nulResult = await readWorkspaceFile("workspace/bin/nul.dat");
  const invalidUtf8Result = await readWorkspaceFile("workspace/bin/invalid.dat");

  assert.equal(nulResult.kind, "binary");
  assert.equal(nulResult.download_path, "/api/webchat/v2/fs/content?mount=workspace&path=bin%2Fnul.dat");
  assert.equal(invalidUtf8Result.kind, "binary");
  assert.equal(
    invalidUtf8Result.download_path,
    "/api/webchat/v2/fs/content?mount=workspace&path=bin%2Finvalid.dat",
  );
  assert.equal(calls.length, 4);
});
