// Health check for IronClaw web UI

const AUTH_TOKEN = process.env.IRONCLAW_TOKEN ?? 'screenshot-test-token';

// List of candidate ports to check, starting from lowest
const CANDIDATE_PORTS = [3000, 3001, 3002, 3003, 3004, 3005, 3006, 3007, 3008, 3009, 3010, 8080, 13001];

// Check if a port responds with HTTP 200 to /api/health
async function checkPortHealth(port: number): Promise<boolean> {
  try {
    const response = await fetch(`http://127.0.0.1:${port}/api/health`, {
      method: 'GET',
      signal: AbortSignal.timeout(3000),
    });
    return response.status === 200;
  } catch {
    return false;
  }
}

// Try to auto-detect IronClaw port by checking HTTP health endpoint
async function detectIronClawPort(): Promise<string | null> {
  // Check configured URL first if provided
  if (process.env.IRONCLAW_URL) {
    try {
      const url = new URL(process.env.IRONCLAW_URL);
      const port = parseInt(url.port, 10);
      if (await checkPortHealth(port)) {
        return process.env.IRONCLAW_URL;
      }
    } catch {
      // Invalid URL, continue to auto-detection
    }
  }

  // Check candidate ports starting from lowest
  for (const port of CANDIDATE_PORTS) {
    if (await checkPortHealth(port)) {
      return `http://127.0.0.1:${port}`;
    }
  }

  return null;
}

let detectedUrl: string | null = null;

async function getBaseUrl(): Promise<string> {
  if (detectedUrl) return detectedUrl;

  // Use configured URL or auto-detect
  detectedUrl = process.env.IRONCLAW_URL ?? await detectIronClawPort();

  if (!detectedUrl) {
    throw new Error('IronClaw not found on any known port');
  }

  return detectedUrl;
}

// Check if IronClaw web UI is available
export async function isIronClawReady(): Promise<boolean> {
  try {
    const base = await getBaseUrl();
    const response = await fetch(`${base}/api/health`, {
      signal: AbortSignal.timeout(5000),
    });
    return response.ok;
  } catch {
    return false;
  }
}

// Get helpful error message for when IronClaw is not running
export function getIronClawErrorMessage(): string {
  return `
⚠️  IronClaw is not running

The script checked for:
- Configured IRONCLAW_URL environment variable
- HTTP health endpoint on ports: ${CANDIDATE_PORTS.join(', ')}

Detected URL: ${detectedUrl || 'Not detected'}

To run screenshots:
1. Build IronClaw: cargo build --release
2. Start IronClaw with gateway enabled:
   GATEWAY_ENABLED=true ./target/release/ironclaw
3. Or use the capture script which auto-detects and manages IronClaw:
   ./docs/scripts/capture-screenshots.sh

The capture script will:
- Find the IronClaw process by name
- Check all listening ports with HTTP GET requests
- Filter out WebSocket ports (typically higher numbers)
- Use the first port that returns HTTP 200
- Or start a new instance if needed

You can also set IRONCLAW_URL to specify a custom URL:
   export IRONCLAW_URL=http://localhost:8080
`;
}

// Export the detected URL for Playwright tests
export async function getIronClawUrl(): Promise<string> {
  return await getBaseUrl();
}

// Get URL with token appended as query parameter
export async function getIronClawUrlWithToken(path?: string): Promise<string> {
  const baseUrl = await getBaseUrl();
  const token = process.env.IRONCLAW_TOKEN ?? 'screenshot-test-token';

  // Build the URL: base + path (if provided) + ?token=
  let url = baseUrl;
  if (path) {
    // Ensure path starts with /
    const normalizedPath = path.startsWith('/') ? path : `/${path}`;
    // Ensure base doesn't end with / to avoid double slashes
    url = baseUrl.endsWith('/') ? baseUrl.slice(0, -1) : baseUrl;
    url = `${url}${normalizedPath}`;
  }

  return `${url}?token=${token}`;
}
