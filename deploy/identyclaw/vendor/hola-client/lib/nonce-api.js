/**
 * Fetch a fresh HOLA nonce from GET /api/holanonce16ts (requires JWT).
 *
 * @param {object} params
 * @param {string} params.baseUrl - API base URL (default https://api.identyclaw.com)
 * @param {string} params.jwt - Bearer JWT from POST /api/login
 * @returns {Promise<{ noncetsHex: string, timestamp: string, length: number, algorithm: string, requestId?: string }>}
 */
async function getNonce({ baseUrl = "https://api.identyclaw.com", jwt }) {
  if (!jwt || typeof jwt !== "string") {
    throw new Error("jwt is required");
  }

  const resp = await fetch(`${baseUrl.replace(/\/+$/, "")}/api/holanonce16ts`, {
    headers: {
      authorization: `Bearer ${jwt}`
    }
  });

  if (!resp.ok) {
    const text = await resp.text().catch(() => "");
    throw new Error(`GET /api/holanonce16ts failed: HTTP ${resp.status}${text ? ` — ${text.trim()}` : ""}`);
  }

  const data = await resp.json();
  if (!data.noncetsHex || !data.timestamp) {
    throw new Error("Nonce response missing noncetsHex or timestamp");
  }

  return data;
}

module.exports = {
  getNonce
};
