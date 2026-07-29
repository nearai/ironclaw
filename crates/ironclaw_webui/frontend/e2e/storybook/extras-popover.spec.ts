import { expect, test } from "@playwright/test";
import { gotoStory } from "./helpers";

const DEFAULT_STORY = "extras-popover--default";
const ALIGNMENTS_STORY = "extras-popover--alignments";

test.describe("extras/popover", () => {
  test("opens on click, moves focus in, and closes on Escape", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const trigger = page.getByRole("button", { name: "Dimensions" });
    await expect(trigger).toHaveAttribute("aria-haspopup", "dialog");
    await expect(trigger).toHaveAttribute("data-state", "closed");

    await trigger.click();
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    await expect(trigger).toHaveAttribute("data-state", "open");
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText("Set dimensions");
    await expect(dialog.getByLabel("Width")).toHaveValue("320");

    // Focus moves inside the popover content.
    const focusInside = await dialog.evaluate((el) =>
      el.contains(document.activeElement)
    );
    expect(focusInside).toBe(true);

    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await expect(trigger).toBeFocused();
  });

  test("opens with Enter from the keyboard and focuses the first field", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT_STORY);
    const trigger = page.getByRole("button", { name: "Dimensions" });
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();

    await page.keyboard.press("Enter");
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    // Keyboard-opened popovers autofocus the first tabbable field.
    await expect(dialog.getByLabel("Width")).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(dialog.getByLabel("Height")).toBeFocused();
  });

  test("keyboard focus shows a focus ring on the trigger", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT_STORY);
    const trigger = page.getByRole("button", { name: "Dimensions" });
    await expect(trigger).toHaveCSS("box-shadow", "none");
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();
    await expect(trigger).not.toHaveCSS("box-shadow", "none");
  });

  test("alignments story opens content per trigger with matching align", async ({
    page,
  }) => {
    await gotoStory(page, ALIGNMENTS_STORY);
    await page.getByRole("button", { name: "start" }).click();
    const startDialog = page.getByRole("dialog");
    await expect(startDialog).toContainText("Aligned start.");
    await expect(startDialog).toHaveAttribute("data-align", "start");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);

    await page.getByRole("button", { name: "end" }).click();
    const endDialog = page.getByRole("dialog");
    await expect(endDialog).toContainText("Aligned end.");
    await expect(endDialog).toHaveAttribute("data-align", "end");
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("button", { name: "Dimensions" })).toBeVisible();
  });
});
