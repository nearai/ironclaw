/**
 * Extras/AspectRatio — structural component: asserts the ratio box actually
 * constrains its content to 16:9 and 1:1, plus theme rendering.
 */
import { expect, test } from "@playwright/test";
import { gotoStory } from "./helpers";

const SIXTEEN_NINE = "extras-aspectratio--sixteen-by-nine";
const SQUARE = "extras-aspectratio--square";

test.describe("extras aspect-ratio", () => {
  test("16:9 story renders content sized to the ratio (dark theme)", async ({
    page,
  }) => {
    await gotoStory(page, SIXTEEN_NINE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const frame = page.getByText("16 : 9");
    await expect(frame).toBeVisible();
    const box = await frame.boundingBox();
    expect(box).not.toBeNull();
    // w-80 container = 320px wide → height should be 320 * 9/16 = 180px.
    expect(box!.width).toBeCloseTo(320, 0);
    expect(box!.width / box!.height).toBeCloseTo(16 / 9, 1);
  });

  test("square story renders a 1:1 box", async ({ page }) => {
    await gotoStory(page, SQUARE);
    const frame = page.getByText("1 : 1");
    await expect(frame).toBeVisible();
    const box = await frame.boundingBox();
    expect(box).not.toBeNull();
    // w-48 container = 192px wide → equal height.
    expect(box!.width).toBeCloseTo(192, 0);
    expect(box!.width / box!.height).toBeCloseTo(1, 1);
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, SIXTEEN_NINE, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByText("16 : 9")).toBeVisible();
  });
});
