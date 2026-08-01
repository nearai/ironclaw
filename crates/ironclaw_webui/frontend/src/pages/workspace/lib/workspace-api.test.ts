// @vitest-environment happy-dom
import assert from "node:assert/strict";
import { beforeEach, test, vi } from "vitest";

const { apiFetchMock, fetchAttachmentBlobMock } = vi.hoisted(() => ({
  apiFetchMock: vi.fn(),
  fetchAttachmentBlobMock: vi.fn(),
}));

vi.mock("../../../lib/api", () => ({
  apiFetch: apiFetchMock,
  fetchAttachmentBlob: fetchAttachmentBlobMock,
  fetchAttachmentDataUrl: vi.fn(),
}));

beforeEach(() => {
  apiFetchMock.mockReset();
  fetchAttachmentBlobMock.mockReset();
});

test("workspace browse requests preserve the authorized project scope", async () => {
  apiFetchMock
    .mockResolvedValueOnce({ entries: [] })
    .mockResolvedValueOnce({
      stat: { kind: "file", mime_type: "text/plain", size_bytes: 4 },
    });
  fetchAttachmentBlobMock.mockResolvedValueOnce(
    new Blob(["test"], { type: "text/plain" }),
  );

  const { listWorkspace, readWorkspaceFile } = await import("./workspace-api");
  await listWorkspace("workspace/reports", "project/non-default");
  await readWorkspaceFile("workspace/report.txt", "project/non-default");

  assert.equal(
    apiFetchMock.mock.calls[0][0],
    "/api/webchat/v2/fs/list?mount=workspace&path=reports&project_id=project%2Fnon-default",
  );
  assert.equal(
    apiFetchMock.mock.calls[1][0],
    "/api/webchat/v2/fs/stat?mount=workspace&path=report.txt&project_id=project%2Fnon-default",
  );
  assert.equal(
    fetchAttachmentBlobMock.mock.calls[0][0],
    "/api/webchat/v2/fs/content?mount=workspace&path=report.txt&project_id=project%2Fnon-default",
  );
});
