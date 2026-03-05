import { test } from '@playwright/test';
import { isIronClawReady, getIronClawErrorMessage, getIronClawUrlWithToken } from '../fixtures/seed.js';

test('chat interface overview', async ({ page }) => {
  // Check if IronClaw is running
  const ready = await isIronClawReady();
  if (!ready) {
    console.log(getIronClawErrorMessage());
    test.skip(true, 'IronClaw not running');
    return;
  }

  // Get URL with token and navigate
  const url = await getIronClawUrlWithToken('/');
  await page.goto(url);

  // Wait for auto-authentication to complete and app to be visible
  await page.waitForSelector('#app', { state: 'visible', timeout: 10000 });
  await page.waitForTimeout(500); // Extra buffer for content to render

  // Take screenshot of the chat interface
  await page.screenshot({
    path: '../assets/screenshots/web-chat-overview.png',
    fullPage: false,
  });
});
