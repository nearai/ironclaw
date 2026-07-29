import { expect, test, type Page } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const DEFAULT_STORY = "extras-slider--default";
const RANGE_STORY = "extras-slider--range";
const DISABLED_STORY = "extras-slider--disabled";
const VERTICAL_STORY = "extras-slider--vertical";

/**
 * The stories put aria-label on the slider Root, so the thumbs (role=slider)
 * carry no accessible name — locate them by role alone.
 *
 * The Range/Disabled stories render the bare horizontal slider as the root
 * story child; inside Storybook's shrink-wrapped root it computes to 0px
 * width, so gotoStory's visible-wait would hang. Navigate with an
 * attached-wait instead (the 16px thumbs themselves stay visible).
 */
async function gotoBareStory(
  page: Page,
  storyId: string,
  theme: "dark" | "light" = "dark"
) {
  await page.goto(
    `/iframe.html?id=${storyId}&viewMode=story&globals=theme:${theme}`
  );
  await page
    .locator("#storybook-root > *")
    .first()
    .waitFor({ state: "attached" });
}

test.describe("extras/slider", () => {
  test("renders a horizontal slider with the initial value", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const thumb = page.getByRole("slider");
    await expect(thumb).toBeVisible();
    await expect(thumb).toHaveAttribute("aria-valuenow", "40");
    await expect(thumb).toHaveAttribute("aria-valuemin", "0");
    await expect(thumb).toHaveAttribute("aria-valuemax", "100");
    await expect(thumb).toHaveAttribute("aria-orientation", "horizontal");
    // The root carries the accessible label in these stories.
    await expect(page.getByLabel("Concurrency")).toBeVisible();
  });

  test("arrows step the value; Home/End jump to min/max", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const thumb = page.getByRole("slider");
    await page.keyboard.press("Tab");
    await expect(thumb).toBeFocused();

    await page.keyboard.press("ArrowRight");
    await expect(thumb).toHaveAttribute("aria-valuenow", "45");
    // Story label mirrors the value.
    await expect(page.getByText("45", { exact: true })).toBeVisible();

    await page.keyboard.press("ArrowLeft");
    await expect(thumb).toHaveAttribute("aria-valuenow", "40");

    await page.keyboard.press("Home");
    await expect(thumb).toHaveAttribute("aria-valuenow", "0");
    await page.keyboard.press("End");
    await expect(thumb).toHaveAttribute("aria-valuenow", "100");
  });

  test("keyboard focus shows a focus ring on the thumb", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const thumb = page.getByRole("slider");
    await expect(thumb).toHaveCSS("box-shadow", "none");
    await page.keyboard.press("Tab");
    await expect(thumb).toBeFocused();
    await expect(thumb).not.toHaveCSS("box-shadow", "none");
  });

  test("hovering the thumb changes its background color", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const thumb = page.getByRole("slider");
    const before = await computedStyle(thumb, "background-color");
    await thumb.hover();
    await expect(thumb).not.toHaveCSS("background-color", before);
  });

  test("range slider renders two thumbs with distinct values", async ({
    page,
  }) => {
    await gotoBareStory(page, RANGE_STORY);
    const thumbs = page.getByRole("slider");
    await expect(thumbs).toHaveCount(2);
    await expect(thumbs.nth(0)).toHaveAttribute("aria-valuenow", "20");
    await expect(thumbs.nth(1)).toHaveAttribute("aria-valuenow", "70");
  });

  test("disabled slider is dimmed and marked disabled", async ({ page }) => {
    await gotoBareStory(page, DISABLED_STORY);
    const root = page.locator("#storybook-root [data-disabled]").first();
    await expect(root).toHaveAttribute("aria-disabled", "true");
    await expect(root).toHaveCSS("opacity", "0.5");
  });

  test("vertical slider reports vertical orientation", async ({ page }) => {
    await gotoStory(page, VERTICAL_STORY);
    const thumb = page.getByRole("slider");
    await expect(thumb).toHaveAttribute("aria-orientation", "vertical");
    await expect(thumb).toHaveAttribute("aria-valuenow", "30");
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("slider")).toBeVisible();
  });
});
