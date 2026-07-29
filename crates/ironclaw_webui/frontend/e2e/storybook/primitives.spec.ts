import { expect, test } from "@playwright/test";
import { gotoStory } from "./helpers";

test.describe("Primitives", () => {
  test("icon gallery renders labelled glyph cells (dark)", async ({ page }) => {
    await gotoStory(page, "primitives-overview--icons");
    for (const name of ["plus", "bell", "close", "check"]) {
      await expect(page.getByText(name, { exact: true })).toBeVisible();
    }
    // Every cell pairs an svg with its mono label.
    const cells = page.locator("#storybook-root .grid > div");
    expect(await cells.count()).toBeGreaterThan(10);
    await expect(cells.first().locator("svg")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("icon gallery renders in light theme", async ({ page }) => {
    await gotoStory(page, "primitives-overview--icons", { theme: "light" });
    await expect(page.getByText("plus", { exact: true })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("spinner story renders three accessible loading indicators", async ({ page }) => {
    await gotoStory(page, "primitives-overview--spinner-story");
    const spinners = page.getByRole("status", { name: "Loading" });
    await expect(spinners).toHaveCount(3);
    for (let index = 0; index < 3; index += 1) {
      await expect(spinners.nth(index)).toBeVisible();
    }
  });

  test("skeleton story renders three placeholder blocks", async ({ page }) => {
    await gotoStory(page, "primitives-overview--skeleton-story");
    const skeletons = page.locator('#storybook-root [aria-hidden="true"]');
    await expect(skeletons).toHaveCount(3);
    // Gradient background resolves to a non-transparent surface.
    await expect(skeletons.first()).not.toHaveCSS("background-image", "none");
    await expect(skeletons.first()).toHaveCSS("border-radius", "6px");
  });
});
