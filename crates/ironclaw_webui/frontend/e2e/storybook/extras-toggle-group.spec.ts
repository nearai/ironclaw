import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const SINGLE_STORY = "extras-togglegroup--single";
const MULTIPLE_STORY = "extras-togglegroup--multiple";
const DISABLED_STORY = "extras-togglegroup--disabled";

test.describe("extras/toggle-group", () => {
  test("single: renders with one item on and click moves selection", async ({
    page,
  }) => {
    await gotoStory(page, SINGLE_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const list = page.getByLabel("List view", { exact: true });
    const grid = page.getByLabel("Grid view", { exact: true });
    const board = page.getByLabel("Board view", { exact: true });
    await expect(list).toHaveAttribute("data-state", "on");
    await expect(grid).toHaveAttribute("data-state", "off");
    await expect(board).toHaveAttribute("data-state", "off");

    await grid.click();
    await expect(grid).toHaveAttribute("data-state", "on");
    await expect(list).toHaveAttribute("data-state", "off");
  });

  test("single: arrows move focus and space selects", async ({ page }) => {
    await gotoStory(page, SINGLE_STORY);
    const list = page.getByLabel("List view", { exact: true });
    const grid = page.getByLabel("Grid view", { exact: true });

    await page.keyboard.press("Tab");
    await expect(list).toBeFocused();

    await page.keyboard.press("ArrowRight");
    await expect(grid).toBeFocused();
    // Focus alone does not select in a toggle group.
    await expect(grid).toHaveAttribute("data-state", "off");

    await page.keyboard.press("Space");
    await expect(grid).toHaveAttribute("data-state", "on");
    await expect(list).toHaveAttribute("data-state", "off");
  });

  test("multiple: several items can be on at once", async ({ page }) => {
    await gotoStory(page, MULTIPLE_STORY);
    const bold = page.getByLabel("Bold", { exact: true });
    const italic = page.getByLabel("Italic", { exact: true });
    await expect(bold).toHaveAttribute("data-state", "on");
    await expect(italic).toHaveAttribute("data-state", "off");

    await italic.click();
    await expect(italic).toHaveAttribute("data-state", "on");
    await expect(bold).toHaveAttribute("data-state", "on");

    await bold.click();
    await expect(bold).toHaveAttribute("data-state", "off");
    await expect(italic).toHaveAttribute("data-state", "on");
  });

  test("disabled item is dimmed and skipped by arrow navigation", async ({
    page,
  }) => {
    await gotoStory(page, DISABLED_STORY);
    const grid = page.getByLabel("Grid view", { exact: true });
    await expect(grid).toBeDisabled();
    await expect(grid).toHaveCSS("opacity", "0.5");

    const list = page.getByLabel("List view", { exact: true });
    const board = page.getByLabel("Board view", { exact: true });
    await page.keyboard.press("Tab");
    await expect(list).toBeFocused();
    await page.keyboard.press("ArrowRight");
    await expect(board).toBeFocused();
    await expect(grid).toHaveAttribute("data-state", "off");
  });

  test("hover changes an off item's background color", async ({ page }) => {
    await gotoStory(page, SINGLE_STORY);
    const grid = page.getByLabel("Grid view", { exact: true });
    const before = await computedStyle(grid, "background-color");
    await grid.hover();
    await expect(grid).not.toHaveCSS("background-color", before);
  });

  test("keyboard focus shows a focus ring", async ({ page }) => {
    await gotoStory(page, SINGLE_STORY);
    const list = page.getByLabel("List view", { exact: true });
    await expect(list).toHaveCSS("box-shadow", "none");
    await page.keyboard.press("Tab");
    await expect(list).toBeFocused();
    await expect(list).not.toHaveCSS("box-shadow", "none");
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, SINGLE_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByLabel("List view", { exact: true })).toHaveAttribute(
      "data-state",
      "on"
    );
  });
});
