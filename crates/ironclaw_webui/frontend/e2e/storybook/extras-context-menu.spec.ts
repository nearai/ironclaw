/**
 * Extras/ContextMenu — right-click invocation, arrow-key navigation with
 * data-highlighted (skipping the disabled item), checkbox item toggling,
 * radio submenu via ArrowRight, Escape dismissal, hover highlight, theming.
 */
import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const DEFAULT = "extras-contextmenu--default";

test.describe("extras context-menu", () => {
  test("right-click opens the menu (dark theme)", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const trigger = page.getByText("Right-click here");
    await expect(trigger).toHaveAttribute("data-state", "closed");
    await trigger.click({ button: "right" });
    await expect(trigger).toHaveAttribute("data-state", "open");
    const menu = page.getByRole("menu");
    await expect(menu).toBeVisible();
    await expect(page.getByText("Run actions")).toBeVisible();
    await expect(
      page.getByRole("menuitem", { name: /Rename/ })
    ).toBeVisible();
    await expect(
      page.getByRole("menuitem", { name: "Delete run" })
    ).toBeVisible();
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await page.getByText("Right-click here").click({ button: "right" });
    await expect(page.getByRole("menu")).toBeVisible();
  });

  test("arrow navigation moves data-highlighted and skips the disabled item", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    await page.getByText("Right-click here").click({ button: "right" });
    const rename = page.getByRole("menuitem", { name: /Rename/ });
    const duplicate = page.getByRole("menuitem", { name: "Duplicate" });
    const pinned = page.getByRole("menuitemcheckbox", { name: "Pinned" });

    await expect(duplicate).toHaveAttribute("data-disabled", "");
    await expect(duplicate).toHaveAttribute("aria-disabled", "true");
    await expect(duplicate).toHaveCSS("opacity", "0.5");

    await page.keyboard.press("ArrowDown");
    await expect(rename).toHaveAttribute("data-highlighted", "");
    // Next ArrowDown skips the disabled Duplicate and lands on Pinned.
    await page.keyboard.press("ArrowDown");
    await expect(pinned).toHaveAttribute("data-highlighted", "");
    await expect(duplicate).not.toHaveAttribute("data-highlighted", "");
  });

  test("Enter toggles the checkbox item and closes; state persists on reopen", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByText("Right-click here");
    await trigger.click({ button: "right" });
    const pinned = page.getByRole("menuitemcheckbox", { name: "Pinned" });
    await expect(pinned).toHaveAttribute("aria-checked", "true");
    await expect(pinned).toHaveAttribute("data-state", "checked");
    await pinned.click();
    await expect(page.getByRole("menu")).toBeHidden();
    await trigger.click({ button: "right" });
    await expect(
      page.getByRole("menuitemcheckbox", { name: "Pinned" })
    ).toHaveAttribute("aria-checked", "false");
  });

  test("ArrowRight opens the radio submenu; radio selection is exposed", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    await page.getByText("Right-click here").click({ button: "right" });
    const subTrigger = page.getByRole("menuitem", { name: "Sort by" });
    // Walk the highlight down to the submenu trigger, asserting each hop so
    // the key presses can't race the menu mount.
    await page.keyboard.press("ArrowDown");
    await expect(
      page.getByRole("menuitem", { name: /Rename/ })
    ).toHaveAttribute("data-highlighted", "");
    await page.keyboard.press("ArrowDown");
    await expect(
      page.getByRole("menuitemcheckbox", { name: "Pinned" })
    ).toHaveAttribute("data-highlighted", "");
    await page.keyboard.press("ArrowDown");
    await expect(subTrigger).toHaveAttribute("data-highlighted", "");
    await page.keyboard.press("ArrowRight");
    await expect(subTrigger).toHaveAttribute("data-state", "open");
    const recent = page.getByRole("menuitemradio", { name: "Most recent" });
    await expect(recent).toBeVisible();
    await expect(recent).toHaveAttribute("aria-checked", "true");
    await expect(
      page.getByRole("menuitemradio", { name: "Name" })
    ).toHaveAttribute("aria-checked", "false");
  });

  test("Escape closes the menu", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByText("Right-click here");
    await trigger.click({ button: "right" });
    await expect(page.getByRole("menu")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toBeHidden();
    await expect(trigger).toHaveAttribute("data-state", "closed");
  });

  test("hovering an item highlights it and changes its background", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    await page.getByText("Right-click here").click({ button: "right" });
    const rename = page.getByRole("menuitem", { name: /Rename/ });
    const before = await computedStyle(rename, "background-color");
    await rename.hover();
    await expect(rename).toHaveAttribute("data-highlighted", "");
    await expect(rename).not.toHaveCSS("background-color", before);
  });
});
