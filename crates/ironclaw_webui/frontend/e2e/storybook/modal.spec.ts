import { expect, test } from "@playwright/test";
import { gotoStory } from "./helpers";

test.describe("Modal", () => {
  test("opens via click, locks body scroll, closes via Escape (dark)", async ({ page }) => {
    await gotoStory(page, "components-modal--default");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeHidden();

    await page.getByRole("button", { name: "Open modal" }).click();
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAttribute("aria-modal", "true");
    await expect(page.getByRole("heading", { name: "Configure extension" })).toBeVisible();
    await expect(page.locator("body")).toHaveCSS("overflow", "hidden");

    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(page.locator("body")).not.toHaveCSS("overflow", "hidden");
  });

  test("renders in light theme", async ({ page }) => {
    await gotoStory(page, "components-modal--default", { theme: "light" });
    await page.getByRole("button", { name: "Open modal" }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("opens with keyboard and Tab moves focus into the dialog", async ({ page }) => {
    await gotoStory(page, "components-modal--default");
    const trigger = page.getByRole("button", { name: "Open modal" });
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();
    await page.keyboard.press("Enter");
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    // Next tab stop is the dialog's close button.
    await page.keyboard.press("Tab");
    await expect(dialog.getByRole("button", { name: "Close" })).toBeFocused();
  });

  test("closes via the close button", async ({ page }) => {
    await gotoStory(page, "components-modal--default");
    await page.getByRole("button", { name: "Open modal" }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("button", { name: "Close" }).click();
    await expect(dialog).toBeHidden();
  });

  test("closes via backdrop click", async ({ page }) => {
    await gotoStory(page, "components-modal--default");
    await page.getByRole("button", { name: "Open modal" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    // The dim layer covers the viewport corners; the panel sits centered.
    await page.locator('[class*="bg-black"]').click({ position: { x: 10, y: 10 } });
    await expect(dialog).toBeHidden();
  });

  test("closes via the footer buttons", async ({ page }) => {
    await gotoStory(page, "components-modal--default");
    await page.getByRole("button", { name: "Open modal" }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();

    await page.getByRole("button", { name: "Open modal" }).click();
    await dialog.getByRole("button", { name: "Save" }).click();
    await expect(dialog).toBeHidden();
  });

  test("large story renders the wider panel", async ({ page }) => {
    await gotoStory(page, "components-modal--large");
    await page.getByRole("button", { name: "Open modal" }).click();
    const panel = page.getByRole("dialog").locator("> div").nth(1);
    // max-w-2xl
    await expect(panel).toHaveCSS("max-width", "672px");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toBeHidden();
  });
});
