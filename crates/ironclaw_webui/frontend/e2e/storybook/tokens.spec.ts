import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const EXPECTED_TOKENS = [
  "--v2-canvas",
  "--v2-surface",
  "--v2-text",
  "--v2-accent",
  "--v2-positive-text",
  "--v2-warning-text",
  "--v2-danger-text",
  "--v2-info-text",
  "--v2-focus-ring",
];

test.describe("Tokens", () => {
  test("color swatch grid renders every token name (dark)", async ({ page }) => {
    await gotoStory(page, "tokens-overview--colors");
    for (const token of EXPECTED_TOKENS) {
      await expect(page.getByText(token, { exact: true })).toBeVisible();
    }
    // 25 swatch cells, each with one mono token label.
    await expect(page.locator("#storybook-root span")).toHaveCount(25);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("canvas swatch resolves to different colors per theme", async ({ page }) => {
    await gotoStory(page, "tokens-overview--colors");
    const canvasSwatch = page
      .locator("#storybook-root > div > div")
      .filter({ has: page.getByText("--v2-canvas", { exact: true }) })
      .locator("> div")
      .first();
    const darkColor = await computedStyle(canvasSwatch, "background-color");
    expect(darkColor).not.toBe("rgba(0, 0, 0, 0)");

    await gotoStory(page, "tokens-overview--colors", { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    const lightSwatch = page
      .locator("#storybook-root > div > div")
      .filter({ has: page.getByText("--v2-canvas", { exact: true }) })
      .locator("> div")
      .first();
    await expect(lightSwatch).not.toHaveCSS("background-color", darkColor);
  });

  test("type scale story renders each step at its token size", async ({ page }) => {
    await gotoStory(page, "tokens-overview--type-scale");
    const samples = page.getByText("Shared control typography");
    await expect(samples).toHaveCount(3);
    await expect(page.getByText("--text-ui-sm · 0.75rem")).toBeVisible();
    await expect(page.getByText("--text-ui · 0.8125rem")).toBeVisible();
    await expect(page.getByText("--text-ui-lg · 1rem")).toBeVisible();
    // 0.75rem / 0.8125rem / 1rem → 12px / 13px / 16px.
    await expect(samples.nth(0)).toHaveCSS("font-size", "12px");
    await expect(samples.nth(1)).toHaveCSS("font-size", "13px");
    await expect(samples.nth(2)).toHaveCSS("font-size", "16px");
  });

  test("type scale renders in light theme", async ({ page }) => {
    await gotoStory(page, "tokens-overview--type-scale", { theme: "light" });
    await expect(page.getByText("Shared control typography").first()).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });
});
