/**
 * Extras/Menubar — open menus via click and keyboard, ArrowRight walks
 * across top-level menus, disabled item skipped by arrow nav, checkbox and
 * radio items expose aria-checked, Export submenu, Escape dismissal,
 * hover/focus-visible states, theming.
 */
import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const DEFAULT = "extras-menubar--default";

test.describe("extras menubar", () => {
  test("renders the three top-level triggers closed (dark theme)", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    for (const name of ["File", "View", "Help"]) {
      const trigger = page.getByRole("menuitem", { name });
      await expect(trigger).toBeVisible();
      await expect(trigger).toHaveAttribute("data-state", "closed");
    }
    await expect(page.getByRole("menu")).toHaveCount(0);
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("menuitem", { name: "File" })).toBeVisible();
  });

  test("click opens a menu; Escape closes it", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    const file = page.getByRole("menuitem", { name: "File" });
    await file.click();
    await expect(file).toHaveAttribute("data-state", "open");
    await expect(file).toHaveAttribute("aria-expanded", "true");
    await expect(page.getByRole("menuitem", { name: /New run/ })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(file).toHaveAttribute("data-state", "closed");
    await expect(page.getByRole("menuitem", { name: /New run/ })).toBeHidden();
  });

  test("ArrowRight moves across open top-level menus", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    const file = page.getByRole("menuitem", { name: "File" });
    const view = page.getByRole("menuitem", { name: "View" });
    const help = page.getByRole("menuitem", { name: "Help" });
    await file.click();
    await expect(file).toHaveAttribute("data-state", "open");
    await page.keyboard.press("ArrowRight");
    await expect(view).toHaveAttribute("data-state", "open");
    await expect(file).toHaveAttribute("data-state", "closed");
    await page.keyboard.press("ArrowRight");
    await expect(help).toHaveAttribute("data-state", "open");
    await expect(
      page.getByRole("menuitem", { name: "Documentation" })
    ).toBeVisible();
  });

  test("keyboard: Enter opens the focused trigger; disabled item is skipped", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const file = page.getByRole("menuitem", { name: "File" });
    await page.keyboard.press("Tab");
    await expect(file).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(file).toHaveAttribute("data-state", "open");
    const revert = page.getByRole("menuitem", { name: "Revert (no changes)" });
    await expect(revert).toHaveAttribute("data-disabled", "");
    await expect(revert).toHaveAttribute("aria-disabled", "true");
    await expect(revert).toHaveCSS("opacity", "0.5");
    // Keyboard-open already highlights the first item.
    await expect(
      page.getByRole("menuitem", { name: /New run/ })
    ).toHaveAttribute("data-highlighted", "");
    await page.keyboard.press("ArrowDown");
    await expect(page.getByRole("menuitem", { name: "Open…" })).toHaveAttribute(
      "data-highlighted",
      ""
    );
    // Disabled Revert is skipped: highlight lands on the Export sub-trigger.
    await page.keyboard.press("ArrowDown");
    await expect(
      page.getByRole("menuitem", { name: "Export" })
    ).toHaveAttribute("data-highlighted", "");
  });

  test("Export submenu opens with ArrowRight", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    await page.getByRole("menuitem", { name: "File" }).click();
    const exportTrigger = page.getByRole("menuitem", { name: "Export" });
    // Assert after every press so key events can't race the menu mount.
    await page.keyboard.press("ArrowDown");
    await expect(
      page.getByRole("menuitem", { name: /New run/ })
    ).toHaveAttribute("data-highlighted", "");
    await page.keyboard.press("ArrowDown");
    await expect(page.getByRole("menuitem", { name: "Open…" })).toHaveAttribute(
      "data-highlighted",
      ""
    );
    await page.keyboard.press("ArrowDown");
    await expect(exportTrigger).toHaveAttribute("data-highlighted", "");
    await page.keyboard.press("ArrowRight");
    await expect(exportTrigger).toHaveAttribute("data-state", "open");
    await expect(page.getByRole("menuitem", { name: "JSON" })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "CSV" })).toBeVisible();
  });

  test("View menu exposes checkbox and radio states", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    const view = page.getByRole("menuitem", { name: "View" });
    await view.click();
    const wordWrap = page.getByRole("menuitemcheckbox", { name: "Word wrap" });
    await expect(wordWrap).toHaveAttribute("aria-checked", "true");
    await expect(wordWrap).toHaveAttribute("data-state", "checked");
    await expect(
      page.getByRole("menuitemradio", { name: "Dark" })
    ).toHaveAttribute("aria-checked", "true");
    const light = page.getByRole("menuitemradio", { name: "Light" });
    await expect(light).toHaveAttribute("aria-checked", "false");
    await light.click();
    await expect(page.getByRole("menu")).toHaveCount(0);
    await view.click();
    await expect(
      page.getByRole("menuitemradio", { name: "Light" })
    ).toHaveAttribute("aria-checked", "true");
  });

  test("hover changes the trigger text color", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    const file = page.getByRole("menuitem", { name: "File" });
    const before = await computedStyle(file, "color");
    await file.hover();
    await expect(file).not.toHaveCSS("color", before);
  });

  test("focus-visible: keyboard focus rings the trigger", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    const file = page.getByRole("menuitem", { name: "File" });
    await page.keyboard.press("Tab");
    await expect(file).toBeFocused();
    await expect(file).not.toHaveCSS("box-shadow", "none");
  });
});
