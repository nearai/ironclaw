import { expect, test } from "@playwright/test";

test.describe("app smoke (demo mode)", () => {
  test("boots into the workspace shell without a backend", async ({ page }) => {
    await page.goto("/");
    // Demo mode seeds a token, so the SPA lands on the default /chat route.
    await expect(page).toHaveURL(/\/chat/);
    await expect(page.getByRole("navigation").first()).toBeVisible();
  });
});
