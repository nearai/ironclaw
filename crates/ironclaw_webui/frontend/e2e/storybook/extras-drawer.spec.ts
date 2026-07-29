/**
 * Extras/Drawer — open via trigger for all three sides, dismissal via
 * Escape / close button / footer action / backdrop, edge anchoring,
 * hover state on the close button, focus-visible ring, theming.
 */
import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const RIGHT = "extras-drawer--right";
const LEFT = "extras-drawer--left";
const BOTTOM = "extras-drawer--bottom";

test.describe("extras drawer", () => {
  test("opens from the trigger and shows title and body (dark theme)", async ({
    page,
  }) => {
    await gotoStory(page, RIGHT);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByRole("button", { name: "Open right drawer" }).click();
    const dialog = page.getByRole("dialog", { name: "Run details" });
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAttribute("aria-modal", "true");
    await expect(dialog.getByText("Edge-anchored panel")).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Save" })).toBeVisible();
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, RIGHT, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await page.getByRole("button", { name: "Open right drawer" }).click();
    await expect(page.getByRole("dialog", { name: "Run details" })).toBeVisible();
  });

  test("Escape closes the drawer", async ({ page }) => {
    await gotoStory(page, RIGHT);
    await page.getByRole("button", { name: "Open right drawer" }).click();
    const dialog = page.getByRole("dialog", { name: "Run details" });
    await expect(dialog).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
  });

  test("close button and footer Cancel both dismiss", async ({ page }) => {
    await gotoStory(page, RIGHT);
    const open = page.getByRole("button", { name: "Open right drawer" });
    await open.click();
    const dialog = page.getByRole("dialog", { name: "Run details" });
    await dialog.getByRole("button", { name: "Close" }).click();
    await expect(dialog).toBeHidden();
    await open.click();
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();
  });

  test("backdrop click closes the drawer", async ({ page }) => {
    await gotoStory(page, RIGHT);
    await page.getByRole("button", { name: "Open right drawer" }).click();
    const dialog = page.getByRole("dialog", { name: "Run details" });
    await expect(dialog).toBeVisible();
    // Right-anchored panel → the top-left corner is backdrop.
    await page.mouse.click(5, 5);
    await expect(dialog).toBeHidden();
  });

  test("left and bottom drawers anchor to their edges", async ({ page }) => {
    await gotoStory(page, LEFT);
    await page.getByRole("button", { name: "Open left drawer" }).click();
    const leftDialog = page.getByRole("dialog", { name: "Run details" });
    await expect(leftDialog).toBeVisible();
    const leftBox = await leftDialog.boundingBox();
    expect(leftBox!.x).toBe(0);

    await gotoStory(page, BOTTOM);
    await page.getByRole("button", { name: "Open bottom drawer" }).click();
    const bottomDialog = page.getByRole("dialog", { name: "Run details" });
    await expect(bottomDialog).toBeVisible();
    const viewport = page.viewportSize()!;
    const bottomBox = await bottomDialog.boundingBox();
    expect(bottomBox!.y + bottomBox!.height).toBeCloseTo(viewport.height, 0);
    expect(bottomBox!.width).toBeCloseTo(viewport.width, 0);
  });

  test("hover changes the close button background", async ({ page }) => {
    await gotoStory(page, RIGHT);
    await page.getByRole("button", { name: "Open right drawer" }).click();
    const close = page.getByRole("button", { name: "Close" });
    const before = await computedStyle(close, "background-color");
    await close.hover();
    await expect(close).not.toHaveCSS("background-color", before);
  });

  test("focus-visible: keyboard focus rings the trigger button", async ({
    page,
  }) => {
    await gotoStory(page, RIGHT);
    const open = page.getByRole("button", { name: "Open right drawer" });
    await page.keyboard.press("Tab");
    await expect(open).toBeFocused();
    await expect(open).not.toHaveCSS("box-shadow", "none");
  });
});
