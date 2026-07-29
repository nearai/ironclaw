import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const DEFAULT_STORY = "extras-progress--default";
const LIVE_STORY = "extras-progress--live";
const TONES_STORY = "extras-progress--tones";

test.describe("extras/progress", () => {
  test("renders a progressbar with the correct value", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const bar = page.getByRole("progressbar", { name: "Upload" });
    await expect(bar).toBeVisible();
    await expect(bar).toHaveAttribute("aria-valuenow", "64");
    await expect(bar).toHaveAttribute("aria-valuemax", "100");
    // The indicator is sized to the value.
    await expect(bar.locator("> *").first()).toHaveAttribute(
      "style",
      /width: 64%/
    );
  });

  test("live story keeps reporting a numeric value", async ({ page }) => {
    await gotoStory(page, LIVE_STORY);
    const bar = page.getByRole("progressbar", { name: "Sync progress" });
    await expect(bar).toBeVisible();
    await expect(bar).toHaveAttribute("aria-valuenow", /^\d+$/);
  });

  test("tones story renders four bars with distinct indicator colors", async ({
    page,
  }) => {
    await gotoStory(page, TONES_STORY);
    const expected: Array<[string, string]> = [
      ["Accent", "80"],
      ["Positive", "100"],
      ["Warning", "55"],
      ["Danger", "25"],
    ];
    for (const [name, value] of expected) {
      await expect(
        page.getByRole("progressbar", { name, exact: true })
      ).toHaveAttribute("aria-valuenow", value);
    }

    const positiveIndicator = page
      .getByRole("progressbar", { name: "Positive", exact: true })
      .locator("> *")
      .first();
    const dangerIndicator = page
      .getByRole("progressbar", { name: "Danger", exact: true })
      .locator("> *")
      .first();
    const positiveColor = await computedStyle(
      positiveIndicator,
      "background-color"
    );
    await expect(dangerIndicator).not.toHaveCSS(
      "background-color",
      positiveColor
    );
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("progressbar", { name: "Upload" })).toBeVisible();
  });
});
