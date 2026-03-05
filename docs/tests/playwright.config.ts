import { defineConfig, devices } from '@playwright/test';
import * as dotenv from 'dotenv';
import * as fs from 'fs';
import * as path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Load docs/.env.screenshot (base config)
const docsDir = path.resolve(__dirname, '..');
dotenv.config({ path: path.join(docsDir, '.env.screenshot') });

// Load ~/.ironclaw/.env and apply only GATEWAY_AUTH_TOKEN as an override
const ironclawEnv = path.join(process.env.HOME ?? '~', '.ironclaw', '.env');
if (fs.existsSync(ironclawEnv)) {
  const parsed = dotenv.parse(fs.readFileSync(ironclawEnv));
  if (parsed.GATEWAY_AUTH_TOKEN) {
    process.env.GATEWAY_AUTH_TOKEN = parsed.GATEWAY_AUTH_TOKEN;
  }
}

// IRONCLAW_TOKEN mirrors GATEWAY_AUTH_TOKEN (used by seed.ts)
if (!process.env.IRONCLAW_TOKEN && process.env.GATEWAY_AUTH_TOKEN) {
  process.env.IRONCLAW_TOKEN = process.env.GATEWAY_AUTH_TOKEN;
}

function parseViewport(env: string | undefined) {
  if (!env) return null;
  const [w, h] = env.split('x').map(Number);
  return { width: w, height: h };
}

export default defineConfig({
  testDir: './specs',
  outputDir: '../assets/screenshots/',
  fullyParallel: false, // Sequential for consistent state
  workers: 1,
  use: {
    baseURL: process.env.IRONCLAW_URL ?? 'http://localhost:3000',
    viewport: parseViewport(process.env.SCREENSHOT_VIEWPORT) ?? { width: 1280, height: 800 },
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chromium'] },
    },
  ],
});
