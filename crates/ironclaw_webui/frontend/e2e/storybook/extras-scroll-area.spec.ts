import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const VERTICAL_STORY = "extras-scrollarea--vertical";
const HORIZONTAL_STORY = "extras-scrollarea--horizontal";

const VIEWPORT = "[data-radix-scroll-area-viewport]";

test.describe("extras/scroll-area", () => {
  test("vertical: content overflows and scrolls to the last row", async ({
    page,
  }) => {
    await gotoStory(page, VERTICAL_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByText("Log line 1", { exact: true })).toBeVisible();

    const viewport = page.locator(VIEWPORT);
    const overflows = await viewport.evaluate(
      (el) => el.scrollHeight > el.clientHeight
    );
    expect(overflows).toBe(true);

    await expect(
      page.getByText("Log line 30", { exact: true })
    ).not.toBeInViewport();
    await viewport.evaluate((el) => {
      el.scrollTop = el.scrollHeight;
    });
    await expect(
      page.getByText("Log line 30", { exact: true })
    ).toBeInViewport();
  });

  test("vertical: hovering a row changes its background color", async ({
    page,
  }) => {
    await gotoStory(page, VERTICAL_STORY);
    const row = page.getByText("Log line 1", { exact: true });
    const before = await computedStyle(row, "background-color");
    await row.hover();
    await expect(row).not.toHaveCSS("background-color", before);
  });

  test("horizontal: content overflows and scrolls to the last card", async ({
    page,
  }) => {
    await gotoStory(page, HORIZONTAL_STORY);
    await expect(page.getByText("Card 1", { exact: true })).toBeVisible();

    const viewport = page.locator(VIEWPORT);
    const overflows = await viewport.evaluate(
      (el) => el.scrollWidth > el.clientWidth
    );
    expect(overflows).toBe(true);

    await expect(
      page.getByText("Card 12", { exact: true })
    ).not.toBeInViewport();
    await viewport.evaluate((el) => {
      el.scrollLeft = el.scrollWidth;
    });
    await expect(page.getByText("Card 12", { exact: true })).toBeInViewport();
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, VERTICAL_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByText("Log line 1", { exact: true })).toBeVisible();
  });
});
