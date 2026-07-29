/**
 * Extras/HoverCard — card appears after the hover delay and disappears on
 * unhover (retrying visibility expects, no manual sleeps), keyboard focus
 * also opens it, data-state on the trigger, theming.
 */
import { expect, test } from "@playwright/test";
import { gotoStory } from "./helpers";

const DEFAULT = "extras-hovercard--default";

test.describe("extras hover-card", () => {
  test("renders the trigger with the card closed (dark theme)", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const trigger = page.getByRole("button", { name: "@ironclaw-agent" });
    await expect(trigger).toBeVisible();
    await expect(trigger).toHaveAttribute("data-state", "closed");
    await expect(page.getByText("IronClaw Agent")).toBeHidden();
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(
      page.getByRole("button", { name: "@ironclaw-agent" })
    ).toBeVisible();
  });

  test("card appears on hover (after the open delay) and hides on unhover", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "@ironclaw-agent" });
    await trigger.hover();
    // openDelay=200ms — the retrying expect polls until the card mounts.
    await expect(page.getByText("IronClaw Agent")).toBeVisible();
    await expect(page.getByText(/128 runs this week/)).toBeVisible();
    await expect(trigger).toHaveAttribute("data-state", "open");
    // Park the pointer far from trigger and card to start the close delay.
    await page.mouse.move(5, 5);
    await expect(page.getByText("IronClaw Agent")).toBeHidden();
    await expect(trigger).toHaveAttribute("data-state", "closed");
  });

  test("keyboard focus on the trigger also opens the card", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "@ironclaw-agent" });
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();
    await expect(page.getByText("IronClaw Agent")).toBeVisible();
    await expect(trigger).toHaveAttribute("data-state", "open");
  });
});
