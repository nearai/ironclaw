/**
 * Extras/Avatar — fallback rendering (including when the remote image fails),
 * size scale, circular shape, and theming.
 */
import { expect, test } from "@playwright/test";
import { gotoStory } from "./helpers";

const WITH_IMAGE = "extras-avatar--with-image";
const FALLBACK_ONLY = "extras-avatar--fallback-only";
const SIZES = "extras-avatar--sizes";

test.describe("extras avatar", () => {
  test("fallback initials show when the image request fails", async ({
    page,
  }) => {
    // Kill the remote avatar request so the Radix fallback path is exercised
    // deterministically (no dependence on external network).
    await page.route("**/avatars.githubusercontent.com/**", (route) =>
      route.abort()
    );
    await gotoStory(page, WITH_IMAGE);
    await expect(page.getByText("GH")).toBeVisible();
  });

  test("fallback-only avatar renders initials in a circle (dark theme)", async ({
    page,
  }) => {
    await gotoStory(page, FALLBACK_ONLY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const fallback = page.getByText("AL", { exact: true });
    await expect(fallback).toBeVisible();
    // rounded-full (calc(infinity*1px) in Tailwind v4) → fully circular.
    await expect(fallback).toHaveCSS("border-radius", "3.35544e+07px");
  });

  test("size variants follow the control scale (28 / 36 / 48px)", async ({
    page,
  }) => {
    await gotoStory(page, SIZES);
    for (const [initial, px] of [
      ["S", "28px"],
      ["M", "36px"],
      ["L", "48px"],
    ] as const) {
      const fallback = page.getByText(initial, { exact: true });
      await expect(fallback).toBeVisible();
      // Measure the Avatar root (the fallback's parent), which owns h-*/w-*.
      const root = fallback.locator("..");
      await expect(root).toHaveCSS("height", px);
      await expect(root).toHaveCSS("width", px);
    }
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, FALLBACK_ONLY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByText("AL", { exact: true })).toBeVisible();
  });
});
