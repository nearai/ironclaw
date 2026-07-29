import { expect, test } from "@playwright/test";
import { gotoStory } from "./helpers";

const STORY = "extras-separator--default";

test.describe("extras/separator", () => {
  test("renders horizontal and vertical separators with 1px lines", async ({
    page,
  }) => {
    await gotoStory(page, STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByText("IronClaw UI")).toBeVisible();

    const horizontal = page.locator(
      '#storybook-root [data-orientation="horizontal"]'
    );
    await expect(horizontal).toHaveCount(1);
    await expect(horizontal).toHaveCSS("height", "1px");
    await expect(horizontal).not.toHaveCSS(
      "background-color",
      "rgba(0, 0, 0, 0)"
    );

    const vertical = page.locator(
      '#storybook-root [data-orientation="vertical"]'
    );
    await expect(vertical).toHaveCount(2);
    await expect(vertical.first()).toHaveCSS("width", "1px");
    await expect(vertical.first()).not.toHaveCSS(
      "background-color",
      "rgba(0, 0, 0, 0)"
    );
  });

  test("decorative separators are hidden from the accessibility tree", async ({
    page,
  }) => {
    await gotoStory(page, STORY);
    // decorative -> role="none", so no semantic separators are exposed.
    await expect(page.getByRole("separator")).toHaveCount(0);
    await expect(
      page.locator('#storybook-root [role="none"]')
    ).toHaveCount(3);
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(
      page.locator('#storybook-root [data-orientation="horizontal"]')
    ).toBeVisible();
  });
});
