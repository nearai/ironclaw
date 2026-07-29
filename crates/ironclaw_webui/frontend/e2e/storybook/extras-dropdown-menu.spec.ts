/**
 * Extras/DropdownMenu — open via click and keyboard (aria-expanded /
 * data-state), arrow-key highlight nav skipping the disabled item, checkbox
 * item aria-checked toggling, radio submenu via ArrowRight, Escape
 * dismissal, hover highlight, focus-visible ring, theming.
 */
import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const DEFAULT = "extras-dropdownmenu--default";

test.describe("extras dropdown-menu", () => {
  test("click opens and closes the menu (dark theme)", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    // CSS locator: the modal menu aria-hides the rest of the page while
    // open, which would break a role-based trigger lookup.
    const trigger = page.locator("button", { hasText: "Options" });
    await expect(trigger).toHaveAttribute("aria-haspopup", "menu");
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await expect(trigger).toHaveAttribute("data-state", "closed");
    await trigger.click();
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    await expect(trigger).toHaveAttribute("data-state", "open");
    const menu = page.getByRole("menu");
    await expect(menu).toBeVisible();
    await expect(page.getByText("Workspace", { exact: true })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: /Rename/ })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(menu).toBeHidden();
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("button", { name: "Options" })).toBeVisible();
  });

  test("keyboard: Enter on the trigger opens with the first item highlighted", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "Options" });
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("menu")).toBeVisible();
    await expect(
      page.getByRole("menuitem", { name: /Rename/ })
    ).toHaveAttribute("data-highlighted", "");
  });

  test("arrow navigation skips the disabled item", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    await page.getByRole("button", { name: "Options" }).click();
    const duplicate = page.getByRole("menuitem", { name: "Duplicate" });
    await expect(duplicate).toHaveAttribute("data-disabled", "");
    await expect(duplicate).toHaveAttribute("aria-disabled", "true");
    await expect(duplicate).toHaveCSS("opacity", "0.5");
    await page.keyboard.press("ArrowDown");
    await expect(
      page.getByRole("menuitem", { name: /Rename/ })
    ).toHaveAttribute("data-highlighted", "");
    // Duplicate (disabled) is skipped: highlight jumps to Notifications.
    await page.keyboard.press("ArrowDown");
    await expect(
      page.getByRole("menuitemcheckbox", { name: "Notifications" })
    ).toHaveAttribute("data-highlighted", "");
  });

  test("checkbox item toggles aria-checked across open cycles", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "Options" });
    await trigger.click();
    const notifications = page.getByRole("menuitemcheckbox", {
      name: "Notifications",
    });
    await expect(notifications).toHaveAttribute("aria-checked", "true");
    await expect(notifications).toHaveAttribute("data-state", "checked");
    await notifications.click();
    await expect(page.getByRole("menu")).toBeHidden();
    await trigger.click();
    await expect(
      page.getByRole("menuitemcheckbox", { name: "Notifications" })
    ).toHaveAttribute("aria-checked", "false");
  });

  test("ArrowRight opens the radio submenu and selection updates", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "Options" });
    await trigger.click();
    const subTrigger = page.getByRole("menuitem", { name: "Density" });
    await page.keyboard.press("ArrowDown");
    await expect(
      page.getByRole("menuitem", { name: /Rename/ })
    ).toHaveAttribute("data-highlighted", "");
    await page.keyboard.press("ArrowDown");
    await expect(
      page.getByRole("menuitemcheckbox", { name: "Notifications" })
    ).toHaveAttribute("data-highlighted", "");
    await page.keyboard.press("ArrowDown");
    await expect(subTrigger).toHaveAttribute("data-highlighted", "");
    await page.keyboard.press("ArrowRight");
    await expect(subTrigger).toHaveAttribute("data-state", "open");
    const comfortable = page.getByRole("menuitemradio", {
      name: "Comfortable",
    });
    await expect(comfortable).toHaveAttribute("aria-checked", "true");
    const compact = page.getByRole("menuitemradio", { name: "Compact" });
    await expect(compact).toHaveAttribute("aria-checked", "false");
    await compact.click();
    await expect(page.getByRole("menu")).toBeHidden();
    // Reopen and re-enter the submenu: the radio selection persisted.
    await trigger.click();
    await page.getByRole("menuitem", { name: "Density" }).hover();
    await expect(
      page.getByRole("menuitemradio", { name: "Compact" })
    ).toHaveAttribute("aria-checked", "true");
  });

  test("hovering an item highlights it and changes its background", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    await page.getByRole("button", { name: "Options" }).click();
    const rename = page.getByRole("menuitem", { name: /Rename/ });
    const before = await computedStyle(rename, "background-color");
    await rename.hover();
    await expect(rename).toHaveAttribute("data-highlighted", "");
    await expect(rename).not.toHaveCSS("background-color", before);
  });

  test("focus-visible: keyboard focus rings the trigger button", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "Options" });
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();
    await expect(trigger).not.toHaveCSS("box-shadow", "none");
  });
});
