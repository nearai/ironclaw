import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const HORIZONTAL_STORY = "extras-resizable--horizontal";
const VERTICAL_STORY = "extras-resizable--vertical";
const NESTED_STORY = "extras-resizable--nested";

test.describe("extras/resizable", () => {
  test("horizontal: renders both panels and a separator handle", async ({
    page,
  }) => {
    await gotoStory(page, HORIZONTAL_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByText("Sidebar")).toBeVisible();
    await expect(page.getByText("Content")).toBeVisible();
    await expect(page.getByRole("separator")).toBeVisible();
  });

  test("arrow keys resize the panels via the focused handle", async ({
    page,
  }) => {
    await gotoStory(page, HORIZONTAL_STORY);
    const handle = page.getByRole("separator");
    const sidebar = page.getByText("Sidebar");
    const before = (await sidebar.boundingBox())!.width;

    await page.keyboard.press("Tab");
    await expect(handle).toBeFocused();
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowRight");
    await expect
      .poll(async () => (await sidebar.boundingBox())!.width)
      .toBeGreaterThan(before);

    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");
    await expect
      .poll(async () => (await sidebar.boundingBox())!.width)
      .toBeLessThanOrEqual(before);
  });

  test("keyboard focus shows a focus ring on the handle", async ({ page }) => {
    await gotoStory(page, HORIZONTAL_STORY);
    const handle = page.getByRole("separator");
    await expect(handle).toHaveCSS("box-shadow", "none");
    await page.keyboard.press("Tab");
    await expect(handle).toBeFocused();
    await expect(handle).not.toHaveCSS("box-shadow", "none");
  });

  test("hovering the handle changes its background color", async ({
    page,
  }) => {
    await gotoStory(page, HORIZONTAL_STORY);
    const handle = page.getByRole("separator");
    const before = await computedStyle(handle, "background-color");
    await handle.hover();
    await expect(handle).not.toHaveCSS("background-color", before);
  });

  test("vertical: renders stacked panels with a handle", async ({ page }) => {
    await gotoStory(page, VERTICAL_STORY);
    await expect(page.getByText("Editor", { exact: true })).toBeVisible();
    await expect(page.getByText("Terminal", { exact: true })).toBeVisible();
    await expect(page.getByRole("separator")).toBeVisible();
  });

  test("nested: renders all panes and both handles", async ({ page }) => {
    await gotoStory(page, NESTED_STORY);
    for (const label of ["Nav", "Main", "Logs"]) {
      await expect(page.getByText(label, { exact: true })).toBeVisible();
    }
    // One handle in the outer group and one in the nested group.
    await expect(page.getByRole("separator")).toHaveCount(2);
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, HORIZONTAL_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByText("Sidebar")).toBeVisible();
  });
});
