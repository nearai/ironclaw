/**
 * Extras/Accordion — rendering, keyboard toggling, single/multiple modes,
 * disabled item, hover/focus-visible interaction states, theming.
 */
import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const SINGLE = "extras-accordion--single";
const MULTIPLE = "extras-accordion--multiple";

test.describe("extras accordion", () => {
  test("renders triggers closed with dark theme applied", async ({ page }) => {
    await gotoStory(page, SINGLE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const first = page.getByRole("button", { name: "What are the v2 tokens?" });
    await expect(first).toBeVisible();
    await expect(first).toHaveAttribute("data-state", "closed");
    await expect(first).toHaveAttribute("aria-expanded", "false");
    await expect(page.getByText("CSS custom properties")).toBeHidden();
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, SINGLE, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(
      page.getByRole("button", { name: "What are the v2 tokens?" })
    ).toBeVisible();
  });

  test("keyboard: Tab reaches trigger, Enter and Space toggle data-state", async ({
    page,
  }) => {
    await gotoStory(page, SINGLE);
    const trigger = page.getByRole("button", {
      name: "What are the v2 tokens?",
    });
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();

    await page.keyboard.press("Enter");
    await expect(trigger).toHaveAttribute("data-state", "open");
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    await expect(page.getByText("CSS custom properties")).toBeVisible();

    await page.keyboard.press("Space");
    await expect(trigger).toHaveAttribute("data-state", "closed");
    await expect(page.getByText("CSS custom properties")).toBeHidden();
  });

  test("single mode: opening one section closes the other", async ({
    page,
  }) => {
    await gotoStory(page, SINGLE);
    const first = page.getByRole("button", { name: "What are the v2 tokens?" });
    const second = page.getByRole("button", { name: "Is it animated?" });
    await first.click();
    await expect(first).toHaveAttribute("data-state", "open");
    await second.click();
    await expect(second).toHaveAttribute("data-state", "open");
    await expect(first).toHaveAttribute("data-state", "closed");
  });

  test("disabled item is disabled with reduced opacity and never opens", async ({
    page,
  }) => {
    await gotoStory(page, SINGLE);
    const disabled = page.getByRole("button", { name: "Disabled section" });
    await expect(disabled).toBeDisabled();
    await expect(disabled).toHaveAttribute("data-disabled", "");
    await expect(disabled).toHaveCSS("opacity", "0.5");
    await disabled.click({ force: true });
    await expect(disabled).toHaveAttribute("data-state", "closed");
    await expect(page.getByText("Never visible.")).toBeHidden();
  });

  test("multiple mode: both sections start open and collapse independently", async ({
    page,
  }) => {
    await gotoStory(page, MULTIPLE);
    const first = page.getByRole("button", { name: "First (open)" });
    const second = page.getByRole("button", { name: "Second (open)" });
    await expect(first).toHaveAttribute("data-state", "open");
    await expect(second).toHaveAttribute("data-state", "open");
    await first.click();
    await expect(first).toHaveAttribute("data-state", "closed");
    await expect(second).toHaveAttribute("data-state", "open");
  });

  test("hover changes the trigger text color", async ({ page }) => {
    await gotoStory(page, SINGLE);
    const trigger = page.getByRole("button", {
      name: "What are the v2 tokens?",
    });
    const before = await computedStyle(trigger, "color");
    await trigger.hover();
    await expect(trigger).not.toHaveCSS("color", before);
  });

  test("focus-visible: keyboard focus shows a ring (box-shadow)", async ({
    page,
  }) => {
    await gotoStory(page, SINGLE);
    const trigger = page.getByRole("button", {
      name: "What are the v2 tokens?",
    });
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();
    await expect(trigger).not.toHaveCSS("box-shadow", "none");
  });
});
