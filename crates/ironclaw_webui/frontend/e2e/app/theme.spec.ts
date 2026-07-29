/**
 * Theme selection on /settings/appearance (demo mode).
 *
 * The appearance tab renders a light/dark radiogroup
 * (src/pages/settings/components/appearance-tab.tsx). Selecting a theme
 * flips html[data-theme] immediately and persists it to localStorage under
 * "ironclaw:v2-theme" (packages/ui/src/theme/theme.ts), surviving reloads.
 */
import { expect, test, type Page } from "@playwright/test";

const THEME_STORAGE_KEY = "ironclaw:v2-theme";

function html(page: Page) {
  return page.locator("html");
}

async function storedTheme(page: Page) {
  return page.evaluate(
    (key) => window.localStorage.getItem(key),
    THEME_STORAGE_KEY
  );
}

test.describe("theme toggle (settings/appearance)", () => {
  test("switching light↔dark updates html[data-theme] and persists across reload", async ({ page }) => {
    await page.goto("/settings/appearance");
    await expect(page.getByRole("heading", { name: "Appearance" })).toBeVisible();

    const lightRadio = page.getByTestId("appearance-theme-light");
    const darkRadio = page.getByTestId("appearance-theme-dark");
    await expect(lightRadio).toBeVisible();
    await expect(darkRadio).toBeVisible();

    // Headless Chromium prefers light, so the demo boots into the light theme.
    await expect(html(page)).toHaveAttribute("data-theme", "light");
    await expect(lightRadio).toBeChecked();

    // Light → dark.
    await darkRadio.check();
    await expect(html(page)).toHaveAttribute("data-theme", "dark");
    await expect(darkRadio).toBeChecked();
    expect(await storedTheme(page)).toBe("dark");

    // Persists across a full reload.
    await page.reload();
    await expect(page.getByRole("heading", { name: "Appearance" })).toBeVisible();
    await expect(html(page)).toHaveAttribute("data-theme", "dark");
    await expect(page.getByTestId("appearance-theme-dark")).toBeChecked();
    expect(await storedTheme(page)).toBe("dark");

    // Dark → light.
    await page.getByTestId("appearance-theme-light").check();
    await expect(html(page)).toHaveAttribute("data-theme", "light");
    expect(await storedTheme(page)).toBe("light");

    // The light selection persists too.
    await page.reload();
    await expect(page.getByRole("heading", { name: "Appearance" })).toBeVisible();
    await expect(html(page)).toHaveAttribute("data-theme", "light");
    await expect(page.getByTestId("appearance-theme-light")).toBeChecked();
  });
});
