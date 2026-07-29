import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const STORY = "extras-navigationmenu--default";

test.describe("extras/navigation-menu", () => {
  test("renders triggers and a plain link", async ({ page }) => {
    await gotoStory(page, STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const product = page.getByRole("button", { name: "Product" });
    await expect(product).toBeVisible();
    await expect(product).toHaveAttribute("aria-expanded", "false");
    await expect(product).toHaveAttribute("data-state", "closed");
    await expect(page.getByRole("button", { name: "Resources" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Docs" })).toBeVisible();
  });

  test("Enter opens the panel and Escape closes it", async ({ page }) => {
    await gotoStory(page, STORY);
    const product = page.getByRole("button", { name: "Product" });
    await page.keyboard.press("Tab");
    await expect(product).toBeFocused();

    await page.keyboard.press("Enter");
    await expect(product).toHaveAttribute("data-state", "open");
    await expect(product).toHaveAttribute("aria-expanded", "true");
    await expect(page.getByRole("link", { name: /Agents/ })).toBeVisible();
    await expect(page.getByRole("link", { name: /Tools/ })).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(product).toHaveAttribute("data-state", "closed");
    await expect(page.getByRole("link", { name: /Agents/ })).toHaveCount(0);
    await expect(product).toBeFocused();
  });

  test("hover opens a panel and moving away closes it", async ({ page }) => {
    await gotoStory(page, STORY);
    const resources = page.getByRole("button", { name: "Resources" });
    await resources.hover();
    await expect(resources).toHaveAttribute("data-state", "open");
    await expect(page.getByRole("link", { name: /Guides/ })).toBeVisible();
    await expect(page.getByRole("link", { name: /Changelog/ })).toBeVisible();

    await page.mouse.move(10, 400);
    await expect(resources).toHaveAttribute("data-state", "closed");
    await expect(page.getByRole("link", { name: /Guides/ })).toHaveCount(0);
  });

  test("keyboard focus shows a focus ring on the trigger", async ({
    page,
  }) => {
    await gotoStory(page, STORY);
    const product = page.getByRole("button", { name: "Product" });
    await expect(product).toHaveCSS("box-shadow", "none");
    await page.keyboard.press("Tab");
    await expect(product).toBeFocused();
    await expect(product).not.toHaveCSS("box-shadow", "none");
  });

  test("hovering a trigger changes its background color", async ({ page }) => {
    await gotoStory(page, STORY);
    const product = page.getByRole("button", { name: "Product" });
    const before = await computedStyle(product, "background-color");
    await product.hover();
    await expect(product).not.toHaveCSS("background-color", before);
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("button", { name: "Product" })).toBeVisible();
  });
});
