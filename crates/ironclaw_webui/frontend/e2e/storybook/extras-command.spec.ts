/**
 * Extras/Command — inline palette filtering (including keyword matches and
 * the empty state), arrow-key highlight movement, hover highlight, the
 * dialog host open/Escape/backdrop behavior, and theming. Disabled items are
 * excluded from the registry entirely, so they never render.
 */
import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const INLINE = "extras-command--inline";
const DIALOG = "extras-command--dialog";

test.describe("extras command", () => {
  test("inline palette renders groups and items (dark theme)", async ({
    page,
  }) => {
    await gotoStory(page, INLINE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByRole("combobox")).toBeVisible();
    await expect(page.getByRole("listbox")).toBeVisible();
    await expect(page.getByText("Runs", { exact: true })).toBeVisible();
    await expect(page.getByText("Settings", { exact: true })).toBeVisible();
    await expect(page.getByRole("option", { name: /New run/ })).toBeVisible();
    // Disabled items are filtered out of the visible registry entirely.
    await expect(page.getByRole("option", { name: /Archive run/ })).toHaveCount(
      0
    );
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, INLINE, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("combobox")).toBeVisible();
  });

  test("typing filters items and hides empty groups", async ({ page }) => {
    await gotoStory(page, INLINE);
    const input = page.getByRole("combobox");
    await input.click();
    await page.keyboard.type("settings");
    await expect(
      page.getByRole("option", { name: /Open settings/ })
    ).toBeVisible();
    await expect(page.getByRole("option")).toHaveCount(1);
    await expect(page.getByText("Runs", { exact: true })).toBeHidden();
  });

  test("keyword search matches items whose label doesn't contain the query", async ({
    page,
  }) => {
    await gotoStory(page, INLINE);
    await page.getByRole("combobox").click();
    // "halt" is a keyword of "Pause all runs".
    await page.keyboard.type("halt");
    await expect(page.getByRole("option")).toHaveCount(1);
    await expect(
      page.getByRole("option", { name: /Pause all runs/ })
    ).toBeVisible();
  });

  test("no matches shows the empty message", async ({ page }) => {
    await gotoStory(page, INLINE);
    await page.getByRole("combobox").click();
    await page.keyboard.type("zzzz");
    await expect(page.getByText("No results found.")).toBeVisible();
    await expect(page.getByRole("option")).toHaveCount(0);
  });

  test("arrow keys move the active highlight (aria-selected)", async ({
    page,
  }) => {
    await gotoStory(page, INLINE);
    const first = page.getByRole("option", { name: /New run/ });
    const second = page.getByRole("option", { name: /Pause all runs/ });
    await expect(first).toHaveAttribute("aria-selected", "true");
    await page.getByRole("combobox").click();
    await page.keyboard.press("ArrowDown");
    await expect(second).toHaveAttribute("aria-selected", "true");
    await expect(first).toHaveAttribute("aria-selected", "false");
    await page.keyboard.press("ArrowUp");
    await expect(first).toHaveAttribute("aria-selected", "true");
  });

  test("hovering an item makes it active and changes its background", async ({
    page,
  }) => {
    await gotoStory(page, INLINE);
    const item = page.getByRole("option", { name: /Toggle theme/ });
    const before = await computedStyle(item, "background-color");
    await item.hover();
    await expect(item).toHaveAttribute("aria-selected", "true");
    await expect(item).not.toHaveCSS("background-color", before);
  });

  test("dialog story opens the palette and Escape closes it", async ({
    page,
  }) => {
    await gotoStory(page, DIALOG);
    const open = page.getByRole("button", { name: "Open command palette" });
    await open.click();
    const dialog = page.getByRole("dialog", { name: "Command menu" });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole("combobox")).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
  });

  test("dialog closes on backdrop click", async ({ page }) => {
    await gotoStory(page, DIALOG);
    await page.getByRole("button", { name: "Open command palette" }).click();
    const dialog = page.getByRole("dialog", { name: "Command menu" });
    await expect(dialog).toBeVisible();
    // Click the far corner of the viewport — always backdrop territory.
    await page.mouse.click(5, 5);
    await expect(dialog).toBeHidden();
  });

  test("focus-visible: keyboard focus rings the dialog trigger button", async ({
    page,
  }) => {
    await gotoStory(page, DIALOG);
    const open = page.getByRole("button", { name: "Open command palette" });
    await page.keyboard.press("Tab");
    await expect(open).toBeFocused();
    await expect(open).not.toHaveCSS("box-shadow", "none");
  });
});
