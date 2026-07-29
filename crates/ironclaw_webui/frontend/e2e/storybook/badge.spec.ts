import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

test.describe("Badge", () => {
  test("muted badge renders with its dot in dark theme", async ({ page }) => {
    await gotoStory(page, "components-badge--muted");
    const badge = page.getByText("Badge");
    await expect(badge).toBeVisible();
    // The tone dot is the only child span inside the chip.
    await expect(badge.locator("span")).toHaveCount(1);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("muted badge renders in light theme", async ({ page }) => {
    await gotoStory(page, "components-badge--muted", { theme: "light" });
    await expect(page.getByText("Badge")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("all-tones story renders each tone with distinct colors", async ({ page }) => {
    await gotoStory(page, "components-badge--all-tones");
    for (const tone of ["success", "warning", "danger", "info", "accent", "muted"]) {
      await expect(page.getByText(tone, { exact: true })).toBeVisible();
    }
    const successBackground = await computedStyle(
      page.getByText("success", { exact: true }),
      "background-color"
    );
    await expect(page.getByText("danger", { exact: true })).not.toHaveCSS(
      "background-color",
      successBackground
    );
  });

  test("small size renders at the compact 24px height", async ({ page }) => {
    await gotoStory(page, "components-badge--small");
    const badge = page.getByText("small");
    await expect(badge).toBeVisible();
    await expect(badge).toHaveCSS("height", "24px");
  });

  test("without-dot story omits the tone dot", async ({ page }) => {
    await gotoStory(page, "components-badge--without-dot");
    const badge = page.getByText("no dot");
    await expect(badge).toBeVisible();
    await expect(badge.locator("span")).toHaveCount(0);
  });
});
