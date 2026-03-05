import { test } from '@playwright/test';
import { isIronClawReady, getIronClawErrorMessage, getIronClawUrlWithToken } from '../fixtures/seed.js';

test('memory tab overview', async ({ page }) => {
  // Check if IronClaw is running
  const ready = await isIronClawReady();
  if (!ready) {
    console.log(getIronClawErrorMessage());
    test.skip(true, 'IronClaw not running');
    return;
  }

  // Get URL with token and navigate to root (memory is client-side route)
  const url = await getIronClawUrlWithToken('/');
  await page.goto(url);

  // Wait for auto-authentication to complete and app to be visible
  await page.waitForSelector('#app', { state: 'visible', timeout: 10000 });
  await page.waitForTimeout(500); // Extra buffer for content to render

  // Click on the Memory tab
  await page.click('button[data-tab="memory"]');
  await page.waitForTimeout(500); // Wait for tab content to load

  // Take screenshot of the memory tab
  await page.screenshot({
    path: '../assets/screenshots/web-memory-overview.png',
    fullPage: false,
  });
});
