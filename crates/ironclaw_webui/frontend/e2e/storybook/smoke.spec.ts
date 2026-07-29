import { expect, test } from "@playwright/test";
import { gotoStory } from "./helpers";

test.describe("storybook smoke", () => {
  test("renders a story and applies the dark theme", async ({ page }) => {
    await gotoStory(page, "components-button--primary");
    await expect(page.getByRole("button", { name: "Button" })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("renders in the light theme via globals", async ({ page }) => {
    await gotoStory(page, "components-button--primary", { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });
});
