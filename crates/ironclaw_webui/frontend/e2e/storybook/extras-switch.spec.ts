import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const DEFAULT_STORY = "extras-switch--default";
const STATES_STORY = "extras-switch--states";

test.describe("extras/switch", () => {
  test("renders checked by default and toggles on click", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const control = page.getByRole("switch", { name: "Run notifications" });
    await expect(control).toBeVisible();
    await expect(control).toHaveAttribute("aria-checked", "true");
    await expect(control).toHaveAttribute("data-state", "checked");

    await control.click();
    await expect(control).toHaveAttribute("aria-checked", "false");
    await expect(control).toHaveAttribute("data-state", "unchecked");
  });

  test("space toggles the switch from the keyboard", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const control = page.getByRole("switch", { name: "Run notifications" });
    await page.keyboard.press("Tab");
    await expect(control).toBeFocused();

    await page.keyboard.press("Space");
    await expect(control).toHaveAttribute("data-state", "unchecked");
    await page.keyboard.press("Space");
    await expect(control).toHaveAttribute("data-state", "checked");
  });

  test("keyboard focus shows a focus ring", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const control = page.getByRole("switch", { name: "Run notifications" });
    await expect(control).toHaveCSS("box-shadow", "none");
    await page.keyboard.press("Tab");
    await expect(control).toBeFocused();
    await expect(control).not.toHaveCSS("box-shadow", "none");
  });

  test("states story renders off/on and disabled variants", async ({
    page,
  }) => {
    await gotoStory(page, STATES_STORY);
    const off = page.getByRole("switch", { name: "Off", exact: true });
    const on = page.getByRole("switch", { name: "On", exact: true });
    const disabledOff = page.getByRole("switch", {
      name: "Disabled off",
      exact: true,
    });
    const disabledOn = page.getByRole("switch", {
      name: "Disabled on",
      exact: true,
    });

    await expect(off).toHaveAttribute("data-state", "unchecked");
    await expect(on).toHaveAttribute("data-state", "checked");
    await expect(disabledOff).toBeDisabled();
    await expect(disabledOff).toHaveCSS("opacity", "0.5");
    await expect(disabledOn).toBeDisabled();
    await expect(disabledOn).toHaveAttribute("data-state", "checked");
    await expect(disabledOn).toHaveCSS("opacity", "0.5");
  });

  test("hovering an unchecked switch changes its border color", async ({
    page,
  }) => {
    await gotoStory(page, STATES_STORY);
    const off = page.getByRole("switch", { name: "Off", exact: true });
    const before = await computedStyle(off, "border-color");
    await off.hover();
    await expect(off).not.toHaveCSS("border-color", before);
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(
      page.getByRole("switch", { name: "Run notifications" })
    ).toBeVisible();
  });
});
