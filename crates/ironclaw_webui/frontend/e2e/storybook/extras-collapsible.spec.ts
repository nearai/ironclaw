/**
 * Extras/Collapsible — open/close via mouse and keyboard (data-state flips),
 * hover state on the ghost-button trigger, focus-visible ring, theming.
 */
import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const DEFAULT = "extras-collapsible--default";

test.describe("extras collapsible", () => {
  test("renders closed with hidden content (dark theme)", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const trigger = page.getByRole("button", { name: "Show" });
    await expect(trigger).toBeVisible();
    await expect(trigger).toHaveAttribute("data-state", "closed");
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await expect(page.getByText("run-4821")).toBeHidden();
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("button", { name: "Show" })).toBeVisible();
  });

  test("click toggles content and data-state", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    await page.getByRole("button", { name: "Show" }).click();
    const trigger = page.getByRole("button", { name: "Hide" });
    await expect(trigger).toHaveAttribute("data-state", "open");
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    await expect(page.getByText("run-4821")).toBeVisible();
    await expect(page.getByText("run-4819")).toBeVisible();
    await trigger.click();
    await expect(page.getByRole("button", { name: "Show" })).toHaveAttribute(
      "data-state",
      "closed"
    );
    await expect(page.getByText("run-4821")).toBeHidden();
  });

  test("keyboard: Tab to trigger, Enter opens, Space closes", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    await page.keyboard.press("Tab");
    await expect(page.getByRole("button", { name: "Show" })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("button", { name: "Hide" })).toHaveAttribute(
      "data-state",
      "open"
    );
    await expect(page.getByText("run-4820")).toBeVisible();
    await page.keyboard.press("Space");
    await expect(page.getByRole("button", { name: "Show" })).toHaveAttribute(
      "data-state",
      "closed"
    );
    await expect(page.getByText("run-4820")).toBeHidden();
  });

  test("hover changes the ghost trigger background", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "Show" });
    const before = await computedStyle(trigger, "background-color");
    await trigger.hover();
    await expect(trigger).not.toHaveCSS("background-color", before);
  });

  test("focus-visible: keyboard focus shows a ring (box-shadow)", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "Show" });
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();
    await expect(trigger).not.toHaveCSS("box-shadow", "none");
  });
});
