import { expect, test } from "@playwright/test";
import { gotoStory } from "./helpers";

const DEFAULT_STORY = "extras-tooltip--default";
const SIDES_STORY = "extras-tooltip--sides";

/** The visible tooltip bubble (Radix also renders a visually-hidden copy). */
function tooltipBubble(page: import("@playwright/test").Page, text: string) {
  return page.locator("[data-side][data-align]").filter({ hasText: text });
}

test.describe("extras/tooltip", () => {
  test("appears on hover and disappears when the pointer leaves", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const trigger = page.getByRole("button", { name: "Settings" });
    await expect(trigger).toBeVisible();

    await trigger.hover();
    const bubble = tooltipBubble(page, "Open settings");
    await expect(bubble).toBeVisible();
    await expect(page.getByRole("tooltip")).toBeAttached();

    // Radix tracks pointer movement, so leave with stepped moves rather
    // than a single teleporting jump.
    await page.mouse.move(0, 0, { steps: 10 });
    await expect(bubble).toHaveCount(0);
  });

  test("appears on keyboard focus and closes on Escape", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const trigger = page.getByRole("button", { name: "Settings" });
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();

    const bubble = tooltipBubble(page, "Open settings");
    await expect(bubble).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(bubble).toHaveCount(0);
    // Trigger keeps focus after dismissal.
    await expect(trigger).toBeFocused();
  });

  test("sides story places the tooltip on the requested side", async ({
    page,
  }) => {
    await gotoStory(page, SIDES_STORY);
    await page.getByRole("button", { name: "top", exact: true }).hover();
    const top = tooltipBubble(page, "Tooltip on top");
    await expect(top).toBeVisible();
    await expect(top).toHaveAttribute("data-side", "top");

    // Close the first tooltip with a stepped pointer exit before opening
    // the next one (Radix needs real pointer movement to dismiss).
    await page.mouse.move(0, 0, { steps: 10 });
    await expect(top).toHaveCount(0);

    await page.getByRole("button", { name: "right", exact: true }).hover();
    const right = tooltipBubble(page, "Tooltip on right");
    await expect(right).toBeVisible();
    await expect(right).toHaveAttribute("data-side", "right");
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await page.getByRole("button", { name: "Settings" }).hover();
    await expect(tooltipBubble(page, "Open settings")).toBeVisible();
  });
});
