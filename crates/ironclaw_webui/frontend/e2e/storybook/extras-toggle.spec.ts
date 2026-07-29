import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const DEFAULT_STORY = "extras-toggle--default";
const SIZES_STORY = "extras-toggle--sizes";

test.describe("extras/toggle", () => {
  test("renders unpressed and toggles on click with distinct styling", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const toggle = page.getByRole("button", { name: "Pin thread" });
    await expect(toggle).toBeVisible();
    await expect(toggle).toHaveAttribute("aria-pressed", "false");
    await expect(toggle).toHaveAttribute("data-state", "off");
    await expect(toggle).toHaveText(/Pin$/);
    const offBackground = await computedStyle(toggle, "background-color");

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-pressed", "true");
    await expect(toggle).toHaveAttribute("data-state", "on");
    await expect(toggle).toHaveText(/Pinned$/);
    await expect(toggle).not.toHaveCSS("background-color", offBackground);
  });

  test("space and enter toggle pressed state from the keyboard", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT_STORY);
    const toggle = page.getByRole("button", { name: "Pin thread" });
    await page.keyboard.press("Tab");
    await expect(toggle).toBeFocused();

    await page.keyboard.press("Space");
    await expect(toggle).toHaveAttribute("data-state", "on");
    await page.keyboard.press("Enter");
    await expect(toggle).toHaveAttribute("data-state", "off");
  });

  test("keyboard focus shows a focus ring", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const toggle = page.getByRole("button", { name: "Pin thread" });
    await expect(toggle).toHaveCSS("box-shadow", "none");
    await page.keyboard.press("Tab");
    await expect(toggle).toBeFocused();
    await expect(toggle).not.toHaveCSS("box-shadow", "none");
  });

  test("hover changes the background color", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const toggle = page.getByRole("button", { name: "Pin thread" });
    const before = await computedStyle(toggle, "background-color");
    await toggle.hover();
    await expect(toggle).not.toHaveCSS("background-color", before);
  });

  test("sizes story renders disabled and pressed variants", async ({
    page,
  }) => {
    await gotoStory(page, SIZES_STORY);
    const disabled = page.getByRole("button", {
      name: "Disabled",
      exact: true,
    });
    await expect(disabled).toBeDisabled();
    await expect(disabled).toHaveCSS("opacity", "0.5");
    await expect(disabled).toHaveAttribute("data-state", "off");

    const pressed = page.getByRole("button", { name: "Pressed", exact: true });
    await expect(pressed).toHaveAttribute("data-state", "on");
    await expect(pressed).toHaveAttribute("aria-pressed", "true");

    for (const name of ["Small", "Medium", "Large"]) {
      await expect(
        page.getByRole("button", { name, exact: true })
      ).toBeVisible();
    }
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("button", { name: "Pin thread" })).toBeVisible();
  });
});
